use macros::app;

use crate::{
    bench_dbg_print, bench_println,
    benchmarks::{
        benchmark_end, benchmark_reset, benchmark_reset_pm, benchmark_start, get_wall_clock_begin,
        get_wall_clock_end, print_current_task_stats, set_benchmark_done, wall_clock_begin,
        wall_clock_end,
    },
    debug_print, declare_pm_loop_cnt, nv_for_loop,
    pmem::JournalHandle,
    syscalls::{sys_get_time, SyscallToken},
    task,
    user::{
        pbox::{PRef, Ptr},
        transaction,
    },
    util::debug_user_tx_cache,
};

// Newton method for square root
fn sqrt16(x: u32) -> u16 {
    let mut hi: u16 = 0xffff;
    let mut lo: u16 = 0;
    let mut mid: u16 = ((hi as u32 + lo as u32) >> 1) as u16;
    let mut s: u32 = 0;
    while s != x && hi - lo > 1 {
        mid = ((hi as u32 + lo as u32) >> 1) as u16;
        s = mid as u32 * mid as u32;
        if s < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    return mid;
}

const ACCEL_WINDOW_SIZE: usize = 3;
const MODEL_SIZE: usize = 16;
const SAMPLE_NOISE_FLOOR: u8 = 10;

#[derive(Clone, Copy)]
struct AccelReading {
    x: u8,
    y: u8,
    z: u8,
}

impl AccelReading {
    pub const fn new() -> Self {
        Self { x: 0, y: 0, z: 0 }
    }
}

impl core::fmt::Display for AccelReading {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "x: {}, y: {}, z: {}", self.x, self.y, self.z)
    }
}

struct AccelWindow {
    win: [AccelReading; ACCEL_WINDOW_SIZE],
}

impl AccelWindow {
    pub const fn new() -> Self {
        Self {
            win: [AccelReading::new(); ACCEL_WINDOW_SIZE],
        }
    }

    pub fn print(&self) {
        // debug_print!("Win 1: {},  Win 2: {}, Win 3: {}", self.win[0], self.win[1], self.win[2]);
    }
}

#[derive(Clone, Copy)]
struct Feature {
    mean_mag: u32,
    stddev_mag: u32,
}

impl Feature {
    pub const fn new() -> Self {
        Self {
            mean_mag: 0,
            stddev_mag: 0,
        }
    }
}

struct Model {
    stationary_features: [Feature; MODEL_SIZE],
    moving_features: [Feature; MODEL_SIZE],
}

enum RunMode {
    ModeIdle = 0,
    ModeTrainStationary = 1,
    ModeTrainMoving = 2,
    ModeRecognize = 3,
}

enum Class {
    ClassMoving,
    ClassStationary,
}

struct Stats {
    total_cnt: usize,
    moving_cnt: usize,
    stationary_cnt: usize,
}

impl Stats {
    pub fn new() -> Self {
        Self {
            total_cnt: 0,
            moving_cnt: 0,
            stationary_cnt: 0,
        }
    }
}

fn abs<T>(x: T, y: T) -> T
where
    T: core::ops::Sub<Output = T> + PartialOrd,
{
    if x > y {
        x - y
    } else {
        y - x
    }
}

static mut CNT: usize = 0;

fn accel_gen_sample(t: SyscallToken) -> AccelReading {
    let cnt = unsafe {
        CNT += 1;
        CNT
    };
    let seed = sys_get_time(t) as usize + cnt;
    let x = ((seed * 17) % 85) as u8;
    let y = ((seed * 17 * 17) % 85) as u8;
    let z = ((seed * 17 * 17 * 17) % 85) as u8;
    AccelReading { x, y, z }
}

fn acquire_window(windows: &mut AccelWindow, t: SyscallToken) {
    for i in 0..ACCEL_WINDOW_SIZE {
        let sample = accel_gen_sample(t);
        windows.win[i] = sample;
    }
}

fn transform(window: &mut AccelWindow, j: JournalHandle) {
    for i in 0..ACCEL_WINDOW_SIZE {
        let sample = &mut window.win[i];
        if sample.x < SAMPLE_NOISE_FLOOR
            || sample.y < SAMPLE_NOISE_FLOOR
            || sample.z < SAMPLE_NOISE_FLOOR
        {
            sample.x = if sample.x > SAMPLE_NOISE_FLOOR {
                sample.x
            } else {
                0
            };
            sample.y = if sample.y > SAMPLE_NOISE_FLOOR {
                sample.y
            } else {
                0
            };
            sample.z = if sample.z > SAMPLE_NOISE_FLOOR {
                sample.z
            } else {
                0
            };
        }
    }
}

fn featurize(accel_win: &AccelWindow) -> Feature {
    let mut mean = AccelReading { x: 0, y: 0, z: 0 };
    let mut stddev = AccelReading { x: 0, y: 0, z: 0 };

    for i in 0..ACCEL_WINDOW_SIZE {
        mean.x += accel_win.win[i].x;
        mean.y += accel_win.win[i].y;
        mean.z += accel_win.win[i].z;
    }

    mean.x >>= 2;
    mean.y >>= 2;
    mean.z >>= 2;

    for i in 0..ACCEL_WINDOW_SIZE {
        stddev.x += abs(accel_win.win[i].x, mean.x);
        stddev.y += abs(accel_win.win[i].y, mean.y);
        stddev.z += abs(accel_win.win[i].z, mean.z);
    }

    stddev.x >>= 2;
    stddev.y >>= 2;
    stddev.z >>= 2;

    let mean_mag = (mean.x as u32) * (mean.x as u32)
        + (mean.y as u32) * (mean.y as u32)
        + (mean.z as u32) * (mean.z as u32);

    let stddev_mag = (stddev.x as u32) * (stddev.x as u32)
        + (stddev.y as u32) * (stddev.y as u32)
        + (stddev.z as u32) * (stddev.z as u32);

    Feature {
        mean_mag,
        stddev_mag,
    }
    // Feature { mean_mag: sqrt16(mean_mag) as u32, stddev_mag: sqrt16(stddev_mag) as u32 }
}

