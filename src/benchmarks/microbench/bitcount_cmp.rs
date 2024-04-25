use macros::app;

use crate::{
    bench_dbg_print, bench_println,
    benchmarks::{
        benchmark_end, benchmark_reset, benchmark_reset_pm, benchmark_start,
        print_current_task_stats, print_wall_clock_time, set_benchmark_done, wall_clock_begin,
        wall_clock_end,
    },
    declare_const_pm_var, declare_pm_loop_cnt, declare_pm_static, declare_pm_var, nv_for_loop,
    os_print, task,
    user::{pvec::PVec, transaction},
};

static BITS: [u8; 256] = [
    0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4, /* 0   - 15  */
    1, 2, 2, 3, 2, 3, 3, 4, 2, 3, 3, 4, 3, 4, 4, 5, /* 16  - 31  */
    1, 2, 2, 3, 2, 3, 3, 4, 2, 3, 3, 4, 3, 4, 4, 5, /* 32  - 47  */
    2, 3, 3, 4, 3, 4, 4, 5, 3, 4, 4, 5, 4, 5, 5, 6, /* 48  - 63  */
    1, 2, 2, 3, 2, 3, 3, 4, 2, 3, 3, 4, 3, 4, 4, 5, /* 64  - 79  */
    2, 3, 3, 4, 3, 4, 4, 5, 3, 4, 4, 5, 4, 5, 5, 6, /* 80  - 95  */
    2, 3, 3, 4, 3, 4, 4, 5, 3, 4, 4, 5, 4, 5, 5, 6, /* 96  - 111 */
    3, 4, 4, 5, 4, 5, 5, 6, 4, 5, 5, 6, 5, 6, 6, 7, /* 112 - 127 */
    1, 2, 2, 3, 2, 3, 3, 4, 2, 3, 3, 4, 3, 4, 4, 5, /* 128 - 143 */
    2, 3, 3, 4, 3, 4, 4, 5, 3, 4, 4, 5, 4, 5, 5, 6, /* 144 - 159 */
    2, 3, 3, 4, 3, 4, 4, 5, 3, 4, 4, 5, 4, 5, 5, 6, /* 160 - 175 */
    3, 4, 4, 5, 4, 5, 5, 6, 4, 5, 5, 6, 5, 6, 6, 7, /* 176 - 191 */
    2, 3, 3, 4, 3, 4, 4, 5, 3, 4, 4, 5, 4, 5, 5, 6, /* 192 - 207 */
    3, 4, 4, 5, 4, 5, 5, 6, 4, 5, 5, 6, 5, 6, 6, 7, /* 208 - 223 */
    3, 4, 4, 5, 4, 5, 5, 6, 4, 5, 5, 6, 5, 6, 6, 7, /* 224 - 239 */
    4, 5, 5, 6, 5, 6, 6, 7, 5, 6, 6, 7, 6, 7, 7, 8, /* 240 - 255 */
];

// #[inline(always)]
// fn bits(idx: usize) -> u8 {
//     let bits_ptr = BITS.as_ptr();
//     unsafe {
//         let bit_ptr = bits_ptr.add(idx);
//         *bit_ptr
//     }
// }

fn recursive_bitcnt(x: u32) -> usize {
    let idx: usize = (x & 0xFF) as usize;
    let mut cnt: usize = BITS[idx] as usize;
    let x = x >> 8;
    if 0 != x {
        cnt += recursive_bitcnt(x);
    }
    return cnt;
}

fn iter_opt_bitcnt(mut x: u32) -> usize {
    let mut cnt = 0;
    while x != 0 {
        cnt += 1;
        x = x & (x - 1);
    }
    return cnt;
}

fn enum_bitcnt(mut i: u32) -> usize {
    i = ((i & 0xAAAAAAAA) >> 1) + (i & 0x55555555);
    i = ((i & 0xCCCCCCCC) >> 2) + (i & 0x33333333);
    i = ((i & 0xF0F0F0F0) >> 4) + (i & 0x0F0F0F0F);
    i = ((i & 0xFF00FF00) >> 8) + (i & 0x00FF00FF);
    i = ((i & 0xFFFF0000) >> 16) + (i & 0x0000FFFF);
    return i as usize;
}

