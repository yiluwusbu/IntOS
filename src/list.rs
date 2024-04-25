use crate::time;
use crate::{
    critical::{self, CriticalSection},
    debug_print, list_dbg_print, os_print,
    pmem::{JournalHandle, PMPtr, PVolatilePtr},
    task::{
        current, get_current_task_ptr, get_delay_list, get_sched_list, BlockedListItem,
        SchedListItem, Task,
    },
    time::{Time, TimerListItem},
    transaction::Transaction,
    util::compiler_pm_fence,
};
use core::{
    fmt::{self, Display},
    mem::transmute,
};

type PLink<T> = Option<PMPtr<Node<T>>>;
type PListLink<T> = Option<PMPtr<PList<T>>>;

// pub static LIST_OP_LOG: ListOpLog = ListOpLog::new();

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Node<T> {
    pub prev: PLink<T>,
    pub next: PLink<T>,
    pub list: PListLink<T>,
    pub value: T,
}

impl<T> Node<T> {
    pub fn new(value: T) -> Self {
        Self {
            prev: None,
            next: None,
            value,
            list: None,
        }
    }
}

#[cfg(test)]
static mut LIST_CRASH_POINT: usize = 0;

#[cfg(test)]
pub fn set_list_crash_point(crash_point: usize) {
    unsafe { LIST_CRASH_POINT = crash_point };
}

#[cfg(test)]
pub fn get_list_crash_point() -> usize {
    unsafe { LIST_CRASH_POINT }
}

macro_rules! crash_point {
    ($e: expr) => {
        #[cfg(test)]
        {
            if get_list_crash_point() == $e {
                return;
            }
        }
    };
    ($e: expr, $r: expr) => {
        #[cfg(test)]
        {
            if get_list_crash_point() == $e {
                return $r;
            }
        }
    };
}

#[link_section = ".pmem"]
pub static mut LIST_TX_OP_LOG: ListTxOpLog = ListTxOpLog::new();
#[link_section = ".pmem"]
pub static mut TIMER_LIST_TX_OP_LOG: ListTxOpLog = ListTxOpLog::new();

#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ListOpCode {
    Invalid = 0,
    InsertBeforeCursor = 1,
    InsertFront = 2,
    InsertSortedWaitList = 3,
    InsertSortedDelayList = 4,
    PopFront = 5,
    Remove = 6,
    ReadyListNext = 7,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ListTxOpCode {
    Invalid,
    WaitListPopRemoval,
    WaitListInsert,
    WaitListRemoval,
    UnsortedWaitListInsert,
    DelayListRemoval,
    DelayListInsert,
    ReadyListInsert,
    ActiveListRemove,
    ActiveListRemoveReInsert,
    ActiveListPopReInsert,
    ActiveListPop, // not used
    ActiveListTxCommitted,
    ReadyListNext,
}
///     micro_op_old_len
///    | opcode | old len |
///     4 bits    12 bits

pub struct ListTxOpLog {
    op: ListTxOpCode,
    micro_op_old_len: usize,
    block_node_ptr: Option<PMPtr<Node<BlockedListItem>>>,
    task_ptr: Option<PMPtr<Task>>,
    wait_list_ptr: Option<PMPtr<SortedPList<BlockedListItem>>>,
}

unsafe impl Sync for ListTxOpLog {}

fn pre_list_op_hook() {
    #[cfg(feature = "profile_log")]
    {
        crate::pmem::add_klog_sz(core::mem::size_of::<ListTxOpLog>());
    }
}

const OP_CODE_SHIFT: usize = 12;

impl ListTxOpLog {
    #[inline(always)]
    pub fn get_list_tx_op_log() -> &'static mut Self {
        unsafe { &mut LIST_TX_OP_LOG }
    }

    #[inline(always)]
    pub fn get_timer_list_tx_op_log() -> &'static mut Self {
        unsafe { &mut TIMER_LIST_TX_OP_LOG }
    }

    pub const fn new() -> Self {
        Self {
            op: ListTxOpCode::Invalid,
            micro_op_old_len: 0,
            block_node_ptr: None,
            task_ptr: None,
            wait_list_ptr: None,
        }
    }

    #[cfg(feature = "crash_safe")]
    #[inline(always)]
    pub fn set_micro_op_old_len(&mut self, op: ListOpCode, old_len: usize) {
        self.micro_op_old_len = ((op as usize) << OP_CODE_SHIFT) | old_len;
    }

    #[cfg(not(feature = "crash_safe"))]
    #[inline(always)]
    pub fn set_micro_op_old_len(&mut self, _op: ListOpCode, _old_len: usize) {}

    #[cfg(feature = "crash_safe")]
    #[inline(always)]
    pub fn set_tx_op(&mut self, op: ListTxOpCode) {
        self.op = op;
    }

    #[cfg(not(feature = "crash_safe"))]
    #[inline(always)]
    pub fn set_tx_op(&mut self, _op: ListTxOpCode) {}

    #[inline(always)]
    pub fn get_tx_op(&self) -> ListTxOpCode {
        self.op
    }

    #[inline(always)]
    pub fn get_micro_op(&self) -> ListOpCode {
        unsafe { transmute((self.micro_op_old_len >> OP_CODE_SHIFT)) }
    }

    #[inline(always)]
    pub fn get_old_len(&self) -> usize {
        self.micro_op_old_len & 0xfff
    }

    #[cfg(feature = "crash_safe")]
    #[inline(always)]
    pub fn commit(&mut self) {
        // set micro operation to invalid
        self.micro_op_old_len = 0;
        compiler_pm_fence();
        // list TX  done
        current().complete_list_transaction();
        compiler_pm_fence();
        // invalidate
        self.set_tx_op(ListTxOpCode::Invalid);
    }

    #[cfg(not(feature = "crash_safe"))]
    #[inline(always)]
    pub fn commit(&mut self) {}

    #[cfg(feature = "crash_safe")]
    #[inline(always)]
    pub fn commit_activelist(&mut self) {
        self.set_tx_op(ListTxOpCode::ActiveListTxCommitted);
        compiler_pm_fence();
        self.micro_op_old_len = 0;
    }

    #[cfg(not(feature = "crash_safe"))]
    #[inline(always)]
    pub fn commit_activelist(&mut self) {}

    #[cfg(feature = "crash_safe")]
    #[inline(always)]
    pub fn invalidate_activelist(&mut self) {
        // invalidate
        self.set_tx_op(ListTxOpCode::Invalid);
    }

    #[cfg(not(feature = "crash_safe"))]
    #[inline(always)]
    pub fn invalidate_activelist(&mut self) {}

    #[cfg(feature = "crash_safe")]
    #[inline(always)]
    pub fn invalidate(&mut self) {
        // set micro operation to invalid
        self.micro_op_old_len = 0;
        compiler_pm_fence();
        self.op = ListTxOpCode::Invalid;
    }

    #[cfg(not(feature = "crash_safe"))]
    #[inline(always)]
    pub fn invalidate(&mut self) {}

    #[cfg(feature = "crash_safe")]
    #[inline(always)]
    pub fn set_block_node_ptr<T>(&mut self, ptr: Option<PMPtr<Node<T>>>) {
        self.block_node_ptr = unsafe { transmute(ptr) };
    }

    #[cfg(not(feature = "crash_safe"))]
    #[inline(always)]
    pub fn set_block_node_ptr<T>(&mut self, _ptr: Option<PMPtr<Node<T>>>) {}

    #[inline(always)]
    pub fn get_block_node_ptr<T>(&self) -> Option<PMPtr<Node<T>>> {
        unsafe { transmute(self.block_node_ptr) }
    }

    #[cfg(feature = "crash_safe")]
    #[inline(always)]
    pub fn set_task_ptr(&mut self, ptr: Option<PMPtr<Task>>) {
        self.task_ptr = ptr;
    }

    #[cfg(not(feature = "crash_safe"))]
    #[inline(always)]
    pub fn set_task_ptr(&mut self, _ptr: Option<PMPtr<Task>>) {}

    #[inline(always)]
    pub fn get_task_ptr(&self) -> Option<PMPtr<Task>> {
        self.task_ptr
    }

    #[cfg(feature = "crash_safe")]
    #[inline(always)]
    pub fn set_wait_list_ptr(&mut self, ptr: Option<PMPtr<SortedPList<BlockedListItem>>>) {
        self.wait_list_ptr = ptr;
    }

    #[cfg(not(feature = "crash_safe"))]
    #[inline(always)]
    pub fn set_wait_list_ptr(&mut self, _ptr: Option<PMPtr<SortedPList<BlockedListItem>>>) {}

    #[inline(always)]
    pub fn get_wait_list_ptr(&self) -> Option<PMPtr<SortedPList<BlockedListItem>>> {
        self.wait_list_ptr
    }
}

#[cfg(feature = "opt_list")]
pub fn atomic_roll_forward_pop_remove_from_waitlist(
    wait_list: &mut SortedPList<BlockedListItem>,
    cs: &CriticalSection,
) {
    pre_list_op_hook();
    let j = unsafe { JournalHandle::new_dummy() };
    roll_forward_pop_remove_from_waitlist(wait_list, j, cs);
}

#[cfg(feature = "opt_list")]
pub fn roll_forward_pop_remove_from_waitlist(
    wait_list: &mut SortedPList<BlockedListItem>,
    j: JournalHandle,
    cs: &CriticalSection,
) {
    let log = ListTxOpLog::get_list_tx_op_log();
    // commit bypass
    if current().list_transaction_done() {
        debug_assert!(
            current().in_recovery_mode(),
            "can't bypass if not in recovery mode"
        );
        return;
    }
    let head = wait_list.0.head;
    log.set_block_node_ptr(head);
    compiler_pm_fence();
    log.set_tx_op(ListTxOpCode::WaitListPopRemoval);
    wait_list.pop_front(cs, j).map(|node| {
        let task = node.value.get_task();
        task.remove_from_delayed_list(j, cs);
        task.add_to_ready_list(j, cs);
    });
    log.commit();
}

#[cfg(feature = "opt_list")]
pub fn recover_and_roll_forward_of_waitlist_pop_remove(log: &mut ListTxOpLog) {
    let opcode = log.get_micro_op();
    let old_len = log.get_old_len();
    let mut node_ptr = unsafe {
        log.get_block_node_ptr::<BlockedListItem>()
            .unwrap_unchecked()
    };
    let node = unsafe { node_ptr.as_mut_no_logging() };
    let cs = unsafe { CriticalSection::new() };
    let j = unsafe { JournalHandle::new_dummy() };
    match opcode {
        ListOpCode::PopFront => {
            match node.list {
                None => {
                    // already popped
                }
                Some(mut l) => {
                    let list = unsafe { l.as_mut_no_logging() };
                    list.recover_from_failed_pop_front(node_ptr, old_len);
                }
            }
            let task = node.value.get_task();
            task.remove_from_delayed_list(j, &cs);
            task.add_to_ready_list(j, &cs)
        }

        ListOpCode::Remove => {
            let delay_list = unsafe { get_delay_list() };
            let task = node.value.get_task();
            delay_list
                .0
                .recover_from_failed_remove(task.get_sched_node_ptr(), old_len);
            task.add_to_ready_list(j, &cs)
        }

        ListOpCode::InsertBeforeCursor => {
            let task = node.value.get_task();
            let sched_list = unsafe { get_sched_list(task.get_priority()) };
            sched_list
                .0
                .recover_from_failed_insert_before_cursor(task.get_sched_node_ptr(), old_len);
            // change task state to be ready
            task.set_status(crate::task::TaskState::Ready, j);
        }

        ListOpCode::Invalid => {}
        _ => {
            panic!("Impossible Op");
        }
    }
    log.commit();
}

#[cfg(feature = "opt_list")]
pub fn atomic_roll_forward_remove_from_unsorted_waitlist(
    wait_list: &mut UnsortedPList<BlockedListItem>,
    node_ptr: PMPtr<Node<BlockedListItem>>,
    cs: &CriticalSection,
) {
    pre_list_op_hook();
    let j = unsafe { JournalHandle::new_dummy() };
    roll_forward_remove_from_unsorted_waitlist(wait_list, node_ptr, j, cs);
}

