// #![cfg_attr(not(test), no_std)]
// #![cfg_attr(not(test), no_main)]
#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(auto_traits)]
#![feature(negative_impls)]
#![allow(unused)]
#![feature(allocator_api)]
#![feature(alloc_error_handler)]
// #![allow(invalid_reference_casting)]
#![feature(asm_experimental_arch)]
#![feature(abi_msp430_interrupt)]
#![feature(concat_idents)]
// #![allow(incomplete_features)]
// #![feature(generic_const_exprs)]

pub mod arch;
#[cfg(not(test))]
pub mod benchmarks;
pub mod board;
pub mod critical;
pub mod event_group;
pub mod heap;
pub mod list;
pub mod marker;
pub mod pmem;
pub mod queue;
pub mod recover;
pub mod semaphore;
pub mod syscalls;
pub mod task;
pub mod tests;
pub mod time;
pub mod transaction;
pub mod user;
pub mod util;
pub mod vec;
// for users own applications
pub mod app;

use core::{cell::UnsafeCell, ptr::NonNull};
use event_group::{EventGroup, EventGroupHandle};
use macros::app;
use pmem::{Journal, JournalHandle, PMPtr};
use semaphore::Semaphore;
use syscalls::*;
use task::{get_current_task_ptr, get_current_tx, is_scheduler_started};
use time::Time;
use user::{
    pbox::PBox,
    pqueue::PQueue,
    pthread,
    pvec::PVec,
    transaction::{self as tx, idempotent_run},
};
use util::{debug_syscall_tx_cache, debug_user_tx_cache};

use core::panic::PanicInfo;

use crate::{
    task::{current, ErrorCode},
    user::{parc::PArc, pbox::Ptr, pmutex::PMutex},
    util::{debug_kernel_tx_journal, debug_replay_cache},
};

static OS_VERSION: &str = "0.1";

#[cfg(idem)]
#[cfg(any(feature = "opt_list", feature = "crash_safe"))]
compile_error!("Can't have opt_list or crash_safe feature while using idempotent processing..");

#[cfg(baseline)]
#[cfg(any(feature = "opt_list", feature = "crash_safe"))]
compile_error!("Can't have opt_list or crash_safe feature while using baseline..");

#[cfg(sram_baseline)]
#[cfg(any(feature = "opt_list", feature = "crash_safe"))]
compile_error!("Can't have opt_list or crash_safe feature while using sram baseline..");

#[cfg(all(feature = "power_failure", pf_freq = ""))]
compile_error!("Must specify power failure frequency");

#[cfg(not(test))]
#[panic_handler]
fn panic(panic: &PanicInfo<'_>) -> ! {
    let task_name = util::task_name();
    os_print!("PANIC! task_name: {}", task_name);
    os_print!("{}", panic);
    // crate::task::debug_display_all_sched_list();
    // crate::task::debug_display_delayed_list();
    loop {}
}

fn show_additional_features() {
    #[cfg(feature = "opt_tx_cache_space")]
    let f1 = "Opt Tx Cache Space";
    #[cfg(not(feature = "opt_tx_cache_space"))]
    let f1 = "Normal Tx Cache";

    #[cfg(feature = "opt_loop_end")]
    let f2 = "Opt Loop End";
    #[cfg(not(feature = "opt_loop_end"))]
    let f2 = "Normal Loop End";

    os_print!("[*] Opt Features: {} ; {}", f1, f2);
}

fn show_feature() {
    #[cfg(not(feature = "crash_safe"))]
    os_print!("[*] Crash unsafe");
    #[cfg(feature = "crash_safe")]
    os_print!("[*] Crash safe!");

    #[cfg(not(feature = "opt_list"))]
    os_print!("[*] Normal List");
    #[cfg(feature = "opt_list")]
    os_print!("[*] Opt list");
    #[cfg(idem)]
    os_print!("[*] Idem Processing");
    #[cfg(baseline)]
    os_print!("[*] Baseline");
    #[cfg(sram_baseline)]
    os_print!("[*] SRAM Baseline");
    show_additional_features();
    #[cfg(feature = "power_failure")]
    os_print!("[*] Failure Injection");
}

