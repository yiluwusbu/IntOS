use core::marker::PhantomData;
use core::ops::Deref;
use core::sync::atomic;
use vcell::VolatileCell;

use crate::marker::{PSafe, TxInSafe, TxOutSafe, TxRefInSafe};
use crate::pmem::{JournalHandle, PMPtr};
use crate::syscalls::SyscallToken;
use crate::user::pbox::PBox;
use crate::user::transaction;
use crate::{critical, debug_print};

use super::pmutex::PMutex;

unsafe impl<T: PSafe + TxInSafe> TxInSafe for PArc<T> {}
unsafe impl<T: PSafe> TxOutSafe for PArc<T> {}
unsafe impl<T: PSafe> TxInSafe for PArcInner<T> {}

pub struct PArc<T: PSafe> {
    ptr: PMPtr<PArcInner<T>>,
    phantom: PhantomData<PArcInner<T>>,
}

struct PArcInner<T> {
    rc: VolatileCell<usize>,
    data: T,
}

unsafe impl<T: PSafe> PSafe for PArcInner<T> {}
unsafe impl<T> TxRefInSafe for PArcInner<T> {}

unsafe impl<T: TxRefInSafe + PSafe> TxRefInSafe for PArc<T> {}

impl<T: PSafe> PArc<T> {
    pub fn new(data: T, t: SyscallToken) -> PArc<T> {
        // We start the reference count at 1, as that first reference is the
        // current pointer.
        let boxed = PBox::new(
            PArcInner {
                rc: VolatileCell::new(1),
                data,
            },
            t,
        );

        PArc {
            // It is okay to call `.unwrap()` here as we get a pointer from
            // `Box::into_raw` which is guaranteed to not be null.
            ptr: unsafe { PMPtr::new(PBox::into_raw(boxed)) },
            phantom: PhantomData,
        }
    }

    pub fn clone(&self, j: JournalHandle) -> PArc<T> {
        // Using a relaxed ordering is alright here as we don't need any atomic
        // synchronization here as we're not modifying or accessing the inner
        // data.
        // let inner = unsafe { self.ptr.as_ref() };
        // let old_rc = critical::with_no_interrupt(|cs|
        //             transaction::run(|j| {
        //                 let old = inner.rc.get();
        //                 inner.rc.set(old+1);
        //                 old
        //             }));

        // debug_print!("rc = {}", old_rc+1);
        // if old_rc >= isize::MAX as usize {
        //     panic!("PArc: ref >= isize::MAX");
        // }

        Self {
            ptr: self.ptr,
            phantom: PhantomData,
        }
    }
}

unsafe impl<T: Sync + Send + PSafe> Send for PArc<T> {}
unsafe impl<T: Sync + Send + PSafe> Sync for PArc<T> {}

impl<T: PSafe> Deref for PArc<T> {
    type Target = T;

    fn deref(&self) -> &T {
        let inner = unsafe { self.ptr.as_ref() };
        &inner.data
    }
}

impl<T: PSafe> Drop for PArc<T> {
    fn drop(&mut self) {
        // let inner = unsafe { self.ptr.as_ref() };
        // let old_cnt  = critical::with_no_interrupt(|cs|
        //         transaction::run(|j| {
        //             let old = inner.rc.get();
        //             inner.rc.set(old-1);
        //             old
        //         })
        // );

        // if old_cnt != 1 {
        //     debug_print!("ref = {}", old_cnt-1);
        //     return;
        // }
        debug_print!("Dropping PArc");
        // This fence is needed to prevent reordering of the use and deletion
        // of the data.
        // atomic::fence(atomic::Ordering::Acquire);
        // This is safe as we know we have the last pointer to the `ArcInner`
        // and that its pointer is valid.
        // unsafe { PBox::from_raw(self.ptr.as_ptr()); }
    }
}
