use crate::task::current;
use crate::task::task_switch;
use crate::task::{kernel_recovery_begin_stat, kernel_recovery_end_stat};
use crate::{
    critical::{exit_all_critical, is_in_critical},
    debug_print, declare_pm_var_unsafe,
    list::{self, ListTxOpCode, ListTxOpLog},
    pmem::Journal,
    time::TIME_MANAGER,
    transaction::{Transaction, TxCache},
};
// TODO: make them PMVars
// static mut BOOT_JOURNAL: Journal = Journal::new();
declare_pm_var_unsafe!(BOOT_JOURNAL, Journal, Journal::new());
// static mut BOOT_TX: Transaction = Transaction::new_empty();
declare_pm_var_unsafe!(BOOT_TX, Transaction, Transaction::new_empty());
// static mut BOOT_TX_CACHE: TxCache = TxCache::new();
declare_pm_var_unsafe!(BOOT_TX_CACHE, TxCache, TxCache::new());
// static mut FIRST_BOOT_DONE: bool = false;
declare_pm_var_unsafe!(FIRST_BOOT_DONE, bool, false);
// static mut CURRENT_GENERATION: usize = 0;
declare_pm_var_unsafe!(CURRENT_GENERATION, usize, 0);
// static mut CURRENT_GENERATION: usize = 0;
declare_pm_var_unsafe!(IN_CONTEXT_SWITCH_TX, usize, 0);

pub fn is_first_boot_done() -> bool {
    unsafe { FIRST_BOOT_DONE }
}

pub fn complete_first_boot() {
    unsafe {
        FIRST_BOOT_DONE = true;
    }
}

pub fn init_boot_tx() {
    unsafe {
        BOOT_TX.set_journal(&BOOT_JOURNAL);
        BOOT_TX.set_cache(&BOOT_TX_CACHE);
    }
}

pub fn get_boot_tx() -> &'static mut Transaction {
    unsafe { &mut BOOT_TX }
}

pub fn get_boot_tx_cache() -> &'static mut TxCache {
    unsafe { &mut BOOT_TX_CACHE }
}

pub fn current_generation() -> usize {
    unsafe { CURRENT_GENERATION }
}

pub fn start_ctx_switch_tx() {
    unsafe {
        IN_CONTEXT_SWITCH_TX += 1;
    }
}

pub fn finish_ctx_switch_tx() {
    unsafe {
        IN_CONTEXT_SWITCH_TX -= 1;
    }
}

pub fn exit_all_ctx_switch_tx() {
    unsafe {
        IN_CONTEXT_SWITCH_TX = 0;
    }
}

pub fn in_ctx_switch_tx() -> bool {
    unsafe { IN_CONTEXT_SWITCH_TX > 0 }
}

pub fn increase_generation() {
    unsafe {
        CURRENT_GENERATION += 1;
    }
}

#[cfg(feature = "opt_list")]
pub fn recover_list_transaction() {
    let log = ListTxOpLog::get_list_tx_op_log();
    let list_op = log.get_tx_op();
    match list_op {
        ListTxOpCode::Invalid => {
            // Do nothing
            debug_print!("Nothing to recover for lists");
        }
        ListTxOpCode::WaitListPopRemoval => {
            list::recover_and_roll_forward_of_waitlist_pop_remove(log)
        }
        ListTxOpCode::WaitListInsert => {
            list::recover_and_roll_forward_of_waitlist_insert(log);
        }
        ListTxOpCode::WaitListRemoval => {
            list::recover_and_roll_forward_of_unsorted_waitlist_remove(log);
        }
        ListTxOpCode::UnsortedWaitListInsert => {
            list::recover_and_roll_forward_of_unsorted_waitlist_insert(log);
        }
        ListTxOpCode::DelayListRemoval => {
            list::recover_and_roll_forward_of_delaylist_removal(log);
        }
        ListTxOpCode::DelayListInsert => {
            list::recover_and_roll_forward_of_delaylist_insert(log);
        }
        ListTxOpCode::ReadyListInsert => {
            list::recover_and_roll_forward_of_readylist_insert(log);
        }
        _ => {
            panic!("Impossible log type");
        }
    }
    // if crashed in context switch
    crate::list::recover_and_roll_forward_context_switch(log);
}

