use crate::marker::{TxInSafe, TxOutSafe};
use crate::pmem::JournalHandle;
use crate::syscalls::SyscallToken;
use crate::task::{current, task_get_stats, ErrorCode};
use crate::util::{benchmark_clock, get_time_diff, max, min};
use crate::{critical, debug_print, get_time_diff};

#[macro_export]
macro_rules! nv_loop {
    ($body: block) => {
        let user_tx_info = crate::task::current().get_mut_user_tx_info();
        loop {
            user_tx_info.enter_idempotent_loop();
            $body
            user_tx_info.exit_infinite_idempotent_loop();
        }
    };
}

#[macro_export]
macro_rules! nv_for_loop {
    ($i: ident, $cnt: ident, $begin: expr => $end: expr, $body: block) => {
        // crate::user::transaction::run(|j| {
        //     unsafe { $i.set($begin) };
        // });
        crate::user::transaction::fast_run(|| {
            unsafe { $i.set($begin) };
        });

        loop {
            let $cnt = unsafe {$i.get()};
            if $cnt >= $end {
                break;
            }
            let user_tx_info = crate::task::current().get_mut_user_tx_info();
            user_tx_info.enter_idempotent_loop();
            $body

            #[cfg(not(feature="opt_loop_end"))]
            {
                crate::user::transaction::run(|j| {
                    unsafe { $i.inc(1, j) };
                });
            }
            #[cfg(feature="opt_loop_end")]
            {
                #[cfg(feature="crash_safe")]
                {
                    user_tx_info.log_loop_cnt($i.as_ptr(), $cnt, 1);
                }
                #[cfg(not(feature="crash_safe"))]
                {
                    crate::user::transaction::run(|j| {
                        unsafe { $i.inc(1, j) };
                    });
                }
            }
            user_tx_info.exit_idempotent_loop();


        }
    };
    ($i: ident, $cnt: ident, $begin: expr => $end: expr, $step: expr, $body: block) => {
        // crate::user::transaction::run(|j| {
        //     unsafe { $i.set($begin) };
        // });
        crate::user::transaction::fast_run(|| {
            unsafe { $i.set($begin) };
        });
        loop {
            let $cnt = unsafe {$i.get()};
            if $cnt >= $end {
                break;
            }
            let user_tx_info = crate::task::current().get_mut_user_tx_info();
            user_tx_info.enter_idempotent_loop();
            $body
            #[cfg(not(feature="opt_loop_end"))]
            {
                crate::user::transaction::run(|j| {
                    unsafe { $i.inc($step, j) };
                });
            }
            #[cfg(feature="opt_loop_end")]
            {
                #[cfg(feature="crash_safe")]
                {
                    user_tx_info.log_loop_cnt($i.as_ptr(), $cnt, $step);
                }
                #[cfg(not(feature="crash_safe"))]
                {
                    crate::user::transaction::run(|j| {
                        unsafe { $i.inc($step, j) };
                    });
                }
            }
            user_tx_info.exit_idempotent_loop();
        }
    };
}

#[macro_export]
macro_rules! breaki {
    ($i: ident) => {
        crate::user::transaction::run(|j| {
            unsafe { $i.inc(j) };
        });
        crate::task::current().user_idempotent_loop_end();
        break;
    };
}

#[macro_export]
macro_rules! continuei {
    ($i: ident) => {
        crate::user::transaction::run(|j| {
            unsafe { $i.inc(j) };
        });
        crate::task::current().user_idempotent_loop_end();
        continue;
    };
}

fn pre_tx_hook() {
    #[cfg(feature = "profile_tx")]
    {
        let task = current();
        let stats = task_get_stats(task);
        critical::with_no_interrupt(|cs| {
            let clk = benchmark_clock();
            stats.tx_stat.usr_tx_start_time = clk;
            stats.tx_stat.in_usr_tx = true;
            stats.tx_stat.usr_tx_time = 0;
        });
    }
}