#[cfg(not(test))]
#[export_name = "main"]
fn main() {
    // os_print!("Hello, world!");
    #[cfg(feature = "verbose_os_info")]
    {
        os_print!("OS_VERSION: {}", OS_VERSION);
        show_feature();
    }
    // debug_print!(
    //     "size of ptr: {}",
    //     core::mem::size_of::<Option<PMPtr<task::Task>>>()
    // );

    // Create bunch of testing tasks
    #[cfg(app = "demo")]
    task::register_app_no_param("ping", 1, crate::app::demo::task_ping);
    // task::register_app_no_param("ping", 1, crate::tests::examples::task_ping);
    // task::register_app_no_param("eg", 1, crate::tests::examples::task_eg);
    // task::register_app_no_param("locker 1", 1, crate::tests::examples::task_lock_1);
    // task::register_app_no_param("timer_user", 1, crate::tests::examples::task_timer);
    // task::register_app_no_param("lock_safety", 1, crate::tests::examples::lock_safety);
    // task::register_app_no_param("thread", 1, crate::tests::examples::task_test_closure);
    // task::register_app_no_param("pbox", 1, crate::tests::examples::task_pbox);
    // task::register_app_no_param("pqueue", 1, crate::tests::examples::task_test_pqueue);

    #[cfg(bench_task = "kv")]
    benchmarks::microbench::kvstore::register();
    #[cfg(bench_task = "sense")]
    benchmarks::microbench::periodic_sensing::register();
    #[cfg(bench_task = "em")]
    benchmarks::microbench::event_monitor::register();
    #[cfg(bench_task = "mq")]
    benchmarks::microbench::message_queue::register();
    #[cfg(bench_task = "dnn")]
    benchmarks::microbench::dnn::register();
    #[cfg(bench_task = "bc")]
    //benchmarks::microbench::bitcount_cmp::register();
    benchmarks::microbench::bitcount::register();
    #[cfg(bench_task = "ar")]
    //benchmarks::microbench::ar_cmp::register();
    benchmarks::microbench::activity_recognition::register();
    #[cfg(bench_task = "etl")]
    benchmarks::riotbench::etl::register();
    #[cfg(bench_task = "pred")]
    benchmarks::riotbench::pred::register();
    #[cfg(bench_task = "stats")]
    benchmarks::riotbench::stats::register();
    #[cfg(bench_task = "train")]
    benchmarks::riotbench::train::register();
    // baselines
    #[cfg(bench_task = "kv_base")]
    benchmarks::microbench::microbench_baseline::kvstore::register();
    #[cfg(bench_task = "sense_base")]
    benchmarks::microbench::microbench_baseline::periodic_sensing::register();
    #[cfg(bench_task = "em_base")]
    benchmarks::microbench::microbench_baseline::event_monitor::register();
    #[cfg(bench_task = "mq_base")]
    benchmarks::microbench::microbench_baseline::message_queue::register();
    #[cfg(bench_task = "dnn_base")]
    benchmarks::microbench::microbench_baseline::dnn::register();
    #[cfg(bench_task = "bc_base")]
    benchmarks::microbench::microbench_baseline::bitcount::register();
    #[cfg(bench_task = "ar_base")]
    benchmarks::microbench::microbench_baseline::activity_recognition::register();
}

#[cfg(test)]
pub mod test {
    use super::*;
    use crate::{
        recover::{increase_generation, init_boot_tx},
        task::{get_task_cnt, mock_task_switch},
        time::TimeManager,
        user::transaction,
    };
    use core::mem::forget;
    pub const NO_CRASH: usize = 10000;
    static TASK_NAMES: [&str; 4] = ["Test1", "Test2", "Test3", "Test4"];
    // Helper functions
    pub fn syscall_ptr() -> usize {
        current().get_syscall_replay_cache().get_ptr()
    }

    pub fn syscall_tail() -> usize {
        current().get_syscall_replay_cache().get_tail()
    }

    pub fn user_tx_ptr() -> usize {
        current().get_user_tx_cache().get_tx_id_of_ptr()
    }

    pub fn user_tx_tail() -> usize {
        current().get_user_tx_cache().get_tx_id_of_tail()
    }

    pub fn sys_tx_ptr() -> usize {
        current().get_sys_tx_cache().get_tx_id_of_ptr()
    }

