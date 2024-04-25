use macros::app;

use crate::{
    bench_dbg_print, bench_println,
    benchmarks::{
        benchmark_end, benchmark_reset, benchmark_reset_pm, benchmark_start,
        print_current_task_stats, print_wall_clock_time, set_benchmark_done, wall_clock_begin,
        wall_clock_end,
    },
    declare_pm_loop_cnt, nv_for_loop, task,
    user::{pbox::PBox, transaction},
    util::debug_user_tx_cache,
};

#[cfg(target_arch = "arm")]
type Numeric = i32;
#[cfg(not(target_arch = "arm"))]
type Numeric = i16;

pub struct Tensor2D<const H: usize, const W: usize> {
    tensor: [[Numeric; W]; H],
}

impl<const H: usize, const W: usize> Tensor2D<H, W> {
    pub const fn new(tensor: [[Numeric; W]; H]) -> Self {
        Self { tensor }
    }

    #[inline(always)]
    pub fn at(&self, rol: usize, col: usize) -> &Numeric {
        &self.tensor[rol][col]
    }

    #[inline(always)]
    pub fn mut_at(&mut self, rol: usize, col: usize) -> &mut Numeric {
        &mut self.tensor[rol][col]
    }
}

pub struct Tensor1D<const W: usize> {
    tensor: [Numeric; W],
}

impl<const W: usize> Tensor1D<W> {
    pub const fn new(tensor: [Numeric; W]) -> Self {
        Self { tensor }
    }

    #[inline(always)]
    pub fn at(&self, i: usize) -> &Numeric {
        &self.tensor[i]
    }

    #[inline(always)]
    pub fn mut_at(&mut self, i: usize) -> &mut Numeric {
        &mut self.tensor[i]
    }
}

// Layer 1   input(50, 1)  ----Relu(FC) ---> output(10, 1)
// Layer 2   input(10, 1)  ----Relu(FC) ---> output(2, 1)

declare_pm_loop_cnt!(H_CNT_1, 0);
pub fn fc_layer_impl<const FC_H: usize, const FC_W: usize>(
    param: &Tensor2D<FC_H, FC_W>,
    input: &Tensor1D<FC_W>,
    output: &PBox<Tensor1D<FC_H>>,
) {
    let param_h = FC_H;
    let param_w = FC_W;
    nv_for_loop!(H_CNT_1, i, 0 => param_h, {
        transaction::run(|j| {
            let mut sum_i = 0;
            for j in 0..param_w {
                sum_i += *param.at(i, j) * *input.at(j);
            }
            let output_i = if sum_i > 0 {sum_i} else {0};
            let output_ref = output.as_mut(j);
            *output_ref.mut_at(i) = output_i;
        });

    });
}
declare_pm_loop_cnt!(H_CNT_2, 0);
pub fn fc_layer_impl2<const FC_H: usize, const FC_W: usize>(
    param: &Tensor2D<FC_H, FC_W>,
    input: &PBox<Tensor1D<FC_W>>,
    output: &PBox<Tensor1D<FC_H>>,
) {
    let param_h = FC_H;
    let param_w = FC_W;
    nv_for_loop!(H_CNT_2, i, 0 => param_h, {

        transaction::run(|j| {
            let input_ref = input.as_ref(j);
            let output_ref = output.as_mut(j);
            let mut sum_i = 0;
            for j in 0..param_w {
                sum_i += *param.at(i, j) * *input_ref.at(j);
            }
            let output_i = if sum_i > 0 {sum_i} else {0};
            *output_ref.mut_at(i) =  output_i;
        });
    });
    //debug_user_tx_cache();
}

