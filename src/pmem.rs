use core::cell::UnsafeCell;
use core::fmt::Display;
use core::mem::size_of;
use core::ops::Deref;
use core::ptr::NonNull;
use vcell::VolatileCell;

use crate::marker::{PSafe, TxInSafe, TxOutSafe};
use crate::task::{current, is_scheduler_started, task_get_stats};
use crate::util::compiler_pm_fence;
use crate::util::{arch_addr_align_up, benchmark_clock};
use crate::{board, debug_print};

const JOURNAL_MAGIC: usize = 0xABCD;

#[macro_export]
macro_rules! declare_pm_var {
    ($name: ident, $t: ty, $e: expr) => {
      #[link_section = ".pmem"]
      static mut $name: crate::pmem::PMVar<$t> = unsafe { crate::pmem::PMVar::new($e) };
    };
    ($v: vis, $name: ident, $t: ty, $e: expr) => {
      #[link_section = ".pmem"]
      $v static mut $name: crate::pmem::PMVar<$t> = unsafe { crate::pmem::PMVar::new($e) };
    };
}

#[macro_export]
macro_rules! declare_pm_var_array {
    ($name: ident, $t: ty, $sz: expr, $e: expr) => {
        #[link_section = ".pmem"]
        static mut $name: [crate::pmem::PMVar<$t>; $sz] =
            [unsafe { crate::pmem::PMVar::new($e) }; $sz];
    };
}

#[macro_export]
macro_rules! declare_const_pm_var {
    ($name: ident, $t: ty, $e: expr) => {
        #[link_section = ".pmem"]
        static $name: crate::pmem::PMVar<$t> = unsafe { crate::pmem::PMVar::new($e) };
    };
}

#[macro_export]
macro_rules! declare_pm_var_unsafe {
    ($name: ident, $t: ty, $e: expr) => {
      #[link_section = ".pmem"]
      static mut $name: $t = $e;
    };
    ($v: vis, $name: ident, $t: ty, $e: expr) => {
      #[link_section = ".pmem"]
      $v static mut $name: $t = $e;
    };
}

#[macro_export]
macro_rules! declare_const_pm_var_unsafe {
    ($name: ident, $t: ty, $e: expr) => {
      #[link_section = ".pmem"]
      static $name: $t = $e;
    };
    ($v: vis, $name: ident, $t: ty, $e: expr) => {
      #[link_section = ".pmem"]
      $v static  $name: $t = $e;
    };
}

// pub unsafe trait CreateUndoLog {
//   fn create_undo_log(&self, j: JournalHandle)
//   where Self : Sized
//   {
//     j.get_mut().append_log_of(self as * const Self as * mut Self);
//   }
// }

#[repr(transparent)]
pub struct PMPtr<T: ?Sized> {
    addr: NonNull<T>,
}

impl<T> PartialEq for PMPtr<T> {
    fn eq(&self, other: &Self) -> bool {
        self.addr == other.addr
    }
}

impl<T> Eq for PMPtr<T> {}

impl<T> Clone for PMPtr<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for PMPtr<T> {}

unsafe impl<T> TxInSafe for PMPtr<T> {}
unsafe impl<T> TxOutSafe for PMPtr<T> {}
unsafe impl<T> PSafe for PMPtr<T> {}

impl<T> PMPtr<T> {
    #[inline(always)]
    pub unsafe fn from_u8(ptr: PMPtr<u8>) -> Self {
        Self {
            addr: unsafe { NonNull::new_unchecked(ptr.as_ptr() as *mut T) },
        }
    }

    #[inline(always)]
    pub fn create_log(&self, j: JournalHandle) {
        j.get_mut().append_log_of(self.addr.as_ptr());
    }

    #[inline(always)]
    pub fn as_mut<'a>(&mut self, j: JournalHandle) -> &'a mut T {
        self.create_log(j);
        unsafe { self.addr.as_mut() }
    }
}

