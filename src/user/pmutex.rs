use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::Deref;
use core::ptr::NonNull;

use crate::marker::{PSafe, TxInSafe, TxOutSafe, TxRefInSafe};
use crate::pmem::JournalHandle;
use crate::syscalls::SyscallToken;
use crate::task::ErrorCode;
use crate::time::Time;
use crate::{debug_print, syscalls as sys};

use super::transaction;

const DEFAULT_MUTEX_WAIT_TIME: Time = (Time::MAX) >> 2;

pub struct PMutex<T> {
    inner: UnsafeCell<T>,
    semaphore_handle: sys::SemaphoreHandle,
}

unsafe impl<T: PSafe> PSafe for PMutex<T> {}
impl<T> !TxRefInSafe for PMutex<T> {}
impl<T> !TxInSafe for PMutex<T> {}

// unsafe impl<T> TxInSafe for PMutexGuard<'_, T> {}

pub struct PMutexGuard<'a, T> {
    mutex_ptr: NonNull<PMutex<T>>,
    phantom: PhantomData<&'a PMutex<T>>,
}

#[derive(Debug)]
pub enum PMutexError {
    NoMemory,
    Poisoned,
}

pub enum PMutexStatus {
    Unlocked = 0,
    Locked = 1,
    Poisoned = 2,
}

unsafe impl<T: Send> Send for PMutex<T> {}
unsafe impl<T: Send> Sync for PMutex<T> {}

impl<T> PMutex<T> {
    pub fn new(data: T, t: SyscallToken) -> Self {
        let semaphore = sys::sys_create_semaphore(1, t);
        let sem = match semaphore {
            None => {
                panic!("PMutex New OOM");
            }
            Some(s) => s,
        };
        Self {
            inner: UnsafeCell::new(data),
            semaphore_handle: sem,
        }
    }

    pub fn try_new(data: T, t: SyscallToken) -> Result<Self, PMutexError> {
        let semaphore = sys::sys_create_semaphore(1, t);
        let sem = match semaphore {
            None => {
                return Err(PMutexError::NoMemory);
            }
            Some(s) => s,
        };
        Ok(Self {
            inner: UnsafeCell::new(data),
            semaphore_handle: sem,
        })
    }

    fn sys_lock(&self) {
        while let Err(_) = sys::sys_semaphore_take(self.semaphore_handle, DEFAULT_MUTEX_WAIT_TIME) {
        }
    }

    pub fn lock(&self) -> Result<PMutexGuard<T>, PMutexError> {
        self.sys_lock();
        let guard = PMutexGuard {
            mutex_ptr: unsafe { NonNull::new_unchecked(self as *const Self as *mut Self) },
            phantom: PhantomData,
        };
        Ok(guard)
    }

    fn sys_unlock(&self) {
        sys::sys_semaphore_give(self.semaphore_handle);
    }

    pub fn unlock(&self) {
        self.sys_unlock()
    }
}

impl<T> PMutexGuard<'_, T> {
    // pub fn as_mut(&self, t: SyscallToken) -> &mut T {
    //     let mut_ref = unsafe {&mut *self.mutex.inner.get()};
    //     j.get_mut().append_log_of(mut_ref as * mut T);
    //     mut_ref
    // }
    pub fn as_mut(&self, j: JournalHandle) -> &mut T {
        let mutex = unsafe { self.mutex_ptr.as_ref() };
        let mut_ref = unsafe { &mut *mutex.inner.get() };
        j.get_mut().append_log_of(mut_ref as *mut T);
        mut_ref
    }

    pub fn as_ref(&self, j: JournalHandle) -> &T {
        let mutex = unsafe { self.mutex_ptr.as_ref() };
        let r = unsafe { &*mutex.inner.get() };
        r
    }
}

impl<T> Drop for PMutexGuard<'_, T> {
    fn drop(&mut self) {
        // debug_print!("releasing mutex");
        sys::sys_semaphore_give(unsafe { self.mutex_ptr.as_ref().semaphore_handle })
    }
}

impl<T> Deref for PMutexGuard<'_, T> {
    type Target = T;
    // fn deref(&self) -> &Self::Target {
    //     unsafe { &*self.mutex.inner.get() }
    // }

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex_ptr.as_ref().inner.get() }
    }
}
