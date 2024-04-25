use core::mem::{size_of, MaybeUninit};
use core::ptr::NonNull;

use crate::arch::ARCH_ALIGN;
use crate::marker::{TxInSafe, TxOutSafe};
use crate::pmem::{Journal, JournalHandle, PMPtr};
use crate::recover::{finish_ctx_switch_tx, get_boot_tx, in_ctx_switch_tx, start_ctx_switch_tx};
use crate::syscalls::SyscallToken;
use crate::task::{current, get_current_tx, is_scheduler_started, ErrorCode};
use crate::util::debug_syscall_tx_cache;
use crate::{arch, debug_print, os_print};

pub struct Transaction {
    journal: Option<JournalHandle>,
    nesting_level: usize,
    cache: Option<PMPtr<TxCache>>,
}

pub fn get_addr<T>(obj: &T) -> usize {
    assert!(core::mem::size_of::<T>() <= core::mem::size_of::<usize>());
    obj as *const _ as usize
}

impl Transaction {
    pub fn new(j: JournalHandle, c: PMPtr<TxCache>) -> Self {
        Self {
            journal: Some(j),
            nesting_level: 0,
            cache: Some(c),
        }
    }

    pub fn reset_nesting_level(&mut self) {
        self.nesting_level = 0;
    }

    pub fn get_nesting_level(&mut self) -> usize {
        self.nesting_level
    }

    #[inline(always)]
    pub fn set_journal(&mut self, j: &Journal) {
        self.journal = unsafe { Some(JournalHandle::new(j as *const Journal)) };
    }

    #[inline(always)]
    pub fn get_journal(&mut self) -> JournalHandle {
        unsafe { self.journal.unwrap_unchecked() }
    }

    #[inline(always)]
    pub fn set_cache(&mut self, c: &TxCache) {
        self.cache = unsafe { Some(PMPtr::from_ref(c)) };
    }

    #[inline(always)]
    pub fn get_cache(&mut self) -> &mut TxCache {
        unsafe { self.cache.unwrap_unchecked().as_mut_no_logging() }
    }

    pub const fn new_empty() -> Self {
        Self {
            journal: None,
            nesting_level: 0,
            cache: None,
        }
    }

    pub fn debug_commit_invariant_assert(&self) {
        debug_assert!(self.nesting_level == 0, "tx nesting level is not 0");
    }

    pub fn commit<T>(&mut self, res: &T) {
        self.dec_nesting();
        unsafe {
            let tx_cache = self.cache.unwrap_unchecked().as_mut_no_logging();
            tx_cache.cache_result(res);
            self.journal.unwrap_unchecked().get_mut().clear()
        };
    }

    pub fn commit_no_replay(&mut self) {
        self.dec_nesting();
        unsafe {
            let tx_cache = self.cache.unwrap_unchecked().as_mut_no_logging();
            tx_cache.commit_no_replay();
            self.journal.unwrap_unchecked().get_mut().clear()
        };
    }

    pub fn commit_no_replay_roll_forward(&mut self) {
        unsafe {
            self.journal.unwrap_unchecked().get_mut().clear();
            self.reset_nesting_level();
            let tx_cache = self.cache.unwrap_unchecked().as_mut_no_logging();
            tx_cache.commit_no_replay();
        };
    }

    #[inline(always)]
    pub fn check_committed(&mut self) -> Result<(), ()> {
        let cache = self.get_cache();
        if !cache.is_committed() {
            Err(())
        } else {
            self.get_journal().get_mut().clear();
            Ok(())
        }
    }

    pub fn roll_back_if_uncommitted(&mut self) {
        // roll back if uncommitted
        if let Err(_) = self.check_committed() {
            self.get_journal().get_mut().recover();
        }
    }

    #[inline(always)]
    pub fn roll_back(&mut self) {
        // roll back
        self.get_journal().get_mut().recover();
    }

    #[inline(always)]
    pub fn try_get_cached_result<T>(&mut self) -> Result<T, ()> {
        let tx_cache = self.get_cache();
        tx_cache.try_retrieve_result()
    }

    #[inline(always)]
    fn clear_commit_flag(&mut self) {
        let tx_cache = self.get_cache();
        tx_cache.tail &= !TX_COMMITTED;
    }

    #[inline(always)]
    fn inc_nesting(&mut self) {
        #[cfg(debug_assertions)]
        {
            self.nesting_level += 1;
            debug_assert_eq!(self.nesting_level, 1);
        }
    }

    #[inline(always)]
    fn dec_nesting(&mut self) {
        #[cfg(debug_assertions)]
        {
            self.nesting_level -= 1;
            self.debug_commit_invariant_assert();
        }
    }

    pub fn begin(&mut self) {
        self.inc_nesting();
        self.clear_commit_flag();
    }

