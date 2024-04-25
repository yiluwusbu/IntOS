use vcell::VolatileCell;

use crate::critical::{self, CriticalSection};
use crate::heap::{self, create_per_task_pm_heap, MemStat, PMHeap, PerTaskPMBumpAllocator};
use crate::list::{self, CircularPList, InsertSortedPList, Node, SortedPList};
use crate::marker::TxInSafe;
use crate::pmem::{Journal, JournalHandle, PMPtr, PVolatilePtr};
use crate::recover::{current_generation, get_boot_tx};
use crate::syscalls::SyscallReplayCache;
use crate::time::{Time, MAX_DELAY_TIME, TIME_MANAGER};
use crate::transaction::{self, run, Transaction, TxCache, UserTxInfo};
use crate::util::{benchmark_clock, bubble_sort, get_time_diff, max, min, pretty_print_task_stats};
use core::arch::asm;
use core::cell::UnsafeCell;
use core::fmt::{self, Display};
use core::mem;
use core::ptr::NonNull;

use crate::pmem::PMVar;

use crate::{
    arch, board, debug_print, debug_print_no_header, declare_pm_var, declare_pm_var_array,
    declare_pm_var_unsafe, get_time_diff, os_print, os_print_no_header, recover, task_print,
};

pub type TaskFunc = fn();
pub type TaskSchedNode = Node<SchedListItem>;
pub type TaskEventNode = Node<BlockedListItem>;
// #[cfg(all(any(bench_task = "mq", bench_task = "mq_base"), board = "msp430fr5994"))]
// pub const TASK_NUM_LIMIT: usize = 5;
// #[cfg(all(any(bench_task = "em", bench_task = "em_base"), board = "msp430fr5994"))]
// pub const TASK_NUM_LIMIT: usize = 4;
// #[cfg(all(
//     any(bench_task = "etl", bench_task = "stats", bench_task = "pred",),
//     board = "msp430fr5994"
// ))]
// pub const TASK_NUM_LIMIT: usize = 6;
// #[cfg(all(bench_task = "train", board = "msp430fr5994"))]
// pub const TASK_NUM_LIMIT: usize = 5;
// #[cfg(all(
//     not(any(
//         bench_task = "mq",
//         bench_task = "em",
//         bench_task = "mq_base",
//         bench_task = "em_base",
//         bench_task = "etl",
//         bench_task = "stats",
//         bench_task = "pred",
//         bench_task = "train"
//     )),
//     board = "msp430fr5994"
// ))]
// pub const TASK_NUM_LIMIT: usize = 3;
// #[cfg(not(board = "msp430fr5994"))]
// pub const TASK_NUM_LIMIT: usize = 6;
pub const TASK_NUM_LIMIT: usize = board::TASK_NUM_LIMIT;
const STACK_SIZE: usize = board::STACK_SIZE;
const TASK_STRUCT_SIZE: usize = mem::size_of::<Task>();
const NUM_PRIORITY_LEVELS: usize = 8;
const MIN_PRIORITY: usize = NUM_PRIORITY_LEVELS;

// #[link_section = ".pmem"]
// static mut TASK_LISTS: [PMVar<CircularPList<SchedListItem>>; NUM_PRIORITY_LEVELS] =
//     [unsafe { PMVar::new(CircularPList::new())},unsafe { PMVar::new(CircularPList::new())},unsafe { PMVar::new(CircularPList::new())},unsafe { PMVar::new(CircularPList::new())},
//     unsafe { PMVar::new(CircularPList::new())},unsafe { PMVar::new(CircularPList::new())},unsafe { PMVar::new(CircularPList::new())},unsafe { PMVar::new(CircularPList::new())}];

declare_pm_var_array!(
    TASK_LISTS,
    CircularPList<SchedListItem>,
    NUM_PRIORITY_LEVELS,
    CircularPList::new()
);

// static mut DELAYED_TASK_LIST: PMVar<SortedPList<SchedListItem>> = unsafe { PMVar::new(SortedPList::new()) };
declare_pm_var!(
    DELAYED_TASK_LIST,
    SortedPList<SchedListItem>,
    SortedPList::new()
);

static mut USER_STACKS: [[usize; STACK_SIZE]; TASK_NUM_LIMIT] = [[0; STACK_SIZE]; TASK_NUM_LIMIT];
// declare_pm_var_unsafe!(USER_STACKS,  [[u32; STACK_SIZE]; TASK_NUM_LIMIT], [[0; STACK_SIZE]; TASK_NUM_LIMIT]);
// static mut TASK_STRUCTS: [[u8; TASK_STRUCT_SIZE]; TASK_NUM_LIMIT] =
//     [[0; TASK_STRUCT_SIZE]; TASK_NUM_LIMIT];
declare_pm_var_unsafe!(
    TASK_STRUCTS,
    [[u8; TASK_STRUCT_SIZE]; TASK_NUM_LIMIT],
    [[0; TASK_STRUCT_SIZE]; TASK_NUM_LIMIT]
);

// static mut TASK_ARRAY: [Option<PMPtr<Task>>; TASK_NUM_LIMIT] = [None; TASK_NUM_LIMIT];
declare_pm_var_unsafe!(
    TASK_ARRAY,
    [Option<PMPtr<Task>>; TASK_NUM_LIMIT],
    [None; TASK_NUM_LIMIT]
);
// static mut CUR_MAX_PRIORITY: PMVar<Priority> = unsafe { PMVar::new(Priority::min_priority()) };
declare_pm_var!(CUR_MAX_PRIORITY, Priority, Priority::min_priority());
// static mut SCHEDULER_STARTED: PMVar<bool> = unsafe { PMVar::new(false) };
// declare_pm_var!(pub, MAGIC, u32, 0xEFEF);
static mut SCHEDULER_STARTED: bool = false;
// static mut TASK_CNT: PMVar<usize> = unsafe { PMVar::new(0) };
declare_pm_var!(TASK_CNT, usize, 0);

#[no_mangle]
#[link_section = ".pmem"]
static mut CURRENT_TASK_PTR: PVolatilePtr<Task> = PVolatilePtr::new(core::ptr::null_mut());

// #[link_section = ".magic"]
// pub static mut MAIN_STACK_COOKIE: usize = 0xABCDABCD;
pub unsafe fn get_task_cnt() -> usize {
    *TASK_CNT.borrow_mut_no_logging()
}

pub unsafe fn get_delay_list() -> &'static mut SortedPList<SchedListItem> {
    DELAYED_TASK_LIST.borrow_mut_no_logging()
}

pub unsafe fn get_sched_list(prio: Priority) -> &'static mut CircularPList<SchedListItem> {
    let prio = prio.get_value();
    TASK_LISTS[prio].borrow_mut_no_logging()
}

pub fn debug_display_sched_list(prio: usize) {
    os_print_no_header!("Sched List at Priority {}: \n {}", prio, unsafe {
        *TASK_LISTS[prio]
    });
}

pub fn debug_display_all_sched_list() {
    for i in 0..NUM_PRIORITY_LEVELS {
        os_print_no_header!("Sched List at Priority {}: \n {}", i, unsafe {
            *TASK_LISTS[i]
        });
    }
}

