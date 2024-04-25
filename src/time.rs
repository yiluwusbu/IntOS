use vcell::VolatileCell;

// use crate::benchmarks::benchmark_start;
#[cfg(test)]
use crate::crash_point;
use crate::critical::CriticalSection;
use crate::debug_print;
use crate::list::SortedPList;
use crate::list::{self, InsertSortedPList, ListTxOpLog};
use crate::marker::TxInSafe;
use crate::pmem::{JournalHandle, PMPtr, PMVar};
use crate::queue::{queue_block_until_not_empty, queue_create, queue_send_back};
use crate::syscalls::{sys_queue_receive, QueueHandle};
use crate::task::{create_task_static, ErrorCode, Task};
use crate::user::pbox::RelaxedPBox;
use crate::user::transaction as user_tx;
use crate::util::cast_to_u8_ptr;
use crate::{arch, declare_const_pm_var_unsafe, os_print, transaction};
use crate::{heap, list::Node, queue::Queue};
use core::cell::UnsafeCell;
use core::mem::{size_of, transmute};
pub const MAX_DELAY_TIME: Time = Time::MAX;

const CMD_QUEUE_SIZE: usize = 32;

unsafe impl TxInSafe for TimeManager {}

pub type Time = arch::ArchTimeType;
pub type TimerCallBackFnType = fn(usize, JournalHandle);

#[cfg(test)]
static mut TIMER_DAEMON_CRASH_POINT: usize = 0;

#[cfg(test)]
pub fn set_timer_daemon_crash_point(cp: usize) {
    unsafe { TIMER_DAEMON_CRASH_POINT = cp };
}

#[cfg(test)]
pub fn get_timer_daemon_crash_point() -> usize {
    unsafe { TIMER_DAEMON_CRASH_POINT }
}

#[repr(C)]
pub struct TimeManager {
    tick_counter: VolatileCell<Time>,
    next_unblock_time: VolatileCell<Time>,
    next_expiry_time: VolatileCell<Time>,
    timer_cmd_queue: UnsafeCell<Option<PMPtr<Queue>>>,
    active_timer_list: UnsafeCell<PMVar<SortedPList<TimerListItem>>>,
    message_buf: UnsafeCell<TimerMessage>,
    timer_daemon_task: UnsafeCell<Option<PMPtr<Task>>>,
}

unsafe impl Sync for TimeManager {}

#[cfg(feature = "opt_list")]
pub unsafe fn get_timer_active_list() -> &'static mut SortedPList<TimerListItem> {
    TIME_MANAGER.get_timer_list_no_logging()
}

fn timer_closure_runner<F>(p: usize, j: JournalHandle)
where
    F: FnMut(JournalHandle) + Send + 'static,
{
    let mut f: RelaxedPBox<F> = unsafe { transmute(p) };
    unsafe { (f.as_mut_no_logging())(j) };
    core::mem::forget(f);
}

impl TimeManager {
    pub const fn new() -> Self {
        Self {
            tick_counter: VolatileCell::new(0),
            next_unblock_time: VolatileCell::new(MAX_DELAY_TIME),
            next_expiry_time: VolatileCell::new(MAX_DELAY_TIME),
            timer_cmd_queue: UnsafeCell::new(None),
            active_timer_list: unsafe { UnsafeCell::new(PMVar::new(SortedPList::new())) },
            message_buf: UnsafeCell::new(TimerMessage {
                cmd: TimerCmd::Invalid,
                timer: None,
                opaque: 0,
            }),
            timer_daemon_task: UnsafeCell::new(None),
        }
    }

    #[inline(always)]
    pub fn inc_tick(&self, x: Time) -> Time {
        let v = self.tick_counter.get() + 1;
        self.tick_counter.set(v);
        v
    }

    #[inline(always)]
    pub fn get_ticks(&self) -> Time {
        self.tick_counter.get()
    }

    #[inline(always)]
    pub fn set_next_unblock_time(&self, x: Time) {
        self.next_unblock_time.set(x);
    }

    #[inline(always)]
    pub fn get_next_unblock_time(&self) -> Time {
        self.next_unblock_time.get()
    }

    #[inline(always)]
    pub fn set_next_expiry_time(&self, x: Time) {
        self.next_expiry_time.set(x);
    }

    #[inline(always)]
    pub fn get_next_expiry_time(&self) -> Time {
        self.next_expiry_time.get()
    }

    #[inline(always)]
    pub fn expired_by_now(&self, time: Time) -> bool {
        debug_print!("Expiry time: {}, now: {}", time, self.get_ticks());
        self.get_ticks() >= time
    }

    #[cfg(not(feature = "opt_list"))]
    #[inline(always)]
    fn get_mut_active_timer_list(&self, j: JournalHandle) -> &mut SortedPList<TimerListItem> {
        unsafe { &mut *self.active_timer_list.get() }.borrow_mut(j)
    }