    #[inline(always)]
    pub fn try_run<F, T>(&mut self, f: F) -> Result<T, ErrorCode>
    where
        F: FnOnce(JournalHandle) -> Result<T, ErrorCode>,
    {
        self.begin();
        let ret = f(unsafe { self.journal.unwrap_unchecked() });
        match ret {
            Err(ErrorCode::TxRetry) => {
                self.commit_no_replay();
            }
            _ => {
                self.commit(&ret);
            }
        };
        ret
    }

    #[inline(always)]
    pub fn run<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce(JournalHandle) -> T,
    {
        self.begin();
        let ret = f(unsafe { self.journal.unwrap_unchecked() });
        self.commit(&ret);
        ret
    }

    #[inline(always)]
    pub fn run_sys<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce(JournalHandle, SyscallToken) -> T,
    {
        self.begin();
        let ret = f(unsafe { self.journal.unwrap_unchecked() }, unsafe {
            SyscallToken::new()
        });
        self.commit(&ret);
        ret
    }

    #[inline(always)]
    pub fn fast_run<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        self.begin();
        let ret = f();
        // commit without resetting the journal
        self.dec_nesting();
        let tx_cache = unsafe { self.cache.unwrap_unchecked().as_mut_no_logging() };
        tx_cache.cache_result(&ret);
        ret
    }

    #[inline(always)]
    pub fn run_pure_sys<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce(SyscallToken) -> T,
    {
        self.begin();
        let ret = f(unsafe { SyscallToken::new() });
        // commit without resetting the journal
        self.dec_nesting();
        let tx_cache = unsafe { self.cache.unwrap_unchecked().as_mut_no_logging() };
        tx_cache.cache_result(&ret);
        ret
    }

    #[cfg(feature = "crash_safe")]
    pub fn run_no_replay_sys<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce(JournalHandle, SyscallToken) -> T,
    {
        self.begin();
        let ret = f(unsafe { self.journal.unwrap_unchecked() }, unsafe {
            SyscallToken::new()
        });
        self.commit_no_replay();
        ret
    }

    #[cfg(feature = "crash_safe")]
    pub fn run_no_replay<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce(JournalHandle) -> T,
    {
        self.begin();
        let ret = f(unsafe { self.journal.unwrap_unchecked() });
        self.commit_no_replay();
        ret
    }

    #[cfg(not(feature = "crash_safe"))]
    pub fn run_no_replay<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce(JournalHandle) -> T,
    {
        let ret = f(unsafe { self.journal.unwrap_unchecked() });
        ret
    }

    /* Testing Interfaces */
    #[cfg(test)]
    #[inline(always)]
    pub fn may_crashed_run_sys<F, T>(&mut self, crash: bool, f: F) -> T
    where
        F: FnOnce(JournalHandle, SyscallToken) -> T,
    {
        self.begin();
        let ret = f(unsafe { self.journal.unwrap_unchecked() }, unsafe {
            SyscallToken::new()
        });
        if !crash {
            self.commit(&ret);
        }
        ret
    }
    #[cfg(test)]
    #[inline(always)]
    pub fn may_crashed_run<F, T>(&mut self, crash: bool, f: F) -> T
    where
        F: FnOnce(JournalHandle) -> T,
    {
        self.begin();
        let ret = f(unsafe { self.journal.unwrap_unchecked() });
        if !crash {
            self.commit(&ret);
        }
        ret
    }

    #[cfg(test)]
    pub fn may_crashed_run_no_replay<F, T>(&mut self, crash: bool, f: F) -> T
    where
        F: FnOnce(JournalHandle) -> T,
    {
        self.begin();
        let ret = f(unsafe { self.journal.unwrap_unchecked() });
        if !crash {
            self.commit_no_replay();
        }
        ret
    }

    #[cfg(test)]
    pub fn may_crashed_run_no_replay_sys<F, T>(&mut self, crash: bool, f: F) -> T
    where
        F: FnOnce(JournalHandle, SyscallToken) -> T,
    {
        self.begin();
        let ret = f(unsafe { self.journal.unwrap_unchecked() }, unsafe {
            SyscallToken::new()
        });
        if !crash {
            self.commit_no_replay();
        }
        ret
    }
}

fn debug_tx_exit_assert<T>(r: &Result<T, ErrorCode>) {
    debug_assert!(
        !(match r {
            Err(ErrorCode::TxRetry) => true,
            _ => false,
        }),
        "Tx can't return TXRetry here"
    );
}

#[cfg(feature = "crash_safe")]
#[inline(always)]
pub unsafe fn try_run_relaxed<F, T>(f: F) -> Result<T, ErrorCode>
where
    F: FnOnce(JournalHandle) -> Result<T, ErrorCode>,
{
    let current_tx = get_current_tx();
    // if current_tx.get_nesting_level() == 0 {
    debug_assert_eq!(current_tx.get_nesting_level(), 0);
    if let Ok(cache_res) = current_tx.try_get_cached_result() {
        debug_print!("bypassing sys TX...");
        debug_assert!(
            !unsafe { is_scheduler_started() } || current().in_recovery_mode(),
            "Can't bypass TX when not recovering"
        );
        cache_res
    } else {
        current_tx.try_run(f)
    }
    // } else {
    //     let r = f(unsafe {current_tx.get_journal()});
    // debug_tx_exit_assert(&r);
    //     r
    // }
}

