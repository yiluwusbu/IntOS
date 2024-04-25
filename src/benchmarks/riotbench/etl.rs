use macros::task;

use crate::{
    bench_dbg_print, bench_println,
    benchmarks::{
        benchmark_end, benchmark_start,
        microbench::kvstore::SharedKVStore,
        print_all_task_stats,
        riotbench::{libs::data_layout::read_sensor_value, INVALID_VALUE, NUM_SENSORS},
        set_benchmark_done,
    },
    declare_pm_loop_cnt, nv_for_loop,
    syscalls::{self, QueueHandle},
    task::{register_app_custom, register_app_no_param, register_app_no_param_custom},
    time::Time,
    user::{
        pbox::{PBox, Ptr},
        transaction,
    },
    util::{print_all_task_pmem_used, print_pmem_used},
};

use super::{
    libs::{
        annotation::Annotation,
        data_layout::SensorData,
        filters::{BloomFilter, KalmanFilter, RangeFilter},
        interpolation::Interpolation,
    },
    ValueType, DEFAULT_Q_LENGTH,
};

const BENCH_ITER: usize = 50;
#[cfg(board = "msp430fr5994")]
const TASK_STORE_PMEM_SZ: usize = 354;
#[cfg(board = "apollo4bp")]
const TASK_STORE_PMEM_SZ: usize = 2000;
#[cfg(board = "qemu")]
const TASK_STORE_PMEM_SZ: usize = 800;
const ITER: usize = BENCH_ITER * NUM_SENSORS;

declare_pm_loop_cnt!(BENCH_ITER_CNT, 0);

#[task]
fn task_sense() {
    let wallclk_begin = benchmark_start();
    let q = transaction::run_sys(|j, t| {
        let q = syscalls::sys_queue_create(DEFAULT_Q_LENGTH, t).unwrap();
        syscalls::sys_create_task("task filter", 4, task_filter, q, t).unwrap();
        q
    });

    nv_for_loop!(BENCH_ITER_CNT, i, 0 => BENCH_ITER, {
        bench_dbg_print!("Task sense sending datas {}", i);
        transaction::run_sys(|j,t| {
            for sid in 0..NUM_SENSORS {
                let sensor_data = read_sensor_value(sid as u8);
                syscalls::sys_queue_send_back(q, sensor_data, 5000, t);
            }
        });
    });
    let wallclk_end = benchmark_end();
    syscalls::sys_yield();
    set_benchmark_done();
    print_all_task_stats();
    // print_all_task_pmem_used();
    // bench_println!("Wall Clock: {}", wallclk_end - wallclk_begin);
}

#[link_section = ".pmem"]
static BLOOM_FILTER_1: BloomFilter<8> =
    BloomFilter::new_with([true, false, true, false, true, false, false, false]);
#[link_section = ".pmem"]
static BLOOM_FILTER_2: BloomFilter<8> =
    BloomFilter::new_with([false, false, true, false, false, false, false, false]);
#[link_section = ".pmem"]
static BLOOM_FILTER_3: BloomFilter<8> =
    BloomFilter::new_with([true, false, true, false, true, true, false, false]);

fn multiple_bloom_filter_check(v: ValueType) -> bool {
    let filters = [&BLOOM_FILTER_1, &BLOOM_FILTER_2, &BLOOM_FILTER_3];
    for bf in filters {
        if bf.might_contain(v) {
            return true;
        }
    }
    false
}

// declare_pm_loop_cnt!(ITER_CNT_1, 0);

#[task]
fn task_filter(rq: QueueHandle<SensorData>) {
    benchmark_start();
    let (q, rf) = transaction::run_sys(|j, t| {
        let q = syscalls::sys_queue_create(DEFAULT_Q_LENGTH, t).unwrap();
        let r_filter = PBox::new(RangeFilter::new(0, 120), t);
        syscalls::sys_create_task("task interp", 3, task_interpolate, q, t).unwrap();
        (q, r_filter)
    });
    let mut i = 0;
    loop {
        bench_dbg_print!("Task filter receiving data {}", i);
        i += 1;
        transaction::run_sys_once(|j, t| {
            let rf = rf.as_ref(j);
            let mut data = syscalls::sys_queue_receive(rq, 5000, t).unwrap();
            // bench_dbg_print!("Data is {:#?}", data);
            for idx in 0..data.value_cnt {
                let idx = idx as usize;
                let val = data.values[idx];
                let valid = rf.check(val);
                if valid {
                    let res = multiple_bloom_filter_check(val);
                    if res == false {
                        data.values[idx] = 0;
                    }
                } else {
                    data.values[idx] = INVALID_VALUE;
                }
            }
            // bench_dbg_print!("Filtered Data is {:#?}", data);
            syscalls::sys_queue_send_back(q, data, 5000, t);
        });
    }
    benchmark_end();
}

// declare_pm_loop_cnt!(ITER_CNT_2, 0);

