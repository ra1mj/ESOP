//! Caller-owned fixed arena for configuration-time allocation.
//!
//! The arena is deliberately backed by caller-provided `usize` storage so its
//! base address is aligned for the common protocol/core types. Allocation is
//! only valid before activation; callers freeze the arena once all runtime
//! plans and state have been built.

use core::mem::{MaybeUninit, align_of, size_of};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArenaError {
    AlignmentTooLarge,
    SizeOverflow,
    OutOfMemory,
    Frozen,
}

/// A bounded bump arena with an explicit activation freeze boundary.
pub struct Arena<'a> {
    storage: &'a mut [MaybeUninit<usize>],
    cursor: usize,
    frozen: bool,
}

impl<'a> Arena<'a> {
    pub fn new(storage: &'a mut [MaybeUninit<usize>]) -> Self {
        Self {
            storage,
            cursor: 0,
            frozen: false,
        }
    }

    pub const fn capacity_bytes(&self) -> usize {
        self.storage.len() * size_of::<usize>()
    }

    pub const fn used_bytes(&self) -> usize {
        self.cursor
    }

    pub const fn remaining_bytes(&self) -> usize {
        self.capacity_bytes().saturating_sub(self.cursor)
    }

    pub const fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Freeze the arena after activation. Further allocations fail closed.
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// Reset storage for a new configuration/activation epoch.
    pub fn reset(&mut self) {
        self.cursor = 0;
        self.frozen = false;
    }

    pub fn alloc<T>(&mut self) -> Result<&mut MaybeUninit<T>, ArenaError> {
        let slice = self.alloc_slice::<T>(1)?;
        Ok(&mut slice[0])
    }

    pub fn alloc_init<T>(&mut self, value: T) -> Result<&mut T, ArenaError> {
        Ok(self.alloc::<T>()?.write(value))
    }

    pub fn alloc_slice<T>(&mut self, count: usize) -> Result<&mut [MaybeUninit<T>], ArenaError> {
        if self.frozen {
            return Err(ArenaError::Frozen);
        }
        if align_of::<T>() > align_of::<usize>() {
            return Err(ArenaError::AlignmentTooLarge);
        }

        let alignment = align_of::<T>().max(1);
        let start = align_up(self.cursor, alignment).ok_or(ArenaError::SizeOverflow)?;
        let bytes = size_of::<T>()
            .checked_mul(count)
            .ok_or(ArenaError::SizeOverflow)?;
        let end = start.checked_add(bytes).ok_or(ArenaError::SizeOverflow)?;
        if end > self.capacity_bytes() {
            return Err(ArenaError::OutOfMemory);
        }

        self.cursor = end;
        let pointer = self.storage.as_mut_ptr().cast::<u8>().wrapping_add(start);
        // SAFETY: `storage` is aligned to `align_of::<usize>()`, `start` is
        // aligned to `align_of::<T>()`, and the checked byte range stays
        // within the caller-owned backing storage.
        Ok(unsafe { core::slice::from_raw_parts_mut(pointer.cast::<MaybeUninit<T>>(), count) })
    }
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    let mask = alignment.checked_sub(1)?;
    value.checked_add(mask).map(|next| next & !mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Pair {
        left: u32,
        right: u16,
    }

    #[test]
    fn arena_allocates_aligned_initialized_values_and_slices() -> Result<(), ArenaError> {
        let mut storage = [MaybeUninit::<usize>::uninit(); 8];
        let mut arena = Arena::new(&mut storage);
        let value = arena.alloc_init(Pair {
            left: 0x1234_5678,
            right: 0x9ABC,
        })?;
        assert_eq!(
            *value,
            Pair {
                left: 0x1234_5678,
                right: 0x9ABC
            }
        );
        assert_eq!((value as *const Pair as usize) % align_of::<Pair>(), 0);

        let words = arena.alloc_slice::<u16>(3)?;
        words[0].write(1);
        words[1].write(2);
        words[2].write(3);
        assert_eq!(arena.used_bytes(), size_of::<Pair>() + 6);
        Ok::<(), ArenaError>(())
    }

    #[test]
    fn arena_rejects_overflow_and_frozen_allocations() {
        let mut storage = [MaybeUninit::<usize>::uninit(); 1];
        let mut arena = Arena::new(&mut storage);
        assert!(matches!(
            arena.alloc_slice::<u64>(2),
            Err(ArenaError::OutOfMemory)
        ));
        arena.freeze();
        assert!(matches!(arena.alloc::<u8>(), Err(ArenaError::Frozen)));
        assert!(arena.is_frozen());
        arena.reset();
        assert!(!arena.is_frozen());
        assert!(arena.alloc::<u8>().is_ok());
    }

    #[test]
    fn arena_reports_unsupported_alignment() {
        #[repr(align(16))]
        struct Wide;

        let mut storage = [MaybeUninit::<usize>::uninit(); 4];
        let mut arena = Arena::new(&mut storage);
        assert!(matches!(
            arena.alloc::<Wide>(),
            Err(ArenaError::AlignmentTooLarge)
        ));
    }
}
