use super::AllocError;
use crate::debug_print;
use crate::marker::{PSafe, TxInSafe, TxOutSafe};
use crate::pmem::{JournalHandle, PMPtr};
use crate::syscalls::{self as sys, sys_pfree, SyscallToken};
use crate::task::current;
use crate::user::transaction;
use core::mem::MaybeUninit;
use core::ops::Deref;

pub struct RelaxedPBox<T> {
    ptr: PMPtr<T>,
}

impl<T> RelaxedPBox<T> {
    pub fn new(x: T, t: SyscallToken) -> Self {
        let ptr = unsafe { sys::sys_palloc_relaxed(x, t) };
        let ptr = match ptr {
            None => {
                panic!("PBox OOM");
            }
            Some(p) => p,
        };
        Self { ptr }
    }

    pub fn try_new(x: T, t: SyscallToken) -> Result<Self, AllocError> {
        let ptr = unsafe { sys::sys_palloc_relaxed(x, t) };
        match ptr {
            None => Err(AllocError),
            Some(p) => Ok(Self { ptr: p }),
        }
    }

    pub fn try_new_for_kernel(x: T, j: JournalHandle) -> Result<Self, AllocError> {
        let ptr = unsafe { crate::heap::pm_new_relaxed(x, j) };
        match ptr {
            None => Err(AllocError),
            Some(p) => Ok(Self { ptr: p }),
        }
    }

    pub unsafe fn as_mut_no_logging(&mut self) -> &mut T {
        self.ptr.as_mut_no_logging()
    }
}

unsafe impl<T> PSafe for PBox<T> {}

pub struct PBox<T> {
    ptr: PMPtr<T>,
}

impl<T: PSafe> PBox<T> {
    pub fn new(x: T, t: SyscallToken) -> Self {
        let ptr = sys::sys_palloc(x, t);
        let ptr = match ptr {
            None => {
                panic!("PBox OOM");
            }
            Some(p) => p,
        };
        Self { ptr }
    }

    pub fn try_new(x: T, t: SyscallToken) -> Result<Self, AllocError> {
        let ptr = sys::sys_palloc(x, t);
        match ptr {
            None => Err(AllocError),
            Some(p) => Ok(Self { ptr: p }),
        }
    }

    // pub fn as_mut(&mut self, j: JournalHandle) -> &mut T{
    //     self.ptr.as_mut(j)
    // }

    pub fn as_mut(&self, j: JournalHandle) -> &mut T {
        self.ptr.create_log(j);
        unsafe { &mut *self.ptr.as_ptr() }
    }

    pub fn as_ref(&self, _j: JournalHandle) -> &T {
        self.ptr.as_ref()
    }

    pub fn as_pref_wlog(&self, _j: JournalHandle) -> PRefWLog<T> {
        PRefWLog {
            inner: unsafe { &mut *self.ptr.as_ptr() },
        }
    }

    pub unsafe fn as_ref_no_journal(&self) -> &T {
        self.ptr.as_ref()
    }

    pub unsafe fn as_mut_no_logging(&mut self) -> &mut T {
        self.ptr.as_mut_no_logging()
    }

    pub fn into_inner(boxed: Self, t: SyscallToken) -> T {
        let mut dst: T = unsafe { MaybeUninit::uninit().assume_init() };
        unsafe {
            core::ptr::copy_nonoverlapping(boxed.ptr.as_ptr(), &mut dst as *mut T, 1);
        }
        unsafe {
            sys_pfree(boxed.ptr, t);
        }
        core::mem::forget(boxed);
        dst
    }

    pub unsafe fn from_raw(ptr: *mut T) -> Self {
        Self {
            ptr: PMPtr::new(ptr),
        }
    }

    pub fn into_raw(b: Self) -> *mut T {
        let ptr = b.ptr.as_ptr();
        core::mem::forget(b);
        ptr
    }
}