fn post_tx_hook() {
    #[cfg(feature = "profile_tx")]
    {
        let task = current();
        let stats = task_get_stats(task);
        critical::with_no_interrupt(|cs| {
            stats.tx_stat.in_usr_tx = false;
            let clk = benchmark_clock();
            if stats.stat_started {
                let elapsed = get_time_diff(clk, stats.tx_stat.usr_tx_start_time);
                stats.tx_stat.usr_tx_time += elapsed;
                crate::task::sample_user_tx(stats.tx_stat.usr_tx_time);
                stats.tx_stat.usr_tx_time_max =
                    max(stats.tx_stat.usr_tx_time, stats.tx_stat.usr_tx_time_max);
                stats.tx_stat.usr_tx_time_min =
                    min(stats.tx_stat.usr_tx_time, stats.tx_stat.usr_tx_time_min);
                stats.tx_stat.usr_tx_time_total += stats.tx_stat.usr_tx_time;
                stats.tx_stat.usr_tx_cnt += 1;
            }
        });
    }
}

#[cfg(feature = "crash_safe")]
#[inline(always)]
pub fn run<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle) -> T,
    T: TxOutSafe,
{
    let current_tx = current().get_mut_user_tx();
    let level = current_tx.get_nesting_level();
    // if level == 0 {
    // crate::os_print!("TX Ok...");
    // let mut cache_res: T = unsafe { core::mem::MaybeUninit::uninit().assume_init() };
    debug_assert_eq!(level, 0);
    if let Ok(cache_res) = current_tx.try_get_cached_result() {
        debug_print!("bypassing user TX...");
        debug_assert!(
            current().in_recovery_mode(),
            "Can't bypass TX when not recovering"
        );
        cache_res
    } else {
        // reset ptr of the syscall replay table
        pre_tx_hook();
        let r = current_tx.run(f);
        post_tx_hook();
        r
    }
    // } else {
    //     f(current_tx.get_journal())
    // }
}

#[cfg(feature = "crash_safe")]
#[inline(always)]
pub fn run_sys<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle, SyscallToken) -> T,
    T: TxOutSafe,
{
    let current_tx = current().get_mut_user_tx();
    let level = current_tx.get_nesting_level();
    debug_assert_eq!(level, 0);
    if let Ok(cache_res) = current_tx.try_get_cached_result() {
        debug_print!("bypassing user TX...");
        debug_assert!(
            current().in_recovery_mode(),
            "Can't bypass TX when not recovering"
        );
        cache_res
    } else {
        // reset ptr of the syscall replay table
        pre_tx_hook();
        current().user_tx_start();
        let r = current_tx.run_sys(f);
        current().user_tx_end();
        post_tx_hook();
        r
    }
}

#[cfg(feature = "crash_safe")]
#[inline(always)]
pub fn run_pure_sys<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(SyscallToken) -> T,
    T: TxOutSafe,
{
    let current_tx = current().get_mut_user_tx();
    let level = current_tx.get_nesting_level();
    debug_assert_eq!(level, 0);
    if let Ok(cache_res) = current_tx.try_get_cached_result() {
        debug_print!("bypassing user TX...");
        debug_assert!(
            current().in_recovery_mode(),
            "Can't bypass TX when not recovering"
        );
        cache_res
    } else {
        // reset ptr of the syscall replay table
        pre_tx_hook();
        current().user_tx_start();
        let r = current_tx.run_pure_sys(f);
        current().user_tx_end();
        post_tx_hook();
        r
    }
}

#[cfg(feature = "crash_safe")]
#[inline(always)]
pub fn fast_run<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce() -> T,
    T: TxOutSafe,
{
    let current_tx = current().get_mut_user_tx();
    let level = current_tx.get_nesting_level();
    debug_assert_eq!(level, 0);
    // crate::os_print!("TX Ok...");
    // let mut cache_res: T = unsafe { core::mem::MaybeUninit::uninit().assume_init() };
    if let Ok(cache_res) = current_tx.try_get_cached_result() {
        debug_print!("bypassing user TX...");
        debug_assert!(
            current().in_recovery_mode(),
            "Can't bypass TX when not recovering"
        );
        cache_res
    } else {
        // pre_tx_hook();
        let r = current_tx.fast_run(f);
        // post_tx_hook();
        r
    }
}