pub fn debug_display_delayed_list() {
    os_print_no_header!("Delayed List: \n {}", unsafe { *DELAYED_TASK_LIST });
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ErrorCode {
    InvalidCode,
    NoSpace,
    InvalidParam,
    TxRetry,
    TxExit,
    TxFatal,
}

#[derive(PartialEq, Eq, Copy, Clone, Debug)]
pub enum TaskState {
    Ready,
    Blocked,
    Running,
    Killed,
}

#[derive(Eq, Clone, Copy)]
pub struct SchedListItem {
    task: PMPtr<Task>,
    wakeup_time: Time,
}

impl SchedListItem {
    #[inline(always)]
    pub fn get_task(&self) -> &Task {
        self.task.as_ref()
    }

    #[inline(always)]
    pub fn get_mut_task(&mut self) -> &mut Task {
        unsafe { self.task.as_mut_no_logging() }
    }
}

impl Display for SchedListItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.task.as_ref())
    }
}

impl PartialEq for SchedListItem {
    fn eq(&self, other: &Self) -> bool {
        self.wakeup_time.eq(&other.wakeup_time)
    }
}

impl PartialOrd for SchedListItem {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        self.wakeup_time.partial_cmp(&other.wakeup_time)
    }
}

impl Ord for SchedListItem {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.wakeup_time.cmp(&other.wakeup_time)
    }
}

#[derive(Eq, Clone, Copy)]
pub struct BlockedListItem {
    task: PMPtr<Task>,
    opaque: usize,
}

impl Display for BlockedListItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.task.as_ref())
    }
}

impl BlockedListItem {
    #[inline(always)]
    pub fn get_task(&self) -> &Task {
        self.task.as_ref()
    }

    #[inline(always)]
    pub fn get_mut_task(&mut self) -> &mut Task {
        unsafe { self.task.as_mut_no_logging() }
    }

    pub fn get_opaque_value(&self) -> usize {
        self.opaque
    }

    pub fn set_opaque_value(&mut self, val: usize) {
        self.opaque = val;
    }
}

impl PartialEq for BlockedListItem {
    fn eq(&self, other: &Self) -> bool {
        let task = self.task.as_ref();
        let other = other.task.as_ref();
        task.priority == other.priority
    }
}

impl PartialOrd for BlockedListItem {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        let task = self.task.as_ref();
        let other = other.task.as_ref();
        task.priority.partial_cmp(&other.priority)
    }
}

impl Ord for BlockedListItem {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        let task = self.task.as_ref();
        let other = other.task.as_ref();
        task.priority.cmp(&other.priority)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Priority {
    value: usize,
}

impl PartialOrd for Priority {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        // higher value has lower priority
        other.value.partial_cmp(&self.value)
    }
}

impl Ord for Priority {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        // higher value has lower priority
        other.value.cmp(&self.value)
    }
}

impl Priority {
    pub fn new(val: usize) -> Self {
        Priority { value: val }
    }

    pub fn is_higher_than(&self, other: &Self) -> bool {
        self.value < other.value
    }

    pub fn is_lower_than(&self, other: &Self) -> bool {
        self.value > other.value
    }

    pub const fn min_priority() -> Self {
        Priority {
            value: MIN_PRIORITY,
        }
    }
    pub const fn max_priority() -> Self {
        Priority { value: 0 }
    }
    #[inline(always)]
    pub fn get_value(&self) -> usize {
        self.value
    }
}

#[derive(Clone, Copy)]
pub struct TaskHandle(usize);

impl TaskHandle {
    pub fn get_task_ptr(&self) -> Option<PMPtr<Task>> {
        unsafe { TASK_ARRAY[self.0] }
    }
}

#[repr(C)]
pub struct Task {
    stack_top: usize,
    priority: Priority,
    task_id: usize,
    name: &'static str,
    task_func: usize,
    param: usize,
    status: UnsafeCell<PMVar<TaskState>>,
    stack_bottom: usize,
    tx: Transaction,
    journal: Journal,
    syscall_tx_cache: TxCache,
    syscall_replay_cache: SyscallReplayCache,
    list_tx_done: bool,
    user_tx_info: UserTxInfo,
    generation: usize,
    recovery_mode: bool,
    pm_heap: PMHeap<PerTaskPMBumpAllocator>,
    sched_node: TaskSchedNode,
    event_node: TaskEventNode,
}

pub unsafe fn is_scheduler_started() -> bool {
    SCHEDULER_STARTED
}

pub unsafe fn set_scheduler_started() {
    SCHEDULER_STARTED = true;
}

pub unsafe fn reset_scheduler_started() {
    SCHEDULER_STARTED = false;
}

unsafe impl TxInSafe for Task {}

impl Task {
    pub unsafe fn alloc_static(id: usize) -> (PMPtr<Task>, usize) {
        unsafe {
            let task_ptr = &mut TASK_STRUCTS[id][0] as *mut u8 as *mut Task;
            let task_ptr = PMPtr::from_ptr(task_ptr);
            let stack_ptr = &mut USER_STACKS[id][STACK_SIZE - 1] as *mut usize as usize;
            TASK_ARRAY[id] = Some(task_ptr);
            (task_ptr, stack_ptr)
        }
    }

    pub fn initialize_task(
        task: &mut Task,
        tid: usize,
        stack_ptr: usize,
        name: &'static str,
        prio: usize,
        func: usize,
        param: usize,
    ) {
        let task_pmptr = unsafe { PMPtr::from_mut_ref(task) };
        task.task_id = tid;
        task.stack_top = stack_ptr;
        task.stack_bottom = stack_ptr;
        task.param = param;
        task.sched_node = TaskSchedNode {
            prev: None,
            next: None,
            value: SchedListItem {
                task: task_pmptr,
                wakeup_time: 0,
            },
            list: None,
        };
        task.event_node = TaskEventNode {
            prev: None,
            next: None,
            value: BlockedListItem {
                task: task_pmptr,
                opaque: 0,
            },
            list: None,
        };
        task.pm_heap = PMHeap::new(PerTaskPMBumpAllocator::new());
        task.journal.init();
        task.tx = Transaction::new(
            unsafe { JournalHandle::new(&task.journal as *const Journal) },
            unsafe { PMPtr::from_ref(&task.syscall_tx_cache) },
        );
        task.syscall_tx_cache.init();
        task.syscall_replay_cache.reset();
        task.list_tx_done = false;
        task.user_tx_info.init();
        task.task_func = func;
        task.name = name;
        task.priority = Priority::new(prio);
        task.status = unsafe { UnsafeCell::new(PMVar::new(TaskState::Ready)) };
        task.recovery_mode = false;
        task.generation = current_generation();
        // initialize the stack by caling arch specific init function
        task.stack_top = arch::initialize_stack(task.stack_top, task.task_func, param);
    }

    pub fn get_task_id(&self) -> usize {
        self.task_id
    }

    pub fn get_syscall_replay_cache(&mut self) -> &mut SyscallReplayCache {
        &mut self.syscall_replay_cache
    }

    pub fn user_tx_start(&mut self) {
        self.syscall_replay_cache.restart();
    }

    pub fn stack_size(&self) -> usize {
        self.stack_bottom - self.stack_top
    }

    pub fn user_tx_end(&mut self) {
        self.syscall_replay_cache.reset();
    }

    #[inline(always)]
    pub fn user_tx_group_start(&mut self) -> bool {
        self.user_tx_info.enter_idempotent()
    }

    #[inline(always)]
    pub fn user_tx_group_end(&mut self) {
        self.user_tx_info.exit_idempotent();
    }

    #[inline(always)]
    pub fn user_idempotent_loop_start(&mut self) {
        self.user_tx_info.enter_idempotent_loop();
    }

