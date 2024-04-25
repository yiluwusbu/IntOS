const CPU_ADDR: u32 = 0x4800_0000;
use core::mem::transmute;

use volatile_register::*;

use crate::{os_print, val_to_bit_field};

#[repr(C)]
struct CPU {
    cachecfg: RW<u32>,
    reserved_0: RW<u32>,
    cachectrl: RW<u32>,
    reserved_1: RW<u32>,
    ncr0start: RW<u32>,
    ncr0end: RW<u32>,
    ncr1start: RW<u32>,
    ncr1end: RW<u32>,
    reserved_2: [u32; 12],
    daxicfg: RW<u32>,
    daxictrl: RW<u32>,
    /* Rest of the fields omitted */
}

impl CPU {
    pub fn new() -> &'static mut Self {
        unsafe { &mut *(CPU_ADDR as *mut Self) }
    }
}

type BitField = (u32, u32);

const CACHECFG_ENABLE: BitField = (0, 1);
const CACHECFG_LRU: BitField = (1, 1);
const CACHECFG_NC0ENABLE: BitField = (2, 1);
const CACHECFG_NC1ENABLE: BitField = (3, 1);
const CACHECFG_CONFIG: BitField = (4, 4);
const CACHECFG_IENABLE: BitField = (8, 1);
const CACHECFG_DENABLE: BitField = (9, 1);
const CACHECFG_CLKGATE: BitField = (10, 1);
const CACHECFG_LS: BitField = (11, 1);
const CACHECFG_NC1_CACHE_LOCK: BitField = (12, 1);
const CACHECFG_NC0_CACHE_LOCK: BitField = (13, 1);
const CACHECFG_DATA_CLKGATE: BitField = (20, 1);
const CACHECFG_ENABLE_MONITOR: BitField = (24, 1);

const CACHECTRL_INVALIDATE: BitField = (0, 1);
const CACHECTRL_RESETSTAT: BitField = (1, 1);
const CACHECTRL_CACHEREADY: BitField = (2, 1);

//*****************************************************************************
// const am_hal_cachectrl_config_t am_hal_cachectrl_defaults =
// {
//     .bLRU                       = 0,
//     .eDescript                  = AM_HAL_CACHECTRL_DESCR_1WAY_128B_4096E,
//     .eMode                      = AM_HAL_CACHECTRL_CONFIG_MODE_INSTR_DATA,
// };
const DEFAULT_CACHECFG_LRU: u32 = 0;
const DEFAULT_CACHECFG_DESC: u32 = 14; // AM_HAL_CACHECTRL_DESCR_1WAY_128B_4096E,
const DEFAULT_CACHECFG_MODE: u32 = 3; // AM_HAL_CACHECTRL_CONFIG_MODE_INSTR_DATA

// const am_hal_daxi_config_t am_hal_daxi_defaults =
// {
//     .bDaxiPassThrough         = false,
//     .bAgingSEnabled            = true,
//     .eAgingCounter            = AM_HAL_DAXI_CONFIG_AGING_4,      =2
//     .eNumBuf                  = AM_HAL_DAXI_CONFIG_NUMBUF_32,    =15
//     .eNumFreeBuf              = AM_HAL_DAXI_CONFIG_NUMFREEBUF_3, =1
// };

//DAXICTRL
const DAXIFLUSHWRITE: BitField = (0, 1); //< [0..0] Writing a 1 to this bitfield forces a flush of WRITE*/
const DAXIINVALIDATE: BitField = (1, 1); //< [1..1] Writing a 1 to this bitfield invalidates any SHARED dataconstr.
const DAXIREADY: BitField = (2, 1); //< [2..2] DAXI Ready Status (enabled and not processing a flush
const DAXIBUSY: BitField = (3, 1); //< [3..3] DAXI status indicating DAXI is busy.
const DAXIAHBBUSY: BitField = (4, 1); //< [4..4] DAXI status indicating DAXI AHB interface is busy.
const DAXISHARED: BitField = (5, 1); //< [5..5] DAXI status indicating at least one full buffer is shared.
const DAXIMODIFIED: BitField = (6, 1); //< [6..6] DAXI status indicating at least one full buffer has modified
const DAXIWRITE: BitField = (7, 1); //< [7..7] DAXI status indicating at least one partially written
const DAXIWALLOC: BitField = (8, 1); //< [8..8] DAXI status indicating at least one write allocation
const DAXIWRLOAD: BitField = (9, 1); //< [9..9] DAXI status indicating at least one partially written
const DAXISTORE: BitField = (10, 1); //< [10..10] DAXI status indicating at least one buffer has outstanding
const DAXIBRESPPENDING: BitField = (11, 1); //< [11..11] DAXI status indicating at least one AXI B repsonse
const DAXIRAXIBUSY: BitField = (12, 1); //< [12..12]

