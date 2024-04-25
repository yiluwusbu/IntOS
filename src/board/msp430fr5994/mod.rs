pub mod peripherals;
pub mod print;

pub(super) const HEAP_SIZE: usize = 2;

///
/// Legacy hard-coded config used in OSDI submission
///

// #[cfg(any(bench_task = "ar", bench_task = "ar_base"))]
// pub(super) const PM_HEAP_SIZE_PER_TASK: usize = 300;
// #[cfg(any(bench_task = "sense", bench_task = "sense_base"))]
// pub(super) const PM_HEAP_SIZE_PER_TASK: usize = 500;
// #[cfg(any(bench_task = "kv", bench_task = "kv_base"))]
// pub(super) const PM_HEAP_SIZE_PER_TASK: usize = 700;
// #[cfg(bench_task = "etl")]
// pub(super) const PM_HEAP_SIZE_PER_TASK: usize = 120;
// #[cfg(bench_task = "pred")]
// pub(super) const PM_HEAP_SIZE_PER_TASK: usize = 4;
// #[cfg(bench_task = "stats")]
// pub(super) const PM_HEAP_SIZE_PER_TASK: usize = 28;
// #[cfg(bench_task = "train")]
// pub(super) const PM_HEAP_SIZE_PER_TASK: usize = 28;
// #[cfg(not(any(
//     bench_task = "ar",
//     bench_task = "sense",
//     bench_task = "kv",
//     bench_task = "ar_base",
//     bench_task = "sense_base",
//     bench_task = "kv_base",
//     bench_task = "etl",
//     bench_task = "pred",
//     bench_task = "stats",
//     bench_task = "train"
// )))]
// pub(super) const PM_HEAP_SIZE_PER_TASK: usize = 256;

// #[cfg(not(any(
//     bench_task = "etl",
//     bench_task = "pred",
//     bench_task = "stats",
//     bench_task = "train"
// )))]
// pub const PM_HEAP_SIZE: usize = PM_HEAP_SIZE_PER_TASK * (crate::task::TASK_NUM_LIMIT - 1);
// #[cfg(bench_task = "etl")]
// pub const PM_HEAP_SIZE: usize = PM_HEAP_SIZE_PER_TASK * 4 + 354;
// #[cfg(bench_task = "pred")]
// pub const PM_HEAP_SIZE: usize = PM_HEAP_SIZE_PER_TASK * 4 + 434;
// #[cfg(bench_task = "stats")]
// pub const PM_HEAP_SIZE: usize = PM_HEAP_SIZE_PER_TASK * 4 + 380;
// #[cfg(bench_task = "train")]
// pub const PM_HEAP_SIZE: usize = PM_HEAP_SIZE_PER_TASK * 2 + 500 + 216;

// #[cfg(not(any(bench_task = "sense", bench_task = "sense_base", timer_daemon)))]
// pub(super) const BOOT_PM_HEAP_SIZE: usize = 4;
// #[cfg(any(bench_task = "sense", bench_task = "sense_base", timer_daemon))]
// pub(super) const BOOT_PM_HEAP_SIZE: usize = 256;

// #[cfg(all(bench_task = "etl", feature = "opt_list"))]
// pub(super) const PM_JOURNAL_SIZE: usize = 64;
// #[cfg(all(bench_task = "pred", feature = "opt_list"))]
// pub(super) const PM_JOURNAL_SIZE: usize = 64;
// #[cfg(all(bench_task = "stats", feature = "opt_list"))]
// pub(super) const PM_JOURNAL_SIZE: usize = 64;
// #[cfg(all(bench_task = "train", feature = "opt_list"))]
// pub(super) const PM_JOURNAL_SIZE: usize = 64;
// #[cfg(not(feature = "opt_list"))]
// #[cfg(any(
//     bench_task = "stats",
//     bench_task = "pred",
//     bench_task = "etl",
//     bench_task = "train"
// ))]
// pub(super) const PM_JOURNAL_SIZE: usize = 200;
// #[cfg(not(any(
//     bench_task = "etl",
//     bench_task = "pred",
//     bench_task = "stats",
//     bench_task = "train"
// )))]
// pub(super) const PM_JOURNAL_SIZE: usize = 128;
// pub(super) const TASK_NUM_LIMIT: usize = 5;

