use crate::marker::PSafe;
use crate::pmem::{Journal, JournalHandle, PMPtr};
use crate::task::{current, is_scheduler_started, ErrorCode};
use crate::util::align_up;
use crate::{board, critical, debug_print, declare_pm_var, declare_pm_var_unsafe, task};
use core::alloc::Layout;
use core::cell::UnsafeCell;
use core::mem::size_of;
use core::ptr;
use core::ptr::NonNull;

const HEAP_SIZE: usize = board::HEAP_SIZE;
pub const PM_HEAP_SIZE_PER_TASK: usize = board::PM_HEAP_SIZE_PER_TASK;
pub const PM_HEAP_SIZE: usize = board!(PM_HEAP_SIZE);
const BOOT_PM_HEAP_SIZE: usize = board::BOOT_PM_HEAP_SIZE;

static mut HEAP_AREA: [u8; HEAP_SIZE] = [0x0; HEAP_SIZE];

// static mut PM_HEAP_AREA: [u8; PM_HEAP_SIZE] = [0x0; PM_HEAP_SIZE];
declare_pm_var_unsafe!(PM_HEAP_AREA, [u8; PM_HEAP_SIZE], [0x0; PM_HEAP_SIZE]);

// static mut BOOT_PM_HEAP_AREA: [u8; BOOT_PM_HEAP_SIZE] = [0x0; BOOT_PM_HEAP_SIZE];
declare_pm_var_unsafe!(
    BOOT_PM_HEAP_AREA,
    [u8; BOOT_PM_HEAP_SIZE],
    [0x0; BOOT_PM_HEAP_SIZE]
);

static mut HEAP: Heap<BumpAllocator> = Heap::new(BumpAllocator::new());
// static mut BOOT_PM_HEAP: PMHeap<PerTaskPMBumpAllocator> = PMHeap::new(PerTaskPMBumpAllocator::new());
declare_pm_var_unsafe!(
    BOOT_PM_HEAP,
    PMHeap<PerTaskPMBumpAllocator>,
    PMHeap::new(PerTaskPMBumpAllocator::new())
);

// static mut GLOBAL_PM_HEAP: GlobalPMHeap = GlobalPMHeap::new(0,0);
declare_pm_var_unsafe!(GLOBAL_PM_HEAP, GlobalPMHeap, GlobalPMHeap::new(0, 0));
// static mut HEAP_FIRST_INIT_DONE: bool = false;
declare_pm_var_unsafe!(HEAP_FIRST_INIT_DONE, bool, false);

struct GlobalPMHeap {
    end: usize,
    next: usize,
}

impl GlobalPMHeap {
    const fn new(start: usize, end: usize) -> Self {
        GlobalPMHeap { end, next: start }
    }

    fn init(&mut self, start: usize, size: usize) {
        self.next = start;
        self.end = start + size;
    }

    pub fn alloc(&mut self, size: usize, j: JournalHandle) -> Result<usize, ErrorCode> {
        critical::with_no_interrupt(|cs| {
            if self.next + size > self.end {
                return Err(ErrorCode::NoSpace);
            }
            j.get_mut().append_log_of(self as *mut GlobalPMHeap);
            let r = self.next;
            self.next += size;
            Ok(r)
        })
    }
}

pub struct Heap<A: Allocator> {
    allocator: A,
}

pub struct PMHeap<PA: PMAllocator> {
    allocator: PA,
}

impl<A> Heap<A>
where
    A: Allocator,
{
    pub const fn new(allocator: A) -> Self {
        Heap { allocator }
    }

    pub fn alloc(&mut self, layout: Layout) -> *mut u8 {
        self.allocator.alloc(layout)
    }
    pub fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        self.allocator.dealloc(ptr, layout);
    }
}

impl<A> PMHeap<A>
where
    A: PMAllocator,
{
    pub const fn new(allocator: A) -> Self {
        PMHeap { allocator }
    }

    pub fn alloc(&mut self, journal: JournalHandle, layout: Layout) -> *mut u8 {
        self.allocator.alloc(journal, layout)
    }
    pub fn dealloc(&mut self, journal: JournalHandle, ptr: *mut u8, layout: Layout) {
        self.allocator.dealloc(journal, ptr, layout);
    }

    pub fn reset(&mut self) {
        self.allocator.reset();
    }

    pub fn stat(&self) -> MemStat {
        self.allocator.stat()
    }
}