//DAXICFG
const FLUSHLEVEL: BitField = (0, 1); //     [0..0] Level of free buffers to flush out dirty buffers.
const AGINGSENABLE: BitField = (1, 1); //    [1..1] Enables flushing out shared lines using the aging mechanism.
const DAXIPASSTHROUGH: BitField = (2, 1); //   [2..2] Passes requests through DAXI logic, disables cachiconst
const DAXIBECLKGATEEN: BitField = (3, 1); //  [3..3] Enables clock gating of DAXI line buffer byte enables.
const DAXIDATACLKGATEEN: BitField = (4, 1); // [4..4] Enables clock gating of DAXI line buffer data.
const DAXISTATECLKGATEEN: BitField = (5, 1); //   [5..5] Enables clock gating of DAXI state.
const BUFFERENABLE: BitField = (8, 4); //  [11..8] Enables DAXI buffers
const AGINGCOUNTER: BitField = (16, 5); // [20..16] Specifies the relative time that DAXI buffers may remaconst
const MRUGROUPLEVEL: BitField = (24, 2); // [25..24] Sets the MRU group population limit.

///////////////////////////////////
const POWERCTRL_BASE: u32 = 0x40021000;
const POWERCTRL_MEMPWREN_OFF: usize = 0x14 / 4; // word offset
const POWERCTRL_MEMPWRST_OFF: usize = 0x18 / 4; // word offset
const PWRENDTCM: BitField = (0, 3); //   < [2..0] Power up DTCM                                                      */
const PWRENNVM0: BitField = (3, 1); //   < [3..3] Power up NVM0                                                      */
const PWRENCACHEB0: BitField = (4, 1); //   < [4..4] Power up Cache Bank 0. This works in conjunction with
                                       //   Cache enable from flash_cache module. To power up cache
                                       //   bank0, cache has to be enabled and this bit has to be set.                */
const PWRENCACHEB2: BitField = (5, 1); // [5..5] Power up Cache Bank 2. This works in conjunction with

///////////////////////////////////
const POWERCTRL_SSRAMPWREN_OFF: usize = 0x24 / 4;
const POWERCTRL_SSRAMPWRST_OFF: usize = 0x28 / 4;
const PWRENSSRAM: BitField = (0, 2);

// const am_hal_pwrctrl_mcu_memory_config_t    g_DefaultMcuMemCfg =
// {
//     .eCacheCfg          = AM_HAL_PWRCTRL_CACHE_ALL,
//     .bRetainCache       = true,
//     .eDTCMCfg           = AM_HAL_PWRCTRL_DTCM_384K,
//     .eRetainDTCM        = AM_HAL_PWRCTRL_DTCM_384K,
//     .bEnableNVM0        = true,
//     .bRetainNVM0        = false
// };

const DEFAULT_CACHE_CFG: u32 = 2; // Enable all
const DEFAULT_DTCM_CFG: u32 = 7; // 384K
const DEFAULT_SRAM_CFG: u32 = 3; // ALL SRAM

#[derive(Clone, Copy)]
enum DaxiCfgNumBuf {
    DaxiConfigNumBuf1 = 0,
    DaxiConfigNumBuf2 = 1,
    DaxiConfigNumBuf3 = 2,
    DaxiConfigNumBuf4 = 3,
    DaxiConfigNumBuf8 = 7,
    DaxiConfigNumBuf16 = 11,
    DaxiConfigNumBuf31 = 14,
    DaxiConfigNumBuf32 = 15,
}

#[derive(Clone, Copy)]
enum DaxiConfigAging {
    DaxiConfigAging1 = 0,
    DaxiConfigAging2 = 1,
    DaxiConfigAging4 = 2,
    DaxiConfigAging8 = 3,
}

#[derive(Clone, Copy)]
enum DaxiCfgNumFreeBuf {
    DaxiConfigNumFreeBuf2 = 0,
    DaxiConfigNumFreeBuf3 = 1,
}

