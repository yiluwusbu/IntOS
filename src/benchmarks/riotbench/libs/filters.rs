use crate::benchmarks::{riotbench::ValueType, Hash};

pub struct RangeFilter<T>
where
    T: PartialOrd,
{
    min: T,
    max: T,
}

impl<T> RangeFilter<T>
where
    T: PartialOrd,
{
    pub fn new(min: T, max: T) -> Self {
        Self { min, max }
    }

    pub fn check(&self, val: T) -> bool {
        val >= self.min && val <= self.max
    }
}

pub struct BloomFilter<const N: usize> {
    table: [bool; N],
}

impl<const N: usize> BloomFilter<N> {
    pub const fn new() -> Self {
        Self { table: [false; N] }
    }

    pub const fn new_with(table: [bool; N]) -> Self {
        Self { table }
    }

    pub fn might_contain<T: Hash>(&self, val: T) -> bool {
        let idx = val.hash() % N;
        return self.table[idx];
    }
}

pub struct KalmanFilter {
    q_process_noise: ValueType,
    r_sensor_noise: ValueType,
    p0_prior_error_covariance: ValueType,
    x0_previous_estimation: ValueType,
}

impl KalmanFilter {
    pub fn new(q: ValueType, r: ValueType, p0: ValueType, x0: ValueType) -> Self {
        Self {
            q_process_noise: q,
            r_sensor_noise: r,
            p0_prior_error_covariance: p0,
            x0_previous_estimation: x0,
        }
    }

    pub fn filter(&mut self, v: ValueType) -> ValueType {
        let mut p1_current_error_covariance = self.p0_prior_error_covariance + self.q_process_noise;
        let kalman_gain =
            p1_current_error_covariance / (p1_current_error_covariance + self.r_sensor_noise);
        let x1_current_estimation =
            self.x0_previous_estimation + kalman_gain * (v - self.x0_previous_estimation);
        p1_current_error_covariance = (1 - kalman_gain) * p1_current_error_covariance;
        self.x0_previous_estimation = x1_current_estimation;
        self.p0_prior_error_covariance = p1_current_error_covariance;
        x1_current_estimation
    }
}