fn classify(feature: &Feature, model: &Model) -> Class {
    let mut move_less_error = 0;
    let mut stat_less_error = 0;

    for i in 0..MODEL_SIZE {
        let model_feature = &model.stationary_features[i];

        let stat_mean_err = abs(model_feature.mean_mag, feature.mean_mag);
        let stat_std_err = abs(model_feature.stddev_mag, feature.stddev_mag);

        let model_feature = &model.moving_features[i];

        let move_mean_err = abs(model_feature.mean_mag, feature.mean_mag);
        let move_std_err = abs(model_feature.stddev_mag, feature.stddev_mag);

        if move_mean_err < stat_mean_err {
            move_less_error += 1;
        } else {
            stat_less_error += 1;
        }

        if move_std_err < stat_std_err {
            move_less_error += 1;
        } else {
            stat_less_error += 1;
        }
    }

    let class = if move_less_error > stat_less_error {
        Class::ClassMoving
    } else {
        Class::ClassStationary
    };
    class
}

declare_pm_loop_cnt!(ITER_TRAIN, 0);

fn train(model: PRef<Model>, which: Class) {
    nv_for_loop!(ITER_TRAIN, i, 0 => MODEL_SIZE , {
        transaction::run_sys(|j, t| {
            let mut windows = AccelWindow::new();
            acquire_window(&mut windows, t);
            transform(&mut windows, j);
            let feature = featurize(&windows);
            match which {
                Class::ClassMoving => {
                    model.partial_write(|m| &m.moving_features[i], feature, j);
                },

                Class::ClassStationary => {
                    model.partial_write(|m| &m.stationary_features[i], feature, j);
                }
            }
        });

    });
}

const SAMPLES_TO_COLLECT: usize = 64;

declare_pm_loop_cnt!(SAMPLE_CNT, 0);

fn recognize(model: PRef<Model>, stats: &mut Ptr<Stats>) {
    let model_ro = model.into_readable();
    nv_for_loop!(SAMPLE_CNT, _i, 0 => SAMPLES_TO_COLLECT, {
        let stats = stats.as_pref();
        transaction::run_sys(|j, t| {
            let mdl_ref = model_ro.as_ref(j);
            let mut window = AccelWindow::new();
            acquire_window(&mut window, t);
            window.print();
            transform(&mut window, j);
            let feature = featurize(&window);
            let class = classify(&feature, mdl_ref);
            let mut stats_rw = stats.into_readable();
            let stats_mut_ref = stats_rw.as_mut(j);
            record_stats(stats_mut_ref, class);
        });
    });
}

fn record_stats(stats: &mut Stats, class: Class) {
    stats.total_cnt += 1;
    match class {
        Class::ClassMoving => {
            stats.moving_cnt += 1;
        }
        Class::ClassStationary => {
            stats.stationary_cnt += 1;
        }
    };
}

fn print_stats(stats: &Ptr<Stats>) {
    let s = unsafe { stats.as_ref() };
    bench_dbg_print!(
        "Total cnt: {}, Moving cnt: {}, Stationary cnt: {}",
        s.total_cnt,
        s.moving_cnt,
        s.stationary_cnt
    );
}

fn select_mode(i: usize) -> RunMode {
    match i {
        0 => RunMode::ModeTrainStationary,
        1 => RunMode::ModeTrainMoving,
        2 => RunMode::ModeRecognize,
        _ => RunMode::ModeIdle,
    }
}

declare_pm_loop_cnt!(STAGE, 0);
declare_pm_loop_cnt!(BENCH_ITER_CNT, 0);

const BENCH_ITER: usize = 100;

#[app]
fn task_ar() {
    wall_clock_begin();
    benchmark_start();
    // debug_user_tx_cache();
    nv_for_loop!(BENCH_ITER_CNT, i, 0 => BENCH_ITER, {
        bench_dbg_print!("running iter: {}", i);
        run_ar();
        benchmark_reset_pm();
    });

    // debug_user_tx_cache();
    benchmark_end();

    set_benchmark_done();
    wall_clock_end();
    bench_println!(
        "Wall clock cycles: {}",
        get_wall_clock_end() - get_wall_clock_begin()
    );
    print_current_task_stats();
}

fn run_ar() {
    let (mut model_p, mut stats) = transaction::run_pure_sys(|t| {
        let mdl = Ptr::new(
            Model {
                stationary_features: [Feature::new(); MODEL_SIZE],
                moving_features: [Feature::new(); MODEL_SIZE],
            },
            t,
        );

        let stats = Ptr::new(Stats::new(), t);
        (mdl, stats)
    });

    nv_for_loop!(STAGE, cnt, 0 => 4, {
        let mode = select_mode(cnt);
        let model = model_p.as_pref();
        match mode {
            RunMode::ModeTrainStationary => {
                train(model, Class::ClassStationary);
            },
            RunMode::ModeTrainMoving => {
                train(model, Class::ClassMoving);
            },
            RunMode::ModeRecognize => {
                recognize(model, &mut stats);
            },
            RunMode::ModeIdle => {

            }
        }
    });

    print_stats(&stats);
}

pub fn register() {
    task::register_app_no_param_custom("AR", 1, task_ar, 300);
}
