use core::{cell::UnsafeCell, marker::PhantomData, ptr::NonNull};

pub unsafe auto trait PSafe {}

impl<T: ?Sized> !PSafe for *const T {}
impl<T: ?Sized> !PSafe for *mut T {}
impl<T> !PSafe for &T {}
impl<T> !PSafe for &mut T {}
unsafe impl PSafe for &str {}
impl<T: ?Sized> !PSafe for UnsafeCell<T> {}

pub unsafe auto trait TxOutSafe {}

pub unsafe auto trait TxRefInSafe {}

// impl<T: ?Sized> !TxOutSafe for *const T {}
impl<T: ?Sized> !TxOutSafe for *mut T {}
impl<T: ?Sized> !TxOutSafe for &mut T {}
impl<T: ?Sized> !TxOutSafe for UnsafeCell<T> {}

pub unsafe auto trait TxInSafe {}

impl<T: ?Sized> !TxInSafe for *mut T {}
impl<T: ?Sized> !TxInSafe for &mut T {}
impl<T: ?Sized> !TxInSafe for UnsafeCell<T> {}

unsafe impl<T: ?Sized + TxRefInSafe> TxInSafe for &T {}
unsafe impl<T: ?Sized> TxRefInSafe for crate::pmem::PMPtr<T> {}
unsafe impl<T: ?Sized> TxRefInSafe for NonNull<T> {}
unsafe impl<T: ?Sized> TxRefInSafe for PhantomData<T> {}