#[cfg(feature = "crash_safe")]
#[inline(always)]
pub fn try_run<F, T>(f: F) -> Result<T, ErrorCode>
where
    F: TxInSafe + FnOnce(JournalHandle) -> Result<T, ErrorCode>,
    T: TxOutSafe,
{
    let current_tx = current().get_mut_user_tx();
    // if current_tx.get_nesting_level() == 0 {
    debug_assert_eq!(current_tx.get_nesting_level(), 0);
    if let Ok(cache_res) = current_tx.try_get_cached_result() {
        debug_print!("bypassing user TX...");
        debug_assert!(
            current().in_recovery_mode(),
            "Can't bypass TX when not recovering"
        );
        cache_res
    } else {
        // reset ptr of the syscall replay table
        pre_tx_hook();
        let r = current_tx.try_run(f);
        post_tx_hook();
        r
    }
    // } else {
    //     f(current_tx.get_journal())
    // }
}

// No cache version
#[cfg(feature = "crash_safe")]
#[inline(always)]
pub fn run_once<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle) -> T,
    T: TxOutSafe,
{
    pre_tx_hook();
    let current_tx = current().get_mut_user_tx();
    let r = current_tx.run_no_replay(f);
    post_tx_hook();
    r
}

#[cfg(feature = "crash_safe")]
#[inline(always)]
pub fn run_sys_once<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle, SyscallToken) -> T,
    T: TxOutSafe,
{
    pre_tx_hook();
    let current_tx = current().get_mut_user_tx();

    // reset ptr of the syscall replay table
    current().user_tx_start();
    let r = current_tx.run_no_replay_sys(f);
    current().user_tx_end();
    post_tx_hook();
    r
}

/* ---------- crash unsafe version for comparison ------------------- */
#[cfg(not(feature = "crash_safe"))]
#[inline(always)]
pub fn run<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle) -> T,
    T: TxOutSafe,
{
    let current_tx = current().get_mut_user_tx();
    f(current_tx.get_journal())
}

#[cfg(not(feature = "crash_safe"))]
#[inline(always)]
pub fn run_sys<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle, SyscallToken) -> T,
    T: TxOutSafe,
{
    let current_tx = current().get_mut_user_tx();
    let st = unsafe { SyscallToken::new() };
    f(current_tx.get_journal(), st)
}

#[cfg(not(feature = "crash_safe"))]
#[inline(always)]
pub fn run_pure_sys<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(SyscallToken) -> T,
    T: TxOutSafe,
{
    let st = unsafe { SyscallToken::new() };
    f(st)
}

#[cfg(not(feature = "crash_safe"))]
#[inline(always)]
pub fn fast_run<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce() -> T,
    T: TxOutSafe,
{
    f()
}

#[cfg(not(feature = "crash_safe"))]
#[inline(always)]
pub fn try_run<F, T>(f: F) -> Result<T, ErrorCode>
where
    F: TxInSafe + FnOnce(JournalHandle) -> Result<T, ErrorCode>,
    T: TxOutSafe,
{
    let current_tx = current().get_mut_user_tx();
    f(current_tx.get_journal())
}

// No cache version
#[cfg(not(feature = "crash_safe"))]
#[inline(always)]
pub fn run_once<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle) -> T,
    T: TxOutSafe,
{
    let current_tx = current().get_mut_user_tx();
    let r = current_tx.run_no_replay(f);
    r
}

#[cfg(not(feature = "crash_safe"))]
#[inline(always)]
pub fn run_sys_once<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle, SyscallToken) -> T,
    T: TxOutSafe,
{
    let current_tx = current().get_mut_user_tx();
    f(current_tx.get_journal(), unsafe {
        crate::syscalls::SyscallToken::new()
    })
}

/* ---------------------------------------------------------------------- */
#[inline(always)]
pub fn idempotent_region_start() -> bool {
    current().user_tx_group_start()
}

#[inline(always)]
pub fn idempotent_region_end() {
    current().user_tx_group_end();
}

#[inline(always)]
pub fn idempotent_run<F>(f: F)
where
    F: FnOnce(),
{
    if !idempotent_region_start() {
        // bypassed...
        return;
    }
    f();
    idempotent_region_end();
}

