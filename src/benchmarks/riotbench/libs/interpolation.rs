use crate::benchmarks::riotbench::ValueType;

pub struct Interpolation<const N: usize> {
    past_values: [ValueType; N],
    size: usize,
}

impl<const N: usize> Interpolation<N> {
    pub fn new() -> Self {
        Self {
            past_values: [0; N],
            size: 0,
        }
    }

    pub fn interpolate(&self) -> ValueType {
        let mut sum = 0;
        for i in 0..self.size {
            sum += self.past_values[i];
        }
        if self.size != 0 {
            return sum / self.size as ValueType;
        } else {
            return 0;
        }
    }

    fn remove_first(&mut self) {
        for i in 0..self.size - 1 {
            self.past_values[i] = self.past_values[i + 1];
        }
        self.size -= 1;
    }

    pub fn insert(&mut self, v: ValueType) {
        if self.size == N {
            self.remove_first();
        }

        self.past_values[self.size] = v;
        self.size += 1;
    }
}