#[cfg(feature = "opt_list")]
pub fn roll_forward_remove_from_unsorted_waitlist(
    wait_list: &mut UnsortedPList<BlockedListItem>,
    node_ptr: PMPtr<Node<BlockedListItem>>,
    j: JournalHandle,
    cs: &CriticalSection,
) {
    let log = ListTxOpLog::get_list_tx_op_log();
    log.set_block_node_ptr(Some(node_ptr));
    compiler_pm_fence();
    log.set_tx_op(ListTxOpCode::WaitListRemoval);
    compiler_pm_fence();
    let node = node_ptr.as_ref();
    wait_list.remove(cs, j, node_ptr);
    let task = node.value.get_task();
    task.remove_from_delayed_list(j, cs);
    task.add_to_ready_list(j, cs);
    // invalidate?
    log.invalidate();
}

#[cfg(feature = "opt_list")]
pub fn recover_and_roll_forward_of_unsorted_waitlist_remove(log: &mut ListTxOpLog) {
    let opcode = log.get_micro_op();
    let old_len = log.get_old_len();
    let mut node_ptr = unsafe {
        log.get_block_node_ptr::<BlockedListItem>()
            .unwrap_unchecked()
    };
    let node = unsafe { node_ptr.as_mut_no_logging() };
    let cs = unsafe { CriticalSection::new() };
    let j = unsafe { JournalHandle::new_dummy() };
    match opcode {
        ListOpCode::Remove => {
            match node.list {
                None => {
                    // already removed from waitlist..
                    let delay_list = unsafe { get_delay_list() };
                    let task = node.value.get_task();
                    delay_list
                        .0
                        .recover_from_failed_remove(task.get_sched_node_ptr(), old_len);
                    task.add_to_ready_list(j, &cs)
                }
                Some(mut l) => {
                    let list = unsafe { l.as_mut_no_logging() };
                    list.recover_from_failed_remove(node_ptr, old_len);
                    let task = node.value.get_task();
                    task.remove_from_delayed_list(j, &cs);
                    task.add_to_ready_list(j, &cs)
                }
            }
        }

        ListOpCode::InsertBeforeCursor => {
            let task = node.value.get_task();
            let sched_list = unsafe { get_sched_list(task.get_priority()) };
            sched_list
                .0
                .recover_from_failed_insert_before_cursor(task.get_sched_node_ptr(), old_len);
            // change task state to be ready
            task.set_status(crate::task::TaskState::Ready, j);
        }

        ListOpCode::Invalid => {}

        _ => {
            panic!("Impossible Op");
        }
    }
    log.invalidate();
}

#[cfg(feature = "opt_list")]
pub fn atomic_roll_forward_insert_into_waitlist(
    wait_list: &mut SortedPList<BlockedListItem>,
    task: &mut Task,
    wakeup_time: Time,
    cs: &CriticalSection,
) {
    pre_list_op_hook();
    let j = unsafe { JournalHandle::new_dummy() };
    task.set_wakeup_time(wakeup_time);
    roll_forward_insert_into_waitlist(wait_list, task, j, cs);
}

#[cfg(feature = "opt_list")]
pub fn roll_forward_insert_into_waitlist(
    wait_list: &mut SortedPList<BlockedListItem>,
    task: &mut Task,
    j: JournalHandle,
    cs: &CriticalSection,
) {
    let wl_ptr = unsafe { PMPtr::from_mut_ref(wait_list) };
    let log = ListTxOpLog::get_list_tx_op_log();
    log.set_wait_list_ptr(Some(wl_ptr));
    compiler_pm_fence();
    log.set_tx_op(ListTxOpCode::WaitListInsert);
    // remove from ready list
    task.remove_from_ready_list(j, cs);
    // insert into wait list
    wait_list.insert(cs, j, task.get_event_node_ptr());
    // insert into delay list
    task.add_to_delayed_list(j, cs);
    // TODO() invalidate ??
    log.invalidate();
}

#[cfg(feature = "opt_list")]
pub fn atomic_roll_forward_insert_into_unsorted_waitlist(
    wait_list: &mut UnsortedPList<BlockedListItem>,
    task: &mut Task,
    cs: &CriticalSection,
) {
    pre_list_op_hook();
    let j = unsafe { JournalHandle::new_dummy() };
    roll_forward_insert_into_unsorted_waitlist(wait_list, task, j, cs);
}

#[cfg(feature = "opt_list")]
pub fn roll_forward_insert_into_unsorted_waitlist(
    wait_list: &mut UnsortedPList<BlockedListItem>,
    task: &mut Task,
    j: JournalHandle,
    cs: &CriticalSection,
) {
    let wl_ptr = unsafe { PMPtr::from_mut_ref(wait_list) };
    let log = ListTxOpLog::get_list_tx_op_log();
    log.set_wait_list_ptr(unsafe { transmute(Some(wl_ptr)) });
    compiler_pm_fence();
    log.set_tx_op(ListTxOpCode::UnsortedWaitListInsert);
    // remove from ready list
    task.remove_from_ready_list(j, cs);
    // insert into wait list
    wait_list.insert(cs, j, task.get_event_node_ptr());
    // insert into delay list
    task.add_to_delayed_list(j, cs);
    // TODO() invalidate ??
    log.invalidate();
}

#[cfg(feature = "opt_list")]
pub fn recover_and_roll_forward_of_waitlist_insert(log: &mut ListTxOpLog) {
    let opcode = log.get_micro_op();
    let old_len = log.get_old_len();
    let wl = unsafe {
        log.get_wait_list_ptr()
            .unwrap_unchecked()
            .as_mut_no_logging()
    };
    let task = current();
    let cs = unsafe { CriticalSection::new() };
    let j = unsafe { JournalHandle::new_dummy() };
    match opcode {
        ListOpCode::Remove => {
            let sched_list = unsafe { get_sched_list(task.get_priority()) };
            sched_list
                .0
                .recover_from_failed_remove(task.get_sched_node_ptr(), old_len);
            wl.insert(&cs, j, task.get_event_node_ptr());
            task.add_to_delayed_list(j, &cs);
        }

        ListOpCode::InsertSortedWaitList => {
            wl.0.recover_from_failed_insert_sorted(task.get_event_node_ptr(), old_len);
            task.add_to_delayed_list(j, &cs);
        }

        ListOpCode::InsertSortedDelayList => {
            let dl = unsafe { get_delay_list() };
            dl.0.recover_from_failed_insert_sorted(task.get_sched_node_ptr(), old_len);
            task.set_status(crate::task::TaskState::Blocked, j);
        }
        ListOpCode::Invalid => {
            // committed,
        }
        _ => {
            panic!("Impossible Op");
        }
    }
    // TODO() invalidate ??
    log.invalidate();
}

#[cfg(feature = "opt_list")]
pub fn recover_and_roll_forward_of_unsorted_waitlist_insert(log: &mut ListTxOpLog) {
    let opcode = log.get_micro_op();
    let old_len = log.get_old_len();
    let wl = unsafe {
        log.get_wait_list_ptr()
            .unwrap_unchecked()
            .as_mut_no_logging()
    };
    let task = current();
    let cs = unsafe { CriticalSection::new() };
    let j = unsafe { JournalHandle::new_dummy() };
    match opcode {
        ListOpCode::Remove => {
            let sched_list = unsafe { get_sched_list(task.get_priority()) };
            sched_list
                .0
                .recover_from_failed_remove(task.get_sched_node_ptr(), old_len);
            wl.insert(&cs, j, task.get_event_node_ptr());
            task.add_to_delayed_list(j, &cs);
        }

        ListOpCode::InsertFront => {
            wl.0.recover_from_failed_insert_front(task.get_event_node_ptr(), old_len);
            task.add_to_delayed_list(j, &cs);
        }

        ListOpCode::InsertSortedDelayList => {
            let dl = unsafe { get_delay_list() };
            dl.0.recover_from_failed_insert_sorted(task.get_sched_node_ptr(), old_len);
            task.set_status(crate::task::TaskState::Blocked, j);
        }
        ListOpCode::Invalid => {
            // committed,
        }
        _ => {
            panic!("Impossible Op");
        }
    }
    // TODO() invalidate ??
    log.invalidate();
}

#[cfg(feature = "opt_list")]
pub fn atomic_roll_forward_remove_from_delaylist(cs: &CriticalSection) {
    pre_list_op_hook();
    let j = unsafe { JournalHandle::new_dummy() };
    roll_forward_remove_from_delaylist(j, cs);
}

#[cfg(feature = "opt_list")]
pub fn roll_forward_remove_from_delaylist(j: JournalHandle, cs: &CriticalSection) {
    let log = ListTxOpLog::get_list_tx_op_log();
    log.set_tx_op(ListTxOpCode::DelayListRemoval);
    compiler_pm_fence();
    let delay_list = unsafe { get_delay_list() };
    let sched_node = unsafe { delay_list.pop_front(cs, j).unwrap_unchecked() };
    let task = sched_node.value.get_mut_task();
    task.get_event_node().list.map(|mut wait_list| {
        let wait_list = unsafe { wait_list.as_mut_no_logging() };
        wait_list.optimized_remove(task.get_event_node_ptr(), cs, j);
    });
    task.add_to_ready_list(j, cs);
    log.invalidate();
}

#[cfg(feature = "opt_list")]
pub fn recover_and_roll_forward_of_delaylist_removal(log: &mut ListTxOpLog) {
    let opcode = log.get_micro_op();
    let old_len = log.get_old_len();
    let mut node_ptr = unsafe { log.get_block_node_ptr::<SchedListItem>().unwrap_unchecked() };
    let node = unsafe { node_ptr.as_mut_no_logging() };
    let cs = unsafe { CriticalSection::new() };
    let j = unsafe { JournalHandle::new_dummy() };
    match opcode {
        // pop from delaylist (sched node)
        ListOpCode::PopFront => {
            match node.list {
                None => {
                    // already popped
                }
                Some(mut l) => {
                    let list = unsafe { l.as_mut_no_logging() };
                    list.recover_from_failed_pop_front(node_ptr, old_len);
                }
            }
            let task = node.value.get_mut_task();
            task.get_event_node().list.map(|mut wait_list| {
                let wait_list = unsafe { wait_list.as_mut_no_logging() };
                wait_list.optimized_remove(task.get_event_node_ptr(), &cs, j);
            });
            task.add_to_ready_list(j, &cs);
        }
        // remove from wait list if present (event node)
        ListOpCode::Remove => {
            let task = node.value.get_mut_task();
            let wait_list = unsafe {
                task.get_event_node()
                    .list
                    .unwrap_unchecked()
                    .as_mut_no_logging()
            };
            wait_list.recover_from_failed_remove(task.get_event_node_ptr(), old_len);
            task.add_to_ready_list(j, &cs);
        }

        ListOpCode::InsertBeforeCursor => {
            let task = node.value.get_task();
            let sched_list = unsafe { get_sched_list(task.get_priority()) };
            sched_list
                .0
                .recover_from_failed_insert_before_cursor(task.get_sched_node_ptr(), old_len);
            // change task state to be ready
            task.set_status(crate::task::TaskState::Ready, j);
        }
        ListOpCode::Invalid => {}
        _ => {
            panic!("Impossible Op");
        }
    }
    log.invalidate();
}

#[cfg(feature = "opt_list")]
pub fn atomic_roll_forward_insert_into_delaylist(cs: &CriticalSection) {
    pre_list_op_hook();
    let j = unsafe { JournalHandle::new_dummy() };
    roll_forward_insert_into_delaylist(j, cs);
}

#[cfg(feature = "opt_list")]
pub fn roll_forward_insert_into_delaylist(j: JournalHandle, cs: &CriticalSection) {
    let log = ListTxOpLog::get_list_tx_op_log();
    if current().list_transaction_done() {
        debug_assert!(
            current().in_recovery_mode(),
            "can't bypass if not in recovery mode"
        );
        return;
    }
    log.set_tx_op(ListTxOpCode::DelayListInsert);
    compiler_pm_fence();
    let task = current();
    task.remove_from_ready_list(j, cs);
    task.add_to_delayed_list(j, cs);
    log.commit();
}