    pub fn sys_tx_tail() -> usize {
        current().get_sys_tx_cache().get_tx_id_of_tail()
    }

    pub fn user_tx_tail_at(level: usize) -> usize {
        current().get_user_tx_info().get_tx_tail_id_at_level(level)
    }

    pub fn boot_tx_ptr() -> usize {
        recover::get_boot_tx_cache().get_tx_id_of_ptr()
    }

    pub fn boot_tx_tail() -> usize {
        recover::get_boot_tx_cache().get_tx_id_of_tail()
    }

    pub fn user_tx_stack_top() -> usize {
        current().get_user_tx_info().get_stack_top() as usize
    }

    pub fn clean_up() {
        syscalls::set_syscall_end_crash_point(10000);
        task::reset_static_vars();
        recover::reset_static_vars();
        heap::reset_static_vars();
    }

    fn boot_common(task_cnt: usize) {
        clean_up();
        recover::increase_generation();
        recover::init_boot_tx();
        heap::init();
        os_print!("Creating Test Task");
        for i in 0..task_cnt {
            let _ = task::create_task_static(TASK_NAMES[i], 1, 0, 0, heap::PM_HEAP_SIZE_PER_TASK)
                .unwrap();
        }
    }

    pub fn mock_boot(task_cnt: usize) {
        boot_common(task_cnt);
        arch::start_kernel();
    }
    pub fn mock_boot_with_timer_daemon(task_cnt: usize) {
        boot_common(task_cnt);
        time::TIME_MANAGER.create_daemon_timer_task();
        arch::start_kernel();
    }

    pub fn mock_reboot() {
        syscalls::set_syscall_end_crash_point(10000);
        recover::increase_generation();
        recover::init_boot_tx();
        arch::start_kernel();
    }

    #[test]
    fn test_tx_execution() {
        mock_boot(1);

        transaction::run_sys(|j, t| {
            let mut pbox = PBox::new(5, t);
            let v = pbox.as_mut(j);
            task_print!("value is {}", *v);
            *v += 10;
            task_print!("new value is {}", *v);
        });
    }

    #[derive(Clone, Copy)]
    struct TestStruct {
        data1: usize,
        data2: u16,
        data3: u32,
        data4: bool,
    }

    fn basic_tx_crash_task(crash_point: usize) -> (Option<PBox<i32>>, Option<PBox<TestStruct>>) {
        let crash = if crash_point > 4 { false } else { true };
        let r = transaction::may_crashed_run_sys(crash, |j, t| {
            if crash_point == 0 {
                return (None, None);
            }
            let mut boxed_i = PBox::new(6, t);
            if crash_point == 1 {
                return (Some(boxed_i), None);
            }
            let i = boxed_i.as_mut(j);
            task_print!("i = {}", i);
            *i += 6;
            assert_eq!(*i, 12);
            task_print!("i addr = {:#X}", i as *const i32 as usize);
            if crash_point == 2 {
                return (Some(boxed_i), None);
            }

            let mut boxed_s = PBox::new(
                TestStruct {
                    data1: 1,
                    data2: 2,
                    data3: 3,
                    data4: false,
                },
                t,
            );

            if crash_point == 3 {
                return (Some(boxed_i), Some(boxed_s));
            }
            let s = boxed_s.as_mut(j);
            task_print!("s addr = {:#X}", s as *const TestStruct as usize);
            task_print!(
                "data1 = {}, data2 = {}, data3 = {}, data4 = {}",
                s.data1,
                s.data2,
                s.data3,
                s.data4
            );
            s.data1 += 10;
            s.data2 += 10;
            s.data3 += 10;
            s.data4 = true;
            assert_eq!(s.data1, 11);
            assert_eq!(s.data4, true);

            return (Some(boxed_i), Some(boxed_s));
        });
        return r;
    }

