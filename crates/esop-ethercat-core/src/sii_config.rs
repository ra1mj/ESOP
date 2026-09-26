//! Fixed-capacity configuration candidates derived from SII categories.
//!
//! SII describes a slave's physical SyncManagers and PDO entries, but it does
//! not provide the master's logical address allocation. This module projects
//! only the information that is safe to derive. FMMU allocation is available
//! for fixed PDO-category segments, including multiple SyncManagers per
//! process-data direction.

use crate::mapping::{FmmuConfig, MappingError, MappingTable, SyncManagerConfig};
use crate::pdo::{PdoDirection, PdoEntry, PdoError, PdoLayout};
use crate::sii::{
    SII_CATEGORY_RX_PDO, SII_CATEGORY_SYNC_MANAGER, SII_CATEGORY_TX_PDO, SiiBlockError,
    SiiBlockReader, SiiCategory, SiiCategoryError, SiiCategoryReader,
};
use crate::sii_stream::{SiiCategoryStreamError, SiiCategoryStreamReader};
use sha2::{Digest, Sha256};

pub const SII_CONFIGURATION_SIGNATURE_SCHEMA: u8 = 1;

const SII_CONFIGURATION_SIGNATURE_DOMAIN: &[u8] = b"esop.sii-configuration.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiiConfigurationSignatureError {
    SyncManagerCountOutOfBounds,
    EnabledSyncManagerOutOfRange,
    OpOnlySyncManagerOutOfRange,
    UnknownSyncManager(u8),
    PdoAlreadyOpen,
    PdoNotOpen,
    EmptyPdo,
    InvalidBitLength,
    CountOverflow,
    DirectionOrder,
    EntryRangeOutOfBounds,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SiiConfigurationSignature {
    schema: u8,
    sync_manager_count: u8,
    enabled_sync_managers: u16,
    op_only_sync_managers: u16,
    rx_pdo_count: u16,
    tx_pdo_count: u16,
    rx_entry_count: u16,
    tx_entry_count: u16,
    digest: [u8; 32],
}

impl SiiConfigurationSignature {
    pub const fn schema(self) -> u8 {
        self.schema
    }

    pub const fn sync_manager_count(self) -> u8 {
        self.sync_manager_count
    }

    pub const fn enabled_sync_managers(self) -> u16 {
        self.enabled_sync_managers
    }

    pub const fn op_only_sync_managers(self) -> u16 {
        self.op_only_sync_managers
    }

    pub const fn rx_pdo_count(self) -> u16 {
        self.rx_pdo_count
    }

    pub const fn tx_pdo_count(self) -> u16 {
        self.tx_pdo_count
    }

    pub const fn rx_entry_count(self) -> u16 {
        self.rx_entry_count
    }

    pub const fn tx_entry_count(self) -> u16 {
        self.tx_entry_count
    }

    pub const fn digest(self) -> [u8; 32] {
        self.digest
    }
}

pub struct SiiConfigurationSignatureBuilder {
    hasher: Sha256,
    sync_manager_count: u8,
    enabled_sync_managers: u16,
    op_only_sync_managers: u16,
    rx_pdo_count: u16,
    tx_pdo_count: u16,
    rx_entry_count: u16,
    tx_entry_count: u16,
    open_direction: Option<PdoDirection>,
    open_entry_count: u16,
    tx_started: bool,
}

impl SiiConfigurationSignatureBuilder {
    pub fn new(
        sync_manager_count: u8,
        enabled_sync_managers: u16,
        op_only_sync_managers: u16,
    ) -> Result<Self, SiiConfigurationSignatureError> {
        if usize::from(sync_manager_count) > crate::MAX_ESC_SYNC_MANAGERS {
            return Err(SiiConfigurationSignatureError::SyncManagerCountOutOfBounds);
        }
        let valid_mask = if usize::from(sync_manager_count) == crate::MAX_ESC_SYNC_MANAGERS {
            u16::MAX
        } else if sync_manager_count == 0 {
            0
        } else {
            (1u16 << sync_manager_count) - 1
        };
        if enabled_sync_managers & !valid_mask != 0 {
            return Err(SiiConfigurationSignatureError::EnabledSyncManagerOutOfRange);
        }
        if op_only_sync_managers & !valid_mask != 0 {
            return Err(SiiConfigurationSignatureError::OpOnlySyncManagerOutOfRange);
        }

        let mut hasher = Sha256::new();
        hasher.update(SII_CONFIGURATION_SIGNATURE_DOMAIN);
        hasher.update([SII_CONFIGURATION_SIGNATURE_SCHEMA]);
        hasher.update([sync_manager_count]);
        hasher.update(enabled_sync_managers.to_le_bytes());
        hasher.update(op_only_sync_managers.to_le_bytes());
        Ok(Self {
            hasher,
            sync_manager_count,
            enabled_sync_managers,
            op_only_sync_managers,
            rx_pdo_count: 0,
            tx_pdo_count: 0,
            rx_entry_count: 0,
            tx_entry_count: 0,
            open_direction: None,
            open_entry_count: 0,
            tx_started: false,
        })
    }

    pub fn begin_pdo(
        &mut self,
        direction: PdoDirection,
        pdo_index: u16,
        sync_manager: u8,
    ) -> Result<(), SiiConfigurationSignatureError> {
        if self.open_direction.is_some() {
            return Err(SiiConfigurationSignatureError::PdoAlreadyOpen);
        }
        if sync_manager >= self.sync_manager_count {
            return Err(SiiConfigurationSignatureError::UnknownSyncManager(
                sync_manager,
            ));
        }
        if direction == PdoDirection::Rx && self.tx_started {
            return Err(SiiConfigurationSignatureError::DirectionOrder);
        }
        if direction == PdoDirection::Tx {
            self.tx_started = true;
        }
        let count = match direction {
            PdoDirection::Rx => &mut self.rx_pdo_count,
            PdoDirection::Tx => &mut self.tx_pdo_count,
        };
        *count = count
            .checked_add(1)
            .ok_or(SiiConfigurationSignatureError::CountOverflow)?;
        self.hasher.update([0x10, direction_code(direction)]);
        self.hasher.update(pdo_index.to_le_bytes());
        self.hasher.update([sync_manager]);
        self.open_direction = Some(direction);
        self.open_entry_count = 0;
        Ok(())
    }

    pub fn entry(
        &mut self,
        index: u16,
        subindex: u8,
        bit_length: u8,
    ) -> Result<(), SiiConfigurationSignatureError> {
        let direction = self
            .open_direction
            .ok_or(SiiConfigurationSignatureError::PdoNotOpen)?;
        if !(1..=64).contains(&bit_length) {
            return Err(SiiConfigurationSignatureError::InvalidBitLength);
        }
        self.open_entry_count = self
            .open_entry_count
            .checked_add(1)
            .ok_or(SiiConfigurationSignatureError::CountOverflow)?;
        let count = match direction {
            PdoDirection::Rx => &mut self.rx_entry_count,
            PdoDirection::Tx => &mut self.tx_entry_count,
        };
        *count = count
            .checked_add(1)
            .ok_or(SiiConfigurationSignatureError::CountOverflow)?;
        self.hasher.update([0x11]);
        self.hasher.update(index.to_le_bytes());
        self.hasher.update([subindex, bit_length]);
        Ok(())
    }

    pub fn end_pdo(&mut self) -> Result<(), SiiConfigurationSignatureError> {
        if self.open_direction.is_none() {
            return Err(SiiConfigurationSignatureError::PdoNotOpen);
        }
        if self.open_entry_count == 0 {
            return Err(SiiConfigurationSignatureError::EmptyPdo);
        }
        self.hasher.update([0x12]);
        self.hasher.update(self.open_entry_count.to_le_bytes());
        self.open_direction = None;
        self.open_entry_count = 0;
        Ok(())
    }

