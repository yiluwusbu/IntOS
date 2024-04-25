use crate::benchmarks::riotbench::ValueType;

pub struct BlockAvg<const WIN: usize> {
    sum: ValueType,
    cnt: usize,
}

impl<const WIN: usize> BlockAvg<WIN> {
    pub const fn new() -> Self {
        Self { sum: 0, cnt: 0 }
    }

    pub fn add(&mut self, v: ValueType) -> ValueType {
        self.sum += v;
        self.cnt += 1;
        let ret = self.sum / self.cnt as ValueType;
        if self.cnt == WIN {
            self.cnt = 0;
            self.sum = 0;
        }
        ret
    }
}
