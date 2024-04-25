use core::arch::asm;
use core::ptr;
use cortex_m_semihosting::hprintln;
use volatile_register::{RO, RW};

use crate::{
    arch,
    board::board_init,
    critical::{self, CriticalSection},
    os_print,
    recover::current_generation,
    task::{process_tick, task_switch},
};

pub(super) type Time = u32;

pub union Vector {
    reserved: u32,
    handler: unsafe extern "C" fn(),
}

// extern "C" {
//     fn NMI();
//     fn HardFault();
//     fn MemManage();
//     fn BusFault();
//     fn UsageFault();
// }

#[no_mangle]
pub extern "C" fn DefaultExceptionHandler() {
    os_print!("FAULT!");
    loop {}
}

#[cfg(not(feature = "power_failure"))]
pub const CLK_RELOAD_VALUE: u32 = crate::board!(CLK_RELOAD_VALUE);
#[cfg(feature = "power_failure")]
pub const CLK_RELOAD_VALUE: u32 = crate::board!(CLK_RELOAD_VALUE);
#[cfg(feature = "power_failure")]
pub const POWER_FAILURE_CYCLE: u32 = crate::board!(POWER_FAILURE_CYCLE);
#[cfg(feature = "power_failure")]
pub const CTX_SWITCH_CYCLE: u32 = crate::board!(CTX_SWITCH_CYCLE);

#[cfg(feature = "power_failure")]
pub const MAX_FAILURE_CNT: usize = 10000;

struct SystemTimer {
    p: &'static mut SYSTRegisterBlock,
}

#[repr(C)]
pub struct SYSTRegisterBlock {
    csr: RW<u32>,
    rvr: RW<u32>,
    cvr: RW<u32>,
    calib: RO<u32>,
}

/// SysTick clock source
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystClkSource {
    /// Core-provided clock
    Core,
    /// External reference clock
    External,
}

// const SYST_COUNTER_MASK: u32 = 0x00ff_ffff;

const SYST_CSR_ENABLE: u32 = 1 << 0;
const SYST_CSR_TICKINT: u32 = 1 << 1;
const SYST_CSR_CLKSOURCE: u32 = 1 << 2;
// const SYST_CSR_COUNTFLAG: u32 = 1 << 16;

// const SYST_CALIB_SKEW: u32 = 1 << 30;
// const SYST_CALIB_NOREF: u32 = 1 << 31;

const NVIC_PENDSV_PRIORITY: u8 = 0xff;
const NVIC_SYSTICK_PRIORITY: u8 = 0xff;

impl SystemTimer {
    #[inline(always)]
    pub fn new() -> SystemTimer {
        SystemTimer {
            p: unsafe { &mut *(0xE000_E010 as *mut SYSTRegisterBlock) },
        }
    }

    #[inline(always)]
    pub fn get_time(&self) -> u32 {
        self.p.cvr.read()
    }

    #[inline(always)]
    pub fn set_time(&mut self, count_value: u32) {
        unsafe {
            self.p.cvr.write(count_value);
        }
    }

    #[inline(always)]
    pub fn set_reload(&mut self, reload_value: u32) {
        unsafe { self.p.rvr.write(reload_value) }
    }

    #[inline(always)]
    pub fn set_config(&mut self, config_value: u32) {
        unsafe { self.p.csr.write(config_value) }
    }
}

pub fn get_sysclk_counter_elapsed() -> u32 {
    let syst = SystemTimer::new();
    CLK_RELOAD_VALUE - syst.get_time()
}

/// Register block
#[repr(C)]
pub struct SCBRegisterBlock {
    /// Interrupt Control and State
    pub icsr: RW<u32>,

    /// Vector Table Offset (not present on Cortex-M0 variants)
    pub vtor: RW<u32>,

    /// Application Interrupt and Reset Control
    pub aircr: RW<u32>,

    /// System Control
    pub scr: RW<u32>,

    /// Configuration and Control
    pub ccr: RW<u32>,

    /// System Handler Priority (word accessible only on Cortex-M0 variants)
    ///
    /// On ARMv7-M, `shpr[0]` points to SHPR1
    ///
    /// On ARMv6-M, `shpr[0]` points to SHPR2
    #[cfg(not(armv6m))]
    pub shpr: [RW<u8>; 12],
    #[cfg(armv6m)]
    _reserved1: u32,
    /// System Handler Priority (word accessible only on Cortex-M0 variants)
    ///
    /// On ARMv7-M, `shpr[0]` points to SHPR1
    ///
    /// On ARMv6-M, `shpr[0]` points to SHPR2
    #[cfg(armv6m)]
    pub shpr: [RW<u32>; 2],