#[cfg(feature = "opt_list")]
pub fn recover_and_roll_forward_of_delaylist_insert(log: &mut ListTxOpLog) {
    let opcode = log.get_micro_op();
    let old_len = log.get_old_len();
    let task = current();
    let cs = unsafe { CriticalSection::new() };
    let j = unsafe { JournalHandle::new_dummy() };

    match opcode {
        ListOpCode::Remove => {
            let rdlist = unsafe { get_sched_list(task.get_priority()) };
            rdlist
                .0
                .recover_from_failed_remove(task.get_sched_node_ptr(), old_len);
            task.add_to_delayed_list(j, &cs);
        }
        ListOpCode::InsertSortedDelayList => {
            let dl = unsafe { get_delay_list() };
            dl.0.recover_from_failed_insert_sorted(task.get_sched_node_ptr(), old_len);
            task.set_status(crate::task::TaskState::Blocked, j);
        }
        ListOpCode::Invalid => {}
        _ => {
            panic!("Impossible Op");
        }
    }

    log.commit();
}

#[cfg(feature = "opt_list")]
pub fn atomic_roll_forward_insert_into_readylist(task: &mut Task, cs: &CriticalSection) {
    pre_list_op_hook();
    let j = unsafe { JournalHandle::new_dummy() };
    roll_forward_insert_into_readylist(task, j, cs);
}

pub fn roll_forward_insert_into_readylist(task: &mut Task, j: JournalHandle, cs: &CriticalSection) {
    let log = ListTxOpLog::get_list_tx_op_log();
    log.set_task_ptr(Some(unsafe { PMPtr::from_mut_ref(task) }));
    compiler_pm_fence();
    log.set_tx_op(ListTxOpCode::ReadyListInsert);
    compiler_pm_fence();
    task.add_to_ready_list(j, cs);
    log.invalidate();
}

#[cfg(feature = "opt_list")]
pub fn recover_and_roll_forward_of_readylist_insert(log: &mut ListTxOpLog) {
    let opcode = log.get_micro_op();
    let old_len = log.get_old_len();
    let task = unsafe { log.get_task_ptr().unwrap_unchecked().as_mut_no_logging() };
    let cs = unsafe { CriticalSection::new() };
    let j = unsafe { JournalHandle::new_dummy() };

    match opcode {
        ListOpCode::InsertBeforeCursor => {
            let rdlist = unsafe { get_sched_list(task.get_priority()) };
            rdlist
                .0
                .recover_from_failed_insert_before_cursor(task.get_sched_node_ptr(), old_len);
            task.set_status(crate::task::TaskState::Ready, j);
        }
        ListOpCode::Invalid => {}
        _ => {
            panic!("Impossible Op");
        }
    }

    log.commit();
}

#[cfg(feature = "opt_list")]
pub fn atomic_roll_forward_remove_reinsert_into_activelist(node_ptr: PMPtr<Node<TimerListItem>>) {
    pre_list_op_hook();
    // log
    let log = ListTxOpLog::get_timer_list_tx_op_log();
    log.set_block_node_ptr(Some(node_ptr));
    compiler_pm_fence();
    log.set_tx_op(ListTxOpCode::ActiveListRemoveReInsert);
    compiler_pm_fence();
    let cs = unsafe { CriticalSection::new() };
    let j = unsafe { JournalHandle::new_dummy() };
    let al = unsafe { time::get_timer_active_list() };
    al.remove(&cs, j, node_ptr);
    al.insert(&cs, j, node_ptr);
    log.commit_activelist();
}

#[cfg(feature = "opt_list")]
pub fn recover_and_roll_forward_of_activelist_remove_reinsert(
    log: &mut ListTxOpLog,
) -> Result<(), ()> {
    let opcode = log.get_micro_op();
    let old_len = log.get_old_len();
    let al = unsafe { time::get_timer_active_list() };
    let mut node_ptr = unsafe { log.get_block_node_ptr::<TimerListItem>().unwrap_unchecked() };
    let node = unsafe { node_ptr.as_mut_no_logging() };
    let cs = unsafe { CriticalSection::new() };
    let j = unsafe { JournalHandle::new_dummy() };
    match opcode {
        ListOpCode::Remove => {
            list_dbg_print!("[recover remove-reinsert]: opcode = Remove");
            if !node.list.is_none() {
                al.0.recover_from_failed_remove(node_ptr, old_len);
            }
            al.insert(&cs, j, node_ptr);
        }

        ListOpCode::InsertSortedDelayList => {
            list_dbg_print!("[recover remove-reinsert]: opcode = InsertSortedDelayList");
            al.0.recover_from_failed_insert_sorted(node_ptr, old_len);
        }
        ListOpCode::Invalid => {
            return Err(());
        }
        _ => {
            panic!("Impossible Op");
        }
    }
    log.commit_activelist();
    Ok(())
}

#[cfg(feature = "opt_list")]
pub fn atomic_roll_forward_pop_reinsert_into_activelist() {
    pre_list_op_hook();
    let log = ListTxOpLog::get_timer_list_tx_op_log();
    let al = unsafe { time::get_timer_active_list() };
    let head = al.0.head;
    log.set_block_node_ptr(head);
    compiler_pm_fence();
    log.set_tx_op(ListTxOpCode::ActiveListPopReInsert);
    compiler_pm_fence();
    let cs = unsafe { CriticalSection::new() };
    let j = unsafe { JournalHandle::new_dummy() };

    let popped = al
        .pop_front(&cs, j)
        .map(|n| unsafe { PMPtr::from_mut_ref(n) });

    match popped {
        Some(ptr) => {
            al.insert(&cs, j, ptr);
        }
        None => {}
    }
    log.commit_activelist();
}

#[cfg(feature = "opt_list")]
pub fn recover_and_roll_forward_of_activelist_pop_reinsert(
    log: &mut ListTxOpLog,
) -> Result<(), ()> {
    let opcode = log.get_micro_op();
    let old_len = log.get_old_len();
    let al = unsafe { time::get_timer_active_list() };
    let mut node_ptr = unsafe { log.get_block_node_ptr::<TimerListItem>().unwrap_unchecked() };
    let node = unsafe { node_ptr.as_mut_no_logging() };
    let cs = unsafe { CriticalSection::new() };
    let j = unsafe { JournalHandle::new_dummy() };
    match opcode {
        ListOpCode::PopFront => {
            match node.list {
                None => {
                    // already popped
                }
                Some(mut l) => {
                    let list = unsafe { l.as_mut_no_logging() };
                    list.recover_from_failed_pop_front(node_ptr, old_len);
                }
            }
            al.insert(&cs, j, node_ptr)
        }

        ListOpCode::InsertSortedDelayList => {
            al.0.recover_from_failed_insert_sorted(node_ptr, old_len);
        }

        ListOpCode::Invalid => {
            return Err(());
        }

        _ => {
            panic!("Impossible Op");
        }
    }
    log.commit_activelist();
    Ok(())
}

#[cfg(feature = "opt_list")]
pub fn atomic_roll_forward_remove_from_activelist(node_ptr: PMPtr<Node<TimerListItem>>) {
    pre_list_op_hook();
    let log = ListTxOpLog::get_timer_list_tx_op_log();
    log.set_block_node_ptr(Some(node_ptr));
    compiler_pm_fence();
    log.set_tx_op(ListTxOpCode::ActiveListRemove);
    compiler_pm_fence();
    let al = unsafe { time::get_timer_active_list() };
    let cs = unsafe { CriticalSection::new() };
    let j = unsafe { JournalHandle::new_dummy() };
    al.remove(&cs, j, node_ptr);
    log.commit_activelist();
}

#[cfg(feature = "opt_list")]
pub fn recover_and_roll_forward_of_activelist_remove(log: &mut ListTxOpLog) -> Result<(), ()> {
    let opcode = log.get_micro_op();
    let old_len = log.get_old_len();
    let al = unsafe { time::get_timer_active_list() };
    let mut node_ptr = unsafe { log.get_block_node_ptr::<TimerListItem>().unwrap_unchecked() };
    let node = unsafe { node_ptr.as_mut_no_logging() };
    match opcode {
        ListOpCode::Remove => {
            list_dbg_print!("Opcode is remove, removing...");
            al.0.recover_from_failed_remove(node_ptr, old_len);
        }
        ListOpCode::Invalid => {
            list_dbg_print!("Opcode is Invalid, do nothing...");
            return Err(());
        }
        _ => {
            panic!("Impossible Op");
        }
    }
    log.commit_activelist();
    Ok(())
}

#[cfg(feature = "opt_list")]
#[inline(always)]
pub fn atomic_roll_foward_context_switch(
    task: &mut Task,
    prev_task: &mut Task,
    readylist: &mut CircularPList<SchedListItem>,
    current_ptr: &mut PVolatilePtr<Task>,
    j: JournalHandle,
    cs: &CriticalSection,
) {
    pre_list_op_hook();
    let log = ListTxOpLog::get_list_tx_op_log();
    // set prev ptr of task
    log.set_task_ptr(unsafe { Some(PMPtr::from_mut_ref(prev_task)) });
    compiler_pm_fence();
    log.set_micro_op_old_len(ListOpCode::ReadyListNext, 0);
    task.set_status(crate::task::TaskState::Running, j);
    if prev_task.get_status() != crate::task::TaskState::Blocked {
        prev_task.set_status(crate::task::TaskState::Ready, j);
    }
    readylist.next(cs);
    current_ptr.store(task as *mut Task);
    compiler_pm_fence();
    log.set_micro_op_old_len(ListOpCode::Invalid, 0);
}

#[cfg(feature = "opt_list")]
pub fn recover_and_roll_forward_context_switch(log: &mut ListTxOpLog) {
    let opcode = log.get_micro_op();
    let cs = unsafe { CriticalSection::new() };
    let j = unsafe { JournalHandle::new_dummy() };
    let current_ptr = unsafe { get_current_task_ptr() };
    match opcode {
        ListOpCode::ReadyListNext => {
            let mut prev_task_ptr = log.get_task_ptr().unwrap();
            let prev_task = unsafe { prev_task_ptr.as_mut_no_logging() };
            let rdlist = unsafe { get_sched_list(prev_task.get_priority()) };
            let cursor = rdlist.cursor(&cs).unwrap();
            if cursor.get_task() as *const Task == prev_task as *const Task {
                prev_task.set_status(crate::task::TaskState::Ready, j);
                let task = rdlist.next(&cs).unwrap().get_task();
                task.set_status(crate::task::TaskState::Running, j);
                current_ptr.store(task as *const Task as *mut Task);
            } else {
                // cursor already moved
                let task = cursor.get_task();
                task.set_status(crate::task::TaskState::Running, j);
                current_ptr.store(task as *const Task as *mut Task);
            }
            compiler_pm_fence();
            log.set_micro_op_old_len(ListOpCode::Invalid, 0);
        }
        ListOpCode::Invalid => {
            // Do nothing, we are good.
        }
        _ => {
            panic!("Invalid Op");
        }
    }
}

impl<T: Display> fmt::Display for Node<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

pub trait OpLogListItem {
    #[cfg(feature = "crash_safe")]
    #[inline(always)]
    fn start_op_log(opcode: ListOpCode, old_len: usize) {
        let list_tx_log = ListTxOpLog::get_list_tx_op_log();
        list_tx_log.set_micro_op_old_len(opcode, old_len);
    }

    #[cfg(not(feature = "crash_safe"))]
    #[inline(always)]
    fn start_op_log(_opcode: ListOpCode, _old_len: usize) {}
}

impl OpLogListItem for BlockedListItem {}
impl OpLogListItem for SchedListItem {}

impl OpLogListItem for TimerListItem {
    #[cfg(feature = "crash_safe")]
    #[inline(always)]
    fn start_op_log(opcode: ListOpCode, old_len: usize) {
        let list_tx_log = ListTxOpLog::get_timer_list_tx_op_log();
        list_tx_log.set_micro_op_old_len(opcode, old_len);
    }

