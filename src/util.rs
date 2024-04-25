use core::ptr::NonNull;
use core::sync::atomic::compiler_fence;

use crate::arch::{self, ARCH_ALIGN};
use crate::heap::MemStat;
use crate::recover::in_ctx_switch_tx;
use crate::syscalls::sys_get_pmem_stat;
use crate::task::{self, get_current_tx, Task};
use crate::{board, critical};

pub fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

pub fn arch_addr_align_up(addr: usize) -> usize {
    (addr + ARCH_ALIGN - 1) & !(ARCH_ALIGN - 1)
}

pub fn cast_to_u8_ptr<T>(r: &T) -> NonNull<u8> {
    unsafe { NonNull::new_unchecked(r as *const T as *mut u8) }
}
// #[cfg(feature="debug")]
#[macro_export]
macro_rules! debug_print {
    ($($arg: expr), *) => {
        #[cfg(debug_assertions)]
        {
            crate::board_hprint!("[DEBUG] Task: {} :", crate::util::task_name());
            crate::board_hprintln!($($arg), *);
        }
    };
}

#[macro_export]
macro_rules! os_print {
    ($($arg: expr), *) => {
        crate::board_hprintln!($($arg), *);
    };
}

#[macro_export]
macro_rules! os_dbg_print {
    ($($arg: expr), *) => {
        #[cfg(debug_assertions)]
        {
            crate::board_hprintln!($($arg), *);
        }
    };
}

#[macro_export]
macro_rules! bench_println {
    ($($arg: expr), *) => {
        crate::board_hprint!("[Task {}]: ", crate::util::task_name());
        crate::board_hprintln!($($arg), *);
    };
}

#[macro_export]
macro_rules! bench_println_no_header {
    ($($arg: expr), *) => {
        crate::board_hprintln!($($arg), *);
    };
}

#[macro_export]
macro_rules! task_print {
    ($($arg: expr), *) => {
        crate::board_hprint!("[Task {}]: ", crate::util::task_name());
        crate::board_hprintln!($($arg), *);
    };
}

#[macro_export]
macro_rules! bench_dbg_print {
    ($($arg: expr), *) => {
        #[cfg(feature="debug_bench")]
        {
            crate::board_hprint!("[DEBUG Task {}]: ", crate::util::task_name());
            crate::board_hprintln!($($arg), *);
        }
    };
}

#[macro_export]
macro_rules! list_dbg_print {
    ($($arg: expr), *) => {
        #[cfg(debug_assertions)]
        crate::board_hprintln!($($arg), *);
    };
}

#[macro_export]
macro_rules! debug_print_no_header {
    ($($arg: expr), *) => {
        #[cfg(debug_assertions)]
        crate::board_hprintln!($($arg), *);
    };
}

#[macro_export]
macro_rules! os_print_no_header {
    ($($arg: expr), *) => {
        crate::board_hprintln!($($arg), *);
    };
}

#[macro_export]
macro_rules! task_end {
    () => {
        loop {}
    };
}

pub fn pretty_print_task_stats_header() {
    bench_println_no_header!("---------Benchmark Completed!----------\nStats:");
    bench_println_no_header!(
        "{0: <12} | {1: <12} | {2: <12} | {3: <12} | {4: <12}",
        "task",
        "run time",
        "user time",
        "kernel time",
        "recover time"
    );
}

pub fn pretty_print_task_stats(
    task: &'static str,
    runtime: u32,
    utime: u32,
    ktime: u32,
    recover_time: u32,
) {
    bench_println_no_header!(
        "{0: <12} | {1: <12} | {2: <12} | {3: <12} | {4: <12}",
        task,
        runtime,
        utime,
        ktime,
        recover_time
    );
}

// #[cfg(not(feature="debug"))]
// #[macro_export]
// macro_rules! debug_print {
//     ($($arg: expr), *) => {
//         cortex_m_semihosting::hprintln!("[DEBUG] Task: {}", crate::util::task_name()).unwrap();
//         cortex_m_semihosting::hprintln!($($arg), *).unwrap();
//     };
// }

pub fn task_name() -> &'static str {
    if unsafe { task::is_scheduler_started() } {
        if in_ctx_switch_tx() {
            "CTX Switcher"
        } else {
            task::current().get_name()
        }
    } else {
        "BOOT"
    }
}

