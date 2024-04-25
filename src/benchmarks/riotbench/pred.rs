use macros::task;

use crate::{
    bench_dbg_print, bench_println,
    benchmarks::{
        benchmark_end, benchmark_start, print_all_task_stats,
        riotbench::{
            libs::{self, average::BlockAvg, data_layout::read_sensor_value, decision_tree},
            NUM_SENSORS,
        },
        set_benchmark_done, wall_clock_end,
    },
    declare_pm_loop_cnt, nv_for_loop, os_print,
    pmem::JournalHandle,
    queue::Queue,
    syscalls::{self, QueueHandle},
    task::{print_all_task_pm_usage, register_app_no_param_custom},
    time::Time,
    user::{
        pbox::{PBox, PRef, Ptr},
        transaction,
    },
    util::print_pmem_used,
};

use super::{
    libs::{data_layout::SensorData, linear_regression::LinearRegression},
    ValueType,
};

const BENCH_ITER: usize = 50;
const ITER: usize = BENCH_ITER * NUM_SENSORS;
#[cfg(board = "msp430fr5994")]
const TASK_SENSE_PMEM_SZ: usize = 434;
#[cfg(not(board = "msp430fr5994"))]
const TASK_SENSE_PMEM_SZ: usize = 800;
declare_pm_loop_cnt!(BENCH_ITER_CNT, 0);

#[derive(Debug, Clone, Copy)]
enum AnalysisType {
    Avg,
    MLR,
    DT,
    ErrEst,
}

#[derive(Clone, Copy)]
struct ResultData {
    res_type: AnalysisType,
    opaque: ValueType,
}

struct QueueGroup {
    rq: QueueHandle<SensorData>,
    sq: QueueHandle<ResultData>,
}

struct QueueGroup2 {
    rq: QueueHandle<ResultData>,
    sq: QueueHandle<ResultData>,
}

struct QueueGroup3 {
    rq1: QueueHandle<SensorData>,
    rq2: QueueHandle<ResultData>,
    sq: QueueHandle<ResultData>,
}

#[task]
fn task_sense() {
    let wallclk_begin = benchmark_start();
    let (q_dt, q_lr, q_avg) = transaction::run_pure_sys(|t| {
        bench_dbg_print!("Size of SensorData: {}", core::mem::size_of::<SensorData>());
        let q_dt = syscalls::sys_queue_create(4, t).unwrap();
        let q_lr = syscalls::sys_queue_create(4, t).unwrap();
        let q_avg = syscalls::sys_queue_create(4, t).unwrap();
        (q_dt, q_lr, q_avg)
    });

    transaction::run_sys(|j, t| {
        let q_res = syscalls::sys_queue_create(8, t).unwrap();
        let q_lr_res = syscalls::sys_queue_create(8, t).unwrap();
        let qs_dt = PBox::new(
            QueueGroup {
                rq: q_dt,
                sq: q_res,
            },
            t,
        );
        let qs_lr = PBox::new(
            QueueGroup {
                rq: q_lr,
                sq: q_lr_res,
            },
            t,
        );
        let qs_avg = PBox::new(
            QueueGroup3 {
                rq1: q_avg,
                rq2: q_lr_res,
                sq: q_res,
            },
            t,
        );
        syscalls::sys_create_task("DT", 4, task_decision_tree, qs_dt, t).unwrap();
        syscalls::sys_create_task("LR", 4, task_linear_regression, qs_lr, t);
        syscalls::sys_create_task("AVG", 4, task_avg_error_estimate, qs_avg, t);
        syscalls::sys_create_task("Send", 3, task_send, q_res, t);
    });

    nv_for_loop!(BENCH_ITER_CNT, i, 0 => BENCH_ITER, {
        transaction::run_pure_sys(|t| {
            //bench_println!("Task sense sending datas {}", i);
            bench_dbg_print!("Task sense sending datas {}", i);
            for sid in 0..NUM_SENSORS {
                let sensor_data = read_sensor_value(sid as u8);
                syscalls::sys_queue_send_back(q_dt, sensor_data , 5000, t);
                syscalls::sys_queue_send_back(q_lr, sensor_data , 5000, t);
                syscalls::sys_queue_send_back(q_avg, sensor_data , 5000, t);
            }
        });
    });
    let wallclk_end = benchmark_end();
    syscalls::sys_yield();
    set_benchmark_done();
    print_all_task_stats();
    // #[cfg(not(feature = "power_failure"))]
    // bench_println!("Wall Clock: {}", wallclk_end - wallclk_begin);
    // print_all_task_pm_usage();
}

// declare_pm_loop_cnt!(ITER_CNT_1, 0);

#[task]
fn task_decision_tree(qs: PBox<QueueGroup>) {
    benchmark_start();
    let mut i = 0;
    loop {
        i += 1;
        transaction::run_sys_once(|j, t| {
            bench_dbg_print!("Processing data {} using DT", i);
            let qs = qs.as_ref(j);
            let rq = qs.rq;
            let sq = qs.sq;
            let sensor_data = syscalls::sys_queue_receive(rq, 5000, t).unwrap();
            let dt = match sensor_data.sensor_id {
                0 => decision_tree::get_tree_1(),
                _ => decision_tree::get_tree_2(),
            };
            let r = dt.classify(&sensor_data);
            let result = ResultData {
                res_type: AnalysisType::DT,
                opaque: r as ValueType,
            };
            syscalls::sys_queue_send_back(sq, result, 5000, t);
        });
    }
    benchmark_end();
    bench_dbg_print!("Task DT Finished");
}