#[cfg(feature = "crash_safe")]
#[inline(always)]
pub unsafe fn run_relaxed<F, T>(f: F) -> T
where
    F: FnOnce(JournalHandle) -> T,
{
    let current_tx = get_current_tx();
    // if current_tx.get_nesting_level()== 0 {
    debug_assert_eq!(current_tx.get_nesting_level(), 0);
    if let Ok(cache_res) = current_tx.try_get_cached_result() {
        debug_print!("bypassing sys TX...");
        debug_assert!(
            !unsafe { is_scheduler_started() } || current().in_recovery_mode(),
            "Can't bypass TX when not recovering"
        );
        cache_res
    } else {
        current_tx.run(f)
    }
    // } else {
    //     let r = f(unsafe {current_tx.get_journal()});
    //     r
    // }
}

#[cfg(feature = "crash_safe")]
#[inline(always)]
pub fn try_run<F, T>(f: F) -> Result<T, ErrorCode>
where
    F: TxInSafe + FnOnce(JournalHandle) -> Result<T, ErrorCode>,
    T: TxOutSafe,
{
    unsafe { try_run_relaxed(f) }
}

#[cfg(feature = "crash_safe")]
#[inline(always)]
pub fn run<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle) -> T,
    T: TxOutSafe,
{
    unsafe { run_relaxed(f) }
}

#[cfg(feature = "crash_safe")]
#[inline(always)]
pub fn fast_run<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce() -> T,
    T: TxOutSafe,
{
    let current_tx = get_current_tx();

    if let Ok(cache_res) = current_tx.try_get_cached_result() {
        debug_print!("bypassing sys TX...");
        debug_assert!(
            !unsafe { is_scheduler_started() } || current().in_recovery_mode(),
            "Can't bypass TX when not recovering"
        );
        cache_res
    } else {
        current_tx.fast_run(f)
    }
}

#[cfg(feature = "crash_safe")]
// There is no nesting for this use case
#[inline(always)]
pub fn run_no_ctx<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle) -> T,
    T: TxOutSafe,
{
    let tx = get_boot_tx();
    start_ctx_switch_tx();
    let r = tx.run_no_replay(f);
    finish_ctx_switch_tx();
    debug_assert!(!in_ctx_switch_tx());
    r
}

#[cfg(feature = "crash_safe")]
#[inline(always)]
pub fn run_in_loop<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle) -> T,
    T: TxOutSafe,
{
    let tx = get_current_tx();
    tx.run_no_replay(f)
}

/* Testing Interfaces */
#[cfg(all(feature = "crash_safe", test))]
#[inline(always)]
pub fn may_crashed_run<F, T>(crash: bool, f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle) -> T,
    T: TxOutSafe,
{
    let current_tx = get_current_tx();
    // if current_tx.nesting_level == 0 {
    debug_assert_eq!(current_tx.get_nesting_level(), 0);
    if let Ok(cache_res) = current_tx.try_get_cached_result() {
        debug_print!("bypassing sys TX...");
        debug_assert!(
            !unsafe { is_scheduler_started() } || current().in_recovery_mode(),
            "Can't bypass TX when not recovering"
        );
        cache_res
    } else {
        current_tx.may_crashed_run(crash, f)
    }
    // } else {
    //     let r = f(unsafe {current_tx.get_journal()});
    //     r
    // }
}

/* ------------- crash unsafe version of TX for comparison ---------------------*/
#[cfg(not(feature = "crash_safe"))]
#[inline(always)]
pub unsafe fn try_run_relaxed<F, T>(f: F) -> Result<T, ErrorCode>
where
    F: FnOnce(JournalHandle) -> Result<T, ErrorCode>,
{
    let current_tx = get_current_tx();
    f(unsafe { current_tx.get_journal() })
}

#[cfg(not(feature = "crash_safe"))]
#[inline(always)]
pub unsafe fn run_relaxed<F, T>(f: F) -> T
where
    F: FnOnce(JournalHandle) -> T,
{
    let current_tx = get_current_tx();
    f(unsafe { current_tx.get_journal() })
}

#[cfg(not(feature = "crash_safe"))]
#[inline(always)]
pub fn try_run<F, T>(f: F) -> Result<T, ErrorCode>
where
    F: TxInSafe + FnOnce(JournalHandle) -> Result<T, ErrorCode>,
    T: TxOutSafe,
{
    unsafe { try_run_relaxed(f) }
}

