use macros::task;

use crate::{
    bench_dbg_print, bench_println,
    benchmarks::{
        benchmark_end, benchmark_start, print_all_task_stats,
        riotbench::{
            libs::{
                self, approx_distinct_count::ApproxDistinctCount, average::BlockAvg,
                data_layout::read_sensor_value, decision_tree,
            },
            NUM_SENSORS,
        },
        set_benchmark_done,
    },
    declare_pm_loop_cnt, nv_for_loop,
    pmem::JournalHandle,
    queue::Queue,
    syscalls::{self, QueueHandle},
    task::register_app_no_param_custom,
    time::Time,
    user::{
        pbox::{PBox, PRef, Ptr},
        transaction,
    },
    util::print_pmem_used,
};

use super::{
    libs::{
        data_layout::SensorData, filters::KalmanFilter, linear_regression::LinearRegression,
        simple_linear_regression::SimpleRegression,
    },
    ValueType,
};

const BENCH_ITER: usize = 50;
const ITER: usize = BENCH_ITER * NUM_SENSORS;

#[cfg(board = "msp430fr5994")]
const TASK_SENSE_PMEM_SZ: usize = 380;
#[cfg(not(board = "msp430fr5994"))]
const TASK_SENSE_PMEM_SZ: usize = 800;
declare_pm_loop_cnt!(BENCH_ITER_CNT, 0);