    #[cfg(feature = "opt_list")]
    #[inline(always)]
    unsafe fn get_timer_list_no_logging(&self) -> &mut SortedPList<TimerListItem> {
        unsafe { (&mut *self.active_timer_list.get()).borrow_mut_no_logging() }
    }

    #[inline(always)]
    fn get_active_timer_list(&self) -> &SortedPList<TimerListItem> {
        unsafe { &*self.active_timer_list.get() }.borrow()
    }

    #[inline(always)]
    fn get_cmd_queue(&self) -> Option<&mut Queue> {
        unsafe { *self.timer_cmd_queue.get() }.map(|mut q| unsafe { q.as_mut_no_logging() })
    }

    #[inline(always)]
    fn get_cmd_queue_ptr(&self) -> Option<PMPtr<Queue>> {
        unsafe { *self.timer_cmd_queue.get() }
    }

    #[inline(always)]
    pub fn get_timer_daemon_task(&self) -> Option<&mut Task> {
        unsafe { *self.timer_daemon_task.get() }.map(|mut t| unsafe { t.as_mut_no_logging() })
    }

    #[inline(always)]
    fn set_timer_daemon_task(&self, t: Option<PMPtr<Task>>) {
        *unsafe { &mut *self.timer_daemon_task.get() } = t;
    }

    #[cfg(not(feature = "opt_list"))]
    fn insert_into_active_list(&self, timer: &mut Timer, expiry_time: Time, j: JournalHandle) {
        let active_list = self.get_mut_active_timer_list(j);
        timer.active = true;
        // There is only one task inserting timer into active list
        // So we don't need to actually put it into a critical section
        let cs = unsafe { CriticalSection::new() };
        let node = timer.get_list_node().as_mut(j);
        timer.set_expiry_time(node, expiry_time);
        active_list.insert_node(&cs, j, node);
    }

    #[cfg(feature = "opt_list")]
    fn activate_timer(&self, timer: &mut Timer, expiry_time: Time) {
        timer.active = true;
        let node = unsafe { timer.get_list_node().as_mut_no_logging() };
        timer.set_expiry_time(node, expiry_time);
    }

    #[cfg(feature = "opt_list")]
    fn deactivate_timer(&self, timer: &mut Timer) {
        timer.active = false;
    }

    #[cfg(not(feature = "opt_list"))]
    fn remove_from_active_list(&self, timer: &mut Timer, j: JournalHandle) {
        if timer.is_in_active_list() {
            assert!(timer.active);
            let active_list = self.get_mut_active_timer_list(j);
            let cs = unsafe { CriticalSection::new() };
            let node = timer.get_list_node().as_mut(j);
            active_list.remove_node(&cs, j, node);
        }
    }

    unsafe fn get_message_buf(&self) -> &mut TimerMessage {
        unsafe { &mut *self.message_buf.get() }
    }