    #[cfg(not(feature = "crash_safe"))]
    #[inline(always)]
    fn start_op_log(_opcode: ListOpCode, _old_len: usize) {}
}

#[derive(Clone, Copy)]
pub struct PList<T> {
    len: usize,
    cursor: PLink<T>, // cursor is the next node to visit when iterating through the list
    head: PLink<T>,
}

impl<T: Display> fmt::Display for PList<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.head.is_none() {
            write!(f, "(Empty List)")
        } else {
            let mut ret = write!(f, "${}$", self.head.unwrap().as_ref());
            let mut cur = self.head.unwrap().as_ref().next;
            while cur != self.head && !cur.is_none() {
                if cur == self.cursor {
                    ret = write!(f, " --> *<{}>*", cur.unwrap().as_ref());
                } else {
                    ret = write!(f, " --> <{}>", cur.unwrap().as_ref());
                }
                cur = cur.unwrap().as_ref().next;
            }
            if cur.is_none() {
                ret = write!(f, " --> <None>");
            }
            ret
        }
    }
}

impl<T: OpLogListItem> PList<T> {
    pub const fn new() -> Self {
        Self {
            len: 0,
            cursor: None,
            head: None,
        }
    }

    pub fn empty(&self) -> bool {
        self.len == 0
    }

    fn as_pm_ptr(&mut self) -> PMPtr<Self> {
        unsafe { PMPtr::from_mut_ref(self) }
    }

    pub fn insert_before_cursor(
        &mut self,
        cs: &CriticalSection,
        j: JournalHandle,
        mut link: PMPtr<Node<T>>,
    ) {
        // deref the persistent pointer
        let node: &mut Node<T> = link.as_mut(j);
        self.insert_before_cursor_impl(cs, j, node);
    }

    pub fn insert_before_cursor_impl(
        &mut self,
        _: &CriticalSection,
        j: JournalHandle,
        node: &mut Node<T>,
    ) {
        // set container
        debug_assert!(node.list.is_none());
        node.list = Some(self.as_pm_ptr());
        let link = unsafe { PMPtr::from_mut_ref(node) };
        match self.cursor {
            None => {
                node.prev = Some(link);
                node.next = Some(link);
                self.cursor = Some(link);
                self.head = Some(link);
            }
            Some(mut ptr) => {
                let cursor: &mut Node<T> = ptr.as_mut(j);
                // initialize node
                node.prev = cursor.prev;
                node.next = self.cursor;
                // connect the node into the list
                cursor.prev = Some(link);
                node.prev.unwrap().as_mut(j).next = Some(link);

                // update head if cursor is the old head
                if self.cursor == self.head {
                    self.head = Some(link);
                }
            }
        }

        self.len += 1;
    }

    pub fn optimized_insert_before_cursor(
        &mut self,
        mut link: PMPtr<Node<T>>,
        cs: &CriticalSection,
        j: JournalHandle,
    ) {
        let node = unsafe { link.as_mut_no_logging() };
        T::start_op_log(ListOpCode::InsertBeforeCursor, self.len);
        self.optimized_insert_before_cursor_impl(node, cs, j);
    }

    pub fn optimized_insert_before_cursor_impl(
        &mut self,
        node: &mut Node<T>,
        _: &CriticalSection,
        _j: JournalHandle,
    ) {
        if node.list != None {
            return;
        }
        debug_assert!(node.list.is_none());
        node.list = Some(self.as_pm_ptr());
        let link = unsafe { PMPtr::from_mut_ref(node) };
        match self.cursor {
            None => {
                node.prev = Some(link);
                node.next = Some(link);
                self.cursor = Some(link);
                self.head = Some(link);
            }
            Some(mut ptr) => {
                let cursor: &mut Node<T> = unsafe { ptr.as_mut_no_logging() };
                // initialize node
                node.prev = cursor.prev;
                node.next = self.cursor;
                unsafe {
                    node.prev.unwrap_unchecked().as_mut_no_logging().next = Some(link);
                }
                // check
                // connect the node into the list
                compiler_pm_fence();
                cursor.prev = Some(link);

                // update head if cursor is the old head
                if self.cursor == self.head {
                    self.head = Some(link);
                }
            }
        }

        self.len += 1;
    }

    pub fn recover_from_failed_insert_before_cursor(
        &mut self,
        mut node_ptr: PMPtr<Node<T>>,
        old_len: usize,
    ) {
        let node = unsafe { node_ptr.as_mut_no_logging() };
        if old_len == 0 {
            // idempotent
            node.prev = Some(node_ptr);
            node.next = Some(node_ptr);
            self.cursor = Some(node_ptr);
            self.head = Some(node_ptr);
        } else {
            let cursor = unsafe { self.cursor.unwrap_unchecked().as_mut_no_logging() };
            if cursor.prev == Some(node_ptr) {
                // we are done here
            } else {
                // idempotent
                // initialize node
                node.prev = cursor.prev;
                node.next = self.cursor;
                unsafe {
                    node.prev.unwrap_unchecked().as_mut_no_logging().next = Some(node_ptr);
                }
                // check
                // connect the node into the list
                compiler_pm_fence();
                cursor.prev = Some(node_ptr);
            }
            if self.cursor == self.head {
                self.head = Some(node_ptr);
            }
        }
        self.len = old_len + 1;
    }

    #[inline(always)]
    pub fn next(&mut self, _: &CriticalSection) -> Option<&T> {
        match self.cursor {
            None => None,
            Some(ptr) => {
                let node = ptr.as_ref();
                self.cursor = node.next;
                self.cursor.map(|n| &n.as_ref().value)
            }
        }
    }

    #[inline(always)]
    pub fn cursor(&self, _: &CriticalSection) -> Option<&T> {
        match self.cursor {
            None => None,
            Some(ptr) => self.cursor.map(|n| &n.as_ref().value),
        }
    }

    #[inline(always)]
    pub fn peek_next(&self, _: &CriticalSection) -> Option<&T> {
        match self.cursor {
            None => None,
            Some(ptr) => {
                let node = ptr.as_ref();
                node.next.map(|n| &n.as_ref().value)
            }
        }
    }

    pub fn remove(
        &mut self,
        cs: &CriticalSection,
        j: JournalHandle,
        mut link: PMPtr<Node<T>>,
    ) -> &mut Node<T> {
        let node = link.as_mut(j);
        self.remove_impl(cs, j, node);
        node
    }

    pub fn remove_impl(&mut self, _: &CriticalSection, j: JournalHandle, node: &mut Node<T>) {
        debug_assert!(self.len > 0);
        if node.list.is_none() {
            return;
        }
        let link = unsafe { PMPtr::from_mut_ref(node) };

        match self.len {
            1 => {
                self.cursor = None;
                self.head = None;
            }
            _ => {
                node.prev.map(|mut ptr| ptr.as_mut(j).next = node.next);
                node.next.map(|mut ptr| ptr.as_mut(j).prev = node.prev);

                // if cursor is removed, update the cursor
                if self.cursor == Some(link) {
                    self.cursor = node.next;
                }
                // if head is removed, update the head
                if self.head == Some(link) {
                    self.head = node.next;
                }
                // container list must be set to None
            }
        }
        node.list = None;
        self.len -= 1;
    }

    pub fn optimized_remove(
        &mut self,
        mut link: PMPtr<Node<T>>,
        cs: &CriticalSection,
        j: JournalHandle,
    ) {
        let node = unsafe { link.as_mut_no_logging() };
        T::start_op_log(ListOpCode::Remove, self.len);
        self.optimized_remove_impl(node, cs, j);
    }

    pub fn optimized_remove_impl(
        &mut self,
        node: &mut Node<T>,
        _: &CriticalSection,
        _j: JournalHandle,
    ) {
        if node.list.is_none() {
            return;
        }
        debug_assert!(self.len > 0);
        let link = unsafe { PMPtr::from_mut_ref(node) };

        match self.len {
            1 => {
                self.cursor = None;
                self.head = None;
            }
            _ => {
                node.prev
                    .map(|mut ptr| unsafe { ptr.as_mut_no_logging() }.next = node.next);
                node.next
                    .map(|mut ptr| unsafe { ptr.as_mut_no_logging() }.prev = node.prev);

                // if cursor is removed, update the cursor
                if self.cursor == Some(link) {
                    self.cursor = node.next;
                }
                // if head is removed, update the head
                if self.head == Some(link) {
                    self.head = node.next;
                }
                // container list must be set to None
            }
        }
        self.len -= 1;
        compiler_pm_fence();
        node.list = None;
    }

    pub fn recover_from_failed_remove(&mut self, mut removed_ptr: PMPtr<Node<T>>, old_len: usize) {
        // roll forward
        // it's idempotent!
        let removed_node = unsafe { removed_ptr.as_mut_no_logging() };
        let removed_link = Some(removed_ptr);
        removed_node
            .prev
            .map(|mut ptr| unsafe { ptr.as_mut_no_logging() }.next = removed_node.next);
        removed_node
            .next
            .map(|mut ptr| unsafe { ptr.as_mut_no_logging() }.prev = removed_node.prev);

        // if cursor is removed, update the cursor
        if self.cursor == removed_link {
            self.cursor = removed_node.next;
        }

        // if head is removed, update the head
        if self.head == removed_link {
            self.head = removed_node.next;
        }

        // set new len
        self.len = old_len - 1;
        compiler_pm_fence();
        removed_node.list = None;
    }

    pub fn pop_front(&mut self, _: &CriticalSection, j: JournalHandle) -> Option<&mut Node<T>> {
        let old_head: PLink<T> = self.head;
        match self.head {
            None => None,
            Some(mut ptr) => {
                let old_head_ref = ptr.as_mut(j);
                self.head = old_head_ref.next;

                self.head.map(|mut ptr| ptr.as_mut(j).prev = None);

                self.len -= 1;
                if self.cursor == old_head {
                    self.cursor = self.head;
                }
                old_head_ref.list = None;
                Some(old_head_ref)
            }
        }
    }

    pub fn optimized_pop_front(
        &mut self,
        cs: &CriticalSection,
        j: JournalHandle,
    ) -> Option<&mut Node<T>> {
        let old_head: PLink<T> = self.head;
        match self.head {
            None => None,
            Some(mut ptr) => {
                T::start_op_log(ListOpCode::PopFront, self.len);

                let old_head_ref = unsafe { ptr.as_mut_no_logging() };
                self.head = old_head_ref.next;
                compiler_pm_fence();
                self.head
                    .map(|mut ptr| unsafe { ptr.as_mut_no_logging() }.prev = None);

                if self.cursor == old_head {
                    self.cursor = self.head;
                }
                self.len -= 1;
                compiler_pm_fence();
                old_head_ref.list = None;

                Some(old_head_ref)
            }
        }
    }

    pub fn recover_from_failed_pop_front(
        &mut self,
        mut old_head_ptr: PMPtr<Node<T>>,
        old_len: usize,
    ) {
        if self.head != Some(old_head_ptr) {
            // just roll forward!
            self.head
                .map(|mut ptr| unsafe { ptr.as_mut_no_logging() }.prev = None);
            if self.cursor == Some(old_head_ptr) {
                self.cursor = self.head;
            }
            self.len = old_len - 1;
            unsafe {
                old_head_ptr.as_mut_no_logging().list = None;
            }
        } else {
            // not poped, Do the pop !
            let (cs, j) = unsafe { (CriticalSection::new(), JournalHandle::new_dummy()) };
            self.optimized_pop_front(&cs, j);
        }
    }

    pub fn insert_front(
        &mut self,
        _: &CriticalSection,
        j: JournalHandle,
        mut link: PMPtr<Node<T>>,
    ) {
        let new_node = link.as_mut(j);
        assert!(new_node.list.is_none());
        new_node.list = Some(self.as_pm_ptr());
        new_node.prev = None;
        new_node.next = None;
        match self.head {
            None => {
                self.head = Some(link);
                self.cursor = Some(link);
            }
            Some(mut ptr) => {
                new_node.next = self.head;
                let old_head = ptr.as_mut(j);
                old_head.prev = Some(link);
                self.head = Some(link);
            }
        }
        self.len += 1;
    }