    #[inline(always)]
    pub fn user_idempotent_loop_end(&mut self) {
        self.user_tx_info.exit_idempotent_loop();
    }

    pub fn restart_syscall_tx_cache(&mut self) {
        self.syscall_tx_cache.reset_ptr();
    }

    pub fn commit_syscall_tx_cache(&mut self) {
        self.syscall_tx_cache.commit_reset_tail();
    }

    pub fn get_mut_tx(&mut self) -> &mut Transaction {
        &mut self.tx
    }

    pub fn get_mut_user_tx(&mut self) -> &mut Transaction {
        self.user_tx_info.get_tx()
    }

    pub fn get_sys_tx_cache(&mut self) -> &mut TxCache {
        &mut self.syscall_tx_cache
    }

    pub fn get_user_tx_cache(&mut self) -> &mut TxCache {
        self.user_tx_info.get_tx_cache()
    }

    pub fn get_user_tx_info(&self) -> &UserTxInfo {
        &self.user_tx_info
    }

    pub fn get_mut_user_tx_info(&mut self) -> &mut UserTxInfo {
        &mut self.user_tx_info
    }

    pub fn get_pm_heap(&mut self) -> &mut PMHeap<PerTaskPMBumpAllocator> {
        &mut self.pm_heap
    }

    #[inline(always)]
    pub fn is_ready(&self) -> bool {
        let status = &unsafe { *self.status.get() };
        **status == TaskState::Ready
    }

    #[inline(always)]
    pub fn is_running(&self) -> bool {
        let status = &unsafe { *self.status.get() };
        **status == TaskState::Running
    }

    #[inline(always)]
    pub fn is_schedulable(&self) -> bool {
        self.is_ready() || self.is_running()
    }

    #[inline(always)]
    #[cfg(not(feature = "opt_list"))]
    pub fn set_status(&self, status: TaskState, j: JournalHandle) {
        let status_pm = &mut unsafe { *self.status.get() };
        status_pm.set(status, j);
    }

    #[inline(always)]
    #[cfg(feature = "opt_list")]
    pub fn set_status(&self, status: TaskState, _j: JournalHandle) {
        unsafe {
            let status_pm_var = &mut (*self.status.get());
            *status_pm_var.borrow_mut_no_logging() = status;
        };
    }

    #[inline(always)]
    pub fn get_status(&self) -> TaskState {
        let status = &unsafe { *self.status.get() };
        **status
    }