    #[test]
    fn test_basic_tx_crash_recovery_1() {
        fn run(crash_point: usize) {
            mock_boot(1);
            let (r1, r2) = basic_tx_crash_task(crash_point);
            // recovery
            mock_reboot();
            current().jit_recovery();

            let j = unsafe { JournalHandle::new_dummy() };

            if crash_point > 4 {
                let boxed_i = r1.unwrap();
                let i = boxed_i.as_ref(j);
                let boxed_s = r2.unwrap();
                let s = boxed_s.as_ref(j);
                assert_eq!(*i, 12);
                assert_eq!(s.data1, 11);
                assert_eq!(s.data2, 12);
                assert_eq!(s.data3, 13);
                assert_eq!(s.data4, true);
                forget(boxed_i);
                forget(boxed_s);
                return;
            }

            if !r1.is_none() {
                let boxed_i = r1.unwrap();
                let i = boxed_i.as_ref(j);
                assert_eq!(*i, 6);
                forget(boxed_i);
            }

            if !r2.is_none() {
                let boxed_s = r2.unwrap();
                let s = boxed_s.as_ref(j);
                assert_eq!(s.data1, 1);
                assert_eq!(s.data2, 2);
                assert_eq!(s.data3, 3);
                assert_eq!(s.data4, false);
                forget(boxed_s);
            }
        }

        for cp in 0..6 {
            os_print!("Running crash point {}", cp);
            run(cp);
        }
    }

    #[test]
    fn test_basic_tx_crash_recovery_2() {
        fn run(crash_point: usize) {
            mock_boot(1);
            let (r1, r2) = basic_tx_crash_task(crash_point);
            forget(r1);
            forget(r2);
            // recovery
            mock_reboot();
            current().jit_recovery();

            let (r1, r2) = basic_tx_crash_task(10000);
            let user_tx = current().get_mut_user_tx();

            assert_eq!(user_tx.get_nesting_level(), 0);
            assert_eq!(syscall_tail(), 0);
            assert_eq!(user_tx_tail(), 1);

            let boxed_i = r1.unwrap();
            let boxed_s = r2.unwrap();

            let j = unsafe { JournalHandle::new_dummy() };
            let i = boxed_i.as_ref(j);
            task_print!("addr = {:#X}", i as *const i32 as usize);
            let s = boxed_s.as_ref(j);

            assert_eq!(*i, 12);
            assert_eq!(s.data1, 11);
            assert_eq!(s.data2, 12);
            assert_eq!(s.data3, 13);
            assert_eq!(s.data4, true);

            forget(boxed_i);
            forget(boxed_s);
        }
        for cp in 0..6 {
            os_print!("Running crash point {}", cp);
            run(cp);
        }
    }

    fn pvec_tx_task(cp: usize) -> PVec<i32> {
        fn show(v: &PVec<i32>) {
            for elem in v.iter() {
                os_print!("val = {}", elem);
            }
        }
        let crash = cp <= 8;
        let v = transaction::may_crashed_run_sys(crash, |j, t| {
            let i = PBox::new(10, t);
            let mut vec = PVec::<i32>::new(10, t);
            if cp == 0 {
                show(&vec);
                return vec;
            }
            vec.push(1, j);
            if cp == 1 {
                show(&vec);
                return vec;
            }
            vec.push(2, j);
            if cp == 2 {
                show(&vec);
                return vec;
            }
            vec.pop(j);
            if cp == 3 {
                show(&vec);
                return vec;
            }
            vec.push(3, j);
            if cp == 4 {
                show(&vec);
                return vec;
            }
            vec.push(4, j);
            if cp == 5 {
                show(&vec);
                return vec;
            }
            vec.push(5, j);
            if cp == 6 {
                show(&vec);
                return vec;
            }
            vec.push(6, j);
            if cp == 7 {
                show(&vec);
                return vec;
            }
            vec.pop(j);
            if cp == 8 {
                show(&vec);
                return vec;
            }
            vec.pop(j);
            vec
        });

        v
    }

    #[test]
    fn test_basic_tx_crash_recovery_3() {
        fn run(cp: usize) {
            mock_boot(1);
            let v = pvec_tx_task(cp);
            forget(v);
            mock_reboot();
            current().jit_recovery();
            let v = pvec_tx_task(1000);
            let j = unsafe { JournalHandle::new_dummy() };
            assert_eq!(user_tx_tail(), 1);
            assert_eq!(v.len(j), 3);
            let mut i = 0;
            let expected_vals = [1, 3, 4];
            for elem in v.iter() {
                os_print!("val = {}", elem);
                // assert_eq!(*elem, expected_vals[i]);
                i += 1;
            }
            forget(v);
        }

        for i in 0..10 {
            run(i)
        }
    }

