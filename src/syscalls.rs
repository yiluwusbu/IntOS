use crate::arch::ARCH_ALIGN;
use crate::event_group::{self, EventBits, EventGroup, EventGroupHandle};
use crate::heap::MemStat;
use crate::marker::PSafe;
use crate::pmem::{JournalHandle, PMPtr};
use crate::queue::{self, Queue, QueueErr};
use crate::semaphore::{self, Semaphore};
use crate::task::{self, current, task_enter_kernel, task_exit_kernel, ErrorCode, TaskHandle};
use crate::time::{self, Time, Timer, TimerCallBackFnType, TimerErr};
use crate::user::pbox::{PBox, PRef, PRefRW, Ptr};
use crate::util::{align_up, cast_to_u8_ptr};
use crate::vec::PArray;
use crate::{debug_print, heap, os_print, transaction};
use core::borrow::BorrowMut;
use core::marker::PhantomData;
use core::mem::{forget, MaybeUninit};
use core::ptr::NonNull;

const N_SYSCALL_RET_CACHE_ENTRY: usize = 16;

#[cfg(not(sram_baseline))]
const SYSCALL_REPLAY_CACHE_SZ: usize = core::mem::size_of::<usize>() * N_SYSCALL_RET_CACHE_ENTRY;
#[cfg(sram_baseline)]
const SYSCALL_REPLAY_CACHE_SZ: usize = 0;

#[derive(Clone, Copy)]
pub struct SyscallToken {
    opaque: (),
}

impl SyscallToken {
    pub unsafe fn new() -> Self {
        SyscallToken { opaque: () }
    }
}

pub struct SyscallReplayCache {
    ptr: usize,
    tail: usize,
    cache: [u8; SYSCALL_REPLAY_CACHE_SZ],
}

impl SyscallReplayCache {
    pub const fn new() -> Self {
        Self {
            ptr: 0,
            tail: 0,
            cache: [0; SYSCALL_REPLAY_CACHE_SZ],
        }
    }

    pub fn get_ptr(&self) -> usize {
        self.ptr
    }

    pub fn get_tail(&self) -> usize {
        self.tail
    }

    pub fn add_entry_start<T>(&mut self, entry: &T) {
        // TODO: don't just panic here
        let aligned_sz = align_up(core::mem::size_of::<T>(), ARCH_ALIGN);
        assert!(self.tail + aligned_sz <= SYSCALL_REPLAY_CACHE_SZ);
        unsafe {
            core::ptr::copy_nonoverlapping(
                entry as *const T as *const u8,
                &self.cache[self.tail] as *const u8 as *mut u8,
                aligned_sz,
            );
        }
        self.tail += aligned_sz;
    }

    pub fn add_empty_entry_start(&mut self) {
        self.tail += ARCH_ALIGN;
    }

    // increment pointer value
    #[inline(always)]
    pub fn add_entry_commit(&mut self) {
        self.ptr = self.tail;
    }

    pub fn get_entry<T>(&mut self) -> Result<T, ()> {
        if self.ptr < self.tail {
            let ret = unsafe { core::ptr::read(&self.cache[self.ptr] as *const u8 as *const T) };
            self.ptr += align_up(core::mem::size_of::<T>(), ARCH_ALIGN);
            Ok(ret)
        } else {
            Err(())
        }
    }