pub fn debug_replay_cache() {
    let tail = task::current().get_syscall_replay_cache().get_tail();
    let ptr = task::current().get_syscall_replay_cache().get_ptr();
    task_print!("replay cache tail: {}, replay cache ptr: {}", tail, ptr);
}

pub fn debug_user_tx_cache() {
    task::current().debug_user_tx();
}

pub fn debug_syscall_tx_cache() {
    let cache = task::current().get_sys_tx_cache();
    let tail = cache.get_tx_id_of_tail();
    let ptr = cache.get_tx_id_of_ptr();
    debug_print!("Syscall Tx cache tail: {}, ptr: {}", tail, ptr);
}

pub fn debug_kernel_tx_journal() {
    let tx = get_current_tx();
    let j = tx.get_journal();
    debug_print!("Kernel Tx Journal: {}", j.get());
}

pub fn debug_list_tx() {
    let task = task::current();
    debug_print!("List Tx done status: {}", task.list_transaction_done());
}

#[inline(always)]
pub fn compiler_pm_fence() {
    #[cfg(not(target_arch = "msp430"))]
    compiler_fence(core::sync::atomic::Ordering::SeqCst);
    #[cfg(target_arch = "msp430")]
    msp430::asm::barrier();
}

#[macro_export]
macro_rules! get_time_diff {
    ($e: expr, $s: expr) => {
        // if $e < $s {
        //     os_print!("end: {}, start: {}",$e,$s);
        // }
        get_time_diff($e, $s)
    };
}

pub fn print_pmem_used() {
    let mem_stat = sys_get_pmem_stat();
    task_print!("PMEM used: {} bytes", mem_stat.mem_used);
}

pub fn print_all_task_pmem_used() {
    task::print_all_task_pm_usage();
}

pub fn print_tx_stats() {}

pub fn get_time_diff(mut end: u32, start: u32) -> u32 {
    #[cfg(feature = "profile_tx")]
    {
        if end < start {
            return 0;
        } else {
            return end - start;
        }
    }
    #[cfg(feature = "power_failure")]
    {
        if end < start {
            crate::os_dbg_print!("Warning: time overflowed....");
            // overflowed...
            #[cfg(not(feature = "msp430_use_timerb"))]
            {
                end += board!(CLK_RELOAD_VALUE) as u32;
            }
            #[cfg(feature = "msp430_use_timerb")]
            {
                return 0;
            }
        }
    }
    return end - start;
}

pub fn max<T>(v1: T, v2: T) -> T
where
    T: PartialOrd,
{
    if v1 >= v2 {
        v1
    } else {
        v2
    }
}

pub fn min<T>(v1: T, v2: T) -> T
where
    T: PartialOrd,
{
    if v1 <= v2 {
        v1
    } else {
        v2
    }
}

pub fn bubble_sort<T: PartialOrd>(input: &mut [T]) {
    if input.len() < 2 {
        return;
    }

    let input_len = input.len();

    for i in (0..input_len).rev() {
        let mut has_swapped = false;
        for j in 0..i {
            if input[j] > input[j + 1] {
                input.swap(j, j + 1);
                has_swapped = true;
            }
        }
        if !has_swapped {
            break;
        }
    }
}

pub fn benchmark_clock() -> u32 {
    #[cfg(board = "qemu")]
    return arch::arch_get_system_clock_cycles();
    #[cfg(board = "apollo4bp")]
    return arch::arch_get_cycle_cnt();
    #[cfg(board = "msp430fr5994")]
    return arch::arch_get_system_clock_cycles();
    #[cfg(board = "test")]
    return 0;
}

#[cfg(test)]
#[macro_export]
macro_rules! crash_point {
    ($nm: ident, $e: expr) => {
        #[cfg(test)]
        {
            if concat_idents!(get_, $nm, _crash_point)() == $e {
                return;
            }
        }
    };
    ($nm: ident, $e: expr, $action_t: block, $action_f: block) => {
        #[cfg(test)]
        {
            if concat_idents!(get_, $nm, _crash_point)() == $e {
                $action_t
            } else {
                $action_f
            }
        }
    };
    ($nm: ident, $e: expr, $r: expr) => {
        #[cfg(test)]
        {
            if concat_idents!(get_, $nm, _crash_point)() == $e {
                return $r;
            }
        }
    };
}
#[cfg(test)]
#[macro_export]
macro_rules! set_crash_point {
    ($nm: ident, $e: expr) => {
        #[cfg(test)]
        {
            concat_idents!(set_, $nm, _crash_point)($e);
        }
    };
}