    /// System Handler Control and State
    pub shcsr: RW<u32>,

    /// Configurable Fault Status (not present on Cortex-M0 variants)
    #[cfg(not(armv6m))]
    pub cfsr: RW<u32>,
    #[cfg(armv6m)]
    _reserved2: u32,

    /// HardFault Status (not present on Cortex-M0 variants)
    #[cfg(not(armv6m))]
    pub hfsr: RW<u32>,
    #[cfg(armv6m)]
    _reserved3: u32,

    /// Debug Fault Status (not present on Cortex-M0 variants)
    #[cfg(not(armv6m))]
    pub dfsr: RW<u32>,
    #[cfg(armv6m)]
    _reserved4: u32,

    /// MemManage Fault Address (not present on Cortex-M0 variants)
    #[cfg(not(armv6m))]
    pub mmfar: RW<u32>,
    #[cfg(armv6m)]
    _reserved5: u32,

    /// BusFault Address (not present on Cortex-M0 variants)
    #[cfg(not(armv6m))]
    pub bfar: RW<u32>,
    #[cfg(armv6m)]
    _reserved6: u32,

    /// Auxiliary Fault Status (not present on Cortex-M0 variants)
    #[cfg(not(armv6m))]
    pub afsr: RW<u32>,
    #[cfg(armv6m)]
    _reserved7: u32,

    _reserved8: [u32; 18],

    /// Coprocessor Access Control (not present on Cortex-M0 variants)
    #[cfg(not(armv6m))]
    pub cpacr: RW<u32>,
    #[cfg(armv6m)]
    _reserved9: u32,
}

const ICSR_PENDSV_SET_BIT: u32 = 1 << 28;

pub struct SCB {
    p: &'static mut SCBRegisterBlock,
}

impl SCB {
    #[inline(always)]
    pub fn new() -> SCB {
        SCB {
            p: unsafe { &mut *(0xE000_ED04 as *mut SCBRegisterBlock) },
        }
    }
}

#[repr(C)]
pub struct FPURegisterBlock {
    /// Floating Point Context Control
    pub fpccr: RW<u32>,
    /// Floating Point Context Address
    pub fpcar: RW<u32>,
    /// Floating Point Default Status Control
    pub fpdscr: RW<u32>,
}

pub struct FPUControl {
    p: &'static mut FPURegisterBlock,
}

impl FPUControl {
    #[inline(always)]
    pub fn new() -> FPUControl {
        Self {
            p: unsafe { &mut *(0xE000_EF34 as *mut FPURegisterBlock) },
        }
    }
}

const ASPEN_AND_LSPEN_BITS: u32 = (0x3 << 30);

const DWT_CONTROL_NOCYCLECNT_BIT: u32 = (0x1 << 25);
const DWT_CONTROL_CYCLECNT_ENABLE_BIT: u32 = 0x1;

#[repr(C)]
pub struct DWTRegisterBlock {
    pub dwt_control: RW<u32>,
    pub dwt_cyccnt: RW<u32>,
}

pub struct DWT {
    p: &'static mut DWTRegisterBlock,
}

impl DWT {
    #[inline(always)]
    pub fn new() -> DWT {
        Self {
            p: unsafe { &mut *(0xE000_1000 as *mut DWTRegisterBlock) },
        }
    }
}

pub fn is_cycle_cnt_supported() -> bool {
    let dwt = DWT::new();
    let ctrl = dwt.p.dwt_control.read();
    return (ctrl & DWT_CONTROL_NOCYCLECNT_BIT) == 0;
}

#[inline(always)]
pub fn get_cycle_cnt() -> u32 {
    let dwt = DWT::new();
    let cycle_cnt = dwt.p.dwt_cyccnt.read();
    cycle_cnt
}

pub fn enable_cycle_cnt() {
    let dwt = DWT::new();
    unsafe {
        dwt.p.dwt_cyccnt.write(0);
        dwt.p
            .dwt_control
            .modify(|v| v | DWT_CONTROL_CYCLECNT_ENABLE_BIT);
    }
}

