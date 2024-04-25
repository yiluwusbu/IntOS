use core::arch::asm;

use crate::{
    arch::recover_and_boot,
    benchmarks::is_benchnmark_done,
    critical,
    recover::current_generation,
    task::{get_current_task_ptr, power_failure_task_update_stats, process_tick, task_switch},
};

#[cfg(not(feature = "power_failure"))]
pub const CLK_RELOAD_VALUE: u32 = crate::board!(CLK_RELOAD_VALUE) as u32;
#[cfg(feature = "power_failure")]
pub const CLK_RELOAD_VALUE: u32 = crate::board!(CLK_RELOAD_VALUE) as u32;
#[cfg(feature = "power_failure")]
pub const POWER_FAILURE_CYCLE: u32 = crate::board!(POWER_FAILURE_CYCLE) as u32;
#[cfg(feature = "power_failure")]
pub const CTX_SWITCH_CYCLE: u32 = crate::board!(CTX_SWITCH_CYCLE) as u32;

#[cfg(feature = "power_failure")]
pub const MAX_FAILURE_CNT: usize = 10000;

pub union Vector {
    _reserved: u16,
    handler: unsafe extern "msp430-interrupt" fn(),
}

#[no_mangle]
extern "msp430-interrupt" fn DefaultHandler() {
    // The interrupts are already disabled here.
    loop {
        // Prevent optimizations that can remove this loop.
        msp430::asm::barrier();
    }
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
    core::ptr::write_bytes(&mut _sbss as *mut u8, 0, count);

    // This section will be also on FRAM when using idem processing
    #[cfg(all(not(idem), not(baseline)))]
    {
        let count = &_edata as *const u8 as usize - &_sdata as *const u8 as usize;
        core::ptr::copy_nonoverlapping(&_sidata as *const u8, &mut _sdata as *mut u8, count);
    }
    // We don't need to copy & initialize pmem
    #[cfg(sram_baseline)]
    {
        let count = &_epmem as *const u8 as usize - &_spmem as *const u8 as usize;
        core::ptr::copy_nonoverlapping(&_sipmem as *const u8, &mut _spmem as *mut u8, count);
    }
}

pub(super) type Time = u16;

#[no_mangle]
unsafe extern "C" fn entry() {
    extern "Rust" {
        fn init_ram();
        fn recover_and_boot();
    }
    crate::board::board_init();
    init_ram();
    recover_and_boot();
    // panic!("Can't return to here!");
}

#[no_mangle]
#[naked]
unsafe extern "msp430-interrupt" fn Reset() {
    #[cfg(not(idem))]
    asm!("mov #_stack_start, r1", "jmp entry", options(noreturn));
    #[cfg(idem)]
    asm!(
        "mov #_stack_start, r1",
        "call #_init_store_ptrs",
        "jmp entry",
        options(noreturn)
    );
}

#[cfg(not(feature = "power_failure"))]
#[no_mangle]
#[naked]
unsafe extern "msp430-interrupt" fn TickHandler() {
    // crate::peripherals::gpio_toggle_output_on_pin(crate::peripherals::GPIOPort::P1, GPIO_PIN0);
    unsafe {
        asm!(
            /* save context */
            "push r4",
            "push r5",
            "push r6",
            "push r7",
            "push r8",
            "push r9",
            "push r10",
            "push r11",
            "push r12",
            "push r13",
            "push r14",
            "push r15",
            "mov  &CURRENT_TASK_PTR, r12",
            "mov  r1, 0(r12)",
            /* call process tick */
            "call #process_tick",
            /* restore context */
            "mov  &CURRENT_TASK_PTR, r12",
            "mov  0(r12), r1",
            "pop r15",
            "pop r14",
            "pop r13",
            "pop r12",
            "pop r11",
            "pop r10",
            "pop r9",
            "pop r8",
            "pop r7",
            "pop r6",
            "pop r5",
            "pop r4",
            "reti",
            options(noreturn)
        );
    }
}