impl<T: Sized> Drop for PBox<T> {
    fn drop(&mut self) {
        // call the deallocation function
        // crate::task_print!("Dropping PBox");
        debug_print!("Dropping PBox of {}", core::any::type_name::<T>());
        // transaction::run(|j| {
        //     unsafe {
        //         let obj = core::ptr::read(self.ptr.as_ptr());
        //         // content of object should drop here
        //         drop(obj);
        //         sys_pfree(self.ptr, j);
        //     }
        // });
    }
}

unsafe impl<T: PSafe + Send> Send for PBox<T> {}
unsafe impl<T: PSafe + Sync> Sync for PBox<T> {}

unsafe impl<T: Send> Send for RelaxedPBox<T> {}
unsafe impl<T: Sync> Sync for RelaxedPBox<T> {}

// Init -> Read -> WLog
// Init -> W -> RW
// WLog -> W -> RW
// WLog -> R -> WLog
// RW -> Any -> RW
//

impl<T> !TxOutSafe for PtrRW<T> {}
impl<T> !TxOutSafe for PtrWLog<T> {}

unsafe impl<T> TxInSafe for PRef<'_, T> {}
pub struct Ptr<T> {
    boxed: PBox<T>,
}

pub struct PtrRW<T> {
    boxed: PBox<T>,
}
pub struct PtrRO<T> {
    boxed: PBox<T>,
}

pub struct PtrWLog<T> {
    boxed: PBox<T>,
}

impl<T: PSafe> Ptr<T> {
    pub fn new(x: T, t: SyscallToken) -> Self {
        Self {
            boxed: PBox::new(x, t),
        }
    }

    pub fn write(mut self, new_data: T, _j: JournalHandle) -> PtrRW<T> {
        unsafe {
            *self.boxed.as_mut_no_logging() = new_data;
        }
        PtrRW { boxed: self.boxed }
    }

    pub fn read<F, R>(self, f: F, j: JournalHandle) -> (R, PtrWLog<T>)
    where
        F: FnOnce(&T) -> R,
    {
        let v = f(self.boxed.as_ref(j));
        let p = PtrWLog { boxed: self.boxed };
        (v, p)
    }

    pub fn into_ptr_ro(self, _j: JournalHandle) -> PtrRO<T> {
        PtrRO { boxed: self.boxed }
    }

    pub fn into_pbox(self) -> PBox<T> {
        self.boxed
    }

    pub fn as_pref(&mut self) -> PRef<T> {
        PRef {
            inner: unsafe { self.boxed.as_mut_no_logging() },
        }
    }

    pub unsafe fn as_pm_ptr(&self) -> PMPtr<T> {
        self.boxed.ptr
    }

    pub unsafe fn as_ref(&self) -> &T {
        self.boxed.as_ref_no_journal()
    }
}

impl<T: PSafe> PtrRO<T> {
    pub fn as_ref(&self, j: JournalHandle) -> &T {
        self.boxed.as_ref(j)
    }

    pub unsafe fn into_ptr(self) -> Ptr<T> {
        Ptr { boxed: self.boxed }
    }
}

impl<T: PSafe> core::ops::Deref for PtrRO<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { self.boxed.as_ref_no_journal() }
    }
}

impl<T: PSafe> PtrWLog<T> {
    pub fn write<F>(mut self, f: F, j: JournalHandle) -> PtrRW<T>
    where
        F: FnOnce(&mut T),
    {
        let mut_ref = self.boxed.as_mut(j);
        f(mut_ref);
        PtrRW { boxed: self.boxed }
    }

    pub fn as_mut(&mut self, j: JournalHandle) -> &mut T {
        self.boxed.as_mut(j)
    }

    pub fn as_ref(&self, j: JournalHandle) -> &T {
        self.boxed.as_ref(j)
    }

    pub fn into_pbox(self) -> PBox<T> {
        self.boxed
    }

    pub unsafe fn into_ptr(self) -> Ptr<T> {
        Ptr { boxed: self.boxed }
    }
}

impl<T: PSafe> PtrRW<T> {
    pub fn down_cast(self) -> PtrWLog<T> {
        PtrWLog { boxed: self.boxed }
    }

    pub fn into_pbox(self) -> PBox<T> {
        self.boxed
    }

    pub fn into_ptr(self) -> Ptr<T> {
        Ptr { boxed: self.boxed }
    }
}