    #[cfg(feature = "opt_list")]
    fn process_receive_timer_cmd(&self) {
        let q = self.get_cmd_queue();
        debug_assert!(!q.is_none());

        loop {
            ListTxOpLog::get_timer_list_tx_op_log().invalidate();
            let res = user_tx::run_sys_once(|j, t| {
                let msg = unsafe { self.get_message_buf() };
                let ptr = self.get_cmd_queue_ptr().unwrap();
                let q_handle = unsafe { QueueHandle::new(ptr) };
                let r = sys_queue_receive::<TimerMessage>(q_handle, 0, t);
                match r {
                    Err(_) => {
                        return Err(ErrorCode::TxExit);
                    }
                    Ok(v) => {
                        *msg = v;
                    }
                };
                let timer = match msg.timer {
                    Some(mut t) => unsafe { t.as_mut_no_logging() },
                    None => {
                        return Ok(());
                    }
                };
                match msg.cmd {
                    TimerCmd::Start | TimerCmd::Reset => {
                        let cmd_time = msg.opaque;
                        let expiry_time = cmd_time.checked_add(timer.period_ticks).unwrap();
                        self.activate_timer(timer, expiry_time);
                        list::atomic_roll_forward_remove_reinsert_into_activelist(
                            timer.get_list_node(),
                        );
                    }
                    TimerCmd::Stop => {
                        timer.active = false;
                        list::atomic_roll_forward_remove_from_activelist(timer.get_list_node());
                    }
                    TimerCmd::Delete => {
                        timer.active = false;
                        list::atomic_roll_forward_remove_from_activelist(timer.get_list_node());
                        // TODO(): free the timer here
                    }
                    TimerCmd::SetPeriod => {
                        let new_period = msg.opaque;
                        if new_period <= 0 {
                            return Ok(());
                        }
                        timer.period_ticks = new_period;
                        let cur_time = self.get_ticks();
                        let expiry_time = cur_time.checked_add(new_period).unwrap();
                        self.activate_timer(timer, expiry_time);
                        list::atomic_roll_forward_remove_reinsert_into_activelist(
                            timer.get_list_node(),
                        );
                    }
                    TimerCmd::Invalid => {
                        debug_print!("Invalid Timer cmd!");
                    }
                }
                Ok(())
            });
            if let Err(_) = res {
                debug_print!("No more messages...");
                break;
            }
        }
    }
    #[cfg(test)]
    #[cfg(feature = "opt_list")]
    fn crashed_process_receive_timer_cmd(
        &self,
        cp0: usize,
        cp1: usize,
        cp2: usize,
        cp_syscall: usize,
    ) {
        let q = self.get_cmd_queue();
        debug_assert!(!q.is_none());
        crate::set_crash_point!(timer_daemon, cp0);
        crate::syscalls::set_syscall_end_crash_point(cp_syscall);
        loop {
            ListTxOpLog::get_timer_list_tx_op_log().invalidate();
            let res = user_tx::may_crashed_run_sys_once(cp0 < 4, |j, t| {
                let msg = unsafe { self.get_message_buf() };
                let ptr = self.get_cmd_queue_ptr().unwrap();
                let q_handle = unsafe { QueueHandle::new(ptr) };
                crash_point!(timer_daemon, 0, Err(ErrorCode::TxExit));
                let r = sys_queue_receive::<TimerMessage>(q_handle, 0, t);
                crash_point!(timer_daemon, 1, Err(ErrorCode::TxExit));
                match r {
                    Err(_) => {
                        return Err(ErrorCode::TxExit);
                    }
                    Ok(v) => {
                        *msg = v;
                    }
                };
                let timer = match msg.timer {
                    Some(mut t) => unsafe { t.as_mut_no_logging() },
                    None => {
                        return Ok(());
                    }
                };
                match msg.cmd {
                    TimerCmd::Start | TimerCmd::Reset => {
                        let cmd_time = msg.opaque;
                        let expiry_time = cmd_time.checked_add(timer.period_ticks).unwrap();
                        crash_point!(timer_daemon, 2, Err(ErrorCode::TxExit));
                        self.activate_timer(timer, expiry_time);
                        crash_point!(
                            timer_daemon,
                            3,
                            {
                                list::test::crashed_atomic_roll_forward_remove_reinsert_into_activelist(cp1, cp2, timer.get_list_node());
                            },
                            {
                                list::atomic_roll_forward_remove_reinsert_into_activelist(
                                    timer.get_list_node(),
                                );
                            }
                        );

                        crash_point!(timer_daemon, 3, Err(ErrorCode::TxExit));
                    }
                    TimerCmd::Stop => {
                        crash_point!(timer_daemon, 2, Err(ErrorCode::TxExit));
                        timer.active = false;
                        crash_point!(
                            timer_daemon,
                            3,
                            {
                                list::test::crashed_atomic_roll_forward_remove_from_activelist(
                                    cp1,
                                    cp2,
                                    timer.get_list_node(),
                                );
                            },
                            {
                                list::atomic_roll_forward_remove_from_activelist(
                                    timer.get_list_node(),
                                );
                            }
                        );
                        crash_point!(timer_daemon, 3, Err(ErrorCode::TxExit));
                    }
                    TimerCmd::Delete => {
                        timer.active = false;
                        list::atomic_roll_forward_remove_from_activelist(timer.get_list_node());
                        // TODO(): free the timer here
                    }
                    TimerCmd::SetPeriod => {
                        let new_period = msg.opaque;
                        if new_period <= 0 {
                            return Ok(());
                        }
                        timer.period_ticks = new_period;
                        let cur_time = self.get_ticks();
                        let expiry_time = cur_time.checked_add(new_period).unwrap();
                        self.activate_timer(timer, expiry_time);
                        list::atomic_roll_forward_remove_reinsert_into_activelist(
                            timer.get_list_node(),
                        );
                    }
                    TimerCmd::Invalid => {
                        debug_print!("Invalid Timer cmd!");
                    }
                }
                Ok(())
            });
            if let Err(_) = res {
                debug_print!("No more messages...");
                break;
            }
        }
    }

