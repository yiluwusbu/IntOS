use crate::marker::{PSafe, TxInSafe};
use crate::pmem::{JournalHandle, PMPtr};
use crate::syscalls::{sys_palloc_array, sys_pfree_array, SyscallToken};
use core::ops::{Deref, Index};
use core::ptr::NonNull;
use core::{marker::PhantomData, mem::size_of};

use crate::{debug_print, syscalls as sys};

use super::pbox::{PBox, PRef};
use super::{transaction, AllocError};

pub struct PVec<T: PSafe> {
    boxed_buf: PBox<RawPVec<T>>,
}

struct RawPVec<T: PSafe> {
    ptr: PMPtr<T>,
    cap: usize,
    len: usize,
}

unsafe impl<T: Send + PSafe> Send for RawPVec<T> {}
unsafe impl<T: Sync + PSafe> Sync for RawPVec<T> {}

impl<T: PSafe> RawPVec<T> {
    pub fn try_new_empty(cap: usize, t: SyscallToken) -> Result<Self, AllocError> {
        assert!(cap != 0);
        let ptr = unsafe { sys::sys_palloc_array(cap, t) };

        let ptr = match ptr {
            None => return Err(AllocError),
            Some(p) => unsafe { PMPtr::new(p.as_ptr()) },
        };

        let v = Self { ptr, cap, len: 0 };
        Ok(v)
    }

    pub fn try_new<const N: usize>(x: [T; N], t: SyscallToken) -> Result<Self, AllocError> {
        let ptr = unsafe { sys::sys_palloc(x, t) };

        let ptr = match ptr {
            None => return Err(AllocError),
            Some(p) => unsafe { PMPtr::new(p.as_ptr() as *mut T) },
        };

        let v = Self {
            ptr,
            cap: N,
            len: N,
        };
        Ok(v)
    }

    pub fn new<const N: usize>(x: [T; N], t: SyscallToken) -> Self {
        Self::try_new(x, t).unwrap()
    }

    pub fn new_empty(cap: usize, t: SyscallToken) -> Self {
        Self::try_new_empty(cap, t).unwrap()
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn grow(&self) {
        panic!("Length of PVec is fixed currently");
    }

    pub unsafe fn init_element(&mut self, idx: usize, elem: T) {
        core::ptr::write(self.ptr.as_ptr().add(idx), elem);
    }
}

impl<T: PSafe> PVec<T> {
    pub fn try_new_empty(cap: usize, t: SyscallToken) -> Result<Self, AllocError> {
        match RawPVec::<T>::try_new_empty(cap, t) {
            Ok(buf) => {
                let ptr = buf.ptr.as_ptr();
                let cap = buf.cap;
                let res = PBox::try_new(buf, t);
                match res {
                    Err(_) => {
                        unsafe {
                            sys_pfree_array(NonNull::new_unchecked(ptr), cap, t);
                        }
                        return Err(AllocError);
                    }

                    Ok(boxed) => return Ok(Self { boxed_buf: boxed }),
                }
            }
            Err(e) => Err(e),
        }
    }

    pub fn try_new<const N: usize>(x: [T; N], t: SyscallToken) -> Result<Self, AllocError> {
        match RawPVec::try_new(x, t) {
            Ok(buf) => {
                let ptr = buf.ptr.as_ptr();
                let cap = buf.cap;
                let res = PBox::try_new(buf, t);
                match res {
                    Err(_) => {
                        unsafe {
                            sys_pfree_array(NonNull::new_unchecked(ptr), cap, t);
                        }
                        return Err(AllocError);
                    }

                    Ok(boxed) => return Ok(Self { boxed_buf: boxed }),
                }
            }
            Err(e) => Err(e),
        }
    }

    pub fn new_with<const N: usize>(x: [T; N], t: SyscallToken) -> Self {
        let buf = RawPVec::try_new(x, t).unwrap();
        Self {
            boxed_buf: PBox::new(buf, t),
        }
    }

    pub fn new(cap: usize, t: SyscallToken) -> Self {
        let buf = RawPVec::try_new_empty(cap, t).unwrap();
        Self {
            boxed_buf: PBox::new(buf, t),
        }
    }

    // fn raw_vec_mut(&self, t: SyscallToken) -> &mut RawPVec<T> {
    //     let mut_ref =  unsafe { &mut *(self as * const Self as * mut Self) };
    //     mut_ref.boxed_buf.as_mut(j)
    // }

    fn raw_vec_mut(&self, j: JournalHandle) -> &mut RawPVec<T> {
        self.boxed_buf.as_mut(j)
    }

    fn raw_vec(&self) -> &RawPVec<T> {
        unsafe { self.boxed_buf.as_ref_no_journal() }
    }

    fn ptr(&self) -> *mut T {
        unsafe { self.boxed_buf.as_ref_no_journal().ptr.as_ptr() }
    }

    pub fn clear(&self, j: JournalHandle) {
        let buf = self.raw_vec_mut(j);
        buf.len = 0;
    }

    pub fn push(&self, elem: T, j: JournalHandle) {
        let buf = self.raw_vec_mut(j);
        if buf.len == buf.cap {
            buf.grow();
        }
        unsafe {
            core::ptr::write(buf.ptr.as_ptr().add(buf.len), elem);
        }
        buf.len += 1;
    }

    pub fn is_full(&self, _j: JournalHandle) -> bool {
        self.raw_vec().len == self.raw_vec().cap
    }

