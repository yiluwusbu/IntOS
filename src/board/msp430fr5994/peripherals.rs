use volatile_register::RW;

use super::CLK_RELOAD_VALUE;

// LED 

pub fn led_toggle() {
    gpio_toggle_output_on_pin(GPIOPort::P1, GPIO_PIN0);
}


#[repr(C)]
pub struct PMMRegisterBlock {
    pmmctl0: RW<u16>,
    _reserved1: [u16; 4],
    pmmifg: RW<u16>,
    _reserved2: [u16; 2],
    pm5ctl0: RW<u16>,
}

pub struct PMM {
    pub p: &'static PMMRegisterBlock,
}

/* PM5CTL0 Power Mode 5 Control Bits */
const PMM_BASE: u16 = 0x120;
const LOCKLPM5: u16 = 0x0001; /* Lock I/O pin configuration upon entry/exit to/from LPM5 */

impl PMM {
    pub fn new() -> Self {
        Self {
            p: unsafe { &*(PMM_BASE as *const PMMRegisterBlock) },
        }
    }
}

pub fn pmm_unlock_lpm5() {
    let pmm = PMM::new();
    unsafe {
        pmm.p.pm5ctl0.modify(|v| v & !LOCKLPM5);
    }
}

#[repr(C)]
pub struct WDTRegisterBlock {
    wdtctl: RW<u16>,
}

pub struct WDT {
    pub p: &'static WDTRegisterBlock,
}

const WDT_A_BASE: u16 = 0x015c;
const WDTHOLD: u16 = 0x0080; /* Watchdog timer is stopped */
const WDTPW: u16 = 0x5a00; /* Watchdog timer password */

impl WDT {
    pub fn new() -> Self {
        Self {
            p: unsafe { &*(WDT_A_BASE as *const WDTRegisterBlock) },
        }
    }
}

pub fn wdt_a_hold() {
    let wdt = WDT::new();
    unsafe {
        wdt.p.wdtctl.modify(|v| ((v & 0x00ff) | WDTHOLD) + WDTPW);
    }
}

#[derive(Clone, Copy)]
pub enum GPIOPort {
    P1 = 1,
    P2 = 2,
    P3 = 3,
    P4 = 4,
}

#[repr(C)]
pub struct GPIORegisterBlock {
    input: RW<u16>,
    output: RW<u16>,
    dir: RW<u16>,
    ren: RW<u16>,
    _reserved: u16,
    sel0: RW<u16>,
    sel1: RW<u16>,
}

pub struct GPIO {
    pub p: &'static GPIORegisterBlock,
}

impl GPIO {
    pub fn new(port: GPIOPort) -> Self {
        let base: u16 = match port {
            GPIOPort::P1 => 0x200,
            GPIOPort::P2 => 0x200,
            GPIOPort::P3 => 0x220,
            GPIOPort::P4 => 0x220,
        };
        Self {
            p: unsafe { &*(base as *const GPIORegisterBlock) },
        }
    }
}

pub const GPIO_PIN0: u16 = 0x0001;
pub const GPIO_PIN1: u16 = 0x0002;
pub const GPIO_PIN2: u16 = 0x0004;
pub const GPIO_PIN3: u16 = 0x0008;
pub const GPIO_PIN4: u16 = 0x0010;
pub const GPIO_PIN5: u16 = 0x0020;
pub const GPIO_PIN6: u16 = 0x0040;
pub const GPIO_PIN7: u16 = 0x0080;
pub const GPIO_PIN8: u16 = 0x0100;
pub const GPIO_PIN9: u16 = 0x0200;
pub const GPIO_PIN10: u16 = 0x0400;
pub const GPIO_PIN11: u16 = 0x0800;
pub const GPIO_PIN12: u16 = 0x1000;
pub const GPIO_PIN13: u16 = 0x2000;
pub const GPIO_PIN14: u16 = 0x4000;
pub const GPIO_PIN15: u16 = 0x8000;

pub fn gpio_toggle_output_on_pin(port: GPIOPort, mut pins: u16) {
    let gpio = GPIO::new(port);
    if (((port as u8) & 1) ^ 1) != 0 {
        pins <<= 8;
    }
    unsafe { gpio.p.output.modify(|v| v ^ pins) }
}

pub fn gpio_set_output_high_on_pin(port: GPIOPort, mut pins: u16) {
    let gpio = GPIO::new(port);
    if (((port as u8) & 1) ^ 1) != 0 {
        pins <<= 8;
    }
    unsafe {
        gpio.p.output.modify(|v| v | pins);
    }
}

pub fn gpio_set_output_low_on_pin(port: GPIOPort, mut pins: u16) {
    let gpio = GPIO::new(port);
    if (((port as u8) & 1) ^ 1) != 0 {
        pins <<= 8;
    }
    unsafe {
        gpio.p.output.modify(|v| v & !pins);
    }
}