    pub fn optimized_insert_front(
        &mut self,
        link: PMPtr<Node<T>>,
        cs: &CriticalSection,
        j: JournalHandle,
    ) {
        T::start_op_log(ListOpCode::InsertFront, self.len);
        self.optimized_insert_front_impl(link, cs, j);
    }

    pub fn optimized_insert_front_impl(
        &mut self,
        mut link: PMPtr<Node<T>>,
        _: &CriticalSection,
        _j: JournalHandle,
    ) {
        let new_node = unsafe { link.as_mut_no_logging() };
        debug_assert!(new_node.list.is_none());
        new_node.prev = None;
        new_node.next = None;
        match self.head {
            None => {
                self.head = Some(link);
            }
            Some(mut ptr) => {
                new_node.next = self.head;
                let old_head = unsafe { ptr.as_mut_no_logging() };
                old_head.prev = Some(link);
                compiler_pm_fence();
                self.head = Some(link);
            }
        }

        self.len += 1;
        new_node.list = Some(self.as_pm_ptr());
    }

    pub fn recover_from_failed_insert_front(
        &mut self,
        mut node_ptr: PMPtr<Node<T>>,
        old_len: usize,
    ) {
        let node = unsafe { node_ptr.as_mut_no_logging() };
        if self.head == Some(node_ptr) {
            // we are done
        } else {
            // re-insert
            node.prev = None;
            node.next = None;
            match self.head {
                None => {
                    self.head = Some(node_ptr);
                }
                Some(mut ptr) => {
                    node.next = self.head;
                    let old_head = unsafe { ptr.as_mut_no_logging() };
                    old_head.prev = Some(node_ptr);
                    compiler_pm_fence();
                    self.head = Some(node_ptr);
                }
            }
        }
        self.len = old_len + 1;
        node.list = Some(self.as_pm_ptr());
    }

    pub fn peek_front_mut(&self, _: &CriticalSection, j: JournalHandle) -> Option<&mut T> {
        match self.head {
            None => None,
            Some(mut ptr) => Some(&mut ptr.as_mut(j).value),
        }
    }

    pub fn peek_front(&self, _: &CriticalSection) -> Option<&T> {
        match self.head {
            None => None,
            Some(ptr) => Some(&ptr.as_ref().value),
        }
    }

    #[cfg(test)]
    pub fn crashed_optimized_insert_before_cursor(
        &mut self,
        link: &mut PMPtr<Node<T>>,
        _: &CriticalSection,
        _j: JournalHandle,
    ) {
        let node = unsafe { link.as_mut_no_logging() };
        crash_point!(0);
        T::start_op_log(ListOpCode::InsertBeforeCursor, self.len);
        crash_point!(1);

        if node.list == Some(self.as_pm_ptr()) {
            return;
        }
        debug_assert!(node.list.is_none());
        node.list = Some(self.as_pm_ptr());
        crash_point!(2);
        let link = unsafe { PMPtr::from_mut_ref(node) };
        match self.cursor {
            None => {
                node.prev = Some(link);
                crash_point!(3);
                node.next = Some(link);
                crash_point!(4);
                self.cursor = Some(link);
                crash_point!(5);
                self.head = Some(link);
                crash_point!(6);
            }
            Some(mut ptr) => {
                let cursor: &mut Node<T> = unsafe { ptr.as_mut_no_logging() };
                // initialize node
                node.prev = cursor.prev;
                crash_point!(3);
                node.next = self.cursor;
                crash_point!(4);
                unsafe {
                    node.prev.unwrap_unchecked().as_mut_no_logging().next = Some(link);
                }
                crash_point!(5);
                // check
                // connect the node into the list
                compiler_pm_fence();
                cursor.prev = Some(link);
                crash_point!(6);
                // update head if cursor is the old head
                if self.cursor == self.head {
                    self.head = Some(link);
                }
                crash_point!(7);
            }
        }
        self.len += 1;
        crash_point!(8);
    }

    #[cfg(test)]
    pub fn crashed_optimized_remove(
        &mut self,
        mut link: PMPtr<Node<T>>,
        _: &CriticalSection,
        _j: JournalHandle,
    ) {
        let node = unsafe { link.as_mut_no_logging() };
        crash_point!(0);
        T::start_op_log(ListOpCode::Remove, self.len);
        crash_point!(1);
        if node.list.is_none() {
            return;
        }
        debug_assert!(self.len > 0);
        let link = unsafe { PMPtr::from_mut_ref(node) };

        match self.len {
            1 => {
                self.cursor = None;
                crash_point!(2);
                self.head = None;
                crash_point!(3);
            }
            _ => {
                node.prev
                    .map(|mut ptr| unsafe { ptr.as_mut_no_logging() }.next = node.next);
                crash_point!(2);
                node.next
                    .map(|mut ptr| unsafe { ptr.as_mut_no_logging() }.prev = node.prev);
                crash_point!(3);
                // if cursor is removed, update the cursor
                if self.cursor == Some(link) {
                    self.cursor = node.next;
                }
                crash_point!(4);
                // if head is removed, update the head
                if self.head == Some(link) {
                    self.head = node.next;
                }
                crash_point!(5);
                // container list must be set to None
            }
        }
        self.len -= 1;
        crash_point!(6);
        compiler_pm_fence();
        node.list = None;
        crash_point!(7);
    }

    #[cfg(test)]
    pub fn crashed_optimized_pop_front(
        &mut self,
        cs: &CriticalSection,
        j: JournalHandle,
    ) -> Option<&mut Node<T>> {
        let old_head: PLink<T> = self.head;
        match self.head {
            None => None,
            Some(mut ptr) => {
                crash_point!(0, None);
                T::start_op_log(ListOpCode::PopFront, self.len);
                crash_point!(1, None);
                let old_head_ref = unsafe { ptr.as_mut_no_logging() };
                self.head = old_head_ref.next;
                crash_point!(2, None);
                compiler_pm_fence();
                self.head
                    .map(|mut ptr| unsafe { ptr.as_mut_no_logging() }.prev = None);
                crash_point!(3, None);
                if self.cursor == old_head {
                    self.cursor = self.head;
                }
                crash_point!(4, None);
                self.len -= 1;
                crash_point!(5, None);
                compiler_pm_fence();
                old_head_ref.list = None;
                crash_point!(6, None);
                Some(old_head_ref)
            }
        }
    }
}

// Assending Sorted List

impl<T: Ord + OpLogListItem> PList<T> {
    pub fn length(&self) -> usize {
        self.len
    }
    pub fn insert_sorted(
        &mut self,
        cs: &CriticalSection,
        j: JournalHandle,
        mut link: PMPtr<Node<T>>,
    ) {
        let new_node = link.as_mut(j);
        self.insert_sorted_impl(cs, j, new_node);
    }
    pub fn insert_sorted_impl(
        &mut self,
        _: &CriticalSection,
        j: JournalHandle,
        new_node: &mut Node<T>,
    ) {
        let link = unsafe { PMPtr::from_mut_ref(new_node) };
        assert!(new_node.list.is_none());
        new_node.list = Some(self.as_pm_ptr());
        new_node.next = None;
        new_node.prev = None;
        match self.head {
            None => self.head = Some(link),
            Some(mut ptr) => {
                let mut node = ptr.as_ref();
                while new_node.value > node.value {
                    match node.next {
                        None => {
                            // insert after this node, becoming the new tail
                            let node = ptr.as_mut(j);
                            new_node.prev = Some(unsafe { PMPtr::from_ref(node) });
                            new_node.next = None;
                            node.next = Some(link);
                            self.len += 1;
                            return;
                        }
                        Some(next_ptr) => {
                            node = next_ptr.as_ref();
                            ptr = next_ptr;
                        }
                    }
                }
                // insert at the prev of the node
                new_node.next = Some(unsafe { PMPtr::from_ref(node) });
                new_node.prev = node.prev;

                // Need to obtain mutable ref first!
                let node = ptr.as_mut(j);
                node.prev = Some(link);

                if !new_node.prev.is_none() {
                    unsafe {
                        let prev = new_node.prev.unwrap_unchecked().as_mut(j);
                        prev.next = Some(link);
                    }
                } else {
                    self.head = Some(link);
                }
            }
        }

        self.len += 1;
    }

    pub fn optimized_insert_sorted(
        &mut self,
        mut link: PMPtr<Node<T>>,
        cs: &CriticalSection,
        j: JournalHandle,
    ) {
        // clear the prev & next field...
        unsafe {
            let n = link.as_mut_no_logging();
            n.next = None;
            n.prev = None;
        }
        self.optimized_insert_sorted_impl(link, cs, j);
    }

    pub fn optimized_insert_sorted_impl(
        &mut self,
        mut link: PMPtr<Node<T>>,
        _: &CriticalSection,
        _j: JournalHandle,
    ) {
        let new_node = unsafe { link.as_mut_no_logging() };
        #[cfg(not(test))]
        debug_assert!(new_node.list.is_none());
        new_node.list = Some(self.as_pm_ptr());
        compiler_pm_fence();
        match self.head {
            None => {
                self.head = Some(link);
            }
            Some(mut ptr) => {
                let mut node = ptr.as_ref();
                while new_node.value > node.value {
                    match node.next {
                        None => {
                            // insert after this node, becoming the new tail
                            let node = unsafe { ptr.as_mut_no_logging() };
                            new_node.prev = Some(unsafe { PMPtr::from_ref(node) });
                            compiler_pm_fence();
                            new_node.next = None;
                            node.next = Some(link);
                            self.len += 1;
                            return;
                        }
                        Some(next_ptr) => {
                            node = next_ptr.as_ref();
                            ptr = next_ptr;
                        }
                    }
                }
                // insert at the prev of the node
                new_node.next = Some(unsafe { PMPtr::from_ref(node) });
                compiler_pm_fence();
                new_node.prev = node.prev;
                compiler_pm_fence();
                // Need to obtain mutable ref first!
                let node = unsafe { ptr.as_mut_no_logging() };
                node.prev = Some(link);
                compiler_pm_fence();
                if !new_node.prev.is_none() {
                    unsafe {
                        let prev = new_node.prev.unwrap_unchecked().as_mut_no_logging();
                        prev.next = Some(link);
                    }
                } else {
                    self.head = Some(link);
                }
            }
        }

        self.len += 1;
    }

    pub fn recover_from_failed_insert_sorted(
        &mut self,
        mut node_ptr: PMPtr<Node<T>>,
        old_len: usize,
    ) {
        let node = unsafe { node_ptr.as_mut_no_logging() };
        if node.list != Some(self.as_pm_ptr()) {
            list_dbg_print!("[recover insert_sorted]: case 1");
            node.next = None;
            node.prev = None;
            let cs = unsafe { CriticalSection::new() };
            let j = unsafe { JournalHandle::new_dummy() };
            self.optimized_insert_sorted_impl(node_ptr, &cs, j);
        } else {
            if node.next != None {
                list_dbg_print!("[recover insert_sorted]: case 2");
                let next = unsafe { node.next.unwrap_unchecked().as_mut_no_logging() };
                if next.prev == Some(node_ptr) {
                    // do nothing here
                } else {
                    node.prev = next.prev;
                    compiler_pm_fence();
                    next.prev = Some(node_ptr);
                }
                // connect prev of node to the node (set prev->next)
                if !node.prev.is_none() {
                    unsafe {
                        let prev = node.prev.unwrap_unchecked().as_mut_no_logging();
                        prev.next = Some(node_ptr);
                    }
                } else {
                    self.head = Some(node_ptr);
                }
                node.list = Some(self.as_pm_ptr());
                self.len = old_len + 1;
            } else if node.prev != None {
                list_dbg_print!("[recover insert_sorted]: case 3");
                // insert to the end
                node.next = None;
                unsafe {
                    node.prev.unwrap_unchecked().as_mut_no_logging().next = Some(node_ptr);
                }
                node.list = Some(self.as_pm_ptr());
                self.len = old_len + 1;
            } else if self.head == Some(node_ptr) {
                list_dbg_print!("[recover insert_sorted]: case 4");
                self.len = old_len + 1;
            } else {
                list_dbg_print!("[recover insert_sorted]: case 5");
                // never inserted, just redo everything
                self.len = old_len;
                let cs = unsafe { CriticalSection::new() };
                let j = unsafe { JournalHandle::new_dummy() };
                self.optimized_insert_sorted_impl(node_ptr, &cs, j);
            }
        }
    }