    pub fn len(&self, _j: JournalHandle) -> usize {
        self.raw_vec().len
    }

    pub fn pop(&mut self, j: JournalHandle) -> Option<T> {
        if self.raw_vec().len == 0 {
            return None;
        }
        let buf = self.raw_vec_mut(j);
        buf.len -= 1;
        unsafe { Some(core::ptr::read(buf.ptr.as_ptr().add(buf.len))) }
    }

    pub fn index_mut(&self, idx: usize, j: JournalHandle) -> &mut T {
        let buf = self.raw_vec();
        assert!(idx < buf.len);
        let ptr = unsafe { self.ptr().add(idx) };
        j.get_mut().append_log_of(ptr);
        unsafe { &mut *ptr }
    }

    pub fn drain(&mut self, j: JournalHandle) -> Drain<T> {
        let iter = unsafe { RawValIter::new(&self) };
        let buf = self.raw_vec_mut(j);
        buf.len = 0;
        Drain {
            vec: PhantomData,
            iter,
        }
    }
}

impl<T: PSafe> Drop for RawPVec<T> {
    fn drop(&mut self) {
        // for i in 0..self.len {
        //     let obj = unsafe {
        //         core::ptr::read(self.ptr.as_ptr().add(i))
        //     };
        //     drop(obj);
        // }
        // transaction::run(|j| {
        //     unsafe {
        //         sys::sys_pfree(self.ptr, t);
        //     }
        // });
    }
}

impl<T: PSafe> Deref for PVec<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        unsafe { core::slice::from_raw_parts(self.ptr(), self.raw_vec().len) }
    }
}

impl<T: PSafe> IntoIterator for PVec<T> {
    type IntoIter = IntoIter<T>;
    type Item = T;
    fn into_iter(self) -> Self::IntoIter {
        unsafe {
            let iter = RawValIter::new(&self);
            Self::IntoIter {
                _boxed_buf: self.boxed_buf,
                iter,
            }
        }
    }
}

struct RawValIter<T: PSafe> {
    start: *const T,
    end: *const T,
}

impl<T: PSafe> RawValIter<T> {
    unsafe fn new(slice: &[T]) -> Self {
        RawValIter {
            start: slice.as_ptr(),
            end: slice.as_ptr().add(slice.len()),
        }
    }
}

impl<T: PSafe> Iterator for RawValIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        if self.start == self.end {
            None
        } else {
            unsafe {
                let old_ptr = self.start;
                self.start = self.start.offset(1);
                Some(core::ptr::read(old_ptr))
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let elem_size = core::mem::size_of::<T>();
        let len =
            (self.end as usize - self.start as usize) / if elem_size == 0 { 1 } else { elem_size };
        (len, Some(len))
    }
}

impl<T: PSafe> DoubleEndedIterator for RawValIter<T> {
    fn next_back(&mut self) -> Option<T> {
        if self.start == self.end {
            None
        } else {
            unsafe {
                self.end = self.end.offset(-1);
                Some(core::ptr::read(self.end))
            }
        }
    }
}

pub struct IntoIter<T: PSafe> {
    _boxed_buf: PBox<RawPVec<T>>,
    iter: RawValIter<T>,
}

impl<T: PSafe> Iterator for IntoIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.iter.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<T: PSafe> DoubleEndedIterator for IntoIter<T> {
    fn next_back(&mut self) -> Option<T> {
        self.iter.next_back()
    }
}

impl<T: PSafe> Drop for IntoIter<T> {
    fn drop(&mut self) {
        for _ in &mut *self {}
    }
}

pub struct Drain<'a, T: 'a + PSafe> {
    vec: PhantomData<&'a mut PVec<T>>,
    iter: RawValIter<T>,
}

impl<'a, T: PSafe> Iterator for Drain<'a, T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.iter.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a, T: PSafe> DoubleEndedIterator for Drain<'a, T> {
    fn next_back(&mut self) -> Option<T> {
        self.iter.next_back()
    }
}

impl<'a, T: PSafe> Drop for Drain<'a, T> {
    fn drop(&mut self) {
        // pre-drain the iter
        for _ in &mut *self {}
    }
}

unsafe impl<T: PSafe> PSafe for PArray<T> {}

unsafe impl<T: PSafe> Send for PArray<T> {}

pub struct PArray<T: PSafe> {
    ptr: NonNull<T>,
    size: usize,
}

impl<'a, T: PSafe + Default + 'a> PArray<T> {
    pub fn new(size: usize, t: SyscallToken) -> Option<Self> {
        let ptr = unsafe { sys_palloc_array::<T>(size, t) };
        match ptr {
            Some(p) => {
                for i in 0..size {
                    unsafe {
                        let ptr_of_i = p.as_ptr().add(i);
                        core::ptr::write(ptr_of_i, T::default());
                    }
                }
                Some(Self { ptr: p, size })
            }
            None => None,
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn index(&self, index: usize) -> PRef<'a, T> {
        let ptr_of_idx = unsafe { self.ptr.as_ptr().add(index) };
        unsafe { PRef::new(ptr_of_idx) }
    }
}

impl<T: PSafe> Drop for PArray<T> {
    fn drop(&mut self) {
        // call the deallocation function
        debug_print!("Freeing the PArray");
        // transaction::run(|j| {
        //     unsafe {
        //         sys_pfree_array(self.ptr, self.size, t);
        //     }
        // });
    }
}
