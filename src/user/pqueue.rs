use crate::marker::{PSafe, TxInSafe};
use crate::pmem::{JournalHandle, PMPtr};
use crate::syscalls::{sys_palloc_array, SyscallToken};
use core::ptr::NonNull;

use crate::{debug_print, syscalls as sys};

use super::pbox::{PBox, PRef};
use super::{transaction, AllocError};

pub struct PQueue<T: PSafe> {
    boxed_buf: PBox<RawPQueue<T>>,
}

struct RawPQueue<T: PSafe> {
    ptr: PMPtr<T>,
    cap: usize,
    head: usize,
    tail: usize,
    len: usize,
}

impl<T: PSafe> RawPQueue<T> {
    pub fn try_new(cap: usize, t: SyscallToken) -> Result<Self, AllocError> {
        assert!(cap != 0);
        let ptr = unsafe { sys::sys_palloc_array(cap, t) };

        let ptr = match ptr {
            None => return Err(AllocError),
            Some(p) => unsafe { PMPtr::new(p.as_ptr()) },
        };

        let v = Self {
            ptr,
            cap,
            head: 0,
            tail: 0,
            len: 0,
        };
        Ok(v)
    }

    pub fn new(cap: usize, t: SyscallToken) -> Self {
        Self::try_new(cap, t).unwrap()
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn len(&self) -> usize {
        self.len
    }
}

impl<T: PSafe> PQueue<T> {
    pub fn new(cap: usize, t: SyscallToken) -> Self {
        let buf = RawPQueue::try_new(cap, t).unwrap();
        Self {
            boxed_buf: PBox::new(buf, t),
        }
    }

    fn raw_mut_pqueue(&self, j: JournalHandle) -> &mut RawPQueue<T> {
        unsafe { self.boxed_buf.as_mut(j) }
    }

    fn raw_pqueue(&self, j: JournalHandle) -> &RawPQueue<T> {
        unsafe { self.boxed_buf.as_ref(j) }
    }

    pub fn push_back(&self, elem: T, j: JournalHandle) {
        let buf = self.raw_mut_pqueue(j);
        assert!(buf.len() < buf.cap());
        unsafe {
            core::ptr::write(buf.ptr.as_ptr().add(buf.tail), elem);
        }
        buf.tail += 1;
        if buf.tail == buf.cap {
            buf.tail = 0;
        }
        buf.len += 1;
    }

    pub fn clear(&self, j: JournalHandle) {
        let buf = self.raw_mut_pqueue(j);
        buf.tail = 0;
        buf.head = 0;
        buf.len = 0;
    }

    pub fn len(&self, j: JournalHandle) -> usize {
        self.raw_pqueue(j).len()
    }

    pub fn peek_front(&self, j: JournalHandle) -> Option<&T> {
        if self.raw_pqueue(j).len() == 0 {
            return None;
        }
        let buf = self.raw_pqueue(j);
        let r = unsafe {
            let p = buf.ptr.as_ptr().add(buf.head);
            p.as_ref()
        };
        r
        // let r = unsafe {
        //     Some(core::ptr::read(buf.ptr.as_ptr().add(buf.head)))
        // };
        // r
    }

    pub fn pop_front(&self, j: JournalHandle) -> Option<T> {
        if self.raw_pqueue(j).len() == 0 {
            return None;
        }
        let buf = self.raw_mut_pqueue(j);

        let r = unsafe { Some(core::ptr::read(buf.ptr.as_ptr().add(buf.head))) };
        buf.head += 1;
        if buf.head == buf.cap {
            buf.head = 0;
        }
        buf.len -= 1;
        r
    }
}

impl<T: PSafe> Drop for RawPQueue<T> {
    fn drop(&mut self) {}
}