///
/// Configuration generated from kparam_config.toml
///

#[cfg(kparam_config)]
include!("./kparam_config_constants.inc");

#[cfg(not(kparam_config))]
const MSP430FR5994_PM_HEAP_SIZE_PER_TASK: usize = 256;
#[cfg(not(kparam_config))]
const MSP430FR5994_BOOT_PM_HEAP_SIZE: usize = 4;
#[cfg(not(kparam_config))]
const MSP430FR5994_PM_JOURNAL_SIZE: usize = 128;
#[cfg(not(kparam_config))]
const MSP430FR5994_PM_HEAP_SIZE: usize = 512;
#[cfg(not(kparam_config))]
const MSP430FR5994_TASK_NUM_LIMIT: usize = 3;

pub(super) const PM_HEAP_SIZE_PER_TASK: usize = MSP430FR5994_PM_HEAP_SIZE_PER_TASK;
pub(super) const BOOT_PM_HEAP_SIZE: usize = MSP430FR5994_BOOT_PM_HEAP_SIZE;
pub(super) const PM_JOURNAL_SIZE: usize = MSP430FR5994_PM_JOURNAL_SIZE;
pub const PM_HEAP_SIZE: usize = MSP430FR5994_PM_HEAP_SIZE;
pub(super) const TASK_NUM_LIMIT: usize = MSP430FR5994_TASK_NUM_LIMIT;

// pub(super) const STACK_SIZE: usize = 400;
#[cfg(not(feature = "power_failure"))]
pub(super) const STACK_SIZE: usize = 400;

#[cfg(feature = "power_failure")]
pub(super) const STACK_SIZE: usize = 500;

pub(super) fn init() {
    peripherals::wdt_a_hold();

    peripherals::gpio_set_as_output_pin(peripherals::GPIOPort::P1, peripherals::GPIO_PIN0);
    peripherals::config_gpio_for_uart_default();
    peripherals::config_gpio_for_smclk();
    peripherals::pmm_unlock_lpm5();
    peripherals::configure_clock_system();
    peripherals::config_uart_default();
}

#[cfg(not(feature = "power_failure"))]
pub const CLK_RELOAD_VALUE: u16 = 65535;
//1ms
#[cfg(all(feature = "power_failure", pf_freq = "1ms"))]
pub const CLK_RELOAD_VALUE: u16 = 4000;
#[cfg(all(feature = "power_failure", pf_freq = "1ms"))]
pub const POWER_FAILURE_CYCLE: u16 = 4000;

//2ms
#[cfg(all(feature = "power_failure", pf_freq = "2ms"))]
pub const CLK_RELOAD_VALUE: u16 = 8000;
#[cfg(all(feature = "power_failure", pf_freq = "2ms"))]
pub const POWER_FAILURE_CYCLE: u16 = 8000;

// 5ms
#[cfg(all(feature = "power_failure", pf_freq = "5ms"))]
pub const CLK_RELOAD_VALUE: u16 = 20000;
#[cfg(all(feature = "power_failure", pf_freq = "5ms"))]
pub const POWER_FAILURE_CYCLE: u16 = 20000;

// 10ms
#[cfg(all(feature = "power_failure", pf_freq = "10ms"))]
pub const CLK_RELOAD_VALUE: u16 = 40000;
#[cfg(all(feature = "power_failure", pf_freq = "10ms"))]
pub const POWER_FAILURE_CYCLE: u16 = 40000;

#[cfg(feature = "power_failure")]
pub const CTX_SWITCH_CYCLE: u16 = 65535;