    #[cfg(not(feature = "opt_list"))]
    fn process_receive_timer_cmd(&self) {
        let q = self.get_cmd_queue();
        assert!(!q.is_none());
        loop {
            let res = user_tx::run_sys_once(|j, t| {
                let msg = unsafe { self.get_message_buf() };
                let ptr = self.get_cmd_queue_ptr().unwrap();
                let q_handle = unsafe { QueueHandle::new(ptr) };
                let r = sys_queue_receive(q_handle, 0, t);
                match r {
                    Err(_) => {
                        return Err(ErrorCode::TxExit);
                    }
                    Ok(v) => {
                        *msg = v;
                    }
                };
                let timer = match msg.timer {
                    Some(mut t) => unsafe { t.as_mut_no_logging() },
                    None => {
                        return Ok(());
                    }
                };
                // if timer is already in active list, remove it

                self.remove_from_active_list(timer, j);
                debug_print!("Cmd received: {:?}", msg.cmd);
                match msg.cmd {
                    TimerCmd::Start | TimerCmd::Reset => {
                        let cmd_time = msg.opaque;
                        let expiry_time = cmd_time.checked_add(timer.period_ticks).unwrap();
                        self.insert_into_active_list(timer, expiry_time, j);
                    }
                    TimerCmd::Stop => {
                        timer.active = false;
                    }
                    TimerCmd::Delete => {
                        timer.active = false;
                        // TODO(): free the timer here
                    }
                    TimerCmd::SetPeriod => {
                        let new_period = msg.opaque;
                        if new_period <= 0 {
                            return Ok(());
                        }
                        timer.active = true;
                        timer.period_ticks = new_period;
                        let cur_time = self.get_ticks();
                        let expiry_time = cur_time.checked_add(new_period).unwrap();
                        self.insert_into_active_list(timer, expiry_time, j);
                    }
                    TimerCmd::Invalid => {
                        debug_print!("Invalid Timer cmd!");
                    }
                }
                Ok(())
            });
            if let Err(_) = res {
                debug_print!("No more messages...");
                break;
            }
        }
    }

    #[cfg(not(feature = "opt_list"))]
    fn reload_timer(&self, timer: &mut Timer, j: JournalHandle) {
        let cs = unsafe { CriticalSection::new() };
        let next_expiry_time = self.get_ticks().checked_add(timer.period_ticks).unwrap();
        let active_list = self.get_mut_active_timer_list(j);

        let node = timer.get_list_node().as_mut(j);
        timer.set_expiry_time(node, next_expiry_time);
        active_list.insert_node(&cs, j, node);
    }

    #[cfg(feature = "opt_list")]
    fn process_expired_timer(&self) {
        let active_list = self.get_active_timer_list();
        let cs = unsafe { CriticalSection::new() };
        ListTxOpLog::get_timer_list_tx_op_log().invalidate();
        if let Some(&TimerListItem { timer, expiry_time }) = active_list.peek_front(&cs) {
            self.set_next_expiry_time(expiry_time);
            if self.expired_by_now(expiry_time) {
                debug_print!("Expired!");
                user_tx::run_once(|j| {
                    let mut timer_pm_ptr = timer.unwrap();
                    let timer = unsafe { timer_pm_ptr.as_mut_no_logging() };
                    // dispatch timer callback
                    // TODO: should we execute it in caller's context ?
                    debug_print!("Timer name is {}", timer.name);
                    if timer.call_back_is_closure {
                        // let boxed_f: RelaxedPBox<FnMut()> = unsafe { transmute(timer.callback)  };
                    } else {
                        let f = timer.callback;
                        f(timer.param, j);
                    }
                    // reload timer if necessary
                    if timer.auto_reload {
                        let next_expiry_time =
                            self.get_ticks().checked_add(timer.period_ticks).unwrap();
                        self.activate_timer(timer, next_expiry_time);
                        list::atomic_roll_forward_pop_reinsert_into_activelist();
                    } else {
                        debug_print!("Auto reload is false");
                        timer.active = false;
                        list::atomic_roll_forward_remove_from_activelist(timer.get_list_node());
                    }
                });
            } else {
                debug_print!("Not expired, blocks until next timer expires");
                let now = self.get_ticks();
                let expiry = self.get_next_expiry_time();
                if expiry > now {
                    queue_block_until_not_empty(
                        self.get_cmd_queue_ptr().unwrap(),
                        expiry - now,
                        false,
                    );
                }
            }
        } else {
            // currently no timer is in active state
            debug_print!("No timer active, blocking until next timer cmd arrives");
            queue_block_until_not_empty(self.get_cmd_queue_ptr().unwrap(), 0, true);
        }
    }

