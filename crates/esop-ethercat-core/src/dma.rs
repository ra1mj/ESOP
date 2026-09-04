//! Fixed-capacity DMA descriptor ownership and cache-maintenance contract.
//!
//! The protocol core cannot know how a particular MAC represents ownership or
//! cache lines. This module keeps the lifecycle explicit and bounded while
//! leaving the actual descriptor register layout to a platform port.

use core::convert::TryFrom;
use core::mem::size_of;

pub const DMA_ALIGNMENT: usize = 32;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DmaOwner {
    Free = 0,
    CpuOwned = 1,
    DmaOwned = 2,
    Completed = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DmaRingError {
    InvalidConfiguration,
    NoTxDescriptor,
    NoRxDescriptor,
    InvalidHandle,
    StaleHandle,
    InvalidState { actual: DmaOwner },
    InvalidLength,
}

/// Platform-specific cache operations required at descriptor ownership edges.
///
/// Implementations must perform the required clean/invalidate operation for
/// the address range before returning. A port with coherent memory can use
/// [`NoopDmaCache`].
pub trait DmaCacheOps {
    fn clean_buffer(&mut self, address: usize, length: usize);
    fn clean_descriptor(&mut self, address: usize, length: usize);
    fn invalidate_buffer(&mut self, address: usize, length: usize);
    fn invalidate_descriptor(&mut self, address: usize, length: usize);

    /// Publish cleaned descriptor/buffer contents before setting DMA ownership.
    fn before_dma_submit(&mut self);

    /// Acquire hardware-written descriptor/buffer contents before CPU access.
    fn after_dma_complete(&mut self);
}

#[derive(Default)]
pub struct NoopDmaCache;

impl DmaCacheOps for NoopDmaCache {
    fn clean_buffer(&mut self, _: usize, _: usize) {}

    fn clean_descriptor(&mut self, _: usize, _: usize) {}

    fn invalidate_buffer(&mut self, _: usize, _: usize) {}

    fn invalidate_descriptor(&mut self, _: usize, _: usize) {}

    fn before_dma_submit(&mut self) {}

    fn after_dma_complete(&mut self) {}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DmaTxHandle {
    index: u8,
    generation: u16,
}

impl DmaTxHandle {
    pub const fn index(self) -> usize {
        self.index as usize
    }

    pub const fn generation(self) -> u16 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DmaRxHandle {
    index: u8,
    generation: u16,
}

impl DmaRxHandle {
    pub const fn index(self) -> usize {
        self.index as usize
    }

    pub const fn generation(self) -> u16 {
        self.generation
    }
}

/// Cache-line aligned frame storage suitable for placement in a caller-owned
/// DMA region. The exact linker section and address constraints remain a port
/// responsibility.
#[repr(C, align(32))]
#[derive(Clone, Copy)]
pub struct DmaFrame<const MTU: usize> {
    bytes: [u8; MTU],
}

impl<const MTU: usize> DmaFrame<MTU> {
    const fn new() -> Self {
        Self { bytes: [0; MTU] }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DmaDescriptor<const MTU: usize> {
    frame: DmaFrame<MTU>,
    length: usize,
    owner: DmaOwner,
    generation: u16,
}

impl<const MTU: usize> DmaDescriptor<MTU> {
    const fn new() -> Self {
        Self {
            frame: DmaFrame::new(),
            length: 0,
            owner: DmaOwner::Free,
            generation: 0,
        }
    }
}

/// A pair of bounded TX/RX descriptor rings with explicit ownership.
///
/// The ring scans at most `TX` or `RX` descriptors for each acquire/poll, so
/// all operations have a fixed upper bound. Hardware ports may use the frame
/// slices to program their native descriptors, then call the completion
/// methods after observing the corresponding hardware status bits.
pub struct DmaDescriptorRing<const TX: usize, const RX: usize, const MTU: usize> {
    tx: [DmaDescriptor<MTU>; TX],
    rx: [DmaDescriptor<MTU>; RX],
    tx_cursor: usize,
    rx_cursor: usize,
}

impl<const TX: usize, const RX: usize, const MTU: usize> DmaDescriptorRing<TX, RX, MTU> {
    pub const fn try_new() -> Result<Self, DmaRingError> {
        if TX > u8::MAX as usize || RX > u8::MAX as usize || MTU == 0 {
            return Err(DmaRingError::InvalidConfiguration);
        }
        Ok(Self {
            tx: [DmaDescriptor::new(); TX],
            rx: [DmaDescriptor::new(); RX],
            tx_cursor: 0,
            rx_cursor: 0,
        })
    }

    pub const fn new() -> Self {
        match Self::try_new() {
            Ok(ring) => ring,
            Err(_) => panic!("invalid DMA descriptor ring configuration"),
        }
    }

    pub const fn tx_capacity(&self) -> usize {
        TX
    }

    pub const fn rx_capacity(&self) -> usize {
        RX
    }

    pub fn tx_in_flight(&self) -> usize {
        count_non_free(&self.tx)
    }

    pub fn rx_in_flight(&self) -> usize {
        count_non_free(&self.rx)
    }

    pub fn tx_owner(&self, handle: DmaTxHandle) -> Result<DmaOwner, DmaRingError> {
        let index = self.validate_tx(handle)?;
        Ok(self.tx[index].owner)
    }

    pub fn rx_owner(&self, handle: DmaRxHandle) -> Result<DmaOwner, DmaRingError> {
        let index = self.validate_rx(handle)?;
        Ok(self.rx[index].owner)
    }

    pub fn tx_acquire(&mut self) -> Result<DmaTxHandle, DmaRingError> {
        let index = self.find_free_tx().ok_or(DmaRingError::NoTxDescriptor)?;
        let descriptor = &mut self.tx[index];
        descriptor.generation = next_generation(descriptor.generation);
        descriptor.length = 0;
        descriptor.owner = DmaOwner::CpuOwned;
        self.tx_cursor = (index + 1) % TX.max(1);
        Ok(DmaTxHandle {
            index: u8::try_from(index).map_err(|_| DmaRingError::InvalidConfiguration)?,
            generation: descriptor.generation,
        })
    }

    pub fn tx_buffer_mut(&mut self, handle: DmaTxHandle) -> Result<&mut [u8; MTU], DmaRingError> {
        let index = self.validate_tx(handle)?;
        let descriptor = &mut self.tx[index];
        if descriptor.owner != DmaOwner::CpuOwned {
            return Err(DmaRingError::InvalidState {
                actual: descriptor.owner,
            });
        }
        Ok(&mut descriptor.frame.bytes)
    }

    /// Borrow the frame storage after acquisition and before hardware starts
    /// reading it. A zero-length slice is returned until `tx_submit` records
    /// the descriptor length.
    pub fn tx_buffer(&self, handle: DmaTxHandle) -> Result<&[u8], DmaRingError> {
        let index = self.validate_tx(handle)?;
        let descriptor = &self.tx[index];
        if descriptor.owner == DmaOwner::Free {
            return Err(DmaRingError::InvalidState {
                actual: descriptor.owner,
            });
        }
        Ok(&descriptor.frame.bytes[..descriptor.length])
    }

    pub fn tx_length(&self, handle: DmaTxHandle) -> Result<usize, DmaRingError> {
        let index = self.validate_tx(handle)?;
        let descriptor = &self.tx[index];
        if descriptor.owner == DmaOwner::Free {
            return Err(DmaRingError::InvalidState {
                actual: descriptor.owner,
            });
        }
        Ok(descriptor.length)
    }

    pub fn tx_submit<C: DmaCacheOps>(
        &mut self,
        handle: DmaTxHandle,
        length: usize,
        cache: &mut C,
    ) -> Result<(), DmaRingError> {
        if length == 0 || length > MTU {
            return Err(DmaRingError::InvalidLength);
        }
        let index = self.validate_tx(handle)?;
        let descriptor = &mut self.tx[index];
        if descriptor.owner != DmaOwner::CpuOwned {
            return Err(DmaRingError::InvalidState {
                actual: descriptor.owner,
            });
        }
        descriptor.length = length;
        cache.clean_buffer(descriptor.frame.bytes.as_ptr() as usize, length);
        cache.clean_descriptor(
            descriptor as *const _ as usize,
            size_of::<DmaDescriptor<MTU>>(),
        );
        cache.before_dma_submit();
        descriptor.owner = DmaOwner::DmaOwned;
        Ok(())
    }

    /// Mark a TX descriptor complete after the platform observed the DMA
    /// completion bit. This does not reclaim the descriptor yet.
    pub fn tx_complete<C: DmaCacheOps>(
        &mut self,
        handle: DmaTxHandle,
        cache: &mut C,
    ) -> Result<(), DmaRingError> {
        let index = self.validate_tx(handle)?;
        let descriptor = &mut self.tx[index];
        if descriptor.owner != DmaOwner::DmaOwned {
            return Err(DmaRingError::InvalidState {
                actual: descriptor.owner,
            });
        }
        cache.after_dma_complete();
        descriptor.owner = DmaOwner::Completed;
        Ok(())
    }

    pub fn tx_reclaim(&mut self, handle: DmaTxHandle) -> Result<(), DmaRingError> {
        let index = self.validate_tx(handle)?;
        let descriptor = &mut self.tx[index];
        if descriptor.owner != DmaOwner::Completed {
            return Err(DmaRingError::InvalidState {
                actual: descriptor.owner,
            });
        }
        descriptor.length = 0;
        descriptor.owner = DmaOwner::Free;
        Ok(())
    }

    /// Release a descriptor whose frame build was abandoned while the CPU
    /// still owned it. This cannot release a DMA-owned descriptor because the
    /// platform must first observe hardware completion.
    pub fn tx_cancel(&mut self, handle: DmaTxHandle) -> Result<(), DmaRingError> {
        let index = self.validate_tx(handle)?;
        let descriptor = &mut self.tx[index];
        if descriptor.owner != DmaOwner::CpuOwned {
            return Err(DmaRingError::InvalidState {
                actual: descriptor.owner,
            });
        }
        descriptor.length = 0;
        descriptor.owner = DmaOwner::Free;
        Ok(())
    }

    /// Abort a TX descriptor after the port reports that it did not hand the
    /// descriptor to hardware. The port contract must never return an error
    /// after DMA ownership has been transferred; that case is a hardware
    /// completion/error path and must use `tx_complete` instead.
    pub fn tx_abort(&mut self, handle: DmaTxHandle) -> Result<(), DmaRingError> {
        let index = self.validate_tx(handle)?;
        let descriptor = &mut self.tx[index];
        if !matches!(descriptor.owner, DmaOwner::CpuOwned | DmaOwner::DmaOwned) {
            return Err(DmaRingError::InvalidState {
                actual: descriptor.owner,
            });
        }
        descriptor.length = 0;
        descriptor.owner = DmaOwner::Free;
        Ok(())
    }

    pub fn rx_acquire(&mut self) -> Result<DmaRxHandle, DmaRingError> {
        let index = self.find_free_rx().ok_or(DmaRingError::NoRxDescriptor)?;
        let descriptor = &mut self.rx[index];
        descriptor.generation = next_generation(descriptor.generation);
        descriptor.length = 0;
        descriptor.owner = DmaOwner::CpuOwned;
        self.rx_cursor = (index + 1) % RX.max(1);
        Ok(DmaRxHandle {
            index: u8::try_from(index).map_err(|_| DmaRingError::InvalidConfiguration)?,
            generation: descriptor.generation,
        })
    }

    pub fn rx_buffer_mut(&mut self, handle: DmaRxHandle) -> Result<&mut [u8; MTU], DmaRingError> {
        let index = self.validate_rx(handle)?;
        let descriptor = &mut self.rx[index];
        if descriptor.owner != DmaOwner::CpuOwned {
            return Err(DmaRingError::InvalidState {
                actual: descriptor.owner,
            });
        }
        Ok(&mut descriptor.frame.bytes)
    }

    /// Hand an RX descriptor to DMA after invalidating stale CPU cache lines.
    pub fn rx_submit<C: DmaCacheOps>(
        &mut self,
        handle: DmaRxHandle,
        cache: &mut C,
    ) -> Result<(), DmaRingError> {
        let index = self.validate_rx(handle)?;
        let descriptor = &mut self.rx[index];
        if descriptor.owner != DmaOwner::CpuOwned {
            return Err(DmaRingError::InvalidState {
                actual: descriptor.owner,
            });
        }
        cache.invalidate_buffer(descriptor.frame.bytes.as_ptr() as usize, MTU);
        cache.clean_descriptor(
            descriptor as *const _ as usize,
            size_of::<DmaDescriptor<MTU>>(),
        );
        cache.before_dma_submit();
        descriptor.owner = DmaOwner::DmaOwned;
        Ok(())
    }

    /// Mark an RX descriptor complete after the platform observed a valid
    /// hardware length. The buffer remains inaccessible until `rx_poll`.
    pub fn rx_complete(&mut self, handle: DmaRxHandle, length: usize) -> Result<(), DmaRingError> {
        if length == 0 || length > MTU {
            return Err(DmaRingError::InvalidLength);
        }
        let index = self.validate_rx(handle)?;
        let descriptor = &mut self.rx[index];
        if descriptor.owner != DmaOwner::DmaOwned {
            return Err(DmaRingError::InvalidState {
                actual: descriptor.owner,
            });
        }
        descriptor.length = length;
        descriptor.owner = DmaOwner::Completed;
        Ok(())
    }

    /// Poll at most one completed RX descriptor without blocking. At most `RX`
    /// descriptors are inspected, and completion order follows ring order from
    /// the last poll cursor.
    pub fn rx_poll<C: DmaCacheOps>(
        &mut self,
        cache: &mut C,
    ) -> Result<Option<DmaRxHandle>, DmaRingError> {
        let Some(index) = self.find_completed_rx() else {
            return Ok(None);
        };
        let descriptor = &mut self.rx[index];
        cache.after_dma_complete();
        cache.invalidate_descriptor(
            descriptor as *const _ as usize,
            size_of::<DmaDescriptor<MTU>>(),
        );
        cache.invalidate_buffer(descriptor.frame.bytes.as_ptr() as usize, descriptor.length);
        self.rx_cursor = (index + 1) % RX.max(1);
        Ok(Some(DmaRxHandle {
            index: u8::try_from(index).map_err(|_| DmaRingError::InvalidConfiguration)?,
            generation: descriptor.generation,
        }))
    }

    pub fn rx_buffer(&self, handle: DmaRxHandle) -> Result<&[u8], DmaRingError> {
        let index = self.validate_rx(handle)?;
        let descriptor = &self.rx[index];
        if descriptor.owner != DmaOwner::Completed {
            return Err(DmaRingError::InvalidState {
                actual: descriptor.owner,
            });
        }
        Ok(&descriptor.frame.bytes[..descriptor.length])
    }

    /// Re-arm a completed RX descriptor without exposing it as free to an
    /// unrelated producer. This is the normal hot-path recycle operation.
    pub fn rx_rearm<C: DmaCacheOps>(
        &mut self,
        handle: DmaRxHandle,
        cache: &mut C,
    ) -> Result<(), DmaRingError> {
        let index = self.validate_rx(handle)?;
        let descriptor = &mut self.rx[index];
        if descriptor.owner != DmaOwner::Completed {
            return Err(DmaRingError::InvalidState {
                actual: descriptor.owner,
            });
        }
        descriptor.length = 0;
        cache.invalidate_buffer(descriptor.frame.bytes.as_ptr() as usize, MTU);
        cache.clean_descriptor(
            descriptor as *const _ as usize,
            size_of::<DmaDescriptor<MTU>>(),
        );
        cache.before_dma_submit();
        descriptor.owner = DmaOwner::DmaOwned;
        Ok(())
    }

    pub fn rx_release(&mut self, handle: DmaRxHandle) -> Result<(), DmaRingError> {
        let index = self.validate_rx(handle)?;
        let descriptor = &mut self.rx[index];
        if descriptor.owner != DmaOwner::Completed {
            return Err(DmaRingError::InvalidState {
                actual: descriptor.owner,
            });
        }
        descriptor.length = 0;
        descriptor.owner = DmaOwner::Free;
        Ok(())
    }

    fn find_free_tx(&self) -> Option<usize> {
        find_owner(&self.tx, self.tx_cursor, DmaOwner::Free)
    }

    fn find_free_rx(&self) -> Option<usize> {
        find_owner(&self.rx, self.rx_cursor, DmaOwner::Free)
    }

    fn find_completed_rx(&self) -> Option<usize> {
        find_owner(&self.rx, self.rx_cursor, DmaOwner::Completed)
    }

    fn validate_tx(&self, handle: DmaTxHandle) -> Result<usize, DmaRingError> {
        validate_handle(&self.tx, handle.index as usize, handle.generation)
    }

    fn validate_rx(&self, handle: DmaRxHandle) -> Result<usize, DmaRingError> {
        validate_handle(&self.rx, handle.index as usize, handle.generation)
    }
}

impl<const TX: usize, const RX: usize, const MTU: usize> Default
    for DmaDescriptorRing<TX, RX, MTU>
{
    fn default() -> Self {
        Self::new()
    }
}

fn next_generation(current: u16) -> u16 {
    let next = current.wrapping_add(1);
    if next == 0 { 1 } else { next }
}

fn count_non_free<const MTU: usize>(descriptors: &[DmaDescriptor<MTU>]) -> usize {
    descriptors
        .iter()
        .filter(|descriptor| descriptor.owner != DmaOwner::Free)
        .count()
}

fn find_owner<const MTU: usize>(
    descriptors: &[DmaDescriptor<MTU>],
    cursor: usize,
    wanted: DmaOwner,
) -> Option<usize> {
    if descriptors.is_empty() {
        return None;
    }
    for offset in 0..descriptors.len() {
        let index = (cursor + offset) % descriptors.len();
        if descriptors[index].owner == wanted {
            return Some(index);
        }
    }
    None
}

fn validate_handle<const MTU: usize>(
    descriptors: &[DmaDescriptor<MTU>],
    index: usize,
    generation: u16,
) -> Result<usize, DmaRingError> {
    let descriptor = descriptors.get(index).ok_or(DmaRingError::InvalidHandle)?;
    if descriptor.generation != generation {
        return Err(DmaRingError::StaleHandle);
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum CacheCall {
        CleanBuffer(usize),
        CleanDescriptor(usize),
        InvalidateBuffer(usize),
        InvalidateDescriptor(usize),
        BeforeDmaSubmit,
        AfterDmaComplete,
    }

    struct RecordingCache {
        calls: [Option<CacheCall>; 16],
        count: usize,
    }

    impl RecordingCache {
        fn new() -> Self {
            Self {
                calls: [None; 16],
                count: 0,
            }
        }

        fn push(&mut self, call: CacheCall) {
            self.calls[self.count] = Some(call);
            self.count += 1;
        }
    }

    impl DmaCacheOps for RecordingCache {
        fn clean_buffer(&mut self, _: usize, length: usize) {
            self.push(CacheCall::CleanBuffer(length));
        }

        fn clean_descriptor(&mut self, _: usize, length: usize) {
            self.push(CacheCall::CleanDescriptor(length));
        }

        fn invalidate_buffer(&mut self, _: usize, length: usize) {
            self.push(CacheCall::InvalidateBuffer(length));
        }

        fn invalidate_descriptor(&mut self, _: usize, length: usize) {
            self.push(CacheCall::InvalidateDescriptor(length));
        }

        fn before_dma_submit(&mut self) {
            self.push(CacheCall::BeforeDmaSubmit);
        }

        fn after_dma_complete(&mut self) {
            self.push(CacheCall::AfterDmaComplete);
        }
    }

    #[test]
    fn tx_ownership_requires_cache_clean_and_ordered_reclaim() {
        let mut ring = DmaDescriptorRing::<1, 0, 64>::new();
        let mut cache = RecordingCache::new();
        let handle = ring.tx_acquire().unwrap();
        ring.tx_buffer_mut(handle).unwrap()[..4].copy_from_slice(b"ESOP");
        assert_eq!(ring.tx_owner(handle), Ok(DmaOwner::CpuOwned));
        ring.tx_submit(handle, 4, &mut cache).unwrap();
        assert_eq!(ring.tx_owner(handle), Ok(DmaOwner::DmaOwned));
        assert_eq!(cache.calls[0], Some(CacheCall::CleanBuffer(4)));
        assert_eq!(
            cache.calls[1],
            Some(CacheCall::CleanDescriptor(size_of::<DmaDescriptor<64>>()))
        );
        assert_eq!(cache.calls[2], Some(CacheCall::BeforeDmaSubmit));
        assert_eq!(
            ring.tx_reclaim(handle),
            Err(DmaRingError::InvalidState {
                actual: DmaOwner::DmaOwned
            })
        );
        ring.tx_complete(handle, &mut cache).unwrap();
        ring.tx_reclaim(handle).unwrap();
        assert_eq!(ring.tx_in_flight(), 0);
        assert_eq!(ring.tx_owner(handle), Ok(DmaOwner::Free));
    }

    #[test]
    fn rx_poll_invalidates_and_rearms_without_free_window() {
        let mut ring = DmaDescriptorRing::<0, 1, 64>::new();
        let mut cache = RecordingCache::new();
        let handle = ring.rx_acquire().unwrap();
        ring.rx_buffer_mut(handle).unwrap()[..3].copy_from_slice(b"RX!");
        ring.rx_submit(handle, &mut cache).unwrap();
        assert_eq!(ring.rx_complete(handle, 3), Ok(()));
        let completed = ring.rx_poll(&mut cache).unwrap().unwrap();
        assert_eq!(completed, handle);
        assert_eq!(ring.rx_buffer(handle).unwrap(), b"RX!");
        assert_eq!(ring.rx_owner(handle), Ok(DmaOwner::Completed));
        assert_eq!(cache.calls[2], Some(CacheCall::BeforeDmaSubmit));
        assert_eq!(cache.calls[3], Some(CacheCall::AfterDmaComplete));
        ring.rx_rearm(handle, &mut cache).unwrap();
        assert_eq!(ring.rx_owner(handle), Ok(DmaOwner::DmaOwned));
        assert_eq!(ring.rx_in_flight(), 1);
        assert!(
            cache
                .calls
                .contains(&Some(CacheCall::InvalidateDescriptor(size_of::<
                    DmaDescriptor<64>,
                >(),)))
        );
    }

    #[test]
    fn stale_handles_cannot_control_reused_descriptors() {
        let mut ring = DmaDescriptorRing::<1, 0, 32>::new();
        let first = ring.tx_acquire().unwrap();
        ring.tx_buffer_mut(first).unwrap()[0] = 1;
        let mut cache = NoopDmaCache;
        ring.tx_submit(first, 1, &mut cache).unwrap();
        ring.tx_complete(first, &mut cache).unwrap();
        ring.tx_reclaim(first).unwrap();
        let second = ring.tx_acquire().unwrap();
        assert_ne!(first.generation(), second.generation());
        assert_eq!(
            ring.tx_complete(first, &mut cache),
            Err(DmaRingError::StaleHandle)
        );
    }

    #[test]
    fn invalid_lengths_do_not_change_ownership() {
        let mut ring = DmaDescriptorRing::<1, 1, 16>::new();
        let tx = ring.tx_acquire().unwrap();
        let mut cache = NoopDmaCache;
        assert_eq!(
            ring.tx_submit(tx, 0, &mut cache),
            Err(DmaRingError::InvalidLength)
        );
        assert_eq!(ring.tx_owner(tx), Ok(DmaOwner::CpuOwned));
        let rx = ring.rx_acquire().unwrap();
        ring.rx_submit(rx, &mut cache).unwrap();
        assert_eq!(ring.rx_complete(rx, 17), Err(DmaRingError::InvalidLength));
        assert_eq!(ring.rx_owner(rx), Ok(DmaOwner::DmaOwned));
    }

    #[test]
    fn zero_capacity_or_mtu_is_rejected_by_try_new() {
        assert!(matches!(
            DmaDescriptorRing::<0, 0, 0>::try_new(),
            Err(DmaRingError::InvalidConfiguration)
        ));
        assert!(matches!(
            DmaDescriptorRing::<256, 1, 64>::try_new(),
            Err(DmaRingError::InvalidConfiguration)
        ));
    }

    #[test]
    fn frame_storage_is_cache_line_aligned() {
        let mut ring = DmaDescriptorRing::<1, 1, 64>::new();
        let tx = ring.tx_acquire().unwrap();
        let address = ring.tx_buffer_mut(tx).unwrap().as_ptr() as usize;
        assert_eq!(address % DMA_ALIGNMENT, 0);
    }
}