#[cfg(not(feature = "crash_safe"))]
#[inline(always)]
pub fn run<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle) -> T,
    T: TxOutSafe,
{
    unsafe { run_relaxed(f) }
}

#[cfg(not(feature = "crash_safe"))]
#[inline(always)]
pub fn fast_run<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce() -> T,
    T: TxOutSafe,
{
    f()
}

#[cfg(not(feature = "crash_safe"))]
// There is no nesting for this use case
#[inline(always)]
pub fn run_no_ctx<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle) -> T,
    T: TxOutSafe,
{
    let tx = get_boot_tx();
    let r = tx.run_no_replay(f);
    r
}

#[cfg(not(feature = "crash_safe"))]
#[inline(always)]
pub fn run_in_loop<F, T>(f: F) -> T
where
    F: TxInSafe + FnOnce(JournalHandle) -> T,
    T: TxOutSafe,
{
    let tx = get_current_tx();
    tx.run_no_replay(f)
}

#[cfg(all(board="msp430fr5994", not(sram_baseline)))]
const TX_CACHE_SZ: usize = 32; // for train

#[cfg(all(not(board="msp430fr5994"), not(sram_baseline)))]
const TX_CACHE_SZ: usize = 64; 
//const TX_CACHE_SZ: usize = 128;

#[cfg(sram_baseline)]
const TX_CACHE_SZ: usize = 0;

#[cfg(target_pointer_width = "32")]
const TX_COMMITTED: usize = 0x80000000;
#[cfg(target_pointer_width = "32")]
const TX_CACHE_PTR_MASK: usize = 0x7FFF0000;
#[cfg(target_pointer_width = "32")]
const TX_CACHE_PTR_SHIFT: usize = 16;
#[cfg(target_pointer_width = "32")]
const TX_ID_PTR_MASK: usize = 0x0000FFFF;

#[cfg(any(target_pointer_width = "16", target_pointer_width = "64"))]
const TX_COMMITTED: usize = 0x8000;
#[cfg(any(target_pointer_width = "16", target_pointer_width = "64"))]
const TX_CACHE_PTR_MASK: usize = 0x7F00;
#[cfg(any(target_pointer_width = "16", target_pointer_width = "64"))]
const TX_CACHE_PTR_SHIFT: usize = 8;
#[cfg(any(target_pointer_width = "16", target_pointer_width = "64"))]
const TX_ID_PTR_MASK: usize = 0x00FF;

//  TX Ptr & Tail structure
//  | C | 15 bits | 16 bits |

pub struct TxCache {
    cache: [u8; TX_CACHE_SZ],
    ptr: usize,
    tail: usize,
}

impl TxCache {
    pub fn init(&mut self) {
        self.ptr = 0;
        self.tail = 0;
    }

    pub const fn new() -> Self {
        Self {
            cache: [0; TX_CACHE_SZ],
            ptr: 0,
            tail: 0,
        }
    }

    #[inline(always)]
    pub fn is_committed(&self) -> bool {
        (self.tail & TX_COMMITTED) == TX_COMMITTED
    }

    #[inline(always)]
    pub fn reset_ptr(&mut self) {
        self.ptr = 0;
    }

    #[inline(always)]
    pub fn set_tail_ptr(&mut self, pos: usize) {
        self.ptr = pos;
        self.tail = pos | TX_COMMITTED;
    }

    pub fn set_ptr(&mut self, new_pos: usize) {
        self.ptr = new_pos;
    }

    #[inline(always)]
    pub fn commit_reset_tail(&mut self) {
        self.tail = TX_COMMITTED;
    }

    #[inline(always)]
    pub fn reset_tail(&mut self) {
        self.tail = 0;
    }

    #[inline(always)]
    pub fn set_tail(&mut self, new_pos: usize) {
        self.tail = new_pos;
    }

    #[inline(always)]
    pub fn commit_no_ret_value(&mut self) {
        // advance tx id
        #[cfg(feature = "opt_tx_cache_space")]
        {
            let old_tail = self.tail;
            let new_tail = (old_tail + 1) | TX_COMMITTED;
            // atomic commit
            self.tail = new_tail;
            // advance tx id of ptr
            self.ptr += 1;
        }
        #[cfg(not(feature = "opt_tx_cache_space"))]
        {
            let old_tail = self.tail & !TX_COMMITTED;
            let new_tail = (old_tail + ARCH_ALIGN);
            // atomic commit
            self.tail = new_tail | TX_COMMITTED;
            // advance tx  ptr
            self.ptr = new_tail;
        }
    }

    #[inline(always)]
    pub fn commit_no_replay(&mut self) {
        self.tail |= TX_COMMITTED;
    }

    #[inline(always)]
    pub fn get_tx_id_of_tail(&self) -> usize {
        #[cfg(feature = "opt_tx_cache_space")]
        {
            return self.tail & TX_ID_PTR_MASK;
        }
        #[cfg(not(feature = "opt_tx_cache_space"))]
        {
            return self.tail & !TX_COMMITTED;
        }
    }