    #[cfg(test)]
    #[cfg(feature = "opt_list")]
    fn crashed_process_expired_timer(&self, cp0: usize, cp1: usize, cp2: usize) {
        crate::set_crash_point!(timer_daemon, cp0);
        let active_list = self.get_active_timer_list();
        let cs = unsafe { CriticalSection::new() };
        ListTxOpLog::get_timer_list_tx_op_log().invalidate();
        if let Some(&TimerListItem { timer, expiry_time }) = active_list.peek_front(&cs) {
            crash_point!(timer_daemon, 0);
            self.set_next_expiry_time(expiry_time);
            crash_point!(timer_daemon, 1);
            if self.expired_by_now(expiry_time) {
                debug_print!("Expired!");
                user_tx::may_crashed_run_once(cp0 < 4, |j| {
                    let mut timer_pm_ptr = timer.unwrap();
                    let timer = unsafe { timer_pm_ptr.as_mut_no_logging() };
                    // dispatch timer callback
                    // TODO: should we execute it in caller's context ?
                    debug_print!("Timer name is {}", timer.name);
                    if timer.call_back_is_closure {
                        // let boxed_f: RelaxedPBox<FnMut()> = unsafe { transmute(timer.callback)  };
                    } else {
                        let f = timer.callback;
                        f(timer.param, j);
                    }
                    // reload timer if necessary
                    if timer.auto_reload {
                        let next_expiry_time =
                            self.get_ticks().checked_add(timer.period_ticks).unwrap();
                        self.activate_timer(timer, next_expiry_time);
                        crash_point!(timer_daemon, 2);
                        crash_point!(
                            timer_daemon,
                            3,
                            {
                                list::test::crashed_atomic_roll_forward_pop_reinsert_into_activelist(cp1, cp2);
                            },
                            {
                                list::atomic_roll_forward_pop_reinsert_into_activelist();
                            }
                        );

                        crash_point!(timer_daemon, 3);
                    } else {
                        debug_print!("Auto reload is false");
                        timer.active = false;
                        crash_point!(
                            timer_daemon,
                            2,
                            {
                                list::test::crashed_atomic_roll_forward_remove_from_activelist(
                                    cp1,
                                    cp2,
                                    timer.get_list_node(),
                                )
                            },
                            {
                                list::atomic_roll_forward_remove_from_activelist(
                                    timer.get_list_node(),
                                );
                            }
                        );
                        crash_point!(timer_daemon, 2);
                    }
                });
            } else {
                panic!("unreachable");
                // debug_print!("Not expired, blocks until next timer expires");
                // let now = self.get_ticks();
                // let expiry = self.get_next_expiry_time();
                // if expiry > now {
                //     // queue_block_until_not_empty(self.get_cmd_queue_ptr().unwrap(), expiry - now, false);
                // }
            }
        } else {
            // currently no timer is in active state
            debug_print!("No timer active, blocking until next timer cmd arrives");
            // queue_block_until_not_empty(self.get_cmd_queue_ptr().unwrap(), 0, true);
        }
    }

    #[cfg(not(feature = "opt_list"))]
    fn process_expired_timer(&self) {
        let active_list = self.get_active_timer_list();
        let cs = unsafe { CriticalSection::new() };
        if let Some(&TimerListItem { timer, expiry_time }) = active_list.peek_front(&cs) {
            self.set_next_expiry_time(expiry_time);
            if self.expired_by_now(expiry_time) {
                debug_print!("Expired!");
                user_tx::run_once(|j| {
                    let active_list = self.get_mut_active_timer_list(j);
                    active_list.pop_front(&cs, j);
                    let mut timer_pm_ptr = timer.unwrap();
                    let timer = unsafe { timer_pm_ptr.as_mut_no_logging() };
                    // dispatch timer callback
                    // TODO: should we execute it in caller's context ?
                    debug_print!("Timer name is {}", timer.name);

                    if timer.call_back_is_closure {
                        // let boxed_f: RelaxedPBox<FnMut()> = unsafe { transmute(timer.callback)  };
                    } else {
                        let f = timer.callback;
                        f(timer.param, j);
                    }

                    // reload timer if necessary
                    if timer.auto_reload {
                        self.reload_timer(timer, j);
                    } else {
                        debug_print!("Auto reload is false");
                        timer.active = false;
                    }
                });
            } else {
                debug_print!("Not expired, blocks until next timer expires");
                let now = self.get_ticks();
                let expiry = self.get_next_expiry_time();
                if expiry > now {
                    queue_block_until_not_empty(
                        self.get_cmd_queue_ptr().unwrap(),
                        expiry - now,
                        false,
                    );
                }
            }
        } else {
            // currently no timer is in active state
            debug_print!("No timer active, blocking until next timer cmd arrives");
            queue_block_until_not_empty(self.get_cmd_queue_ptr().unwrap(), 0, true);
        }
    }

    // Called during boot time.
    pub fn create_daemon_timer_task(&self) {
        let msg_size = core::mem::size_of::<TimerMessage>();
        let q = queue_create(CMD_QUEUE_SIZE, msg_size);
        if q.is_none() {
            os_print!("Failed to create timer cmd queue");
        } else {
            unsafe {
                *(self.timer_cmd_queue.get()) = q;
            }
            match create_task_static("timer daemon", 1, daemon_timer_task as usize, 0, 0) {
                Err(e) => {
                    os_print!("Failed to create daemon timer task..., errcode = {:?}", e);
                }
                Ok(handle) => {
                    TIME_MANAGER.set_timer_daemon_task(handle.get_task_ptr());
                    os_print!("Timer daemon task created");
                }
            }
        }
    }
}

pub fn update_countdown(remaining_ticks: &mut Time, expiry_time: Time) {
    let cur_time = TIME_MANAGER.get_ticks();
    if cur_time >= expiry_time {
        *remaining_ticks = 0;
        return;
    }
    *remaining_ticks = expiry_time - cur_time;
}

