use core::convert::TryFrom;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameHandle(u8);

impl FrameHandle {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FrameSlot<const MTU: usize> {
    pub bytes: [u8; MTU],
    pub len: usize,
    pub sequence: u64,
    pub generation: u16,
    pub deadline_ns: u64,
}

impl<const MTU: usize> FrameSlot<MTU> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; MTU],
            len: 0,
            sequence: 0,
            generation: 0,
            deadline_ns: 0,
        }
    }

    pub fn reset(&mut self) {
        self.len = 0;
        self.sequence = 0;
        self.generation = 0;
        self.deadline_ns = 0;
    }
}

impl<const MTU: usize> Default for FrameSlot<MTU> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramePoolError {
    TooManySlots,
    InvalidHandle,
    SlotBusy,
}

pub struct FramePool<const SLOTS: usize, const MTU: usize> {
    slots: [FrameSlot<MTU>; SLOTS],
    used: u64,
}

impl<const SLOTS: usize, const MTU: usize> FramePool<SLOTS, MTU> {
    pub const fn new() -> Self {
        Self {
            slots: [FrameSlot::new(); SLOTS],
            used: 0,
        }
    }

    pub fn acquire(
        &mut self,
        sequence: u64,
        generation: u16,
        deadline_ns: u64,
    ) -> Result<FrameHandle, FramePoolError> {
        if SLOTS == 0 || SLOTS > 64 {
            return Err(FramePoolError::TooManySlots);
        }
        let mask = if SLOTS == 64 {
            u64::MAX
        } else {
            (1u64 << SLOTS) - 1
        };
        let available = (!self.used) & mask;
        if available == 0 {
            return Err(FramePoolError::SlotBusy);
        }
        let index = available.trailing_zeros() as usize;
        let handle = FrameHandle(u8::try_from(index).map_err(|_| FramePoolError::TooManySlots)?);
        self.used |= 1u64 << index;
        let slot = &mut self.slots[index];
        slot.reset();
        slot.sequence = sequence;
        slot.generation = generation;
        slot.deadline_ns = deadline_ns;
        Ok(handle)
    }

    pub fn release(&mut self, handle: FrameHandle) -> Result<(), FramePoolError> {
        let index = handle.index();
        if index >= SLOTS || index >= 64 {
            return Err(FramePoolError::InvalidHandle);
        }
        let bit = 1u64 << index;
        if self.used & bit == 0 {
            return Err(FramePoolError::InvalidHandle);
        }
        self.used &= !bit;
        self.slots[index].reset();
        Ok(())
    }

    pub fn slot(&self, handle: FrameHandle) -> Option<&FrameSlot<MTU>> {
        let index = handle.index();
        if index < SLOTS {
            Some(&self.slots[index])
        } else {
            None
        }
    }

    pub fn slot_mut(&mut self, handle: FrameHandle) -> Option<&mut FrameSlot<MTU>> {
        let index = handle.index();
        if index < SLOTS && self.used & (1u64 << index) != 0 {
            Some(&mut self.slots[index])
        } else {
            None
        }
    }

    pub const fn in_use(&self) -> usize {
        self.used.count_ones() as usize
    }
}

impl<const SLOTS: usize, const MTU: usize> Default for FramePool<SLOTS, MTU> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_is_fixed_and_reuses_slots() {
        let mut pool = FramePool::<2, 64>::new();
        let first = pool.acquire(1, 2, 3).unwrap();
        let second = pool.acquire(4, 5, 6).unwrap();
        assert_eq!(pool.in_use(), 2);
        assert_eq!(pool.acquire(7, 8, 9), Err(FramePoolError::SlotBusy));
        pool.release(first).unwrap();
        let reused = pool.acquire(10, 11, 12).unwrap();
        assert_eq!(reused.index(), first.index());
        assert_eq!(pool.slot(second).unwrap().sequence, 4);
    }
}