impl<T: ?Sized> PMPtr<T> {
    pub unsafe fn new(ptr: *mut T) -> Self {
        Self {
            addr: NonNull::new_unchecked(ptr),
        }
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *mut T {
        self.addr.as_ptr()
    }

    #[inline(always)]
    pub fn as_ref<'a>(&self) -> &'a T {
        unsafe { self.addr.as_ref() }
    }

    #[inline(always)]
    pub unsafe fn as_mut_no_logging<'a>(&mut self) -> &'a mut T {
        self.addr.as_mut()
    }

    #[inline(always)]
    pub unsafe fn from_ptr(p: *mut T) -> Self {
        debug_assert!(!p.is_null());
        Self {
            addr: unsafe { NonNull::new_unchecked(p) },
        }
    }

    #[inline(always)]
    pub unsafe fn from_mut_ref(r: &mut T) -> Self {
        Self {
            addr: unsafe { NonNull::new_unchecked(r as *mut T) },
        }
    }

    #[inline(always)]
    pub unsafe fn from_ref(r: &T) -> Self {
        Self {
            addr: unsafe { NonNull::new_unchecked(r as *const T as *mut T) },
        }
    }

    #[inline(always)]
    pub unsafe fn to_u8(self) -> PMPtr<u8> {
        PMPtr::<u8> {
            addr: unsafe { NonNull::new_unchecked(self.as_ptr() as *mut u8) },
        }
    }
}

unsafe impl<T> TxInSafe for PMVar<T> {}
unsafe impl<T> TxOutSafe for PMVar<T> {}

#[derive(Copy, Clone)]
pub struct PMVar<T> {
    var: T,
}

impl<T> PMVar<T> {
    pub const unsafe fn new(v: T) -> Self {
        Self { var: v }
    }

    #[inline(always)]
    pub fn borrow_mut(&mut self, j: JournalHandle) -> &mut T {
        // let mut_ref: &mut Self = unsafe {
        //   &mut *(self as * const Self as * mut Self)
        // };
        // j.append_log_of(mut_ref as * mut Self);
        // &mut mut_ref.var
        j.get_mut().append_log_of(self as *mut Self);
        &mut self.var
    }

    #[inline(always)]
    pub unsafe fn borrow_mut_no_logging(&mut self) -> &mut T {
        // let mut_ref: &mut Self = unsafe {
        //   &mut *(self as * const Self as * mut Self)
        // };
        // &mut mut_ref.var
        &mut self.var
    }

    #[inline(always)]
    pub fn borrow(&self) -> &T {
        &self.var
    }

    #[inline(always)]
    pub fn set(&mut self, new_val: T, j: JournalHandle) {
        // let mut_ref: &mut Self = unsafe {
        //   &mut *(self as * const Self as * mut Self)
        // };
        // j.append_log_of(mut_ref as * mut Self);
        // mut_ref.var = new_val;
        j.get_mut().append_log_of(self as *mut Self);
        self.var = new_val;
    }

    #[inline(always)]
    pub unsafe fn to_pm_ptr(&self) -> PMPtr<T> {
        PMPtr::from_ref(&self.var)
    }
}

impl<T> Deref for PMVar<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.var
    }
}

#[cfg(not(sram_baseline))]
const JOURNAL_SIZE: usize = board::PM_JOURNAL_SIZE;
#[cfg(sram_baseline)]
const JOURNAL_SIZE: usize = 0;

#[derive(Clone, Copy)]
pub struct JournalHandle(*mut Journal);

impl JournalHandle {
    pub fn new(ptr: *const Journal) -> Self {
        Self(ptr as *mut Journal)
    }

    pub unsafe fn new_dummy() -> Self {
        Self(unsafe { core::ptr::null_mut() })
    }

    #[inline(always)]
    pub fn get_mut(&self) -> &mut Journal {
        // debug_assert!(!self.0.is_dangling());
        unsafe { &mut *self.0 }
    }

    pub fn get(&self) -> &Journal {
        unsafe { &*self.0 }
    }
}

pub struct Journal {
    magic_header: usize,
    tail: usize,
    logs: [u8; JOURNAL_SIZE],
    magic_footer: usize,
}

impl Display for Journal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "tail: {}, magic header: {:#X}, magic footer{:#X}",
            self.tail, self.magic_header, self.magic_footer
        )
    }
}

#[cfg(feature = "profile_log")]
static mut K_LOG_SZ: u32 = 0;

#[cfg(feature = "profile_log")]
static mut U_LOG_SZ: u32 = 0;

#[cfg(feature = "profile_log")]
pub fn add_klog_sz(log_sz: usize) {
    unsafe {
        K_LOG_SZ += log_sz as u32;
    }
}

#[cfg(feature = "profile_log")]
pub fn add_ulog_sz(log_sz: usize) {
    unsafe {
        U_LOG_SZ += log_sz as u32;
    }
}

#[cfg(feature = "profile_log")]
pub fn get_klog_sz() -> u32 {
    unsafe { K_LOG_SZ }
}

#[cfg(feature = "profile_log")]
pub fn get_ulog_sz() -> u32 {
    unsafe { U_LOG_SZ }
}

fn is_jounral_of_kernel(j: &Journal) -> bool {
    let j_ptr = j as *const Journal as *mut Journal;
    if j_ptr == crate::recover::get_boot_tx().get_journal().0 {
        return true;
    } else if j_ptr == current().get_mut_tx().get_journal().0 {
        return true;
    } else {
        return false;
    }
}