pub fn gpio_set_as_output_pin(port: GPIOPort, mut pins: u16) {
    let gpio = GPIO::new(port);

    if (((port as u8) & 1) ^ 1) != 0 {
        pins <<= 8;
    }
    unsafe {
        gpio.p.sel0.modify(|v| v & !pins);
        gpio.p.sel1.modify(|v| v & !pins);
        gpio.p.dir.modify(|v| v | pins);
    }
}

pub fn gpio_set_as_secondary_module_function_input_pin(port: GPIOPort, mut pins: u16) {
    let gpio = GPIO::new(port);

    if (((port as u8) & 1) ^ 1) != 0 {
        pins <<= 8;
    }
    unsafe {
        gpio.p.dir.modify(|v| v & !pins);
        gpio.p.sel0.modify(|v| v & !pins);
        gpio.p.sel1.modify(|v| v | pins);
    }
}

#[repr(C)]
pub struct UCSIRegisterBlock {
    ctlw0: RW<u16>,       // 0x00
    ctlw1: RW<u16>,       // 0x02
    _reserved: RW<u16>,   // 0x04
    brw: RW<u16>,         // 0x06
    mctlw: RW<u16>,       // 0x08
    statw: RW<u16>,       // 0x0A
    pub rxbuf: RW<u16>,   // 0x0C
    pub txbuf: RW<u16>,   // 0x0E
    abctl: RW<u16>,       // 0x10
    irctl: RW<u16>,       // 0x12
    _reserved2: [u16; 3], // 0x14, 0x16, 0x18
    ie: RW<u16>,          // 0x1A
    pub ifg: RW<u16>,     // 0x1C
    iv: RW<u16>,          // 0x1E
}

pub struct UCSI {
    pub p: &'static UCSIRegisterBlock,
}

unsafe impl Sync for UCSI {}

#[repr(u16)]
pub enum USCIPort {
    EUSCIA0 = 0x05c0,
    EUSCIA1 = 0x05e0,
    EUSCIA2 = 0x0600,
    EUSCIA3 = 0x0620,
}

impl UCSI {
    pub fn new(port: USCIPort) -> Self {
        Self {
            p: unsafe { &*(port as u16 as *const UCSIRegisterBlock) },
        }
    }
}

const UCSWRST: u16 = 0x0001;
const UCSSEL_SMCLK: u16 = 0x0080;
pub const UCTXIFG: u16 = 0x0002;

pub fn config_gpio_for_uart_default() {
    gpio_set_as_secondary_module_function_input_pin(
        GPIOPort::P2,
        GPIO_PIN0 as u16 | GPIO_PIN1 as u16,
    );
}

// pub const DEFAULT_SMCLK_DIV: u16 = DIVS_16;
// pub const DEFAULT_PRESCALAR: u16 = 8;
// pub const DEFAULT_FIRSTMOD: u16 = 0;
// pub const DEFAULT_SECONDMOD: u16 = 0xD6;
// pub const DEFAULT_OVERSAMPLING: u16 = 0;

pub const DEFAULT_SMCLK_DIV: u16 = DIVS_4;
pub const DEFAULT_PRESCALAR: u16 = 2;
pub const DEFAULT_FIRSTMOD: u16 = 2;
pub const DEFAULT_SECONDMOD: u16 = 187;
pub const DEFAULT_OVERSAMPLING: u16 = 1;

// pub const DEFAULT_SMCLK_DIV: u16 = DIVS_2;
// pub const DEFAULT_PRESCALAR: u16 = 4;
// pub const DEFAULT_FIRSTMOD: u16 = 5;
// pub const DEFAULT_SECONDMOD: u16 = 85;
// pub const DEFAULT_OVERSAMPLING: u16 = 1;

// pub const DEFAULT_SMCLK_DIV: u16 = DIVS_1;
// pub const DEFAULT_PRESCALAR: u16 = 8;
// pub const DEFAULT_FIRSTMOD: u16 = 10;
// pub const DEFAULT_SECONDMOD: u16 = 247;
// pub const DEFAULT_OVERSAMPLING: u16 = 1;

pub fn config_uart_default() {
    config_uart_custom(
        DEFAULT_PRESCALAR,
        DEFAULT_FIRSTMOD,
        DEFAULT_SECONDMOD,
        DEFAULT_OVERSAMPLING,
    );
}

pub fn config_uart_custom(prescalar: u16, firstmod: u16, secondmod: u16, oversampling: u16) {
    let ucsi = UCSI::new(USCIPort::EUSCIA0);
    let mctlw = (secondmod << 8) + (firstmod << 4) + oversampling;
    unsafe {
        ucsi.p.ctlw0.write(UCSWRST);
        ucsi.p.ctlw0.modify(|v| v | UCSSEL_SMCLK);
        ucsi.p.brw.write(prescalar);
        ucsi.p.mctlw.write(mctlw);
        ucsi.p.ctlw0.modify(|v| v & !UCSWRST);
    }
}