// pub static TIME_MANAGER: TimeManager = TimeManager::new();
declare_const_pm_var_unsafe!(pub, TIME_MANAGER, TimeManager, TimeManager::new());

type TimerListNode = Node<TimerListItem>;

#[derive(Clone, Copy)]
pub struct TimerListItem {
    timer: Option<PMPtr<Timer>>,
    expiry_time: Time,
}

impl PartialEq for TimerListItem {
    fn eq(&self, other: &Self) -> bool {
        self.expiry_time.eq(&other.expiry_time)
    }
}

impl Eq for TimerListItem {}

impl PartialOrd for TimerListItem {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        self.expiry_time.partial_cmp(&other.expiry_time)
    }
}

impl Ord for TimerListItem {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.expiry_time.cmp(&other.expiry_time)
    }
}

pub struct Timer {
    name: &'static str,
    list_node: PMVar<TimerListNode>,
    period_ticks: Time,
    callback: TimerCallBackFnType, // can be a rust closure
    call_back_is_closure: bool,
    param: usize,
    active: bool,
    auto_reload: bool,
}

impl Timer {
    pub fn new_from_heap(
        name: &'static str,
        period_ticks: Time,
        auto_reload: bool,
        callback: TimerCallBackFnType,
        param: usize,
        j: JournalHandle,
    ) -> Option<PMPtr<Self>> {
        heap::pm_new(
            Self {
                name,
                list_node: unsafe {
                    PMVar::new(TimerListNode::new(TimerListItem {
                        timer: None,
                        expiry_time: 0,
                    }))
                },
                period_ticks,
                callback,
                param,
                call_back_is_closure: false,
                active: false,
                auto_reload,
            },
            j,
        )
    }

    pub fn new(
        name: &'static str,
        period_ticks: Time,
        auto_reload: bool,
        callback: TimerCallBackFnType,
        expiry_time: Time,
        param: usize,
    ) -> Self {
        Self {
            name,
            list_node: unsafe {
                PMVar::new(TimerListNode::new(TimerListItem {
                    timer: None,
                    expiry_time,
                }))
            },
            period_ticks,
            callback,
            param,
            call_back_is_closure: false,
            active: false,
            auto_reload,
        }
    }

    #[inline(always)]
    pub fn get_expiry_time(&self) -> Time {
        self.list_node.value.expiry_time
    }

    #[inline(always)]
    pub fn set_expiry_time(&self, node: &mut TimerListNode, time: Time) {
        node.value.expiry_time = time;
    }

    #[inline(always)]
    pub fn is_in_active_list(&self) -> bool {
        !self.list_node.list.is_none()
    }

    #[inline(always)]
    pub fn get_list_node(&self) -> PMPtr<TimerListNode> {
        unsafe { self.list_node.to_pm_ptr() }
    }
}

#[derive(Debug)]
enum TimerCmd {
    Invalid,
    Start,
    Stop,
    Reset,
    Delete,
    SetPeriod,
}

pub struct TimerMessage {
    cmd: TimerCmd,
    timer: Option<PMPtr<Timer>>,
    opaque: Time,
}

pub fn create_timer(
    name: &'static str,
    period_ticks: Time,
    auto_reload: bool,
    callback: TimerCallBackFnType,
    param: usize,
) -> Option<PMPtr<Timer>> {
    transaction::run(|j| {
        let r = Timer::new_from_heap(name, period_ticks, auto_reload, callback, param, j).map(
            |mut ptr| unsafe {
                let timer = ptr.as_mut_no_logging();
                timer.list_node.borrow_mut_no_logging().value.timer = Some(ptr);
                ptr
            },
        );
        r
    })
}

pub fn create_timer_with_closure<F>(
    name: &'static str,
    period_ticks: Time,
    auto_reload: bool,
    callback: F,
) -> Option<PMPtr<Timer>>
where
    F: FnMut(JournalHandle) + Send + 'static,
{
    unsafe {
        transaction::run_relaxed(|j| {
            let boxed_f = RelaxedPBox::try_new_for_kernel(callback, j);
            let boxed_f = match boxed_f {
                Ok(f) => f,
                Err(_) => return None,
            };
            // Unsafe! cast box to usize
            let param: usize = unsafe { transmute(boxed_f) };
            let callback = timer_closure_runner::<F> as TimerCallBackFnType;
            let r = Timer::new_from_heap(name, period_ticks, auto_reload, callback, param, j).map(
                |mut ptr| unsafe {
                    let timer = ptr.as_mut_no_logging();
                    timer.list_node.borrow_mut_no_logging().value.timer = Some(ptr);
                    ptr
                },
            );
            r
        })
    }
}

#[derive(Eq, PartialEq, Debug, Copy, Clone)]
pub enum TimerErr {
    NoTimerDaemon,
    CmdQueueBusy,
}

