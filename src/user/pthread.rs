use crate::marker::PSafe;
use crate::syscalls::{self as sys, SyscallToken};
use crate::task::TaskHandle;

use super::pbox::RelaxedPBox;

fn closure_runner<F, T>(mut pboxed: RelaxedPBox<F>)
where
    F: FnMut() -> T + Send + 'static,
{
    unsafe {
        (pboxed.as_mut_no_logging())();
    }
    drop(pboxed);
    loop {}
}

pub fn create<F, T>(
    name: &'static str,
    prio: usize,
    t: SyscallToken,
    f: F,
) -> Result<TaskHandle, crate::task::ErrorCode>
where
    F: FnMut() -> T + Send + 'static,
{
    let pboxed = RelaxedPBox::new(f, t);
    sys::sys_create_task::<RelaxedPBox<F>>(name, prio, closure_runner::<F, T>, pboxed, t)
}
