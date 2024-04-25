use crate::critical::CriticalSection;
use crate::list::InsertSortedPList;
use crate::list::{self, Node, SortedPList};
use crate::pmem::{JournalHandle, PMPtr, PMVar};
use crate::task::{self, ErrorCode};
use crate::task::{current, BlockedListItem, SchedListItem, Task};
use crate::{
    arch::arch_yield,
    critical, debug_print, heap,
    time::{self, Time},
    transaction,
};

#[cfg(not(feature = "opt_list"))]
pub struct Semaphore {
    count: usize,
    mutex_holder: Option<PMPtr<Task>>, // used only if this is a mutex
    is_mutex: bool,
    size: usize,
    wait_list: SortedPList<BlockedListItem>,
}

#[cfg(feature = "opt_list")]
pub struct Semaphore {
    count: PMVar<usize>,
    mutex_holder: PMVar<Option<PMPtr<Task>>>, // used only if this is a mutex
    is_mutex: bool,
    size: usize,
    wait_list: SortedPList<BlockedListItem>,
}

impl Semaphore {
    pub fn new(count: usize, j: JournalHandle) -> Option<PMPtr<Semaphore>> {
        let ptr = heap::pm_new(
            Semaphore {
                #[cfg(not(feature = "opt_list"))]
                count,
                #[cfg(feature = "opt_list")]
                count: unsafe { PMVar::new(count) },
                size: count,
                #[cfg(not(feature = "opt_list"))]
                mutex_holder: None,
                #[cfg(feature = "opt_list")]
                mutex_holder: unsafe { PMVar::new(None) },
                wait_list: SortedPList::new(),
                is_mutex: count == 1,
            },
            j,
        );
        let sm = unsafe { ptr.unwrap().as_mut_no_logging() };
        ptr
    }

    pub fn mutex_holder(&self) -> Option<&Task> {
        self.mutex_holder.map(|p| p.as_ref())
    }