    #[inline(always)]
    pub fn get_name(&self) -> &'static str {
        self.name
    }

    pub fn get_pm_heap_stat(&self) -> MemStat {
        self.pm_heap.stat()
    }

    pub fn set_wakeup_time(&mut self, time: Time) {
        self.sched_node.value.wakeup_time = time;
    }

    pub fn set_block_item_value(&mut self, opaque: usize) {
        self.event_node.value.opaque = opaque;
    }

    pub fn reset_block_item_value(&mut self) -> usize {
        let ret = self.event_node.value.opaque;
        self.event_node.value.opaque = 0;
        ret
    }

    pub fn get_block_item_value(&mut self) -> usize {
        let ret = self.event_node.value.opaque;
        ret
    }

    pub fn get_wakeup_time(&self) -> Time {
        self.sched_node.value.wakeup_time
    }

    #[inline(always)]
    pub fn is_crashed(&self) -> bool {
        self.generation != current_generation()
    }

    pub fn is_awaken(&self) -> bool {
        TIME_MANAGER.get_ticks() >= self.get_wakeup_time()
    }

    pub fn get_sched_node_ptr(&self) -> PMPtr<TaskSchedNode> {
        unsafe { PMPtr::from_ref(&self.sched_node) }
    }

    pub fn get_event_node(&mut self) -> &TaskEventNode {
        &self.event_node
    }

    pub fn get_event_node_ptr(&self) -> PMPtr<TaskEventNode> {
        unsafe { PMPtr::from_ref(&self.event_node) }
    }

    pub fn as_pm_ptr(&self) -> PMPtr<Self> {
        unsafe { PMPtr::from_ref(self) }
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn add_node_to_delayed_list(
        &self,
        n: &mut Node<SchedListItem>,
        j: JournalHandle,
        cs: &CriticalSection,
    ) {
        self.set_status(TaskState::Blocked, j);
        unsafe {
            DELAYED_TASK_LIST.borrow_mut(j).insert_node(cs, j, n);
        }
        let unblock_time = self.get_wakeup_time();
        if unblock_time < TIME_MANAGER.get_next_unblock_time() {
            TIME_MANAGER.set_next_unblock_time(unblock_time);
        }
    }

    #[cfg(not(feature = "opt_list"))]
    #[inline(always)]
    pub fn add_to_delayed_list(&self, j: JournalHandle, cs: &CriticalSection) {
        let mut sched_node_ptr = self.get_sched_node_ptr();
        let n = sched_node_ptr.as_mut(j);
        self.add_node_to_delayed_list(n, j, cs)
    }

    #[cfg(feature = "opt_list")]
    #[inline(always)]
    pub fn add_to_delayed_list(&self, j: JournalHandle, cs: &CriticalSection) {
        let sched_node_ptr = self.get_sched_node_ptr();
        self.set_status(TaskState::Blocked, j);
        unsafe {
            DELAYED_TASK_LIST
                .borrow_mut_no_logging()
                .insert(cs, j, sched_node_ptr);
        }
        let unblock_time = self.get_wakeup_time();
        if unblock_time < TIME_MANAGER.get_next_unblock_time() {
            TIME_MANAGER.set_next_unblock_time(unblock_time);
        }
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn remove_from_delayed_list(
        &self,
        j: JournalHandle,
        cs: &CriticalSection,
    ) -> &mut Node<SchedListItem> {
        let sched_node_ptr = self.get_sched_node_ptr();
        unsafe {
            DELAYED_TASK_LIST
                .borrow_mut(j)
                .remove(cs, j, sched_node_ptr)
        }
    }

    #[cfg(feature = "opt_list")]
    pub fn remove_from_delayed_list(&self, j: JournalHandle, cs: &CriticalSection) {
        let sched_node_ptr = self.get_sched_node_ptr();
        unsafe {
            DELAYED_TASK_LIST
                .borrow_mut_no_logging()
                .remove(cs, j, sched_node_ptr)
        }
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn add_node_to_ready_list(
        &self,
        n: &mut Node<SchedListItem>,
        j: JournalHandle,
        cs: &CriticalSection,
    ) {
        unsafe {
            TASK_LISTS[self.priority.get_value()]
                .borrow_mut(j)
                .insert_node(cs, j, n);
        }
        self.set_status(TaskState::Ready, j);
    }

    #[cfg(not(feature = "opt_list"))]
    #[inline(always)]
    pub fn add_to_ready_list(&self, j: JournalHandle, cs: &CriticalSection) {
        let n = self.get_sched_node_ptr().as_mut(j);
        self.add_node_to_ready_list(n, j, cs);
    }

    #[cfg(feature = "opt_list")]
    #[inline(always)]
    pub fn add_to_ready_list(&self, j: JournalHandle, cs: &CriticalSection) {
        let n = self.get_sched_node_ptr();
        unsafe {
            TASK_LISTS[self.priority.get_value()]
                .borrow_mut_no_logging()
                .insert(cs, j, n);
        }
        self.set_status(TaskState::Ready, j);
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn remove_from_ready_list(
        &self,
        j: JournalHandle,
        cs: &CriticalSection,
    ) -> &mut Node<SchedListItem> {
        let sched_node_ptr = self.get_sched_node_ptr();
        unsafe {
            TASK_LISTS[self.priority.get_value()]
                .borrow_mut(j)
                .remove(cs, j, sched_node_ptr)
        }
    }

    #[cfg(feature = "opt_list")]
    pub fn remove_from_ready_list(&self, j: JournalHandle, cs: &CriticalSection) {
        let sched_node_ptr = self.get_sched_node_ptr();
        unsafe {
            TASK_LISTS[self.priority.get_value()]
                .borrow_mut_no_logging()
                .remove(cs, j, sched_node_ptr)
        }
    }

    pub fn less_important_than(&self, other: &Self) -> bool {
        self.priority < other.priority
    }

    pub fn get_priority(&self) -> Priority {
        self.priority
    }

    pub fn user_recovery(&mut self) {
        let user_tx = self.get_mut_user_tx();
        if user_tx.check_committed().is_ok() {
            user_tx.reset_nesting_level();
            // reset the system call cache table
            self.user_tx_end();
        } else {
            user_tx.roll_back();
            user_tx.reset_nesting_level();
        }

        // reset user tx/idem ptrs, stack top
        self.user_tx_info.restart();
    }

    pub fn debug_user_tx(&self) {
        self.user_tx_info.show_user_tx_idem_status();
    }

    pub fn bench_reset_user_tx(&mut self) {
        self.user_tx_info.reset_all();
        self.pm_heap.reset();
    }

    pub fn bench_reset_user_pm_heap(&mut self) {
        self.pm_heap.reset();
    }

    pub fn in_recovery_mode(&self) -> bool {
        self.recovery_mode
    }

    pub fn reset_list_transaction(&mut self) {
        self.list_tx_done = false;
    }

    pub fn complete_list_transaction(&mut self) {
        self.list_tx_done = true;
    }

    pub fn list_transaction_done(&self) -> bool {
        self.list_tx_done
    }

    pub fn start_stat(&self) {
        let tid = self.task_id;
        let stat = unsafe { TASK_STATS.get_stats_of_task(tid) };
        stat.stat_started = true;
    }

    pub fn stop_stat(&self) {
        let tid = self.task_id;
        let stat = unsafe { TASK_STATS.get_stats_of_task(tid) };
        critical::with_no_interrupt(|cs| {
            if stat.stat_started {
                stat.stat_started = false;
                let clock = benchmark_clock();
                // os_print!("Clock: {}, enter_time: {}, last_sched_time: {}, delta: {}", clock,  stat.user_enter_time, stat.last_sched_time, clock - stat.last_sched_time);
                stat.user_time += get_time_diff!(clock, stat.user_enter_time);
                stat.total_run_time += get_time_diff!(clock, stat.last_sched_time);
            }
        });
    }

    pub fn jit_recovery(&mut self) {
        if self.is_crashed() {
            task_recovery_begin_stat(self);
            // panic!("Impossible to do JIT recovery: task gen {}, global gen {}", self.generation, current_generation() );
            self.recovery_mode = true;
            // set generation to current one
            debug_print!("Performing JIT recovery for task {}", self.name);
            self.tx.roll_back_if_uncommitted();
            self.tx.reset_nesting_level();
            let syscall_replay_cache = self.get_syscall_replay_cache();
            // This indicates a incomplete syscall return
            if syscall_replay_cache.get_ptr() < syscall_replay_cache.get_tail() {
                debug_print!("Detected crash just before syscall return");
                self.commit_syscall_tx_cache();
                self.reset_list_transaction();
            }
            #[cfg(feature = "opt_loop_end")]
            {
                let tx_cache = self.user_tx_info.get_tx().get_cache();
                if tx_cache.get_tx_id_of_ptr() > tx_cache.get_tx_id_of_tail() {
                    debug_print!("Detected crash just before nvloop end");
                    self.user_tx_info.set_loop_cnt();
                }
            }
            // recover user TX
            self.user_recovery();
            // reinitialize the task stack since it is volatile
            self.stack_top = arch::initialize_stack(self.stack_bottom, self.task_func, self.param);
            // mark the completion of recovery
            self.generation = current_generation();
            task_recovery_end_stat(self);
        }
    }
}

impl fmt::Display for Task {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "<task: {}, prio: {}, status: {:?}>",
            self.name,
            self.priority.get_value(),
            self.get_status()
        )
    }
}

#[cfg(not(feature = "opt_list"))]
pub fn create_task_static(
    name: &'static str,
    prio: usize,
    func: usize,
    param: usize,
    pmem_heap_sz: usize,
) -> Result<TaskHandle, ErrorCode> {
    // TODO:  critical section can be shorter
    critical::with_no_interrupt(|cs| {
        transaction::run(|j| {
            let task_cnt = unsafe { TASK_CNT.borrow_mut(j) };
            let tid = *task_cnt;

            if tid >= 8 {
                os_print!("Maximun number of tasks reached, current number: {}", tid);
                return Err(ErrorCode::NoSpace);
            }

            if prio >= MIN_PRIORITY {
                os_print!("Invalid priority for task: {}", name);
                return Err(ErrorCode::InvalidParam);
            }

            *task_cnt += 1;

            // allocate TCB & stack statically
            let (mut task, stack_ptr) = unsafe { Task::alloc_static(tid) };
            // init TCB & stack
            let task = unsafe { task.as_mut_no_logging() };
            Task::initialize_task(task, tid, stack_ptr, name, prio, func, param);
            if let Err(_) = unsafe { create_per_task_pm_heap(&mut task.pm_heap, pmem_heap_sz, j) } {
                os_print!("No Space for per task PM Heap");
            }
            unsafe {
                if !is_scheduler_started() {
                    if task.priority.is_higher_than(&CUR_MAX_PRIORITY) {
                        CURRENT_TASK_PTR.store(task as *mut Task, j);
                        *CUR_MAX_PRIORITY.borrow_mut(j) = task.priority;
                    }
                }
            }

            task.add_to_ready_list(j, cs);
            Ok(TaskHandle(tid))
        })
    })
}

#[cfg(feature = "opt_list")]
pub fn create_task_static(
    name: &'static str,
    prio: usize,
    func: usize,
    param: usize,
    pmem_heap_sz: usize,
) -> Result<TaskHandle, ErrorCode> {
    // TODO:  critical section can be shorter
    critical::with_no_interrupt(|cs| {
        let r = transaction::run(|j| {
            let task_cnt = unsafe { TASK_CNT.borrow_mut(j) };
            let tid = *task_cnt;

            if tid >= TASK_NUM_LIMIT {
                os_print!("Maximun number of tasks reached, current number: {}", tid);
                return Err(ErrorCode::NoSpace);
            }

            if prio >= MIN_PRIORITY {
                os_print!("Invalid priority for task: {}", name);
                return Err(ErrorCode::InvalidParam);
            }

            *task_cnt += 1;

            // allocate TCB & stack statically
            let (mut task, stack_ptr) = unsafe { Task::alloc_static(tid) };
            // init TCB & stack
            let task = unsafe { task.as_mut_no_logging() };
            Task::initialize_task(task, tid, stack_ptr, name, prio, func, param);
            if let Err(_) = unsafe { create_per_task_pm_heap(&mut task.pm_heap, pmem_heap_sz, j) } {
                os_print!("No Space for per task PM Heap");
            }
            unsafe {
                if !is_scheduler_started() {
                    if task.priority.is_higher_than(&CUR_MAX_PRIORITY) {
                        CURRENT_TASK_PTR.store(task as *mut Task);
                        *CUR_MAX_PRIORITY.borrow_mut_no_logging() = task.priority;
                    }
                }
            }
            Ok(TaskHandle(tid))
        });
        match r {
            Ok(handle) => {
                // os_print!("tid = {}", handle.0);
                // assert!(handle.0 < TASK_NUM_LIMIT);
                // let task = unsafe {TASK_ARRAY[handle.0].unwrap_unchecked().as_mut_no_logging()};
                // os_print!("tid = {}", handle.0);

                let task = unsafe {
                    TASK_ARRAY
                        .get_unchecked_mut(handle.0)
                        .unwrap_unchecked()
                        .as_mut_no_logging()
                };
                list::atomic_roll_forward_insert_into_readylist(task, cs);
            }
            _ => {}
        }
        r
    })
}

pub fn register_app_custom<T: Send + 'static>(
    name: &'static str,
    prio: usize,
    func: fn(T),
    param: T,
    pm_heap_sz: usize,
) {
    assert!(core::mem::size_of::<T>() == core::mem::size_of::<usize>());
    let param_usize = unsafe { core::mem::transmute_copy::<T, usize>(&param) };
    // We should not run task's param destructor
    core::mem::forget(param);
    match create_task_static(name, prio, func as usize, param_usize, pm_heap_sz) {
        Ok(_) => {
            #[cfg(feature = "verbose_os_info")]
            os_print!("Task {} created", name);
        }
        Err(e) => {
            #[cfg(feature = "verbose_os_info")]
            os_print!("Failed to create task {}  with code {:?}", name, e);
        }
    }
}