pub fn is_cycle_cnt_enabled() -> bool {
    let dwt = DWT::new();
    unsafe { (dwt.p.dwt_control.read() & DWT_CONTROL_CYCLECNT_ENABLE_BIT) != 0 }
}

#[inline(always)]
pub fn get_system_clock_cycles() -> u32 {
    crate::time::get_time() * CLK_RELOAD_VALUE + get_sysclk_counter_elapsed()
}

#[no_mangle]
unsafe fn init_ram() {
    // Initialize RAM
    extern "C" {
        static mut _sbss: u8;
        static mut _ebss: u8;

        static mut _sdata: u8;
        static mut _edata: u8;
        static _sidata: u8;

        static mut _spmem: u8;
        static mut _epmem: u8;
        static _sipmem: u8;

        // static mut _smagic: u8;
        // static mut _emagic: u8;
        // static _simagic: u8;
    }

    let count = &_ebss as *const u8 as usize - &_sbss as *const u8 as usize;
    ptr::write_bytes(&mut _sbss as *mut u8, 0, count);

    let count = &_edata as *const u8 as usize - &_sdata as *const u8 as usize;
    ptr::copy_nonoverlapping(&_sidata as *const u8, &mut _sdata as *mut u8, count);

    // This can be avoided
    let count = &_epmem as *const u8 as usize - &_spmem as *const u8 as usize;
    ptr::copy_nonoverlapping(&_sipmem as *const u8, &mut _spmem as *mut u8, count);

    // let count = &_emagic as *const u8 as usize - &_smagic  as *const u8 as usize;
    // ptr::copy_nonoverlapping(&_simagic  as *const u8, &mut _smagic  as *mut u8, count);
}

fn config_timer() {
    let mut syst = SystemTimer::new();
    syst.set_config(0);
    syst.set_time(0);
    syst.set_reload(CLK_RELOAD_VALUE);
    syst.set_config(SYST_CSR_ENABLE | SYST_CSR_TICKINT | SYST_CSR_CLKSOURCE);
}
#[no_mangle]
pub fn start_kernel() {
    // initialize timer
    #[cfg(not(feature = "power_failure"))]
    config_timer();
    #[cfg(feature = "power_failure")]
    {
        if current_generation() == 1 {
            config_timer();
        }
    }
    config_fpu();
    // start first task
    let scb = SCB::new();
    unsafe {
        scb.p.shpr[10].write(NVIC_PENDSV_PRIORITY);
        scb.p.shpr[11].write(NVIC_SYSTICK_PRIORITY);

        // crate::task::switch_to_task_update_stats(crate::task::current());

        #[cfg(feature = "power_failure")]
        {
            if current_generation() == 1 {
                start_first_task();
            } else {
                start_first_task_after_power_failure();
            }
        }
        #[cfg(not(feature = "power_failure"))]
        start_first_task();
        // should never return
        task_exit_error();
        task_switch();
    }
}

#[inline(always)]
fn enable_fpu() {
    let scb = SCB::new();
    unsafe {
        scb.p.cpacr.modify(|v| v | (0xf << 20));
        asm!("dsb", "isb")
    }
}

#[inline(always)]
fn enable_lazy_save() {
    let fpc = FPUControl::new();
    unsafe {
        fpc.p.fpccr.modify(|v| v | ASPEN_AND_LSPEN_BITS);
    }
}

#[cfg(armv7em)]
#[inline(always)]
fn config_fpu() {
    enable_fpu();
    enable_lazy_save();
}

#[cfg(not(armv7em))]
#[inline(always)]
fn config_fpu() {}

#[no_mangle]
pub unsafe extern "C" fn __Reset() {
    extern "Rust" {
        fn init_ram();
        fn recover_and_boot();
    }
    board_init();
    init_ram();
    os_print!("RAM initialization finished, starting main function");
    recover_and_boot();
}

#[no_mangle]
#[naked]
pub unsafe extern "C" fn Reset() {
    #[cfg(not(idem))]
    asm!("bl __Reset", options(noreturn));
    #[cfg(idem)]
    asm!("bl _init_store_ptrs", "bl __Reset", options(noreturn));
}

#[no_mangle]
pub unsafe extern "C" fn PowerReset() {
    extern "Rust" {
        fn recover_and_boot();
    }
    #[cfg(feature = "debug_power_failure")]
    os_print!("Rebooting...");
    recover_and_boot();
}