    #[inline(always)]
    pub fn get_tx_cache_ptr_of_tail(&self) -> usize {
        #[cfg(feature = "opt_tx_cache_space")]
        {
            return (self.tail & TX_CACHE_PTR_MASK) >> TX_CACHE_PTR_SHIFT;
        }
        #[cfg(not(feature = "opt_tx_cache_space"))]
        {
            return self.tail & !TX_COMMITTED;
        }
    }

    #[inline(always)]
    pub fn get_tx_cache_ptr_of_ptr(&self) -> usize {
        #[cfg(feature = "opt_tx_cache_space")]
        {
            return (self.ptr & TX_CACHE_PTR_MASK) >> TX_CACHE_PTR_SHIFT;
        }
        #[cfg(not(feature = "opt_tx_cache_space"))]
        {
            return self.ptr;
        }
    }

    #[inline(always)]
    pub fn get_tx_id_of_ptr(&self) -> usize {
        #[cfg(feature = "opt_tx_cache_space")]
        {
            return self.ptr & TX_ID_PTR_MASK;
        }
        #[cfg(not(feature = "opt_tx_cache_space"))]
        {
            return self.ptr;
        }
    }

    #[inline(always)]
    pub fn advance_composite_id(
        &self,
        tx_id: usize,
        cache_ptr: usize,
        cached_ret_val_sz: usize,
    ) -> usize {
        #[cfg(feature = "opt_tx_cache_space")]
        {
            let new_id = tx_id + 1;
            let new_cache_ptr = cache_ptr + cached_ret_val_sz;
            return (new_id | (new_cache_ptr << TX_CACHE_PTR_SHIFT));
        }
        #[cfg(not(feature = "opt_tx_cache_space"))]
        {
            return (tx_id + cached_ret_val_sz);
        }
    }
    #[inline(always)]
    fn advance_ptr_on_empty(&mut self) {
        #[cfg(feature = "opt_tx_cache_space")]
        {
            self.ptr += 1;
        }
        #[cfg(not(feature = "opt_tx_cache_space"))]
        {
            self.ptr += ARCH_ALIGN;
        }
    }
    #[inline(always)]
    pub fn cache_result<T>(&mut self, res: &T) {
        let rec_size = size_of::<T>();
        if rec_size > 0 {
            let cache_ptr = self.get_tx_cache_ptr_of_tail();
            debug_assert!(
                cache_ptr + rec_size <= TX_CACHE_SZ,
                "Running out of cache size, tail: {}, object type: {}",
                cache_ptr,
                core::any::type_name::<T>()
            );
            unsafe {
                // *(self.cache[self.tail] as * mut usize) = res_len;
                let dst = &mut self.cache[cache_ptr] as *mut u8 as *mut T;
                core::ptr::copy_nonoverlapping(res as *const T, dst, 1);
            }
            let tx_id = self.get_tx_id_of_tail();
            let advanced_tail = self.advance_composite_id(tx_id, cache_ptr, rec_size);
            let new_tail_val = advanced_tail | TX_COMMITTED;
            // atomic commit
            self.tail = new_tail_val;
            self.ptr = advanced_tail;
        } else {
            self.commit_no_ret_value();
        }
    }

    #[inline(always)]
    pub fn try_retrieve_result<T>(&mut self) -> Result<T, ()> {
        if self.get_tx_id_of_ptr() < self.get_tx_id_of_tail() {
            let rec_sz = size_of::<T>();
            let res = if rec_sz > 0 {
                let cache_ptr = self.get_tx_cache_ptr_of_ptr();
                let r = unsafe {
                    let src = &self.cache[cache_ptr] as *const u8 as *const T;
                    core::ptr::read(src)
                    // core::ptr::copy_nonoverlapping(src, res as * mut T, 1);
                };
                let tx_id = self.get_tx_id_of_ptr();
                let advanced_ptr = self.advance_composite_id(tx_id, cache_ptr, rec_sz);
                self.ptr = advanced_ptr;
                r
            } else {
                // increase tx_id of the ptr
                let empty = unsafe { core::ptr::read(&self.cache[0] as *const u8 as *const T) };
                self.advance_ptr_on_empty();
                empty
            };
            Ok(res)
        } else {
            Err(())
        }
    }
}

const DEFAULT_STACK_DEPTH: usize = 8;
type IdemTail = u16;
type TxTail = u16;

#[derive(PartialEq, Eq)]
pub enum PendingExitOp {
    None = 0,
    ExitIdem,
    ExitLoop,
}