    #[cfg(test)]
    pub fn crashed_optimized_insert_sorted(
        &mut self,
        mut link: PMPtr<Node<T>>,
        _: &CriticalSection,
        _j: JournalHandle,
    ) {
        unsafe {
            let n = link.as_mut_no_logging();
            n.next = None;
            crash_point!(0);
            n.prev = None;
            crash_point!(1);
        }
        let new_node = unsafe { link.as_mut_no_logging() };
        debug_assert!(new_node.list.is_none());
        new_node.list = Some(self.as_pm_ptr());
        compiler_pm_fence();
        crash_point!(2);
        match self.head {
            None => {
                self.head = Some(link);
                crash_point!(3);
            }
            Some(mut ptr) => {
                let mut node = ptr.as_ref();
                while new_node.value > node.value {
                    match node.next {
                        None => {
                            // insert after this node, becoming the new tail
                            let node = unsafe { ptr.as_mut_no_logging() };
                            new_node.prev = Some(unsafe { PMPtr::from_ref(node) });
                            crash_point!(3);
                            compiler_pm_fence();
                            new_node.next = None;
                            crash_point!(4);
                            node.next = Some(link);
                            crash_point!(5);
                            self.len += 1;
                            return;
                        }
                        Some(next_ptr) => {
                            node = next_ptr.as_ref();
                            ptr = next_ptr;
                        }
                    }
                }
                // insert at the prev of the node
                new_node.next = Some(unsafe { PMPtr::from_ref(node) });
                crash_point!(3);
                compiler_pm_fence();
                new_node.prev = node.prev;
                crash_point!(4);
                compiler_pm_fence();
                // Need to obtain mutable ref first!
                let node = unsafe { ptr.as_mut_no_logging() };
                node.prev = Some(link);
                crash_point!(5);
                compiler_pm_fence();
                if !new_node.prev.is_none() {
                    unsafe {
                        let prev = new_node.prev.unwrap_unchecked().as_mut_no_logging();
                        prev.next = Some(link);
                        crash_point!(6);
                    }
                } else {
                    self.head = Some(link);
                    crash_point!(6);
                }
            }
        }
        self.len += 1;
        crash_point!(7);
    }
}

pub trait StartOpLog {
    #[cfg(feature = "crash_safe")]
    fn start_op_log(&self, opcode: ListOpCode, old_len: usize) {
        let list_tx_log = ListTxOpLog::get_list_tx_op_log();
        list_tx_log.set_micro_op_old_len(opcode, old_len);
    }

    #[cfg(not(feature = "crash_safe"))]
    fn start_op_log(&self, _opcode: ListOpCode, _old_len: usize) {}
}

pub struct Iter<'a, T> {
    next: Option<&'a Node<T>>,
}

pub struct IterMut<T> {
    next: PLink<T>,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;
    fn next(&mut self) -> Option<Self::Item> {
        self.next.map(|node| {
            self.next = node.next.map(|n| n.as_ref());
            &node.value
        })
    }
}

impl<T> Iterator for IterMut<T> {
    type Item = PMPtr<Node<T>>;
    fn next(&mut self) -> Option<Self::Item> {
        self.next.map(|node| {
            self.next = node.as_ref().next;
            node
        })
    }
}

unsafe impl<T> Sync for SortedPList<T> {}
unsafe impl<T> Sync for CircularPList<T> {}
unsafe impl<T> Sync for UnsortedPList<T> {}

#[derive(Clone, Copy)]
pub struct SortedPList<T>(PList<T>);

pub trait InsertSortedPList {
    type Item;
    fn insert(&mut self, cs: &CriticalSection, j: JournalHandle, link: PMPtr<Node<Self::Item>>);
}

impl<T: OpLogListItem> SortedPList<T> {
    pub fn len(&self) -> usize {
        self.0.len
    }

    #[cfg(test)]
    pub fn head(&self) -> Option<PMPtr<Node<T>>> {
        self.0.head
    }

    #[cfg(test)]
    pub fn as_pm_ptr(&mut self) -> PMPtr<PList<T>> {
        self.0.as_pm_ptr()
    }
}

impl InsertSortedPList for SortedPList<SchedListItem> {
    type Item = SchedListItem;
    fn insert(&mut self, cs: &CriticalSection, j: JournalHandle, link: PMPtr<Node<Self::Item>>) {
        #[cfg(not(feature = "opt_list"))]
        self.0.insert_sorted(cs, j, link);
        #[cfg(feature = "opt_list")]
        {
            SchedListItem::start_op_log(ListOpCode::InsertSortedDelayList, self.0.len);
            compiler_pm_fence();
            self.0.optimized_insert_sorted(link, cs, j);
        }
    }
}

impl InsertSortedPList for SortedPList<BlockedListItem> {
    type Item = BlockedListItem;
    fn insert(&mut self, cs: &CriticalSection, j: JournalHandle, link: PMPtr<Node<Self::Item>>) {
        #[cfg(not(feature = "opt_list"))]
        self.0.insert_sorted(cs, j, link);
        #[cfg(feature = "opt_list")]
        {
            BlockedListItem::start_op_log(ListOpCode::InsertSortedWaitList, self.0.len);
            compiler_pm_fence();
            self.0.optimized_insert_sorted(link, cs, j);
        }
    }
}

impl InsertSortedPList for SortedPList<TimerListItem> {
    type Item = TimerListItem;

    #[cfg(not(feature = "opt_list"))]
    fn insert(&mut self, cs: &CriticalSection, j: JournalHandle, link: PMPtr<Node<Self::Item>>) {
        self.0.insert_sorted(cs, j, link);
    }

    #[cfg(feature = "opt_list")]
    fn insert(&mut self, cs: &CriticalSection, j: JournalHandle, link: PMPtr<Node<Self::Item>>) {
        TimerListItem::start_op_log(ListOpCode::InsertSortedDelayList, self.0.len);
        compiler_pm_fence();
        self.0.optimized_insert_sorted(link, cs, j);
    }
}

impl<T: Ord + OpLogListItem> SortedPList<T> {
    pub const fn new() -> Self {
        Self(PList::<T>::new())
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn insert_node(&mut self, cs: &CriticalSection, j: JournalHandle, n: &mut Node<T>) {
        self.0.insert_sorted_impl(cs, j, n);
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn remove(
        &mut self,
        cs: &CriticalSection,
        j: JournalHandle,
        link: PMPtr<Node<T>>,
    ) -> &mut Node<T> {
        self.0.remove(cs, j, link)
    }

    #[cfg(feature = "opt_list")]
    pub fn remove(&mut self, cs: &CriticalSection, j: JournalHandle, link: PMPtr<Node<T>>) {
        self.0.optimized_remove(link, cs, j);
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn remove_node(&mut self, cs: &CriticalSection, j: JournalHandle, n: &mut Node<T>) {
        self.0.remove_impl(cs, j, n);
    }

    pub fn pop_front(&mut self, cs: &CriticalSection, j: JournalHandle) -> Option<&mut Node<T>> {
        #[cfg(not(feature = "opt_list"))]
        return self.0.pop_front(cs, j);
        #[cfg(feature = "opt_list")]
        return self.0.optimized_pop_front(cs, j);
    }

    pub fn peek_front_mut(&self, cs: &CriticalSection, j: JournalHandle) -> Option<&mut T> {
        self.0.peek_front_mut(cs, j)
    }

    pub fn peek_front(&self, cs: &CriticalSection) -> Option<&T> {
        self.0.peek_front(cs)
    }

    pub fn iter(&self) -> Iter<T> {
        Iter {
            next: self.0.head.map(|n| n.as_ref()),
        }
    }
}

impl<T: Display> fmt::Display for SortedPList<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Copy)]
pub struct CircularPList<T>(PList<T>);

impl<T: OpLogListItem> CircularPList<T> {
    pub const fn new() -> Self {
        Self(PList::<T>::new())
    }

    pub fn length(&self) -> usize {
        self.0.len
    }

