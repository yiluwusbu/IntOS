#[cfg(board = "apollo4bp")]
pub mod apollo4bp;
#[cfg(board = "msp430fr5994")]
pub mod msp430fr5994;
#[cfg(board = "qemu")]
pub mod qemu;

#[macro_export]
macro_rules! set_bit_field {
    ($reg: ident, $field: ident, $val: expr) => {
        let mask: u32 = ((1 << (field.1)) - 1) << field.0;
        $reg.modify(|v| v | (($val << field.0) & mask));
    };
}

#[macro_export]
macro_rules! get_bit_field {
    ($reg: ident, $field: ident, $val: expr) => {
        let mask: u32 = ((1 << (field.1)) - 1) << field.0;
        ($reg.read() & mask) >> field.0
    };
}

#[macro_export]
macro_rules! val_to_bit_field {
    ($field: ident, $val: expr) => {
        (($val << $field.0) & (((1 << ($field.1)) - 1) << $field.0))
    };
}

#[macro_export]
macro_rules! bit_field_to_mask {
    ($field: ident) => {
        ((1 << ($field.1)) - 1) << $field.0
    };
}

#[cfg(not(test))]
#[no_mangle]
pub fn board_init() {
    #[cfg(board = "apollo4bp")]
    apollo4bp::init::am_hw_init();
    #[cfg(board = "qemu")]
    ();
    #[cfg(board = "msp430fr5994")]
    msp430fr5994::init();
}

#[cfg(test)]
pub fn board_init() {}

#[cfg(not(test))]
pub fn hprintln(args: core::fmt::Arguments) {
    #[cfg(board = "apollo4bp")]
    apollo4bp::am_hprintln(args);
    #[cfg(board = "qemu")]
    qemu::qemu_hprintln(args);
    #[cfg(board = "msp430fr5994")]
    msp430fr5994::print::msp430_hprintln(args);
}

#[cfg(test)]
pub fn hprintln(args: core::fmt::Arguments) {
    println!("{}", args)
}

#[cfg(not(test))]
pub fn hprint(args: core::fmt::Arguments) {
    #[cfg(board = "apollo4bp")]
    apollo4bp::am_hprint(args);
    #[cfg(board = "qemu")]
    qemu::qemu_hprint(args);
    #[cfg(board = "msp430fr5994")]
    msp430fr5994::print::msp430_hprint(args);
}

#[cfg(test)]
pub fn hprint(args: core::fmt::Arguments) {
    print!("{}", args)
}

#[macro_export]
macro_rules! board_hprintln {
    ($e: expr) => {
        crate::board::hprintln(core::format_args!($e));
    };
    ($e: expr, $($tt: tt)*) => {
        crate::board::hprintln(core::format_args!($e, $($tt)*));
    };
}

#[macro_export]
macro_rules! board_hprint {
    ($e: expr) => {
        crate::board::hprint(core::format_args!($e));
    };
    ($e: expr, $($tt: tt)*) => {
        crate::board::hprint(core::format_args!($e, $($tt)*));
    };
}

#[cfg(board = "apollo4bp")]
#[macro_export]
macro_rules! board {
    ($var: ident) => {
        crate::board::apollo4bp::$var
    };
}

#[cfg(board = "qemu")]
#[macro_export]
macro_rules! board {
    ($var: ident) => {
        crate::board::qemu::$var
    };
}

#[cfg(board = "msp430fr5994")]
#[macro_export]
macro_rules! board {
    ($var: ident) => {
        crate::board::msp430fr5994::$var
    };
}

#[cfg(board = "apollo4bp")]
pub const HEAP_SIZE: usize = apollo4bp::HEAP_SIZE;
#[cfg(board = "apollo4bp")]
pub const PM_HEAP_SIZE_PER_TASK: usize = apollo4bp::PM_HEAP_SIZE_PER_TASK;
#[cfg(board = "apollo4bp")]
pub const BOOT_PM_HEAP_SIZE: usize = apollo4bp::BOOT_PM_HEAP_SIZE;
#[cfg(board = "apollo4bp")]
pub const PM_JOURNAL_SIZE: usize = apollo4bp::PM_JOURNAL_SIZE;
#[cfg(board = "apollo4bp")]
pub const STACK_SIZE: usize = apollo4bp::STACK_SIZE;
#[cfg(board = "apollo4bp")]
pub const TASK_NUM_LIMIT: usize = apollo4bp::TASK_NUM_LIMIT;

#[cfg(board = "qemu")]
pub const HEAP_SIZE: usize = qemu::HEAP_SIZE;
#[cfg(board = "qemu")]
pub const PM_HEAP_SIZE_PER_TASK: usize = qemu::PM_HEAP_SIZE_PER_TASK;
#[cfg(board = "qemu")]
pub const BOOT_PM_HEAP_SIZE: usize = qemu::BOOT_PM_HEAP_SIZE;
#[cfg(board = "qemu")]
pub const PM_JOURNAL_SIZE: usize = qemu::PM_JOURNAL_SIZE;
#[cfg(board = "qemu")]
pub const STACK_SIZE: usize = qemu::STACK_SIZE;
#[cfg(board = "qemu")]
pub const TASK_NUM_LIMIT: usize = qemu::TASK_NUM_LIMIT;

#[cfg(board = "msp430fr5994")]
pub const HEAP_SIZE: usize = msp430fr5994::HEAP_SIZE;
#[cfg(board = "msp430fr5994")]
pub const PM_HEAP_SIZE_PER_TASK: usize = msp430fr5994::PM_HEAP_SIZE_PER_TASK;
#[cfg(board = "msp430fr5994")]
pub const BOOT_PM_HEAP_SIZE: usize = msp430fr5994::BOOT_PM_HEAP_SIZE;
#[cfg(board = "msp430fr5994")]
pub const PM_JOURNAL_SIZE: usize = msp430fr5994::PM_JOURNAL_SIZE;
#[cfg(board = "msp430fr5994")]
pub const STACK_SIZE: usize = msp430fr5994::STACK_SIZE;
#[cfg(board = "msp430fr5994")]
pub const TASK_NUM_LIMIT: usize = msp430fr5994::TASK_NUM_LIMIT;

#[cfg(board = "test")]
pub const HEAP_SIZE: usize = 512;
#[cfg(board = "test")]
pub const PM_HEAP_SIZE_PER_TASK: usize = 4096;
#[cfg(board = "test")]
pub const BOOT_PM_HEAP_SIZE: usize = 1024;
#[cfg(board = "test")]
pub const PM_JOURNAL_SIZE: usize = 512;
#[cfg(board = "test")]
pub const STACK_SIZE: usize = 512;
#[cfg(board = "test")]
pub const TASK_NUM_LIMIT: usize = 8;