/* Interface for Testing */
#[cfg(test)]
#[macro_export]
macro_rules! crashed_nv_for_loop {
    ($i: ident, $cnt: ident, $begin: expr => $end: expr,  $crash_loop: expr, $cp1: expr, $cp2: expr, $body: block) => {
        // crate::user::transaction::run(|j| {
        //     unsafe { $i.set($begin) };
        // });
        crate::user::transaction::fast_run(|| {
            unsafe { $i.set($begin) };
        });

        loop {
            let $cnt = unsafe {$i.get()};
            if $cnt >= $end {
                break;
            }
            let user_tx_info = crate::task::current().get_mut_user_tx_info();
            user_tx_info.enter_idempotent_loop();
            $body
            #[cfg(not(feature="opt_loop_end"))]
            {
                crate::user::transaction::run(|j| {
                    unsafe { $i.inc(1, j) };
                });
            }
            #[cfg(feature="opt_loop_end")]
            {
                if $crash_loop == $cnt {
                    crate::transaction::set_tx_loop_crash_point($cp1);
                    user_tx_info.crashed_log_loop_cnt($i.as_ptr(), $cnt, 1);
                    if $cp1 != crate::test::NO_CRASH {
                        return;
                    }
                } else {
                    user_tx_info.log_loop_cnt($i.as_ptr(), $cnt, 1);
                }
            }
            if $crash_loop == $cnt {
                crate::transaction::set_tx_loop_crash_point($cp2);
                user_tx_info.crashed_exit_idempotent_loop();
                if $cp2 != crate::test::NO_CRASH {
                    return;
                }
            } else {
                user_tx_info.exit_idempotent_loop();
            }

        }
    };
}

#[cfg(all(feature = "crash_safe", test))]
#[inline(always)]
pub fn may_crashed_run_sys<F, T>(crash: bool, f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle, SyscallToken) -> T,
    T: TxOutSafe,
{
    let current_tx = current().get_mut_user_tx();
    let level = current_tx.get_nesting_level();
    debug_assert_eq!(level, 0);
    if let Ok(cache_res) = current_tx.try_get_cached_result() {
        debug_print!("bypassing user TX...");
        debug_assert!(
            current().in_recovery_mode(),
            "Can't bypass TX when not recovering"
        );
        cache_res
    } else {
        // reset ptr of the syscall replay table
        current().user_tx_start();
        let r = current_tx.may_crashed_run_sys(crash, f);
        if !crash {
            current().user_tx_end();
        }
        r
    }
}

#[cfg(all(feature = "crash_safe", test))]
#[inline(always)]
pub fn may_crashed_run<F, T>(crash: bool, f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle) -> T,
    T: TxOutSafe,
{
    let current_tx = current().get_mut_user_tx();
    let level = current_tx.get_nesting_level();
    debug_assert_eq!(level, 0);
    if let Ok(cache_res) = current_tx.try_get_cached_result() {
        debug_print!("bypassing user TX...");
        debug_assert!(
            current().in_recovery_mode(),
            "Can't bypass TX when not recovering"
        );
        cache_res
    } else {
        // reset ptr of the syscall replay table
        let r = current_tx.may_crashed_run(crash, f);
        r
    }
}

#[cfg(all(feature = "crash_safe", test))]
#[inline(always)]
pub fn crashed_run_sys<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle, SyscallToken) -> T,
    T: TxOutSafe,
{
    may_crashed_run_sys(true, f)
}

#[cfg(all(feature = "crash_safe", test))]
#[inline(always)]
pub fn crashed_run<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle) -> T,
    T: TxOutSafe,
{
    may_crashed_run(true, f)
}

#[cfg(all(feature = "crash_safe", test))]
#[inline(always)]
pub fn may_crashed_run_sys_once<F, T>(crash: bool, f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle, SyscallToken) -> T,
    T: TxOutSafe,
{
    let current_tx = current().get_mut_user_tx();

    // reset ptr of the syscall replay table
    current().user_tx_start();
    let r = current_tx.may_crashed_run_no_replay_sys(crash, f);
    if !crash {
        current().user_tx_end();
    }
    r
}

#[cfg(all(feature = "crash_safe", test))]
#[inline(always)]
pub fn may_crashed_run_once<F, T>(crash: bool, f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle) -> T,
    T: TxOutSafe,
{
    let current_tx = current().get_mut_user_tx();
    let r = current_tx.may_crashed_run_no_replay(crash, f);
    r
}