    pub fn finish(mut self) -> Result<SiiConfigurationSignature, SiiConfigurationSignatureError> {
        if self.open_direction.is_some() {
            return Err(SiiConfigurationSignatureError::PdoAlreadyOpen);
        }
        self.hasher.update([0xff]);
        self.hasher.update(self.rx_pdo_count.to_le_bytes());
        self.hasher.update(self.tx_pdo_count.to_le_bytes());
        self.hasher.update(self.rx_entry_count.to_le_bytes());
        self.hasher.update(self.tx_entry_count.to_le_bytes());
        let digest = self.hasher.finalize().into();
        Ok(SiiConfigurationSignature {
            schema: SII_CONFIGURATION_SIGNATURE_SCHEMA,
            sync_manager_count: self.sync_manager_count,
            enabled_sync_managers: self.enabled_sync_managers,
            op_only_sync_managers: self.op_only_sync_managers,
            rx_pdo_count: self.rx_pdo_count,
            tx_pdo_count: self.tx_pdo_count,
            rx_entry_count: self.rx_entry_count,
            tx_entry_count: self.tx_entry_count,
            digest,
        })
    }
}

const fn direction_code(direction: PdoDirection) -> u8 {
    match direction {
        PdoDirection::Rx => 0,
        PdoDirection::Tx => 1,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiiConfigurationError {
    Block(SiiBlockError),
    Stream(SiiCategoryStreamError),
    Category(SiiCategoryError),
    Mapping(MappingError),
    Pdo(PdoError),
    BitOffsetOverflow,
    SegmentCapacityExceeded,
    MissingSyncManagerForDirection(PdoDirection),
    PdoExceedsSyncManager {
        direction: PdoDirection,
        pdo_bytes: usize,
        sync_manager_bytes: u16,
    },
    PdoByteLengthOverflow,
    LogicalAddressOverflow,
    FmmuAlreadyAllocated,
    FmmuNotAllocated,
    FmmuCountMismatch,
    FmmuMappingMismatch,
    ProcessImageLengthOverflow,
    OpOnlySyncManagerNotOutput(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiiConfigurationProgress {
    SyncManagers(usize),
    RxPdo { index: u16, entries: usize },
    TxPdo { index: u16, entries: usize },
}

/// One process-data PDO category projected into a physical SyncManager
/// segment. The logical bit offset is relative to its direction's process
/// image; the physical bit offset is relative to the selected SyncManager.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SiiProcessDataSegment {
    pub direction: PdoDirection,
    pub pdo_index: u16,
    pub sync_manager: u8,
    pub entry_start: usize,
    pub entry_count: usize,
    pub logical_bit_offset: usize,
    pub physical_bit_offset: usize,
    pub bit_length: usize,
}

impl SiiProcessDataSegment {
    const EMPTY: Self = Self {
        direction: PdoDirection::Rx,
        pdo_index: 0,
        sync_manager: 0,
        entry_start: 0,
        entry_count: 0,
        logical_bit_offset: 0,
        physical_bit_offset: 0,
        bit_length: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SiiConfigurationCandidate<
    const SMS: usize,
    const FMMUS: usize,
    const RX_ENTRIES: usize,
    const TX_ENTRIES: usize,
> {
    mapping: MappingTable<SMS, FMMUS>,
    rx_layout: PdoLayout<RX_ENTRIES>,
    tx_layout: PdoLayout<TX_ENTRIES>,
    rx_segments: [SiiProcessDataSegment; RX_ENTRIES],
    tx_segments: [SiiProcessDataSegment; TX_ENTRIES],
    rx_segment_count: usize,
    tx_segment_count: usize,
    rx_sync_manager: Option<u8>,
    tx_sync_manager: Option<u8>,
    rx_pdo_count: usize,
    tx_pdo_count: usize,
}

/// A validated SII process-data layout ready to be consumed by a Domain.
///
/// SII keeps RxPDO and TxPDO offsets in separate direction-local images. The
/// projection makes that boundary explicit: Rx starts at bit zero and Tx
/// starts after the byte-rounded Rx image. The original mapping table and
/// physical segment information remain available for the activation-time
/// SyncManager/FMMU writer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SiiDomainProjection<
    const SMS: usize,
    const FMMUS: usize,
    const RX_ENTRIES: usize,
    const TX_ENTRIES: usize,
> {
    candidate: SiiConfigurationCandidate<SMS, FMMUS, RX_ENTRIES, TX_ENTRIES>,
    logical_base: u32,
    process_image_len: usize,
    tx_bit_offset: usize,
}

impl<const SMS: usize, const FMMUS: usize, const RX_ENTRIES: usize, const TX_ENTRIES: usize>
    SiiDomainProjection<SMS, FMMUS, RX_ENTRIES, TX_ENTRIES>
{
    pub fn mapping(&self) -> &MappingTable<SMS, FMMUS> {
        self.candidate.mapping()
    }

    pub fn rx_layout(&self) -> &PdoLayout<RX_ENTRIES> {
        self.candidate.rx_layout()
    }

    pub fn tx_layout(&self) -> &PdoLayout<TX_ENTRIES> {
        self.candidate.tx_layout()
    }

    pub fn rx_segments(&self) -> &[SiiProcessDataSegment] {
        self.candidate.rx_segments()
    }

    pub fn tx_segments(&self) -> &[SiiProcessDataSegment] {
        self.candidate.tx_segments()
    }

    pub const fn logical_base(&self) -> u32 {
        self.logical_base
    }

    pub const fn process_image_len(&self) -> usize {
        self.process_image_len
    }

    pub const fn tx_bit_offset(&self) -> usize {
        self.tx_bit_offset
    }

    /// Translate a direction-local PDO or segment offset into the unified
    /// Domain image used by the cycle engine.
    pub fn domain_bit_offset(
        &self,
        direction: PdoDirection,
        local_bit_offset: usize,
    ) -> Result<usize, SiiConfigurationError> {
        let base = match direction {
            PdoDirection::Rx => 0,
            PdoDirection::Tx => self.tx_bit_offset,
        };
        base.checked_add(local_bit_offset)
            .ok_or(SiiConfigurationError::ProcessImageLengthOverflow)
    }
}

impl<const SMS: usize, const FMMUS: usize, const RX_ENTRIES: usize, const TX_ENTRIES: usize>
    SiiConfigurationCandidate<SMS, FMMUS, RX_ENTRIES, TX_ENTRIES>
{
    pub const fn new() -> Self {
        Self {
            mapping: MappingTable::new(),
            rx_layout: PdoLayout::new(),
            tx_layout: PdoLayout::new(),
            rx_segments: [SiiProcessDataSegment::EMPTY; RX_ENTRIES],
            tx_segments: [SiiProcessDataSegment::EMPTY; TX_ENTRIES],
            rx_segment_count: 0,
            tx_segment_count: 0,
            rx_sync_manager: None,
            tx_sync_manager: None,
            rx_pdo_count: 0,
            tx_pdo_count: 0,
        }
    }

    pub fn mapping(&self) -> &MappingTable<SMS, FMMUS> {
        &self.mapping
    }

    pub fn rx_layout(&self) -> &PdoLayout<RX_ENTRIES> {
        &self.rx_layout
    }

    pub fn tx_layout(&self) -> &PdoLayout<TX_ENTRIES> {
        &self.tx_layout
    }

    pub fn rx_segments(&self) -> &[SiiProcessDataSegment] {
        &self.rx_segments[..self.rx_segment_count]
    }

    pub fn tx_segments(&self) -> &[SiiProcessDataSegment] {
        &self.tx_segments[..self.tx_segment_count]
    }

    pub const fn rx_pdo_count(&self) -> usize {
        self.rx_pdo_count
    }

    pub const fn tx_pdo_count(&self) -> usize {
        self.tx_pdo_count
    }

    pub const fn rx_sync_manager(&self) -> Option<u8> {
        self.rx_sync_manager
    }

    pub const fn tx_sync_manager(&self) -> Option<u8> {
        self.tx_sync_manager
    }

    pub fn signature(&self) -> Result<SiiConfigurationSignature, SiiConfigurationSignatureError> {
        let sync_manager_count = u8::try_from(self.mapping.sync_manager_count())
            .map_err(|_| SiiConfigurationSignatureError::SyncManagerCountOutOfBounds)?;
        let mut enabled_sync_managers = 0u16;
        for sync_manager in self.mapping.sync_managers() {
            if usize::from(sync_manager.index) >= crate::MAX_ESC_SYNC_MANAGERS {
                return Err(SiiConfigurationSignatureError::SyncManagerCountOutOfBounds);
            }
            if sync_manager.enable {
                enabled_sync_managers |= 1u16 << sync_manager.index;
            }
        }
        let mut builder = SiiConfigurationSignatureBuilder::new(
            sync_manager_count,
            enabled_sync_managers,
            self.mapping.op_only_outputs().mask(),
        )?;
        append_signature_segments(&mut builder, self.rx_segments(), self.rx_layout.entries())?;
        append_signature_segments(&mut builder, self.tx_segments(), self.tx_layout.entries())?;
        builder.finish()
    }

    /// Validate and freeze the SII layout for use by a unified Domain image.
    ///
    /// `allocate_fmmus` must have succeeded first. Validation is repeated at
    /// this boundary so a caller cannot accidentally pair a candidate with a
    /// different logical base or with a partially built mapping table.
    pub fn domain_projection(
        &self,
        logical_base: u32,
    ) -> Result<SiiDomainProjection<SMS, FMMUS, RX_ENTRIES, TX_ENTRIES>, SiiConfigurationError>
    {
        let rx_bytes = self.rx_layout.total_bytes();
        let tx_bytes = self.tx_layout.total_bytes();
        let process_image_len = rx_bytes
            .checked_add(tx_bytes)
            .ok_or(SiiConfigurationError::ProcessImageLengthOverflow)?;
        let tx_bit_offset = rx_bytes
            .checked_mul(8)
            .ok_or(SiiConfigurationError::ProcessImageLengthOverflow)?;
        let segment_count = self
            .rx_segment_count
            .checked_add(self.tx_segment_count)
            .ok_or(SiiConfigurationError::ProcessImageLengthOverflow)?;
        if segment_count != 0 && self.mapping.fmmu_count() == 0 {
            return Err(SiiConfigurationError::FmmuNotAllocated);
        }
        if self.mapping.fmmu_count() != segment_count {
            return Err(SiiConfigurationError::FmmuCountMismatch);
        }

        for (fmmu_index, segment) in self
            .rx_segments()
            .iter()
            .chain(self.tx_segments().iter())
            .enumerate()
        {
            let fmmu = self
                .mapping
                .fmmus()
                .get(fmmu_index)
                .ok_or(SiiConfigurationError::FmmuCountMismatch)?;
            let domain_offset = match segment.direction {
                PdoDirection::Rx => segment.logical_bit_offset,
                PdoDirection::Tx => tx_bit_offset
                    .checked_add(segment.logical_bit_offset)
                    .ok_or(SiiConfigurationError::ProcessImageLengthOverflow)?,
            };
            validate_segment_mapping(&self.mapping, *segment, fmmu, logical_base, domain_offset)?;
        }
        self.validate_op_only_outputs()?;

        Ok(SiiDomainProjection {
            candidate: *self,
            logical_base,
            process_image_len,
            tx_bit_offset,
        })
    }

    /// Apply one parsed SII category without guessing signedness.
    ///
    /// SII PDO entries expose the object and bit layout, but the profile or
    /// object dictionary must decide whether a value is signed. Call
    /// [`Self::apply_category_with_signed`] when that metadata is available.
    pub fn apply_category(
        &mut self,
        category: SiiCategory<'_>,
    ) -> Result<SiiConfigurationProgress, SiiConfigurationError> {
        self.apply_category_with_signed(category, false)
    }

    /// Apply one parsed SII category atomically with an explicit signedness.
    ///
    /// The candidate uses a copy of the current fixed-capacity table/layout,
    /// then commits it only after every entry in the category validates.
    pub fn apply_category_with_signed(
        &mut self,
        category: SiiCategory<'_>,
        signed: bool,
    ) -> Result<SiiConfigurationProgress, SiiConfigurationError> {
        match category.kind {
            SII_CATEGORY_SYNC_MANAGER => self.apply_sync_managers(category),
            SII_CATEGORY_RX_PDO => self.apply_pdo(category, PdoDirection::Rx, signed),
            SII_CATEGORY_TX_PDO => self.apply_pdo(category, PdoDirection::Tx, signed),
            _ => Err(SiiConfigurationError::Category(
                SiiCategoryError::UnexpectedCategory,
            )),
        }
    }

    /// Parse a complete SII image and apply only the categories that define
    /// the process-data layout. Other valid SII categories remain available
    /// to their own consumers and are intentionally ignored here.
    pub fn apply_bytes(&mut self, bytes: &[u8]) -> Result<usize, SiiConfigurationError> {
        self.apply_bytes_with_signed(bytes, false)
    }

    pub fn apply_bytes_with_signed(
        &mut self,
        bytes: &[u8],
        signed: bool,
    ) -> Result<usize, SiiConfigurationError> {
        let mut next = *self;
        let applied = next.apply_bytes_in_place(bytes, signed)?;
        *self = next;
        Ok(applied)
    }

    fn apply_bytes_in_place(
        &mut self,
        bytes: &[u8],
        signed: bool,
    ) -> Result<usize, SiiConfigurationError> {
        let mut reader = SiiCategoryReader::new(bytes);
        let mut applied = 0;
        while let Some(category) = reader
            .next_category()
            .map_err(SiiConfigurationError::Category)?
        {
            if matches!(
                category.kind,
                SII_CATEGORY_SYNC_MANAGER | SII_CATEGORY_RX_PDO | SII_CATEGORY_TX_PDO
            ) {
                self.apply_category_with_signed(category, signed)?;
                applied += 1;
            }
        }
        self.validate_op_only_outputs()?;
        Ok(applied)
    }

    /// Copy a completed EEPROM transaction into caller-owned scratch storage
    /// and project its SII categories atomically into this candidate.
    pub fn apply_completed_block<const WORDS: usize>(
        &mut self,
        reader: &SiiBlockReader<WORDS>,
        scratch: &mut [u8],
    ) -> Result<usize, SiiConfigurationError> {
        self.apply_completed_block_with_signed(reader, scratch, false)
    }

    pub fn apply_completed_block_with_signed<const WORDS: usize>(
        &mut self,
        reader: &SiiBlockReader<WORDS>,
        scratch: &mut [u8],
        signed: bool,
    ) -> Result<usize, SiiConfigurationError> {
        let length = reader
            .copy_bytes(scratch)
            .map_err(SiiConfigurationError::Block)?;
        self.apply_bytes_with_signed(&scratch[..length], signed)
    }

    pub fn apply_completed_stream<const WORDS: usize>(
        &mut self,
        reader: &SiiCategoryStreamReader<WORDS>,
        scratch: &mut [u8],
    ) -> Result<usize, SiiConfigurationError> {
        self.apply_completed_stream_with_signed(reader, scratch, false)
    }

    pub fn apply_completed_stream_with_signed<const WORDS: usize>(
        &mut self,
        reader: &SiiCategoryStreamReader<WORDS>,
        scratch: &mut [u8],
        signed: bool,
    ) -> Result<usize, SiiConfigurationError> {
        let length = reader
            .copy_bytes(scratch)
            .map_err(SiiConfigurationError::Stream)?;
        self.apply_bytes_with_signed(&scratch[..length], signed)
    }

    /// Allocate one FMMU per non-empty PDO category segment.
    ///
    /// SII categories identify the physical SyncManager and PDO bit layout,
    /// while the logical process-image base belongs to the master. This
    /// method assigns RxPDO segments first and TxPDO segments second,
    /// preserving a deterministic logical layout. Each segment carries its
    /// own physical SyncManager offset, so PDO categories may use different
    /// SyncManagers without aliasing their physical process data.
    pub fn allocate_fmmus(
        &mut self,
        logical_base: u32,
    ) -> Result<crate::mapping::MappingSummary, SiiConfigurationError> {
        if self.mapping.fmmu_count() != 0 {
            return Err(SiiConfigurationError::FmmuAlreadyAllocated);
        }
        self.validate_op_only_outputs()?;

        let mut next = *self;
        let mut fmmu_index = 0u8;
        let rx_bytes = next.rx_layout.total_bytes();
        let rx_segments = next.rx_segments;
        let tx_segments = next.tx_segments;
        fmmu_index = append_segments(
            &mut next.mapping,
            &rx_segments[..next.rx_segment_count],
            PdoDirection::Rx,
            logical_base,
            fmmu_index,
        )?;
        let tx_base = logical_base
            .checked_add(rx_bytes as u32)
            .ok_or(SiiConfigurationError::LogicalAddressOverflow)?;
        let _ = append_segments(
            &mut next.mapping,
            &tx_segments[..next.tx_segment_count],
            PdoDirection::Tx,
            tx_base,
            fmmu_index,
        )?;
        let summary = next.mapping.summary();
        *self = next;
        Ok(summary)
    }

    fn apply_sync_managers(
        &mut self,
        category: SiiCategory<'_>,
    ) -> Result<SiiConfigurationProgress, SiiConfigurationError> {
        let source = category
            .sync_managers()
            .map_err(SiiConfigurationError::Category)?;
        if source.len() > u8::MAX as usize {
            return Err(SiiConfigurationError::Category(
                SiiCategoryError::SyncManagerCountOutOfBounds,
            ));
        }
        let mut next = self.mapping;
        for index in 0..source.len() {
            let item = source.get(index).map_err(SiiConfigurationError::Category)?;
            next.add_sync_manager(SyncManagerConfig {
                index: index as u8,
                physical_start: item.start_address,
                length: item.length,
                control: item.control,
                status: item.status,
                enable: item.is_enabled(),
            })
            .map_err(SiiConfigurationError::Mapping)?;
            if item.is_op_only() {
                next.mark_op_only_output(index as u8, item.enable)
                    .map_err(SiiConfigurationError::Mapping)?;
            }
        }
        self.mapping = next;
        Ok(SiiConfigurationProgress::SyncManagers(source.len()))
    }

    fn apply_pdo(
        &mut self,
        category: SiiCategory<'_>,
        direction: PdoDirection,
        signed: bool,
    ) -> Result<SiiConfigurationProgress, SiiConfigurationError> {
        let source = category.pdo().map_err(SiiConfigurationError::Category)?;
        let pdo_index = source.index();
        let sync_manager = source.sync_manager();
        if direction == PdoDirection::Tx && self.mapping.op_only_outputs().contains(sync_manager) {
            return Err(SiiConfigurationError::OpOnlySyncManagerNotOutput(
                sync_manager,
            ));
        }
        let result = match direction {
            PdoDirection::Rx => {
                let mut next = self.rx_layout;
                let entries = append_pdo_entries(&mut next, &source, direction, signed)?;
                let mut segments = self.rx_segments;
                let physical_bit_offset =
                    physical_bit_offset(&segments, self.rx_segment_count, sync_manager)?;
                let segment = SiiProcessDataSegment {
                    direction,
                    pdo_index,
                    sync_manager,
                    entry_start: self.rx_layout.len(),
                    entry_count: entries,
                    logical_bit_offset: self.rx_layout.total_bits(),
                    physical_bit_offset,
                    bit_length: next.total_bits() - self.rx_layout.total_bits(),
                };
                let segment_count = append_segment(&mut segments, self.rx_segment_count, segment)?;
                self.rx_layout = next;
                self.rx_segments = segments;
                self.rx_segment_count = segment_count;
                self.rx_sync_manager.get_or_insert(sync_manager);
                self.rx_pdo_count = self.rx_pdo_count.saturating_add(1);
                SiiConfigurationProgress::RxPdo {
                    index: pdo_index,
                    entries,
                }
            }
            PdoDirection::Tx => {
                let mut next = self.tx_layout;
                let entries = append_pdo_entries(&mut next, &source, direction, signed)?;
                let mut segments = self.tx_segments;
                let physical_bit_offset =
                    physical_bit_offset(&segments, self.tx_segment_count, sync_manager)?;
                let segment = SiiProcessDataSegment {
                    direction,
                    pdo_index,
                    sync_manager,
                    entry_start: self.tx_layout.len(),
                    entry_count: entries,
                    logical_bit_offset: self.tx_layout.total_bits(),
                    physical_bit_offset,
                    bit_length: next.total_bits() - self.tx_layout.total_bits(),
                };
                let segment_count = append_segment(&mut segments, self.tx_segment_count, segment)?;
                self.tx_layout = next;
                self.tx_segments = segments;
                self.tx_segment_count = segment_count;
                self.tx_sync_manager.get_or_insert(sync_manager);
                self.tx_pdo_count = self.tx_pdo_count.saturating_add(1);
                SiiConfigurationProgress::TxPdo {
                    index: pdo_index,
                    entries,
                }
            }
        };
        Ok(result)
    }

    fn validate_op_only_outputs(&self) -> Result<(), SiiConfigurationError> {
        let profile = self.mapping.op_only_outputs();
        for sync_manager in 0..crate::op_only::MAX_ESC_SYNC_MANAGERS as u8 {
            if !profile.contains(sync_manager) {
                continue;
            }
            let has_rx = self
                .rx_segments()
                .iter()
                .any(|segment| segment.sync_manager == sync_manager);
            let has_tx = self
                .tx_segments()
                .iter()
                .any(|segment| segment.sync_manager == sync_manager);
            if !has_rx || has_tx {
                return Err(SiiConfigurationError::OpOnlySyncManagerNotOutput(
                    sync_manager,
                ));
            }
        }
        Ok(())
    }
}

fn append_signature_segments(
    builder: &mut SiiConfigurationSignatureBuilder,
    segments: &[SiiProcessDataSegment],
    entries: &[PdoEntry],
) -> Result<(), SiiConfigurationSignatureError> {
    for segment in segments {
        let entry_end = segment
            .entry_start
            .checked_add(segment.entry_count)
            .ok_or(SiiConfigurationSignatureError::EntryRangeOutOfBounds)?;
        let segment_entries = entries
            .get(segment.entry_start..entry_end)
            .ok_or(SiiConfigurationSignatureError::EntryRangeOutOfBounds)?;
        builder.begin_pdo(segment.direction, segment.pdo_index, segment.sync_manager)?;
        for entry in segment_entries {
            if entry.direction != segment.direction {
                return Err(SiiConfigurationSignatureError::EntryRangeOutOfBounds);
            }
            builder.entry(entry.index, entry.subindex, entry.bit_length)?;
        }
        builder.end_pdo()?;
    }
    Ok(())
}

fn validate_segment_mapping<const SMS: usize, const FMMUS: usize>(
    mapping: &MappingTable<SMS, FMMUS>,
    segment: SiiProcessDataSegment,
    fmmu: &FmmuConfig,
    logical_base: u32,
    domain_bit_offset: usize,
) -> Result<(), SiiConfigurationError> {
    let sync_manager = mapping
        .sync_manager(segment.sync_manager)
        .map_err(SiiConfigurationError::Mapping)?;
    let logical_start_bit = (domain_bit_offset % 8) as u8;
    let covered_bits = (logical_start_bit as usize)
        .checked_add(segment.bit_length)
        .ok_or(SiiConfigurationError::BitOffsetOverflow)?;
    if covered_bits == 0 {
        return Err(SiiConfigurationError::FmmuMappingMismatch);
    }
    let logical_length = covered_bits.div_ceil(8);
    let logical_start = logical_base
        .checked_add(
            u32::try_from(domain_bit_offset / 8)
                .map_err(|_| SiiConfigurationError::LogicalAddressOverflow)?,
        )
        .ok_or(SiiConfigurationError::LogicalAddressOverflow)?;
    let logical_end = (logical_start as u64)
        .checked_add(logical_length as u64)
        .ok_or(SiiConfigurationError::LogicalAddressOverflow)?;
    if logical_end > u32::MAX as u64 + 1 {
        return Err(SiiConfigurationError::LogicalAddressOverflow);
    }

    let physical_end_bits = segment
        .physical_bit_offset
        .checked_add(segment.bit_length)
        .ok_or(SiiConfigurationError::BitOffsetOverflow)?;
    let physical_bytes = physical_end_bits.div_ceil(8);
    if physical_bytes > sync_manager.length as usize {
        return Err(SiiConfigurationError::PdoExceedsSyncManager {
            direction: segment.direction,
            pdo_bytes: physical_bytes,
            sync_manager_bytes: sync_manager.length,
        });
    }
    let physical_start = sync_manager
        .physical_start
        .checked_add(
            u16::try_from(segment.physical_bit_offset / 8)
                .map_err(|_| SiiConfigurationError::PdoByteLengthOverflow)?,
        )
        .ok_or(SiiConfigurationError::PdoByteLengthOverflow)?;
    let expected = FmmuConfig {
        index: fmmu.index,
        logical_start,
        length: u16::try_from(logical_length)
            .map_err(|_| SiiConfigurationError::PdoByteLengthOverflow)?,
        logical_start_bit,
        logical_end_bit: ((covered_bits - 1) % 8) as u8,
        physical_start,
        physical_start_bit: (segment.physical_bit_offset % 8) as u8,
        fmmu_type: match segment.direction {
            PdoDirection::Rx => 2,
            PdoDirection::Tx => 1,
        },
        enable: true,
    };
    if *fmmu != expected {
        return Err(SiiConfigurationError::FmmuMappingMismatch);
    }
    Ok(())
}

fn append_segments<const SMS: usize, const FMMUS: usize>(
    mapping: &mut MappingTable<SMS, FMMUS>,
    segments: &[SiiProcessDataSegment],
    direction: PdoDirection,
    direction_logical_base: u32,
    fmmu_index: u8,
) -> Result<u8, SiiConfigurationError> {
    let mut next_index = fmmu_index;
    for segment in segments {
        if segment.direction != direction || segment.bit_length == 0 {
            continue;
        }
        let sync_manager = mapping
            .sync_manager(segment.sync_manager)
            .map_err(SiiConfigurationError::Mapping)?;
        let physical_end_bits = segment
            .physical_bit_offset
            .checked_add(segment.bit_length)
            .ok_or(SiiConfigurationError::BitOffsetOverflow)?;
        let pdo_bytes = physical_end_bits.div_ceil(8);
        if pdo_bytes > sync_manager.length as usize {
            return Err(SiiConfigurationError::PdoExceedsSyncManager {
                direction,
                pdo_bytes,
                sync_manager_bytes: sync_manager.length,
            });
        }
        let logical_byte_offset = segment.logical_bit_offset / 8;
        let logical_start = direction_logical_base
            .checked_add(logical_byte_offset as u32)
            .ok_or(SiiConfigurationError::LogicalAddressOverflow)?;
        let physical_byte_offset = segment.physical_bit_offset / 8;
        let physical_start = sync_manager
            .physical_start
            .checked_add(
                u16::try_from(physical_byte_offset)
                    .map_err(|_| SiiConfigurationError::PdoByteLengthOverflow)?,
            )
            .ok_or(SiiConfigurationError::PdoByteLengthOverflow)?;
        let start_bit = (segment.logical_bit_offset % 8) as u8;
        let physical_start_bit = (segment.physical_bit_offset % 8) as u8;
        let covered_bits = start_bit as usize + segment.bit_length;
        let length = covered_bits.div_ceil(8);
        let logical_end_bit = ((covered_bits - 1) % 8) as u8;
        mapping
            .add_fmmu(crate::mapping::FmmuConfig {
                index: next_index,
                logical_start,
                length: u16::try_from(length)
                    .map_err(|_| SiiConfigurationError::PdoByteLengthOverflow)?,
                logical_start_bit: start_bit,
                logical_end_bit,
                physical_start,
                physical_start_bit,
                fmmu_type: match direction {
                    PdoDirection::Rx => 2,
                    PdoDirection::Tx => 1,
                },
                enable: true,
            })
            .map_err(SiiConfigurationError::Mapping)?;
        next_index = next_index.wrapping_add(1);
    }
    Ok(next_index)
}

fn physical_bit_offset(
    segments: &[SiiProcessDataSegment],
    count: usize,
    sync_manager: u8,
) -> Result<usize, SiiConfigurationError> {
    let mut offset = 0usize;
    for segment in segments[..count].iter().copied() {
        if segment.sync_manager == sync_manager {
            offset = offset
                .checked_add(segment.bit_length)
                .ok_or(SiiConfigurationError::BitOffsetOverflow)?;
        }
    }
    Ok(offset)
}

fn append_segment<const SEGMENTS: usize>(
    segments: &mut [SiiProcessDataSegment; SEGMENTS],
    count: usize,
    segment: SiiProcessDataSegment,
) -> Result<usize, SiiConfigurationError> {
    if segment.bit_length == 0 {
        return Ok(count);
    }
    if count >= SEGMENTS {
        return Err(SiiConfigurationError::SegmentCapacityExceeded);
    }
    segments[count] = segment;
    Ok(count + 1)
}

impl<const SMS: usize, const FMMUS: usize, const RX_ENTRIES: usize, const TX_ENTRIES: usize> Default
    for SiiConfigurationCandidate<SMS, FMMUS, RX_ENTRIES, TX_ENTRIES>
{
    fn default() -> Self {
        Self::new()
    }
}

fn append_pdo_entries<const ENTRIES: usize>(
    layout: &mut PdoLayout<ENTRIES>,
    source: &crate::sii::SiiPdoCategory<'_>,
    direction: PdoDirection,
    signed: bool,
) -> Result<usize, SiiConfigurationError> {
    let mut bit_offset = layout.total_bits();
    for index in 0..source.entry_count() {
        let source_entry = source
            .entry(index)
            .map_err(SiiConfigurationError::Category)?;
        let next_bit_offset = bit_offset
            .checked_add(source_entry.bit_length as usize)
            .ok_or(SiiConfigurationError::BitOffsetOverflow)?;
        layout
            .add(PdoEntry {
                index: source_entry.index,
                subindex: source_entry.subindex,
                bit_offset,
                bit_length: source_entry.bit_length,
                signed,
                direction,
            })
            .map_err(SiiConfigurationError::Pdo)?;
        bit_offset = next_bit_offset;
    }
    Ok(source.entry_count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sii::{SII_CATEGORY_END, SII_CATEGORY_RX_PDO, SII_CATEGORY_SYNC_MANAGER};

    fn append_category(bytes: &mut std::vec::Vec<u8>, kind: u16, data: &[u8]) {
        assert_eq!(data.len() % 2, 0);
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&((data.len() / 2) as u16).to_le_bytes());
        bytes.extend_from_slice(data);
    }

    fn configuration_bytes(rx_object: u16) -> std::vec::Vec<u8> {
        let mut bytes = std::vec::Vec::new();
        append_category(
            &mut bytes,
            SII_CATEGORY_SYNC_MANAGER,
            &[
                0x00, 0x10, 0x02, 0x00, 0x26, 0x00, 0x01, 0x00, 0x00, 0x11, 0x02, 0x00, 0x22, 0x00,
                0x01, 0x00,
            ],
        );
        for (kind, pdo_index, sync_manager, object_index) in [
            (SII_CATEGORY_RX_PDO, 0x1600u16, 0u8, rx_object),
            (SII_CATEGORY_TX_PDO, 0x1A00u16, 1u8, 0x6041u16),
        ] {
            let mut pdo = [0u8; 16];
            pdo[0..2].copy_from_slice(&pdo_index.to_le_bytes());
            pdo[2] = 1;
            pdo[3] = sync_manager;
            pdo[8..10].copy_from_slice(&object_index.to_le_bytes());
            pdo[10] = 0;
            pdo[12] = 16;
            append_category(&mut bytes, kind, &pdo);
        }
        bytes.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes
    }

    fn expected_configuration_signature(
        rx_object: u16,
    ) -> Result<SiiConfigurationSignature, SiiConfigurationSignatureError> {
        let mut builder = SiiConfigurationSignatureBuilder::new(2, 0b11, 0)?;
        builder.begin_pdo(PdoDirection::Rx, 0x1600, 0)?;
        builder.entry(rx_object, 0, 16)?;
        builder.end_pdo()?;
        builder.begin_pdo(PdoDirection::Tx, 0x1A00, 1)?;
        builder.entry(0x6041, 0, 16)?;
        builder.end_pdo()?;
        builder.finish()
    }

    fn detailed_configuration_signature(
        sync_manager_count: u8,
        enabled_sync_managers: u16,
        op_only_sync_managers: u16,
        first: (u16, u8, u16, u8, u8),
        second: Option<(u16, u8, u16, u8, u8)>,
        reverse_rx_order: bool,
    ) -> SiiConfigurationSignature {
        fn append(
            builder: &mut SiiConfigurationSignatureBuilder,
            direction: PdoDirection,
            item: (u16, u8, u16, u8, u8),
        ) {
            builder.begin_pdo(direction, item.0, item.1).unwrap();
            builder.entry(item.2, item.3, item.4).unwrap();
            builder.end_pdo().unwrap();
        }

        let mut builder = SiiConfigurationSignatureBuilder::new(
            sync_manager_count,
            enabled_sync_managers,
            op_only_sync_managers,
        )
        .unwrap();
        if reverse_rx_order {
            append(&mut builder, PdoDirection::Rx, second.unwrap());
            append(&mut builder, PdoDirection::Rx, first);
        } else {
            append(&mut builder, PdoDirection::Rx, first);
            if let Some(second) = second {
                append(&mut builder, PdoDirection::Rx, second);
            }
        }
        append(&mut builder, PdoDirection::Tx, (0x1A00, 3, 0x6041, 0, 16));
        builder.finish().unwrap()
    }

    #[test]
    fn configuration_signature_covers_ordered_sm_and_pdo_structure() {
        let bytes = configuration_bytes(0x6040);
        let mut unsigned = SiiConfigurationCandidate::<2, 0, 2, 2>::new();
        unsigned.apply_bytes_with_signed(&bytes, false).unwrap();
        let signature = unsigned.signature().unwrap();
        assert_eq!(signature, expected_configuration_signature(0x6040).unwrap());
        assert_eq!(signature.schema(), SII_CONFIGURATION_SIGNATURE_SCHEMA);
        assert_eq!(signature.sync_manager_count(), 2);
        assert_eq!(signature.enabled_sync_managers(), 0b11);
        assert_eq!(signature.op_only_sync_managers(), 0);
        assert_eq!(signature.rx_pdo_count(), 1);
        assert_eq!(signature.tx_pdo_count(), 1);
        assert_eq!(signature.rx_entry_count(), 1);
        assert_eq!(signature.tx_entry_count(), 1);

        let mut signed = SiiConfigurationCandidate::<2, 0, 2, 2>::new();
        signed.apply_bytes_with_signed(&bytes, true).unwrap();
        assert_eq!(signed.signature().unwrap(), signature);
        assert_ne!(expected_configuration_signature(0x607A).unwrap(), signature);

        let mut reordered = SiiConfigurationSignatureBuilder::new(2, 0b11, 0).unwrap();
        reordered.begin_pdo(PdoDirection::Tx, 0x1A00, 1).unwrap();
        reordered.entry(0x6041, 0, 16).unwrap();
        reordered.end_pdo().unwrap();
        assert_eq!(
            reordered.begin_pdo(PdoDirection::Rx, 0x1600, 0),
            Err(SiiConfigurationSignatureError::DirectionOrder)
        );
    }

    #[test]
    fn configuration_signature_builder_rejects_invalid_profiles() {
        assert!(matches!(
            SiiConfigurationSignatureBuilder::new(1, 0b10, 0),
            Err(SiiConfigurationSignatureError::EnabledSyncManagerOutOfRange)
        ));
        assert!(matches!(
            SiiConfigurationSignatureBuilder::new(1, 0, 0b10),
            Err(SiiConfigurationSignatureError::OpOnlySyncManagerOutOfRange)
        ));
        let mut builder = SiiConfigurationSignatureBuilder::new(1, 1, 0).unwrap();
        assert_eq!(
            builder.begin_pdo(PdoDirection::Rx, 0x1600, 1),
            Err(SiiConfigurationSignatureError::UnknownSyncManager(1))
        );
        builder.begin_pdo(PdoDirection::Rx, 0x1600, 0).unwrap();
        assert_eq!(
            builder.end_pdo(),
            Err(SiiConfigurationSignatureError::EmptyPdo)
        );
    }

    #[test]
    fn every_structural_signature_field_changes_the_comparison_value() {
        let first = (0x1600, 2, 0x6040, 0, 16);
        let second = (0x1601, 2, 0x607A, 0, 32);
        let baseline =
            detailed_configuration_signature(4, 0x000f, 0x0004, first, Some(second), false);
        for changed in [
            detailed_configuration_signature(5, 0x000f, 0x0004, first, Some(second), false),
            detailed_configuration_signature(4, 0x0007, 0x0004, first, Some(second), false),
            detailed_configuration_signature(4, 0x000f, 0, first, Some(second), false),
            detailed_configuration_signature(
                4,
                0x000f,
                0x0004,
                (0x1602, 2, 0x6040, 0, 16),
                Some(second),
                false,
            ),
            detailed_configuration_signature(
                4,
                0x000f,
                0x0004,
                (0x1600, 1, 0x6040, 0, 16),
                Some(second),
                false,
            ),
            detailed_configuration_signature(
                4,
                0x000f,
                0x0004,
                (0x1600, 2, 0x6041, 0, 16),
                Some(second),
                false,
            ),
            detailed_configuration_signature(
                4,
                0x000f,
                0x0004,
                (0x1600, 2, 0x6040, 1, 16),
                Some(second),
                false,
            ),
            detailed_configuration_signature(
                4,
                0x000f,
                0x0004,
                (0x1600, 2, 0x6040, 0, 8),
                Some(second),
                false,
            ),
            detailed_configuration_signature(4, 0x000f, 0x0004, first, None, false),
            detailed_configuration_signature(4, 0x000f, 0x0004, first, Some(second), true),
        ] {
            assert_ne!(changed, baseline);
        }
    }

    #[test]
    fn candidate_projects_sii_categories_into_fixed_mapping_and_pdo_layouts() {
        let mut bytes = std::vec::Vec::new();
        append_category(
            &mut bytes,
            SII_CATEGORY_SYNC_MANAGER,
            &[0x00, 0x10, 0x20, 0x00, 0x26, 0x64, 0x01, 0x00],
        );
        let mut rx_pdo = [0u8; 24];
        rx_pdo[0..2].copy_from_slice(&0x1600u16.to_le_bytes());
        rx_pdo[2] = 2;
        rx_pdo[3] = 0;
        rx_pdo[8..10].copy_from_slice(&0x6040u16.to_le_bytes());
        rx_pdo[10] = 0;
        rx_pdo[11] = 2;
        rx_pdo[12] = 16;
        rx_pdo[16..18].copy_from_slice(&0x607Au16.to_le_bytes());
        rx_pdo[18] = 0;
        rx_pdo[19] = 3;
        rx_pdo[20] = 32;
        append_category(&mut bytes, SII_CATEGORY_RX_PDO, &rx_pdo);
        bytes.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        let mut reader = crate::sii::SiiCategoryReader::new(&bytes);
        let mut candidate = SiiConfigurationCandidate::<2, 1, 2, 1>::new();
        let sync = reader.next_category().unwrap().unwrap();
        assert_eq!(
            candidate.apply_category(sync),
            Ok(SiiConfigurationProgress::SyncManagers(1))
        );
        let pdo = reader.next_category().unwrap().unwrap();
        assert_eq!(
            candidate.apply_category(pdo),
            Ok(SiiConfigurationProgress::RxPdo {
                index: 0x1600,
                entries: 2,
            })
        );
        assert_eq!(candidate.mapping().sync_manager_count(), 1);
        assert_eq!(candidate.rx_pdo_count(), 1);
        assert_eq!(candidate.rx_layout().total_bits(), 48);
        assert_eq!(candidate.rx_layout().entry(1).unwrap().bit_offset, 16);
        let summary = candidate.allocate_fmmus(0x1000).unwrap();
        assert_eq!(summary.fmmu_count, 1);
        let fmmu = candidate.mapping().fmmu(0).unwrap();
        assert_eq!(fmmu.logical_start, 0x1000);
        assert_eq!(fmmu.length, 6);
        assert_eq!(fmmu.physical_start, 0x1000);
        assert_eq!(fmmu.fmmu_type, 2);
    }

    #[test]
    fn sii_op_only_flag_is_separate_from_enable_and_requires_rxpdo() {
        let mut bytes = std::vec::Vec::new();
        append_category(
            &mut bytes,
            SII_CATEGORY_SYNC_MANAGER,
            &[0x00, 0x10, 0x20, 0x00, 0x26, 0x64, 0x09, 0x00],
        );
        let mut rx_pdo = [0u8; 16];
        rx_pdo[0..2].copy_from_slice(&0x1600u16.to_le_bytes());
        rx_pdo[2] = 1;
        rx_pdo[3] = 0;
        rx_pdo[8..10].copy_from_slice(&0x6040u16.to_le_bytes());
        rx_pdo[12] = 16;
        append_category(&mut bytes, SII_CATEGORY_RX_PDO, &rx_pdo);
        bytes.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        let mut candidate = SiiConfigurationCandidate::<1, 1, 1, 1>::new();
        candidate.apply_bytes(&bytes).unwrap();
        assert!(candidate.mapping().sync_manager(0).unwrap().enable);
        assert!(candidate.mapping().op_only_outputs().contains(0));
        assert_eq!(
            candidate.mapping().op_only_outputs().activation_template(0),
            Some(0x09)
        );

        let mut invalid = std::vec::Vec::new();
        append_category(
            &mut invalid,
            SII_CATEGORY_SYNC_MANAGER,
            &[0x00, 0x10, 0x20, 0x00, 0x26, 0x64, 0x09, 0x00],
        );
        append_category(&mut invalid, SII_CATEGORY_TX_PDO, &rx_pdo);
        invalid.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
        invalid.extend_from_slice(&0u16.to_le_bytes());
        let mut candidate = SiiConfigurationCandidate::<1, 1, 1, 1>::new();
        assert_eq!(
            candidate.apply_bytes(&invalid),
            Err(SiiConfigurationError::OpOnlySyncManagerNotOutput(0))
        );
    }

    #[test]
    fn category_application_is_transactional_on_capacity_failure() {
        let mut bytes = std::vec::Vec::new();
        append_category(
            &mut bytes,
            SII_CATEGORY_SYNC_MANAGER,
            &[
                0x00, 0x10, 0x20, 0x00, 0x26, 0x64, 0x01, 0x00, 0x20, 0x10, 0x20, 0x00, 0x22, 0x60,
                0x01, 0x00,
            ],
        );
        let mut reader = crate::sii::SiiCategoryReader::new(&bytes);
        let category = reader.next_category().unwrap().unwrap();
        let mut candidate = SiiConfigurationCandidate::<1, 1, 1, 1>::new();
        assert_eq!(
            candidate.apply_category(category),
            Err(SiiConfigurationError::Mapping(
                MappingError::CapacityExceeded
            ))
        );
        assert_eq!(candidate.mapping().sync_manager_count(), 0);
    }

    #[test]
    fn apply_bytes_projects_only_process_data_categories() {
        let mut bytes = std::vec::Vec::new();
        append_category(&mut bytes, crate::sii::SII_CATEGORY_GENERAL, &[0, 0]);
        append_category(
            &mut bytes,
            SII_CATEGORY_SYNC_MANAGER,
            &[0x00, 0x10, 0x20, 0x00, 0x26, 0x64, 0x01, 0x00],
        );
        bytes.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        let mut candidate = SiiConfigurationCandidate::<2, 1, 1, 1>::new();
        assert_eq!(candidate.apply_bytes(&bytes), Ok(1));
        assert_eq!(candidate.mapping().sync_manager_count(), 1);
    }

    #[test]
    fn apply_bytes_is_transactional_across_categories() {
        let mut bytes = std::vec::Vec::new();
        append_category(
            &mut bytes,
            SII_CATEGORY_SYNC_MANAGER,
            &[0x00, 0x10, 0x20, 0x00, 0x26, 0x64, 0x01, 0x00],
        );
        append_category(
            &mut bytes,
            SII_CATEGORY_SYNC_MANAGER,
            &[0x20, 0x10, 0x20, 0x00, 0x22, 0x60, 0x01, 0x00],
        );
        bytes.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        let mut candidate = SiiConfigurationCandidate::<2, 1, 1, 1>::new();
        assert!(matches!(
            candidate.apply_bytes(&bytes),
            Err(SiiConfigurationError::Mapping(
                MappingError::DuplicateSyncManager
            ))
        ));
        assert_eq!(candidate.mapping().sync_manager_count(), 0);
    }

    #[test]
    fn incomplete_block_cannot_be_projected() {
        let reader = crate::sii::SiiBlockReader::<2>::new();
        let mut candidate = SiiConfigurationCandidate::<1, 1, 1, 1>::new();
        let mut scratch = [0; 4];
        assert_eq!(
            candidate.apply_completed_block(&reader, &mut scratch),
            Err(SiiConfigurationError::Block(SiiBlockError::NotComplete))
        );
        assert_eq!(reader.word_count(), 0);
    }

    #[test]
    fn fmmu_allocation_rejects_process_data_larger_than_sync_manager() {
        let mut bytes = std::vec::Vec::new();
        append_category(
            &mut bytes,
            SII_CATEGORY_SYNC_MANAGER,
            &[0x00, 0x10, 0x04, 0x00, 0x26, 0x64, 0x01, 0x00],
        );
        let mut pdo = [0u8; 16];
        pdo[0..2].copy_from_slice(&0x1600u16.to_le_bytes());
        pdo[2] = 1;
        pdo[3] = 0;
        pdo[8..10].copy_from_slice(&0x6040u16.to_le_bytes());
        pdo[12] = 40;
        append_category(&mut bytes, SII_CATEGORY_RX_PDO, &pdo);
        bytes.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        let mut candidate = SiiConfigurationCandidate::<1, 1, 1, 1>::new();
        candidate.apply_bytes(&bytes).unwrap();
        assert_eq!(
            candidate.allocate_fmmus(0),
            Err(SiiConfigurationError::PdoExceedsSyncManager {
                direction: PdoDirection::Rx,
                pdo_bytes: 5,
                sync_manager_bytes: 4,
            })
        );
        assert_eq!(candidate.mapping().fmmu_count(), 0);
    }

    #[test]
    fn pdo_categories_across_sync_managers_allocate_distinct_fmmus() {
        let mut bytes = std::vec::Vec::new();
        append_category(
            &mut bytes,
            SII_CATEGORY_SYNC_MANAGER,
            &[
                0x00, 0x10, 0x20, 0x00, 0x26, 0x64, 0x01, 0x00, 0x30, 0x10, 0x20, 0x00, 0x26, 0x64,
                0x01, 0x00,
            ],
        );
        for (index, sync_manager, object_index) in
            [(0x1600u16, 0u8, 0x6040u16), (0x1601u16, 1u8, 0x6041u16)]
        {
            let mut pdo = [0u8; 16];
            pdo[0..2].copy_from_slice(&index.to_le_bytes());
            pdo[2] = 1;
            pdo[3] = sync_manager;
            pdo[8..10].copy_from_slice(&object_index.to_le_bytes());
            pdo[12] = 16;
            append_category(&mut bytes, SII_CATEGORY_RX_PDO, &pdo);
        }
        bytes.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        let mut candidate = SiiConfigurationCandidate::<2, 2, 2, 1>::new();
        assert_eq!(candidate.apply_bytes(&bytes), Ok(3));
        assert_eq!(candidate.rx_layout().len(), 2);
        assert_eq!(candidate.rx_segments().len(), 2);
        assert_eq!(candidate.rx_segments()[0].sync_manager, 0);
        assert_eq!(candidate.rx_segments()[1].sync_manager, 1);
        assert_eq!(candidate.rx_segments()[1].logical_bit_offset, 16);
        assert_eq!(candidate.rx_segments()[1].physical_bit_offset, 0);

        let summary = candidate.allocate_fmmus(0x2000).unwrap();
        assert_eq!(summary.fmmu_count, 2);
        let first = candidate.mapping().fmmu(0).unwrap();
        let second = candidate.mapping().fmmu(1).unwrap();
        assert_eq!(first.logical_start, 0x2000);
        assert_eq!(first.physical_start, 0x1000);
        assert_eq!(second.logical_start, 0x2002);
        assert_eq!(second.physical_start, 0x1030);
        assert_eq!(first.fmmu_type, 2);
        assert_eq!(second.fmmu_type, 2);
    }

    #[test]
    fn domain_projection_unifies_rx_and_tx_offsets_and_checks_fmmus() {
        let mut bytes = std::vec::Vec::new();
        append_category(
            &mut bytes,
            SII_CATEGORY_SYNC_MANAGER,
            &[
                0x00, 0x10, 0x02, 0x00, 0x26, 0x64, 0x01, 0x00, 0x00, 0x11, 0x02, 0x00, 0x26, 0x64,
                0x01, 0x00,
            ],
        );
        for (kind, index, sync_manager, object_index) in [
            (SII_CATEGORY_RX_PDO, 0x1600u16, 0u8, 0x6040u16),
            (SII_CATEGORY_TX_PDO, 0x1A00u16, 1u8, 0x6041u16),
        ] {
            let mut pdo = [0u8; 16];
            pdo[0..2].copy_from_slice(&index.to_le_bytes());
            pdo[2] = 1;
            pdo[3] = sync_manager;
            pdo[8..10].copy_from_slice(&object_index.to_le_bytes());
            pdo[12] = 16;
            append_category(&mut bytes, kind, &pdo);
        }
        bytes.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        let mut candidate = SiiConfigurationCandidate::<2, 2, 1, 1>::new();
        candidate.apply_bytes(&bytes).unwrap();
        assert_eq!(
            candidate.domain_projection(0x2000),
            Err(SiiConfigurationError::FmmuNotAllocated)
        );
        candidate.allocate_fmmus(0x2000).unwrap();

        let projection = candidate.domain_projection(0x2000).unwrap();
        assert_eq!(projection.process_image_len(), 4);
        assert_eq!(projection.tx_bit_offset(), 16);
        assert_eq!(projection.domain_bit_offset(PdoDirection::Rx, 0), Ok(0));
        assert_eq!(projection.domain_bit_offset(PdoDirection::Tx, 0), Ok(16));
        assert_eq!(projection.mapping().fmmu_count(), 2);
        assert_eq!(projection.mapping().fmmu(0).unwrap().logical_start, 0x2000);
        assert_eq!(projection.mapping().fmmu(1).unwrap().logical_start, 0x2002);
        assert_eq!(projection.rx_segments().len(), 1);
        assert_eq!(projection.tx_segments().len(), 1);
        assert_eq!(
            candidate.domain_projection(0x2001),
            Err(SiiConfigurationError::FmmuMappingMismatch)
        );
    }

    #[test]
    fn bit_packed_segments_share_a_logical_byte_without_overlap() {
        let mut bytes = std::vec::Vec::new();
        append_category(
            &mut bytes,
            SII_CATEGORY_SYNC_MANAGER,
            &[0x00, 0x10, 0x01, 0x00, 0x26, 0x64, 0x01, 0x00],
        );
        for (index, bits, object_index) in
            [(0x1600u16, 3u8, 0x6040u16), (0x1601u16, 5u8, 0x6041u16)]
        {
            let mut pdo = [0u8; 16];
            pdo[0..2].copy_from_slice(&index.to_le_bytes());
            pdo[2] = 1;
            pdo[3] = 0;
            pdo[8..10].copy_from_slice(&object_index.to_le_bytes());
            pdo[12] = bits;
            append_category(&mut bytes, SII_CATEGORY_RX_PDO, &pdo);
        }
        bytes.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        let mut candidate = SiiConfigurationCandidate::<1, 2, 2, 1>::new();
        candidate.apply_bytes(&bytes).unwrap();
        candidate.allocate_fmmus(0x2000).unwrap();
        let first = candidate.mapping().fmmu(0).unwrap();
        let second = candidate.mapping().fmmu(1).unwrap();
        assert_eq!(first.logical_start, 0x2000);
        assert_eq!(first.logical_start_bit, 0);
        assert_eq!(first.logical_end_bit, 2);
        assert_eq!(second.logical_start, 0x2000);
        assert_eq!(second.logical_start_bit, 3);
        assert_eq!(second.logical_end_bit, 7);
        assert_eq!(second.physical_start_bit, 3);
    }
}