pub fn register_app<T: Send + 'static>(name: &'static str, prio: usize, func: fn(T), param: T) {
    register_app_custom(name, prio, func, param, heap::PM_HEAP_SIZE_PER_TASK);
}

pub fn register_app_no_param_custom(
    name: &'static str,
    prio: usize,
    func: fn(),
    pm_heap_sz: usize,
) {
    match create_task_static(name, prio, func as usize, 0, pm_heap_sz) {
        Ok(_) => {
            os_print!("Task {} created", name);
        }
        Err(e) => {
            os_print!("Failed to create task {}  with code {:?}", name, e);
        }
    }
}

pub fn register_app_no_param(name: &'static str, prio: usize, func: fn()) {
    register_app_no_param_custom(name, prio, func, heap::PM_HEAP_SIZE_PER_TASK);
}

pub fn create_task_static_with_closure<F>(
    name: &'static str,
    prio: usize,
    f: &mut F,
    pm_heap_sz: usize,
) -> Result<TaskHandle, ErrorCode>
where
    F: FnMut(),
{
    let param = f as *mut _ as usize;
    create_task_static(
        name,
        prio,
        closure_task_runner::<F> as usize,
        param,
        pm_heap_sz,
    )
}

pub fn idle_fn() {
    loop {}
}

pub fn closure_task_runner<F>(f: &mut F)
where
    F: FnMut(),
{
    f();
    // should never return to here
    panic!("Should never return to here");
}

pub fn create_idle_task() {
    let res = create_task_static("idle", MIN_PRIORITY - 1, idle_fn as usize, 0, 0);
    match res {
        Err(_) => {
            #[cfg(feature = "verbose_os_info")]
            os_print!("Failed to create idle task...");
            loop {}
        }
        Ok(_) => {
            #[cfg(feature = "verbose_os_info")]
            os_print!("Task Idle is created...");
        }
    }
}

#[cfg(feature = "opt_list")]
#[export_name = "process_tick"]
pub fn process_tick() {
    critical::with_no_interrupt(|cs| {
        let cur_ticks = TIME_MANAGER.inc_tick(1);
        let next_unblock_time = TIME_MANAGER.get_next_unblock_time();

        #[cfg(not(feature = "power_failure"))]
        if cur_ticks % 100 == 0 {
            os_print!(
                "Ticks: {}, next_unblock_time = {}",
                cur_ticks,
                next_unblock_time
            );
        }

        if cur_ticks >= next_unblock_time {
            loop {
                let item = unsafe { DELAYED_TASK_LIST.peek_front(cs) };
                match item {
                    None => break,
                    Some(item) => {
                        let wakeup_time = item.wakeup_time;
                        if wakeup_time > cur_ticks {
                            break;
                        }
                        // debug_print!("removing task {} from delay list", item.task.as_ref().get_name());
                        list::atomic_roll_forward_remove_from_delaylist(cs);
                    }
                }
            }
            // set next unblock time
            let new_next_unblock_time = unsafe {
                DELAYED_TASK_LIST
                    .peek_front(cs)
                    .map_or_else(|| MAX_DELAY_TIME, |item| item.wakeup_time)
            };

            TIME_MANAGER.set_next_unblock_time(new_next_unblock_time);
        }
        #[cfg(all(target_arch = "msp430", not(feature = "power_failure")))]
        {
            unsafe { task_switch() }
        }
    });
}

// Must be called inside a critical section
#[cfg(not(feature = "opt_list"))]
#[export_name = "process_tick"]
pub fn process_tick() {
    critical::with_no_interrupt(|cs| {
        let cur_ticks = TIME_MANAGER.inc_tick(1);
        // let msp: usize;
        // unsafe {
        //     asm!(
        //         "mrs {0}, msp",
        //         out(reg) msp
        //     );
        // }
        // debug_print!("MSP: {}", msp);
        // wake up delayed tasks
        let next_unblock_time = TIME_MANAGER.get_next_unblock_time();
        if cur_ticks % 100 == 0 {
            os_print!(
                "Ticks: {}, next_unblock_time = {}",
                cur_ticks,
                next_unblock_time
            );
            // Debug Code
            // os_print!("Current delay list: ");
            // for item in unsafe { DELAYED_TASK_LIST.iter() } {
            //     let t = item.task.as_ref() ;
            //     os_print!("Task : {}, wakeup_time: {}", t.name, item.wakeup_time);
            // }
        }

        if cur_ticks >= next_unblock_time {
            loop {
                let item = unsafe { DELAYED_TASK_LIST.peek_front(cs) };
                match item {
                    None => break,
                    Some(item) => {
                        let wakeup_time = item.wakeup_time;
                        if wakeup_time > cur_ticks {
                            break;
                        }

                        transaction::run_no_ctx(|j| {
                            // remove task from delayed list
                            #[cfg(not(feature = "opt_list"))]
                            let old_head = unsafe {
                                DELAYED_TASK_LIST
                                    .borrow_mut(j)
                                    .pop_front(cs, j)
                                    .unwrap_unchecked()
                            };
                            // remove task from event list
                            let task = unsafe { old_head.value.task.as_mut_no_logging() };
                            debug_print_no_header!("Removing task {} from delayed list", task.name);
                            task.event_node.list.map(|mut wait_list| {
                                let wait_list = wait_list.as_mut(j);
                                wait_list.remove(cs, j, task.get_event_node_ptr());
                            });
                            // insert it into the ready list
                            unsafe {
                                TASK_LISTS[task.priority.get_value()]
                                    .borrow_mut(j)
                                    .insert_node(cs, j, old_head);
                            }
                            //debug_print!("Updating status...");
                            // update task status
                            task.set_status(TaskState::Ready, j);
                            // debug_display_sched_list(0);
                            //debug_print!("returning from process_tick...");
                        });
                    }
                }
            }
            // set next unblock time
            let new_next_unblock_time = unsafe {
                DELAYED_TASK_LIST
                    .peek_front(cs)
                    .map_or_else(|| MAX_DELAY_TIME, |item| item.wakeup_time)
            };

            TIME_MANAGER.set_next_unblock_time(new_next_unblock_time);
        }
        #[cfg(target_arch = "msp430")]
        {
            unsafe { task_switch() }
        }
    });
}