#[cfg(feature = "opt_list")]
pub fn recover_timer_daemon_list_transaction() {
    use crate::os_dbg_print;

    let timer_task = TIME_MANAGER.get_timer_daemon_task();
    let timer_task = match timer_task {
        Some(t) => t,
        None => {
            return;
        }
    };
    let commited = timer_task.get_mut_user_tx().check_committed().is_ok();
    let log = ListTxOpLog::get_timer_list_tx_op_log();
    if !commited {
        let list_op = log.get_tx_op();
        match list_op {
            ListTxOpCode::Invalid => {
                // Never started or just finished, do nothing
                os_dbg_print!("AL Invalid, skipping recovery");
                return;
            }
            ListTxOpCode::ActiveListTxCommitted => {
                os_dbg_print!("AL committed");
            }
            ListTxOpCode::ActiveListRemove => {
                os_dbg_print!("Recovering from failed AL remove");
                if let Err(_) = list::recover_and_roll_forward_of_activelist_remove(log) {
                    return;
                }
            }
            ListTxOpCode::ActiveListPopReInsert => {
                os_dbg_print!("Recovering from failed AL pop reinsert");
                if let Err(_) = list::recover_and_roll_forward_of_activelist_pop_reinsert(log) {
                    return;
                }
            }
            ListTxOpCode::ActiveListRemoveReInsert => {
                os_dbg_print!("Recovering from failed AL remove reinsert");
                if let Err(_) = list::recover_and_roll_forward_of_activelist_remove_reinsert(log) {
                    return;
                }
            }
            _ => {
                // Should not occur
                panic!("Impossible log type");
            }
        }
        timer_task.get_mut_user_tx().commit_no_replay_roll_forward();
    }
    timer_task.user_tx_end();
    log.invalidate_activelist();
}

#[cfg(feature = "crash_safe")]
pub fn recover() {
    // check whether there is unfinished tx

    use crate::critical;
    if !is_first_boot_done() {
        // only one transaction object during boot
        debug_print!("Recovering from failed boot...");
        let tx = get_boot_tx();
        tx.roll_back_if_uncommitted();
        #[cfg(feature = "opt_list")]
        // #[cfg(not(target_arch="msp430"))]
        recover_list_transaction();
        debug_print!("Recover for Boot Done!");
    } else {
        kernel_recovery_begin_stat();
        #[cfg(feature = "opt_list")]
        {
            recover_list_transaction();
            recover_timer_daemon_list_transaction();
        }
        if in_ctx_switch_tx() {
            // no list tx here
            debug_print!("Recovering from unfinished context switch transaction...");
            let tx = get_boot_tx();
            tx.roll_back_if_uncommitted();
            exit_all_ctx_switch_tx();
        } else {
            // outstanding system call transaction ?
            // If a system-call  is just finished...
            debug_print!("Recovering from unfinished syscall transaction...");
            let tx = current().get_mut_tx();
            if is_in_critical() {
                tx.roll_back_if_uncommitted();
                exit_all_critical();
            }
        }
        kernel_recovery_end_stat();
        if !current().is_schedulable() {
            crate::os_dbg_print!("Reschedule before start....");
            unsafe {
                critical::with_no_interrupt(|_| {
                    task_switch();
                });
            }
        } else {
            current().jit_recovery();
        }
    }
}

#[cfg(not(feature = "crash_safe"))]
pub fn recover() {}

#[cfg(feature = "crash_safe")]
#[inline(always)]
pub fn idempotent_boot<F, T>(f: F)
where
    F: FnOnce() -> T,
{
    if is_first_boot_done() {
        return;
    }
    unsafe {
        BOOT_TX_CACHE.reset_ptr();
    }

    let _ = f();
    complete_first_boot();
}

#[cfg(not(feature = "crash_safe"))]
#[inline(always)]
pub fn idempotent_boot<F, T>(f: F)
where
    F: FnOnce() -> T,
{
    f();
}

#[cfg(test)]
pub fn reset_static_vars() {
    unsafe {
        FIRST_BOOT_DONE = false;
        CURRENT_GENERATION = 0;
        IN_CONTEXT_SWITCH_TX = 0;
        BOOT_TX_CACHE = TxCache::new();
        BOOT_TX = Transaction::new_empty();
        BOOT_JOURNAL = Journal::new();
    }
}