pub struct BumpAllocator {
    heap_start: usize,
    heap_end: usize,
    next: usize,
}

pub struct PerTaskPMBumpAllocator {
    bump: BumpAllocator,
}

pub struct MemStat {
    pub mem_used: usize,
    pub mem_left: usize,
}

pub trait Allocator {
    fn alloc(&mut self, layout: Layout) -> *mut u8;
    fn dealloc(&mut self, ptr: *mut u8, layout: Layout);
    fn init(&mut self, heap_start: usize, heap_size: usize);
    fn reset(&mut self);
    fn stat(&self) -> MemStat;
}

pub trait PMAllocator {
    fn alloc(&mut self, journal: JournalHandle, layout: Layout) -> *mut u8;
    fn dealloc(&mut self, journal: JournalHandle, ptr: *mut u8, layout: Layout);
    fn init(&mut self, heap_start: usize, heap_size: usize);
    fn reset(&mut self);
    fn stat(&self) -> MemStat;
}

impl BumpAllocator {
    pub const fn new() -> Self {
        BumpAllocator {
            heap_start: 0,
            heap_end: 0,
            next: 0,
        }
    }

    fn per_task_alloc(&mut self, layout: Layout) -> *mut u8 {
        let alloc_start = align_up(self.next, layout.align());
        // debug_assert!(alloc_start % crate::arch::ARCH_ALIGN == 0);
        let alloc_end = match alloc_start.checked_add(layout.size()) {
            Some(end) => end,
            None => {
                crate::task_print!("No PMEM Left!");
                return ptr::null_mut();
            }
        };
        if alloc_end > self.heap_end {
            crate::task_print!("No PMEM Left!");
            ptr::null_mut() // out of memory
        } else {
            self.next = alloc_end;
            alloc_start as *mut u8
        }
    }
}

impl PerTaskPMBumpAllocator {
    pub const fn new() -> Self {
        PerTaskPMBumpAllocator {
            bump: BumpAllocator::new(),
        }
    }
}

impl Allocator for BumpAllocator {
    fn alloc(&mut self, layout: Layout) -> *mut u8 {
        critical::with_no_interrupt(|_| {
            debug_print!("Alignment: {}", layout.align());
            let alloc_start = align_up(self.next, layout.align());
            debug_print!("start: {}", alloc_start);
            let alloc_end = match alloc_start.checked_add(layout.size()) {
                Some(end) => end,
                None => return ptr::null_mut(),
            };

            if alloc_end > self.heap_end {
                ptr::null_mut() // out of memory
            } else {
                self.next = alloc_end;
                alloc_start as *mut u8
            }
        })
    }

    fn dealloc(&mut self, _ptr: *mut u8, _layout: Layout) {}

    fn init(&mut self, heap_start: usize, heap_size: usize) {
        self.heap_start = heap_start;
        self.heap_end = heap_start + heap_size;
        self.next = heap_start;
        #[cfg(feature = "verbose_os_info")]
        crate::os_print!(
            "Heap initailized, start: {:#X}, end: {:#X}",
            self.heap_start,
            self.heap_end
        );
    }

    fn reset(&mut self) {
        self.next = self.heap_start;
    }

    fn stat(&self) -> MemStat {
        MemStat {
            mem_used: self.next - self.heap_start,
            mem_left: self.heap_end - self.next,
        }
    }
}

impl PMAllocator for PerTaskPMBumpAllocator {
    fn alloc(&mut self, journal: JournalHandle, layout: Layout) -> *mut u8 {
        journal
            .get_mut()
            .append_log_of(self as *mut PerTaskPMBumpAllocator);
        self.bump.per_task_alloc(layout)
    }

    fn dealloc(&mut self, _: JournalHandle, ptr: *mut u8, layout: Layout) {
        self.bump.dealloc(ptr, layout)
    }

    fn init(&mut self, heap_start: usize, heap_size: usize) {
        self.bump.init(heap_start, heap_size);
    }

    fn reset(&mut self) {
        self.bump.reset();
    }

    fn stat(&self) -> MemStat {
        self.bump.stat()
    }
}