    #[test]
    fn test_tx_replay() {
        mock_boot(1);
        // start execution...
        fn task(crash: bool) {
            let (bi, bj) = transaction::run_sys(|j, t| {
                let mut bi = PBox::new(3, t);
                let i = bi.as_mut(j);
                *i = 10;
                let mut bj = PBox::new(6, t);
                let j_ref = bj.as_mut(j);
                *j_ref = 20;
                (bi, bj)
            });

            let mut bk = transaction::run_sys(|j, t| {
                let bk = Ptr::new(
                    TestStruct {
                        data1: 1,
                        data2: 2,
                        data3: 3,
                        data4: false,
                    },
                    t,
                );
                bk
            });
            if crash {
                transaction::crashed_run(move |j| {
                    let bk_ref = bk.as_pref();
                    let (v, bk_ref) = bk_ref.read(|v| *v, j);
                    task_print!(" i is: {}", bi.as_ref(j));
                    task_print!(" j is: {}", bj.as_ref(j));
                    task_print!("struct value is: {},{},{}", v.data1, v.data2, v.data3);
                    bk_ref.write(
                        |v| {
                            v.data1 = 10;
                            v.data2 = 20;
                            v.data3 = 30;
                        },
                        j,
                    );
                    // three sys_pfree here
                });
            } else {
                let ret = transaction::run(move |j| {
                    let bk_ref = bk.as_pref();
                    let (v, bk_ref) = bk_ref.read(|v| *v, j);
                    assert_eq!(v.data1, 1);
                    assert_eq!(v.data2, 2);
                    assert_eq!(v.data3, 3);

                    task_print!(" i is: {}", bi.as_ref(j));
                    task_print!(" j is: {}", bj.as_ref(j));
                    task_print!("struct value is: {},{},{}", v.data1, v.data2, v.data3);
                    let bk_ref = bk_ref.write(
                        |v| {
                            v.data1 = 10;
                            v.data2 = 20;
                            v.data3 = 30;
                        },
                        j,
                    );
                    let ret = *bk_ref.as_ref();
                    ret
                });
                assert_eq!(ret.data1, 10);
                assert_eq!(ret.data2, 20);
                assert_eq!(ret.data3, 30);
            }
        }

        task(true);
        mock_reboot();
        // crash produced here...
        // Just-in-time recovery when rebooting
        current().jit_recovery();
        // assert_eq!(syscall_tail(), 3 * crate::arch::ARCH_ALIGN);
        assert_eq!(user_tx_tail(), 2);
        assert_eq!(user_tx_ptr(), 0);
        task(false);
    }

    declare_pm_loop_cnt!(LOOP_CNT_1, 0);
    declare_pm_loop_cnt!(LOOP_CNT_2, 0);

    fn task_for_loop_tx(crash_point: usize) {
        let px = transaction::run_sys(|j, t| {
            let px = PBox::new(42, t);
            px
        });

        nv_for_loop!(LOOP_CNT_1, i, 0 => 10, {
            if i == crash_point {
                transaction::crashed_run(|j| {
                    let x = px.as_mut(j);
                    *x += 1
                });
                forget(px);
                return;
            } else {
                transaction::run(|j| {
                    let x = px.as_mut(j);
                    *x += 1
                });
            }
        });
        let x = unsafe { *px.as_ref_no_journal() };
        os_print!("x is {}", x);
        assert_eq!(x, 42 + 10);
        forget(px);
    }

    fn task_for_loop_tx_2(crash_loop: usize, cp1: usize, cp2: usize) {
        let px = transaction::run_sys(|j, t| {
            let px = PBox::new(42, t);
            px
        });

        crashed_nv_for_loop!(LOOP_CNT_1, i, 0 => 10, crash_loop, cp1, cp2, {
            transaction::run(|j| {
                let x = px.as_mut(j);
                *x += 1
            });
        });
        let x = unsafe { *px.as_ref_no_journal() };
        os_print!("x is {}", x);
        assert_eq!(x, 42 + 10);
        forget(px);
    }