const INITIAL_XPSR: u32 = 0x0100_0000;
const INITIAL_EXC_RETURN: u32 = 0xffff_fffd;
const FUNC_ADDRESS_MASK: u32 = 0xffff_fffe;

#[cfg(armv7m)]
pub unsafe fn initialize_stack(stack_top: usize, task_code: usize, param: usize) -> usize {
    unsafe {
        let mut stack_top = stack_top as *mut u32;
        // first entry is for double-word alignment
        *stack_top.offset(-1) = INITIAL_XPSR; // xPSR
        *stack_top.offset(-2) = (task_code as u32) & FUNC_ADDRESS_MASK; // pc
        *stack_top.offset(-3) = task_exit_error as *const () as u32; // lr

        // R12 R3, R2, R1, R0
        *stack_top.offset(-8) = param as u32;
        //  exec_return value for exection return
        // *stack_top.offset(-9) = INITIAL_EXC_RETURN;
        // R11, R10, R9, R8, R7, R6, R5 and R4.
        stack_top = stack_top.offset(-16);
        return stack_top as usize;
    }
}

#[cfg(armv7em)]
pub unsafe fn initialize_stack(stack_top: usize, task_code: usize, param: usize) -> usize {
    unsafe {
        let mut stack_top = stack_top as *mut u32;
        // first entry is for double-word alignment
        *stack_top.offset(-1) = INITIAL_XPSR; // xPSR
        *stack_top.offset(-2) = (task_code as u32) & FUNC_ADDRESS_MASK; // pc
        *stack_top.offset(-3) = task_exit_error as *const () as u32; // lr

        // R12 R3, R2, R1, R0
        *stack_top.offset(-8) = param as u32;
        // // exec_return value for exection return
        *stack_top.offset(-9) = INITIAL_EXC_RETURN;
        // R11, R10, R9, R8, R7, R6, R5 and R4.
        stack_top = stack_top.offset(-17);
        return stack_top as usize;
    }
}

#[no_mangle]
unsafe fn task_exit_error() {
    os_print!("Fatal: task should never exit!");
    loop {}
}

unsafe fn start_first_task() {
    unsafe {
        asm!(
            "ldr r0, =0xE000ED08 ",
            "ldr r0, [r0]        ",
            "ldr r0, [r0]        ",
            "msr msp, r0         ", // Reset MSP
            "mov r0, #0",
            "msr control, r0",
            "cpsie i",
            "cpsie f",
            "dsb",
            "isb",
            "svc 0",
            "nop",
        )
    }
}
#[cfg(feature = "power_failure")]
#[cfg(armv7em)]
unsafe fn start_first_task_after_power_failure() {
    unsafe {
        asm!(
            "   ldr r0, =0xE000ED08 ",
            "   ldr r0, [r0]        ",
            "   ldr r0, [r0]        ",
            "   msr msp, r0         ", // Reset MSP
            "   mov r0, #0",
            "   msr control, r0",
            "   cpsie i",
            "   cpsie f",
            "   dsb",
            "   isb",
            "   ldr r3, =CURRENT_TASK_PTR           ", // Get the location of the current TCB, may be different
            "   ldr r2, [r3]                        ", // The first item in CurrentTCB is the task top of stack.
            "   ldr r0, [r2]                        ",
            "                                       ",
            "   ldmia r0!, {{r4-r11, r14}}          ", // Pop the core registers.
            "                                       ",
            "                                       ",
            "   msr psp, r0                         ",
            "   isb                                 ",
            "                                       ",
            "   bx r14                              ",
        )
    }
}

#[cfg(armv7m)]
#[naked]
#[no_mangle]
pub unsafe extern "C" fn PendSV() {
    unsafe {
        asm!(
            "   mrs r0, psp                         ",
            "   isb                                 ",
            "                                       ",
            "   ldr r3, =CURRENT_TASK_PTR           ", // Get the location of the current TCB
            "   ldr r2, [r3]                        ",
            "                                       ",
            "   stmdb r0!, {{r4-r11}}               ", // Save the core registers.
            "   str r0, [r2]                        ", // Save the new top of stack into the first member of the TCB.
            "   stmdb sp!, {{r3, r14}}              ",
            "                                       ",
            "   mov r0, #0xff                       ",
            "   msr basepri, r0                     ",
            "   dsb                                 ",
            "   isb                                 ",
            "   bl task_switch                      ",
            "   mov r0, #0                          ",
            "   msr basepri, r0                     ",
            "   ldmia sp!, {{r3, r14}}              ",
            "                                       ",
            "   ldr r2, [r3]                        ", // The first item in CurrentTCB is the task top of stack.
            "   ldr r0, [r2]                        ",
            "                                       ",
            "   ldmia r0!, {{r4-r11}}               ", // Pop the core registers.
            "                                       ",
            "                                       ",
            "   msr psp, r0                         ",
            // "ldr r0, =0xE000ED08 ",
            // "ldr r0, [r0]        ",
            // "ldr r0, [r0]        ",
            // "msr msp, r0         ", // Reset MSP
            "   isb                                 ",
            "                                       ",
            "                                       ",
            "   bx r14                              ",
            options(noreturn)
        )
    }
}