#[derive(Debug, Clone, Copy)]
enum AnalysisType {
    Avg,
    LR,
    COUNT,
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

#[task]
fn task_sense() {
    let wallclk_begin = benchmark_start();
    let (q_count, q_filter_lr, q_avg) = transaction::run_pure_sys(|t| {
        let q_count = syscalls::sys_queue_create(4, t).unwrap();
        let q_filter_lr = syscalls::sys_queue_create(4, t).unwrap();
        let q_avg = syscalls::sys_queue_create(4, t).unwrap();
        (q_count, q_filter_lr, q_avg)
    });

    transaction::run_sys(|j, t| {
        let q_res = syscalls::sys_queue_create(8, t).unwrap();
        let qs_count = PBox::new(
            QueueGroup {
                rq: q_count,
                sq: q_res,
            },
            t,
        );
        let qs_filter_lr = PBox::new(
            QueueGroup {
                rq: q_filter_lr,
                sq: q_res,
            },
            t,
        );
        let qs_avg = PBox::new(
            QueueGroup {
                rq: q_avg,
                sq: q_res,
            },
            t,
        );
        syscalls::sys_create_task("COUNT", 4, task_distinct_count, qs_count, t).unwrap();
        syscalls::sys_create_task(
            "FILTER_LR",
            4,
            task_filter_sliding_linear_regression,
            qs_filter_lr,
            t,
        );
        syscalls::sys_create_task("AVG", 4, task_average, qs_avg, t);
        syscalls::sys_create_task("Send", 3, task_send, q_res, t);
    });

    nv_for_loop!(BENCH_ITER_CNT, i, 0 => BENCH_ITER, {
        transaction::run_pure_sys(|t| {
            bench_dbg_print!("Task sense sending datas {}", i);
            for sid in 0..NUM_SENSORS {
                let sensor_data = read_sensor_value(sid as u8);
                syscalls::sys_queue_send_back(q_avg, sensor_data , 5000, t);
                syscalls::sys_queue_send_back(q_filter_lr, sensor_data , 5000, t);
                syscalls::sys_queue_send_back(q_count, sensor_data , 5000, t);
            }
        });
    });
    let wallclk_end = benchmark_end();
    syscalls::sys_yield();
    set_benchmark_done();
    print_all_task_stats();
    // bench_println!("Wall Clock: {}", wallclk_end - wallclk_begin);
    // print_pmem_used();
}

const WIN_SZ: usize = 5;
// declare_pm_loop_cnt!(ITER_CNT_1, 0);

#[task]
fn task_filter_sliding_linear_regression(qs: PBox<QueueGroup>) {
    benchmark_start();
    let (kf, lr) = transaction::run_pure_sys(|t| {
        let kf = PBox::new(KalmanFilter::new(1, 2, 3, 4), t);
        let lr = PBox::new(SimpleRegression::new(true), t);
        (kf, lr)
    });
    let mut i = 0;
    // print_pmem_used();
    loop {
        i += 1;
        transaction::run_sys_once(|j, t| {
            bench_dbg_print!("Processing data {} using KF+LR", i);
            let qs = qs.as_ref(j);
            let rq = qs.rq;
            let sq = qs.sq;
            let mut sensor_data = syscalls::sys_queue_receive(rq, 5000, t).unwrap();
            let kf = kf.as_mut(j);
            let filtered_v = kf.filter(sensor_data.values[0]);
            let lr = lr.as_mut(j);
            lr.add_data(filtered_v, sensor_data.values[1]);
            if lr.size() > WIN_SZ {
                lr.remove_data(filtered_v, sensor_data.values[1]);
            }
            let r = lr.predict(filtered_v);
            let result = ResultData {
                res_type: AnalysisType::LR,
                opaque: r as ValueType,
            };
            syscalls::sys_queue_send_back(sq, result, 5000, t);
        });
    }
    benchmark_end();
    bench_dbg_print!("Task FILTER_LR Finished");
}

// declare_pm_loop_cnt!(ITER_CNT_2, 0);
#[task]
fn task_average(qs: PBox<QueueGroup>) {
    benchmark_start();
    let block_avg = transaction::run_pure_sys(|t| PBox::new(BlockAvg::<WIN_SZ>::new(), t));
    let mut i = 0;
    loop {
        i += 1;
        transaction::run_sys_once(|j, t| {
            bench_dbg_print!("Processing data {} using Avg", i);
            let qs = qs.as_ref(j);
            let rq = qs.rq;
            let sq = qs.sq;
            let sensor_data = syscalls::sys_queue_receive(rq, 5000, t).unwrap();
            // get avg
            let block_avg = block_avg.as_mut(j);
            let avg = block_avg.add(sensor_data.values[2]);
            let result = ResultData {
                res_type: AnalysisType::Avg,
                opaque: avg as ValueType,
            };
            syscalls::sys_queue_send_back(sq, result, 5000, t);
        });
    }
    bench_dbg_print!("Task AVG Finished");
    benchmark_end();
}

// declare_pm_loop_cnt!(ITER_CNT_3, 0);
#[task]
fn task_distinct_count(qs: PBox<QueueGroup>) {
    benchmark_start();
    let boxed_count = transaction::run_pure_sys(|t| PBox::new(ApproxDistinctCount::new(), t));
    let mut i = 0;
    loop {
        i += 1;
        transaction::run_sys_once(|j, t| {
            bench_dbg_print!("Processing data {} using Distin. Count", i);
            let qs = qs.as_ref(j);
            let rq = qs.rq;
            let sq = qs.sq;
            let sensor_data = syscalls::sys_queue_receive(rq, 5000, t).unwrap();
            // count
            let v = sensor_data.values[1];
            let count = boxed_count.as_mut(j);
            let cnt = if count.check(v) == false {
                let count = boxed_count.as_mut(j);
                count.inc_count(v)
            } else {
                count.get_count()
            };
            let result = ResultData {
                res_type: AnalysisType::COUNT,
                opaque: cnt as ValueType,
            };
            syscalls::sys_queue_send_back(sq, result, 5000, t);
        });
    }
    benchmark_end();
    bench_dbg_print!("Task COUNT Finished");
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
    bench_dbg_print!("Bench Stats Finished");
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
    bench_dbg_print!("Bench Stats Finished");
}

pub fn register() {
    register_app_no_param_custom("task sense", 5, task_sense, TASK_SENSE_PMEM_SZ);
}