impl<T: PSafe> core::ops::Deref for PtrRW<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { self.boxed.as_ref_no_journal() }
    }
}

impl<T: PSafe> core::ops::DerefMut for PtrRW<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.boxed.as_mut_no_logging() }
    }
}

pub struct PRef<'a, T: 'a> {
    inner: &'a mut T,
}

pub struct PRefWLog<'a, T: 'a> {
    inner: &'a mut T,
}

pub struct PRefRW<'a, T: 'a> {
    inner: &'a mut T,
}

impl<'a, T> PRef<'a, T> {
    pub unsafe fn new(p: *mut T) -> PRef<'a, T> {
        Self { inner: &mut *p }
    }

    pub fn read<R, F>(self, f: F, _j: JournalHandle) -> (R, PRefWLog<'a, T>)
    where
        F: FnOnce(&T) -> R,
    {
        let r2 = PRefWLog { inner: self.inner };

        let r1 = f(r2.inner);

        (r1, r2)
    }

    pub fn into_readable(self) -> PRefWLog<'a, T> {
        PRefWLog { inner: self.inner }
    }

    pub fn write(mut self, data: T, _j: JournalHandle) -> PRefRW<'a, T> {
        *self.inner = data;
        let r = PRefRW { inner: self.inner };
        r
    }

    pub fn partial_write<Selector, DT>(&self, selector: Selector, field_data: DT, _j: JournalHandle)
    where
        Selector: FnOnce(&T) -> &DT,
    {
        let field_ptr = unsafe { selector(&self.inner) as *const DT as *mut DT };
        unsafe { field_ptr.write(field_data) }
    }

    pub fn read_wr<R, F>(mut self, f: F, j: JournalHandle) -> (R, PRefRW<'a, T>)
    where
        F: FnOnce(&mut T) -> R,
    {
        // log
        j.get_mut().append_log_of(self.inner as *mut T);
        let r1 = f(&mut self.inner);
        let r2 = PRefRW { inner: self.inner };
        (r1, r2)
    }

    pub fn as_pm_ptr(&self) -> PMPtr<T> {
        unsafe { PMPtr::from_ref(&self.inner) }
    }

    pub unsafe fn into_pref_rw(self) -> PRefRW<'a, T> {
        PRefRW { inner: self.inner }
    }
}

impl<'a, T> PRefWLog<'a, T> {
    pub fn read<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        f(self.inner)
    }

    pub fn as_ref(&self, _j: JournalHandle) -> &T {
        &self.inner
    }

    pub fn as_mut(&mut self, j: JournalHandle) -> &mut T {
        //log
        j.get_mut().append_log_of(self.inner as *mut T);
        &mut self.inner
    }

    pub fn write<F>(mut self, f: F, j: JournalHandle) -> PRefRW<'a, T>
    where
        F: FnOnce(&mut T),
    {
        //log
        j.get_mut().append_log_of(self.inner as *mut T);
        f(&mut self.inner);
        let r = PRefRW { inner: self.inner };
        r
    }

    pub fn into_pref_rw(self, j: JournalHandle) -> PRefRW<'a, T> {
        j.get_mut().append_log_of(self.inner as *mut T);
        let r = PRefRW { inner: self.inner };
        r
    }

    pub fn read_wr<R, F>(mut self, f: F, j: JournalHandle) -> (R, PRefRW<'a, T>)
    where
        F: FnOnce(&mut T) -> R,
    {
        // log
        j.get_mut().append_log_of(self.inner as *mut T);
        let r1 = f(&mut self.inner);
        let r2 = PRefRW { inner: self.inner };
        (r1, r2)
    }
}

impl<'a, T> PRefRW<'a, T> {
    pub fn as_ref(&self) -> &T {
        &self.inner
    }

    pub fn as_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    pub fn as_pm_ptr(&self) -> PMPtr<T> {
        unsafe { PMPtr::from_ref(&self.inner) }
    }
}

impl<'a, T> core::ops::Deref for PRefRW<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.inner
    }
}

impl<'a, T> core::ops::DerefMut for PRefRW<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.inner }
    }
}