#[cfg(armv7em)]
#[naked]
#[no_mangle]
pub unsafe extern "C" fn PendSV() {
    unsafe {
        asm!(
            "   mrs r0, psp                         ",
            "   isb                                 ",
            "                                       ",
            "   ldr r3, =CURRENT_TASK_PTR           ", // Get the location of the current TCB
            "   ldr r2, [r3]                        ",
            "                                       ",
            "   tst r14, #0x10                      ", // Is the task using the FPU context?  If so, push high vfp registers.
            "   it eq                               ",
            "   vstmdbeq r0!, {{s16-s31}}           ",
            "                                       ",
            "   stmdb r0!, {{r4-r11, r14}}               ", // Save the core registers.
            "   str r0, [r2]                        ", // Save the new top of stack into the first member of the TCB.
            "   stmdb sp!, {{r0, r3}}               ",
            "                                       ",
            "   mov r0, #0xff                       ",
            "   msr basepri, r0                     ",
            "   dsb                                 ",
            "   isb                                 ",
            "   bl task_switch                      ",
            "   mov r0, #0                          ",
            "   msr basepri, r0                     ",
            "   ldmia sp!, {{r0, r3}}               ",
            "                                       ",
            "   ldr r2, [r3]                        ", // The first item in CurrentTCB is the task top of stack.
            "   ldr r0, [r2]                        ",
            "                                       ",
            "   ldmia r0!, {{r4-r11, r14}}          ", // Pop the core registers.
            "   tst r14, #0x10                      ", // Is the task using the FPU context?  If so, pop the high vfp registers too.
            "   it eq                               ",
            "   vldmiaeq r0!, {{s16-s31}}           ",
            "                                       ",
            "                                       ",
            "   msr psp, r0                         ",
            "   isb                                 ",
            "                                       ",
            "                                       ",
            "   bx r14                              ",
            options(noreturn)
        )
    }
}

// Used to start the first task
#[cfg(armv7m)]
#[no_mangle]
pub unsafe extern "C" fn SVCall() {
    unsafe {
        asm!(
            "   ldr r3, =CURRENT_TASK_PTR           ", // Get the location of the current TCB, may be different
            "   ldr r2, [r3]                        ", // The first item in CurrentTCB is the task top of stack.
            "   ldr r0, [r2]                        ",
            "                                       ",
            "   ldmia r0!, {{r4-r11}}               ", // Pop the core registers.
            "                                       ",
            "                                       ",
            "   msr psp, r0                         ",
            "   isb                                 ",
            "                                       ",
            "   orr r14, #0xd                       ",
            "   bx r14                              ",
        )
    }
}

#[cfg(armv7em)]
#[no_mangle]
pub unsafe extern "C" fn SVCall() {
    unsafe {
        asm!(
            "   ldr r3, =CURRENT_TASK_PTR           ", // Get the location of the current TCB, may be different
            "   ldr r2, [r3]                        ", // The first item in CurrentTCB is the task top of stack.
            "   ldr r0, [r2]                        ",
            "                                       ",
            "   ldmia r0!, {{r4-r11, r14}}           ", // Pop the core registers.
            "                                       ",
            "                                       ",
            "   msr psp, r0                         ",
            "   isb                                 ",
            "                                       ",
            "   bx r14                              ",
        )
    }
}

