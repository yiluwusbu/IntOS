use crate::arch::arch_yield;
use crate::list;
use crate::{
    arch,
    critical::{self, CriticalSection},
    heap,
    list::UnsortedPList,
    task::{current, BlockedListItem},
    time::TIME_MANAGER,
    transaction,
};
use crate::{
    debug_print,
    marker::TxOutSafe,
    pmem::{JournalHandle, PMPtr, PMVar},
    time::Time,
    util::debug_syscall_tx_cache,
};

pub type EventBits = usize;

#[cfg(target_pointer_width = "32")]
const CLEAR_EVENTS_ON_EXIT_BIT: usize = 0x01000000;
#[cfg(target_pointer_width = "32")]
const UNBLOCKED_DUE_TO_BIT_SET: usize = 0x02000000;
#[cfg(target_pointer_width = "32")]
const WAIT_FOR_ALL_BITS: usize = 0x04000000;
#[cfg(target_pointer_width = "32")]
const EVENT_BITS_CONTROL_BYTES: usize = 0xff000000;

#[cfg(any(target_pointer_width = "16", target_pointer_width = "64"))]
const CLEAR_EVENTS_ON_EXIT_BIT: usize = 0x0100;
#[cfg(any(target_pointer_width = "16", target_pointer_width = "64"))]
const UNBLOCKED_DUE_TO_BIT_SET: usize = 0x0200;
#[cfg(any(target_pointer_width = "16", target_pointer_width = "64"))]
const WAIT_FOR_ALL_BITS: usize = 0x0400;
#[cfg(any(target_pointer_width = "16", target_pointer_width = "64"))]
const EVENT_BITS_CONTROL_BYTES: usize = 0xff00;

#[cfg(not(feature = "opt_list"))]
pub struct EventGroup {
    event_bits: EventBits, // 0xAAXXXXXX: AA is the control byte
    waiting_tasks: UnsortedPList<BlockedListItem>,
}

#[cfg(feature = "opt_list")]
pub struct EventGroup {
    event_bits: PMVar<EventBits>, // 0xAAXXXXXX: AA is the control byte
    waiting_tasks: UnsortedPList<BlockedListItem>,
}

impl EventGroup {
    fn new_from_heap(j: JournalHandle) -> Option<PMPtr<EventGroup>> {
        heap::pm_new(
            EventGroup {
                #[cfg(not(feature = "opt_list"))]
                event_bits: 0,
                #[cfg(feature = "opt_list")]
                event_bits: unsafe { PMVar::new(0) },
                waiting_tasks: UnsortedPList::new(),
            },
            j,
        )
    }

    #[cfg(not(feature = "opt_list"))]
    #[inline(always)]
    fn get_event_bits(&self, _: &CriticalSection) -> EventBits {
        self.event_bits & (!EVENT_BITS_CONTROL_BYTES)
    }

    #[cfg(not(feature = "opt_list"))]
    #[inline(always)]
    fn set_event_bits(&mut self, bits: EventBits, _: &CriticalSection) {
        self.event_bits |= bits;
    }

    #[cfg(not(feature = "opt_list"))]
    #[inline(always)]
    fn clear_event_bits(&mut self, bits: EventBits, _: &CriticalSection) {
        self.event_bits &= !bits;
    }

    #[cfg(feature = "opt_list")]
    #[inline(always)]
    fn get_event_bits(&self, _: &CriticalSection) -> EventBits {
        *self.event_bits & (!EVENT_BITS_CONTROL_BYTES)
    }

    #[cfg(feature = "opt_list")]
    #[inline(always)]
    fn set_event_bits(&mut self, bits: EventBits, j: JournalHandle, _: &CriticalSection) {
        *self.event_bits.borrow_mut(j) |= bits;
    }

    #[cfg(feature = "opt_list")]
    #[inline(always)]
    fn clear_event_bits(&mut self, bits: EventBits, j: JournalHandle, _: &CriticalSection) {
        *self.event_bits.borrow_mut(j) &= !bits;
    }

