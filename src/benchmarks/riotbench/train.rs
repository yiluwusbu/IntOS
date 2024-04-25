use macros::task;

use crate::{
    bench_println,
    benchmarks::{
        benchmark_end, benchmark_start, print_all_task_stats,
        riotbench::libs::{
            data_layout::read_sensor_value,
            decision_tree::{train_decision_tree, DTParams, DTTrainMeta},
            linear_regression::{debug_print_lr_model, train},
            random_u32,
        },
        set_benchmark_done,
    },
    declare_pm_loop_cnt,
    marker::PSafe,
    nv_for_loop, nv_loop,
    syscalls::{self, QueueHandle},
    task::print_all_task_pm_usage,
    task_print,
    user::{
        pbox::{PBox, Ptr},
        pqueue::PQueue,
        pvec::{self, PVec},
        transaction,
    },
    util::benchmark_clock,
};

use super::{libs::linear_regression::LinearRegression, ValueType, TRAIN_DATASET_SZ};

const BENCH_ITER: usize = 1;
const TASK_ITER: usize = 0xffff;
#[cfg(board = "msp430fr5994")]
const TASK_SENSE_PMEM_SZ: usize = 500;
#[cfg(board = "msp430fr5994")]
const TASK_DT_PMEM_SZ: usize = 216;
#[cfg(not(board = "msp430fr5994"))]
const TASK_SENSE_PMEM_SZ: usize = 1024;
#[cfg(not(board = "msp430fr5994"))]
const TASK_DT_PMEM_SZ: usize = 800;

declare_pm_loop_cnt!(ITER, 0);

struct TrainData {
    data: [[ValueType; 4]; TRAIN_DATASET_SZ],
    label: [ValueType; TRAIN_DATASET_SZ],
}

impl TrainData {
    pub fn new() -> Self {
        Self {
            data: [[0; 4]; TRAIN_DATASET_SZ],
            label: [0; TRAIN_DATASET_SZ],
        }
    }
}
unsafe impl PSafe for QueueGroupDT {}
unsafe impl PSafe for QueueGroupLR {}
struct QueueGroupDT {
    rq: QueueHandle<&'static TrainData>,
    sq: QueueHandle<DTParams>,
}

struct QueueGroupLR {
    rq: QueueHandle<&'static TrainData>,
    sq: QueueHandle<LinearRegression<4>>,
}

struct QueueGroup2 {
    rq_dt: QueueHandle<DTParams>,
    rq_lr: QueueHandle<LinearRegression<4>>,
}

fn end_task() {
    // task_print!("Finished!");
    // syscalls::sys_task_delay(5000);
}

#[task]
fn task_sense() {
    benchmark_start();
    let (q_dt, q_lr, mut dt_buf, mut lr_buf) = transaction::run_pure_sys(|t| {
        let q_dt = syscalls::sys_queue_create::<&TrainData>(1, t).unwrap();
        let q_lr = syscalls::sys_queue_create::<&TrainData>(1, t).unwrap();
        let dt_buf = Ptr::new(TrainData::new(), t);
        let lr_buf = Ptr::new(TrainData::new(), t);
        (q_dt, q_lr, dt_buf, lr_buf)
    });

    transaction::run_pure_sys(|t| {
        let rq_dt = syscalls::sys_queue_create::<DTParams>(1, t).unwrap();
        let rq_lr = syscalls::sys_queue_create::<LinearRegression<4>>(4, t).unwrap();

        let qs_dt = PBox::new(
            QueueGroupDT {
                rq: q_dt,
                sq: rq_dt,
            },
            t,
        );
        syscalls::sys_create_task_custom(
            "DT trainer",
            2,
            task_dt_trainer,
            qs_dt,
            TASK_DT_PMEM_SZ,
            t,
        );

        let qs_lr = PBox::new(
            QueueGroupLR {
                rq: q_lr,
                sq: rq_lr,
            },
            t,
        );
        syscalls::sys_create_task("LR trainer", 2, task_lr_trainer, qs_lr, t);

        let qs_consumer = PBox::new(QueueGroup2 { rq_dt, rq_lr }, t);
        syscalls::sys_create_task("model user", 1, task_model_consumer, qs_consumer, t);
    });

    nv_for_loop!(ITER, i, 0=>BENCH_ITER, {
        let dt_buf_pref = dt_buf.as_pref();
        let lr_buf_pref = lr_buf.as_pref();

        let t1 = benchmark_clock();
        transaction::run_sys(|j,t| {
            for i in 0..TRAIN_DATASET_SZ {
                let sensor_data = read_sensor_value(0);
                let mut values: [ValueType; 4] = [0; 4];
                for j in 0..4 {
                    values[j] = sensor_data.values[j] % 2 + 1;
                }
                let label = random_u32(2) as ValueType;
                // let y = random_u32(0xffffffff) as ValueType;
                dt_buf_pref.partial_write(|v| &v.data[i], values, j);
                dt_buf_pref.partial_write(|v| &v.label[i], label, j);
                // lr_buf_pref.partial_write(|v| &v.data[i], values, j);
                // lr_buf_pref.partial_write(|v| &v.label[i], y, j);
            }
        });
        let t2 = benchmark_clock();

        transaction::run_sys(|j,t| {
            for i in 0..TRAIN_DATASET_SZ {
                let sensor_data = read_sensor_value(0);
                let mut values: [ValueType; 4] = [0; 4];
                for j in 0..4 {
                    values[j] = sensor_data.values[j] % 2 + 1;
                }
                let y = random_u32(0xffffffff) as ValueType;
                lr_buf_pref.partial_write(|v| &v.data[i], values, j);
                lr_buf_pref.partial_write(|v| &v.label[i], y, j);
            }
        });

        transaction::run_sys(|j,t| {
            let dt_buf_pref = dt_buf_pref.into_readable();
            let td_dt = dt_buf_pref.as_ref(j);
            let lr_buf_pref = lr_buf_pref.into_readable();
            let td_lr = lr_buf_pref.as_ref(j);
            syscalls::sys_queue_send_back(q_dt, td_dt, 5000, t);
            syscalls::sys_queue_send_back(q_lr, td_lr, 5000, t);
        });
        syscalls::sys_yield();
    });
    benchmark_end();
    syscalls::sys_yield();
    set_benchmark_done();
    // print_all_task_pm_usage();
    print_all_task_stats();
}

