use macros::app;

use crate::debug_print;
use crate::event_group::EventGroupHandle;
use crate::nv_loop;
use crate::os_print;
use crate::pmem::JournalHandle;
use crate::syscalls;
use crate::syscalls::*;
use crate::task::ErrorCode;
use crate::task_print;
use crate::time::{self, Time};
use crate::user::parc::PArc;
use crate::user::pbox::PBox;
use crate::user::pbox::Ptr;
use crate::user::pmutex::PMutex;
use crate::user::pqueue::PQueue;
use crate::user::pthread;
use crate::user::pvec::PVec;
use crate::user::transaction as tx;
use crate::util::*;

fn delay(nticks: Time) {
    sys_task_delay(nticks);
}

fn delay_in_tx(nticks: Time, t: SyscallToken) {
    sys_task_delay_in_tx(nticks, t);
}

struct XX {
    v: i32,
}

#[app]
pub fn lock_safety() {
    let shared_cnt = tx::run_sys(|j, t| {
        let shared_data = PArc::new(PMutex::new(0, t), t);
        shared_data
    });

    let mut rci = tx::run_sys(|j, t| {
        let rci = PArc::new(5, t);
        rci
    });

    let data_ptr = shared_cnt.lock().unwrap();
    let xx = XX { v: 5 };
    tx::run(|j| {
        let value = *rci + xx.v;
        debug_print!("value is {}", value);
    });
}

#[app]
pub fn task_lock_1() {
    os_print!("Hello I'm Task Lock 1");

    let res = tx::run_sys(|j, t| {
        debug_replay_cache();
        let shared_data = PArc::new(PMutex::new(0, t), t);
        if let Err(_) = sys_create_task("task 2", 0, task_lock_2, shared_data.clone(j), t) {
            return Err(ErrorCode::NoSpace);
        }
        Ok(shared_data)
    });
    let shared_cnt = match res {
        Err(_) => {
            os_print!("Error in creating mutex & task");
            loop {}
        }
        Ok(v) => v,
    };
    debug_print!("Running lock grabbing loop");
    for i in 0..5 {
        debug_print!("delaying");
        delay(15);
        debug_print!("locking");
        let data_ptr = shared_cnt.lock().unwrap();
        tx::run_once(|j| {
            let data = data_ptr.as_mut(j);
            *data += 1;
            os_print!("[Task 1] Lock grabbed, cnt = {}", *data);
        });
        debug_print!("Task 1 Unlocked!, i={}", i);
    }
    debug_print!("Task lock 1 done");
}

#[app]
pub fn task_lock_2(shared_data: PArc<PMutex<usize>>) {
    os_print!("Hello I'm Task Lock 2");
    for i in 0..5 {
        delay(15);
        debug_print!("locking");
        let data_ptr = shared_data.lock().unwrap();
        debug_print!("locked");
        tx::run_once(|j| {
            debug_print!("dereferencing data...");
            let data = data_ptr.as_mut(j);
            *data += 1;
            os_print!("[Task 2] Lock grabbed, cnt = {}", *data);
        });
        debug_print!("Task 2 Unlocked!, i = {}", i);
    }
    debug_print!("Task lock 2 done");
}

fn callback(_dummy: usize, _j: JournalHandle) {
    os_print!("Hello from Timer callback");
}

#[app]
pub fn task_timer() {
    task_print!("Hello I'm task timer");
    let mut cnt = 0;
    let data61 = 61;
    let data62 = 62;
    let callback = move |j| {
        let v = data61 + data62;
        task_print!("Hello from Timer callback, Data is {}", v);
    };
    debug_replay_cache();
    let v = tx::run_sys(|j, t| {
        let tmr = sys_timer_create_with_closure("t2", 25, true, callback, t);
        match tmr {
            Some(v) => Ok(v),
            _ => Err(ErrorCode::TxExit),
        }
    });
    // debug_replay_cache();
    if let Ok(tmr) = v {
        task_print!("Timer is created");
        nv_loop!({
            let r = tx::run_sys(|j, t| match sys_start_timer(tmr, 100, t) {
                Ok(_) => Ok(()),
                Err(e) => {
                    if e == time::TimerErr::NoTimerDaemon {
                        os_print!("No Timer Daemon task...");
                        Err(ErrorCode::TxFatal)
                    } else {
                        os_print!("Failed to start timer, retrying...");
                        Err(ErrorCode::TxExit)
                    }
                }
            });
            if let Ok(_) = r {
                break;
            } else if let Err(ErrorCode::TxFatal) = r {
                break;
            }
        });
    } else {
        task_print!("Failed to create timer. Panic!");
        loop {}
    }

    loop {
        delay(50);
        cnt += 1;
        task_print!("Task timer cnt: {}", cnt);
    }
}

fn task_eg_1(eg: EventGroupHandle) {
    debug_syscall_tx_cache();
    tx::run_sys(|j, t| {
        sys_task_delay_in_tx(20, t);
        os_print!("Signaling event 1...");
        sys_event_group_set(eg, 0x1, t);
        debug_syscall_tx_cache();
    });
    loop {}
}