    fn wait_condition_ok(
        &self,
        bits_to_wait_for: EventBits,
        wait_for_all_bits: bool,
        _: &CriticalSection,
    ) -> bool {
        let mut res = false;
        #[cfg(not(feature = "opt_list"))]
        let event_bits = self.event_bits;
        #[cfg(feature = "opt_list")]
        let event_bits = *self.event_bits;
        if !wait_for_all_bits {
            if (event_bits & bits_to_wait_for) != 0 {
                res = true;
            }
        } else {
            if (event_bits & bits_to_wait_for) == bits_to_wait_for {
                res = true;
            }
        }
        res
    }

    #[cfg(feature = "opt_list")]
    fn wait_for_event_group_bits(
        mut eg_ptr: PMPtr<EventGroup>,
        bits_to_wait_for: EventBits,
        clear_on_exit: bool,
        wait_for_all_bits: bool,
        wait_ticks: Time,
    ) -> Result<EventBits, EventBits> {
        let mut res = 0;
        let mut control_bits = 0;
        let mut wait_ticks = wait_ticks;

        // If there is a power-failure just after we are awaken
        // we need to avoid unnesssary waiting as the event flags
        // may have been cleared by the event sender
        let bits = current().get_block_item_value();
        if bits & UNBLOCKED_DUE_TO_BIT_SET != 0 {
            crate::os_dbg_print!("Failed just before awaken...");
            return Ok(bits & !EVENT_BITS_CONTROL_BYTES);
        }

        let eg = eg_ptr.as_ref();
        if clear_on_exit {
            control_bits |= CLEAR_EVENTS_ON_EXIT_BIT;
        }
        if wait_for_all_bits {
            control_bits |= WAIT_FOR_ALL_BITS;
        }
        critical::with_no_interrupt(|cs| {
            if eg.wait_condition_ok(bits_to_wait_for, wait_for_all_bits, cs) {
                res = eg.get_event_bits(cs);
                if clear_on_exit {
                    transaction::run(move |j| {
                        let eg_mut = unsafe { eg_ptr.as_mut_no_logging() };
                        eg_mut.clear_event_bits(bits_to_wait_for, j, cs);
                    });
                }
                wait_ticks = 0;
            } else if wait_ticks > 0 {
                let task = current();
                let block_item_val = bits_to_wait_for | control_bits;
                task.set_block_item_value(block_item_val);
                let wakeup_time = TIME_MANAGER.get_ticks().checked_add(wait_ticks).unwrap();
                task.set_wakeup_time(wakeup_time);
                ///////////////////////////////////////////////
                let wl = unsafe { &mut eg_ptr.as_mut_no_logging().waiting_tasks };
                list::atomic_roll_forward_insert_into_unsorted_waitlist(wl, task, cs);
            } else {
                res = eg.get_event_bits(cs);
            }
        });

        if wait_ticks > 0 {
            arch::arch_yield();
            let task = current();
            let event_bits = task.get_block_item_value();
            // debug_print!("Event bits: {:#X}", event_bits);
            res = event_bits & !EVENT_BITS_CONTROL_BYTES;
            if event_bits & UNBLOCKED_DUE_TO_BIT_SET == 0 {
                let ok = critical::with_no_interrupt(|cs| {
                    // debug_print!("Bits: {}, bits_to_wait_for: {}", eg.get_event_bits(cs) & !EVENT_BITS_CONTROL_BYTES, bits_to_wait_for);
                    let ok = eg.wait_condition_ok(bits_to_wait_for, wait_for_all_bits, cs);
                    if ok && (event_bits & CLEAR_EVENTS_ON_EXIT_BIT) != 0 {
                        // clear event bits
                        transaction::run(move |j| {
                            let eg_mut = unsafe { eg_ptr.as_mut_no_logging() };
                            eg_mut.clear_event_bits(bits_to_wait_for, j, cs);
                        });
                    }
                    ok
                });
                // Time out !
                if !ok {
                    return Err(res);
                }
            }
        }

        Ok(res)
    }

