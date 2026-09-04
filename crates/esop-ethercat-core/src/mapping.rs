//! Static SyncManager/FMMU configuration for the activation phase.
//!
//! The table is intentionally separate from the cycle engine. It validates
//! all address ranges and emits the exact ESC register images before a master
//! is activated; the real-time path only uses the resulting frozen layout.

pub const ESC_FMMU_BASE: u16 = 0x0600;
pub const ESC_FMMU_STRIDE: u16 = 16;
pub const ESC_SYNC_MANAGER_BASE: u16 = 0x0800;
pub const ESC_SYNC_MANAGER_STRIDE: u16 = 8;

pub const SYNC_MANAGER_IMAGE_LEN: usize = 8;
pub const FMMU_IMAGE_LEN: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyncManagerConfig {
    pub index: u8,
    pub physical_start: u16,
    pub length: u16,
    pub control: u8,
    pub status: u8,
    pub enable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FmmuConfig {
    pub index: u8,
    pub logical_start: u32,
    pub length: u16,
    pub logical_start_bit: u8,
    pub logical_end_bit: u8,
    pub physical_start: u16,
    pub physical_start_bit: u8,
    pub fmmu_type: u8,
    pub enable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MappingSummary {
    pub sync_manager_count: usize,
    pub fmmu_count: usize,
    pub logical_end: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MappingError {
    CapacityExceeded,
    DuplicateSyncManager,
    DuplicateFmmu,
    EmptySyncManager,
    EmptyFmmu,
    InvalidBitRange,
    PhysicalRangeOverflow,
    LogicalRangeOverflow,
    SyncManagerOverlap,
    LogicalOverlap,
    BufferTooSmall,
    UnknownSyncManager,
    UnknownFmmu,
}

impl SyncManagerConfig {
    pub fn encode(&self, dst: &mut [u8]) -> Result<(), MappingError> {
        if dst.len() < SYNC_MANAGER_IMAGE_LEN {
            return Err(MappingError::BufferTooSmall);
        }
        dst[0..2].copy_from_slice(&self.physical_start.to_le_bytes());
        dst[2..4].copy_from_slice(&self.length.to_le_bytes());
        dst[4] = self.control;
        dst[5] = self.status;
        dst[6] = u8::from(self.enable);
        dst[7] = 0;
        Ok(())
    }

    pub const fn register_address(&self) -> u16 {
        ESC_SYNC_MANAGER_BASE + self.index as u16 * ESC_SYNC_MANAGER_STRIDE
    }
}

impl FmmuConfig {
    pub fn encode(&self, dst: &mut [u8]) -> Result<(), MappingError> {
        if dst.len() < FMMU_IMAGE_LEN {
            return Err(MappingError::BufferTooSmall);
        }
        dst[0..4].copy_from_slice(&self.logical_start.to_le_bytes());
        dst[4..6].copy_from_slice(&self.length.to_le_bytes());
        dst[6] = self.logical_start_bit;
        dst[7] = self.logical_end_bit;
        dst[8..10].copy_from_slice(&self.physical_start.to_le_bytes());
        dst[10] = self.physical_start_bit;
        dst[11] = self.fmmu_type;
        dst[12] = u8::from(self.enable);
        dst[13..16].fill(0);
        Ok(())
    }

    pub const fn register_address(&self) -> u16 {
        ESC_FMMU_BASE + self.index as u16 * ESC_FMMU_STRIDE
    }

    fn logical_bit_range(&self) -> Result<(u64, u64), MappingError> {
        if self.length == 0 {
            return Err(MappingError::EmptyFmmu);
        }
        if self.logical_start_bit > 7 || self.logical_end_bit > 7 || self.physical_start_bit > 7 {
            return Err(MappingError::InvalidBitRange);
        }
        let logical_byte_end = self
            .logical_start
            .checked_add(self.length as u32 - 1)
            .ok_or(MappingError::LogicalRangeOverflow)?;
        let start = (self.logical_start as u64)
            .checked_mul(8)
            .and_then(|value| value.checked_add(self.logical_start_bit as u64))
            .ok_or(MappingError::LogicalRangeOverflow)?;
        let end = (logical_byte_end as u64)
            .checked_mul(8)
            .and_then(|value| value.checked_add(self.logical_end_bit as u64 + 1))
            .ok_or(MappingError::LogicalRangeOverflow)?;
        if end <= start {
            return Err(MappingError::InvalidBitRange);
        }
        let mapped_bits = end - start;
        let physical_bytes = ((self.physical_start_bit as u64)
            .checked_add(mapped_bits)
            .ok_or(MappingError::PhysicalRangeOverflow)?
            .div_ceil(8)) as u32;
        let physical_end = (self.physical_start as u32)
            .checked_add(physical_bytes)
            .ok_or(MappingError::PhysicalRangeOverflow)?;
        if physical_end > u16::MAX as u32 + 1 {
            return Err(MappingError::PhysicalRangeOverflow);
        }
        Ok((start, end))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MappingTable<const SMS: usize, const FMMUS: usize> {
    sync_managers: [SyncManagerConfig; SMS],
    sync_manager_count: usize,
    fmmus: [FmmuConfig; FMMUS],
    fmmu_count: usize,
}

impl<const SMS: usize, const FMMUS: usize> MappingTable<SMS, FMMUS> {
    pub const fn new() -> Self {
        Self {
            sync_managers: [SyncManagerConfig {
                index: 0,
                physical_start: 0,
                length: 0,
                control: 0,
                status: 0,
                enable: false,
            }; SMS],
            sync_manager_count: 0,
            fmmus: [FmmuConfig {
                index: 0,
                logical_start: 0,
                length: 0,
                logical_start_bit: 0,
                logical_end_bit: 0,
                physical_start: 0,
                physical_start_bit: 0,
                fmmu_type: 0,
                enable: false,
            }; FMMUS],
            fmmu_count: 0,
        }
    }

    pub const fn sync_manager_count(&self) -> usize {
        self.sync_manager_count
    }

    pub const fn fmmu_count(&self) -> usize {
        self.fmmu_count
    }

    pub fn sync_managers(&self) -> &[SyncManagerConfig] {
        &self.sync_managers[..self.sync_manager_count]
    }

    pub fn fmmus(&self) -> &[FmmuConfig] {
        &self.fmmus[..self.fmmu_count]
    }

    pub fn add_sync_manager(&mut self, config: SyncManagerConfig) -> Result<(), MappingError> {
        if SMS == 0 || self.sync_manager_count >= SMS {
            return Err(MappingError::CapacityExceeded);
        }
        if config.length == 0 {
            return Err(MappingError::EmptySyncManager);
        }
        if self
            .sync_managers()
            .iter()
            .any(|existing| existing.index == config.index)
        {
            return Err(MappingError::DuplicateSyncManager);
        }
        let start = config.physical_start as u32;
        let end = start
            .checked_add(config.length as u32)
            .ok_or(MappingError::PhysicalRangeOverflow)?;
        if end > u16::MAX as u32 + 1 {
            return Err(MappingError::PhysicalRangeOverflow);
        }
        if self.sync_managers().iter().any(|existing| {
            let existing_start = existing.physical_start as u32;
            let existing_end = existing_start + existing.length as u32;
            start < existing_end && existing_start < end
        }) {
            return Err(MappingError::SyncManagerOverlap);
        }
        self.sync_managers[self.sync_manager_count] = config;
        self.sync_manager_count += 1;
        Ok(())
    }

    pub fn add_fmmu(&mut self, config: FmmuConfig) -> Result<(), MappingError> {
        if FMMUS == 0 || self.fmmu_count >= FMMUS {
            return Err(MappingError::CapacityExceeded);
        }
        if self
            .fmmus()
            .iter()
            .any(|existing| existing.index == config.index)
        {
            return Err(MappingError::DuplicateFmmu);
        }
        let (start, end) = config.logical_bit_range()?;
        if self.fmmus().iter().any(|existing| {
            let Ok((existing_start, existing_end)) = existing.logical_bit_range() else {
                return true;
            };
            start < existing_end && existing_start < end
        }) {
            return Err(MappingError::LogicalOverlap);
        }
        self.fmmus[self.fmmu_count] = config;
        self.fmmu_count += 1;
        Ok(())
    }

    pub fn sync_manager(&self, index: u8) -> Result<SyncManagerConfig, MappingError> {
        self.sync_managers()
            .iter()
            .find(|config| config.index == index)
            .copied()
            .ok_or(MappingError::UnknownSyncManager)
    }

    pub fn fmmu(&self, index: u8) -> Result<FmmuConfig, MappingError> {
        self.fmmus()
            .iter()
            .find(|config| config.index == index)
            .copied()
            .ok_or(MappingError::UnknownFmmu)
    }

    pub fn summary(&self) -> MappingSummary {
        let logical_end = self
            .fmmus()
            .iter()
            .map(|fmmu| fmmu.logical_start.saturating_add(fmmu.length as u32))
            .max()
            .unwrap_or(0);
        MappingSummary {
            sync_manager_count: self.sync_manager_count,
            fmmu_count: self.fmmu_count,
            logical_end,
        }
    }
}

impl<const SMS: usize, const FMMUS: usize> Default for MappingTable<SMS, FMMUS> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sm(index: u8, start: u16, length: u16) -> SyncManagerConfig {
        SyncManagerConfig {
            index,
            physical_start: start,
            length,
            control: 0x26,
            status: 0,
            enable: true,
        }
    }

    fn fmmu(index: u8, logical_start: u32, physical_start: u16) -> FmmuConfig {
        FmmuConfig {
            index,
            logical_start,
            length: 4,
            logical_start_bit: 0,
            logical_end_bit: 7,
            physical_start,
            physical_start_bit: 0,
            fmmu_type: 2,
            enable: true,
        }
    }

    #[test]
    fn mapping_table_rejects_overlaps_and_exposes_register_addresses() {
        let mut table = MappingTable::<2, 2>::new();
        table.add_sync_manager(sm(2, 0x1000, 8)).unwrap();
        assert_eq!(
            table.add_sync_manager(sm(3, 0x1004, 4)),
            Err(MappingError::SyncManagerOverlap)
        );
        table.add_fmmu(fmmu(0, 0x2000, 0x1000)).unwrap();
        assert_eq!(
            table.add_fmmu(fmmu(1, 0x2002, 0x1010)),
            Err(MappingError::LogicalOverlap)
        );
        assert_eq!(table.sync_manager(2).unwrap().register_address(), 0x0810);
        assert_eq!(table.fmmu(0).unwrap().register_address(), 0x0600);
        assert_eq!(table.summary().logical_end, 0x2004);
    }

    #[test]
    fn mapping_register_images_are_little_endian_and_fixed_size() {
        let sync = sm(0, 0x1234, 0x0020);
        let mut sync_image = [0xFF; SYNC_MANAGER_IMAGE_LEN];
        sync.encode(&mut sync_image).unwrap();
        assert_eq!(sync_image, [0x34, 0x12, 0x20, 0x00, 0x26, 0, 1, 0]);

        let fmmu = fmmu(0, 0x1122_3344, 0x5566);
        let mut fmmu_image = [0xFF; FMMU_IMAGE_LEN];
        fmmu.encode(&mut fmmu_image).unwrap();
        assert_eq!(&fmmu_image[0..4], &0x1122_3344u32.to_le_bytes());
        assert_eq!(&fmmu_image[8..10], &0x5566u16.to_le_bytes());
        assert_eq!(fmmu_image[11], 2);
        assert_eq!(fmmu_image[12], 1);
        assert_eq!(&fmmu_image[13..16], &[0, 0, 0]);
    }
}
