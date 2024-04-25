use macros::{app, task};

use crate::{
    bench_dbg_print, bench_println,
    benchmarks::{benchmark_end, benchmark_start, print_all_task_stats, set_benchmark_done},
    declare_pm_loop_cnt, nv_for_loop,
    syscalls::{self, QueueHandle},
    task::{self, task_yield},
    time::Time,
    user::{pbox::PBox, transaction},
};

const MAX_MQ_SZ: usize = 8;
const MAX_MQ_NUM: usize = 4;
struct MessageQueueGroup<T> {
    subscribers: [Option<QueueHandle<T>>; MAX_MQ_NUM],
}

#[derive(Clone, Copy)]
struct Message {
    data: usize,
    timestamp: Time,
}

struct DataBuffer {
    window: [usize; 10],
}

const BENCH_ITER: usize = 150;
declare_pm_loop_cnt!(BENCH_ITER_CNT, 0);
static mut AVG_MAX: (usize, usize) = (0, 0);

fn update_avg(avg: usize) {
    unsafe { AVG_MAX.0 = avg };
}

fn update_max(mx: usize) {
    unsafe { AVG_MAX.1 = mx };
}

fn get_avg_max() -> (usize, usize) {
    unsafe { AVG_MAX }
}

#[app]
fn task_entry() {
    benchmark_start();
    // create tasks && message queues:
    transaction::run_pure_sys(|t| {
        let msgq_1 = syscalls::sys_queue_create::<Message>(MAX_MQ_SZ, t).unwrap();
        let msgq_2 = syscalls::sys_queue_create::<Message>(MAX_MQ_SZ, t).unwrap();
        let mq_box = PBox::new(
            MessageQueueGroup {
                subscribers: [Some(msgq_1), Some(msgq_2), None, None],
            },
            t,
        );
        syscalls::sys_create_task("network_agent", 1, task_network_agent, mq_box, t).unwrap();
        syscalls::sys_create_task("worker_1", 0, task_worker_1, msgq_1, t).unwrap();
        syscalls::sys_create_task("worker_2", 0, task_worker_2, msgq_2, t).unwrap();
    });
    benchmark_end();
    set_benchmark_done();
    syscalls::sys_task_delay(2000);
}

#[task]
fn task_network_agent(mq: PBox<MessageQueueGroup<Message>>) {
    benchmark_start();
    nv_for_loop!(BENCH_ITER_CNT, i, 0 => BENCH_ITER, {
        // #[cfg(not(power_failure))]
        // syscalls::sys_task_delay(10);
        let ts = syscalls::sys_get_time_out_of_tx();
        let data = ts as usize + 42;
        let msg = Message {
            data,
            timestamp: ts,
        };
        transaction::run_sys(|j, t| {
            for i in 0..mq.as_ref(j).subscribers.len() {
                if let Some(q) = mq.as_ref(j).subscribers[i] {
                    syscalls::sys_queue_send_back(q, msg,  5 , t);
                }
            }
        });
    });
    benchmark_end();
    task_yield();
    set_benchmark_done();
    print_all_task_stats();
}

#[task]
fn task_worker_1(q: QueueHandle<Message>) {
    benchmark_start();
    let msg = Message {
        data: 0,
        timestamp: 0,
    };

    let (mut msg_buf, mut data_buf, mut pcnt) = transaction::run_pure_sys(|t| {
        let msg_buf = PBox::new(msg, t);
        let data_buf = PBox::new(DataBuffer { window: [0; 10] }, t);
        let pcnt = PBox::new(0usize, t);
        (msg_buf, data_buf, pcnt)
    });

    loop {
        transaction::run_sys_once(|j, t| {
            let r = syscalls::sys_queue_receive(q, 5000, t);
            match r {
                Ok(data) => {
                    // process data
                    let msg = msg_buf.as_mut(j);
                    let cnt = pcnt.as_mut(j);
                    let data = data_buf.as_mut(j);
                    let tmp = msg.data;
                    let i = *cnt;
                    data.window[i] = tmp;
                    *cnt = (i + 1) % 10;
                    // bench_dbg_print!("i is: {}", i);
                    if i == 9 {
                        let mut sum = 0;
                        for i in 0..10 {
                            sum += data.window[i];
                        }
                        let avg = sum / 10;
                        update_avg(avg);
                        bench_dbg_print!("avg for a window of 10 is: {}", avg);
                    }
                }
                Err(_) => {
                    bench_dbg_print!("Data receiving timed out!");
                }
            }
        });
    }
    benchmark_end();
}

#[task]
fn task_worker_2(q: QueueHandle<Message>) {
    benchmark_start();
    let msg = Message {
        data: 0,
        timestamp: 0,
    };

    let (mut msg_buf, mut data_buf, mut pcnt) = transaction::run_pure_sys(|t| {
        let msg_buf = PBox::new(msg, t);
        let data_buf = PBox::new(DataBuffer { window: [0; 10] }, t);
        let pcnt = PBox::new(0usize, t);
        (msg_buf, data_buf, pcnt)
    });

    loop {
        transaction::run_sys_once(|j, t| {
            let r = syscalls::sys_queue_receive(q, 5000, t);
            match r {
                Ok(data) => {
                    // process data
                    let msg = msg_buf.as_mut(j);
                    let cnt = pcnt.as_mut(j);
                    let data = data_buf.as_mut(j);
                    let tmp = msg.data;
                    // bench_dbg_print!("i is: {}", i);
                    let i = *cnt;
                    data.window[i] = tmp;
                    *cnt = (i + 1) % 10;
                    if i == 9 {
                        let mut max_v = 0;
                        for j in 0..10 {
                            if data.window[j] > max_v {
                                max_v = data.window[j]
                            }
                        }
                        update_max(max_v);
                        bench_dbg_print!("max for a window of 10 is: {}", max_v);
                    }
                }
                Err(_) => {}
            }
        });
    }
    benchmark_end();
}

pub fn register() {
    task::register_app_no_param("task entry", 0, task_entry);
}