    #[test]
    fn test_for_loop_replay() {
        fn test_crash_point(cp: usize) {
            mock_boot(1);
            // run
            task_for_loop_tx(cp);
            assert_eq!(user_tx_stack_top(), 1);

            mock_reboot();
            current().jit_recovery();
            assert_eq!(user_tx_tail(), 2);
            assert_eq!(user_tx_ptr(), 0);
            assert_eq!(syscall_tail(), 0);
            assert_eq!(user_tx_stack_top(), 0);

            task_for_loop_tx(1000);

            assert_eq!(user_tx_tail(), 2);
            assert_eq!(user_tx_ptr(), 2);
            assert_eq!(syscall_tail(), 0);
            assert_eq!(user_tx_stack_top(), 0);
        }

        for cp in 0..10 {
            test_crash_point(cp);
        }
    }

    #[test]
    fn test_for_loop_replay_2() {
        fn test_crash_point(lp: usize, cp1: usize, cp2: usize) {
            mock_boot(1);
            // run
            task_for_loop_tx_2(lp, cp1, cp2);

            mock_reboot();
            current().jit_recovery();
            assert_eq!(user_tx_ptr(), 0);
            assert_eq!(syscall_tail(), 0);
            assert_eq!(user_tx_stack_top(), 0);

            task_for_loop_tx_2(NO_CRASH, NO_CRASH, NO_CRASH);

            assert_eq!(user_tx_tail(), 2);
            assert_eq!(user_tx_ptr(), 2);
            assert_eq!(syscall_tail(), 0);
            assert_eq!(user_tx_stack_top(), 0);
        }
        for lp in 0..10 {
            for cp1 in 0..4 {
                os_print!("Testing crash loop {}, crash point 1 {}", lp, cp1);
                test_crash_point(lp, cp1, NO_CRASH);
            }

            for cp2 in 0..6 {
                os_print!("Testing crash loop {}, crash point 2 {}", lp, cp2);
                test_crash_point(lp, NO_CRASH, cp2);
            }
        }
    }

    fn task_double_for_loop_tx(crash_point_x: usize, crash_point_y: usize, crash_after: bool) {
        debug_user_tx_cache();
        let px = transaction::run_sys(|j, t| {
            let px = PBox::new(42, t);
            px
        });

        nv_for_loop!(LOOP_CNT_1, i, 0 => 10, {
            nv_for_loop!(LOOP_CNT_2, j, 0 => 10, {
                if i == crash_point_x && j == crash_point_y {
                    transaction::crashed_run(|j| {
                        let x = px.as_mut(j);
                        *x += 1
                    });
                    forget(px);
                    return;
                } else {
                    transaction::run(|j| {
                        let x = px.as_mut(j);
                        *x += 1
                    });
                }
            });
        });
        debug_user_tx_cache();
        if crash_after {
            forget(px);
            return;
        }
        let x = transaction::run(|j| *px.as_ref(j));

        os_print!("x is {}", x);
        assert_eq!(x, 42 + 100);
        forget(px);
    }

    #[test]
    fn test_double_for_loop_replay() {
        fn test_crash_point(cp_x: usize, cp_y: usize, crash_after: bool) {
            mock_boot(1);
            // run
            task_double_for_loop_tx(cp_x, cp_y, crash_after);
            debug_user_tx_cache();
            if !crash_after {
                assert_eq!(user_tx_stack_top(), 2);
            } else {
                assert_eq!(user_tx_stack_top(), 0);
            }
            mock_reboot();
            current().jit_recovery();
            if !crash_after {
                assert_eq!(user_tx_tail(), 3);
            } else {
                assert_eq!(user_tx_tail(), 2);
            }

            assert_eq!(user_tx_ptr(), 0);
            assert_eq!(syscall_tail(), 0);
            assert_eq!(user_tx_stack_top(), 0);

            task_double_for_loop_tx(10000, 10000, false);

            assert_eq!(user_tx_tail(), 3);
            assert_eq!(user_tx_ptr(), 3);
            assert_eq!(syscall_tail(), 0);
            assert_eq!(user_tx_stack_top(), 0);
        }

        for i in 0..10 {
            for j in 0..10 {
                test_crash_point(i, j, false);
            }
        }
        test_crash_point(10000, 10000, true);
    }