struct DaxiCfg {
    daxi_pass_through: bool,
    aging_enable: bool,
    aging_counter: DaxiConfigAging,
    num_buf: DaxiCfgNumBuf,
    num_free_buf: DaxiCfgNumFreeBuf,
}

// emulate no write caching
const DEFAULT_DAXI_CFG: DaxiCfg = DaxiCfg {
    daxi_pass_through: true,
    aging_enable: true,
    aging_counter: DaxiConfigAging::DaxiConfigAging4,
    num_buf: DaxiCfgNumBuf::DaxiConfigNumBuf32,
    num_free_buf: DaxiCfgNumFreeBuf::DaxiConfigNumFreeBuf3,
};

const SYNC_READL: *const u32 = 0x47FF0000 as *const u32;

fn am_sysctrl_sysbus_write_flush() {
    unsafe {
        core::ptr::read_volatile(SYNC_READL);
    }
}

fn am_daxi_config(cfg: &DaxiCfg) {
    am_sysctrl_sysbus_write_flush();
    let cpu = CPU::new();
    let daxicfg = val_to_bit_field!(FLUSHLEVEL, cfg.num_free_buf as u32)
        | val_to_bit_field!(AGINGSENABLE, cfg.aging_enable as u32)
        | val_to_bit_field!(DAXIPASSTHROUGH, cfg.daxi_pass_through as u32)
        | val_to_bit_field!(BUFFERENABLE, cfg.num_buf as u32)
        | val_to_bit_field!(AGINGCOUNTER, cfg.aging_counter as u32)
        | val_to_bit_field!(MRUGROUPLEVEL, 0);
    unsafe {
        cpu.daxicfg.write(daxicfg);
    }
    am_sysctrl_sysbus_write_flush();
}

#[repr(C)]
struct PowerCtrl {
    ctrls: [RW<u32>; 0x250],
}

impl PowerCtrl {
    pub fn new() -> &'static mut Self {
        unsafe { &mut *(POWERCTRL_BASE as *mut Self) }
    }
}

fn am_cache_ctrl_set_default() {
    let cache_cfg = val_to_bit_field!(CACHECFG_ENABLE, 0)
        | val_to_bit_field!(CACHECFG_CLKGATE, 1)
        | val_to_bit_field!(CACHECFG_LS, 0)
        | val_to_bit_field!(CACHECFG_DATA_CLKGATE, 1)
        | val_to_bit_field!(CACHECFG_ENABLE_MONITOR, 0)
        | val_to_bit_field!(CACHECFG_LRU, DEFAULT_CACHECFG_LRU)
        | val_to_bit_field!(CACHECFG_CONFIG, DEFAULT_CACHECFG_DESC)
        | (DEFAULT_CACHECFG_MODE << CACHECFG_IENABLE.0);

    let cpu = CPU::new();
    unsafe {
        cpu.cachecfg.write(cache_cfg);
    }
}

fn am_cache_ctrl_enable() {
    let cpu = CPU::new();
    unsafe {
        cpu.cachecfg
            .modify(|v| v | val_to_bit_field!(CACHECFG_ENABLE, 1));

        cpu.cachectrl
            .modify(|v| v | val_to_bit_field!(CACHECTRL_INVALIDATE, 1));
    }
}

fn am_configure_mem() -> Result<(), ()> {
    let pwr = PowerCtrl::new();
    let mem_en = val_to_bit_field!(PWRENCACHEB0, 1)
        | val_to_bit_field!(PWRENCACHEB2, 1)
        | val_to_bit_field!(PWRENDTCM, DEFAULT_DTCM_CFG)
        | val_to_bit_field!(PWRENNVM0, 1);
    unsafe {
        pwr.ctrls[POWERCTRL_MEMPWREN_OFF].modify(|v| v | mem_en);
    }

    // delay
    am_delay_us(5);
    // for i in 0..1000_00 {
    //     ;
    // }

    // check
    let res = pwr.ctrls[POWERCTRL_MEMPWRST_OFF].read() & mem_en;
    if res != mem_en {
        return Err(());
    }

    let sram_en = val_to_bit_field!(PWRENSSRAM, DEFAULT_SRAM_CFG);
    unsafe {
        pwr.ctrls[POWERCTRL_SSRAMPWREN_OFF].modify(|v| v | sram_en);
    }

    // delay
    am_delay_us(5);

    // check
    let res = pwr.ctrls[POWERCTRL_SSRAMPWRST_OFF].read() & sram_en;
    if res != sram_en {
        return Err(());
    }
    return Ok(());
}