    #[cfg(not(feature = "opt_list"))]
    fn wait_for_event_group_bits(
        &mut self,
        bits_to_wait_for: EventBits,
        clear_on_exit: bool,
        wait_for_all_bits: bool,
        wait_ticks: Time,
        j: JournalHandle,
    ) -> Result<EventBits, EventBits> {
        let mut res = 0;
        let mut control_bits = 0;
        let mut wait_ticks = wait_ticks;
        if clear_on_exit {
            control_bits |= CLEAR_EVENTS_ON_EXIT_BIT;
        }
        if wait_for_all_bits {
            control_bits |= WAIT_FOR_ALL_BITS;
        }
        critical::with_no_interrupt(|cs| {
            if self.wait_condition_ok(bits_to_wait_for, wait_for_all_bits, cs) {
                res = self.get_event_bits(cs);
                if clear_on_exit {
                    self.clear_event_bits(bits_to_wait_for, cs);
                }
                wait_ticks = 0;
            } else if wait_ticks > 0 {
                let task = current();
                let block_item_val = bits_to_wait_for | control_bits;
                task.set_block_item_value(block_item_val);
                let wakeup_time = TIME_MANAGER.get_ticks().checked_add(wait_ticks).unwrap();
                task.set_wakeup_time(wakeup_time);
                let sched_node = task.remove_from_ready_list(j, cs);
                self.waiting_tasks.insert(cs, j, task.get_event_node_ptr());
                task.add_node_to_delayed_list(sched_node, j, cs);
            } else {
                res = self.get_event_bits(cs);
            }
        });

        if wait_ticks > 0 {
            arch::arch_yield();
            let task = current();
            let event_bits = task.reset_block_item_value();
            debug_print!("Event bits: {:#X}", event_bits);
            res = event_bits & !EVENT_BITS_CONTROL_BYTES;
            if event_bits & UNBLOCKED_DUE_TO_BIT_SET == 0 {
                let ok = critical::with_no_interrupt(|cs| {
                    debug_print!("Checking...");
                    debug_print!(
                        "Bits: {}, bits_to_wait_for: {}",
                        self.event_bits & !EVENT_BITS_CONTROL_BYTES,
                        bits_to_wait_for
                    );
                    let ok = self.wait_condition_ok(bits_to_wait_for, wait_for_all_bits, cs);
                    if ok && (event_bits & CLEAR_EVENTS_ON_EXIT_BIT) != 0 {
                        self.clear_event_bits(bits_to_wait_for, cs);
                    }
                    ok
                });
                // Time out !
                if !ok {
                    return Err(res);
                }
            }
        }

        Ok(res)
    }

