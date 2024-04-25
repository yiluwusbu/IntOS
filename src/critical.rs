use crate::{
    arch::{disable_interrupt, enable_interrupt},
    declare_const_pm_var_unsafe,
};
use core::cell::UnsafeCell;
// pub fn with<F>(f : F)
// where
//   F: FnOnce() {
//     disable_interrupt();
//     f();
//     enable_interrupt();
// }

pub struct CriticalNestingLevel {
    cnt: UnsafeCell<usize>,
}

impl CriticalNestingLevel {
    #[inline(always)]
    pub fn inc(&self) {
        unsafe {
            *self.cnt.get() += 1;
        }
    }

    #[inline(always)]
    pub fn dec(&self) -> usize {
        unsafe {
            *self.cnt.get() -= 1;
            *self.cnt.get()
        }
    }

    pub const fn new() -> Self {
        Self {
            cnt: UnsafeCell::new(0),
        }
    }
}

unsafe impl Sync for CriticalNestingLevel {}

// static NESTING_CNT: CriticalNestingLevel = CriticalNestingLevel::new();
declare_const_pm_var_unsafe!(
    NESTING_CNT,
    CriticalNestingLevel,
    CriticalNestingLevel::new()
);

pub struct CriticalSection();

impl CriticalSection {
    pub unsafe fn new() -> Self {
        CriticalSection()
    }
}

#[inline(always)]
pub fn with_no_interrupt<F, T>(f: F) -> T
where
    F: FnOnce(&CriticalSection) -> T,
{
    let cs = CriticalSection {};
    disable_interrupt();
    NESTING_CNT.inc();
    let v = f(&cs);
    if NESTING_CNT.dec() == 0 {
        enable_interrupt();
    }
    return v;
}

pub fn is_in_critical() -> bool {
    unsafe { *NESTING_CNT.cnt.get() > 0 }
}

pub fn exit_all_critical() {
    unsafe {
        *NESTING_CNT.cnt.get() = 0;
    }
}