pub fn task_yield() {
    arch::arch_yield();
}

#[cfg(feature = "opt_list")]
pub fn task_delay(nticks: Time, yield_now: bool) {
    critical::with_no_interrupt(|cs| {
        let task = current();
        let wakeup_time = TIME_MANAGER.get_ticks().checked_add(nticks).unwrap();
        task.set_wakeup_time(wakeup_time);
        list::atomic_roll_forward_insert_into_delaylist(cs);
    });
    // yield immediately
    if yield_now {
        task_yield();
    }
}

#[cfg(feature = "opt_list")]
#[export_name = "task_switch"]
pub unsafe extern "C" fn task_switch() {
    ctx_switch_start_stat();
    let prev_task = CURRENT_TASK_PTR.load().as_mut().unwrap();
    // critical section token
    let cs = CriticalSection::new();
    let j = JournalHandle::new_dummy();
    let mut ok = false;

    // For Timing purpose
    switch_out_task_update_stats(prev_task);
    for i in 0..NUM_PRIORITY_LEVELS {
        match TASK_LISTS[i].peek_next(&cs) {
            None => continue,

            Some(list_item) => {
                let mut task_ptr = list_item.task;
                ok = true;
                // let task = task_ptr.as_ref();
                // os_print!("Switching to task : {}, prio = {}", task.name, task.priority.get_value());
                let task = task_ptr.as_mut_no_logging();
                // For Timing purpose
                switch_to_task_update_stats(task);

                task.jit_recovery();

                if task_ptr.as_ptr() != prev_task as *mut _ {
                    //debug_print_no_header!("Ready to switch to task: {}", task.name);
                    debug_assert!(task.is_schedulable());
                    let readylist = unsafe { TASK_LISTS[i].borrow_mut_no_logging() };
                    list::atomic_roll_foward_context_switch(
                        task,
                        prev_task,
                        readylist,
                        &mut CURRENT_TASK_PTR,
                        j,
                        &cs,
                    );
                } else {
                    // debug_display_sched_list(i);
                    debug_assert!(TASK_LISTS[i].length() == 1)
                }
                break;
            }
        }
    }
    ctx_switch_end_stat();
    if !ok {
        panic!("no task is alive");
    }
}

#[cfg(not(feature = "opt_list"))]
pub fn task_delay(nticks: Time, yield_now: bool) {
    critical::with_no_interrupt(|cs| {
        transaction::run(|j| {
            let task = current();
            let wakeup_time = TIME_MANAGER.get_ticks().checked_add(nticks).unwrap();
            task.set_status(TaskState::Blocked, j);
            task.set_wakeup_time(wakeup_time);
            // remove from the ready list
            let n = task.remove_from_ready_list(j, cs);
            // insert into delayed list
            task.add_node_to_delayed_list(n, j, cs);
        });
    });
    // yield immediately
    if yield_now {
        task_yield();
    }
}

#[cfg(not(feature = "opt_list"))]
#[export_name = "task_switch"]
pub unsafe extern "C" fn task_switch() {
    use crate::recover::{finish_ctx_switch_tx, start_ctx_switch_tx};

    ctx_switch_start_stat();
    start_ctx_switch_tx();
    let prev_task = CURRENT_TASK_PTR.load().as_mut().unwrap();
    // critical section token
    let cs = CriticalSection::new();
    let mut ok = false;

    // For Timing purpose
    switch_out_task_update_stats(prev_task);

    for i in 0..NUM_PRIORITY_LEVELS {
        match TASK_LISTS[i].peek_next(&cs) {
            None => continue,

            Some(list_item) => {
                let mut task_ptr = list_item.task;
                ok = true;
                // let task = task_ptr.as_mut();
                // os_print!("task : {}", task.name).unwrap();
                let task = task_ptr.as_mut_no_logging();
                // For Timing purpose
                switch_to_task_update_stats(task);
                task.jit_recovery();
                if task_ptr.as_ptr() != prev_task as *mut _ {
                    //debug_print_no_header!("Ready to switch to task: {}", task.name);
                    transaction::run_no_ctx(|j| {
                        // os_print!("switching to task : {}", task.name).unwrap();
                        // debug_display_sched_list(i);
                        debug_assert!(task.is_schedulable());
                        task.set_status(TaskState::Running, j);
                        if prev_task.get_status() != TaskState::Blocked {
                            prev_task.set_status(TaskState::Ready, j);
                        }
                        // move the cursor to next task ready to run
                        TASK_LISTS[i].borrow_mut(j).next(&cs);
                        CURRENT_TASK_PTR.store(task_ptr.as_ptr(), j);
                    });
                } else {
                    // debug_display_sched_list(i);
                    debug_assert!(TASK_LISTS[i].length() == 1)
                }
                break;
            }
        }
    }
    finish_ctx_switch_tx();
    ctx_switch_end_stat();

    if !ok {
        panic!("no task is alive");
    }
}

pub fn current() -> &'static mut Task {
    unsafe {
        NonNull::new(CURRENT_TASK_PTR.load())
            .unwrap_unchecked()
            .as_mut()
    }
}

pub fn get_current_tx() -> &'static mut Transaction {
    if !unsafe { is_scheduler_started() } {
        recover::get_boot_tx()
    } else {
        current().get_mut_tx()
    }
}

pub unsafe fn get_current_task_ptr() -> &'static mut PVolatilePtr<Task> {
    &mut CURRENT_TASK_PTR
}

/* ------ Testing Interfaces ----- */
#[cfg(test)]
pub fn mock_task_switch() {
    unsafe {
        task_switch();
    }
}
// Timing related metadata and functions

#[derive(Clone, Copy)]
pub struct TaskStats {
    pub total_run_time: u32,
    pub in_kernel_run_time: u32,
    pub user_time: u32,
    pub last_sched_time: u32,
    pub kernel_enter_time: u32,
    pub user_enter_time: u32,
    in_kernel: bool,
    pub stat_started: bool,
    recovery_begin_time: u32,
    pub total_recovery_time: u32,
    #[cfg(feature = "profile_tx")]
    pub tx_stat: TxStat,
}

struct KernelStat {
    context_switch_start: u32,
    total_switch_time: u32,
    kernel_recovery_time: u32,
    kernel_recovery_begin_time: u32,
}

#[derive(Clone, Copy)]
pub struct TxStat {
    pub usr_tx_start_time: u32,
    pub in_usr_tx: bool,
    pub usr_tx_time_max: u32,
    pub usr_tx_time_min: u32,
    pub usr_tx_time: u32,
    pub usr_tx_time_total: u32,
    pub usr_tx_cnt: usize,
    // pub in_kernel_tx: bool,
    // pub kernel_tx_start_time: u32,
    // pub kernel_tx_time_max: u32,
    // pub kernel_tx_time_min: u32,
    // pub kernel_tx_time_total: u32,
    // pub kernel_tx_cnt: usize,
}