// delay functions

const AM_HAL_CLKGEN_FREQ_MAX_HZ: u32 = 96000000;

const AM_HAL_CLKGEN_FREQ_MAX_KHZ: u32 = AM_HAL_CLKGEN_FREQ_MAX_HZ / 1000;
const AM_HAL_CLKGEN_FREQ_MAX_MHZ: u32 = AM_HAL_CLKGEN_FREQ_MAX_HZ / 1000000;
const CYCLESPERITER: u32 = AM_HAL_CLKGEN_FREQ_MAX_MHZ / 3;

const BOOT_ROM_DELAY_FN_ADDR: u32 = 0x0800009D;

fn bootrom_cycle_us(us: u32) -> u32 {
    us * CYCLESPERITER + 0
}

fn am_delay_us(us: u32) {
    let mut n_iter = bootrom_cycle_us(us);
    let cycle_cnt_adj: u32 = ((13 * 1) + 32) / 3;
    let bootrom_delay_fn: extern "C" fn(u32) = unsafe { transmute(BOOT_ROM_DELAY_FN_ADDR) };
    //
    // Allow for the overhead of the burst-mode check and these comparisons
    // by eliminating an appropriate number of iterations.
    //
    if n_iter > cycle_cnt_adj {
        n_iter -= cycle_cnt_adj;
        bootrom_delay_fn(n_iter);
    }
}

const FLASHWPROT0: usize = 0x000003A8 / 4;
const MCUCTRL_BASE: u32 = 0x40020000;
struct MCUCtrl {
    ctrls: [RW<u32>; 1112 / 4],
}

impl MCUCtrl {
    pub fn new() -> &'static mut MCUCtrl {
        unsafe { &mut *(MCUCTRL_BASE as *mut Self) }
    }
}

#[no_mangle]
pub fn am_hw_init() {
    // os_print!("Initializing board: Apollp 4 Blue Plus KXR");
    // #[cfg(idem)]
    // extern "C" {
    //     fn _init_store_ptrs();
    // }
    am_cache_ctrl_set_default();
    am_cache_ctrl_enable();
    match am_configure_mem() {
        Err(_) => {
            os_print!("Failed to configure mem");
        }
        _ => {
            // #[cfg(idem)]
            // unsafe { _init_store_ptrs() };
            os_print!("mem configure ok!");
        }
    }
    // This optional
    am_daxi_config(&DEFAULT_DAXI_CFG);
    am_delay_us(100);

    // unsafe {
    //     test_ram_speed();
    // }

    // loop {

    // }
}

// macro_rules! vec_write {
//     ($id: ident, $v: expr, [ $($e: expr), * ]) => {
//         $(
//             $id[$e] = $v;
//         )*
//     };
// }

// macro_rules! vec_read {
//     ($id: ident, $v: expr, [ $($e: expr), * ]) => {
//         $(
//             assert!($id[$e] == $v);
//         )*
//     };
// }

// #[link_section = ".pmem"]
// static mut SSRAM_ARRAY: [u32; 512] = [42; 512];
// static mut TCM_ARRAY: [u32; 512] = [24; 512];

