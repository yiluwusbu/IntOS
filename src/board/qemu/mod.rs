#[cfg(target_arch = "arm")]
pub(super) fn qemu_hprintln(args: core::fmt::Arguments) {
    cortex_m_semihosting::hprintln!("{}", args);
}
#[cfg(target_arch = "arm")]
pub(super) fn qemu_hprint(args: core::fmt::Arguments) {
    cortex_m_semihosting::hprint!("{}", args);
}

pub(super) const HEAP_SIZE: usize = 8;
pub(super) const PM_HEAP_SIZE_PER_TASK: usize = 800;
pub(super) const BOOT_PM_HEAP_SIZE: usize = 1024;
pub(super) const PM_JOURNAL_SIZE: usize = 1024;
pub(super) const STACK_SIZE: usize = 1024;
pub(super) const TASK_NUM_LIMIT: usize = 6;
pub const PM_HEAP_SIZE: usize = PM_HEAP_SIZE_PER_TASK * (crate::task::TASK_NUM_LIMIT - 1);

#[cfg(not(feature = "power_failure"))]
pub const CLK_RELOAD_VALUE: u32 = 0x000f_ffff;
#[cfg(feature = "power_failure")]
pub const CLK_RELOAD_VALUE: u32 = 0xffff;
#[cfg(feature = "power_failure")]
pub const POWER_FAILURE_CYCLE: u32 = 0xffff;
#[cfg(feature = "power_failure")]
pub const CTX_SWITCH_CYCLE: u32 = 0xfffff;