#[no_mangle]
pub unsafe extern "C" fn NMI() {
    os_print!("NMI");
    loop {}
}
#[no_mangle]
pub unsafe extern "C" fn HardFault() {
    os_print!("HARD FAULT!");
    loop {}
}
#[no_mangle]
pub unsafe extern "C" fn MemManage() {
    os_print!("Mem Manage Exception!");
    loop {}
}
#[no_mangle]
pub unsafe extern "C" fn BusFault() {
    os_print!("BUS FAULT!");
    loop {}
}
#[no_mangle]
pub unsafe extern "C" fn UsageFault() {
    os_print!("USAGE FAULT!");
    loop {}
}

#[inline(always)]
pub unsafe fn disable_interrupt() {
    unsafe {
        asm!(
          "mov {tmp}, #0xff",
          "msr basepri, {tmp}",
          "isb",
          "dsb",
          tmp = out(reg) _,
        );
    }
}

#[inline(always)]
pub unsafe fn enable_interrupt() {
    unsafe {
        asm!(
          "mov {tmp}, #0",
          "msr basepri, {tmp}",
          tmp = out(reg) _,
        );
    }
}

#[no_mangle]
unsafe fn new_power_cycle() {
    asm!(
        "ldr r0, =0xE000ED08 ",
        "ldr r0, [r0]        ",
        "ldr r0, [r0]        ",
        "msr msp, r0         ", // Reset MSP
        "bl PowerReset",
        options(noreturn)
    )
}

fn disable_systick() {
    let mut sys_t = SystemTimer::new();
    sys_t.set_config(0);
}

static mut TIME_SINCE_LAST_CTX_SW: u32 = 0;
static mut TIME_SINCE_LAST_POWER_FAILURE: u32 = 0;

#[cfg(feature = "power_failure")]
#[no_mangle]
pub unsafe extern "C" fn SysTick() {
    use crate::{
        benchmarks::is_benchnmark_done,
        task::{get_current_task_ptr, power_failure_task_update_stats},
    };

    extern "Rust" {
        fn process_tick();
    }
    TIME_SINCE_LAST_CTX_SW += CLK_RELOAD_VALUE;
    TIME_SINCE_LAST_POWER_FAILURE += CLK_RELOAD_VALUE;
    process_tick();
    let mut ctx_sw_pending = false;
    if TIME_SINCE_LAST_CTX_SW >= CTX_SWITCH_CYCLE {
        ctx_sw_pending = true;
        TIME_SINCE_LAST_CTX_SW = 0;
    }
    if TIME_SINCE_LAST_POWER_FAILURE >= POWER_FAILURE_CYCLE
        && !is_benchnmark_done()
        && current_generation() < MAX_FAILURE_CNT
    {
        TIME_SINCE_LAST_POWER_FAILURE = 0;
        if ctx_sw_pending {
            critical::with_no_interrupt(|cs| {
                task_switch();
            });
        }
        let current_task = unsafe { &*get_current_task_ptr().volatile_load() };
        power_failure_task_update_stats(current_task);
        //disable_systick();
        #[cfg(feature = "debug_power_failure")]
        os_print!("injecting power failure...");
        new_power_cycle();
    }
    let scb = SCB::new();
    scb.p.icsr.modify(|v| v | ICSR_PENDSV_SET_BIT);
}

#[cfg(not(feature = "power_failure"))]
#[no_mangle]
pub unsafe extern "C" fn SysTick() {
    extern "Rust" {
        fn process_tick();
    }
    // critical::with_no_interrupt(|cs| {
    //     process_tick(cs);
    // });
    process_tick();
    let scb = SCB::new();
    scb.p.icsr.modify(|v| v | ICSR_PENDSV_SET_BIT);
}

pub unsafe fn arch_yield() {
    let scb = SCB::new();
    scb.p.icsr.modify(|v| v | ICSR_PENDSV_SET_BIT);
    unsafe {
        asm!("isb", "dsb",);
    }
}

#[link_section = ".vector_table.exceptions"]
#[no_mangle]
pub static EXCEPTIONS: [Vector; 15] = [
    Vector { handler: Reset },
    Vector { handler: NMI },
    Vector { handler: HardFault },
    Vector { handler: MemManage },
    Vector { handler: BusFault },
    Vector {
        handler: UsageFault,
    },
    Vector { reserved: 0 },
    Vector { reserved: 0 },
    Vector { reserved: 0 },
    Vector { reserved: 0 },
    Vector { handler: SVCall },
    Vector { reserved: 0 },
    Vector { reserved: 0 },
    Vector { handler: PendSV },
    Vector { handler: SysTick },
];
