use macros::{app, task};

use crate::{
    bench_dbg_print, bench_println,
    benchmarks::{benchmark_end, benchmark_start, print_all_task_stats, set_benchmark_done},
    declare_pm_loop_cnt,
    event_group::{EventBits, EventGroupHandle},
    nv_for_loop,
    pmem::JournalHandle,
    syscalls::{self, sys_yield, SyscallToken},
    task::{self},
    time::Time,
    user::{
        pbox::{PRef, Ptr},
        transaction,
    },
};

const EVENT_1_FLAG: EventBits = 0x1;
const EVENT_2_FLAG: EventBits = 0x2;

#[cfg(not(feature = "power_failure"))]
const WAIT_TIME: Time = 100;
#[cfg(not(feature = "power_failure"))]
const EVENT_PERIOD: Time = 25;
#[cfg(feature = "power_failure")]
const WAIT_TIME: Time = 1000;
#[cfg(feature = "power_failure")]
const EVENT_PERIOD: Time = 50;

const DATA_BUF_SZ: usize = 200;
struct DataBuffer {
    data: [u8; DATA_BUF_SZ],
}

fn collect_sensor_1_data(db: PRef<DataBuffer>, j: JournalHandle, t: SyscallToken) {
    let ts = syscalls::sys_get_time(t);
    let ts = (ts % 128) as u8;

    let mut buffer = DataBuffer {
        data: [0; DATA_BUF_SZ],
    };
    for i in 0..DATA_BUF_SZ {
        buffer.data[i] = 42 + ts;
    }
    // syscalls::sys_task_delay(5);
    let _ = db.write(buffer, j);
}

fn collect_sensor_2_data(db: PRef<DataBuffer>, j: JournalHandle, t: SyscallToken) {
    let ts = syscalls::sys_get_time(t);
    let ts = (ts % 128) as u8;
    let mut buffer = DataBuffer {
        data: [0; DATA_BUF_SZ],
    };
    for i in 0..DATA_BUF_SZ {
        buffer.data[i] = 24 + ts;
    }
    // syscalls::sys_task_delay(5);
    let _ = db.write(buffer, j);
}

#[task]
fn task_actor_1(event_group: EventGroupHandle) {
    benchmark_start();
    let mut db = transaction::run_pure_sys(|t| {
        let db = Ptr::new(
            DataBuffer {
                data: [0; DATA_BUF_SZ],
            },
            t,
        );
        db
    });

    loop {
        let db_ref = db.as_pref();
        transaction::run_sys_once(|j, t| {
            let r =
                syscalls::sys_event_group_wait(event_group, EVENT_1_FLAG, true, true, WAIT_TIME, t);
            match r {
                Ok(_) => {
                    bench_dbg_print!("Event 1 occurred, collecting data!");
                    collect_sensor_1_data(db_ref, j, t);
                }
                Err(_) => {
                    bench_dbg_print!("Event 2 didn't occur, re-entering listening state!");
                }
            };
        });
    }
    benchmark_end();
}

#[task]
fn task_actor_2(event_group: EventGroupHandle) {
    benchmark_start();
    let mut db = transaction::run_pure_sys(|t| {
        let db = Ptr::new(
            DataBuffer {
                data: [0; DATA_BUF_SZ],
            },
            t,
        );
        db
    });

    loop {
        let db_ref = db.as_pref();
        transaction::run_sys_once(|j, t| {
            let r =
                syscalls::sys_event_group_wait(event_group, EVENT_2_FLAG, true, true, WAIT_TIME, t);
            match r {
                Ok(_) => {
                    bench_dbg_print!("Event 2 occurred, collecting data!");
                    collect_sensor_2_data(db_ref, j, t);
                }
                Err(_) => {
                    bench_dbg_print!("Event 2 didn't occur, re-entering listening state!");
                }
            };
        });
    }
    benchmark_end();
}

const EVENT_COUNT: usize = 100;
declare_pm_loop_cnt!(BENCH_ITER_CNT, 0);
#[app]
fn monitor_task() {
    let wall_clk_start = benchmark_start();
    let event_grp = transaction::run_pure_sys(|t| {
        let event_group = syscalls::sys_event_group_create(t).unwrap();
        syscalls::sys_create_task("actor1", 0, task_actor_1, event_group, t).unwrap();
        syscalls::sys_create_task("actor2", 0, task_actor_2, event_group, t).unwrap();
        event_group
    });
    syscalls::sys_yield();
    nv_for_loop!(BENCH_ITER_CNT,  i,  0 => EVENT_COUNT, {
        transaction::run_pure_sys(|t| {
            bench_dbg_print!("Event has occured!");
            syscalls::sys_event_group_set(event_grp, EVENT_1_FLAG | EVENT_2_FLAG, t);
        });
    });

    syscalls::sys_yield();
    let wall_clk_end = benchmark_end();
    set_benchmark_done();
    bench_println!("Wall clock cycles: {}", wall_clk_end - wall_clk_start);
    print_all_task_stats();
}

pub fn register() {
    task::register_app_no_param("monitor", 1, monitor_task);
}