#[repr(C)]
pub struct TimerARegisterBlock {
    ta0ctl: RW<u16>,      // 0x00
    ta0cctl0: RW<u16>,    // 0x02
    ta0cctl1: RW<u16>,    // 0x04
    ta0cctl2: RW<u16>,    // 0x06
    _reserved: [u16; 4],  // 0x08, 0x0A, 0x0C, 0x0E
    ta0r: RW<u16>,        // 0x10
    pub ta0ccr0: RW<u16>, // 0x12
    ta0ccr1: RW<u16>,     // 0x14
    ta0ccr2: RW<u16>,     // 0x16
    _reserved1: RW<u16>,  // 0x18
    ta0ex0: RW<u16>,      // 0x20
    _reserved2: [u16; 6], // 22 24 26 28 2a 2c
    ta0iv: RW<u16>,
}

#[repr(C)]
pub struct TimerBRegisterBlock {
    tb0ctl: RW<u16>,      // 0x00
    tb0cctl0: RW<u16>,    // 0x02
    tb0cctl1: RW<u16>,    // 0x04
    tb0cctl2: RW<u16>,    // 0x06
    tb0cctl3: RW<u16>,    // 0x08
    tb0cctl4: RW<u16>,    // 0x0a
    tb0cctl5: RW<u16>,    // 0x0c
    tb0cctl6: RW<u16>,    // 0x0e
    tb0r: RW<u16>,        // 0x10
    pub tb0ccr0: RW<u16>, // 0x12
    tb0ccr1: RW<u16>,     // 0x14
    tb0ccr2: RW<u16>,     // 0x16
    tb0ccr3: RW<u16>,     // 0x18
    tb0ccr4: RW<u16>,     // 0x1a
    tb0ccr5: RW<u16>,     // 0x1c
    tb0ccr6: RW<u16>,     // 0x1e
    tb0ex0: RW<u16>,      // 0x20
    _reserved: [u16; 6],  // 22 24 26 28 2a 2c
    tb0iv: RW<u16>,       //0x2e
}

const TIMER_A_BASE: u16 = 0x0340;
const TIMER_B_BASE: u16 = 0x03C0;

const CCIE: u16 = 0x0010; // interrupt enable
const MC_UP: u16 = 0x0010; // up mode
const TASSEL_SMCLK: u16 = 0x0200;
const TBSSEL_ACLK: u16 = 0x0100; /* ACLK */
const TACLR: u16 = 0x0004;
const TBCLR: u16 = 0x0004;

pub struct TimerA {
    pub p: &'static TimerARegisterBlock,
}

impl TimerA {
    pub fn new() -> Self {
        Self {
            p: unsafe { &*(TIMER_A_BASE as *const TimerARegisterBlock) },
        }
    }

    pub fn read_cycles(&self) -> u16 {
        self.p.ta0r.read()
    }
}

pub struct TimerB {
    pub p: &'static TimerBRegisterBlock,
}

impl TimerB {
    pub fn new() -> Self {
        Self {
            p: unsafe { &*(TIMER_B_BASE as *const TimerBRegisterBlock) },
        }
    }

    pub fn read_cycles(&self) -> u16 {
        self.p.tb0r.read()
    }
}

pub fn stop_timer_interrupt() {
    let timer_a = TimerA::new();
    unsafe {
        timer_a.p.ta0ctl.write(0);
        timer_a.p.ta0ctl.modify(|v| v | TACLR);
    }
}

pub fn setup_timer_b() {
    let timer_b = TimerB::new();
    unsafe {
        timer_b.p.tb0ctl.write(0);
        timer_b.p.tb0ctl.modify(|v| v | TBCLR);
        // timer_b.p.tb0cctl0.modify(|v| v | CCIE);
        timer_b.p.tb0ccr0.write(0xffff);
        timer_b.p.tb0ctl.modify(|v| v | (TBSSEL_ACLK | MC_UP));
    }
}

pub fn setup_timer_interrupt() {
    let timer_a = TimerA::new();
    unsafe {
        timer_a.p.ta0ctl.write(0);
        timer_a.p.ta0ctl.modify(|v| v | TACLR);
        timer_a.p.ta0cctl0.modify(|v| v | CCIE);
        timer_a.p.ta0ccr0.write(CLK_RELOAD_VALUE);
        timer_a.p.ta0ctl.modify(|v| v | (TASSEL_SMCLK | MC_UP));
    }
}

pub fn config_gpio_for_smclk() {
    let gpio = GPIO::new(GPIOPort::P3);
    unsafe {
        gpio.p.dir.modify(|v| v | GPIO_PIN4);
        gpio.p.sel1.modify(|v| v | GPIO_PIN4);
        gpio.p.sel0.modify(|v| v | GPIO_PIN4);
    }
}