fn timer_control(
    cmd: TimerCmd,
    opaque: Time,
    timer: PMPtr<Timer>,
    wait_ticks: Time,
) -> Result<(), TimerErr> {
    let cmd_queue = match TIME_MANAGER.get_cmd_queue_ptr() {
        Some(q) => q,
        None => return Err(TimerErr::NoTimerDaemon),
    };

    let msg = TimerMessage {
        cmd,
        timer: Some(timer),
        opaque,
    };

    let buf_ptr = cast_to_u8_ptr(&msg);
    if let Err(_) = queue_send_back(cmd_queue, buf_ptr, wait_ticks) {
        Err(TimerErr::CmdQueueBusy)
    } else {
        Ok(())
    }
}

pub fn start_timer(timer: PMPtr<Timer>, wait_ticks: Time) -> Result<(), TimerErr> {
    let now = TIME_MANAGER.get_ticks();
    timer_control(TimerCmd::Start, now, timer, wait_ticks)
}

pub fn reset_timer(timer: PMPtr<Timer>, wait_ticks: Time) -> Result<(), TimerErr> {
    let now = TIME_MANAGER.get_ticks();
    timer_control(TimerCmd::Reset, now, timer, wait_ticks)
}

pub fn delete_timer(timer: PMPtr<Timer>, wait_ticks: Time) -> Result<(), TimerErr> {
    timer_control(TimerCmd::Delete, 0, timer, wait_ticks)
}

pub fn change_timer_period(
    timer: PMPtr<Timer>,
    new_period: Time,
    wait_ticks: Time,
) -> Result<(), TimerErr> {
    timer_control(TimerCmd::SetPeriod, new_period, timer, wait_ticks)
}

pub fn stop_timer(timer: PMPtr<Timer>, wait_ticks: Time) -> Result<(), TimerErr> {
    timer_control(TimerCmd::Stop, 0, timer, wait_ticks)
}

pub fn get_time() -> Time {
    TIME_MANAGER.get_ticks()
}

pub fn daemon_timer_task() {
    // loop to execute timer cmds...
    #[cfg(any(bench_task = "sense", bench_task = "sense_base"))]
    crate::benchmarks::benchmark_start();
    loop {
        TIME_MANAGER.process_expired_timer();
        TIME_MANAGER.process_receive_timer_cmd();
    }
}

#[cfg(test)]
pub fn reset_timer_active_list() {
    unsafe {
        *get_timer_active_list() = SortedPList::new();
    }
}

#[cfg(test)]
pub fn reset_timer_manager() {
    reset_timer_active_list();
    TIME_MANAGER.next_expiry_time.set(MAX_DELAY_TIME);
    TIME_MANAGER.next_unblock_time.set(MAX_DELAY_TIME);
    TIME_MANAGER.tick_counter.set(0);
    unsafe { *TIME_MANAGER.timer_cmd_queue.get() = None };
}

#[cfg(test)]
mod test {

    use crate::{
        list::ListTxOpCode,
        os_print, recover,
        task::{current, mock_task_switch},
        test::mock_boot_with_timer_daemon,
    };

    use super::*;

    const NO_CRASH: usize = 10000;

    fn timer_callback(_x: usize, _j: JournalHandle) {
        os_print!("Hello from timer callback");
    }

    fn new_timer(reload: bool, exp: Time) -> Timer {
        Timer::new("test", 0, reload, timer_callback, exp, 0)
    }
    #[test]
    fn test_crashed_process_expired_timer() {
        fn run(cp0: usize, cp1: usize, cp2: usize, reload: bool) {
            reset_timer_manager();
            mock_boot_with_timer_daemon(0);
            let tl = unsafe { get_timer_active_list() };
            let tl_ptr = tl.as_pm_ptr();
            let mut t = new_timer(reload, 0);
            let t_ptr = unsafe { PMPtr::new(&t as *const Timer as *mut Timer) };
            unsafe {
                t.list_node.borrow_mut_no_logging().value.timer = Some(t_ptr);
            }
            let n = t.get_list_node().as_ref();
            list::atomic_roll_forward_remove_reinsert_into_activelist(t.get_list_node());

            // process expired
            os_print!("Run...");
            TIME_MANAGER.crashed_process_expired_timer(cp0, cp1, cp2);

            // reboot
            crate::test::mock_reboot();

            // recover
            os_print!("Recover...");
            recover::recover();

            // check
            assert_eq!(crate::test::user_tx_ptr(), 0);
            assert_eq!(crate::test::user_tx_tail(), 0);
            assert_eq!(current().get_mut_user_tx().get_nesting_level(), 0);
            // re-execute
            os_print!("Re-execute...");
            TIME_MANAGER.crashed_process_expired_timer(NO_CRASH, NO_CRASH, NO_CRASH);

            // more check
            if !reload {
                assert!(tl.len() == 0);
                assert!(tl.head() == None);
                assert!(n.list == None);
            } else {
                assert!(tl.len() == 1);
                assert!(tl.head() == Some(t.get_list_node()));
                assert!(n.list == Some(tl_ptr));
                assert!(n.prev == None);
                assert!(n.next == None);
            }

            assert_eq!(crate::test::user_tx_ptr(), 0);
            assert_eq!(crate::test::user_tx_tail(), 0);
            assert_eq!(current().get_mut_user_tx().get_nesting_level(), 0);
            // assert_eq!(ListTxOpLog::get_timer_list_tx_op_log().get_tx_op(), ListTxOpCode::ActiveListTxCommitted);
        }

        for cp0 in 0..5 {
            for cp1 in 0..5 {
                for cp2 in 0..8 {
                    os_print!(
                        "Testing cp0: {}, cp1: {}, cp2: {}, reload = false",
                        cp0,
                        cp1,
                        cp2
                    );
                    run(cp0, cp1, cp2, false);
                    os_print!(
                        "Testing cp0: {}, cp1: {}, cp2: {}, reload = true",
                        cp0,
                        cp1,
                        cp2
                    );
                    run(cp0, cp1, cp2, true);
                }
            }
        }
    }