fn half_byte_table_bitcnt(x: u32) -> usize {
    return (BITS[(x & 0x0000000F) as usize]
        + BITS[((x & 0x000000F0) >> 4) as usize]
        + BITS[((x & 0x00000F00) >> 8) as usize]
        + BITS[((x & 0x0000F000) >> 12) as usize]
        + BITS[((x & 0x000F0000) >> 16) as usize]
        + BITS[((x & 0x00F00000) >> 20) as usize]
        + BITS[((x & 0x0F000000) >> 24) as usize]
        + BITS[((x & 0xF0000000) >> 28) as usize]) as usize;
}

fn byte_table_bitcnt(x: u32) -> usize {
    return (BITS[(x & 0xFF) as usize]
        + BITS[((x >> 8) & 0xFF) as usize]
        + BITS[((x >> 16) & 0xFF) as usize]
        + BITS[((x >> 24) & 0xFF) as usize]) as usize;
}

fn half_byte_recur_table_bitcnt(mut x: u32) -> usize {
    let mut cnt = BITS[(x & 0x0000000F) as usize] as usize;
    x >>= 4;
    if x != 0 {
        cnt += half_byte_recur_table_bitcnt(x);
    }
    return cnt;
}

fn shift_bitcnt(mut x: u32) -> usize {
    let mut n: usize = 0;
    for i in 0..32 {
        n += ((x & 0x1) as usize);
        x >>= 1;
    }
    return n;
}

const SEED: usize = 4;
const ITER: usize = 100;
const BATCH_SZ: usize = 4;

declare_pm_loop_cnt!(FCNT, 0);
declare_pm_loop_cnt!(ITER_CNT, 0);
declare_pm_loop_cnt!(BENCH_ITER_CNT, 0);
const BENCH_ITER: usize = 5;

declare_pm_static!(BCNT0, usize, 0);
declare_pm_static!(BCNT1, usize, 0);
declare_pm_static!(BCNT2, usize, 0);
declare_pm_static!(BCNT3, usize, 0);
declare_pm_static!(BCNT4, usize, 0);
declare_pm_static!(BCNT5, usize, 0);
declare_pm_static!(BCNT6, usize, 0);

fn clean_cnts() {
    transaction::run(|j| unsafe {
        BCNT0.set(0);
        BCNT1.set(0);
        BCNT2.set(0);
        BCNT3.set(0);
        BCNT4.set(0);
        BCNT5.set(0);
        BCNT6.set(0);
    });
}

fn print_cnts() {
    bench_println!(
        "Cnt0 = {},Cnt1 = {},Cnt2 = {},Cnt3 = {},Cnt4 = {},Cnt5 = {},Cnt6 = {},",
        unsafe { BCNT0.as_ref_no_journal() },
        unsafe { BCNT1.as_ref_no_journal() },
        unsafe { BCNT2.as_ref_no_journal() },
        unsafe { BCNT3.as_ref_no_journal() },
        unsafe { BCNT4.as_ref_no_journal() },
        unsafe { BCNT5.as_ref_no_journal() },
        unsafe { BCNT6.as_ref_no_journal() }
    );
}

#[app]
fn task_bitcount() {
    wall_clock_begin();
    benchmark_start();
    nv_for_loop!(BENCH_ITER_CNT, i, 0 => BENCH_ITER, {
        clean_cnts();
        bitcount_runner(i);
        //benchmark_reset_pm();
    });
    benchmark_end();
    set_benchmark_done();
    wall_clock_end();
    print_cnts();
    print_wall_clock_time();
    print_current_task_stats();
}

