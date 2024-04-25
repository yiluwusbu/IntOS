use crate::benchmarks::riotbench::{hash32, ValueType};

const BIT_MAP_SZ: usize = 64;

pub struct ApproxDistinctCount {
    distinct_count: usize,
    bit_map: [u8; BIT_MAP_SZ / 8],
}

impl ApproxDistinctCount {
    pub const fn new() -> Self {
        Self {
            distinct_count: 0,
            bit_map: [0; BIT_MAP_SZ / 8],
        }
    }

    pub fn check(&self, v: ValueType) -> bool {
        let h = (hash32(v as u32) as usize) % BIT_MAP_SZ;
        let idx = h / 8;
        let shift = idx % 8;
        return (self.bit_map[idx] & (1 << shift)) != 0;
    }

    pub fn inc_count(&mut self, v: ValueType) -> usize {
        let h = (hash32(v as u32) as usize) % BIT_MAP_SZ;
        let idx = h / 8;
        let shift = idx % 8;
        self.bit_map[idx] |= (1 << shift);
        self.distinct_count += 1;
        return self.distinct_count;
    }

    pub fn get_count(&self) -> usize {
        self.distinct_count
    }
}