    // debug function
    #[cfg(feature = "opt_list")]
    pub fn print(&self) {
        debug_print!("Semaphore: count: {} size: {}", *self.count, self.size);
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn print(&self) {
        debug_print!("Semaphore: count: {} size: {}", self.count, self.size);
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn block_taker(
        &mut self,
        j: JournalHandle,
        cs: &CriticalSection,
    ) -> &mut Node<SchedListItem> {
        let task = task::current();
        let link = task.get_event_node_ptr();
        let n = task.remove_from_ready_list(j, cs);
        self.wait_list.insert(cs, j, link);
        n
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn wakeup_blocked_task(
        &mut self,
        j: JournalHandle,
        cs: &CriticalSection,
    ) -> Option<&mut Task> {
        self.wait_list.pop_front(cs, j).map(|event_node| {
            let task = event_node.value.get_mut_task();
            // remove from delay list if present
            let sched_node = task.remove_from_delayed_list(j, cs);
            // add to ready list
            task.add_node_to_ready_list(sched_node, j, cs);
            task
        })
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn try_take(&mut self, _: &CriticalSection) -> Result<(), ()> {
        if self.count > 0 {
            self.count -= 1;
            self.mutex_holder = Some(current().as_pm_ptr());
            Ok(())
        } else {
            Err(())
        }
    }

    #[cfg(feature = "opt_list")]
    pub fn try_take(&mut self, j: JournalHandle, _: &CriticalSection) -> Result<(), ()> {
        if *self.count > 0 {
            let cnt = self.count.borrow_mut(j);
            *cnt -= 1;
            let mutex_holder = self.mutex_holder.borrow_mut(j);
            *mutex_holder = Some(current().as_pm_ptr());
            Ok(())
        } else {
            // let holder_name = if self.mutex_holder.is_none() {
            //     "None"
            // } else {
            //     self.mutex_holder.unwrap().as_ref().get_name()
            // };
            // crate::os_print!("count = {}, holder name: {}", *self.count, holder_name);
            Err(())
        }
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn give(&mut self, j: JournalHandle, cs: &CriticalSection) -> Option<&mut Task> {
        if self.count == self.size {
            return None;
        }
        assert!(self.count < self.size);
        self.count += 1;
        self.wakeup_blocked_task(j, cs)
    }

    #[cfg(feature = "opt_list")]
    // return true if wake up some other task
    pub fn give(&mut self, j: JournalHandle, cs: &CriticalSection) -> bool {
        if *self.count == self.size {
            return false;
        }
        assert!(*self.count < self.size);
        *self.count.borrow_mut(j) += 1;
        return true;
    }
}

pub fn create_semaphore(n: usize) -> Option<PMPtr<Semaphore>> {
    transaction::run(|j| Semaphore::new(n, j))
}

#[cfg(feature = "opt_list")]
pub fn semaphore_take(mut sem: PMPtr<Semaphore>, mut wait_ticks: Time) -> Result<(), ()> {
    let mut res;
    let mut wakeup_time_set = false;
    let mut wakeup_time = 0;
    let task = task::current();
    let task_ptr = task.as_pm_ptr();
    let sem = unsafe { sem.as_mut_no_logging() };
    let mut exit_loop = false;
    // if already the holder, return
    if *sem.mutex_holder == Some(task_ptr) {
        return Ok(());
    }
    loop {
        res = critical::with_no_interrupt(|cs| {
            let r = current().get_mut_tx().run_no_replay(|j| {
                let r = sem.try_take(j, cs);
                r
            });

            match r {
                Err(_) => {
                    crate::os_dbg_print!("Can't lock target lock...");
                    if wait_ticks != 0 {
                        if !wakeup_time_set {
                            wakeup_time = time::TIME_MANAGER
                                .get_ticks()
                                .checked_add(wait_ticks)
                                .unwrap();
                            wakeup_time_set = true;
                        }
                        // go to sleep
                        list::atomic_roll_forward_insert_into_waitlist(
                            &mut sem.wait_list,
                            task,
                            wakeup_time,
                            cs,
                        );
                        //////////////////////////
                        arch_yield();
                        // update wait_ticks
                        time::update_countdown(&mut wait_ticks, wakeup_time);
                    } else {
                        exit_loop = true;
                    }
                }
                Ok(_) => {
                    exit_loop = true;
                }
            }
            r
        });
        if exit_loop {
            break;
        }
    }
    match res {
        Err(_) => Err(()),
        _ => Ok(()),
    }
}

#[cfg(feature = "opt_list")]
pub fn semaphore_give(mut sem: PMPtr<Semaphore>) {
    let sem = unsafe { sem.as_mut_no_logging() };
    let mut yield_now = false;
    critical::with_no_interrupt(|cs| {
        let unblock_task = current().get_mut_tx().run_no_replay(|j| {
            let t = sem.give(j, cs);
            t
        });

        if unblock_task {
            let item = sem.wait_list.peek_front(cs);

            let current = task::current();
            match item {
                Some(n) => {
                    let t = n.get_task();
                    if current.less_important_than(t) && t.is_ready() {
                        yield_now = true;
                    }
                }
                None => {}
            }
            list::atomic_roll_forward_pop_remove_from_waitlist(&mut sem.wait_list, cs);
        }
    });

    if yield_now {
        arch_yield();
    }
}

#[cfg(not(feature = "opt_list"))]
pub fn semaphore_take(mut sem: PMPtr<Semaphore>, mut wait_ticks: Time) -> Result<(), ()> {
    let mut res;
    let mut wakeup_time_set = false;
    let mut wakeup_time = 0;
    let task = task::current();
    let task_ptr = task.as_pm_ptr();
    // if already the holder, return
    if sem.as_ref().mutex_holder == Some(task_ptr) {
        return Ok(());
    }
    loop {
        res = critical::with_no_interrupt(|cs| {
            current().get_mut_tx().run_no_replay(|j| {
                debug_print!("derefercing semaphore");
                let sem = sem.as_mut(j);
                let r = sem.try_take(cs);
                match r {
                    Err(_) => {
                        debug_print!("Can't lock target lock...");
                        if wait_ticks != 0 {
                            let sched_node = sem.block_taker(j, cs);
                            if !wakeup_time_set {
                                wakeup_time = time::TIME_MANAGER
                                    .get_ticks()
                                    .checked_add(wait_ticks)
                                    .unwrap();
                                wakeup_time_set = true;
                            }
                            task.set_wakeup_time(wakeup_time);
                            task.add_node_to_delayed_list(sched_node, j, cs);
                        }
                        Err(ErrorCode::TxRetry)
                    }
                    Ok(_) => Ok(()),
                }
            })
        });
        match res {
            Ok(_) => {
                // semaphore is taken, just exit
                break;
            }
            Err(_) => {
                if wait_ticks == 0 {
                    break;
                }
                arch_yield();
                // update wait_ticks
                time::update_countdown(&mut wait_ticks, wakeup_time);
            }
        }
    }

    match res {
        Err(_) => Err(()),
        _ => Ok(()),
    }
}

#[cfg(not(feature = "opt_list"))]
pub fn semaphore_give(mut sem: PMPtr<Semaphore>) {
    let mut yield_now = false;
    critical::with_no_interrupt(|cs| {
        current().get_mut_tx().run_no_replay(|j| {
            let sem = sem.as_mut(j);
            let t = sem.give(j, cs);
            match t {
                None => {}
                Some(task) => {
                    let current = task::current();
                    if current.less_important_than(task) {
                        yield_now = true;
                    }
                }
            };
        });
    });
    if yield_now {
        arch_yield();
    }
}
