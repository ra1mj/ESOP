//! Per-thread allocation evidence for the activated master call path. Hosted
//! setup/export and other threads are outside the measured ownership scope.
use serde::Serialize;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    static DEALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    static REALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

#[derive(Clone, Copy, Debug, Default, Serialize, Eq, PartialEq)]
pub struct AllocationCounts {
    pub allocations: u64,
    pub deallocations: u64,
    pub reallocations: u64,
}
impl AllocationCounts {
    pub const fn total(self) -> u64 {
        self.allocations
            .saturating_add(self.deallocations)
            .saturating_add(self.reallocations)
    }
}
pub fn counts() -> AllocationCounts {
    AllocationCounts {
        allocations: ALLOCATIONS.with(Cell::get),
        deallocations: DEALLOCATIONS.with(Cell::get),
        reallocations: REALLOCATIONS.with(Cell::get),
    }
}
pub fn reset() {
    ALLOCATIONS.with(|count| count.set(0));
    DEALLOCATIONS.with(|count| count.set(0));
    REALLOCATIONS.with(|count| count.set(0));
}
pub struct ActivatedScope {
    previous: bool,
}
pub fn enter() -> ActivatedScope {
    ActivatedScope {
        previous: ACTIVE.with(|active| active.replace(true)),
    }
}
impl Drop for ActivatedScope {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.set(self.previous));
    }
}
fn record(counter: &'static std::thread::LocalKey<Cell<u64>>) {
    if ACTIVE.try_with(Cell::get).unwrap_or(false) {
        let _ = counter.try_with(|count| count.set(count.get().saturating_add(1)));
    }
}
pub struct CountingAllocator;
// SAFETY: every operation delegates to System with the exact original layout
// and pointer. Evidence uses only constant-initialized, non-allocating TLS.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(&ALLOCATIONS);
        // SAFETY: delegated GlobalAlloc contract from the caller.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(&ALLOCATIONS);
        // SAFETY: delegated GlobalAlloc contract from the caller.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        record(&DEALLOCATIONS);
        // SAFETY: delegated GlobalAlloc contract from the caller.
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(&REALLOCATIONS);
        // SAFETY: delegated GlobalAlloc contract from the caller.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_counts_all_heap_operations_and_restores_nested_state() {
        reset();
        {
            let _outer = enter();
            let mut bytes = Vec::<u8>::with_capacity(1);
            bytes.push(1);
            {
                let _inner = enter();
                bytes.reserve(128);
            }
            drop(bytes);
        }
        let measured = counts();
        assert_eq!(measured.allocations, 1);
        assert_eq!(measured.reallocations, 1);
        assert_eq!(measured.deallocations, 1);
        drop(Vec::<u8>::with_capacity(16));
        assert_eq!(counts(), measured);
    }
}
