use crate::time::Time;

use super::Hash;

pub mod etl;
pub mod libs;
pub mod pred;
pub mod stats;
pub mod train;

type ValueType = i16;
pub const INVALID_VALUE: ValueType = -1;
pub const NUM_SENSORS: usize = 2;
pub const DEFAULT_Q_LENGTH: usize = 4;
const TRAIN_DATASET_SZ: usize = 16;
// Robert Jenkins' 32 bit integer hash function
pub fn hash32(mut a: u32) -> u32 {
    a = (a + 0x7ed55d16) + (a << 12);
    a = (a ^ 0xc761c23c) ^ (a >> 19);
    a = (a + 0x165667b1) + (a << 5);
    a = (a + 0xd3a2646c) ^ (a << 9);
    a = (a + 0xfd7046c5) + (a << 3);
    a = (a ^ 0xb55a4f09) ^ (a >> 16);
    return a;
}

impl Hash for ValueType {
    fn hash(&self) -> usize {
        hash32(*self as u32) as usize
    }
}

impl Hash for Time {
    fn hash(&self) -> usize {
        hash32(*self as u32) as usize
    }
}