// unsafe fn test_ram_speed() {
//     // sram write speed
//     let start = crate::util::benchmark_clock();
//     vec_write!(SSRAM_ARRAY, 100, [144, 268, 348, 163, 249, 109, 198, 458, 500, 482, 392, 308, 477,
//         242, 364, 497, 345, 441,  27, 492, 444, 317, 468, 457, 185, 370,
//         247, 297, 410, 294, 147, 318, 437, 425, 470, 125, 113, 273, 439,
//         303, 388, 340, 217, 302, 511, 430, 471, 238, 245, 112,  79, 396,
//         122,  41, 381, 401, 408,  61,  22, 115, 485, 191,  91, 267,  75,
//         211, 293,  48,  93,  21, 288, 277, 393, 343, 398, 296, 429, 494,
//         431,  84,   5,  58, 372, 422, 188, 157, 374, 196,   8,  78, 433,
//         195, 221, 383,  98, 295, 137, 448,  94,  26, 158, 214, 111,  64,
//         502,  50, 421, 282, 203, 255, 462, 350,  60, 402, 231, 162, 491,
//         510, 504,  19, 363, 442, 451, 192,  72, 306, 164, 146,  63,  15,
//         449, 495, 251, 349, 167, 432, 332, 310, 339,  96, 389, 382, 106,
//         346, 178, 322, 420, 362, 170, 509, 179, 181, 373, 128, 246,  57,
//         220, 206, 359, 447, 148, 159, 325, 207,  51, 174,   6, 183, 101,
//         124, 335, 464, 210,  16, 454, 139, 120,  49, 419, 150,  11, 212,
//         307, 407,  40, 166, 412, 480, 299, 205, 414,  82, 145, 499, 315,
//          29, 353,   3, 286,  18, 445, 287, 326, 489, 283, 434, 244, 116,
//          81,  70, 404, 190, 254,  53, 186, 161, 129,  23, 452, 264, 256,
//         232, 312,  36,   2, 379, 406, 304, 409, 461, 319,  54, 194, 473,
//           1, 208, 417, 413, 173, 298, 342, 490, 324, 403,  92, 132,  95,
//         394, 375, 329, 411,  44, 266, 263, 463, 358,  12,  77,  80, 182,
//         235, 474, 446, 131, 225, 107, 201, 177, 355,  32, 199, 224, 424,
//         347, 226,  33, 496, 415, 123,  34, 365,  87, 102,  37, 455, 397,
//         285, 466, 305,  31, 338, 241, 153, 443, 252,  99, 354, 143, 233,
//          14, 165, 331, 193,  43, 292, 142,  24, 240, 248, 239, 272, 352,
//         385, 138, 253, 427, 327, 453, 215, 108, 104, 275, 366, 151, 328,
//         265, 269, 323, 426, 280,  97, 160, 486, 204,  59, 337,  68, 136,
//          39, 460,  90,  56, 507, 333, 418, 154, 149, 118,  89, 493,   0,
//         371, 133, 472, 110, 243, 488,  52, 200, 334, 284, 344, 281,  35,
//         479, 309,  47, 209, 218, 103, 257, 436, 376,  88, 152,  85, 478,
//         259,  45, 105, 114, 222, 467, 456, 258, 483,  86, 321,  76, 316,
//         141, 271, 484, 227,  28,  71,  46, 440, 476,  65, 438, 505, 378,
//         384, 127, 260,  73, 234, 469, 380, 423,  25, 202, 184, 172, 171,
//         361, 351, 386, 187, 289, 360, 368, 229, 498,  67, 416,   7,  10,
//         475, 121,  55, 228, 213,  62, 313,  42, 300, 180, 369, 169, 189,
//         487, 290, 219, 459, 314, 274, 330, 435, 134, 126, 336, 262, 117,
//         311, 377, 100,  20, 261, 119, 341, 428, 390, 155, 250, 405, 175,
//         501,  17, 140, 216, 320, 156, 508,  69, 503, 356, 301, 391, 357,
//         176,  83, 465, 395, 130, 291,   4, 223, 135, 278, 197,   9, 270,
//          66,  74, 236, 399, 279, 400, 387, 168, 450, 506,  30, 367, 230,
//         276,  38, 237, 481,  13]);
//     let end = crate::util::benchmark_clock();
//     os_print!("Cycles: {} for 512 SSRAM word write", end - start);