#[cfg(feature = "power_failure")]
#[no_mangle]
#[naked]
unsafe extern "msp430-interrupt" fn TickHandler() {
    // crate::peripherals::gpio_toggle_output_on_pin(crate::peripherals::GPIOPort::P1, GPIO_PIN0);
    unsafe {
        asm!(
            /* save context */
            "push r4",
            "push r5",
            "push r6",
            "push r7",
            "push r8",
            "push r9",
            "push r10",
            "push r11",
            "push r12",
            "push r13",
            "push r14",
            "push r15",
            "mov  &CURRENT_TASK_PTR, r12",
            "mov  r1, 0(r12)",
            /* call process tick */
            "call #tick_handler",
            /* restore context */
            "mov  &CURRENT_TASK_PTR, r12",
            "mov  0(r12), r1",
            "pop r15",
            "pop r14",
            "pop r13",
            "pop r12",
            "pop r11",
            "pop r10",
            "pop r9",
            "pop r8",
            "pop r7",
            "pop r6",
            "pop r5",
            "pop r4",
            "reti",
            options(noreturn)
        );
    }
}

#[cfg(feature = "power_failure")]
#[no_mangle]
unsafe fn new_power_cycle() {
    asm!(
        "mov #_stack_start, r1",
        "call #recover_and_boot",
        options(noreturn)
    );
}

static mut TIME_SINCE_LAST_CTX_SW: u32 = 0;
static mut TIME_SINCE_LAST_POWER_FAILURE: u32 = 0;

#[cfg(feature = "power_failure")]
#[export_name = "tick_handler"]
unsafe fn tick_handler() {
    crate::board::msp430fr5994::peripherals::stop_timer_interrupt();
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
        crate::os_print!("injecting power failure...");
        new_power_cycle();
    } else {
        critical::with_no_interrupt(|cs| {
            task_switch();
        });
    }
    crate::board::msp430fr5994::peripherals::setup_timer_interrupt();
}

macro_rules! save_context {
    () => {
        unsafe {
            asm!(
                "push r4",
                "push r5",
                "push r6",
                "push r7",
                "push r8",
                "push r9",
                "push r10",
                "push r11",
                "push r12",
                "push r13",
                "push r14",
                "push r15",
                "mov  &CURRENT_TASK_PTR, r12",
                "mov  r1, 0(r12)",
                options(noreturn)
            )
        }
    };
}

macro_rules! restore_context {
    () => {
        unsafe {
            asm!(
                "mov  &CURRENT_TASK_PTR, r12",
                "mov  0(r12), r1",
                "pop r15",
                "pop r14",
                "pop r13",
                "pop r12",
                "pop r11",
                "pop r10",
                "pop r9",
                "pop r8",
                "pop r7",
                "pop r6",
                "pop r5",
                "pop r4",
                "reti",
                options(noreturn)
            )
        }
    };
}

const SR_INT_ENABLE: u16 = 0x08;

pub unsafe fn initialize_stack(stack_top: usize, task_code: usize, param: usize) -> usize {
    unsafe {
        let mut stack_top = stack_top as *mut u16;
        // first entry is for double-word alignment
        *stack_top = task_code as u16; // pc
        stack_top = stack_top.sub(1);
        *stack_top = SR_INT_ENABLE; // sr
        stack_top = stack_top.sub(1);
        *stack_top = 0x4444;
        stack_top = stack_top.sub(1);
        *stack_top = 0x5555;
        stack_top = stack_top.sub(1);
        *stack_top = 0x6666;
        stack_top = stack_top.sub(1);
        *stack_top = 0x7777;
        stack_top = stack_top.sub(1);
        *stack_top = 0x8888;
        stack_top = stack_top.sub(1);
        *stack_top = 0x9999;
        stack_top = stack_top.sub(1);
        *stack_top = 0xAAAA;
        stack_top = stack_top.sub(1);
        *stack_top = 0xBBBB;
        stack_top = stack_top.sub(1);
        *stack_top = param as u16; // r12, first parameter
        stack_top = stack_top.sub(1);
        *stack_top = 0xDDDD;
        stack_top = stack_top.sub(1);
        *stack_top = 0xEEEE;
        stack_top = stack_top.sub(1);
        *stack_top = 0xFFFF;
        return stack_top as usize;
    }
}

#[inline(always)]
pub fn enable_interrupt() {
    unsafe {
        msp430::interrupt::enable();
    }
}