static PARAM_1: Tensor2D<10, 50> = Tensor2D::new([
    [
        7, 0, 2, 5, 4, 4, 5, 7, 9, 2, 9, 4, 9, 3, 0, 8, 4, 0, 2, 9, 3, 8, 1, 6, 6, 6, 5, 3, 3, 2,
        4, 0, 6, 9, 3, 7, 6, 3, 4, 9, 2, 5, 0, 5, 7, 3, 5, 8, 7, 5,
    ],
    [
        8, 0, 6, 0, 3, 6, 0, 6, 0, 0, 6, 3, 3, 0, 0, 0, 5, 4, 5, 9, 8, 4, 5, 8, 8, 5, 5, 9, 1, 7,
        0, 3, 8, 8, 5, 9, 5, 5, 2, 4, 2, 7, 1, 7, 2, 5, 0, 7, 6, 8,
    ],
    [
        2, 0, 6, 9, 4, 9, 8, 7, 0, 6, 4, 8, 1, 5, 5, 3, 6, 8, 4, 8, 8, 4, 7, 8, 4, 2, 4, 8, 0, 7,
        0, 7, 5, 3, 9, 7, 1, 6, 2, 1, 5, 8, 5, 9, 1, 8, 7, 5, 8, 9,
    ],
    [
        9, 1, 9, 7, 4, 1, 8, 3, 2, 5, 3, 9, 2, 8, 3, 1, 8, 8, 1, 4, 1, 3, 2, 4, 0, 5, 9, 5, 3, 9,
        2, 9, 1, 9, 5, 0, 2, 7, 0, 7, 3, 9, 1, 4, 6, 0, 2, 4, 6, 7,
    ],
    [
        4, 9, 0, 4, 7, 8, 3, 4, 4, 2, 2, 0, 5, 7, 0, 2, 7, 2, 3, 5, 0, 3, 2, 0, 3, 0, 4, 8, 1, 9,
        8, 2, 4, 5, 3, 1, 8, 0, 7, 1, 8, 1, 9, 1, 6, 8, 9, 3, 8, 5,
    ],
    [
        4, 4, 0, 3, 5, 7, 1, 9, 2, 2, 6, 6, 5, 0, 6, 5, 0, 3, 0, 9, 2, 6, 0, 0, 6, 6, 2, 5, 4, 8,
        7, 9, 4, 5, 6, 4, 8, 9, 3, 6, 3, 4, 3, 4, 4, 4, 6, 8, 6, 1,
    ],
    [
        5, 7, 8, 4, 6, 2, 0, 7, 9, 1, 3, 6, 0, 6, 8, 3, 4, 8, 9, 1, 9, 0, 3, 4, 6, 6, 7, 4, 5, 1,
        6, 0, 9, 9, 8, 6, 5, 5, 4, 8, 6, 4, 5, 9, 6, 7, 9, 8, 7, 8,
    ],
    [
        5, 0, 8, 2, 6, 3, 0, 1, 9, 9, 4, 9, 6, 0, 6, 6, 5, 8, 3, 4, 5, 5, 7, 9, 0, 8, 2, 8, 9, 4,
        0, 1, 7, 6, 7, 8, 8, 7, 7, 9, 1, 4, 9, 7, 2, 9, 0, 7, 8, 7,
    ],
    [
        3, 0, 0, 1, 0, 4, 7, 2, 9, 5, 6, 8, 6, 4, 3, 6, 2, 1, 5, 4, 5, 1, 4, 8, 6, 3, 5, 8, 0, 8,
        0, 3, 0, 1, 9, 0, 9, 8, 0, 9, 0, 5, 2, 8, 1, 6, 1, 9, 5, 9,
    ],
    [
        3, 7, 8, 5, 9, 8, 7, 4, 6, 9, 9, 1, 4, 1, 6, 2, 3, 4, 8, 9, 8, 0, 5, 6, 5, 3, 8, 2, 1, 4,
        3, 1, 6, 9, 5, 9, 1, 1, 9, 3, 0, 9, 6, 3, 3, 0, 8, 5, 6, 6,
    ],
]);
static PARAM_2: Tensor2D<2, 10> = Tensor2D::new([
    [5, 7, 5, 9, 9, 4, 9, 0, 1, 4],
    [2, 9, 2, 3, 2, 2, 8, 0, 8, 4],
]);

const BENCH_ITER: usize = 100;
declare_pm_loop_cnt!(BENCH_ITER_CNT, 0);

#[app]
pub fn task_dnn_inference() {
    wall_clock_begin();
    benchmark_start();
    nv_for_loop!(BENCH_ITER_CNT, i, 0 => BENCH_ITER, {
        dnn_inference();
        benchmark_reset_pm();
    });
    benchmark_end();
    set_benchmark_done();
    wall_clock_end();
    print_wall_clock_time();
    print_current_task_stats();
}

fn dnn_inference() {
    let input: Tensor1D<50> = Tensor1D::new([1; 50]);
    let (mut ob1, mut ob2) = transaction::run_pure_sys(|t| {
        let output_buffer_1 = PBox::new(Tensor1D::new([0; 10]), t);
        let output_buffer_2 = PBox::new(Tensor1D::new([0; 2]), t);
        (output_buffer_1, output_buffer_2)
    });

    fc_layer_impl(&PARAM_1, &input, &ob1);
    fc_layer_impl2(&PARAM_2, &ob1, &ob2);

    transaction::run(|j| {
        let res = ob2.as_ref(j);
        bench_dbg_print!("result: {}, {}", res.at(0), res.at(1));
    });
}

pub fn register() {
    task::register_app_no_param("dnn inference", 1, task_dnn_inference);
}