// FRAM Control
const FRCTLPW: u16 = 0xA500;
const NWAITS_0: u16 = 0x0000; /* FRAM Wait state control: 0 */
const NWAITS_1: u16 = 0x0010; /* FRAM Wait state control: 1 */
const NWAITS_2: u16 = 0x0020; /* FRAM Wait state control: 2 */

const FRCTL_A_BASE: usize = 0x0140;

#[repr(C)]
pub struct FRCTLRegisterBlock {
    frctl0: RW<u16>,
}

pub struct FRCTL {
    pub p: &'static FRCTLRegisterBlock,
}

impl FRCTL {
    pub fn new() -> Self {
        Self {
            p: unsafe { &*(FRCTL_A_BASE as *const FRCTLRegisterBlock) },
        }
    }
}

// Clock Control
const DCORSEL: u16 = 0x0040; /* DCO range select. */
const DCOFSEL_0: u16 = 0x0000; /* DCO frequency select: 0 */
const DCOFSEL_1: u16 = 0x0002; /* DCO frequency select: 1 */
const DCOFSEL_2: u16 = 0x0004; /* DCO frequency select: 2 */
const DCOFSEL_3: u16 = 0x0006; /* DCO frequency select: 3 */
const DCOFSEL_4: u16 = 0x0008; /* DCO frequency select: 4 */
const DCOFSEL_5: u16 = 0x000A; /* DCO frequency select: 5 */
const DCOFSEL_6: u16 = 0x000C; /* DCO frequency select: 6 */
const DCOFSEL_7: u16 = 0x000E; /* DCO frequency select: 7 */

const CSKEY_H: u8 = 0xA5;

const SELA_VLOCLK: u16 = 0x0100; /* ACLK Source Select VLOCLK */
const SELS_DCOCLK: u16 = 0x0030; /* SMCLK Source Select DCOCLK */
const SELM_DCOCLK: u16 = 0x0003; /* MCLK Source Select DCOCLK */

const DIVM_1: u16 = 0x0000; /* MCLK Source Divider f(MCLK)/1 */
const DIVM_2: u16 = 0x0001; /* MCLK Source Divider f(MCLK)/2 */
const DIVM_4: u16 = 0x0002; /* MCLK Source Divider f(MCLK)/4 */
const DIVM_8: u16 = 0x0003; /* MCLK Source Divider f(MCLK)/8 */
const DIVM_16: u16 = 0x0004; /* MCLK Source Divider f(MCLK)/16 */
const DIVM_32: u16 = 0x0005; /* MCLK Source Divider f(MCLK)/32 */

const DIVS_1: u16 = 0x0000; /* SMCLK Source Divider f(SMCLK)/1 */
const DIVS_2: u16 = 0x0010; /* SMCLK Source Divider f(SMCLK)/2 */
const DIVS_4: u16 = 0x0020; /* SMCLK Source Divider f(SMCLK)/4 */
const DIVS_8: u16 = 0x0030; /* SMCLK Source Divider f(SMCLK)/8 */
const DIVS_16: u16 = 0x0040; /* SMCLK Source Divider f(SMCLK)/16 */
const DIVS_32: u16 = 0x0050; /* SMCLK Source Divider f(SMCLK)/32 */

const CS_BASE: usize = 0x0160;

#[repr(C)]
pub struct CSRegisterBlock {
    csctl0_l: RW<u8>,
    csctl0_h: RW<u8>,
    csctl1: RW<u16>,
    csctl2: RW<u16>,
    csctl3: RW<u16>,
    csctl4: RW<u16>,
    csctl5: RW<u16>,
    csctl6: RW<u16>,
}

pub struct CS {
    pub p: &'static CSRegisterBlock,
}

impl CS {
    pub fn new() -> Self {
        Self {
            p: unsafe { &*(CS_BASE as *const CSRegisterBlock) },
        }
    }
}

pub fn configure_clock_system() {
    let frctl = FRCTL::new();
    let cs = CS::new();
    unsafe {
        frctl.p.frctl0.write(FRCTLPW | NWAITS_1);
        cs.p.csctl0_h.write(CSKEY_H);
        cs.p.csctl1.write(DCOFSEL_0);
        cs.p.csctl2.write(SELS_DCOCLK | SELM_DCOCLK);
        cs.p.csctl3.write(DIVS_4 | DIVM_4);
        cs.p.csctl1.write(DCOFSEL_4 | DCORSEL);
        // delay
        for _ in 0..200 {
            msp430::asm::nop();
            msp430::asm::barrier();
        }
        cs.p.csctl3.write(DEFAULT_SMCLK_DIV | DIVM_1);
        cs.p.csctl0_h.write(0);
    }
}