    pub fn get_empty_entry(&mut self) -> Result<(), ()> {
        if self.ptr < self.tail {
            self.ptr += ARCH_ALIGN;
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn restart(&mut self) {
        self.ptr = 0;
    }

    pub fn reset(&mut self) {
        self.tail = 0;
        // self.ptr = 0;
    }
}

#[inline(always)]
fn pre_syscall_hook() {
    debug_assert!(
        unsafe { crate::task::is_scheduler_started() },
        "Can't have syscall when booting"
    );
    #[cfg(not(test))]
    task_enter_kernel(task::current());
}

#[inline(always)]
fn post_syscall_hook() {
    #[cfg(not(test))]
    task_exit_kernel(task::current());
}

#[cfg(feature = "crash_safe")]
macro_rules! syscall_begin_outside_tx {
    () => {
        pre_syscall_hook();
    };
}

#[cfg(feature = "crash_safe")]
macro_rules! syscall_end_outside_tx {
    () => {
        task::current().get_sys_tx_cache().commit_reset_tail();
        task::current().reset_list_transaction();
        post_syscall_hook();
    };
}

#[cfg(feature = "crash_safe")]
macro_rules! syscall_begin {
    ($t: ident) => {
        let cur_task = task::current();
        if let Ok(ret) = cur_task.get_syscall_replay_cache().get_entry() {
            debug_print!("bypassing syscall {} ...", stringify!($t));
            debug_assert!(
                cur_task.in_recovery_mode(),
                "Can't bypass syscall when not recovering"
            );
            return ret;
        }
        pre_syscall_hook();
        cur_task.restart_syscall_tx_cache();
    };

    ($t: ident, $eplilogue: block) => {
        let cur_task = task::current();
        if let Ok(ret) = cur_task.get_syscall_replay_cache().get_entry() {
            debug_print!("bypassing syscall {} ...", stringify!($t));
            debug_assert!(
                cur_task.in_recovery_mode(),
                "Can't bypass syscall when not recovering"
            );
            $eplilogue;
            return ret;
        }
        pre_syscall_hook();
        cur_task.restart_syscall_tx_cache();
    };

    (noret, $t: ident) => {
        let cur_task = task::current();
        if let Ok(_) = cur_task.get_syscall_replay_cache().get_empty_entry() {
            debug_print!("bypassing syscall {} ...", stringify!($t));
            debug_assert!(
                cur_task.in_recovery_mode(),
                "Can't bypass syscall when not recovering"
            );
            return;
        }
        pre_syscall_hook();
        cur_task.restart_syscall_tx_cache();
    };
}

#[cfg(not(test))]
#[cfg(feature = "crash_safe")]
macro_rules! syscall_end {
    ($t: ident, $r: expr) => {
        // First cache the result,
        let cur_task = task::current();
        cur_task.get_syscall_replay_cache().add_entry_start(&($r));
        task::current().get_sys_tx_cache().commit_reset_tail();
        cur_task.reset_list_transaction();
        cur_task.get_syscall_replay_cache().add_entry_commit();
        post_syscall_hook();
        return $r;
    };

    ($t: ident, $r: expr, $eplilogue: block) => {
        // First cache the result,
        let cur_task = task::current();
        cur_task.get_syscall_replay_cache().add_entry_start(&($r));
        task::current().get_sys_tx_cache().commit_reset_tail();
        cur_task.reset_list_transaction();
        cur_task.get_syscall_replay_cache().add_entry_commit();
        post_syscall_hook();
        $eplilogue;
        return $r;
    };

    ($eplilogue: block) => {
        let cur_task = task::current();
        cur_task.get_syscall_replay_cache().add_empty_entry_start();
        cur_task.get_sys_tx_cache().commit_reset_tail();
        cur_task.reset_list_transaction();
        cur_task.get_syscall_replay_cache().add_entry_commit();
        post_syscall_hook();
        $eplilogue;
        return;
    };

    () => {
        // First cache the result,
        let cur_task = task::current();
        cur_task.get_syscall_replay_cache().add_empty_entry_start();
        cur_task.get_sys_tx_cache().commit_reset_tail();
        cur_task.reset_list_transaction();
        cur_task.get_syscall_replay_cache().add_entry_commit();
        post_syscall_hook();
        return;
    };
}

/*------------ crash-unsafe version syscall macros --------------------- */
#[cfg(not(feature = "crash_safe"))]
macro_rules! syscall_begin_outside_tx {
    () => {
        pre_syscall_hook();
    };
}
#[cfg(not(feature = "crash_safe"))]
macro_rules! syscall_end_outside_tx {
    () => {
        post_syscall_hook();
    };
}
#[cfg(not(feature = "crash_safe"))]
macro_rules! syscall_begin {
    ($t: ident) => {
        pre_syscall_hook();
    };

    ($t: ident, $eplilogue: block) => {
        pre_syscall_hook();
    };

    (noret, $t: ident) => {
        pre_syscall_hook();
    };

    () => {
        pre_syscall_hook();
    };
}
#[cfg(not(feature = "crash_safe"))]
macro_rules! syscall_end {
    ($t: ident, $r: expr) => {
        post_syscall_hook();
        return $r;
    };

    ($t: ident, $r: expr, $eplilogue: block) => {
        post_syscall_hook();
        $eplilogue;
        return $r;
    };

    ($eplilogue: block) => {
        post_syscall_hook();
        $eplilogue;
        return;
    };
    () => {
        post_syscall_hook();
        return;
    };
}
/* --------------------Testing Interfaces with crash injection ------------------------------ */
#[cfg(test)]
static mut SYSCALL_END_CRASH_POINT: usize = 10000;

#[cfg(test)]
pub const SYSCALL_END_NUM_CRASH_POINT: usize = 4;

#[cfg(test)]
pub fn set_syscall_end_crash_point(cp: usize) {
    unsafe {
        SYSCALL_END_CRASH_POINT = cp;
    }
}

#[cfg(test)]
pub fn syscall_end_crash_point() -> usize {
    unsafe { SYSCALL_END_CRASH_POINT }
}

#[cfg(test)]
#[cfg(feature = "crash_safe")]
macro_rules! syscall_end {
    ($t: ident, $r: expr) => {
        // First cache the result,
        let cur_task = task::current();
        if syscall_end_crash_point() == 0 {
            return $r;
        }
        cur_task.get_syscall_replay_cache().add_entry_start(&($r));
        if syscall_end_crash_point() == 1 {
            return $r;
        }
        task::current().get_sys_tx_cache().commit_reset_tail();
        if syscall_end_crash_point() == 2 {
            return $r;
        }
        cur_task.reset_list_transaction();
        if syscall_end_crash_point() == 3 {
            return $r;
        }
        cur_task.get_syscall_replay_cache().add_entry_commit();
        post_syscall_hook();
        return $r;
    };
    () => {
        // First cache the result,
        let cur_task = task::current();
        if syscall_end_crash_point() == 0 {
            return;
        }
        cur_task.get_syscall_replay_cache().add_empty_entry_start();
        if syscall_end_crash_point() == 1 {
            return;
        }
        cur_task.get_sys_tx_cache().commit_reset_tail();
        if syscall_end_crash_point() == 2 {
            return;
        }
        cur_task.reset_list_transaction();
        if syscall_end_crash_point() == 3 {
            return;
        }
        cur_task.get_syscall_replay_cache().add_entry_commit();
        post_syscall_hook();
        return;
    };
}

// Task syscalls
pub unsafe fn sys_create_task_unsafe(
    name: &'static str,
    prio: usize,
    func: usize,
    param: usize,
    pm_heap_sz: usize,
    _: SyscallToken,
) -> Result<TaskHandle, ErrorCode> {
    syscall_begin!(task_create);
    let ret = task::create_task_static(name, prio, func, param, pm_heap_sz);
    syscall_end!(task_create, ret);
}

pub fn sys_create_task_custom<T>(
    name: &'static str,
    prio: usize,
    func: fn(T),
    param: T,
    pm_heap_sz: usize,
    _: SyscallToken,
) -> Result<TaskHandle, ErrorCode>
where
    T: Send + 'static,
{
    assert!(core::mem::size_of::<T>() == core::mem::size_of::<usize>());
    let param_usize = unsafe { core::mem::transmute_copy::<T, usize>(&param) };
    // We should not run task's param destructor
    forget(param);
    // assert_sz!(T, core::mem::size_of::<usize>());
    syscall_begin!(task_create);
    let ret = task::create_task_static(name, prio, func as usize, param_usize, pm_heap_sz);
    syscall_end!(task_create, ret);
}

pub fn sys_create_task<T>(
    name: &'static str,
    prio: usize,
    func: fn(T),
    param: T,
    t: SyscallToken,
) -> Result<TaskHandle, ErrorCode>
where
    T: Send + 'static,
{
    sys_create_task_custom(name, prio, func, param, heap::PM_HEAP_SIZE_PER_TASK, t)
}

pub fn sys_task_delay_in_tx(nticks: Time, _: SyscallToken) {
    syscall_begin!(noret, task_delay);
    task::task_delay(nticks, false);
    syscall_end!({
        crate::task::task_yield();
    });
}

pub fn sys_task_delay(nticks: Time) {
    syscall_begin_outside_tx!();
    task::task_delay(nticks, true);
    syscall_end_outside_tx!();
}

pub fn sys_yield() {
    task::task_yield();
}

// Queue syscalls

pub struct QueueHandle<T> {
    ptr: PMPtr<Queue>,
    phantom: PhantomData<T>,
}

impl<T> Clone for QueueHandle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for QueueHandle<T> {}

unsafe impl<T> Send for QueueHandle<T> {}
unsafe impl<T> Sync for QueueHandle<T> {}

impl<T> QueueHandle<T> {
    pub unsafe fn new(ptr: PMPtr<Queue>) -> Self {
        Self {
            ptr,
            phantom: PhantomData,
        }
    }
}

pub fn sys_queue_create<T>(length: usize, _: SyscallToken) -> Option<QueueHandle<T>> {
    syscall_begin!(queue_create);
    let ret = queue::queue_create(length, core::mem::size_of::<T>());
    let ret = ret.map(|ptr| QueueHandle {
        ptr,
        phantom: PhantomData,
    });
    syscall_end!(queue_create, ret);
}

pub fn sys_queue_send_back<T>(
    q: QueueHandle<T>,
    item: T,
    wait_ticks: Time,
    _: SyscallToken,
) -> Result<(), QueueErr> {
    syscall_begin!(queue, { forget(item) });
    let ret = queue::queue_send_back(q.ptr, cast_to_u8_ptr(&item), wait_ticks);
    forget(item);
    syscall_end!(queue, ret);
}

pub fn sys_queue_send_front<T>(
    q: QueueHandle<T>,
    item: T,
    wait_ticks: Time,
    _: SyscallToken,
) -> Result<(), QueueErr> {
    syscall_begin!(queue, { forget(item) });
    let ret = queue::queue_send_front(q.ptr, cast_to_u8_ptr(&item), wait_ticks);
    forget(item);
    syscall_end!(queue, ret);
}

pub fn sys_queue_receive<T>(
    q: QueueHandle<T>,
    wait_ticks: Time,
    _: SyscallToken,
) -> Result<T, QueueErr> {
    syscall_begin!(queue);
    let ret = queue::queue_receive(q.ptr, wait_ticks);
    syscall_end!(queue, ret);
}

pub type SemaphoreHandle = PMPtr<Semaphore>;

// Semaphore syscalls
pub fn sys_create_semaphore(n: usize, _: SyscallToken) -> Option<SemaphoreHandle> {
    syscall_begin!(sema_create);
    let ret = semaphore::create_semaphore(n);
    syscall_end!(sema_create, ret);
}

pub fn sys_semaphore_take(sem: SemaphoreHandle, wait_ticks: Time) -> Result<(), ()> {
    syscall_begin_outside_tx!();
    let r = semaphore::semaphore_take(sem, wait_ticks);
    syscall_end_outside_tx!();
    r
}

pub fn sys_semaphore_give(sem: SemaphoreHandle) {
    syscall_begin_outside_tx!();
    semaphore::semaphore_give(sem);
    syscall_end_outside_tx!();
}

// Event group syscalls
pub fn sys_event_group_create(_: SyscallToken) -> Option<EventGroupHandle> {
    syscall_begin!(event_grp_create);
    let ret = event_group::create_event_group();
    syscall_end!(event_grp_create, ret);
}

pub fn sys_event_group_wait(
    event_grp: EventGroupHandle,
    bits_to_wait_for: EventBits,
    clear_on_exit: bool,
    wait_for_all_bits: bool,
    wait_ticks: Time,
    _: SyscallToken,
) -> Result<EventBits, EventBits> {
    syscall_begin!(event_grp, {
        crate::task::current().reset_block_item_value();
    });
    let ret = event_group::event_group_wait(
        event_grp,
        bits_to_wait_for,
        clear_on_exit,
        wait_for_all_bits,
        wait_ticks,
    );
    syscall_end!(event_grp, ret, {
        crate::task::current().reset_block_item_value();
    });
}

pub fn sys_event_group_set(event_grp: EventGroupHandle, bits_to_set: EventBits, _: SyscallToken) {
    syscall_begin!(noret, event_grp_set);
    event_group::event_group_set(event_grp, bits_to_set);
    syscall_end!();
}

pub fn sys_event_group_sync(
    event_grp: EventGroupHandle,
    bits_to_set: EventBits,
    bits_to_wait_for: EventBits,
    wait_ticks: Time,
    _: SyscallToken,
) -> Result<EventBits, EventBits> {
    syscall_begin!(event_grp);
    let ret = event_group::event_group_sync(event_grp, bits_to_set, bits_to_wait_for, wait_ticks);
    syscall_end!(event_grp, ret);
}

pub type TimerHandle = PMPtr<Timer>;

pub fn sys_timer_create<T>(
    name: &'static str,
    period_ticks: Time,
    auto_reload: bool,
    callback: fn(T, JournalHandle),
    param: T,
    _: SyscallToken,
) -> Option<TimerHandle>
where
    T: Send + 'static,
{
    assert!(core::mem::size_of::<T>() <= core::mem::size_of::<usize>());
    let param_usize = unsafe { core::mem::transmute_copy::<T, usize>(&param) };
    // We should not run task's param destructor
    forget(param);
    let callback_usize: TimerCallBackFnType = unsafe { core::mem::transmute(callback) };
    syscall_begin!(timer_create);
    let ret = time::create_timer(name, period_ticks, auto_reload, callback_usize, param_usize);
    syscall_end!(timer_create, ret);
}

pub fn sys_timer_create_with_closure<F>(
    name: &'static str,
    period_ticks: Time,
    auto_reload: bool,
    callback: F,
    _: SyscallToken,
) -> Option<TimerHandle>
where
    F: FnMut(JournalHandle) + Send + 'static,
{
    syscall_begin!(timer_create);
    let ret = time::create_timer_with_closure(name, period_ticks, auto_reload, callback);
    syscall_end!(timer_create, ret);
}

pub fn sys_start_timer(
    timer: TimerHandle,
    wait_ticks: Time,
    _: SyscallToken,
) -> Result<(), TimerErr> {
    syscall_begin!(timer);
    let ret = time::start_timer(timer, wait_ticks);
    syscall_end!(timer, ret);
}

pub fn sys_reset_timer(
    timer: TimerHandle,
    wait_ticks: Time,
    _: SyscallToken,
) -> Result<(), TimerErr> {
    syscall_begin!(timer);
    let ret = time::reset_timer(timer, wait_ticks);
    syscall_end!(timer, ret);
}

pub fn sys_delete_timer(
    timer: TimerHandle,
    wait_ticks: Time,
    _: SyscallToken,
) -> Result<(), TimerErr> {
    syscall_begin!(timer);
    let ret = time::delete_timer(timer, wait_ticks);
    syscall_end!(timer, ret);
}

pub fn sys_change_timer_period(
    timer: TimerHandle,
    new_period: Time,
    wait_ticks: Time,
    _: SyscallToken,
) -> Result<(), TimerErr> {
    syscall_begin!(timer);
    let ret = time::change_timer_period(timer, new_period, wait_ticks);
    syscall_end!(timer, ret);
}

pub fn sys_stop_timer(
    timer: TimerHandle,
    wait_ticks: Time,
    _: SyscallToken,
) -> Result<(), TimerErr> {
    syscall_begin!(timer);
    let ret = time::stop_timer(timer, wait_ticks);
    syscall_end!(timer, ret);
}

pub fn sys_get_time(t: SyscallToken) -> Time {
    syscall_begin!(time_get);
    let ret = time::get_time();
    syscall_end!(time_get, ret);
}

pub fn sys_get_time_out_of_tx() -> Time {
    time::get_time()
}

pub fn sys_get_pmem_stat() -> MemStat {
    current().get_pm_heap_stat()
}

pub unsafe fn sys_palloc_relaxed<T>(x: T, _: SyscallToken) -> Option<PMPtr<T>> {
    syscall_begin!(palloc);
    // crate::task_print!("Allocating: {}, size = {}", core::any::type_name::<T>(), core::mem::size_of::<T>());
    let ret = transaction::run_relaxed(move |j| heap::pm_new_relaxed(x, j));
    syscall_end!(palloc, ret);
}

pub fn sys_palloc<T: PSafe>(x: T, t: SyscallToken) -> Option<PMPtr<T>> {
    unsafe { sys_palloc_relaxed(x, t) }
}

pub unsafe fn sys_palloc_uninit<T: PSafe>(_: SyscallToken) -> Option<PMPtr<T>> {
    syscall_begin!(palloc);
    let ret = transaction::run(|j| unsafe { heap::palloc::<T>(j) });
    syscall_end!(palloc, ret);
}

pub unsafe fn sys_palloc_array<T: PSafe>(size: usize, _: SyscallToken) -> Option<NonNull<T>> {
    syscall_begin!(palloc_array);
    let ret = transaction::run_relaxed(|j| unsafe { heap::palloc_array::<T>(size, j) });
    syscall_end!(palloc_array, ret);
}

pub unsafe fn sys_pfree_array<T>(ptr: NonNull<T>, size: usize, _: SyscallToken) {
    syscall_begin!(noret, pfree_array);
    transaction::run(|j| {
        let ret = unsafe { heap::pfree_array(ptr.as_ptr(), size, j) };
    });
    syscall_end!();
}

pub unsafe fn sys_pfree<T>(ptr: PMPtr<T>, _: SyscallToken) {
    syscall_begin!(noret, pfree);
    transaction::run(|j| {
        unsafe { heap::pfree(ptr.as_ptr(), j) };
    });
    syscall_end!();
}