    fn mock_boot_seq(cp: usize) {
        if recover::is_first_boot_done() {
            return;
        }
        unsafe {
            recover::get_boot_tx_cache().reset_ptr();
        }
        if cp == 0 {
            return;
        }
        heap::init();
        if cp == 1 {
            return;
        }
        debug_print!("creating idle task...");
        task::create_idle_task();
        if cp == 2 {
            return;
        }
        recover::complete_first_boot();
        if cp == 3 {
            return;
        }
        arch::start_kernel();
    }

    fn mock_recover_and_boot(mut cp: usize) {
        if cp == 0 {
            return;
        }
        increase_generation();
        if cp == 1 {
            return;
        }
        init_boot_tx();
        if cp == 2 {
            return;
        }
        recover::recover();
        if cp == 3 {
            return;
        }
        cp -= 4;
        mock_boot_seq(cp);
    }

    #[test]
    fn test_crash_during_boot() {
        fn run(cp: usize) {
            clean_up();
            mock_recover_and_boot(cp);
            // restart
            os_print!("<-----Reboot------>");
            mock_recover_and_boot(1000);
            let task_cnt = unsafe { get_task_cnt() };
            assert_eq!(task_cnt, 1);
            assert_eq!(boot_tx_ptr(), 1);
            assert_eq!(boot_tx_tail(), 1);
        }
        for cp in 0..10 {
            os_print!("Testing Crash Point {}", cp);
            run(cp);
        }
    }

    fn task_crash_before_syscall_rets_1(
        tx_cp: usize,
        syscall_cp: usize,
    ) -> (Option<PBox<i32>>, Option<PBox<TestStruct>>) {
        transaction::may_crashed_run_sys(tx_cp < 2, |j, t| {
            if tx_cp == 0 {
                syscalls::set_syscall_end_crash_point(syscall_cp);
            }
            let bx = PBox::new(42, t);
            if tx_cp == 0 {
                forget(bx);
                return (None, None);
            }
            let x = bx.as_mut(j);
            *x += 10;
            if tx_cp == 1 {
                syscalls::set_syscall_end_crash_point(syscall_cp);
            }
            let by = PBox::new(
                TestStruct {
                    data1: 1,
                    data2: 2,
                    data3: 3,
                    data4: false,
                },
                t,
            );
            if tx_cp == 1 {
                forget(bx);
                forget(by);
                return (None, None);
            }
            let y = by.as_mut(j);
            y.data1 += 10;
            y.data2 += 10;
            y.data3 += 10;
            y.data4 = !y.data4;
            return (Some(bx), Some(by));
        })
    }

    #[test]
    fn test_crash_before_syscall_ret() {
        fn run(tx_cp: usize, syscall_cp: usize) {
            mock_boot(1);
            task_crash_before_syscall_rets_1(tx_cp, syscall_cp);
            os_print!("<------------ Reboot ------------->");
            mock_reboot();
            current().jit_recovery();
            let (r1, r2) = task_crash_before_syscall_rets_1(1000, 1000);
            let j = unsafe { JournalHandle::new_dummy() };
            let bx = r1.unwrap();
            let by = r2.unwrap();
            let x = bx.as_ref(j);
            let y = by.as_ref(j);
            assert_eq!(*x, 52);
            assert_eq!(y.data1, 11);
            assert_eq!(y.data2, 12);
            assert_eq!(y.data3, 13);
            assert_eq!(y.data4, true);

            assert_eq!(user_tx_tail(), 1);
            assert_eq!(user_tx_ptr(), 1);
            assert_eq!(syscall_tail(), 0);
            assert_eq!(sys_tx_tail(), 0);
            forget(bx);
            forget(by);
        }
        for tx_cp in 0..2 {
            for sys_cp in 0..5 {
                os_print!(
                    "[TX crash point: {}, syscall crash point {}]",
                    tx_cp,
                    sys_cp
                );
                run(tx_cp, sys_cp);
            }
        }
    }

