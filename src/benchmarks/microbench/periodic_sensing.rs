use macros::app;

use crate::time;
use crate::{
    bench_dbg_print, bench_println,
    benchmarks::{
        benchmark_end, benchmark_start, is_benchnmark_done, print_all_task_stats,
        set_benchmark_done,
    },
    pmem::JournalHandle,
    syscalls::{self, SyscallToken},
    task::{self, ErrorCode},
    time::Time,
    user::{
        pbox::{PBox, Ptr},
        transaction,
    },
    util::print_pmem_used,
};

struct PhotoBuffer {
    data: [u8; 400],
}
#[cfg(feature = "power_failure")]
const TIMER_PERIOD: Time = 5;

#[cfg(feature = "power_failure")]
#[cfg(board = "msp430fr5994")]
const BENCH_TIME: Time = 100;

#[cfg(not(feature = "power_failure"))]
const TIMER_PERIOD: Time = 5;
#[cfg(not(feature = "power_failure"))]
const BENCH_TIME: Time = 1000;

#[cfg(feature = "power_failure")]
#[cfg(board = "apollo4bp")]
const BENCH_TIME: Time = 9000;

#[cfg(feature = "power_failure")]
#[cfg(board = "qemu")]
const BENCH_TIME: Time = 9000;

const BENCH_ITER: usize = 5;
// const BENCH_ITER: usize = 50;

struct Photo {
    buf: Ptr<PhotoBuffer>,
    id: usize,
}

impl Default for PhotoBuffer {
    fn default() -> Self {
        Self { data: [0; 400] }
    }
}

impl Photo {
    fn new(t: SyscallToken) -> Self {
        let buf = Ptr::new(PhotoBuffer::default(), t);
        Self { buf, id: 0 }
    }
}

fn take_photo(photo_ptr: PBox<Photo>, jh: JournalHandle) {
    bench_dbg_print!("Taking photo...");
    let mut sensor_data = PhotoBuffer { data: [0; 400] };
    for i in 0..20 {
        for j in 0..20 {
            sensor_data.data[i * 20 + j] = 42;
        }
    }

    let photo = photo_ptr.as_mut(jh);
    photo.id += 1;

    let buf = photo.buf.as_pref();
    buf.write(sensor_data, jh);
    if photo.id == BENCH_ITER {
        let end_time = benchmark_end();
        set_benchmark_done();
        print_all_task_stats();
        bench_println!("Wallclock: {}", end_time);
    }
    bench_dbg_print!("Photo is taken!, ID = {}", photo.id);
    // don't drop it...
    core::mem::forget(photo);
}

#[app]
fn task_sense() {
    benchmark_start();

    bench_dbg_print!("Creating photo buffer && timer");
    let v = transaction::run_pure_sys(|t| {
        let photo = Photo::new(t);
        let photo_ptr = PBox::new(photo, t);
        let tmr = syscalls::sys_timer_create(
            "task_photo_taker",
            TIMER_PERIOD,
            true,
            take_photo,
            photo_ptr,
            t,
        );
        match tmr {
            Some(v) => Ok(v),
            _ => Err(ErrorCode::TxExit),
        }
    });

    bench_dbg_print!("Starting Timer...");
    if let Ok(tmr) = v {
        loop {
            let r = transaction::run_pure_sys(|t| match syscalls::sys_start_timer(tmr, 100, t) {
                Ok(_) => Ok(()),
                Err(e) => {
                    if e == time::TimerErr::NoTimerDaemon {
                        bench_println!("No Timer Daemon task...");
                        Err(ErrorCode::TxFatal)
                    } else {
                        bench_println!("Failed to start timer, retrying...");
                        Err(ErrorCode::TxExit)
                    }
                }
            });
            if let Ok(_) = r {
                break;
            } else if let Err(ErrorCode::TxFatal) = v {
                break;
            }
        }
    } else {
        bench_println!("Failed to create timer. Panic!");
        loop {}
    }

    bench_dbg_print!("Timer started...");
    // syscalls::sys_task_delay(BENCH_TIME);
    benchmark_end();
    set_benchmark_done();
    syscalls::sys_task_delay(BENCH_TIME);
}

pub fn register() {
    task::register_app_no_param("sense", 3, task_sense);
}