//     let start = crate::util::benchmark_clock();
//     vec_write!(TCM_ARRAY, 200, [144, 268, 348, 163, 249, 109, 198, 458, 500, 482, 392, 308, 477,
//         242, 364, 497, 345, 441,  27, 492, 444, 317, 468, 457, 185, 370,
//         247, 297, 410, 294, 147, 318, 437, 425, 470, 125, 113, 273, 439,
//         303, 388, 340, 217, 302, 511, 430, 471, 238, 245, 112,  79, 396,
//         122,  41, 381, 401, 408,  61,  22, 115, 485, 191,  91, 267,  75,
//         211, 293,  48,  93,  21, 288, 277, 393, 343, 398, 296, 429, 494,
//         431,  84,   5,  58, 372, 422, 188, 157, 374, 196,   8,  78, 433,
//         195, 221, 383,  98, 295, 137, 448,  94,  26, 158, 214, 111,  64,
//         502,  50, 421, 282, 203, 255, 462, 350,  60, 402, 231, 162, 491,
//         510, 504,  19, 363, 442, 451, 192,  72, 306, 164, 146,  63,  15,
//         449, 495, 251, 349, 167, 432, 332, 310, 339,  96, 389, 382, 106,
//         346, 178, 322, 420, 362, 170, 509, 179, 181, 373, 128, 246,  57,
//         220, 206, 359, 447, 148, 159, 325, 207,  51, 174,   6, 183, 101,
//         124, 335, 464, 210,  16, 454, 139, 120,  49, 419, 150,  11, 212,
//         307, 407,  40, 166, 412, 480, 299, 205, 414,  82, 145, 499, 315,
//          29, 353,   3, 286,  18, 445, 287, 326, 489, 283, 434, 244, 116,
//          81,  70, 404, 190, 254,  53, 186, 161, 129,  23, 452, 264, 256,
//         232, 312,  36,   2, 379, 406, 304, 409, 461, 319,  54, 194, 473,
//           1, 208, 417, 413, 173, 298, 342, 490, 324, 403,  92, 132,  95,
//         394, 375, 329, 411,  44, 266, 263, 463, 358,  12,  77,  80, 182,
//         235, 474, 446, 131, 225, 107, 201, 177, 355,  32, 199, 224, 424,
//         347, 226,  33, 496, 415, 123,  34, 365,  87, 102,  37, 455, 397,
//         285, 466, 305,  31, 338, 241, 153, 443, 252,  99, 354, 143, 233,
//          14, 165, 331, 193,  43, 292, 142,  24, 240, 248, 239, 272, 352,
//         385, 138, 253, 427, 327, 453, 215, 108, 104, 275, 366, 151, 328,
//         265, 269, 323, 426, 280,  97, 160, 486, 204,  59, 337,  68, 136,
//          39, 460,  90,  56, 507, 333, 418, 154, 149, 118,  89, 493,   0,
//         371, 133, 472, 110, 243, 488,  52, 200, 334, 284, 344, 281,  35,
//         479, 309,  47, 209, 218, 103, 257, 436, 376,  88, 152,  85, 478,
//         259,  45, 105, 114, 222, 467, 456, 258, 483,  86, 321,  76, 316,
//         141, 271, 484, 227,  28,  71,  46, 440, 476,  65, 438, 505, 378,
//         384, 127, 260,  73, 234, 469, 380, 423,  25, 202, 184, 172, 171,
//         361, 351, 386, 187, 289, 360, 368, 229, 498,  67, 416,   7,  10,
//         475, 121,  55, 228, 213,  62, 313,  42, 300, 180, 369, 169, 189,
//         487, 290, 219, 459, 314, 274, 330, 435, 134, 126, 336, 262, 117,
//         311, 377, 100,  20, 261, 119, 341, 428, 390, 155, 250, 405, 175,
//         501,  17, 140, 216, 320, 156, 508,  69, 503, 356, 301, 391, 357,
//         176,  83, 465, 395, 130, 291,   4, 223, 135, 278, 197,   9, 270,
//          66,  74, 236, 399, 279, 400, 387, 168, 450, 506,  30, 367, 230,
//         276,  38, 237, 481,  13]);
//     let end = crate::util::benchmark_clock();
//     os_print!("Cycles: {} for 512 TCM word write", end - start);

