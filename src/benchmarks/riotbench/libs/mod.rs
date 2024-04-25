use crate::syscalls;

pub mod annotation;
pub mod approx_distinct_count;
pub mod average;
pub mod compact_string;
pub mod data_layout;
pub mod decision_tree;
pub mod filters;
pub mod interpolation;
pub mod linear_regression;
pub mod simple_linear_regression;

static mut SEED: u32 = 12345; // Initial seed
const A: u32 = 1664525; // Multiplier
const C: u32 = 1013904223; // Increment
const M: u32 = 4294967295; // Modulus (2^32-1)

// Function to generate a pseudo-random number
pub fn random_u32(bound: u32) -> u32 {
    return unsafe {
        SEED = (A * SEED + C) % M;
        SEED % bound
    };
}
