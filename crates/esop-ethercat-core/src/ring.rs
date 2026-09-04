//! Allocation-free single-producer/single-consumer ring.
//!
//! Ownership is split into one producer and one consumer handle. The producer
//! owns the head cursor and the consumer owns the tail cursor; only the peer
//! cursor is read atomically. This keeps the hot path to bounded indexing and
//! acquire/release operations without a mutex or heap allocation.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RingError {
    CapacityMustBePowerOfTwo,
}

pub struct SpscRing<T, const N: usize> {
    slots: UnsafeCell<[MaybeUninit<T>; N]>,
    head: AtomicUsize,
    tail: AtomicUsize,
}

// Safety: the producer and consumer handles enforce the SPSC access pattern;
// T only crosses the thread boundary when it is Send.
unsafe impl<T: Send, const N: usize> Sync for SpscRing<T, N> {}
unsafe impl<T: Send, const N: usize> Send for SpscRing<T, N> {}

impl<T, const N: usize> SpscRing<T, N> {
    pub const fn try_new() -> Result<Self, RingError> {
        if N == 0 || !N.is_power_of_two() {
            return Err(RingError::CapacityMustBePowerOfTwo);
        }
        Ok(Self {
            slots: UnsafeCell::new(unsafe {
                MaybeUninit::<[MaybeUninit<T>; N]>::uninit().assume_init()
            }),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        })
    }

    pub const fn new() -> Self {
        if N == 0 || !N.is_power_of_two() {
            panic!("SpscRing capacity must be a non-zero power of two");
        }
        Self {
            slots: UnsafeCell::new(unsafe {
                MaybeUninit::<[MaybeUninit<T>; N]>::uninit().assume_init()
            }),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    pub const fn split(&self) -> (SpscProducer<'_, T, N>, SpscConsumer<'_, T, N>) {
        (SpscProducer { ring: self }, SpscConsumer { ring: self })
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        head.wrapping_sub(tail).min(N)
    }

    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Acquire) == self.tail.load(Ordering::Acquire)
    }

    pub fn is_full(&self) -> bool {
        self.len() == N
    }

    unsafe fn write_slot(&self, index: usize, value: T) {
        // SAFETY: only the producer writes a slot after observing capacity;
        // the consumer cannot read it until the release head store.
        unsafe { (*self.slots.get())[index].write(value) };
    }

    unsafe fn read_slot(&self, index: usize) -> T {
        // SAFETY: only the consumer reads a slot after observing a published
        // head value; each slot is read exactly once before reuse.
        unsafe { (*self.slots.get())[index].assume_init_read() }
    }
}

impl<T, const N: usize> Default for SpscRing<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> Drop for SpscRing<T, N> {
    fn drop(&mut self) {
        let head = *self.head.get_mut();
        let tail = *self.tail.get_mut();
        let mask = N.wrapping_sub(1);
        let mut cursor = tail;
        while cursor != head {
            // SAFETY: entries between tail and head are initialized and are
            // not concurrently accessed while the ring is being dropped.
            unsafe { (*self.slots.get())[cursor & mask].assume_init_drop() };
            cursor = cursor.wrapping_add(1);
        }
    }
}

pub struct SpscProducer<'a, T, const N: usize> {
    ring: &'a SpscRing<T, N>,
}

impl<T, const N: usize> SpscProducer<'_, T, N> {
    pub fn push(&self, value: T) -> Result<(), T> {
        let head = self.ring.head.load(Ordering::Relaxed);
        let tail = self.ring.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= N {
            return Err(value);
        }
        let index = head & (N - 1);
        // SAFETY: this producer owns the head cursor and this slot is not
        // readable by the consumer until the release store below.
        unsafe { self.ring.write_slot(index, value) };
        self.ring
            .head
            .store(head.wrapping_add(1), Ordering::Release);
        Ok(())
    }
}

pub struct SpscConsumer<'a, T, const N: usize> {
    ring: &'a SpscRing<T, N>,
}

impl<T, const N: usize> SpscConsumer<'_, T, N> {
    pub fn pop(&self) -> Option<T> {
        let tail = self.ring.tail.load(Ordering::Relaxed);
        let head = self.ring.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let index = tail & (N - 1);
        // SAFETY: the acquire head load observes a fully initialized slot.
        let value = unsafe { self.ring.read_slot(index) };
        self.ring
            .tail
            .store(tail.wrapping_add(1), Ordering::Release);
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_handles_empty_full_and_reuse_without_allocations() {
        let ring = SpscRing::<u32, 2>::new();
        let (producer, consumer) = ring.split();
        assert!(consumer.pop().is_none());
        assert_eq!(producer.push(10), Ok(()));
        assert_eq!(producer.push(20), Ok(()));
        assert_eq!(producer.push(30), Err(30));
        assert!(ring.is_full());
        assert_eq!(consumer.pop(), Some(10));
        assert_eq!(producer.push(30), Ok(()));
        assert_eq!(consumer.pop(), Some(20));
        assert_eq!(consumer.pop(), Some(30));
        assert!(consumer.pop().is_none());
        assert!(ring.is_empty());
    }

    #[test]
    fn ring_drops_unconsumed_values() {
        struct DropCounter<'a>(&'a core::sync::atomic::AtomicUsize);
        impl Drop for DropCounter<'_> {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        let drops = AtomicUsize::new(0);
        {
            let ring = SpscRing::<DropCounter<'_>, 2>::new();
            let (producer, _) = ring.split();
            assert!(producer.push(DropCounter(&drops)).is_ok());
        }
        assert_eq!(drops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn invalid_capacity_is_rejected() {
        assert!(matches!(
            SpscRing::<u8, 3>::try_new(),
            Err(RingError::CapacityMustBePowerOfTwo)
        ));
    }

    #[test]
    fn ring_preserves_order_between_threads() {
        use std::sync::Arc;
        use std::thread;

        let ring = Arc::new(SpscRing::<usize, 64>::new());
        let producer_ring = Arc::clone(&ring);
        let producer = thread::spawn(move || {
            let (producer, _) = producer_ring.split();
            for value in 0..10_000 {
                let mut pending = value;
                while let Err(value) = producer.push(pending) {
                    pending = value;
                    thread::yield_now();
                }
            }
        });

        let (_, consumer) = ring.split();
        for expected in 0..10_000 {
            loop {
                if let Some(actual) = consumer.pop() {
                    assert_eq!(actual, expected);
                    break;
                }
                thread::yield_now();
            }
        }
        producer.join().unwrap();
    }
}