#[inline(always)]
pub fn disable_interrupt() {
    unsafe {
        msp430::interrupt::disable();
    }
}

#[inline(always)]
pub fn get_cycle_cnt() -> u32 {
    0
}

#[inline(always)]
pub fn is_cycle_cnt_supported() -> bool {
    false
}

#[inline(always)]
pub fn enable_cycle_cnt() {}

#[inline(always)]
pub fn is_cycle_cnt_enabled() -> bool {
    false
}

#[inline(always)]
pub fn get_system_clock_cycles() -> u32 {
    #[cfg(not(feature = "msp430_use_timerb"))]
    {
        return msp430::critical_section::with(|_| {
            let sys_timer = crate::board::msp430fr5994::peripherals::TimerA::new();
            let cycles = sys_timer.read_cycles() as u32;
            let ticks = crate::time::get_time() as u32;
            ticks * crate::board::msp430fr5994::CLK_RELOAD_VALUE as u32 + cycles
        });
    }

    #[cfg(feature = "msp430_use_timerb")]
    {
        let sys_timer = crate::board::msp430fr5994::peripherals::TimerB::new();
        return sys_timer.read_cycles() as u32;
    }
}

pub fn start_kernel() {
    #[cfg(not(feature = "power_failure"))]
    {
        crate::board::msp430fr5994::peripherals::setup_timer_interrupt();
        #[cfg(feature = "msp430_use_timerb")]
        crate::board::msp430fr5994::peripherals::setup_timer_b();
    }
    #[cfg(feature = "power_failure")]
    {
        if current_generation() == 1 {
            crate::board::msp430fr5994::peripherals::setup_timer_interrupt();
            #[cfg(feature = "msp430_use_timerb")]
            crate::board::msp430fr5994::peripherals::setup_timer_b();
        }
        // crate::board::msp430fr5994::peripherals::setup_timer_interrupt();
    }
    restore_context!();
    return;
}

#[no_mangle]
#[naked]
pub extern "C" fn arch_yield() {
    unsafe {
        asm!(
            /* save context */
            "push r2",
            "dint {{ nop",
            "push r4",
            "push r5",
            "push r6",
            "push r7",
            "push r8",
            "push r9",
            "push r10",
            "push r11",
            "push r12",
            "push r13",
            "push r14",
            "push r15",
            "mov  &CURRENT_TASK_PTR, r12",
            "mov  r1, 0(r12)",
            /* call task switch */
            "call #task_switch",
            /* restore context */
            "mov  &CURRENT_TASK_PTR, r12",
            "mov  0(r12), r1",
            "pop r15",
            "pop r14",
            "pop r13",
            "pop r12",
            "pop r11",
            "pop r10",
            "pop r9",
            "pop r8",
            "pop r7",
            "pop r6",
            "pop r5",
            "pop r4",
            "reti",
            options(noreturn)
        );
    }
}

#[link_section = ".vector_table.exceptions"]
#[no_mangle]
static VECTOR_TABLE: [Vector; 56] = [
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFF90, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFF92, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFF94, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFF96, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFF98, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFF9A, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFF9C, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFF9E, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFA0, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFA2, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFA4, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFA6, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFA8, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFAA, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFAC, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFAE, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFB0, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFB2, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFB4, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFB6, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFB8, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFBA, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFBC, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFBE, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFC0, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFC2, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFC4, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFC6, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFC8, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFCA, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFCC, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFCE, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFD0, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFD2, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFD4, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFD6, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFD8, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFDA, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFDC, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFDE, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFE0, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFE2, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFE4, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFE6, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFE8, LENGTH = 0x0002
    Vector {
        handler: TickHandler,
    }, // ORIGIN = 0xFFEA, LENGTH = 0x0002   => Timer A Int. Vector
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFEC, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFEE, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFF0, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFF2, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFF4, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFF6, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFF8, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFFA, LENGTH = 0x0002
    Vector {
        handler: DefaultHandler,
    }, // ORIGIN = 0xFFFC, LENGTH = 0x0002
    Vector { handler: Reset }, // ORIGIN = 0xFFFE, LENGTH = 0x0002
];
