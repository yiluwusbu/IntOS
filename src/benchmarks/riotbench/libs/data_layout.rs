use crate::{benchmarks::riotbench::ValueType, syscalls::sys_get_time_out_of_tx, time::Time};

use super::compact_string::CompactString;

const SENSOR_VALUE_NUM: usize = 4;
const STR_SZ: usize = 4;

#[derive(Clone, Copy)]
pub struct SensorData {
    pub sensor_id: u8,
    pub value_cnt: u8,
    pub values: [ValueType; SENSOR_VALUE_NUM],
    pub ts: Time,
    pub annotation: CompactString<STR_SZ>,
}

pub struct TrainingData {
    pub id: u8,
    pub values: [ValueType; SENSOR_VALUE_NUM],
    pub class: usize,
}

pub struct AnalysisResult {
    pub sensor_id: u8,
    pub analysis_type: &'static str,
    pub result: ValueType,
    pub err_val: ValueType,
}

impl SensorData {
    pub fn new() -> Self {
        Self {
            sensor_id: 0,
            value_cnt: 0,
            values: [0, 0, 0, 0],
            ts: 0,
            annotation: CompactString::new(),
        }
    }
}

pub fn read_sensor_value(sensor_id: u8) -> SensorData {
    let ts = sys_get_time_out_of_tx();
    let v1 = ((sys_get_time_out_of_tx() + 1) % 120) as ValueType;
    let v2 = ((sys_get_time_out_of_tx() + 2) % 120) as ValueType;
    let v3 = ((sys_get_time_out_of_tx() + 3) % 120) as ValueType;
    let v4 = ((sys_get_time_out_of_tx() + 4) % 120) as ValueType;
    let ret = SensorData {
        sensor_id,
        value_cnt: SENSOR_VALUE_NUM as u8,
        values: [v1, v2, v3, v4],
        ts,
        annotation: CompactString::new_with("x"),
    };
    ret
}

impl core::fmt::Debug for SensorData {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "sid: {}, v1: {}, v2: {}, v3: {}, v4: {}",
            self.sensor_id, self.values[0], self.values[1], self.values[2], self.values[3]
        )
    }
}