#[cfg(not(feature = "nested_idem_regions"))]
pub struct UserTxInfo {
    user_tx: Transaction,
    user_journal: Journal,
    user_tx_cache: TxCache,
    stack_top: u8,
    tail_stack: [TxTail; DEFAULT_STACK_DEPTH],
    #[cfg(feature = "opt_loop_end")]
    loop_cnt_log: usize,
    #[cfg(feature = "opt_loop_end")]
    loop_cnt_ptr: Option<NonNull<usize>>,
    #[cfg(feature = "opt_loop_end")]
    step: usize,
}

#[cfg(not(feature = "nested_idem_regions"))]
impl UserTxInfo {
    pub fn get_tx(&mut self) -> &mut Transaction {
        &mut self.user_tx
    }

    pub fn get_journal(&mut self) -> JournalHandle {
        unsafe { JournalHandle::new(&self.user_journal as *const Journal) }
    }

    pub fn get_tx_cache(&mut self) -> &mut TxCache {
        &mut self.user_tx_cache
    }

    pub fn restart(&mut self) {
        // reset tx cache ptr
        self.user_tx_cache.reset_ptr();
        // reset stack top
        self.stack_top = 0;
    }

    pub fn reset_all(&mut self) {
        self.user_tx_cache.reset_tail();
        self.user_tx_cache.reset_ptr();
    }

    pub fn init(&mut self) {
        self.user_tx.set_cache(&self.user_tx_cache);
        self.user_tx.set_journal(&self.user_journal);
        self.user_tx.reset_nesting_level();
        self.user_journal.init();
        self.user_tx_cache.init();
        self.stack_top = 0;
        self.tail_stack = [0; DEFAULT_STACK_DEPTH];
        #[cfg(feature = "opt_loop_end")]
        {
            self.loop_cnt_ptr = None;
        }
    }

    pub fn enter_idempotent(&mut self) -> bool {
        true
    }

    pub fn show_user_tx_idem_status(&self) {
        let cache = &self.user_tx_cache;
        let tx_tail = cache.get_tx_id_of_tail();
        let tx_ptr = cache.get_tx_id_of_ptr();
        let depth = self.stack_top as usize;

        debug_print!(
            "Nesting level: {}, tx tail: {}, tx ptr: {}",
            depth,
            tx_tail,
            tx_ptr
        );
    }

    pub fn exit_idempotent(&mut self) {}

    #[cfg(feature = "opt_loop_end")]
    pub fn log_loop_cnt(&mut self, ptr: *mut usize, old_cnt: usize, step: usize) {
        self.loop_cnt_ptr = unsafe { Some(NonNull::new_unchecked(ptr)) };
        self.loop_cnt_log = old_cnt;
        self.step = step;
    }

    #[cfg(feature = "opt_loop_end")]
    pub fn set_loop_cnt(&mut self) {
        unsafe {
            *self.loop_cnt_ptr.unwrap_unchecked().as_mut() = self.loop_cnt_log + self.step;
        }
    }

    #[cfg(feature = "crash_safe")]
    pub fn enter_idempotent_loop(&mut self) {
        // let tx_tail = self.user_tx_cache.tail;
        // self.tail_stack[self.stack_top as usize] = tx_tail as u16;
        let tx_tail = self.user_tx_cache.ptr | TX_COMMITTED;
        self.tail_stack[self.stack_top as usize] = tx_tail as u16;
        self.stack_top += 1;
    }

    #[cfg(feature = "crash_safe")]
    pub fn exit_idempotent_loop(&mut self) {
        // 1. recover old tx tail & ptr (non-idempotent)
        // 2. recover old idempotent tail & ptr (non-idempotent)
        // 3. decrease stack top (non-idempotent)
        // This will be a roll-forward atomic operation
        let top = self.stack_top as usize;
        let old_tx_tail = self.tail_stack[top - 1];
        self.user_tx_cache.set_tail(old_tx_tail as usize);
        // These are non-persistent states
        // reset outer ptr
        #[cfg(feature = "opt_loop_end")]
        self.set_loop_cnt();
        #[cfg(feature = "opt_tx_cache_space")]
        self.user_tx_cache.set_ptr(old_tx_tail as usize);
        #[cfg(not(feature = "opt_tx_cache_space"))]
        self.user_tx_cache
            .set_ptr(old_tx_tail as usize & !TX_COMMITTED);
        self.stack_top -= 1;
    }

    #[cfg(feature = "crash_safe")]
    pub fn exit_infinite_idempotent_loop(&mut self) {
        // 1. recover old tx tail & ptr (non-idempotent)
        // 2. recover old idempotent tail & ptr (non-idempotent)
        // 3. decrease stack top (non-idempotent)
        // This will be a roll-forward atomic operation
        let top = self.stack_top as usize;
        let old_tx_tail = self.tail_stack[top - 1];
        self.user_tx_cache.set_tail(old_tx_tail as usize);
        // These are non-persistent states
        // reset outer ptr
        #[cfg(feature = "opt_tx_cache_space")]
        self.user_tx_cache.set_ptr(old_tx_tail as usize);
        #[cfg(not(feature = "opt_tx_cache_space"))]
        self.user_tx_cache
            .set_ptr(old_tx_tail as usize & !TX_COMMITTED);
        self.stack_top -= 1;
    }