    #[cfg(not(feature = "opt_list"))]
    fn set_event_group_bits(
        mut eg_ptr: PMPtr<EventGroup>,
        bits_to_set: EventBits,
        j: JournalHandle,
    ) {
        let mut yld_to_higher_prio = false;
        critical::with_no_interrupt(|cs| {
            // set event bits
            let eg = eg_ptr.as_mut(j);
            eg.set_event_bits(bits_to_set, cs);
            let bits_set = eg.get_event_bits(cs);
            let mut bits_to_clear: usize = 0;
            debug_print!("Waiting list: \n {}", eg.waiting_tasks);
            let waiting_list = &mut eg.waiting_tasks;
            for node_ptr in waiting_list.iter_mut() {
                let node = node_ptr.as_ref();
                let block_item = node.value;
                let event_bits = block_item.get_opaque_value();
                let bits_waiting_for = event_bits & !EVENT_BITS_CONTROL_BYTES;
                let task = block_item.get_task();
                let mut unblock = false;
                debug_print!(
                    "Bits set: {}, now bits = {}, bits waiting for = {}",
                    bits_to_set,
                    bits_set,
                    bits_waiting_for
                );
                if event_bits & WAIT_FOR_ALL_BITS != 0 {
                    if bits_waiting_for & bits_set == bits_waiting_for {
                        unblock = true;
                    }
                } else if bits_waiting_for & bits_set != 0 {
                    unblock = true;
                }
                if unblock {
                    debug_print!("Unblocking...");
                    if task.get_priority() > current().get_priority() {
                        yld_to_higher_prio = true;
                    }
                    if event_bits & CLEAR_EVENTS_ON_EXIT_BIT != 0 {
                        bits_to_clear |= bits_waiting_for;
                    }
                    let block_item = &mut waiting_list.remove(cs, j, node_ptr).value;
                    // If the task is in waiting list,
                    // It must be in the delayed list as well
                    let sched_node = task.remove_from_delayed_list(j, cs);
                    // add to ready list
                    task.add_node_to_ready_list(sched_node, j, cs);
                    block_item.set_opaque_value(bits_set | UNBLOCKED_DUE_TO_BIT_SET);
                    debug_print!("Event bits Set: {:#X}", block_item.get_opaque_value());
                }
            }
            eg.clear_event_bits(bits_to_clear, cs);
        });
        if yld_to_higher_prio {
            arch_yield();
        }
    }

    #[cfg(feature = "opt_list")]
    fn set_event_group_bits(mut eg_ptr: PMPtr<EventGroup>, bits_to_set: EventBits) {
        use crate::os_print;

        let mut yld_to_higher_prio = false;

        critical::with_no_interrupt(|cs| {
            // set event bits
            let eg = unsafe { eg_ptr.as_mut_no_logging() };
            let bits_to_set = bits_to_set & (!EVENT_BITS_CONTROL_BYTES);
            let mut bits_to_clear: usize = 0;
            // debug_print!("Waiting list: \n {}", eg.waiting_tasks);
            transaction::run(move |j| {
                let eg = unsafe { eg_ptr.as_mut_no_logging() };
                eg.set_event_bits(bits_to_set, j, cs);
            });
            let bits_set = eg.get_event_bits(cs);
            let waiting_list = &mut eg.waiting_tasks;
            for mut node_ptr in waiting_list.iter_mut() {
                let node = unsafe { node_ptr.as_mut_no_logging() };
                let block_item = &mut node.value;
                let event_bits = block_item.get_opaque_value();
                let bits_waiting_for = event_bits & !EVENT_BITS_CONTROL_BYTES;
                let task = block_item.get_task();
                let mut unblock = false;
                // debug_print!("Bits set: {}, now bits = {}, bits waiting for = {}", bits_to_set, bits_set, bits_waiting_for);
                if event_bits & WAIT_FOR_ALL_BITS != 0 {
                    if bits_waiting_for & bits_set == bits_waiting_for {
                        unblock = true;
                    }
                } else if bits_waiting_for & bits_set != 0 {
                    unblock = true;
                }
                if unblock {
                    if task.get_priority() > current().get_priority() {
                        yld_to_higher_prio = true;
                    }
                    if event_bits & CLEAR_EVENTS_ON_EXIT_BIT != 0 {
                        bits_to_clear |= bits_waiting_for;
                    }
                    block_item.set_opaque_value(bits_set | UNBLOCKED_DUE_TO_BIT_SET);
                    // debug_print!("Event bits Set: {:#X}", block_item.get_opaque_value());
                    list::atomic_roll_forward_remove_from_unsorted_waitlist(
                        waiting_list,
                        node_ptr,
                        cs,
                    );
                }
            }
            // TODO: Should we clear it?
            transaction::run(move |j| {
                let eg = unsafe { eg_ptr.as_mut_no_logging() };
                eg.clear_event_bits(bits_to_clear, j, cs);
            });
        });
        if yld_to_higher_prio {
            arch_yield();
        }
    }