#[derive(Clone, Copy)]
pub struct ListStat {
    pub run_time: u32,
    pub list_op_started: bool,
    pub list_op_start_time: u32,
    pub list_log_sz: u32,
}

impl TxStat {
    pub const fn new() -> Self {
        Self {
            usr_tx_start_time: 0,
            in_usr_tx: false,
            usr_tx_time_max: 0,
            usr_tx_time_min: 0xffffffff,
            usr_tx_time: 0,
            usr_tx_time_total: 0,
            usr_tx_cnt: 0,
        }
    }
}

impl ListStat {
    pub const fn new() -> Self {
        Self {
            run_time: 0,
            list_op_started: false,
            list_op_start_time: 0,
            list_log_sz: 0,
        }
    }
}

static mut KERNEL_STAT: KernelStat = KernelStat {
    context_switch_start: 0,
    total_switch_time: 0,
    kernel_recovery_time: 0,
    kernel_recovery_begin_time: 0,
};

pub fn get_kernel_recovery_time() -> u32 {
    unsafe { KERNEL_STAT.kernel_recovery_time }
}

unsafe fn ctx_switch_start_stat() {
    // CTX_SWITCH_STAT.context_switch_start = benchmark_clock();
}

unsafe fn ctx_switch_end_stat() {
    // let end = benchmark_clock();
    // CTX_SWITCH_STAT.total_switch_time += end - CTX_SWITCH_STAT.context_switch_start;
}

#[cfg(feature = "profile_tx")]
static mut USER_TX_TIME_WIN: [u32; TX_SAMPLE_WIN_SZ] = [0; TX_SAMPLE_WIN_SZ];
#[cfg(feature = "profile_tx")]
static mut USER_TX_CNT: usize = 0;

#[cfg(feature = "profile_tx")]
pub fn sample_user_tx(tx_time: u32) {
    unsafe {
        if USER_TX_CNT >= TX_SAMPLE_WIN_SZ {
            return;
        }
        USER_TX_TIME_WIN[USER_TX_CNT] = tx_time;
        USER_TX_CNT += 1;
    }
}

#[cfg(feature = "profile_tx")]
const TX_SAMPLE_WIN_SZ: usize = 100;

impl TaskStats {
    pub const fn new() -> Self {
        Self {
            total_run_time: 0,
            in_kernel_run_time: 0,
            user_time: 0,
            last_sched_time: 0,
            kernel_enter_time: 0,
            user_enter_time: 0,
            in_kernel: false,
            stat_started: false,
            recovery_begin_time: 0,
            total_recovery_time: 0,
            #[cfg(feature = "profile_tx")]
            tx_stat: TxStat::new(),
        }
    }
}

const TASK_STATS_TABLE_SIZE: usize = 8;
pub struct TaskStatsTable {
    table: [TaskStats; TASK_STATS_TABLE_SIZE],
}

impl TaskStatsTable {
    pub const fn new() -> TaskStatsTable {
        Self {
            table: [TaskStats::new(); TASK_STATS_TABLE_SIZE],
        }
    }

    pub fn get_stats_of_task(&mut self, tid: usize) -> &mut TaskStats {
        &mut self.table[tid]
    }
}

static mut TASK_STATS: TaskStatsTable = TaskStatsTable::new();

// update the last_sched_time, kernel enter time
pub fn switch_to_task_update_stats(task: &Task) {
    let tid = task.task_id;
    let stat = unsafe { TASK_STATS.get_stats_of_task(tid) };
    let clock = benchmark_clock();
    stat.last_sched_time = clock;
    if stat.in_kernel == true {
        stat.kernel_enter_time = clock;
    } else {
        stat.user_enter_time = clock;
    }
    // tx profiling
    #[cfg(feature = "profile_tx")]
    {
        if stat.tx_stat.in_usr_tx == true && !stat.in_kernel {
            stat.tx_stat.usr_tx_start_time = clock;
        }
    }
}

// update the run_time base on states
pub fn switch_out_task_update_stats(task: &Task) {
    // update stats only if it is enabled
    let tid = task.task_id;
    let stat = unsafe { TASK_STATS.get_stats_of_task(tid) };

    if stat.stat_started {
        let clock = benchmark_clock();
        if stat.in_kernel == true {
            stat.in_kernel_run_time += get_time_diff!(clock, stat.kernel_enter_time);
        } else {
            // os_print!("In switch out: clock = {}, last_sched: {}, tick: {}", clock, stat.last_sched_time, crate::time::get_time());
            stat.user_time += get_time_diff!(clock, stat.user_enter_time);
        }
        // os_print!("In switch out: total_run_time += {}, task_name: {}", clock - stat.last_sched_time, task.get_name());
        stat.total_run_time += get_time_diff!(clock, stat.last_sched_time);
        #[cfg(feature = "profile_tx")]
        {
            if stat.tx_stat.in_usr_tx == true && !stat.in_kernel {
                let elapsed = get_time_diff!(clock, stat.tx_stat.usr_tx_start_time);
                stat.tx_stat.usr_tx_time += elapsed;
            }
        }
    }
}

pub fn power_failure_task_update_stats(task: &Task) {
    let tid = task.task_id;
    let stat = unsafe { TASK_STATS.get_stats_of_task(tid) };
    let clock = benchmark_clock();
    if stat.stat_started {
        if stat.in_kernel {
            stat.in_kernel_run_time += get_time_diff!(clock, stat.kernel_enter_time);
        } else {
            stat.user_time += get_time_diff!(clock, stat.user_enter_time);
        }
        // os_print!("In powerfailure: total_run_time += {}, task_name: {}", clock - stat.last_sched_time, task.get_name());
        stat.total_run_time += get_time_diff!(clock, stat.last_sched_time);
    }
}

pub fn kernel_recovery_begin_stat() {
    let clock = benchmark_clock();
    unsafe {
        KERNEL_STAT.kernel_recovery_begin_time = clock;
    }
}

pub fn kernel_recovery_end_stat() {
    let clock = benchmark_clock();
    unsafe {
        let elapsed = get_time_diff!(clock, KERNEL_STAT.kernel_recovery_begin_time);
        KERNEL_STAT.kernel_recovery_time += elapsed;
    }
}

pub fn task_recovery_begin_stat(task: &Task) {
    let tid = task.task_id;
    let stat = unsafe { TASK_STATS.get_stats_of_task(tid) };
    let clock = benchmark_clock();
    // os_print!("Recovery clock: {}", clock);
    stat.in_kernel = false;
    stat.user_enter_time = clock;
    stat.last_sched_time = clock;
    stat.recovery_begin_time = clock;
}

pub fn task_recovery_end_stat(task: &Task) {
    let tid = task.task_id;
    let stat = unsafe { TASK_STATS.get_stats_of_task(tid) };
    let clock = benchmark_clock();
    stat.total_recovery_time += get_time_diff!(clock, stat.recovery_begin_time);
}

pub fn task_enter_kernel(task: &Task) {
    let tid = task.task_id;
    let stat = unsafe { TASK_STATS.get_stats_of_task(tid) };
    critical::with_no_interrupt(|cs| {
        let clock = benchmark_clock();
        assert!(!stat.in_kernel);
        stat.in_kernel = true;
        stat.kernel_enter_time = clock;
        if stat.stat_started {
            stat.user_time += get_time_diff!(clock, stat.user_enter_time);
            #[cfg(feature = "profile_tx")]
            {
                if stat.tx_stat.in_usr_tx == true {
                    let elapsed = get_time_diff!(clock, stat.tx_stat.usr_tx_start_time);
                    stat.tx_stat.usr_tx_time += elapsed;
                }
            }
        }
    });
}