//     let start = crate::util::benchmark_clock();
//     vec_read!(SSRAM_ARRAY, 100, [343, 452, 204, 336, 264, 280, 213, 221, 195, 261, 468, 176, 331,
//         358, 356, 474, 434, 118, 481, 509, 494,  92, 420, 180, 324, 237,
//         427, 303, 340, 338, 482, 371, 372, 440,  75,  68, 381, 298,  53,
//          76, 322, 449, 342, 114, 339, 405, 355, 491, 184,  38, 205, 283,
//         307, 253, 135, 510,  72,  32, 390, 352, 168,  34,  63, 130, 368,
//          61, 446,  82, 199,  37, 438, 267, 499, 392, 432,  44, 475, 112,
//         151, 469, 366, 466, 219, 132, 235,   3,  42, 505,  21, 383, 437,
//           4,  15, 214, 106,  10, 186, 113, 188, 165, 459, 492, 453, 226,
//           0, 120,  30, 351, 201, 413, 319, 344, 379, 348, 284, 417, 395,
//         109, 504, 457,  65, 302, 206, 256,  39, 376, 503, 293, 488, 463,
//         272,  59, 496,  26,  33, 439, 462,   9, 312, 131, 196, 398, 483,
//         386,  28, 465, 162, 158, 161, 407, 472, 175, 455, 182,  87,  19,
//         281,  86, 396, 121,  99, 441, 160, 193,  45,  16,  89, 192, 282,
//         416, 245, 236, 489, 238, 242,  18, 209, 217, 167, 279,  12,  25,
//         444, 327, 249,  23, 100, 138,  98, 146, 107, 268, 227, 251, 210,
//         270, 300, 297, 115, 328, 493, 447, 345, 269,  35, 334, 394,  94,
//         189, 406, 139, 458, 292, 487, 169, 141, 289,  60, 385,  54, 263,
//         476, 404, 246, 262, 136, 341, 320,  62,  67, 294,   6, 333, 389,
//         156, 202, 259,  31, 117, 254, 255, 274, 354, 329, 295, 471, 212,
//         424, 177, 411,  50, 277, 470, 435, 467, 111, 185, 305,  47, 247,
//         448, 234, 359,  27, 387, 116,  79, 148,  93, 172, 266,  90, 508,
//         152, 423,  66,  85, 278, 409, 215,  57, 149, 123,  11,  81, 250,
//         233, 159, 425, 365,  71, 291, 450, 170, 384, 485,  91, 129, 428,
//         239, 301, 332,  43, 126,  78, 502, 442, 137,   1, 276, 183, 357,
//         290, 203, 275, 211, 230, 378, 197, 198, 347, 360, 321, 422, 495,
//         498, 223, 401, 155,  48, 191, 350, 454, 105, 313, 163, 443, 110,
//          77,   7,   8, 101, 144, 479, 147, 451, 228, 309, 122, 286, 353,
//         181, 273,  70, 337, 461, 363, 478, 414, 418, 316, 400,  29, 207,
//         433, 231,  49, 374, 134, 403, 311, 103, 178,  55, 287, 127, 241,
//         222, 391,  13, 486,  36, 260, 224, 125,  22, 500, 421, 166, 220,
//         194, 402, 490, 304, 415, 408, 108, 140, 323, 124, 388, 143,  20,
//         317, 380, 426, 477,  41, 325, 150, 431, 271, 362,  17, 473, 252,
//         511,  88, 244,  74,  51, 145, 119,  24, 315, 257,  97,  58, 318,
//         456, 142, 430, 393,  69, 314, 265, 484, 361, 243,  64, 335, 375,
//         190, 382, 296, 154, 258, 367,   5, 128, 330, 232, 308, 200, 288,
//         164,  95, 240,  80, 346, 480, 369, 229, 306, 370, 497, 460, 410,
//         436, 373, 171, 133, 310,  52, 507,   2, 377, 104, 218,  56, 187,
//         364, 299, 464,  73, 102, 501, 285, 248,  14, 412, 419,  96, 179,
//         225, 153,  83, 173,  84, 445, 429, 326, 174, 349,  40, 216, 397,
//         157,  46, 208, 506, 399]);
//     let end = crate::util::benchmark_clock();
//     os_print!("Cycles: {} for 512 SSRAM word read", end - start);