    #[cfg(not(feature = "opt_list"))]
    fn sync_event_group_bits(
        mut eg_ptr: PMPtr<EventGroup>,
        bits_to_set: EventBits,
        bits_to_wait_for: EventBits,
        wait_ticks: Time,
        j: JournalHandle,
    ) -> Result<EventBits, EventBits> {
        let mut ret = 0;
        let mut wait_ticks = wait_ticks;
        let eg = eg_ptr.as_mut(j);
        critical::with_no_interrupt(|cs| {
            let original_bits = eg.get_event_bits(cs);
            EventGroup::set_event_group_bits(eg_ptr, bits_to_set, j);

            if ((original_bits | bits_to_set) & bits_to_wait_for) == bits_to_wait_for {
                ret = original_bits | bits_to_set;
                // clear bits
                eg.clear_event_bits(bits_to_wait_for, cs);
                wait_ticks = 0;
            } else if wait_ticks > 0 {
                let task = current();
                let block_item_val =
                    bits_to_wait_for | CLEAR_EVENTS_ON_EXIT_BIT | WAIT_FOR_ALL_BITS;
                task.set_block_item_value(block_item_val);
                let wakeup_time = TIME_MANAGER.get_ticks().checked_add(wait_ticks).unwrap();

                let sched_node = task.remove_from_ready_list(j, cs);
                eg.waiting_tasks.insert(cs, j, task.get_event_node_ptr());
                task.add_node_to_delayed_list(sched_node, j, cs);

                task.set_wakeup_time(wakeup_time);
            } else {
                ret = eg.get_event_bits(cs);
            }
        });

        if wait_ticks > 0 {
            // Oh no we need to block
            arch::arch_yield();
            let task = current();
            let event_bits = task.reset_block_item_value();
            ret = event_bits & !EVENT_BITS_CONTROL_BYTES;
            if event_bits & UNBLOCKED_DUE_TO_BIT_SET == 0 {
                // check  whether the bits are set anyway
                let ok = critical::with_no_interrupt(|cs| {
                    let ok = eg.wait_condition_ok(bits_to_wait_for, true, cs);
                    if ok {
                        ret = eg.event_bits & !EVENT_BITS_CONTROL_BYTES;
                    }
                    ok
                });
                // Time out !
                if !ok {
                    return Err(ret);
                }
            }
            // else Unblock due to bit set
        }
        Ok(ret)
    }

    #[cfg(feature = "opt_list")]
    fn sync_event_group_bits(
        mut eg_ptr: PMPtr<EventGroup>,
        bits_to_set: EventBits,
        bits_to_wait_for: EventBits,
        wait_ticks: Time,
    ) -> Result<EventBits, EventBits> {
        use crate::time::Time;

        let mut ret = 0;
        let mut wait_ticks = wait_ticks;
        let eg = unsafe { eg_ptr.as_mut_no_logging() };
        critical::with_no_interrupt(|cs| {
            let original_bits = transaction::run(|_| eg.get_event_bits(cs));

            EventGroup::set_event_group_bits(eg_ptr, bits_to_set);

            if ((original_bits | bits_to_set) & bits_to_wait_for) == bits_to_wait_for {
                ret = original_bits | bits_to_set;
                // clear bits
                transaction::run(move |j| {
                    let eg = unsafe { eg_ptr.as_mut_no_logging() };
                    eg.clear_event_bits(bits_to_wait_for, j, cs);
                });
                wait_ticks = 0;
            } else if wait_ticks > 0 {
                let task = current();
                let block_item_val =
                    bits_to_wait_for | CLEAR_EVENTS_ON_EXIT_BIT | WAIT_FOR_ALL_BITS;
                task.set_block_item_value(block_item_val);
                let wakeup_time = TIME_MANAGER.get_ticks().checked_add(wait_ticks).unwrap();
                task.set_wakeup_time(wakeup_time);
                /////////////////////////////////////////
                list::atomic_roll_forward_insert_into_unsorted_waitlist(
                    &mut eg.waiting_tasks,
                    task,
                    cs,
                );
            } else {
                ret = eg.get_event_bits(cs);
            }
        });

        if wait_ticks > 0 {
            // Oh no we need to block
            arch::arch_yield();
            let task = current();
            let event_bits = task.reset_block_item_value();
            ret = event_bits & !EVENT_BITS_CONTROL_BYTES;
            if event_bits & UNBLOCKED_DUE_TO_BIT_SET == 0 {
                // check  whether the bits are set anyway
                let ok = critical::with_no_interrupt(|cs| {
                    let ok = eg.wait_condition_ok(bits_to_wait_for, true, cs);
                    if ok && (event_bits & CLEAR_EVENTS_ON_EXIT_BIT) != 0 {
                        // clear event bits
                        transaction::run(move |j| {
                            let eg_mut = unsafe { eg_ptr.as_mut_no_logging() };
                            eg_mut.clear_event_bits(bits_to_wait_for, j, cs);
                        });
                    }
                    ok
                });
                // Time out !
                if !ok {
                    return Err(ret);
                }
            }
            // else Unblock due to bit set
        }
        Ok(ret)
    }
}