pub fn task_exit_kernel(task: &Task) {
    let tid = task.task_id;
    let stat = unsafe { TASK_STATS.get_stats_of_task(tid) };

    critical::with_no_interrupt(|cs| {
        let exit_time = benchmark_clock();
        let enter_time = stat.kernel_enter_time;
        assert!(stat.in_kernel);
        stat.in_kernel = false;
        stat.user_enter_time = exit_time;
        if stat.stat_started {
            stat.in_kernel_run_time += get_time_diff!(exit_time, enter_time);
        }
        #[cfg(feature = "profile_tx")]
        {
            if stat.tx_stat.in_usr_tx == true {
                stat.tx_stat.usr_tx_start_time = exit_time;
            }
        }
    });
}

pub fn task_get_stats(t: &mut Task) -> &mut TaskStats {
    let tid = t.task_id;
    unsafe { &mut TASK_STATS.table[tid] }
}

pub fn print_ctx_switch_stat() {
    os_print!("[Stat] Context Switch time: {}", unsafe {
        KERNEL_STAT.total_switch_time
    });
}

pub fn print_all_task_stats() {
    let mut total = 0;
    let mut kern = 0;
    let mut user = 0;
    let mut log_time = 0;
    let mut log_sz = 0;
    let mut recovery = 0;
    let kern_recovery = get_kernel_recovery_time();
    // skip idle task (id = 0), so we need to start from 1
    for i in 1..TASK_NUM_LIMIT {
        let task_ptr = unsafe { TASK_ARRAY[i] };
        if !task_ptr.is_none() {
            let task_ptr = task_ptr.unwrap();
            let t = task_ptr.as_ref();

            let stats = unsafe { &TASK_STATS.table[i] };
            total += stats.total_run_time;
            kern += stats.in_kernel_run_time;
            user += stats.user_time;
            recovery += stats.total_recovery_time;
            pretty_print_task_stats(
                t.get_name(),
                stats.total_run_time,
                stats.user_time,
                stats.in_kernel_run_time,
                stats.total_recovery_time,
            );
            // os_print!(
            //     "[Stat] Count task: {}, total: {}, kern: {}, user: {}, recovery: {}",
            //     t.get_name(),
            //     stats.total_run_time,
            //     stats.in_kernel_run_time,
            //     stats.user_time,
            //     stats.total_recovery_time
            // );
        }
    }
    // print_ctx_switch_stat();
    let total_recovery = recovery + kern_recovery;
    pretty_print_task_stats("total", total, user, kern, total_recovery);
    /// Legacy Printing function
    // os_print!(
    //     "[Stat] Total Runtime: {}, User time: {}, In kernel time: {}, Recovery Time: {}",
    //     total,
    //     user,
    //     kern,
    //     recovery
    // );
    // #[cfg(feature = "power_failure")]
    // crate::bench_println!("Runtime + User Recovery: {}, Runtime + Total Recovery: {}, User Recovery: {}, Kernel Recovery: {}, Total Recovery: {}"
    //                     ,total + recovery , total + total_recovery, recovery, kern_recovery, total_recovery);
    #[cfg(feature = "profile_log")]
    os_print!(
        "[Stat] Kernel Log Size: {}, User Log Size: {}",
        crate::pmem::get_klog_sz(),
        crate::pmem::get_ulog_sz()
    );
}
#[cfg(feature = "profile_tx")]
pub fn print_all_task_tx_stats() {
    let mut tx_cnt = 0;
    let mut max_tx_time = 0;
    let mut min_tx_time = 0xffffffff;
    let mut total_tx_time = 0;
    for i in 1..TASK_NUM_LIMIT {
        let task_ptr = unsafe { TASK_ARRAY[i] };
        if !task_ptr.is_none() {
            let task_ptr = task_ptr.unwrap();
            let t = task_ptr.as_ref();
            let stats = unsafe { &TASK_STATS.table[i] };
            tx_cnt += stats.tx_stat.usr_tx_cnt;
            total_tx_time += stats.tx_stat.usr_tx_time_total;
            max_tx_time = max(stats.tx_stat.usr_tx_time_max, max_tx_time);
            min_tx_time = min(stats.tx_stat.usr_tx_time_min, min_tx_time);
        }
    }
    let median = unsafe {
        let win = &mut USER_TX_TIME_WIN[0..USER_TX_CNT];
        bubble_sort(win);
        USER_TX_TIME_WIN[USER_TX_CNT / 2]
    };
    os_print!("[TX Stat]: Max TX Time: {}, Min TX Time: {}, Total Tx Time: {}, Tx Cnt: {}, Avg TX Time: {}, Median: {}", 
    max_tx_time, min_tx_time, total_tx_time, tx_cnt, total_tx_time / tx_cnt as u32, median);
}

pub fn print_all_task_pm_usage() {
    for i in 1..TASK_NUM_LIMIT {
        let task_ptr = unsafe { TASK_ARRAY[i] };
        if !task_ptr.is_none() {
            let task_ptr = task_ptr.unwrap();
            let t = task_ptr.as_ref();
            let name = t.get_name();
            let pm_used = t.get_pm_heap_stat().mem_used;
            os_print!("[PM Usage] task: {}, PM used: {}", t.get_name(), pm_used);
        }
    }
}

/* Interfaces for testing */
#[cfg(test)]
pub fn reset_static_vars() {
    unsafe {
        *TASK_CNT.borrow_mut_no_logging() = 0;
        SCHEDULER_STARTED = false;
        *CUR_MAX_PRIORITY.borrow_mut_no_logging() = Priority::min_priority();
        TASK_LISTS = [PMVar::new(CircularPList::new()); NUM_PRIORITY_LEVELS];
        DELAYED_TASK_LIST = PMVar::new(SortedPList::new());
    }
}

pub fn task_id_off() -> usize {
    unsafe {
        &current().task_id as *const usize as usize - &current().stack_top as *const usize as usize
    }
}

// #[export_name = "task_switch"]
// pub unsafe extern "C" fn task_switch_with_table() {
//   let prev_task = CURRENT_TASK_PTR.load(Ordering::Relaxed).as_mut().unwrap();
//   let prev = prev_task.task_id;
//   let mut selected = prev;
//   let mut max_prio = Priority::min_priority();
//   // os_print!("Performing Task Switch..").unwrap();
//   for i in (prev+1..TASK_NUM_LIMIT).chain(0..prev+1) {
//     match TASK_ARRAY[i] {
//       None => {
//         continue;
//       }
//       Some(ptr) => {
//         unsafe {
//           let task = ptr.as_ref();

//           if !task.is_schedulable() {
//             continue;
//           }
//           if task.priority.is_higher_than(&max_prio) {
//             selected = task.task_id;
//             max_prio = task.priority;
//           }
//         }
//       }
//     }

//   }
//   // change the state of tasks
//   if selected != prev {
//     unsafe {
//       let task = TASK_ARRAY[selected].unwrap().as_mut();
//       task.status = TaskState::Running;
//       prev_task.status = TaskState::Ready;
//       // os_print!("Selected task : {}", task.name).unwrap();
//       CURRENT_TASK_PTR.store(task as * mut Task, Ordering::Relaxed);
//     }
//   }
// }
