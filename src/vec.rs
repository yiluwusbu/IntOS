use crate::heap::palloc_array;
use crate::pmem::JournalHandle;
use crate::syscalls;
use core::mem::size_of;
use core::ops::Index;
use core::ptr::NonNull;

pub struct PArray<T> {
    ptr: NonNull<T>,
    size: usize,
}

impl<T> PArray<T> {
    pub fn new(size: usize, j: JournalHandle) -> Option<Self> {
        let ptr = unsafe { palloc_array(size, j) };
        match ptr {
            Some(p) => Some(Self { ptr: p, size }),
            None => None,
        }
    }

    pub unsafe fn from_ptr(ptr: *const T, size: usize) -> Self {
        Self {
            ptr: unsafe { NonNull::new_unchecked(ptr as *mut T) },
            size,
        }
    }

    pub unsafe fn as_ptr(&self) -> *mut T {
        self.ptr.as_ptr()
    }

    pub fn index_mut(&self, idx: usize, j: JournalHandle) -> &mut T {
        let ret = unsafe { &mut *self.ptr.as_ptr().add(idx) };
        j.get_mut().append_log_of(ret as *mut T);
        ret
    }

    pub fn to_byte_array(self) -> PArray<u8> {
        PArray::<u8> {
            ptr: unsafe { NonNull::new_unchecked(self.as_ptr() as *mut u8) },
            size: size_of::<T>() * self.size,
        }
    }

    pub unsafe fn as_byte_array(&self) -> PArray<u8> {
        PArray::<u8> {
            ptr: unsafe { NonNull::new_unchecked(self.as_ptr() as *mut u8) },
            size: size_of::<T>() * self.size,
        }
    }
}

impl<T> Index<usize> for PArray<T> {
    type Output = T;
    fn index(&self, idx: usize) -> &Self::Output {
        assert!(idx < self.size);
        unsafe {
            let p = self.ptr.as_ptr();
            &*p.add(idx)
        }
    }
}

// TODO: add mutable index
// TODO: implment drop

// impl<T> IndexMut<usize> for PArray<T> {
//     fn index_mut(&mut self, idx: usize) -> PMPtr<Self::Output> {
//         assert!(idx < self.size);
//         unsafe {
//             let p = self.ptr.as_ptr();
//             PMPtr::from_ref(&mut *p.add(idx))
//         }
//     }
// }