//     let start = crate::util::benchmark_clock();
//     vec_read!(TCM_ARRAY, 200, [343, 452, 204, 336, 264, 280, 213, 221, 195, 261, 468, 176, 331,
//         358, 356, 474, 434, 118, 481, 509, 494,  92, 420, 180, 324, 237,
//         427, 303, 340, 338, 482, 371, 372, 440,  75,  68, 381, 298,  53,
//          76, 322, 449, 342, 114, 339, 405, 355, 491, 184,  38, 205, 283,
//         307, 253, 135, 510,  72,  32, 390, 352, 168,  34,  63, 130, 368,
//          61, 446,  82, 199,  37, 438, 267, 499, 392, 432,  44, 475, 112,
//         151, 469, 366, 466, 219, 132, 235,   3,  42, 505,  21, 383, 437,
//           4,  15, 214, 106,  10, 186, 113, 188, 165, 459, 492, 453, 226,
//           0, 120,  30, 351, 201, 413, 319, 344, 379, 348, 284, 417, 395,
//         109, 504, 457,  65, 302, 206, 256,  39, 376, 503, 293, 488, 463,
//         272,  59, 496,  26,  33, 439, 462,   9, 312, 131, 196, 398, 483,
//         386,  28, 465, 162, 158, 161, 407, 472, 175, 455, 182,  87,  19,
//         281,  86, 396, 121,  99, 441, 160, 193,  45,  16,  89, 192, 282,
//         416, 245, 236, 489, 238, 242,  18, 209, 217, 167, 279,  12,  25,
//         444, 327, 249,  23, 100, 138,  98, 146, 107, 268, 227, 251, 210,
//         270, 300, 297, 115, 328, 493, 447, 345, 269,  35, 334, 394,  94,
//         189, 406, 139, 458, 292, 487, 169, 141, 289,  60, 385,  54, 263,
//         476, 404, 246, 262, 136, 341, 320,  62,  67, 294,   6, 333, 389,
//         156, 202, 259,  31, 117, 254, 255, 274, 354, 329, 295, 471, 212,
//         424, 177, 411,  50, 277, 470, 435, 467, 111, 185, 305,  47, 247,
//         448, 234, 359,  27, 387, 116,  79, 148,  93, 172, 266,  90, 508,
//         152, 423,  66,  85, 278, 409, 215,  57, 149, 123,  11,  81, 250,
//         233, 159, 425, 365,  71, 291, 450, 170, 384, 485,  91, 129, 428,
//         239, 301, 332,  43, 126,  78, 502, 442, 137,   1, 276, 183, 357,
//         290, 203, 275, 211, 230, 378, 197, 198, 347, 360, 321, 422, 495,
//         498, 223, 401, 155,  48, 191, 350, 454, 105, 313, 163, 443, 110,
//          77,   7,   8, 101, 144, 479, 147, 451, 228, 309, 122, 286, 353,
//         181, 273,  70, 337, 461, 363, 478, 414, 418, 316, 400,  29, 207,
//         433, 231,  49, 374, 134, 403, 311, 103, 178,  55, 287, 127, 241,
//         222, 391,  13, 486,  36, 260, 224, 125,  22, 500, 421, 166, 220,
//         194, 402, 490, 304, 415, 408, 108, 140, 323, 124, 388, 143,  20,
//         317, 380, 426, 477,  41, 325, 150, 431, 271, 362,  17, 473, 252,
//         511,  88, 244,  74,  51, 145, 119,  24, 315, 257,  97,  58, 318,
//         456, 142, 430, 393,  69, 314, 265, 484, 361, 243,  64, 335, 375,
//         190, 382, 296, 154, 258, 367,   5, 128, 330, 232, 308, 200, 288,
//         164,  95, 240,  80, 346, 480, 369, 229, 306, 370, 497, 460, 410,
//         436, 373, 171, 133, 310,  52, 507,   2, 377, 104, 218,  56, 187,
//         364, 299, 464,  73, 102, 501, 285, 248,  14, 412, 419,  96, 179,
//         225, 153,  83, 173,  84, 445, 429, 326, 174, 349,  40, 216, 397,
//         157,  46, 208, 506, 399]);
//     let end = crate::util::benchmark_clock();
//     os_print!("Cycles: {} for 512 TCM word read", end - start);
// }

// fn am_check_mram_wprot() {
//     let mcuctrl = MCUCtrl::new();
//     let v = mcuctrl.ctrls[FLASHWPROT0].read();
//     os_print!("PROT 0: {:#X}", v);
// }

// #[link_section = ".mram"]
// static mut X: usize = 2;
// #[link_section = ".mram"]
// static mut Y: usize = 3;
// #[link_section = ".mram"]
// static mut Z: usize = 4;

// const BOOT_ROM_MRAM_PROG_MAIN: usize = 0x0800006D;

// #[repr(align(16))]
// struct Ary {
//     a: [u32; 512],
// }

// static mut SRC: Ary = Ary { a: [0; 512] };
// const MRAM_DST: u32 = 1024 * 1024;
// #[no_mangle]
// unsafe fn prog_mram() {
//     let mram_prog_fn: extern "C" fn(u32, u32, u32, u32, u32) -> u32 = unsafe { transmute(BOOT_ROM_MRAM_PROG_MAIN) };
//     for i in 0..64 {
//         SRC.a[i] = i as u32 +1;
//     }
//     let src  = (&SRC as * const Ary as u32);
//     if src & 0xf != 0 {
//         os_print!("Not aligned!");
//     }
//     let ret = mram_prog_fn(0x12344321, 1, src, MRAM_DST / 4, 512);
//     os_print!("ret = {}", ret);
//     let w = MRAM_DST as * const u32;
//     for i in 0..16 {
//         let vptr = w.add(i);
//         os_print!("i = {}", *vptr);
//     }
// }