declare_pm_loop_cnt!(DT_TRAIN_ITER, 0);
#[task]
fn task_dt_trainer(qs: PBox<QueueGroupDT>) {
    benchmark_start();
    // let (res_vec, pqueue) = transaction::run_pure_sys(|t| {
    //     let res_vec = PVec::new(10, t);
    //     let pqueue = PQueue::new(8, t);
    //     (res_vec, pqueue)
    // });
    let (res_vec, pqueue, train_meta) = transaction::run_pure_sys(|t| {
        let res_vec = PVec::new(10, t);
        let pqueue = PQueue::new(8, t);
        let train_meta = PBox::new(DTTrainMeta::new(), t);
        (res_vec, pqueue, train_meta)
    });
    nv_loop!({
        let dataset = transaction::run_sys(|j, t| {
            let q = qs.as_ref(j).rq;
            let data = syscalls::sys_queue_receive(q, 5000, t).unwrap();
            pqueue.clear(j);
            res_vec.clear(j);
            data
        });

        train_decision_tree(
            &dataset.data,
            &dataset.label,
            &train_meta,
            &pqueue,
            &res_vec,
        );
        // train_decision_tree(&dataset.data , &dataset.label, &pqueue, &res_vec);

        transaction::run_sys(|j, t| {
            let q = qs.as_ref(j).sq;
            let mut params = DTParams::new();
            let mut i = 0;
            for param in res_vec.iter() {
                *params.at_mut(i) = *param;
                i += 1;
            }
            // task_print!("#Nodes: {}", i);
            syscalls::sys_queue_send_back(q, params, 5000, t);
        });
    });
    benchmark_end();
    end_task();
}

declare_pm_loop_cnt!(LR_TRAIN_ITER, 0);

#[task]
fn task_lr_trainer(qs: PBox<QueueGroupLR>) {
    benchmark_start();
    let params = transaction::run_pure_sys(|t| {
        let params = PBox::new(LinearRegression::<4>::new([0; 4]), t);
        params
    });
    nv_loop!({
        // nv_for_loop!(LR_TRAIN_ITER, i, 0=>TASK_ITER, {
        let dataset = transaction::run_sys(|j, t| {
            let q = qs.as_ref(j).rq;
            let data = syscalls::sys_queue_receive(q, 5000, t).unwrap();
            data
        });

        train(&dataset.data, &dataset.label, &params, 1, 15);

        transaction::run_sys(|j, t| {
            let q = qs.as_ref(j).sq;
            let params = params.as_ref(j);
            // debug_print_lr_model(params);
            syscalls::sys_queue_send_back(q, *params, 5000, t);
        });
    });
    benchmark_end();
    end_task();
}

declare_pm_loop_cnt!(COMSUMER_ITER, 0);
#[cfg(not(feature = "riotbench_no_log_opt"))]
#[task]
fn task_model_consumer(qs: PBox<QueueGroup2>) {
    benchmark_start();
    let (mut dt_model, mut lr_model) = transaction::run_pure_sys(|t| {
        let dt = Ptr::new(DTParams::new(), t);
        let lr = Ptr::new(LinearRegression::<4>::new([0; 4]), t);
        (dt, lr)
    });

    nv_loop!({
        // nv_for_loop!(COMSUMER_ITER, i, 0=>TASK_ITER, {
        let dt_pref = dt_model.as_pref();
        let lr_pref = lr_model.as_pref();
        transaction::run_sys(|j, t| {
            let q_dt = qs.as_ref(j).rq_dt;
            let q_lr = qs.as_ref(j).rq_lr;
            let dt_params = syscalls::sys_queue_receive(q_dt, 5000, t).unwrap();
            let lr_params = syscalls::sys_queue_receive(q_lr, 5000, t).unwrap();
            dt_pref.write(dt_params, j);
            lr_pref.write(lr_params, j);
        });
    });
    benchmark_end();
    end_task();
}
#[cfg(feature = "riotbench_no_log_opt")]
#[task]
fn task_model_consumer(qs: PBox<QueueGroup2>) {
    benchmark_start();
    let (mut dt_model, mut lr_model) = transaction::run_pure_sys(|t| {
        let dt = PBox::new(DTParams::new(), t);
        let lr = PBox::new(LinearRegression::<4>::new([0; 4]), t);
        (dt, lr)
    });

    nv_loop!({
        // nv_for_loop!(COMSUMER_ITER, i, 0=>TASK_ITER, {

        transaction::run_sys(|j, t| {
            let dt_mdl = dt_model.as_mut(j);
            let lr_mdl = lr_model.as_mut(j);
            let q_dt = qs.as_ref(j).rq_dt;
            let q_lr = qs.as_ref(j).rq_lr;
            let dt_params = syscalls::sys_queue_receive(q_dt, 5000, t).unwrap();
            let lr_params = syscalls::sys_queue_receive(q_lr, 5000, t).unwrap();
            *dt_mdl = dt_params;
            *lr_mdl = lr_params;
        });
    });
    benchmark_end();
    end_task();
}

pub fn register() {
    crate::task::register_app_no_param_custom("Task Sense", 3, task_sense, TASK_SENSE_PMEM_SZ);
}