    #[cfg(not(feature = "crash_safe"))]
    pub fn enter_idempotent_loop(&mut self) {}

    #[cfg(not(feature = "crash_safe"))]
    pub fn exit_idempotent_loop(&mut self) {}

    #[cfg(not(feature = "crash_safe"))]
    pub fn exit_infinite_idempotent_loop(&mut self) {}

    pub fn get_stack_top(&self) -> u8 {
        self.stack_top
    }

    pub fn get_tx_tail_id_at_level(&self, level: usize) -> usize {
        self.tail_stack[level] as usize & TX_ID_PTR_MASK
    }

    /* Interfaces for crash injection tests */
    #[cfg(test)]
    #[cfg(feature = "crash_safe")]
    pub fn crashed_log_loop_cnt(&mut self, ptr: *mut usize, old_cnt: usize, step: usize) {
        use crate::crash_point;
        crash_point!(tx_loop, 0);
        self.loop_cnt_ptr = unsafe { Some(NonNull::new_unchecked(ptr)) };
        crash_point!(tx_loop, 1);
        self.loop_cnt_log = old_cnt;
        crash_point!(tx_loop, 2);
        self.step = step;
        crash_point!(tx_loop, 3);
    }

    #[cfg(test)]
    #[cfg(feature = "crash_safe")]
    pub fn crashed_exit_idempotent_loop(&mut self) {
        use crate::crash_point;
        crash_point!(tx_loop, 0);
        let top = self.stack_top as usize;
        let old_tx_tail = self.tail_stack[top - 1];
        crash_point!(tx_loop, 1);
        self.user_tx_cache.set_tail(old_tx_tail as usize);
        crash_point!(tx_loop, 2);
        #[cfg(feature = "opt_loop_end")]
        self.set_loop_cnt();
        crash_point!(tx_loop, 3);
        #[cfg(feature = "opt_tx_cache_space")]
        {
            self.user_tx_cache.set_ptr(old_tx_tail as usize);
            crash_point!(tx_loop, 4);
        }
        #[cfg(not(feature = "opt_tx_cache_space"))]
        {
            self.user_tx_cache
                .set_ptr(old_tx_tail as usize & !TX_COMMITTED);
            crash_point!(tx_loop, 4);
        }
        self.stack_top -= 1;
        crash_point!(tx_loop, 5);
    }
}

#[cfg(test)]
static mut TX_LOOP_CRASH_POINT: usize = 0;

#[cfg(test)]
pub fn set_tx_loop_crash_point(cp: usize) {
    unsafe { TX_LOOP_CRASH_POINT = cp };
}

#[cfg(test)]
pub fn get_tx_loop_crash_point() -> usize {
    unsafe { TX_LOOP_CRASH_POINT }
}

// #[cfg(feature="nested_idem_regions")]
// pub struct UserTxInfo {
//     user_tx: Transaction,
//     user_journal: Journal,
//     user_tx_cache: TxCache,
//     user_idem_region_ptr: IdemTail,
//     stack_top: u8,
//     exit_op: PendingExitOp,
//     old_outer_tail: IdemTail,
//     tail_stack: [(TxTail, IdemTail); DEFAULT_STACK_DEPTH],
// }
// #[cfg(feature="nested_idem_regions")]
// impl UserTxInfo {
//     pub fn get_tx(&mut self) -> &mut Transaction {
//         &mut self.user_tx
//     }

//     pub fn get_journal(&mut self) -> JournalHandle {
//         unsafe {
//             JournalHandle::new(PMPtr::from_mut_ref(&mut self.user_journal))
//         }
//     }

//     pub fn get_tx_cache(&mut self) -> &mut TxCache {
//         &mut self.user_tx_cache
//     }

//     pub fn restart(&mut self) {
//         // reset tx cache ptr & idem region ptr
//         self.user_idem_region_ptr = 0;
//         self.user_tx_cache.reset_ptr();
//         // reset stack top
//         self.stack_top = 0;
//     }

//     pub fn init(&mut self) {
//         self.user_tx.set_cache(&self.user_tx_cache);
//         self.user_tx.set_journal(&self.user_journal);
//         self.user_tx.reset_nesting_level();
//         self.user_journal.init();
//         self.user_tx_cache.init();
//         self.user_idem_region_ptr = 0;
//         // self.user_idempotent_level = 0;
//         self.stack_top = 0;
//         self.exit_op = PendingExitOp::None;
//         self.tail_stack = [(0,0); DEFAULT_STACK_DEPTH];
//     }

//     pub fn get_idem_tail(&self) -> IdemTail {
//         self.tail_stack[self.stack_top as usize].1 as IdemTail
//     }

