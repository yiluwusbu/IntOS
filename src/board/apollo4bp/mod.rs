pub mod init;

#[cfg(target_arch = "arm")]
pub(super) fn am_hprintln(args: core::fmt::Arguments) {
    cortex_m_semihosting::hprintln!("{}", args);
}

#[cfg(target_arch = "arm")]
pub(super) fn am_hprint(args: core::fmt::Arguments) {
    cortex_m_semihosting::hprint!("{}", args);
}

// 100ns 0x2580
// 200ns 0x4b00
// 500ns 0xbb80
// 1ms  0x17700
// 5ms  0x75300
// 10ms 0xea600
// 50ms 0x493e00

#[cfg(not(feature = "power_failure"))]
pub const CLK_RELOAD_VALUE: u32 = 0x000f_ffff;

// 100ns
#[cfg(all(feature = "power_failure", pf_freq = "100ns"))]
pub const CLK_RELOAD_VALUE: u32 = 0x2580;
#[cfg(all(feature = "power_failure", pf_freq = "100ns"))]
pub const POWER_FAILURE_CYCLE: u32 = 0x2580;

// 200ns
#[cfg(all(feature = "power_failure", pf_freq = "200ns"))]
pub const CLK_RELOAD_VALUE: u32 = 0x4b00;
#[cfg(all(feature = "power_failure", pf_freq = "200ns"))]
pub const POWER_FAILURE_CYCLE: u32 = 0x4b00;

// 500ns
#[cfg(all(feature = "power_failure", pf_freq = "500ns"))]
pub const CLK_RELOAD_VALUE: u32 = 0xbb80;
#[cfg(all(feature = "power_failure", pf_freq = "500ns"))]
pub const POWER_FAILURE_CYCLE: u32 = 0xbb80;

// 1ms
#[cfg(all(feature = "power_failure", pf_freq = "1ms"))]
pub const CLK_RELOAD_VALUE: u32 = 0x17700;
#[cfg(all(feature = "power_failure", pf_freq = "1ms"))]
pub const POWER_FAILURE_CYCLE: u32 = 0x17700;

// 5ms
#[cfg(all(feature = "power_failure", pf_freq = "5ms"))]
pub const CLK_RELOAD_VALUE: u32 = 0x75300;
#[cfg(all(feature = "power_failure", pf_freq = "5ms"))]
pub const POWER_FAILURE_CYCLE: u32 = 0x75300;

// #[cfg(feature="power_failure")]
// pub const CLK_RELOAD_VALUE: u32 = 0xea600;
// #[cfg(feature="power_failure")]
// pub const POWER_FAILURE_CYCLE: u32 = 0xea600;

// #[cfg(feature="power_failure")]
// pub const CLK_RELOAD_VALUE: u32 = 0xfffff;
// #[cfg(feature="power_failure")]
// pub const POWER_FAILURE_CYCLE: u32 = 0x493e00;

#[cfg(feature = "power_failure")]
pub const CTX_SWITCH_CYCLE: u32 = 0xfffff;

pub(super) const HEAP_SIZE: usize = 1024;
pub(super) const PM_HEAP_SIZE_PER_TASK: usize = 1024 * 8;
pub(super) const BOOT_PM_HEAP_SIZE: usize = 1024 * 4;
pub(super) const PM_JOURNAL_SIZE: usize = 1024;
pub(super) const STACK_SIZE: usize = 1024 * 4;
pub(super) const TASK_NUM_LIMIT: usize = 6;
pub const PM_HEAP_SIZE: usize = PM_HEAP_SIZE_PER_TASK * (crate::task::TASK_NUM_LIMIT - 1);
