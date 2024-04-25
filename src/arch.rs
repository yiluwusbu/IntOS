#[cfg(target_arch = "arm")]
pub mod cortex_m;
#[cfg(target_arch = "msp430")]
pub mod msp430;
#[cfg(not(test))]
use crate::main;
use crate::{
    debug_print, heap, os_dbg_print, os_print,
    recover::{complete_first_boot, idempotent_boot, increase_generation, init_boot_tx, recover},
    task, time,
    transaction::run,
};

#[cfg(target_arch = "arm")]
macro_rules! arch {
    ($fn: ident ($($tt:tt)*)) => {
        cortex_m::$fn($($tt)*)
    };
    ($ty: ident) => {
        cortex_m::$ty
    };
}

#[cfg(target_arch = "msp430")]
macro_rules! arch {
    ($fn: ident ($($tt:tt)*)) => {
        msp430::$fn($($tt)*)
    };

    ($ty: ident) => {
        msp430::$ty
    };
}

#[cfg(not(test))]
pub type ArchTimeType = arch!(Time);
#[cfg(test)]
pub type ArchTimeType = u16;

#[inline(always)]
pub fn start_kernel() {
    unsafe { task::set_scheduler_started() };
    // first boot is completed
    complete_first_boot();
    #[cfg(not(test))]
    {
        arch!(start_kernel());
    }
    // cortex_m::start_kernel();
}

#[inline(always)]
pub fn disable_interrupt() {
    #[cfg(not(test))]
    unsafe {
        arch!(disable_interrupt())
    };
}

#[inline(always)]
pub fn enable_interrupt() {
    #[cfg(not(test))]
    unsafe {
        arch!(enable_interrupt())
    };
}

#[inline(always)]
pub fn arch_yield() {
    #[cfg(not(test))]
    unsafe {
        arch!(arch_yield())
    };
    #[cfg(test)]
    panic!("Impossible to yield!");
}

#[cfg(not(test))]
#[inline(always)]
pub fn arch_get_cycle_cnt() -> u32 {
    arch!(get_cycle_cnt())
}

#[cfg(test)]
pub fn arch_get_cycle_cnt() -> u32 {
    0
}

#[cfg(not(test))]
#[inline(always)]
pub fn arch_is_cycle_cnt_supported() -> bool {
    arch!(is_cycle_cnt_supported())
}

#[cfg(test)]
pub fn arch_is_cycle_cnt_supported() -> bool {
    false
}

#[inline(always)]
pub fn arch_enable_cycle_cnt() {
    #[cfg(not(test))]
    arch!(enable_cycle_cnt());
}

#[cfg(not(test))]
#[inline(always)]
pub fn arch_is_cycle_cnt_enabled() -> bool {
    arch!(is_cycle_cnt_enabled())
}

#[cfg(test)]
pub fn arch_is_cycle_cnt_enabled() -> bool {
    false
}

#[inline(always)]
pub fn initialize_stack(stack_top: usize, task_func: usize, param: usize) -> usize {
    #[cfg(not(test))]
    return unsafe { arch!(initialize_stack(stack_top, task_func, param)) };
    #[cfg(test)]
    return 0;
}

#[inline(always)]
pub fn arch_get_system_clock_cycles() -> u32 {
    #[cfg(not(test))]
    return arch!(get_system_clock_cycles());
    #[cfg(test)]
    return 0;
}

fn arch_config_cycle_count() {
    if !arch_is_cycle_cnt_supported() {
        os_dbg_print!("[Warning] Cycle cnt is not supported!");
    } else {
        os_dbg_print!("[Ok] Cycle cnt is supported!");
    }
    arch_enable_cycle_cnt();
    if arch_is_cycle_cnt_enabled() {
        os_dbg_print!("[Ok] Cycle cnt is enabled!");
    } else {
        os_dbg_print!("[Error] Cycle cnt is NOT enabled!");
    }
}

#[cfg(not(test))]
#[export_name = "recover_and_boot"]
pub fn recover_and_boot() {
    use crate::{recover::current_generation, task::reset_scheduler_started};
    debug_print!("Booting..., gen={}", current_generation());
    #[cfg(feature = "power_failure")]
    {
        if current_generation() == 0 {
            arch_config_cycle_count();
        } else {
            #[cfg(board = "msp430fr5994")]
            crate::board::msp430fr5994::peripherals::setup_timer_interrupt();
        }
    }
    #[cfg(not(feature = "power_failure"))]
    arch_config_cycle_count();
    unsafe { reset_scheduler_started() };
    increase_generation();
    init_boot_tx();
    // before doing anything, run a recovery protocal
    recover();
    run_boot_sequence();
}

// #[inline(always)]
#[cfg(not(test))]
pub fn run_boot_sequence() {
    // idempotent boot sequence
    idempotent_boot(|| {
        heap::init();
        debug_print!("creating idle task...");
        task::create_idle_task();
        #[cfg(any(bench_task = "sense", bench_task = "sense_base", timer_daemon))]
        {
            debug_print!("creating timer daemon task...");
            time::TIME_MANAGER.create_daemon_timer_task();
        }

        debug_print!("calling main...");
        main();
    });
    debug_print!("Starting Kernel...");
    crate::arch::start_kernel();
    loop {}
}

pub const ARCH_ALIGN: usize = core::mem::size_of::<usize>();