//     pub fn enter_idempotent(&mut self) -> bool {
//         if self.user_idem_region_ptr < self.get_idem_tail() {
//             debug_assert!(current().in_recovery_mode(), "Can't bypass idempotent region when not recovering");
//             self.user_idem_region_ptr += 1;
//             return false;
//         }
//         // save old tx tail & idem tail
//         let tx_tail = self.user_tx_cache.tail;
//         // TODO() this should be an atomic operation, (u16,u16) = u32;
//         // not sure if compiler will generate an atomic op
//         self.tail_stack[self.stack_top as usize].0 = tx_tail as u16;
//         self.stack_top += 1;
//         self.user_idem_region_ptr = 0;
//         return true;
//     }

//     pub fn show_user_tx_idem_status(&self) {
//         let cache =  &self.user_tx_cache;
//         let tx_tail = cache.get_tail();
//         let tx_ptr = cache.get_ptr();
//         let depth = self.stack_top as usize;
//         let idem_tail = self.tail_stack[depth].1;
//         let idem_ptr = self.user_idem_region_ptr;

//         debug_print!("Nesting level: {}, idem region tail: {}, idem region ptr: {}, tx tail: {}, tx ptr: {}"
//                     , depth, idem_tail, idem_ptr, tx_tail, tx_ptr);
//     }

//     pub fn exit_idempotent(&mut self) {
//         // 1. recover old tx tail & ptr (non-idempotent)
//         // 2. recover old idempotent tail & ptr (non-idempotent)
//         // 3. decrease stack top (non-idempotent)
//         // This will be a roll-forward atomic operation
//         let top = self.stack_top as usize;
//         self.old_outer_tail = self.tail_stack[top - 1].1;
//         // fence()
//         self.exit_op = PendingExitOp::ExitIdem;
//         // fence()
//         let (old_tx_tail, old_idem_tail) =self.tail_stack[top - 1];
//         self.user_tx_cache.set_tail(old_tx_tail as usize);
//         // increase outer tail
//         self.tail_stack[top - 1].1 = old_idem_tail + 1; // non-idempotent..
//         // reset inner tail to 0
//         self.tail_stack[top].1 = 0;
//         // fence()
//         self.exit_op = PendingExitOp::None;
//         // These are half-persistent states
//         // reset outer ptr
//         self.user_tx_cache.set_ptr(old_tx_tail as usize);
//         self.user_idem_region_ptr = self.tail_stack[top - 1].1;
//         self.stack_top -= 1;

//     }

//     pub fn recover_from_potential_crash_when_exiting_idempotent_or_loop(&mut self) {
//         if (self.exit_op== PendingExitOp::ExitIdem) {
//             // stack top will be intact when crash happened
//             let top = self.stack_top as usize;
//             let (old_tx_tail, _) =self.tail_stack[top - 1];
//             self.user_tx_cache.set_tail(old_tx_tail as usize);
//             // increase outer tail
//             self.tail_stack[top - 1].1 = self.old_outer_tail + 1;
//             // reset inner tail to 0
//             self.tail_stack[top].1 = 0;
//             // fence()
//             self.exit_op = PendingExitOp::None;
//         } else if (self.exit_op == PendingExitOp::ExitLoop) {
//             let top = self.stack_top as usize;
//             let (old_tx_tail, _) =self.tail_stack[top - 1];
//             self.user_tx_cache.set_tail(old_tx_tail as usize);
//             // reset inner tail to 0
//             self.tail_stack[top].1 = 0;
//             // fence()
//             self.exit_op = PendingExitOp::None;
//         }
//     }

//     pub fn enter_idempotent_loop(&mut self) {
//             let tx_tail = self.user_tx_cache.tail;
//             self.tail_stack[self.stack_top as usize].0 = tx_tail as u16;
//             self.stack_top += 1;
//             self.user_idem_region_ptr = 0;
//     }

//     pub fn exit_idempotent_loop(&mut self) {
//         // 1. recover old tx tail & ptr (non-idempotent)
//         // 2. recover old idempotent tail & ptr (non-idempotent)
//         // 3. decrease stack top (non-idempotent)
//         // This will be a roll-forward atomic operation
//         let top = self.stack_top as usize;
//         // fence()
//         self.exit_op= PendingExitOp::ExitLoop;
//         // fence()
//         let old_tx_tail =self.tail_stack[top - 1].0;
//         self.user_tx_cache.set_tail(old_tx_tail as usize);
//         // reset inner tail to 0
//         self.tail_stack[top].1 = 0;
//         // fence()
//         self.exit_op= PendingExitOp::None;
//         // These are non-persistent states
//         // reset outer ptr
//         self.user_tx_cache.set_ptr(old_tx_tail as usize);
//         self.user_idem_region_ptr = self.tail_stack[top - 1].1;
//         self.stack_top -= 1;
//     }
// }