#[inline]
fn pre_log_hook(j: &Journal, log_sz: usize) {
    #[cfg(feature = "profile_log")]
    {
        if is_jounral_of_kernel(j) {
            add_klog_sz(log_sz);
        } else {
            add_ulog_sz(log_sz);
        }
    }
}

#[inline]
fn post_log_hook() {
    #[cfg(feature = "profile_log")]
    {
        unsafe {}
    }
}

impl Journal {
    pub const fn new() -> Self {
        Self {
            magic_header: JOURNAL_MAGIC,
            tail: 0,
            logs: [0; JOURNAL_SIZE],
            magic_footer: JOURNAL_MAGIC,
        }
    }

    fn check_integrity(&self) {
        debug_assert!(
            self.magic_footer == JOURNAL_MAGIC && self.magic_header == JOURNAL_MAGIC,
            "Corrupted Journal Header: {}",
            self
        );
    }

    #[cfg(feature = "crash_safe")]
    pub fn append_log_of<T>(&mut self, obj: *mut T) {
        let obj_sz = size_of::<T>();
        let obj_sz_aligned = arch_addr_align_up(obj_sz);

        self.check_integrity();

        let record_sz = obj_sz_aligned + size_of::<usize>() + size_of::<*mut T>();
        pre_log_hook(self, record_sz);
        // debug_print!("type name: {}", core::any::type_name::<T>());
        // debug_print!("Record sz: {}, object sz: {}, aligned obj sz: {}, tail: {}", record_sz, obj_sz, obj_sz_aligned, self.tail);
        // TODO(): Don't just panic here
        assert!(self.tail + record_sz <= JOURNAL_SIZE);
        unsafe {
            // copy object
            core::ptr::copy_nonoverlapping(
                obj as *mut u8,
                &mut self.logs[self.tail] as *mut u8,
                obj_sz,
            );
            // hprintln!("Object sz: {}", obj_sz).unwrap();
            // copy address
            if (self.tail + obj_sz_aligned) % 4 != 0 {
                debug_print!("type name: {}", core::any::type_name::<T>());
                debug_print!(
                    "obj sz: {}, aligned sz: {}, tail: {}",
                    obj_sz,
                    obj_sz_aligned,
                    self.tail
                );
            }
            let address_ptr =
                &mut *(&mut self.logs[self.tail + obj_sz_aligned] as *mut u8 as *mut usize);
            *address_ptr = obj as usize;
            // copy size
            let size_ptr = &mut *(&mut self.logs[self.tail + obj_sz_aligned + size_of::<*mut T>()]
                as *mut u8 as *mut usize);
            *size_ptr = size_of::<T>();
        }
        // increase tail
        compiler_pm_fence();
        self.tail += record_sz;
        post_log_hook();
    }

    #[cfg(not(feature = "crash_safe"))]
    pub fn append_log_of<T>(&mut self, _obj: *mut T) {}

    #[inline(always)]
    pub fn clear(&mut self) {
        self.tail = 0;
    }

    #[inline(always)]
    pub fn init(&mut self) {
        self.magic_footer = JOURNAL_MAGIC;
        self.magic_header = JOURNAL_MAGIC;
        self.tail = 0;
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.tail == 0
    }

    pub fn recover(&mut self) {
        let mut ptr = self.tail;
        while ptr > 0 {
            let size =
                unsafe { *(&self.logs[ptr - size_of::<usize>()] as *const u8 as *const usize) };
            let aligned_size = arch_addr_align_up(size);
            let object_addr =
                unsafe { *(&self.logs[ptr - size_of::<usize>() * 2] as *const u8 as *const usize) };
            let object_log_addr =
                &self.logs[ptr - size_of::<usize>() * 2 - aligned_size] as *const u8;
            unsafe {
                core::ptr::copy_nonoverlapping(object_log_addr, object_addr as *mut u8, size);
            }
            ptr -= size_of::<usize>() * 2 + aligned_size;
        }
        assert!(ptr == 0);
        // recover complete, empty the logs
        self.tail = 0;
    }
}

#[repr(transparent)]
pub struct PVolatilePtr<T> {
    p: VolatileCell<*mut T>,
}

impl<T> PVolatilePtr<T> {
    pub const fn new(ptr: *mut T) -> Self {
        Self {
            p: VolatileCell::new(ptr),
        }
    }
    pub fn load(&self) -> *mut T {
        unsafe { *self.p.as_ptr() }
    }

    pub fn volatile_load(&self) -> *mut T {
        self.p.get()
    }

    #[cfg(not(feature = "opt_list"))]
    pub fn store(&mut self, ptr: *mut T, j: JournalHandle) {
        j.get_mut().append_log_of(self as *mut Self);
        self.p.set(ptr);
    }

    #[cfg(feature = "opt_list")]
    pub fn store(&mut self, ptr: *mut T) {
        self.p.set(ptr);
    }
}