fn task_eg_2(eg: EventGroupHandle) {
    tx::run_sys(|j, t| {
        sys_task_delay_in_tx(40, t);
        os_print!("Signaling event 2...");
        sys_event_group_set(eg, 0x2, t);
    });
    loop {}
}

#[app]
pub fn task_eg() {
    let _ = tx::run_sys(|j, t| {
        let eg = sys_event_group_create(t);
        let eg = match eg {
            Some(v) => v,
            None => return Err(ErrorCode::TxExit),
        };

        let _ = sys_create_task("task_eg_1", 1, task_eg_1, eg, t)?;

        let _ = sys_create_task("task_eg_2", 1, task_eg_2, eg, t)?;
        debug_print!("Calling sys_event_group_set");
        let v = sys_event_group_wait(eg, 0x3, true, true, 200, t);
        match v {
            Err(_) => {
                os_print!("Event waiting timed out");
            }
            Ok(_) => {
                os_print!("Event occured!");
            }
        }
        Ok(())
    });

    loop {
        // delay(100);
    }
}

#[app]
pub fn task_ping() {
    os_print!("Task Ping");
    let buf_q = tx::run_sys(|j, t| {
        let q = sys_queue_create::<usize>(1, t);
        let q = match q {
            Some(v) => v,
            _ => return Err(ErrorCode::TxExit),
        };
        task_print!("Queue created");
        let r = sys_create_task("pong", 1, task_pong, q, t);
        if let Err(e) = r {
            task_print!("Error creating task pong, error: {:?}", e);
            return Err(e);
        }
        task_print!("Task pong created");
        let buf = PVec::try_new([0], t);
        let buf = match buf {
            Ok(v) => v,
            _ => return Err(ErrorCode::TxExit),
        };
        Ok((buf, q))
    });

    let (mut buf, q) = match buf_q {
        Ok((v1, v2)) => (v1, v2),
        _ => {
            task_print!("Error creating q, buf, or task");
            loop {}
        }
    };
    // debug_kernel_tx_journal();
    task_print!("running send loop");

    loop {
        buf = tx::run_sys_once(move |j, t| {
            let res = sys_queue_send_back(q, buf[0], 5000, t);
            match res {
                Err(_) => {
                    os_print!("Error writing queue, retry...");
                }
                Ok(_) => {
                    os_print!("Value written: {}", buf[0]);
                }
            }
            let v = buf.index_mut(0, j);
            *v += 1;
            delay(10);
            buf
        });
    }
}

fn task_pong(q: QueueHandle<usize>) {
    os_print!("Task Pong");

    loop {
        tx::run_sys_once(|j, t| {
            let res = sys_queue_receive(q, 5000, t);
            match res {
                Err(_) => {
                    os_print!("Error reading from queue");
                }
                Ok(v) => {
                    os_print!("Received value: {}", v);
                }
            };
        });
    }
}

#[app]
pub fn task_pbox() {
    let mut bb = tx::run_sys(|j, t| {
        let mut boxed = PBox::new(5, t);
        let v = boxed.as_mut(j);
        let x = 2;
        let z = 3;
        *v += 6;
        syscalls::sys_yield();
        *v += x + z;
        task_print!("The value of v is {} ", v);
        boxed
    });

    tx::run(move |j| {
        let v = bb.as_mut(j);
        task_print!("The value of v is {} ", v);
    });
}

struct X {
    v: u32,
}

impl Drop for X {
    fn drop(&mut self) {
        debug_print!("Dropping x...");
    }
}

#[app]
pub fn task_test_closure() {
    tx::run_sys(|j, t| {
        let mut x = X { v: 1 };
        let mut y = X { v: 2 };
        let _ = pthread::create("x", 1, t, move || {
            x.v += 1 + y.v;
            debug_print!("this is x = {}", x.v);
        });
    });
}

#[app]
pub fn task_test_pvec() {
    let mut pvec = tx::run_sys(|j, t| {
        let v = PVec::new_with([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], t);
        v
    });

    for i in pvec.iter() {
        debug_print!("value is {}", i);
    }

    // tx::run(|j| {
    //     for i in pvec.drain(j) {
    //         debug_print!("value is {}", i);
    //     }
    // });

    loop {}
}

#[app]
pub fn task_test_pqueue() {
    let mut pqueue = tx::run_sys(|j, t| PQueue::new(10, t));

    tx::run(|j| {
        for i in 0..6 {
            pqueue.push_back(i, j);
        }
    });

    tx::run(|j| {
        for i in 0..6 {
            let v = pqueue.pop_front(j).unwrap();
            task_print!("Value is {}", v);
        }
        task_print!("len is {}", pqueue.len(j));
    });
}

#[app]
pub fn task_ptrs() {
    tx::run_sys(|j, t| {
        let mut v = Ptr::new(5, t);
        let (res, mut v) = v.read(|x| *x, j);
        debug_print!("The value of v is {} ", res);
        let mut v = v.write(|x| *x = res + 2, j);
        debug_print!("The value of v is {} ", *v);
        *v += 2;
        debug_print!("The value of v is {} ", *v);
    });
    loop {}
}