    #[test]
    fn test_crashed_process_timer_cmd() {
        fn run(cp0: usize, cp1: usize, cp2: usize, cp_syscall: usize, op: &str) {
            reset_timer_manager();
            mock_boot_with_timer_daemon(1);
            let tl = unsafe { get_timer_active_list() };
            let tl_ptr = tl.as_pm_ptr();
            let mut t = new_timer(true, 0);
            let t_ptr = unsafe { PMPtr::new(&t as *const Timer as *mut Timer) };
            unsafe {
                t.list_node.borrow_mut_no_logging().value.timer = Some(t_ptr);
            }
            let n = t.get_list_node().as_ref();

            assert_eq!(current().get_name(), "Test1");
            if op == "start timer" {
                start_timer(t_ptr, 0);
            } else if op == "stop timer" {
                list::atomic_roll_forward_remove_reinsert_into_activelist(t.get_list_node());
                stop_timer(t_ptr, 0);
            }
            mock_task_switch();
            assert_eq!(current().get_name(), "timer daemon");

            os_print!("Run...");
            TIME_MANAGER.crashed_process_receive_timer_cmd(cp0, cp1, cp2, cp_syscall);

            // reboot
            os_print!("Rebooting...");
            crate::test::mock_reboot();

            // recover
            os_print!("Recover...");
            recover::recover();

            // check
            assert_eq!(crate::test::user_tx_ptr(), 0);
            assert_eq!(crate::test::user_tx_tail(), 0);
            assert_eq!(current().get_mut_user_tx().get_nesting_level(), 0);

            // re-execute
            os_print!("Re-execute...");
            TIME_MANAGER.crashed_process_receive_timer_cmd(NO_CRASH, NO_CRASH, NO_CRASH, NO_CRASH);

            // more check
            if op == "stop timer" {
                assert!(tl.len() == 0);
                assert!(tl.head() == None);
                assert!(n.list == None);
            } else {
                assert!(tl.len() == 1);
                assert!(tl.head() == Some(t.get_list_node()));
                assert!(n.list == Some(tl_ptr));
                assert!(n.prev == None);
                assert!(n.next == None);
            }

            assert_eq!(crate::test::user_tx_ptr(), 0);
            assert_eq!(crate::test::user_tx_tail(), 0);
            assert_eq!(crate::test::sys_tx_tail(), 0);
            assert_eq!(crate::test::sys_tx_tail(), 0);
            assert_eq!(current().get_mut_user_tx().get_nesting_level(), 0);
            // assert_eq!(ListTxOpLog::get_timer_list_tx_op_log().get_tx_op(), ListTxOpCode::ActiveListTxCommitted);
        }

        for cp0 in 0..5 {
            for cp1 in 0..5 {
                for cp2 in 0..8 {
                    os_print!(
                        "Testing cp0: {}, cp1: {}, cp2: {}, start timer",
                        cp0,
                        cp1,
                        cp2
                    );
                    run(cp0, cp1, cp2, NO_CRASH, "start timer");
                    os_print!(
                        "Testing cp0: {}, cp1: {}, cp2: {}, stop timer",
                        cp0,
                        cp1,
                        cp2
                    );
                    run(cp0, cp1, cp2, NO_CRASH, "stop timer");
                }
            }
        }
        for sys_cp in 0..4 {
            os_print!("Testing sys_cp: {}, start timer", sys_cp);
            run(1, NO_CRASH, NO_CRASH, sys_cp, "start timer");
            os_print!("Testing sys_cp: {}, stop timer", sys_cp);
            run(1, NO_CRASH, NO_CRASH, sys_cp, "stop timer");
        }
    }
}