    fn task_crashed_single_lock(cp: usize) {
        let m = transaction::run_sys(|j, t| PArc::new(PMutex::new(32, t), t));

        if cp == 0 {
            forget(m);
            return;
        }

        let guard = m.lock().unwrap();
        if cp == 1 {
            forget(guard);
            forget(m);
            return;
        }

        transaction::may_crashed_run(cp == 2, |j| {
            let data = guard.as_mut(j);
            *data += 10;
        });

        if cp == 2 {
            forget(guard);
            forget(m);
            return;
        }
        // unlock
        drop(guard);

        if cp == 3 {
            forget(m);
            return;
        }
        let guard = m.lock().unwrap();
        transaction::run(|j| {
            let data = guard.as_ref(j);
            assert_eq!(*data, 42);
        });
    }

    #[test]
    fn test_crashed_single_threaded_lock() {
        fn run(cp: usize) {
            mock_boot(1);
            task_crashed_single_lock(cp);
            os_print!("<----- Reboot ------>");
            mock_reboot();
            current().jit_recovery();
            task_crashed_single_lock(10000);
        }

        for cp in 0..5 {
            os_print!("[TX crash point: {}]", cp);
            run(cp);
        }
    }

    fn task_crashed_queue_operations(cp: usize, scp: usize) {
        let (q, b, c) = transaction::may_crashed_run_sys(cp == 0, |j, t| {
            let q = syscalls::sys_queue_create::<i32>(1, t);
            let b = PBox::new(0, t);
            let c = PBox::new(0, t);
            (q, b, c)
        });

        let q = q.unwrap();

        if cp == 0 {
            return;
        }

        let mut data1 = 42;
        if cp == 1 {
            set_syscall_end_crash_point(scp);
        }
        transaction::may_crashed_run_sys(cp == 1, |j, t| {
            sys_queue_send_back(q, &data1, 0xfff, t);
        });
        if cp == 1 {
            forget(b);
            forget(c);
            return;
        }
        mock_task_switch();
        if cp == 2 {
            set_syscall_end_crash_point(scp);
        }
        transaction::may_crashed_run_sys(cp == 2, |j, t| {
            let r = sys_queue_receive(q, 0xfff, t).unwrap();
            task_print!("value received: {}", r);
            let x = b.as_mut(j);
            *x = r;
        });
        if cp == 2 {
            forget(b);
            forget(c);
            return;
        }
        mock_task_switch();
        data1 += 1;
        if cp == 3 {
            set_syscall_end_crash_point(scp);
        }
        transaction::may_crashed_run_sys(cp == 3, |j, t| {
            sys_queue_send_back(q, &data1, 0xfff, t);
        });
        if cp == 3 {
            forget(b);
            forget(c);
            return;
        }

        mock_task_switch();
        if cp == 4 {
            set_syscall_end_crash_point(scp);
        }
        transaction::may_crashed_run_sys(cp == 4, |j, t| {
            let r = sys_queue_receive(q, 0xfff, t).unwrap();
            task_print!("value received: {}", r);
            let x = c.as_mut(j);
            *x = r;
        });
        if cp == 4 {
            forget(b);
            forget(c);
            return;
        }

        let j = unsafe { JournalHandle::new_dummy() };
        let v1 = *b.as_ref(j);
        let v2 = *c.as_ref(j);
        assert_eq!(v1, 42);
        assert_eq!(v2, 43);
        assert_eq!(user_tx_tail(), 2);
        assert_eq!(sys_tx_tail(), 0);
        assert_eq!(syscall_tail(), 0);
        mock_task_switch();
        assert_eq!(user_tx_tail(), 3);
        assert_eq!(sys_tx_tail(), 0);
        assert_eq!(syscall_tail(), 0);
        forget(b);
        forget(c);
    }

    #[test]
    fn test_queue_operations() {
        fn run(cp: usize, scp: usize) {
            mock_boot(2);
            task_crashed_queue_operations(cp, scp);
            os_print!("<----- Reboot ------>");
            mock_reboot();
            if current().get_name() != "Test1" {
                mock_task_switch();
            } else {
                current().jit_recovery();
            }
            task_crashed_queue_operations(1000, 1000);
        }

        for cp in 0..5 {
            for scp in 0..SYSCALL_END_NUM_CRASH_POINT + 1 {
                os_print!("\n[TX crash point: {}, syscall crash point {}]\n", cp, scp);
                run(cp, scp);
            }
        }
    }
}
