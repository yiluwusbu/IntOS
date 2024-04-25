use crate::{
    bench_println,
    benchmarks::riotbench::ValueType,
    declare_pm_loop_cnt, nv_for_loop,
    user::{
        pbox::{PBox, PRef},
        transaction,
    },
    util::benchmark_clock,
};

#[derive(Clone, Copy)]
pub struct LinearRegression<const N: usize> {
    coefficients: [ValueType; N],
}

impl<const N: usize> LinearRegression<N> {
    pub const fn new(coefficients: [ValueType; N]) -> Self {
        Self { coefficients }
    }

    pub fn predict(&self, x: &[ValueType; N]) -> ValueType {
        let mut res = 0;
        for i in 0..N - 1 {
            res += self.coefficients[i] * x[i];
        }
        return res + self.coefficients[N - 1];
    }

    pub fn update_model(&mut self, coefficients: &[ValueType; N]) {
        for i in 0..N {
            self.coefficients[i] = coefficients[i];
        }
    }
}

pub fn transpose<const N: usize, const M: usize>(m: &[[ValueType; N]; M]) -> [[ValueType; M]; N] {
    let mut ret = [[0; M]; N];
    for i in 0..M {
        for j in 0..N {
            ret[j][i] = m[i][j];
        }
    }
    ret
}

pub fn gradient<const N: usize, const M: usize>(
    samples_x: &[[ValueType; N]; M],
    samples_y: &[ValueType; M],
    param: &[ValueType; N],
) -> [ValueType; N] {
    let mut err = [0; M];
    for i in 0..M {
        let mut sum = 0;
        for j in 0..N {
            sum += samples_x[i][j] * param[j];
        }
        err[i] = sum - samples_y[i];
    }
    let x_t = transpose(samples_x);
    let mut ret = [0; N];
    for i in 0..N {
        let mut sum = 0;
        for j in 0..M {
            sum += x_t[i][j] * err[j];
        }
        sum = sum * 2 / M as ValueType;
        ret[i] = sum;
    }
    ret
}

declare_pm_loop_cnt!(TRAIN_LOOP_CNT, 0);

pub fn debug_print_lr_model<const N: usize>(lr: &LinearRegression<N>) {
    crate::task_print!("LR Model Param: ");
    crate::board_hprint!("[");
    for i in 0..N {
        crate::board_hprint!("{}, ", lr.coefficients[i]);
    }
    crate::board_hprintln!("]");
}

pub fn train<const N: usize, const M: usize>(
    samples_x: &[[ValueType; N]; M],
    samples_y: &[ValueType; M],
    new_param: &PBox<LinearRegression<N>>,
    lr: ValueType,
    iter: usize,
) {
    nv_for_loop!(TRAIN_LOOP_CNT, i, 0 => iter, {
        transaction::run(|j| {
            let param = &mut new_param.as_mut(j).coefficients;
            for i in 0..1 {
                let gradient = gradient(samples_x, samples_y, param);
                for i in 0..N {
                    param[i] -= lr * gradient[i];
                }
            }
        });
    });
}