// declare_pm_loop_cnt!(ITER_CNT_2, 0);
const MODEL_NUM: usize = 5;
#[link_section = ".pmem"]
static LR_MODELS: [LinearRegression<4>; MODEL_NUM] = [
    LinearRegression::new([1, 2, 3, 4]),
    LinearRegression::new([3, 20, 1, 7]),
    LinearRegression::new([6, 3, 3, 4]),
    LinearRegression::new([5, 6, 3, 4]),
    LinearRegression::new([7, 12, 7, 4]),
];

// #[task]
fn task_linear_regression(qs: PBox<QueueGroup>) {
    benchmark_start();
    let mut i = 0;
    loop {
        i += 1;
        transaction::run_sys_once(|j, t| {
            bench_dbg_print!("Processing data {} using LR", i);
            let qs = qs.as_ref(j);
            let rq = qs.rq;
            let sq = qs.sq;
            let sensor_data = syscalls::sys_queue_receive(rq, 5000, t).unwrap();
            let mut sum = 0;
            for i in 0..MODEL_NUM {
                let lr = &LR_MODELS[i];
                sum += lr.predict(&sensor_data.values);
            }
            let avg = sum / MODEL_NUM as ValueType;
            let result = ResultData {
                res_type: AnalysisType::MLR,
                opaque: avg as ValueType,
            };
            syscalls::sys_queue_send_back(sq, result, 5000, t);
        });
    }
    benchmark_end();
    bench_dbg_print!("Task LR Finished");
    loop {}
}

declare_pm_loop_cnt!(ITER_CNT_3, 0);

const BLOCK_WIN: usize = 5;

#[task]
fn task_avg_error_estimate(qs: PBox<QueueGroup3>) {
    benchmark_start();
    let block_avg = transaction::run_pure_sys(|t| PBox::new(BlockAvg::<BLOCK_WIN>::new(), t));
    let mut i = 0;
    loop {
        i += 1;
        transaction::run_sys_once(|j, t| {
            bench_dbg_print!("Processing data {} using Avg & Err Est", i);
            let qs = qs.as_ref(j);
            let rq = qs.rq1;
            let rq_lr = qs.rq2;
            let sq = qs.sq;
            let sensor_data = syscalls::sys_queue_receive(rq, 5000, t).unwrap();
            let lr_data = syscalls::sys_queue_receive(rq_lr, 5000, t).unwrap();

            // get avg
            let block_avg = block_avg.as_mut(j);
            let avg = block_avg.add(sensor_data.values[3]);
            // err estimate
            let err = lr_data.opaque - avg;
            let result = ResultData {
                res_type: AnalysisType::ErrEst,
                opaque: err as ValueType,
            };
            syscalls::sys_queue_send_back(sq, result, 5000, t);
        });
    }
    benchmark_end();
    bench_dbg_print!("Task AVG Finished");
}

#[cfg(not(feature = "riotbench_no_log_opt"))]
fn send(res: &ResultData, buf: PRef<ResultData>, j: JournalHandle) {
    buf.write(*res, j);
}

#[cfg(feature = "riotbench_no_log_opt")]
fn send(res: &ResultData, buf: &mut ResultData, j: JournalHandle) {
    *buf = *res;
}

// declare_pm_loop_cnt!(ITER_CNT_4, 0);
#[cfg(not(feature = "riotbench_no_log_opt"))]
#[task]
fn task_send(rq: QueueHandle<ResultData>) {
    benchmark_start();
    let mut buffer = transaction::run_pure_sys(|t| {
        Ptr::new(
            ResultData {
                res_type: AnalysisType::Avg,
                opaque: 0,
            },
            t,
        )
    });
    let mut i = 0;
    loop {
        i += 1;
        let buf_ref = buffer.as_pref();
        transaction::run_sys_once(|j, t| {
            let res = syscalls::sys_queue_receive(rq, 5000, t).unwrap();
            // bench_println!("Sending Result #{}", i);
            bench_dbg_print!(
                "Sending Result #{}, Type: {:#?}, Value: {}",
                i,
                res.res_type,
                res.opaque
            );
            send(&res, buf_ref, j);
        });
    }
    benchmark_end();
    // set_benchmark_done();
    // print_all_task_stats();
    // bench_println!("Wall Clock: {}", wallclk_end);
    // print_all_task_pm_usage();
    bench_dbg_print!("Bench Pred Finished");
}

#[cfg(feature = "riotbench_no_log_opt")]
#[task]
fn task_send(rq: QueueHandle<ResultData>) {
    benchmark_start();
    let mut buffer = transaction::run_pure_sys(|t| {
        PBox::new(
            ResultData {
                res_type: AnalysisType::Avg,
                opaque: 0,
            },
            t,
        )
    });
    let mut i = 0;
    loop {
        i += 1;
        transaction::run_sys_once(|j, t| {
            let res = syscalls::sys_queue_receive(rq, 5000, t).unwrap();
            // bench_println!("Sending Result #{}", i);
            bench_dbg_print!(
                "Sending Result #{}, Type: {:#?}, Value: {}",
                i,
                res.res_type,
                res.opaque
            );
            let buf_ref = buffer.as_mut(j);
            send(&res, buf_ref, j);
        });
    }
    benchmark_end();
    // set_benchmark_done();
    // print_all_task_stats();
    // bench_println!("Wall Clock: {}", wallclk_end);
    // print_all_task_pm_usage();
    bench_dbg_print!("Bench Pred Finished");
}

pub fn register() {
    register_app_no_param_custom("task sense", 5, task_sense, TASK_SENSE_PMEM_SZ);
}