#[task]
fn task_interpolate(rq: QueueHandle<SensorData>) {
    benchmark_start();
    let (q, ip1, ip2) = transaction::run_sys(|j, t| {
        let q = syscalls::sys_queue_create(DEFAULT_Q_LENGTH, t).unwrap();
        let ip1 = PBox::new(Interpolation::<4>::new(), t);
        let ip2 = PBox::new(Interpolation::<4>::new(), t);
        syscalls::sys_create_task("task anno", 2, task_annotate_join, q, t).unwrap();
        (q, ip1, ip2)
    });
    let ips = [ip1, ip2];
    let mut i = 0;
    loop {
        bench_dbg_print!("Task ip receiving data {}", i);
        i += 1;
        transaction::run_sys_once(|j, t| {
            let mut data = syscalls::sys_queue_receive(rq, 5000, t).unwrap();
            // bench_dbg_print!("Data is {:#?}", data);
            let mut need_interpolate = false;
            let measure = data.values[0];
            if measure == 0 || measure == INVALID_VALUE {
                need_interpolate = true;
            }
            let ip = &ips[data.sensor_id as usize];

            if need_interpolate {
                let ip = ip.as_ref(j);
                data.values[0] = ip.interpolate();
            } else {
                let ip = ip.as_mut(j);
                ip.insert(measure);
            }
            syscalls::sys_queue_send_back(q, data, 5000, t);
        });
    }
    benchmark_end();
}

// declare_pm_loop_cnt!(ITER_CNT_3, 0);

#[link_section = ".pmem"]
static ANNO_1: Annotation = Annotation::new("0", "0");
#[link_section = ".pmem"]
static ANNO_2: Annotation = Annotation::new("1", "1");

#[cfg(not(feature = "riotbench_no_log_opt"))]
#[task]
fn task_annotate_join(rq: QueueHandle<SensorData>) {
    benchmark_start();
    let (q, mut s2d) = transaction::run_sys(|j, t| {
        let q = syscalls::sys_queue_create(DEFAULT_Q_LENGTH, t).unwrap();
        let sensor_2_data = Ptr::new(SensorData::new(), t);
        syscalls::sys_create_task_custom("task store", 1, task_store, q, TASK_STORE_PMEM_SZ, t)
            .unwrap();
        (q, sensor_2_data)
    });

    let annos = [&ANNO_1, &ANNO_2];
    let mut i = 0;
    loop {
        bench_dbg_print!("Task anno receiving data {}", i);
        i += 1;
        let s2d_pref = s2d.as_pref();
        transaction::run_sys_once(|j, t| {
            let mut data = syscalls::sys_queue_receive(rq, 5000, t).unwrap();
            let key = if data.sensor_id == 0 { "0" } else { "1" };
            for i in 0..2 {
                annos[i].annotate(key, &mut data.annotation);
            }
            if data.sensor_id == 1 {
                s2d_pref.write(data, j);
            } else {
                // join
                let (data2, _) = s2d_pref.read(|v| v.values, j);
                data.values[2] = data2[2];
                data.values[3] = data2[3];
                syscalls::sys_queue_send_back(q, data, 5000, t);
            }
        });
    }
    benchmark_end();
}

#[cfg(feature = "riotbench_no_log_opt")]
#[task]
fn task_annotate_join(rq: QueueHandle<SensorData>) {
    benchmark_start();
    let (q, mut s2d) = transaction::run_sys(|j, t| {
        let q = syscalls::sys_queue_create(DEFAULT_Q_LENGTH, t).unwrap();
        let sensor_2_data = PBox::new(SensorData::new(), t);
        syscalls::sys_create_task_custom("task store", 1, task_store, q, TASK_STORE_PMEM_SZ, t)
            .unwrap();
        (q, sensor_2_data)
    });

    let annos = [&ANNO_1, &ANNO_2];
    let mut i = 0;
    loop {
        bench_dbg_print!("Task anno receiving data {}", i);
        i += 1;
        transaction::run_sys_once(|j, t| {
            let mut data = syscalls::sys_queue_receive(rq, 5000, t).unwrap();
            let key = if data.sensor_id == 0 { "0" } else { "1" };
            for i in 0..2 {
                annos[i].annotate(key, &mut data.annotation);
            }
            if data.sensor_id == 1 {
                let s2d_data = s2d.as_mut(j);
                *s2d_data = data;
            } else {
                // join
                let s2d_data = s2d.as_ref(j);
                data.values[2] = s2d_data.values[2];
                data.values[3] = s2d_data.values[3];
                syscalls::sys_queue_send_back(q, data, 5000, t);
            }
        });
    }
    benchmark_end();
}
// declare_pm_loop_cnt!(ITER_CNT_4, 0);

#[task]
fn task_store(rq: QueueHandle<SensorData>) {
    let wallclk_begin = benchmark_start();
    let kv = transaction::run_pure_sys(|t| SharedKVStore::<Time, ValueType>::new(t));
    let mut i = 0;
    loop {
        i += 1;
        kv.lock_kv(|m| {
            transaction::run_sys_once(|j, t| {
                bench_dbg_print!("Task kv receiving data {}", i);
                let data = syscalls::sys_queue_receive(rq, 5000, t).unwrap();
                bench_dbg_print!("Inserting key = {}, value = {}", data.ts, data.values[0]);
                m.insert(data.ts, data.values[0], j, t);
            })
        });
    }

    let wallclk_end = benchmark_end();
}

pub fn register() {
    register_app_no_param("task sense", 5, task_sense);
}