    pub fn insert(&mut self, cs: &CriticalSection, j: JournalHandle, link: PMPtr<Node<T>>) {
        #[cfg(not(feature = "opt_list"))]
        self.0.insert_before_cursor(cs, j, link);
        #[cfg(feature = "opt_list")]
        self.0.optimized_insert_before_cursor(link, cs, j);
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn insert_node(&mut self, cs: &CriticalSection, j: JournalHandle, n: &mut Node<T>) {
        self.0.insert_before_cursor_impl(cs, j, n);
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn remove(
        &mut self,
        cs: &CriticalSection,
        j: JournalHandle,
        link: PMPtr<Node<T>>,
    ) -> &mut Node<T> {
        self.0.remove(cs, j, link)
    }

    #[cfg(feature = "opt_list")]
    pub fn remove(&mut self, cs: &CriticalSection, j: JournalHandle, link: PMPtr<Node<T>>) {
        self.0.optimized_remove(link, cs, j);
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn remove_node(&mut self, cs: &CriticalSection, j: JournalHandle, n: &mut Node<T>) {
        self.0.remove_impl(cs, j, n);
    }

    pub fn next(&mut self, cs: &CriticalSection) -> Option<&T> {
        self.0.next(cs)
    }

    pub fn peek_next(&self, cs: &CriticalSection) -> Option<&T> {
        self.0.peek_next(cs)
    }

    pub fn cursor(&self, cs: &CriticalSection) -> Option<&T> {
        self.0.cursor(cs)
    }
}

impl<T: Display> fmt::Display for CircularPList<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub struct UnsortedPList<T>(PList<T>);

impl<T: Display> fmt::Display for UnsortedPList<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<T: OpLogListItem> UnsortedPList<T> {
    pub const fn new() -> Self {
        Self(PList::<T>::new())
    }

    pub fn insert(&mut self, cs: &CriticalSection, j: JournalHandle, link: PMPtr<Node<T>>) {
        #[cfg(not(feature = "opt_list"))]
        self.0.insert_front(cs, j, link);
        #[cfg(feature = "opt_list")]
        self.0.optimized_insert_front(link, cs, j);
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn remove(
        &mut self,
        cs: &CriticalSection,
        j: JournalHandle,
        link: PMPtr<Node<T>>,
    ) -> &mut Node<T> {
        self.0.remove(cs, j, link)
    }

    #[cfg(feature = "opt_list")]
    pub fn remove(&mut self, cs: &CriticalSection, j: JournalHandle, link: PMPtr<Node<T>>) {
        self.0.optimized_remove(link, cs, j);
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn remove_node(&mut self, cs: &CriticalSection, j: JournalHandle, n: &mut Node<T>) {
        self.0.remove_impl(cs, j, n);
    }

    pub fn iter_mut(&mut self) -> IterMut<T> {
        IterMut { next: self.0.head }
    }

    pub fn iter(&self) -> Iter<T> {
        Iter {
            next: self.0.head.map(|n| n.as_ref()),
        }
    }
}

#[cfg(test)]
fn reset_list_tx_logs() {
    unsafe {
        TIMER_LIST_TX_OP_LOG = ListTxOpLog::new();
        LIST_TX_OP_LOG = ListTxOpLog::new();
    }
}

/* Unit Test for Lists */
#[cfg(test)]
pub mod test {
    use crate::{
        heap::new,
        os_print,
        time::{reset_timer_active_list, Timer},
    };

    use super::*;
    fn dummy(_x: usize) {}

    const NO_CRASH: usize = 10000;

    fn cleanup() {
        reset_list_tx_logs();
        reset_timer_active_list();
    }

    fn may_crashed_timer_list_insert(
        cp: usize,
        tl: &mut SortedPList<TimerListItem>,
        link: PMPtr<Node<TimerListItem>>,
    ) {
        let log = ListTxOpLog::get_timer_list_tx_op_log();
        log.set_block_node_ptr(Some(link));
        compiler_pm_fence();
        log.set_tx_op(ListTxOpCode::ActiveListRemoveReInsert);
        compiler_pm_fence();
        let cs = unsafe { CriticalSection::new() };
        let j = unsafe { JournalHandle::new_dummy() };
        TimerListItem::start_op_log(ListOpCode::InsertSortedDelayList, tl.0.len);
        if cp == 0 {
            return;
        }
        set_list_crash_point(cp - 1);
        tl.0.crashed_optimized_insert_sorted(link, &cs, j);
        if cp == NO_CRASH {
            log.commit_activelist();
        }
    }

    fn crashed_sorted_delay_list_insert(
        tl: &mut SortedPList<TimerListItem>,
        link: PMPtr<Node<TimerListItem>>,
    ) {
        let cs = unsafe { CriticalSection::new() };
        let j = unsafe { JournalHandle::new_dummy() };
        TimerListItem::start_op_log(ListOpCode::InsertSortedDelayList, tl.0.len);
        tl.0.crashed_optimized_insert_sorted(link, &cs, j)
    }

    pub fn crashed_atomic_roll_forward_remove_from_activelist(
        cp1: usize,
        cp2: usize,
        node_ptr: PMPtr<Node<TimerListItem>>,
    ) {
        set_list_crash_point(cp1);
        let log = ListTxOpLog::get_timer_list_tx_op_log();
        log.set_block_node_ptr(Some(node_ptr));
        crash_point!(0);
        compiler_pm_fence();
        log.set_tx_op(ListTxOpCode::ActiveListRemove);
        compiler_pm_fence();
        crash_point!(1);
        let al = unsafe { time::get_timer_active_list() };
        let cs = unsafe { CriticalSection::new() };
        let j = unsafe { JournalHandle::new_dummy() };
        if get_list_crash_point() == 2 {
            set_list_crash_point(cp2);
            al.0.crashed_optimized_remove(node_ptr, &cs, j);
        } else {
            al.remove(&cs, j, node_ptr);
        }
        set_list_crash_point(cp1);
        crash_point!(2);
        log.set_tx_op(ListTxOpCode::ActiveListTxCommitted);
        crash_point!(3);
        compiler_pm_fence();
        // invalidate
        log.micro_op_old_len = 0;
        crash_point!(4);
    }

    fn may_crashed_timer_list_remove(
        cp: usize,
        tl: &mut SortedPList<TimerListItem>,
        node_ptr: PMPtr<Node<TimerListItem>>,
    ) {
        let log = ListTxOpLog::get_timer_list_tx_op_log();
        log.set_block_node_ptr(Some(node_ptr));
        compiler_pm_fence();
        log.set_tx_op(ListTxOpCode::ActiveListRemove);
        compiler_pm_fence();
        let cs = unsafe { CriticalSection::new() };
        let j = unsafe { JournalHandle::new_dummy() };
        set_list_crash_point(cp);
        tl.0.crashed_optimized_remove(node_ptr, &cs, j);
        if cp == NO_CRASH {
            log.commit_activelist();
        }
    }

    fn may_crashed_timer_list_pop(cp: usize) {
        let log = ListTxOpLog::get_timer_list_tx_op_log();
        let al = unsafe { time::get_timer_active_list() };
        let head = al.0.head;
        log.set_block_node_ptr(head);
        compiler_pm_fence();
        log.set_tx_op(ListTxOpCode::ActiveListPop);
        compiler_pm_fence();
        let cs = unsafe { CriticalSection::new() };
        let j = unsafe { JournalHandle::new_dummy() };
        set_list_crash_point(cp);
        al.0.crashed_optimized_pop_front(&cs, j);
        if cp == NO_CRASH {
            log.commit_activelist();
        }
    }

    fn timer_list_insert_recovery(tl: &mut SortedPList<TimerListItem>) {
        let log = ListTxOpLog::get_timer_list_tx_op_log();
        let opcode = log.get_micro_op();
        let old_len = log.get_old_len();
        let mut node_ptr = unsafe { log.get_block_node_ptr::<TimerListItem>().unwrap_unchecked() };
        let node = unsafe { node_ptr.as_mut_no_logging() };
        match opcode {
            ListOpCode::InsertSortedDelayList => {
                tl.0.recover_from_failed_insert_sorted(node_ptr, old_len);
            }
            ListOpCode::Invalid => {
                // committed,
            }
            _ => {
                panic!("Impossible Op");
            }
        }
        log.commit_activelist();
    }

    fn new_timer(exp: Time) -> Timer {
        Timer::new("test", 10, false, dummy, exp, 0)
    }

    fn crashed_insert_timer(cp: usize, tl: &mut SortedPList<TimerListItem>, t: &Timer) {
        let link = t.get_list_node();
        may_crashed_timer_list_insert(cp, tl, link);
    }

    fn insert_timer(tl: &mut SortedPList<TimerListItem>, t: &Timer) {
        let link = t.get_list_node();
        may_crashed_timer_list_insert(NO_CRASH, tl, link);
    }

    #[test]
    fn test_timer_list_insert_1() {
        fn run(cp: usize) {
            cleanup();
            let mut tl = SortedPList::<TimerListItem>::new();
            let tl_ptr = tl.0.as_pm_ptr();
            let t = new_timer(10);
            crashed_insert_timer(cp, &mut tl, &t);
            os_print!("\n------ Restart ------\n");
            timer_list_insert_recovery(&mut tl);
            let list_node = t.get_list_node().as_ref();
            assert!(list_node.list == Some(tl_ptr));
            assert!(list_node.next == None);
            assert!(list_node.prev == None);
            assert!(tl.0.head == Some(t.get_list_node()));
            assert!(tl.0.len == 1);
        }

        for cp in 0..9 {
            os_print!("\nTesting Crash Point {}\n", cp);
            run(cp);
        }
    }

    fn backtrack(
        n: usize,
        cand: &Vec<usize>,
        status: &mut Vec<bool>,
        perm: &mut Vec<usize>,
        res: &mut Vec<Vec<usize>>,
    ) {
        if n == 0 {
            res.push(perm.clone());
            return;
        }
        for (i, v) in cand.iter().enumerate() {
            if status[i] == true {
                status[i] = false;
                perm.push(*v);
                backtrack(n - 1, cand, status, perm, res);
                perm.pop();
                status[i] = true;
            }
        }
    }

    fn gen_permutations(n: usize) -> Vec<Vec<usize>> {
        let mut res: Vec<Vec<usize>> = vec![];
        let mut cand = vec![];
        let mut status = Vec::new();
        for i in 0..n {
            cand.push(i);
            status.push(true);
        }
        let mut perm = Vec::new();
        backtrack(n, &cand, &mut status, &mut perm, &mut res);
        res
    }

    #[test]
    fn test_perm_gen() {
        let p = gen_permutations(3);
        for v in p {
            os_print!("{:?}", v);
        }
    }

    // Insert at the end of list
    #[test]
    fn test_timer_list_insert_2() {
        fn run(order: &Vec<usize>, cp: usize) {
            cleanup();
            let mut tl = SortedPList::<TimerListItem>::new();
            let tl_ptr = tl.0.as_pm_ptr();
            let t_a = new_timer(10);
            let t_b = new_timer(20);
            let t_c = new_timer(30);

            let tv = [&t_a, &t_b, &t_c];

            let mut cnt = 0;
            for i in order {
                if cnt == order.len() - 1 {
                    break;
                }
                insert_timer(&mut tl, tv[*i]);
                cnt += 1;
            }

            crashed_insert_timer(cp, &mut tl, tv[order[order.len() - 1]]);

            os_print!("\n------ Restart ------\n");

            timer_list_insert_recovery(&mut tl);
            let na = t_a.get_list_node().as_ref();
            let nb = t_b.get_list_node().as_ref();
            let nc = t_c.get_list_node().as_ref();

            assert!(na.list == Some(tl_ptr));
            assert!(na.next == Some(t_b.get_list_node()));
            assert!(na.prev == None);

            assert!(nb.list == Some(tl_ptr));
            assert!(nb.next == Some(t_c.get_list_node()));
            assert!(nb.prev == Some(t_a.get_list_node()));

            assert!(nc.list == Some(tl_ptr));
            assert!(nc.next == None);
            assert!(nc.prev == Some(t_b.get_list_node()));

            assert!(tl.0.head == Some(t_a.get_list_node()));
            assert!(tl.0.len == 3);
        }

        let perms = gen_permutations(3);

        for perm in perms {
            for cp in 0..9 {
                os_print!("\nInsert Order {:?},  Testing Crash Point {}\n", perm, cp);
                run(&perm, cp);
            }
        }
    }

    #[test]
    fn test_timer_list_remove_1() {
        fn run(cp: usize) {
            cleanup();
            let mut tl = unsafe { time::get_timer_active_list() };
            let tl_ptr = tl.0.as_pm_ptr();
            let t_a = new_timer(10);
            let na = t_a.get_list_node().as_ref();
            insert_timer(&mut tl, &t_a);
            assert!(na.list == Some(tl_ptr));
            assert!(tl.0.len == 1);
            assert!(na.next == None);
            assert!(na.prev == None);
            assert!(tl.0.head == Some(t_a.get_list_node()));

            may_crashed_timer_list_remove(cp, &mut tl, t_a.get_list_node());

            os_print!("\n------ Restart ------\n");

            let log = ListTxOpLog::get_timer_list_tx_op_log();
            recover_and_roll_forward_of_activelist_remove(log);

            assert!(tl.0.head == None);
            assert!(tl.0.len == 0);
            assert!(na.list == None);
        }

        for cp in 1..8 {
            os_print!("Testing Crash Point {}\n", cp);
            run(cp);
        }
    }

    #[test]
    fn test_timer_list_remove_2() {
        fn run(cp: usize, removed: usize) {
            cleanup();
            let mut tl = unsafe { time::get_timer_active_list() };
            let tl_ptr = tl.0.as_pm_ptr();
            let t_a = new_timer(10);
            let t_b = new_timer(20);
            let t_c = new_timer(30);

            insert_timer(&mut tl, &t_a);
            insert_timer(&mut tl, &t_b);
            insert_timer(&mut tl, &t_c);

            match removed {
                0 => {
                    may_crashed_timer_list_remove(cp, &mut tl, t_a.get_list_node());
                }
                1 => {
                    may_crashed_timer_list_remove(cp, &mut tl, t_b.get_list_node());
                }
                2 => {
                    may_crashed_timer_list_remove(cp, &mut tl, t_c.get_list_node());
                }
                _ => {
                    panic!("invalid removed node");
                }
            };

            let log = ListTxOpLog::get_timer_list_tx_op_log();
            recover_and_roll_forward_of_activelist_remove(log);

            let na = t_a.get_list_node().as_ref();
            let nb = t_b.get_list_node().as_ref();
            let nc = t_c.get_list_node().as_ref();
            assert!(tl.0.len == 2);
            if removed == 0 {
                assert!(tl.0.head == Some(t_b.get_list_node()));
                assert!(nb.next == Some(t_c.get_list_node()));
                assert!(nb.prev == None);
                assert!(nc.prev == Some(t_b.get_list_node()));
                assert!(nc.next == None);
                assert!(na.list == None);
            } else if removed == 1 {
                assert!(tl.0.head == Some(t_a.get_list_node()));
                assert!(na.next == Some(t_c.get_list_node()));
                assert!(na.prev == None);
                assert!(nc.prev == Some(t_a.get_list_node()));
                assert!(nc.next == None);
                assert!(nb.list == None);
            } else if removed == 2 {
                assert!(tl.0.head == Some(t_a.get_list_node()));
                assert!(na.next == Some(t_b.get_list_node()));
                assert!(na.prev == None);
                assert!(nb.prev == Some(t_a.get_list_node()));
                assert!(nb.next == None);
                assert!(nc.list == None);
            }
        }
        for rmd in 0..3 {
            for cp in 1..8 {
                os_print!("Removd {}, Testing Crash Point {}\n", rmd, cp);
                run(cp, rmd);
            }
        }
    }

    fn recover_and_roll_forward_of_activelist_pop(log: &mut ListTxOpLog) {
        let opcode = log.get_micro_op();
        let old_len = log.get_old_len();
        let al = unsafe { time::get_timer_active_list() };
        let mut node_ptr = unsafe { log.get_block_node_ptr::<TimerListItem>().unwrap_unchecked() };
        let node = unsafe { node_ptr.as_mut_no_logging() };
        match opcode {
            ListOpCode::PopFront => {
                match node.list {
                    None => {
                        // already popped
                    }
                    Some(mut l) => {
                        let list = unsafe { l.as_mut_no_logging() };
                        list.recover_from_failed_pop_front(node_ptr, old_len);
                    }
                }
            }
            ListOpCode::Invalid => {}
            _ => {
                panic!("Impossible Op");
            }
        }
        log.commit_activelist();
    }

    #[test]
    fn test_timer_list_pop_1() {
        fn run(cp: usize) {
            cleanup();
            let tl = unsafe { time::get_timer_active_list() };
            let tl_ptr = tl.0.as_pm_ptr();
            let t_a = new_timer(10);
            let n_a = t_a.get_list_node().as_ref();
            insert_timer(tl, &t_a);

            may_crashed_timer_list_pop(cp);

            let timer_log = ListTxOpLog::get_timer_list_tx_op_log();
            recover_and_roll_forward_of_activelist_pop(timer_log);

            assert!(tl.0.head == None);
            assert!(tl.0.len == 0);
            assert!(n_a.list == None);
        }
        for cp in 1..8 {
            os_print!("Testing Crash Point {}\n", cp);
            run(cp);
        }
    }

    #[test]
    fn test_timer_list_pop_2() {
        fn run(cp: usize) {
            cleanup();
            let tl = unsafe { time::get_timer_active_list() };
            let tl_ptr = tl.0.as_pm_ptr();
            let t_a = new_timer(10);
            let t_b = new_timer(20);
            let n_a = t_a.get_list_node().as_ref();
            let n_b = t_b.get_list_node().as_ref();

            insert_timer(tl, &t_a);
            insert_timer(tl, &t_b);

            may_crashed_timer_list_pop(cp);

            let timer_log = ListTxOpLog::get_timer_list_tx_op_log();
            recover_and_roll_forward_of_activelist_pop(timer_log);

            assert!(tl.0.head == Some(t_b.get_list_node()));
            assert!(tl.0.len == 1);
            assert!(n_a.list == None);
            assert!(n_b.list == Some(tl_ptr));
            assert!(n_b.next == None);
            assert!(n_b.prev == None);
        }
        for cp in 1..8 {
            os_print!("Testing Crash Point {}\n", cp);
            run(cp);
        }
    }

    pub fn crashed_atomic_roll_forward_pop_reinsert_into_activelist(cp1: usize, cp2: usize) {
        let log = ListTxOpLog::get_timer_list_tx_op_log();
        let al = unsafe { time::get_timer_active_list() };
        let head = al.0.head;
        log.set_block_node_ptr(head);
        compiler_pm_fence();
        log.set_tx_op(ListTxOpCode::ActiveListPopReInsert);
        if cp1 == 0 {
            return;
        }
        compiler_pm_fence();
        let cs = unsafe { CriticalSection::new() };
        let j = unsafe { JournalHandle::new_dummy() };

        let popped = if cp1 == 1 {
            set_list_crash_point(cp2);
            al.0.crashed_optimized_pop_front(&cs, j);
            return;
        } else {
            al.pop_front(&cs, j)
                .map(|n| unsafe { PMPtr::from_mut_ref(n) })
        };

        match popped {
            Some(ptr) => {
                if cp1 == 1 {
                    set_list_crash_point(cp2);
                    crashed_sorted_delay_list_insert(al, ptr);
                } else {
                    al.insert(&cs, j, ptr);
                }
            }
            None => {}
        }
        set_list_crash_point(cp1);
        crash_point!(1);
        log.set_tx_op(ListTxOpCode::ActiveListTxCommitted);
        crash_point!(2);
        compiler_pm_fence();
        // invalidate
        log.micro_op_old_len = 0;
        crash_point!(3);
    }

    pub fn crashed_atomic_roll_forward_remove_reinsert_into_activelist(
        cp1: usize,
        cp2: usize,
        node_ptr: PMPtr<Node<TimerListItem>>,
    ) {
        // log
        let log = ListTxOpLog::get_timer_list_tx_op_log();
        log.set_block_node_ptr(Some(node_ptr));
        compiler_pm_fence();
        log.set_tx_op(ListTxOpCode::ActiveListRemoveReInsert);
        if cp1 == 0 {
            return;
        }
        compiler_pm_fence();
        let cs = unsafe { CriticalSection::new() };
        let j = unsafe { JournalHandle::new_dummy() };
        let al = unsafe { time::get_timer_active_list() };

        if cp1 == 1 {
            set_list_crash_point(cp2);
            al.0.crashed_optimized_remove(node_ptr, &cs, j);
            return;
        } else {
            al.remove(&cs, j, node_ptr);
        }
        if cp1 == 2 {
            set_list_crash_point(cp2);
            crashed_sorted_delay_list_insert(al, node_ptr)
        } else {
            al.insert(&cs, j, node_ptr);
        }
        set_list_crash_point(cp1);
        crash_point!(2);
        log.set_tx_op(ListTxOpCode::ActiveListTxCommitted);
        crash_point!(3);
        compiler_pm_fence();
        // invalidate
        log.micro_op_old_len = 0;
        crash_point!(4);
    }
    #[test]
    fn test_timer_list_remove_reinsert() {
        fn run(cp1: usize, cp2: usize, removed: usize, new_pos: usize) {
            cleanup();
            os_print!("Remove: {}, new_pos: {}", removed, new_pos);
            let tl = unsafe { time::get_timer_active_list() };
            let tl_ptr = tl.0.as_pm_ptr();
            let t_a = new_timer(10);
            let t_b = new_timer(20);
            let t_c = new_timer(30);
            let new_exp = [[10, 25, 35], [5, 20, 35], [5, 15, 30]];
            let exp = new_exp[removed][new_pos];
            let mut tv = vec![&t_a, &t_b, &t_c];
            for t in tv.iter() {
                insert_timer(tl, t);
            }

            let rm = tv[removed].get_list_node();
            let node = unsafe { tv[removed].get_list_node().as_mut_no_logging() };
            tv[removed].set_expiry_time(node, exp as Time);
            crashed_atomic_roll_forward_remove_reinsert_into_activelist(cp1, cp2, rm);
            recover_and_roll_forward_of_activelist_remove_reinsert(
                ListTxOpLog::get_timer_list_tx_op_log(),
            );
            let rmed_t = tv[removed];
            tv.remove(removed);
            tv.insert(new_pos, rmed_t);

            os_print!(
                "New order: {}, {}, {}",
                tv[0].get_expiry_time(),
                tv[1].get_expiry_time(),
                tv[2].get_expiry_time()
            );
            let n0 = tv[0].get_list_node().as_ref();
            let n1 = tv[1].get_list_node().as_ref();
            let n2 = tv[2].get_list_node().as_ref();
            assert!(n0.prev == None);
            assert!(n0.next == Some(tv[1].get_list_node()));
            assert!(n1.prev == Some(tv[0].get_list_node()));
            assert!(n1.next == Some(tv[2].get_list_node()));
            assert!(n2.prev == Some(tv[1].get_list_node()));
            assert!(n2.next == None);
            assert!(tl.0.len == 3);
            assert!(tl.0.head == Some(tv[0].get_list_node()));
        }

        for rm in 0..3 {
            for np in 0..3 {
                for cp1 in 1..3 {
                    if cp1 == 1 {
                        for cp2 in 1..8 {
                            os_print!("Testing crash point 1: {}, crash point 2: {}", cp1, cp2);
                            run(cp1, cp2, rm, np);
                        }
                    } else {
                        for cp2 in 0..8 {
                            os_print!("Testing crash point 1: {}, crash point 2: {}", cp1, cp2);
                            run(cp1, cp2, rm, np);
                        }
                    }
                }
                run(NO_CRASH, NO_CRASH, rm, np);
            }
        }
    }

    #[test]
    fn test_timer_list_pop_reinsert_1() {
        fn run(cp1: usize, cp2: usize, new_pos: usize) {
            cleanup();
            os_print!("new_pos: {}", new_pos);
            let tl = unsafe { time::get_timer_active_list() };
            let tl_ptr = tl.0.as_pm_ptr();
            let t_a = new_timer(10);
            let t_b = new_timer(20);
            let t_c = new_timer(30);
            let new_exp = [10, 25, 35];
            let exp = new_exp[new_pos];
            let mut tv = vec![&t_a, &t_b, &t_c];
            for t in tv.iter() {
                insert_timer(tl, t);
            }

            let node = unsafe { tv[0].get_list_node().as_mut_no_logging() };
            tv[0].set_expiry_time(node, exp as Time);
            crashed_atomic_roll_forward_pop_reinsert_into_activelist(cp1, cp2);
            recover_and_roll_forward_of_activelist_pop_reinsert(
                ListTxOpLog::get_timer_list_tx_op_log(),
            );
            let h = tv[0];
            tv.remove(0);
            tv.insert(new_pos, h);

            os_print!(
                "New order: {}, {}, {}",
                tv[0].get_expiry_time(),
                tv[1].get_expiry_time(),
                tv[2].get_expiry_time()
            );
            let n0 = tv[0].get_list_node().as_ref();
            let n1 = tv[1].get_list_node().as_ref();
            let n2 = tv[2].get_list_node().as_ref();
            assert!(n0.prev == None);
            assert!(n0.next == Some(tv[1].get_list_node()));
            assert!(n1.prev == Some(tv[0].get_list_node()));
            assert!(n1.next == Some(tv[2].get_list_node()));
            assert!(n2.prev == Some(tv[1].get_list_node()));
            assert!(n2.next == None);
            assert!(tl.0.len == 3);
            assert!(tl.0.head == Some(tv[0].get_list_node()));
        }

        for np in 0..3 {
            for cp1 in 1..3 {
                if cp1 == 1 {
                    for cp2 in 1..8 {
                        os_print!("Testing crash point 1: {}, crash point 2: {}", cp1, cp2);
                        run(cp1, cp2, np);
                    }
                } else {
                    for cp2 in 0..8 {
                        os_print!("Testing crash point 1: {}, crash point 2: {}", cp1, cp2);
                        run(cp1, cp2, np);
                    }
                }
            }
            run(NO_CRASH, NO_CRASH, np);
        }
    }
    #[test]
    fn test_timer_list_pop_reinsert_2() {
        fn run(cp1: usize, cp2: usize) {
            cleanup();
            let tl = unsafe { time::get_timer_active_list() };
            let tl_ptr = tl.0.as_pm_ptr();
            let t_a = new_timer(10);
            insert_timer(tl, &t_a);

            crashed_atomic_roll_forward_pop_reinsert_into_activelist(cp1, cp2);
            recover_and_roll_forward_of_activelist_pop_reinsert(
                ListTxOpLog::get_timer_list_tx_op_log(),
            );

            let n = t_a.get_list_node().as_ref();

            assert!(n.prev == None);
            assert!(n.next == None);
            assert!(tl.0.len == 1);
            assert!(tl.0.head == Some(t_a.get_list_node()));
        }

        for cp1 in 1..5 {
            if cp1 == 1 {
                for cp2 in 1..8 {
                    os_print!("Testing crash point 1: {}, crash point 2: {}", cp1, cp2);
                    run(cp1, cp2);
                }
            } else {
                for cp2 in 0..8 {
                    os_print!("Testing crash point 1: {}, crash point 2: {}", cp1, cp2);
                    run(cp1, cp2);
                }
            }
        }
        run(NO_CRASH, NO_CRASH);
    }
}