fn bitcount_runner(iter: usize) {
    nv_for_loop!(FCNT, f, 0 => 7, {
        bench_dbg_print!("running f: {}", f);

        match f {
            0 => {
                nv_for_loop!(ITER_CNT, i, 0=>BATCH_SZ, {
                    transaction::run(|j| {
                        let mut cnt = 0;
                        let mut seed = SEED + (i * 13 * ITER/BATCH_SZ);
                        for _i in 0..(ITER/BATCH_SZ) {
                            cnt += enum_bitcnt(seed as u32);
                            seed += 13;
                        }
                        //*pvec.index_mut(f, j) += cnt;
                        // add_cnt(cnt);
                        *BCNT0.as_mut(j) += cnt;
                    });
                });

            },

            1 => {
                nv_for_loop!(ITER_CNT, i, 0=>BATCH_SZ, {
                    transaction::run(|j| {
                        let mut cnt = 0;
                        let mut seed = SEED + (i * 13 * ITER/BATCH_SZ);
                        for _i in 0..(ITER/BATCH_SZ) {
                            cnt +=  half_byte_recur_table_bitcnt(seed as u32);
                            seed += 13;
                        }
                        //*pvec.index_mut(f, j) += cnt;
                        // add_cnt(cnt);
                        *BCNT1.as_mut(j) += cnt;
                    });
                });
            },

            2 => {
                nv_for_loop!(ITER_CNT, i, 0=>BATCH_SZ, {
                    transaction::run(|j| {
                        let mut cnt = 0;
                        let mut seed = SEED + (i * 13 * ITER/BATCH_SZ);
                        for _i in 0..(ITER/BATCH_SZ) {
                            cnt +=  shift_bitcnt(seed as u32);
                            seed += 13;
                        }
                        //*pvec.index_mut(f, j) += cnt;
                        // add_cnt(cnt);
                        *BCNT2.as_mut(j) += cnt;
                    });
                });
            },

            3 => {
                nv_for_loop!(ITER_CNT, i, 0=>BATCH_SZ, {
                    transaction::run(|j| {
                        let mut cnt = 0;
                        let mut seed = SEED + (i * 13 * ITER/BATCH_SZ);
                        for _i in 0..(ITER/BATCH_SZ) {
                            cnt +=  iter_opt_bitcnt(seed as u32);
                            seed += 13;
                        }
                        //*pvec.index_mut(f, j) += cnt;
                        // add_cnt(cnt);
                        *BCNT3.as_mut(j) += cnt;
                    });
                });
            },

            4 => {
                nv_for_loop!(ITER_CNT, i, 0=>BATCH_SZ, {
                    transaction::run(|j| {
                        let mut cnt = 0;
                        let mut seed = SEED + (i * 13 * ITER/BATCH_SZ);
                        for _i in 0..(ITER/BATCH_SZ) {
                            cnt += recursive_bitcnt(seed as u32);
                            seed += 13;
                        }
                        //*pvec.index_mut(f, j) += cnt;
                        // add_cnt(cnt);
                        *BCNT4.as_mut(j) += cnt;
                    });
                });
            },

            5 => {
                nv_for_loop!(ITER_CNT, i, 0=>BATCH_SZ, {
                    transaction::run(|j| {
                        let mut cnt = 0;
                        let mut seed = SEED + (i * 13 * ITER/BATCH_SZ);
                        for _i in 0..(ITER/BATCH_SZ) {
                            cnt += half_byte_table_bitcnt(seed as u32);
                            seed += 13;
                        }
                        //*pvec.index_mut(f, j) += cnt;
                        // add_cnt(cnt);
                        *BCNT5.as_mut(j) += cnt;
                    });
                });
            },
            6 => {
                nv_for_loop!(ITER_CNT, i, 0=>BATCH_SZ, {
                    transaction::run(|j| {
                        let mut cnt = 0;
                        let mut seed = SEED + (i * 13 * ITER/BATCH_SZ);
                        for _i in 0..(ITER/BATCH_SZ) {
                            cnt += byte_table_bitcnt(seed as u32);
                            seed += 13;
                        }
                        //*pvec.index_mut(f, j) += cnt;
                        // add_cnt(cnt);
                        *BCNT6.as_mut(j) += cnt;
                    });
                });
            }

            _ => {
               os_print!("unimplmented!");
            }
        }

    });
    // bench_println!("Finished!");

    // let mut cnt_this = 0;
    // for c in pvec.iter() {
    //     cnt_this += c;
    // }
    //bench_dbg_print!("iter = {}, bitcnt = {}", iter, cnt_this);
}

pub fn register() {
    task::register_app_no_param("bitcount", 1, task_bitcount);
}
