use crate::critical::{self, CriticalSection};
use crate::debug_print;
use crate::list::{self, InsertSortedPList};
use crate::list::{Node, SortedPList};
use crate::pmem::{JournalHandle, PMPtr, PMVar};
use crate::task::{BlockedListItem, ErrorCode, SchedListItem};
use crate::vec::PArray;
use crate::{
    arch, heap, task,
    time::{self, Time},
    transaction,
};
use core::ptr::{self, NonNull};

pub struct Queue {
    length: usize,
    item_size: usize,
    buffer: PArray<u8>,
    read_pos: PMVar<usize>,
    write_pos: PMVar<usize>,
    tail: usize,
    items_enqueued: PMVar<usize>,
    blocked_readers: PMVar<SortedPList<BlockedListItem>>,
    blocked_writers: PMVar<SortedPList<BlockedListItem>>,
}

#[derive(Debug, Copy, Clone)]
pub enum QueueErr {
    NoMemory,
    InvalidParam,
    QueueDeleted,
    QueueFull,
    QueueEmpty,
}

unsafe impl Sync for Queue {}

impl Queue {
    pub fn new(length: usize, item_size: usize) -> Option<PMPtr<Queue>> {
        if length == 0 {
            return None;
        }

        transaction::run(|j| {
            let r = unsafe { heap::palloc::<Queue>(j) };
            let mut ptr = match r {
                None => {
                    return Err(ErrorCode::NoSpace);
                }
                Some(p) => p,
            };

            // allocate buffer

            let buffer = PArray::<u8>::new(length * item_size, j);
            let buffer = match buffer {
                None => {
                    return Err(ErrorCode::NoSpace);
                }
                Some(b) => b,
            };
            let q = unsafe { ptr.as_mut_no_logging() };
            unsafe {
                q.length = length;
                q.item_size = item_size;
                q.read_pos = PMVar::new(0);
                q.write_pos = PMVar::new(0);
                q.items_enqueued = PMVar::new(0);
                q.buffer = buffer;
                q.tail = item_size * length;
                q.blocked_readers = PMVar::new(SortedPList::new());
                q.blocked_writers = PMVar::new(SortedPList::new());
            }
            Ok(r)
        })
        .map_or_else(|_e| None, |v| v)
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn block_reader(
        &mut self,
        j: JournalHandle,
        cs: &CriticalSection,
    ) -> &mut Node<SchedListItem> {
        let task = task::current();
        let link = task.get_event_node_ptr();
        let node = task.remove_from_ready_list(j, cs);
        self.blocked_readers.borrow_mut(j).insert(cs, j, link);
        node
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn block_writer(
        &mut self,
        j: JournalHandle,
        cs: &CriticalSection,
    ) -> &mut Node<SchedListItem> {
        let task = task::current();
        let link = task.get_event_node_ptr();
        let node = task.remove_from_ready_list(j, cs);
        self.blocked_writers.borrow_mut(j).insert(cs, j, link);
        node
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn wakeup_blocked_task(&mut self, is_reader: bool, j: JournalHandle, cs: &CriticalSection) {
        let wait_list = if is_reader {
            &mut self.blocked_readers
        } else {
            &mut self.blocked_writers
        };
        let wait_list = wait_list.borrow_mut(j);
        wait_list.pop_front(cs, j).map(|node| {
            let task = node.value.get_task();

            // remove from delay list if present
            let n = task.remove_from_delayed_list(j, cs);
            // move to the ready list
            task.add_node_to_ready_list(n, j, cs);
        });
    }

    pub fn push_back(
        &mut self,
        item_ptr: NonNull<u8>,
        j: JournalHandle,
        _: &CriticalSection,
    ) -> Result<(), QueueErr> {
        if self.is_full() {
            return Err(QueueErr::QueueFull);
        }
        unsafe {
            let dst = self.buffer.as_ptr().add(*self.write_pos);
            ptr::copy_nonoverlapping(item_ptr.as_ptr(), dst, self.item_size);
        }
        let write_pos = self.write_pos.borrow_mut(j);
        *write_pos += self.item_size;
        if *write_pos == self.tail {
            *write_pos = 0;
        }
        *self.items_enqueued.borrow_mut(j) += 1;
        Ok(())
    }

    pub fn push_front(
        &mut self,
        item_ptr: NonNull<u8>,
        j: JournalHandle,
        _: &CriticalSection,
    ) -> Result<(), QueueErr> {
        if self.is_full() {
            return Err(QueueErr::QueueFull);
        }
        let read_pos = self.read_pos.borrow_mut(j);
        *read_pos = if *read_pos == 0 {
            self.tail - self.item_size
        } else {
            *read_pos - self.item_size
        };
        unsafe {
            let dst = self.buffer.as_ptr().add(*self.read_pos);
            ptr::copy_nonoverlapping(item_ptr.as_ptr(), dst, self.item_size);
        }

        *self.items_enqueued.borrow_mut(j) += 1;

        Ok(())
    }

    pub fn dequeue<T>(&mut self, j: JournalHandle, _: &CriticalSection) -> Result<T, QueueErr> {
        if self.is_empty() {
            // debug_print!("Q empty");
            return Err(QueueErr::QueueEmpty);
        }
        debug_print!("Q not empty: {} items ", *self.items_enqueued);
        let item = unsafe {
            let src = self.buffer.as_ptr().add(*self.read_pos);
            ptr::read::<T>(src as *const T)
        };
        let read_pos = self.read_pos.borrow_mut(j);
        *read_pos += core::mem::size_of::<T>();
        if *read_pos == self.tail {
            *read_pos = 0;
        }
        *self.items_enqueued.borrow_mut(j) -= 1;
        Ok(item)
    }

    #[inline(always)]
    fn is_full(&self) -> bool {
        *self.items_enqueued == self.length
    }
    #[inline(always)]
    fn is_empty(&self) -> bool {
        *self.items_enqueued == 0
    }
}

pub fn queue_create(length: usize, item_size: usize) -> Option<PMPtr<Queue>> {
    Queue::new(length, item_size)
}

#[cfg(not(feature = "opt_list"))]
fn queue_send_impl(
    q: &mut Queue,
    item_ptr: NonNull<u8>,
    mut wait_ticks: Time,
    is_back: bool,
) -> Result<(), QueueErr> {
    let mut res;
    let mut wakeup_time_set = false;
    let mut wakeup_time = 0;
    let task = task::current();

    loop {
        let r = critical::with_no_interrupt(|cs| unsafe {
            transaction::run_relaxed(|j| {
                let r = if is_back {
                    q.push_back(item_ptr, j, cs)
                } else {
                    q.push_front(item_ptr, j, cs)
                };
                match r {
                    Err(_) => {
                        if wait_ticks != 0 {
                            let sched_node = q.block_writer(j, cs);
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
                    Ok(_) => {
                        q.wakeup_blocked_task(true, j, cs);
                        Ok(r)
                    }
                }
            })
        });
        match r {
            Ok(value) => {
                res = value;
                break;
            }
            Err(_) => {
                res = Err(QueueErr::QueueFull);
                if wait_ticks == 0 {
                    break;
                }
                arch::arch_yield();
                // update wait_ticks
                time::update_countdown(&mut wait_ticks, wakeup_time);
            }
        }
    }

    res
}

#[cfg(feature = "opt_list")]
fn queue_send_impl(
    q: &mut Queue,
    item_ptr: NonNull<u8>,
    mut wait_ticks: Time,
    is_back: bool,
) -> Result<(), QueueErr> {
    let mut res = Ok(());
    let mut wakeup_time_set = false;
    let mut wakeup_time = 0;
    let task = task::current();
    let mut exit_loop = false;
    loop {
        let yld = critical::with_no_interrupt(|cs| {
            let r = unsafe {
                transaction::try_run_relaxed(|j| {
                    let r = if is_back {
                        q.push_back(item_ptr, j, cs)
                    } else {
                        q.push_front(item_ptr, j, cs)
                    };
                    match r {
                        Err(_) => Err(ErrorCode::TxRetry),
                        Ok(_) => Ok(()),
                    }
                })
            };
            let mut yld = false;
            match r {
                Err(_) => {
                    res = Err(QueueErr::QueueFull);
                    if wait_ticks != 0 {
                        if !wakeup_time_set {
                            wakeup_time = time::TIME_MANAGER
                                .get_ticks()
                                .checked_add(wait_ticks)
                                .unwrap();
                            wakeup_time_set = true;
                        }
                        unsafe {
                            list::atomic_roll_forward_insert_into_waitlist(
                                q.blocked_writers.borrow_mut_no_logging(),
                                task,
                                wakeup_time,
                                cs,
                            );
                        }
                        yld = true;
                    } else {
                        exit_loop = true;
                    }
                }
                Ok(value) => {
                    let wl = unsafe { q.blocked_readers.borrow_mut_no_logging() };
                    list::atomic_roll_forward_pop_remove_from_waitlist(wl, cs);
                    res = Ok(());
                    exit_loop = true;
                }
            }
            yld
        });
        if exit_loop {
            break;
        }
        if yld {
            ///////////////////
            arch::arch_yield();
            // update wait_ticks
            time::update_countdown(&mut wait_ticks, wakeup_time);
        }
    }
    res
}

pub fn queue_send_back(
    mut q: PMPtr<Queue>,
    item_ptr: NonNull<u8>,
    wait_ticks: Time,
) -> Result<(), QueueErr> {
    let q = unsafe { q.as_mut_no_logging() };
    queue_send_impl(q, item_ptr, wait_ticks, true)
}

pub fn queue_send_front(
    mut q: PMPtr<Queue>,
    item_ptr: NonNull<u8>,
    wait_ticks: Time,
) -> Result<(), QueueErr> {
    let q = unsafe { q.as_mut_no_logging() };
    queue_send_impl(q, item_ptr, wait_ticks, false)
}

#[cfg(not(feature = "opt_list"))]
pub fn queue_receive<T>(mut q: PMPtr<Queue>, mut wait_ticks: Time) -> Result<T, QueueErr> {
    let mut res;
    let mut wakeup_time_set = false;
    let mut wakeup_time = 0;
    let task = task::current();

    loop {
        res = critical::with_no_interrupt(|cs| unsafe {
            transaction::run_relaxed(|j| {
                let q = unsafe { q.as_mut_no_logging() };
                let r = q.dequeue(j, cs);
                match r {
                    Err(_) => {
                        if wait_ticks != 0 {
                            let node = q.block_reader(j, cs);
                            if !wakeup_time_set {
                                wakeup_time = time::TIME_MANAGER
                                    .get_ticks()
                                    .checked_add(wait_ticks)
                                    .unwrap();
                                wakeup_time_set = true;
                            }
                            task.set_wakeup_time(wakeup_time);
                            task.add_node_to_delayed_list(node, j, cs);
                        }
                        Err(ErrorCode::TxRetry)
                    }
                    Ok(v) => {
                        q.wakeup_blocked_task(false, j, cs);
                        Ok(v)
                    }
                }
            })
        });
        match res {
            Ok(_) => {
                break;
            }
            Err(_) => {
                if wait_ticks == 0 {
                    break;
                }
                arch::arch_yield();
                // update wait_ticks
                time::update_countdown(&mut wait_ticks, wakeup_time);
            }
        }
    }

    match res {
        Ok(v) => Ok(v),
        Err(_) => Err(QueueErr::QueueEmpty),
    }
}

#[cfg(feature = "opt_list")]
pub fn queue_receive<T>(mut q: PMPtr<Queue>, mut wait_ticks: Time) -> Result<T, QueueErr> {
    let mut res;
    let mut wakeup_time_set = false;
    let mut wakeup_time = 0;
    let task = task::current();
    let mut exit_loop = false;
    let mut yld = false;
    loop {
        res = critical::with_no_interrupt(|cs| {
            let r = unsafe {
                transaction::try_run_relaxed(move |j| {
                    let q = unsafe { q.as_mut_no_logging() };
                    let r = q.dequeue(j, cs);
                    match r {
                        Err(_) => Err(ErrorCode::TxRetry),
                        Ok(v) => Ok(v),
                    }
                })
            };

            yld = false;
            match r {
                Err(_) => {
                    if wait_ticks != 0 {
                        if !wakeup_time_set {
                            wakeup_time = time::TIME_MANAGER
                                .get_ticks()
                                .checked_add(wait_ticks)
                                .unwrap();
                            wakeup_time_set = true;
                        }
                        let wait_list = unsafe {
                            q.as_mut_no_logging()
                                .blocked_readers
                                .borrow_mut_no_logging()
                        };
                        list::atomic_roll_forward_insert_into_waitlist(
                            wait_list,
                            task,
                            wakeup_time,
                            cs,
                        );
                        yld = true;
                    } else {
                        exit_loop = true;
                    }
                }
                Ok(_) => {
                    let wait_list = unsafe {
                        q.as_mut_no_logging()
                            .blocked_writers
                            .borrow_mut_no_logging()
                    };
                    list::atomic_roll_forward_pop_remove_from_waitlist(wait_list, cs);
                    exit_loop = true;
                }
            }
            r
        });
        if exit_loop {
            break;
        }
        if yld {
            ///////////////////
            arch::arch_yield();
            // update wait_ticks
            time::update_countdown(&mut wait_ticks, wakeup_time);
        }
    }

    match res {
        Ok(v) => Ok(v),
        Err(_) => Err(QueueErr::QueueEmpty),
    }
}

#[cfg(not(feature = "opt_list"))]
pub fn queue_block_until_not_empty(mut q: PMPtr<Queue>, wait_ticks: Time, forever: bool) {
    debug_print!("Running block until");
    let res = critical::with_no_interrupt(move |cs| {
        transaction::run(move |j| {
            let task = task::current();
            let q = unsafe { q.as_mut_no_logging() };
            if q.is_empty() {
                let wakeup_time;
                if forever {
                    wakeup_time = Time::MAX;
                } else {
                    wakeup_time = time::TIME_MANAGER
                        .get_ticks()
                        .checked_add(wait_ticks)
                        .unwrap();
                }
                debug_print!("Blocking ..., wakeup_time = {}", wakeup_time);

                q.block_reader(j, cs);
                task.set_wakeup_time(wakeup_time);
                task.add_to_delayed_list(j, cs);
                Err(ErrorCode::TxRetry)
            } else {
                Ok(())
            }
        })
    });

    if let Err(_) = res {
        debug_print!("Yielding");
        arch::arch_yield();
    }
}

#[cfg(feature = "opt_list")]
pub fn queue_block_until_not_empty(mut q: PMPtr<Queue>, wait_ticks: Time, forever: bool) {
    debug_print!("Running block until");
    let task = task::current();
    let q = unsafe { q.as_mut_no_logging() };
    let yld = critical::with_no_interrupt(|cs| {
        if q.is_empty() {
            let wakeup_time;
            if forever {
                wakeup_time = Time::MAX;
            } else {
                wakeup_time = time::TIME_MANAGER
                    .get_ticks()
                    .checked_add(wait_ticks)
                    .unwrap();
            }
            debug_print!("Blocking ..., wakeup_time = {}", wakeup_time);
            let wait_list = unsafe { q.blocked_readers.borrow_mut_no_logging() };
            list::atomic_roll_forward_insert_into_waitlist(wait_list, task, wakeup_time, cs);
            true
        } else {
            false
        }
    });

    if yld {
        debug_print!("Yielding");
        arch::arch_yield();
    }
}