#[derive(Clone, Copy)]
pub struct EventGroupHandle(PMPtr<EventGroup>);

unsafe impl TxOutSafe for EventGroupHandle {}

unsafe impl Sync for EventGroupHandle {}
unsafe impl Send for EventGroupHandle {}

pub fn create_event_group() -> Option<EventGroupHandle> {
    transaction::run(|j| EventGroup::new_from_heap(j).map(|ptr| EventGroupHandle(ptr)))
}

pub fn event_group_wait(
    event_grp: EventGroupHandle,
    bits_to_wait_for: EventBits,
    clear_on_exit: bool,
    wait_for_all_bits: bool,
    wait_ticks: Time,
) -> Result<EventBits, EventBits> {
    #[cfg(not(feature = "opt_list"))]
    {
        let mut eg_ptr = event_grp.0;
        transaction::run(move |j| {
            let event_grp = eg_ptr.as_mut(j);
            let r = event_grp.wait_for_event_group_bits(
                bits_to_wait_for,
                clear_on_exit,
                wait_for_all_bits,
                wait_ticks,
                j,
            );
            r
        })
    }
    #[cfg(feature = "opt_list")]
    {
        EventGroup::wait_for_event_group_bits(
            event_grp.0,
            bits_to_wait_for,
            clear_on_exit,
            wait_for_all_bits,
            wait_ticks,
        )
    }
}

pub fn event_group_set(event_grp: EventGroupHandle, bits_to_set: EventBits) {
    // debug_print!("Ready to run the event_group set");
    // debug_syscall_tx_cache();
    #[cfg(not(feature = "opt_list"))]
    {
        transaction::run(|j| {
            EventGroup::set_event_group_bits(event_grp.0, bits_to_set, j);
        });
    }

    #[cfg(feature = "opt_list")]
    {
        EventGroup::set_event_group_bits(event_grp.0, bits_to_set);
    }
}

pub fn event_group_sync(
    event_grp: EventGroupHandle,
    bits_to_set: EventBits,
    bits_to_wait_for: EventBits,
    wait_ticks: Time,
) -> Result<EventBits, EventBits> {
    #[cfg(not(feature = "opt_list"))]
    {
        transaction::run(|j| {
            let r = EventGroup::sync_event_group_bits(
                event_grp.0,
                bits_to_set,
                bits_to_wait_for,
                wait_ticks,
                j,
            );
            r
        })
    }

    #[cfg(feature = "opt_list")]
    EventGroup::sync_event_group_bits(event_grp.0, bits_to_set, bits_to_wait_for, wait_ticks)
}
