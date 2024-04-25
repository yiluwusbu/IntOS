use core::{cell::UnsafeCell, sync::atomic};

use crate::pmem::JournalHandle;

use super::pbox::PRef;

#[macro_export]
macro_rules! declare_pm_static {
    ($name: ident, $t: ty, $e: expr) => {
      #[link_section = ".pmem"]
      static $name: crate::user::pstatics::PStatic<$t> = unsafe { crate::user::pstatics::PStatic::new($e) };
    };

    (mut $name: ident, $t: ty, $e: expr) => {
        #[link_section = ".pmem"]
        static $name: crate::user::pstatics::PStatic<$t> = unsafe { crate::user::pstatics::PStatic::new($e) };
    };

    ($v: vis, $name: ident, $t: ty, $e: expr) => {
      #[link_section = ".pmem"]
      $v static $name: crate::user::pstatics::PStatic<$t> = unsafe { crate::user::pstatics::PStatic::new($e) };
    };

    ($v: vis, mut $name: ident, $t: ty, $e: expr) => {
        #[link_section = ".pmem"]
        $v static mut $name: crate::user::pstatics::PStatic<$t> = unsafe { crate::user::pstatics::PStatic::new($e) };
    };
}

unsafe impl<T> Sync for PStatic<T> {}

pub struct PStatic<T> {
    var: UnsafeCell<T>,
}

impl<T> PStatic<T> {
    pub const unsafe fn new(v: T) -> Self {
        Self {
            var: UnsafeCell::new(v),
        }
    }

    #[inline(always)]
    pub fn as_mut(&self, j: JournalHandle) -> &mut T {
        let ptr = self.var.get();
        j.get_mut().append_log_of(ptr);
        unsafe { &mut *ptr }
    }

    #[inline(always)]
    pub fn as_ref(&self, j: JournalHandle) -> &T {
        let ptr = self.var.get();
        unsafe { &*ptr }
    }

    #[inline(always)]
    pub unsafe fn as_ref_no_journal(&self) -> &T {
        let ptr = self.var.get();
        unsafe { &*ptr }
    }

    #[inline(always)]
    pub fn as_pref(&self) -> PRef<T> {
        unsafe { PRef::new(self.var.get()) }
    }

    #[inline(always)]
    pub unsafe fn set(&self, v: T) {
        let ptr = self.var.get();
        unsafe { *ptr = v }
    }
}

pub struct PLoopCounter {
    cnt: UnsafeCell<usize>,
}

unsafe impl Sync for PLoopCounter {}

impl PLoopCounter {
    pub const fn new(cnt: usize) -> Self {
        Self {
            cnt: UnsafeCell::new(cnt),
        }
    }

    #[inline(always)]
    pub unsafe fn inc(&self, n: usize, j: JournalHandle) {
        j.get_mut().append_log_of(self as *const Self as *mut Self);
        *self.cnt.get() += n;
    }

    #[inline(always)]
    pub fn get(&self) -> usize {
        unsafe { *self.cnt.get() }
    }

    #[inline(always)]
    pub unsafe fn set(&self, v: usize) {
        *self.cnt.get() = v;
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *mut usize {
        self.cnt.get()
    }
}

#[macro_export]
macro_rules! declare_pm_loop_cnt {
    ($name: ident, $e: expr) => {
        #[link_section = ".pmem"]
        pub static $name: crate::user::pstatics::PLoopCounter =
            crate::user::pstatics::PLoopCounter::new($e);
    };
}
