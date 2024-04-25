use crate::{
    arch::arch_yield,
    critical, syscalls,
    task::{self, current, get_kernel_recovery_time},
    task_print,
    util::benchmark_clock,
};

pub mod microbench;
pub mod riotbench;

static mut BENCHMARK_DONE: [bool; 8] = [false; 8];
static mut BENCHMARK_DONE_CNT: usize = 0;
static mut WALL_CLK_BEGIN: u32 = 0;
static mut WALL_CLK_END: u32 = 0;
#[cfg(bench_task = "kv")]
const BENCHMARK_TASK_NUM: usize = 2;
#[cfg(bench_task = "bc")]
const BENCHMARK_TASK_NUM: usize = 1;
#[cfg(bench_task = "ar")]
const BENCHMARK_TASK_NUM: usize = 1;
#[cfg(bench_task = "dnn")]
const BENCHMARK_TASK_NUM: usize = 1;
#[cfg(bench_task = "em")]
const BENCHMARK_TASK_NUM: usize = 1;
#[cfg(bench_task = "mq")]
const BENCHMARK_TASK_NUM: usize = 2;
#[cfg(bench_task = "sense")]
const BENCHMARK_TASK_NUM: usize = 2;
#[cfg(bench_task = "etl")]
const BENCHMARK_TASK_NUM: usize = 1;
#[cfg(bench_task = "pred")]
const BENCHMARK_TASK_NUM: usize = 1;
#[cfg(bench_task = "stats")]
const BENCHMARK_TASK_NUM: usize = 1;
#[cfg(bench_task = "train")]
const BENCHMARK_TASK_NUM: usize = 1;
#[cfg(bench_task = "")]
const BENCHMARK_TASK_NUM: usize = 0;
#[cfg(any(
    bench_task = "ar_base",
    bench_task = "kv_base",
    bench_task = "bc_base",
    bench_task = "dnn_base",
    bench_task = "em_base",
    bench_task = "mq_base",
    bench_task = "sense_base"
))]
const BENCHMARK_TASK_NUM: usize = 1;

pub fn set_benchmark_done() {
    unsafe {
        let tid = current().get_task_id();
        critical::with_no_interrupt(|cs| {
            if !BENCHMARK_DONE[tid] {
                BENCHMARK_DONE[tid] = true;
                BENCHMARK_DONE_CNT += 1;
                crate::debug_print!("Benchmark done: Done count = {}", BENCHMARK_DONE_CNT);
            }
        });
    }
}

pub fn wall_clock_begin() {
    unsafe {
        if WALL_CLK_BEGIN == 0 {
            WALL_CLK_BEGIN = benchmark_clock();
        }
    }
}

pub fn wall_clock_end() {
    unsafe {
        if WALL_CLK_END == 0 {
            WALL_CLK_END = benchmark_clock();
        }
    }
}

pub fn get_wall_clock_begin() -> u32 {
    unsafe { WALL_CLK_BEGIN }
}

pub fn get_wall_clock_end() -> u32 {
    unsafe { WALL_CLK_END }
}

pub fn is_task_benchmark_done(tid: usize) -> bool {
    unsafe { BENCHMARK_DONE[tid] }
}

pub fn get_benchmark_done() -> usize {
    unsafe { BENCHMARK_DONE_CNT }
}

pub fn is_benchnmark_done() -> bool {
    unsafe { BENCHMARK_DONE_CNT >= BENCHMARK_TASK_NUM }
}

pub fn benchmark_reset() {
    task::current().bench_reset_user_tx();
}

pub fn benchmark_reset_pm() {
    task::current().bench_reset_user_pm_heap();
}

pub fn benchmark_start() -> u32 {
    let t = task::current();
    if !is_task_benchmark_done(t.get_task_id()) {
        t.start_stat();
    }
    benchmark_clock()
}

pub fn benchmark_end() -> u32 {
    let r = benchmark_clock();
    task::current().stop_stat();
    // arch_yield();
    r
}

pub fn print_wall_clock_time() {
    let end = get_wall_clock_end();
    let start = get_wall_clock_begin();
    crate::bench_println!(
        "Wallclock time: {}, end = {}, start = {}",
        end - start,
        end,
        start
    );
}

pub fn print_current_task_stats() {
    let task = task::current();
    let stats = task::task_get_stats(task);
    let total = stats.total_run_time;
    let kern = stats.in_kernel_run_time;
    let user = stats.user_time;
    let recovery = stats.total_recovery_time;
    // task::print_ctx_switch_stat();
    // Old Print Format

    // let kern_recovery = get_kernel_recovery_time();
    // crate::bench_println!(
    //     "Total Runtime: {}, User time: {}, In kernel time: {}, Recovery time: {}",
    //     total,
    //     user,
    //     kern,
    //     recovery
    // );
    // #[cfg(feature = "power_failure")]
    // crate::bench_println!("Runtime + User Recovery: {}, Runtime + Total Recovery: {}, User Recovery: {}, Kernel Recovery: {}, Total Recovery: {}"
    //                     ,total + recovery , total + recovery + kern_recovery, recovery, kern_recovery, recovery+kern_recovery);

    // Use better print format
    crate::util::pretty_print_task_stats_header();
    crate::util::pretty_print_task_stats(task.get_name(), total, user, kern, recovery);

    #[cfg(feature = "profile_log")]
    crate::bench_println!(
        "[Stat] Kernel Log Size: {}, User Log Size: {}",
        crate::pmem::get_klog_sz(),
        crate::pmem::get_ulog_sz()
    );
    #[cfg(feature = "profile_tx")]
    task::print_all_task_tx_stats();
}

pub fn print_all_task_stats() {
    crate::util::pretty_print_task_stats_header();
    task::print_all_task_stats();
    #[cfg(feature = "profile_tx")]
    task::print_all_task_tx_stats();
    // print_wall_clock_time();
}

pub trait Hash {
    fn hash(&self) -> usize;
}