pub fn init() {
    unsafe {
        // GLOBAL_BUMP.0 = NonNull::new_unchecked(&mut HEAP.allocator as * mut BumpAllocator);
        if HEAP_SIZE > 0 {
            HEAP.allocator
                .init(&HEAP_AREA[0] as *const u8 as usize, HEAP_SIZE);
        }
        if !HEAP_FIRST_INIT_DONE {
            BOOT_PM_HEAP.allocator.init(
                &BOOT_PM_HEAP_AREA[0] as *const u8 as usize,
                BOOT_PM_HEAP_SIZE,
            );
            GLOBAL_PM_HEAP.init(&PM_HEAP_AREA[0] as *const u8 as usize, PM_HEAP_SIZE);
            HEAP_FIRST_INIT_DONE = true;
        }
    }
}

pub unsafe fn alloc(layout: Layout) -> *mut u8 {
    unsafe { HEAP.allocator.alloc(layout) }
}

pub fn new<T>(object: T) -> Option<NonNull<T>> {
    let layout = Layout::new::<T>();
    let ptr = unsafe { NonNull::new(alloc(layout) as *mut T) };
    match ptr {
        Some(mut p) => unsafe {
            *p.as_mut() = object;
        },
        None => {}
    }

    ptr
}

pub unsafe fn dealloc(ptr: *mut u8, layout: Layout) {
    unsafe {
        HEAP.allocator.dealloc(ptr, layout);
    }
}

pub unsafe fn create_per_task_pm_heap(
    heap: &mut PMHeap<PerTaskPMBumpAllocator>,
    size: usize,
    j: JournalHandle,
) -> Result<(), ()> {
    if size == 0 {
        heap.allocator.init(0, 0);
        return Ok(());
    }
    let start = unsafe {
        match GLOBAL_PM_HEAP.alloc(size, j) {
            Ok(start) => start,
            Err(_) => {
                return Err(());
            }
        }
    };
    heap.allocator.init(start, size);
    Ok(())
}

fn get_pm_heap() -> &'static mut PMHeap<PerTaskPMBumpAllocator> {
    if unsafe { is_scheduler_started() } {
        current().get_pm_heap()
    } else {
        unsafe { &mut BOOT_PM_HEAP }
    }
}

pub unsafe fn pm_new_relaxed<T>(object: T, j: JournalHandle) -> Option<PMPtr<T>> {
    let ptr = unsafe { palloc(j) };
    match ptr {
        Some(p) => unsafe {
            // safe initialization
            let ptr = p.as_ptr();
            core::ptr::write(ptr, object);
        },
        None => {}
    }

    ptr
}

#[inline(always)]
pub fn pm_new<T: PSafe>(object: T, j: JournalHandle) -> Option<PMPtr<T>> {
    unsafe { pm_new_relaxed(object, j) }
}

pub unsafe fn palloc<T>(j: JournalHandle) -> Option<PMPtr<T>> {
    unsafe {
        let p = get_pm_heap().allocator.alloc(j, Layout::new::<T>());
        if p.is_null() {
            None
        } else {
            Some(PMPtr::<T>::new(p as *mut T))
        }
    }
}

pub unsafe fn palloc_array<T>(size: usize, j: JournalHandle) -> Option<NonNull<T>> {
    let layout = match Layout::array::<u8>(size_of::<T>() * size) {
        Err(_) => {
            return None;
        }
        Ok(l) => l,
    };

    let p = get_pm_heap().allocator.alloc(j, layout);
    NonNull::new(p as *mut T)
}

pub unsafe fn pfree_array<T>(ptr: *mut T, size: usize, j: JournalHandle) -> usize {
    let layout = match Layout::array::<u8>(size_of::<T>() * size) {
        Err(_) => {
            return 0;
        }
        Ok(l) => l,
    };

    get_pm_heap().allocator.dealloc(j, ptr as *mut u8, layout);
    return 0;
}

pub unsafe fn pfree<T>(ptr: *mut T, j: JournalHandle) -> usize {
    unsafe {
        get_pm_heap()
            .allocator
            .dealloc(j, ptr as *mut u8, Layout::new::<T>());
    }
    return 0;
}

#[cfg(test)]
pub fn reset_static_vars() {
    unsafe { HEAP_FIRST_INIT_DONE = false };
}
