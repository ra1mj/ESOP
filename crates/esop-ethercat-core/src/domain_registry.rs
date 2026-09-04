//! Activation-time registration for multiple process-data Domains.
//!
//! The registry is the small, allocation-free layer between generated device
//! configuration and the cyclic [`Domain`]/[`FramePlan`] objects. It mirrors
//! the useful part of an IgH-style domain registration API: callers register
//! PDO entries and datagrams first, inspect the resulting offsets, then
//! freeze the complete set by activating a schedule.

use crate::domain::DomainSegment;
use crate::mapping::MappingSummary;
use crate::pdo::{PdoDirection, PdoEntry};
use crate::plan::{DatagramPlan, FramePlan, FramePlanSet, FramePlanSetError, PlanError};
use crate::schedule::{ScheduleDomain, ScheduleError, ScheduleTable};
use crate::sii_config::{
    SiiConfigurationCandidate, SiiConfigurationError, SiiDomainProjection, SiiProcessDataSegment,
};
use crate::wire::Command;

/// Static placement and rate parameters for one process-data Domain.
///
/// `process_image_offset` and `process_image_len` describe a region in the
/// caller-owned process image. PDO offsets returned by this registry are
/// relative to that region; datagram plans are converted to absolute image
/// offsets when registered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DomainConfig {
    pub id: u8,
    pub logical_address: u32,
    pub process_image_offset: usize,
    pub process_image_len: usize,
    pub period_ticks: u32,
    pub phase_ticks: u32,
}

impl DomainConfig {
    pub const EMPTY: Self = Self {
        id: 0,
        logical_address: 0,
        process_image_offset: 0,
        process_image_len: 0,
        period_ticks: 0,
        phase_ticks: 0,
    };

    pub const fn new(
        id: u8,
        logical_address: u32,
        process_image_offset: usize,
        process_image_len: usize,
        period_ticks: u32,
        phase_ticks: u32,
    ) -> Self {
        Self {
            id,
            logical_address,
            process_image_offset,
            process_image_len,
            period_ticks,
            phase_ticks,
        }
    }
}

/// A PDO entry requested by the static device configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdoRegistrationRequest {
    pub slave_position: u16,
    pub index: u16,
    pub subindex: u8,
    pub direction: PdoDirection,
    pub bit_length: u8,
    pub signed: bool,
}

impl PdoRegistrationRequest {
    pub const fn new(
        slave_position: u16,
        index: u16,
        subindex: u8,
        direction: PdoDirection,
        bit_length: u8,
        signed: bool,
    ) -> Self {
        Self {
            slave_position,
            index,
            subindex,
            direction,
            bit_length,
            signed,
        }
    }
}

/// A registered PDO together with the slave that owns the object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegisteredPdo {
    pub slave_position: u16,
    pub entry: PdoEntry,
}

impl RegisteredPdo {
    pub const EMPTY: Self = Self {
        slave_position: 0,
        entry: PdoEntry::EMPTY,
    };
}

/// Stable handle returned by [`DomainRegistry::register_pdo`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdoEntryHandle {
    domain_id: u8,
    entry_index: usize,
    generation: u16,
}

impl PdoEntryHandle {
    pub const fn domain_id(self) -> u8 {
        self.domain_id
    }

    pub const fn entry_index(self) -> usize {
        self.entry_index
    }

    pub const fn generation(self) -> u16 {
        self.generation
    }
}

/// One datagram belonging to a Domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DomainDatagramSpec {
    pub command: Command,
    pub index: u8,
    pub address: u32,
    /// Offset relative to the Domain's process-image region.
    pub payload_offset: usize,
    pub payload_len: usize,
    pub expected_wkc: u16,
    /// Whether the response payload is committed to a Domain input image.
    /// Output-only datagrams still contribute to the total WKC, but are not
    /// copied into `Domain` staging segments.
    pub input: bool,
}

impl DomainDatagramSpec {
    pub const fn new(
        command: Command,
        index: u8,
        address: u32,
        payload_offset: usize,
        payload_len: usize,
        expected_wkc: u16,
        input: bool,
    ) -> Self {
        Self {
            command,
            index,
            address,
            payload_offset,
            payload_len,
            expected_wkc,
            input,
        }
    }

    pub const fn input(
        command: Command,
        index: u8,
        address: u32,
        payload_offset: usize,
        payload_len: usize,
        expected_wkc: u16,
    ) -> Self {
        Self::new(
            command,
            index,
            address,
            payload_offset,
            payload_len,
            expected_wkc,
            true,
        )
    }

    pub const fn output(
        command: Command,
        index: u8,
        address: u32,
        payload_offset: usize,
        payload_len: usize,
        expected_wkc: u16,
    ) -> Self {
        Self::new(
            command,
            index,
            address,
            payload_offset,
            payload_len,
            expected_wkc,
            false,
        )
    }
}

/// Validated datagram plan with its input-consumption classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DomainDatagram {
    pub plan: DatagramPlan,
    pub input: bool,
}

impl DomainDatagram {
    pub const EMPTY: Self = Self {
        plan: DatagramPlan::EMPTY,
        input: false,
    };
}

/// Read-only summary of one registered Domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DomainInfo {
    pub config: DomainConfig,
    pub pdo_count: usize,
    pub datagram_count: usize,
    pub allocated_bits: usize,
    pub expected_wkc: u16,
    pub input_expected_wkc: u16,
    pub process_image_end: usize,
    pub logical_end: u64,
}

/// Summary returned after a validated SII candidate is registered as a Domain.
///
/// The mapping table is deliberately returned by the SII candidate itself;
/// this summary lets the caller bind that same table to the SM/FMMU writer and
/// the newly registered Domain without copying it into the cyclic registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SiiDomainRegistration {
    pub domain: DomainInfo,
    pub mapping: MappingSummary,
    pub process_image_len: usize,
    pub rx_pdo_count: usize,
    pub tx_pdo_count: usize,
    pub segment_count: usize,
}

/// Runtime datagram metadata for one SII process-data segment.
///
/// Segment order is deterministic: all Rx segments first, followed by all Tx
/// segments. The command and process-image offset are derived from the
/// validated projection, so callers only provide the wire index and expected
/// working counter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SiiSegmentDatagramSpec {
    pub index: u8,
    pub expected_wkc: u16,
}

impl SiiSegmentDatagramSpec {
    pub const fn new(index: u8, expected_wkc: u16) -> Self {
        Self {
            index,
            expected_wkc,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainRegistryPhase {
    Configuring,
    Active,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainRegistryError {
    CapacityExceeded,
    ConfigurationLocked,
    NoDomains,
    DuplicateDomainId,
    DomainIdOutOfRange,
    EmptyProcessImage,
    ProcessImageOverflow,
    ProcessImageOverlap,
    LogicalAddressOverflow,
    LogicalAddressOverlap,
    InvalidPeriod,
    InvalidPhase,
    UnknownDomain,
    InvalidPdoHandle,
    StalePdoHandle,
    PdoCapacityExceeded,
    DuplicatePdo,
    InvalidBitLength,
    PdoImageOverflow,
    PdoBitOverlap,
    DatagramCapacityExceeded,
    DuplicateDatagramIndex,
    DatagramLengthOutOfBounds,
    DatagramImageOverflow,
    ExpectedWkcOverflow,
    EmptyDomain,
    SegmentCapacityExceeded,
    SiiConfiguration(SiiConfigurationError),
    SiiLogicalAddressMismatch,
    SiiProcessImageTooSmall,
    SiiDatagramCountMismatch,
    SiiSegmentNotByteAddressable,
    FramePlans(FramePlanSetError),
    ActivationPlanCapacity,
    ActivationPlanNotEmpty,
    Plan(PlanError),
    Schedule(ScheduleError),
}

#[derive(Clone, Copy)]
struct DomainSlot<const PDOS: usize, const DATAGRAMS: usize> {
    configured: bool,
    config: DomainConfig,
    pdos: [RegisteredPdo; PDOS],
    pdo_count: usize,
    datagrams: [DomainDatagram; DATAGRAMS],
    datagram_count: usize,
    next_bit_offset: usize,
    allocated_bits: usize,
    expected_wkc: u16,
    input_expected_wkc: u16,
}

impl<const PDOS: usize, const DATAGRAMS: usize> DomainSlot<PDOS, DATAGRAMS> {
    const EMPTY: Self = Self {
        configured: false,
        config: DomainConfig::EMPTY,
        pdos: [RegisteredPdo::EMPTY; PDOS],
        pdo_count: 0,
        datagrams: [DomainDatagram::EMPTY; DATAGRAMS],
        datagram_count: 0,
        next_bit_offset: 0,
        allocated_bits: 0,
        expected_wkc: 0,
        input_expected_wkc: 0,
    };

    const fn new(config: DomainConfig) -> Self {
        Self {
            configured: true,
            config,
            ..Self::EMPTY
        }
    }
}

/// Fixed-capacity Domain/PDO/datagram registration table.
///
/// `PDOS` and `DATAGRAMS` are per-Domain capacities. Registration order is
/// deterministic: automatic PDO allocation advances a single bit cursor in
/// each Domain, while [`Self::register_pdo_at`] allows generated layouts to
/// provide an explicit offset. All PDO directions share that Domain-local
/// image address space; callers that use separate input/output buffers can
/// still select explicit non-overlapping or direction-specific offsets.
#[derive(Clone, Copy)]
pub struct DomainRegistry<const DOMAINS: usize, const PDOS: usize, const DATAGRAMS: usize> {
    slots: [DomainSlot<PDOS, DATAGRAMS>; DOMAINS],
    domain_count: usize,
    phase: DomainRegistryPhase,
    generation: u16,
}

impl<const DOMAINS: usize, const PDOS: usize, const DATAGRAMS: usize>
    DomainRegistry<DOMAINS, PDOS, DATAGRAMS>
{
    pub const fn new() -> Self {
        Self {
            slots: [DomainSlot::EMPTY; DOMAINS],
            domain_count: 0,
            phase: DomainRegistryPhase::Configuring,
            generation: 1,
        }
    }

    pub const fn phase(&self) -> DomainRegistryPhase {
        self.phase
    }

    pub const fn is_active(&self) -> bool {
        matches!(self.phase, DomainRegistryPhase::Active)
    }

    pub const fn domain_count(&self) -> usize {
        self.domain_count
    }

    pub const fn generation(&self) -> u16 {
        self.generation
    }

    pub fn register_domain(&mut self, config: DomainConfig) -> Result<(), DomainRegistryError> {
        self.ensure_configuring()?;
        if DOMAINS == 0 || self.domain_count >= DOMAINS {
            return Err(DomainRegistryError::CapacityExceeded);
        }
        if config.id >= 64 {
            return Err(DomainRegistryError::DomainIdOutOfRange);
        }
        if config.process_image_len == 0 {
            return Err(DomainRegistryError::EmptyProcessImage);
        }
        if config.period_ticks == 0 {
            return Err(DomainRegistryError::InvalidPeriod);
        }
        if config.phase_ticks >= config.period_ticks {
            return Err(DomainRegistryError::InvalidPhase);
        }

        let image_end = config
            .process_image_offset
            .checked_add(config.process_image_len)
            .ok_or(DomainRegistryError::ProcessImageOverflow)?;
        let logical_end_value = logical_end(config)?;

        for existing in &self.slots[..self.domain_count] {
            if existing.config.id == config.id {
                return Err(DomainRegistryError::DuplicateDomainId);
            }
            let existing_image_end = existing
                .config
                .process_image_offset
                .checked_add(existing.config.process_image_len)
                .ok_or(DomainRegistryError::ProcessImageOverflow)?;
            if ranges_overlap(
                config.process_image_offset,
                image_end,
                existing.config.process_image_offset,
                existing_image_end,
            ) {
                return Err(DomainRegistryError::ProcessImageOverlap);
            }
            let existing_logical_end = logical_end(existing.config)?;
            if u64_ranges_overlap(
                config.logical_address as u64,
                logical_end_value,
                existing.config.logical_address as u64,
                existing_logical_end,
            ) {
                return Err(DomainRegistryError::LogicalAddressOverlap);
            }
        }

        self.slots[self.domain_count] = DomainSlot::new(config);
        self.domain_count += 1;
        Ok(())
    }

    /// Register a complete SII-derived process-data candidate atomically.
    ///
    /// The candidate must have completed [`SiiConfigurationCandidate::allocate_fmmus`].
    /// RxPDO entries keep their direction-local offsets; TxPDO entries are
    /// placed after the byte-rounded Rx image so both directions can share the
    /// registry's single Domain-local address space. No registry state is
    /// published unless every candidate entry fits and validates.
    pub fn register_sii_candidate<
        const SMS: usize,
        const FMMUS: usize,
        const RX_ENTRIES: usize,
        const TX_ENTRIES: usize,
    >(
        &mut self,
        config: DomainConfig,
        slave_position: u16,
        candidate: &SiiConfigurationCandidate<SMS, FMMUS, RX_ENTRIES, TX_ENTRIES>,
    ) -> Result<SiiDomainRegistration, DomainRegistryError> {
        let projection = candidate
            .domain_projection(config.logical_address)
            .map_err(DomainRegistryError::SiiConfiguration)?;
        self.register_sii_projection(config, slave_position, &projection)
    }

    /// Register a complete SII candidate and its segment datagrams atomically.
    ///
    /// This is the normal activation-time entry point when each SII segment is
    /// transferred by its own logical datagram. The candidate and projection
    /// remain available to the mapping writer; only the validated cycle-facing
    /// plans are copied into the registry.
    pub fn register_sii_candidate_with_datagrams<
        const SMS: usize,
        const FMMUS: usize,
        const RX_ENTRIES: usize,
        const TX_ENTRIES: usize,
    >(
        &mut self,
        config: DomainConfig,
        slave_position: u16,
        candidate: &SiiConfigurationCandidate<SMS, FMMUS, RX_ENTRIES, TX_ENTRIES>,
        specs: &[SiiSegmentDatagramSpec],
    ) -> Result<SiiDomainRegistration, DomainRegistryError> {
        self.ensure_configuring()?;
        let projection = candidate
            .domain_projection(config.logical_address)
            .map_err(DomainRegistryError::SiiConfiguration)?;
        let mut next = *self;
        let mut registration = next.register_sii_projection(config, slave_position, &projection)?;
        next.register_sii_segment_datagrams(config.id, &projection, specs)?;
        registration.domain = next.domain(config.id)?;
        *self = next;
        Ok(registration)
    }

    /// Register an already validated SII projection atomically.
    ///
    /// Keeping this method separate allows the same validated mapping table to
    /// be passed to [`crate::MappingConfigController`] while the registry owns
    /// only the immutable cycle-facing PDO offsets.
    pub fn register_sii_projection<
        const SMS: usize,
        const FMMUS: usize,
        const RX_ENTRIES: usize,
        const TX_ENTRIES: usize,
    >(
        &mut self,
        config: DomainConfig,
        slave_position: u16,
        projection: &SiiDomainProjection<SMS, FMMUS, RX_ENTRIES, TX_ENTRIES>,
    ) -> Result<SiiDomainRegistration, DomainRegistryError> {
        self.ensure_configuring()?;
        if projection.logical_base() != config.logical_address {
            return Err(DomainRegistryError::SiiLogicalAddressMismatch);
        }
        if config.process_image_len < projection.process_image_len() {
            return Err(DomainRegistryError::SiiProcessImageTooSmall);
        }

        let mut next = *self;
        next.register_domain(config)?;

        for entry in projection.rx_layout().entries() {
            let bit_offset = projection
                .domain_bit_offset(entry.direction, entry.bit_offset)
                .map_err(DomainRegistryError::SiiConfiguration)?;
            next.register_pdo_at(
                config.id,
                bit_offset,
                PdoRegistrationRequest::new(
                    slave_position,
                    entry.index,
                    entry.subindex,
                    entry.direction,
                    entry.bit_length,
                    entry.signed,
                ),
            )?;
        }
        for entry in projection.tx_layout().entries() {
            let bit_offset = projection
                .domain_bit_offset(entry.direction, entry.bit_offset)
                .map_err(DomainRegistryError::SiiConfiguration)?;
            next.register_pdo_at(
                config.id,
                bit_offset,
                PdoRegistrationRequest::new(
                    slave_position,
                    entry.index,
                    entry.subindex,
                    entry.direction,
                    entry.bit_length,
                    entry.signed,
                ),
            )?;
        }

        let domain = next.domain(config.id)?;
        let segment_count = projection
            .rx_segments()
            .len()
            .checked_add(projection.tx_segments().len())
            .ok_or(DomainRegistryError::SiiConfiguration(
                SiiConfigurationError::ProcessImageLengthOverflow,
            ))?;
        let registration = SiiDomainRegistration {
            domain,
            mapping: projection.mapping().summary(),
            process_image_len: projection.process_image_len(),
            rx_pdo_count: projection.rx_layout().len(),
            tx_pdo_count: projection.tx_layout().len(),
            segment_count,
        };
        *self = next;
        Ok(registration)
    }

    /// Register one logical read/write datagram per SII process-data segment.
    ///
    /// `specs` must follow the projection's Rx-then-Tx segment order. Separate
    /// datagrams are intentionally limited to byte-aligned segments because
    /// the fixed-capacity Domain staging API copies whole bytes; callers with
    /// bit-packed segments should register one aggregate datagram instead.
    pub fn register_sii_segment_datagrams<
        const SMS: usize,
        const FMMUS: usize,
        const RX_ENTRIES: usize,
        const TX_ENTRIES: usize,
    >(
        &mut self,
        domain_id: u8,
        projection: &SiiDomainProjection<SMS, FMMUS, RX_ENTRIES, TX_ENTRIES>,
        specs: &[SiiSegmentDatagramSpec],
    ) -> Result<usize, DomainRegistryError> {
        self.ensure_configuring()?;
        let domain = self.domain(domain_id)?;
        if domain.config.logical_address != projection.logical_base() {
            return Err(DomainRegistryError::SiiLogicalAddressMismatch);
        }
        if domain.config.process_image_len < projection.process_image_len() {
            return Err(DomainRegistryError::SiiProcessImageTooSmall);
        }
        let segment_count = projection
            .rx_segments()
            .len()
            .checked_add(projection.tx_segments().len())
            .ok_or(DomainRegistryError::SiiConfiguration(
                SiiConfigurationError::ProcessImageLengthOverflow,
            ))?;
        if specs.len() != segment_count {
            return Err(DomainRegistryError::SiiDatagramCountMismatch);
        }

        let mut next = *self;
        let mut segment_index = 0;
        for segment in projection.rx_segments() {
            next.register_sii_segment_datagram(
                domain_id,
                projection,
                *segment,
                segment_index,
                specs[segment_index],
            )?;
            segment_index += 1;
        }
        for segment in projection.tx_segments() {
            next.register_sii_segment_datagram(
                domain_id,
                projection,
                *segment,
                segment_index,
                specs[segment_index],
            )?;
            segment_index += 1;
        }
        *self = next;
        Ok(segment_index)
    }

    pub fn register_pdo(
        &mut self,
        domain_id: u8,
        request: PdoRegistrationRequest,
    ) -> Result<PdoEntryHandle, DomainRegistryError> {
        self.ensure_configuring()?;
        let bit_offset = self.slot(domain_id)?.next_bit_offset;
        self.register_pdo_at(domain_id, bit_offset, request)
    }

    pub fn register_pdo_at(
        &mut self,
        domain_id: u8,
        bit_offset: usize,
        request: PdoRegistrationRequest,
    ) -> Result<PdoEntryHandle, DomainRegistryError> {
        self.ensure_configuring()?;
        let slot_index = self.slot_index(domain_id)?;
        let slot = &mut self.slots[slot_index];
        if PDOS == 0 || slot.pdo_count >= PDOS {
            return Err(DomainRegistryError::PdoCapacityExceeded);
        }
        if !(1..=64).contains(&request.bit_length) {
            return Err(DomainRegistryError::InvalidBitLength);
        }
        if slot.pdos[..slot.pdo_count].iter().any(|registered| {
            registered.slave_position == request.slave_position
                && registered.entry.index == request.index
                && registered.entry.subindex == request.subindex
                && registered.entry.direction == request.direction
        }) {
            return Err(DomainRegistryError::DuplicatePdo);
        }

        let end = bit_offset
            .checked_add(request.bit_length as usize)
            .ok_or(DomainRegistryError::PdoImageOverflow)?;
        let image_bits = slot
            .config
            .process_image_len
            .checked_mul(8)
            .ok_or(DomainRegistryError::PdoImageOverflow)?;
        if end > image_bits {
            return Err(DomainRegistryError::PdoImageOverflow);
        }
        if slot.pdos[..slot.pdo_count].iter().any(|registered| {
            let existing_start = registered.entry.bit_offset;
            let existing_end = existing_start + registered.entry.bit_length as usize;
            ranges_overlap(bit_offset, end, existing_start, existing_end)
        }) {
            return Err(DomainRegistryError::PdoBitOverlap);
        }

        let entry = PdoEntry {
            index: request.index,
            subindex: request.subindex,
            bit_offset,
            bit_length: request.bit_length,
            signed: request.signed,
            direction: request.direction,
        };
        let entry_index = slot.pdo_count;
        slot.pdos[entry_index] = RegisteredPdo {
            slave_position: request.slave_position,
            entry,
        };
        slot.pdo_count += 1;
        slot.next_bit_offset = slot.next_bit_offset.max(end);
        slot.allocated_bits = slot.allocated_bits.max(end);
        Ok(PdoEntryHandle {
            domain_id,
            entry_index,
            generation: self.generation,
        })
    }

    pub fn pdo(&self, handle: PdoEntryHandle) -> Result<RegisteredPdo, DomainRegistryError> {
        if handle.generation != self.generation {
            return Err(DomainRegistryError::StalePdoHandle);
        }
        let slot = self.slot(handle.domain_id)?;
        slot.pdos
            .get(handle.entry_index)
            .copied()
            .filter(|_| handle.entry_index < slot.pdo_count)
            .ok_or(DomainRegistryError::InvalidPdoHandle)
    }

    pub fn pdos(&self, domain_id: u8) -> Result<&[RegisteredPdo], DomainRegistryError> {
        let slot = self.slot(domain_id)?;
        Ok(&slot.pdos[..slot.pdo_count])
    }

    pub fn register_datagram(
        &mut self,
        domain_id: u8,
        spec: DomainDatagramSpec,
    ) -> Result<DomainDatagram, DomainRegistryError> {
        self.ensure_configuring()?;
        let slot_index = self.slot_index(domain_id)?;
        if DATAGRAMS == 0 || self.slots[slot_index].datagram_count >= DATAGRAMS {
            return Err(DomainRegistryError::DatagramCapacityExceeded);
        }
        if self.slots[..self.domain_count].iter().any(|slot| {
            slot.datagrams[..slot.datagram_count]
                .iter()
                .any(|datagram| datagram.plan.index == spec.index)
        }) {
            return Err(DomainRegistryError::DuplicateDatagramIndex);
        }
        if spec.payload_len > 0x07FF {
            return Err(DomainRegistryError::DatagramLengthOutOfBounds);
        }

        let slot = &mut self.slots[slot_index];
        let relative_end = spec
            .payload_offset
            .checked_add(spec.payload_len)
            .ok_or(DomainRegistryError::DatagramImageOverflow)?;
        if relative_end > slot.config.process_image_len {
            return Err(DomainRegistryError::DatagramImageOverflow);
        }
        let absolute_offset = slot
            .config
            .process_image_offset
            .checked_add(spec.payload_offset)
            .ok_or(DomainRegistryError::DatagramImageOverflow)?;
        let expected_wkc = slot
            .expected_wkc
            .checked_add(spec.expected_wkc)
            .ok_or(DomainRegistryError::ExpectedWkcOverflow)?;
        let input_expected_wkc = if spec.input {
            slot.input_expected_wkc
                .checked_add(spec.expected_wkc)
                .ok_or(DomainRegistryError::ExpectedWkcOverflow)?
        } else {
            slot.input_expected_wkc
        };

        let datagram = DomainDatagram {
            plan: DatagramPlan {
                command: spec.command,
                index: spec.index,
                address: spec.address,
                payload_offset: absolute_offset,
                payload_len: spec.payload_len,
                expected_wkc: spec.expected_wkc,
            },
            input: spec.input,
        };
        slot.datagrams[slot.datagram_count] = datagram;
        slot.datagram_count += 1;
        slot.expected_wkc = expected_wkc;
        slot.input_expected_wkc = input_expected_wkc;
        Ok(datagram)
    }

    pub fn datagrams(&self, domain_id: u8) -> Result<&[DomainDatagram], DomainRegistryError> {
        let slot = self.slot(domain_id)?;
        Ok(&slot.datagrams[..slot.datagram_count])
    }

    pub fn domain(&self, domain_id: u8) -> Result<DomainInfo, DomainRegistryError> {
        let slot = self.slot(domain_id)?;
        let process_image_end = slot
            .config
            .process_image_offset
            .checked_add(slot.config.process_image_len)
            .ok_or(DomainRegistryError::ProcessImageOverflow)?;
        Ok(DomainInfo {
            config: slot.config,
            pdo_count: slot.pdo_count,
            datagram_count: slot.datagram_count,
            allocated_bits: slot.allocated_bits,
            expected_wkc: slot.expected_wkc,
            input_expected_wkc: slot.input_expected_wkc,
            process_image_end,
            logical_end: logical_end(slot.config)?,
        })
    }

    pub fn domain_configs(&self) -> impl Iterator<Item = DomainConfig> + '_ {
        self.slots[..self.domain_count]
            .iter()
            .map(|slot| slot.config)
    }

    /// Copy the input-bearing datagrams into the segment descriptors expected
    /// by [`crate::domain::Domain`]. The operation validates the destination capacity before
    /// writing, so a too-small caller buffer is left untouched.
    pub fn copy_domain_segments(
        &self,
        domain_id: u8,
        destination: &mut [DomainSegment],
    ) -> Result<usize, DomainRegistryError> {
        let slot = self.slot(domain_id)?;
        let required = slot.datagrams[..slot.datagram_count]
            .iter()
            .filter(|datagram| datagram.input && datagram.plan.payload_len != 0)
            .count();
        if destination.len() < required {
            return Err(DomainRegistryError::SegmentCapacityExceeded);
        }

        let mut count = 0;
        for datagram in &slot.datagrams[..slot.datagram_count] {
            if !datagram.input || datagram.plan.payload_len == 0 {
                continue;
            }
            let input_offset = datagram
                .plan
                .payload_offset
                .checked_sub(slot.config.process_image_offset)
                .ok_or(DomainRegistryError::DatagramImageOverflow)?;
            destination[count] = DomainSegment {
                datagram_index: datagram.plan.index,
                input_offset,
                len: datagram.plan.payload_len,
                expected_wkc: datagram.plan.expected_wkc,
            };
            count += 1;
        }
        Ok(count)
    }

    /// Append all datagrams for one Domain into a frame plan atomically.
    ///
    /// A Domain that exceeds the frame or plan capacity leaves the caller's
    /// existing plan unchanged; split oversized Domains explicitly in the
    /// activation configuration.
    pub fn append_frame_plan<const PLAN: usize>(
        &self,
        domain_id: u8,
        plan: &mut FramePlan<PLAN>,
    ) -> Result<(), DomainRegistryError> {
        let slot = self.slot(domain_id)?;
        let mut next = FramePlan::new();
        for datagram in plan.datagrams() {
            next.push(*datagram).map_err(DomainRegistryError::Plan)?;
        }
        for datagram in &slot.datagrams[..slot.datagram_count] {
            next.push(datagram.plan)
                .map_err(DomainRegistryError::Plan)?;
        }
        *plan = next;
        Ok(())
    }

    /// Append all datagrams for one Domain into a split frame-plan set.
    ///
    /// The caller's set is changed only after every datagram fits. Frame order
    /// follows the Domain registration order and remains stable across
    /// activation cycles.
    pub fn append_frame_plan_set<const FRAMES: usize, const PLAN: usize>(
        &self,
        domain_id: u8,
        plans: &mut FramePlanSet<FRAMES, PLAN>,
    ) -> Result<usize, DomainRegistryError> {
        let slot = self.slot(domain_id)?;
        let mut next = *plans;
        for datagram in &slot.datagrams[..slot.datagram_count] {
            next.append_datagram(datagram.plan)
                .map_err(DomainRegistryError::FramePlans)?;
        }
        let count = slot.datagram_count;
        *plans = next;
        Ok(count)
    }

    /// Build the precomputed multi-rate schedule without changing registry
    /// state. This is useful for checking resource limits before activation.
    pub fn build_schedule<const SLOTS: usize>(
        &self,
        base_tick_ns: u64,
    ) -> Result<ScheduleTable<DOMAINS, SLOTS>, DomainRegistryError> {
        if self.domain_count == 0 {
            return Err(DomainRegistryError::NoDomains);
        }
        let mut domains = [ScheduleDomain::EMPTY; DOMAINS];
        for (index, config) in self.domain_configs().enumerate() {
            domains[index] = ScheduleDomain {
                id: config.id,
                period_ticks: config.period_ticks,
                phase_ticks: config.phase_ticks,
            };
        }
        ScheduleTable::build(base_tick_ns, &domains[..self.domain_count])
            .map_err(DomainRegistryError::Schedule)
    }

    /// Freeze all registrations and return the schedule used by the cyclic
    /// scheduler. Every Domain must own at least one datagram before it can
    /// become active.
    pub fn activate<const SLOTS: usize>(
        &mut self,
        base_tick_ns: u64,
    ) -> Result<ScheduleTable<DOMAINS, SLOTS>, DomainRegistryError> {
        self.ensure_configuring()?;
        if self.domain_count == 0 {
            return Err(DomainRegistryError::NoDomains);
        }
        if self.slots[..self.domain_count]
            .iter()
            .any(|slot| slot.datagram_count == 0)
        {
            return Err(DomainRegistryError::EmptyDomain);
        }
        let schedule = self.build_schedule(base_tick_ns)?;
        self.phase = DomainRegistryPhase::Active;
        Ok(schedule)
    }

    /// Build and publish one split frame-plan set per Domain while freezing
    /// the registry. All validation runs against temporary plan storage, so a
    /// failed frame-capacity or schedule check leaves both arguments unchanged.
    /// The destination slice is indexed in the same order as `domain_configs()`.
    pub fn activate_with_frame_plans<const SLOTS: usize, const FRAMES: usize, const PLAN: usize>(
        &mut self,
        base_tick_ns: u64,
        destination: &mut [FramePlanSet<FRAMES, PLAN>],
    ) -> Result<ScheduleTable<DOMAINS, SLOTS>, DomainRegistryError> {
        self.ensure_configuring()?;
        if self.domain_count == 0 {
            return Err(DomainRegistryError::NoDomains);
        }
        if self.slots[..self.domain_count]
            .iter()
            .any(|slot| slot.datagram_count == 0)
        {
            return Err(DomainRegistryError::EmptyDomain);
        }
        if destination.len() < self.domain_count {
            return Err(DomainRegistryError::ActivationPlanCapacity);
        }
        if destination[..self.domain_count]
            .iter()
            .any(|plans| !plans.is_empty())
        {
            return Err(DomainRegistryError::ActivationPlanNotEmpty);
        }

        let mut plans = [FramePlanSet::new(); DOMAINS];
        for (index, config) in self.domain_configs().enumerate() {
            self.append_frame_plan_set(config.id, &mut plans[index])?;
        }
        let schedule = self.build_schedule(base_tick_ns)?;

        destination[..self.domain_count].copy_from_slice(&plans[..self.domain_count]);
        self.phase = DomainRegistryPhase::Active;
        Ok(schedule)
    }

    /// Clear an inactive registry. Active configurations must be discarded by
    /// the owner as a whole, so stale handles cannot silently become valid.
    pub fn reset(&mut self) -> Result<(), DomainRegistryError> {
        self.ensure_configuring()?;
        let generation = next_generation(self.generation);
        *self = Self {
            generation,
            ..Self::new()
        };
        Ok(())
    }

    fn register_sii_segment_datagram<
        const SMS: usize,
        const FMMUS: usize,
        const RX_ENTRIES: usize,
        const TX_ENTRIES: usize,
    >(
        &mut self,
        domain_id: u8,
        projection: &SiiDomainProjection<SMS, FMMUS, RX_ENTRIES, TX_ENTRIES>,
        segment: SiiProcessDataSegment,
        fmmu_index: usize,
        spec: SiiSegmentDatagramSpec,
    ) -> Result<(), DomainRegistryError> {
        if segment.logical_bit_offset % 8 != 0
            || segment.physical_bit_offset % 8 != 0
            || segment.bit_length == 0
            || segment.bit_length % 8 != 0
        {
            return Err(DomainRegistryError::SiiSegmentNotByteAddressable);
        }

        let fmmu = projection.mapping().fmmus().get(fmmu_index).ok_or(
            DomainRegistryError::SiiConfiguration(SiiConfigurationError::FmmuCountMismatch),
        )?;
        let payload_len = segment.bit_length / 8;
        if fmmu.length as usize != payload_len {
            return Err(DomainRegistryError::SiiConfiguration(
                SiiConfigurationError::FmmuMappingMismatch,
            ));
        }
        let payload_offset = projection
            .domain_bit_offset(segment.direction, segment.logical_bit_offset)
            .map_err(DomainRegistryError::SiiConfiguration)?
            / 8;
        let (command, input) = match segment.direction {
            PdoDirection::Rx => (Command::Lwr, false),
            PdoDirection::Tx => (Command::Lrd, true),
        };
        self.register_datagram(
            domain_id,
            DomainDatagramSpec::new(
                command,
                spec.index,
                fmmu.logical_start,
                payload_offset,
                payload_len,
                spec.expected_wkc,
                input,
            ),
        )?;
        Ok(())
    }

    fn ensure_configuring(&self) -> Result<(), DomainRegistryError> {
        if self.is_active() {
            Err(DomainRegistryError::ConfigurationLocked)
        } else {
            Ok(())
        }
    }

    fn slot_index(&self, domain_id: u8) -> Result<usize, DomainRegistryError> {
        self.slots[..self.domain_count]
            .iter()
            .position(|slot| slot.configured && slot.config.id == domain_id)
            .ok_or(DomainRegistryError::UnknownDomain)
    }

    fn slot(&self, domain_id: u8) -> Result<&DomainSlot<PDOS, DATAGRAMS>, DomainRegistryError> {
        let index = self.slot_index(domain_id)?;
        Ok(&self.slots[index])
    }
}

impl<const DOMAINS: usize, const PDOS: usize, const DATAGRAMS: usize> Default
    for DomainRegistry<DOMAINS, PDOS, DATAGRAMS>
{
    fn default() -> Self {
        Self::new()
    }
}

fn ranges_overlap(
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
) -> bool {
    left_start < right_end && right_start < left_end
}

fn u64_ranges_overlap(left_start: u64, left_end: u64, right_start: u64, right_end: u64) -> bool {
    left_start < right_end && right_start < left_end
}

fn logical_end(config: DomainConfig) -> Result<u64, DomainRegistryError> {
    let length = u64::try_from(config.process_image_len)
        .map_err(|_| DomainRegistryError::LogicalAddressOverflow)?;
    let end = (config.logical_address as u64)
        .checked_add(length)
        .ok_or(DomainRegistryError::LogicalAddressOverflow)?;
    if end > u32::MAX as u64 + 1 {
        return Err(DomainRegistryError::LogicalAddressOverflow);
    }
    Ok(end)
}

fn next_generation(current: u16) -> u16 {
    let next = current.wrapping_add(1);
    if next == 0 { 1 } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(
        id: u8,
        logical_address: u32,
        image_offset: usize,
        image_len: usize,
        period_ticks: u32,
        phase_ticks: u32,
    ) -> DomainConfig {
        DomainConfig::new(
            id,
            logical_address,
            image_offset,
            image_len,
            period_ticks,
            phase_ticks,
        )
    }

    fn pdo(
        slave_position: u16,
        index: u16,
        direction: PdoDirection,
        bits: u8,
    ) -> PdoRegistrationRequest {
        PdoRegistrationRequest::new(slave_position, index, 0, direction, bits, false)
    }

    #[test]
    fn registration_assigns_offsets_and_absolute_datagram_image_ranges() {
        let mut registry = DomainRegistry::<2, 4, 4>::new();
        registry
            .register_domain(config(0, 0x1000, 8, 8, 1, 0))
            .unwrap();
        let first = registry
            .register_pdo(0, pdo(1, 0x6040, PdoDirection::Rx, 12))
            .unwrap();
        let second = registry
            .register_pdo(0, pdo(1, 0x607A, PdoDirection::Rx, 4))
            .unwrap();
        assert_eq!(registry.pdo(first).unwrap().entry.bit_offset, 0);
        assert_eq!(registry.pdo(second).unwrap().entry.bit_offset, 12);

        let input = registry
            .register_datagram(
                0,
                DomainDatagramSpec::input(Command::Lrw, 7, 0x1000, 0, 4, 1),
            )
            .unwrap();
        let output = registry
            .register_datagram(
                0,
                DomainDatagramSpec::output(Command::Lwr, 8, 0x1004, 4, 2, 1),
            )
            .unwrap();
        assert_eq!(input.plan.payload_offset, 8);
        assert_eq!(output.plan.payload_offset, 12);
        let info = registry.domain(0).unwrap();
        assert_eq!(info.pdo_count, 2);
        assert_eq!(info.datagram_count, 2);
        assert_eq!(info.expected_wkc, 2);
        assert_eq!(info.input_expected_wkc, 1);
        assert_eq!(info.process_image_end, 16);
    }

    #[test]
    fn input_segments_and_frame_plan_are_built_without_partial_updates() {
        let mut registry = DomainRegistry::<1, 2, 2>::new();
        registry
            .register_domain(config(0, 0x1000, 4, 4, 1, 0))
            .unwrap();
        registry
            .register_datagram(
                0,
                DomainDatagramSpec::input(Command::Lrd, 3, 0x1000, 1, 2, 1),
            )
            .unwrap();
        registry
            .register_datagram(
                0,
                DomainDatagramSpec::output(Command::Lwr, 4, 0x1002, 0, 1, 1),
            )
            .unwrap();

        let mut segments = [DomainSegment::EMPTY; 1];
        assert_eq!(registry.copy_domain_segments(0, &mut segments), Ok(1));
        assert_eq!(
            segments[0],
            DomainSegment {
                datagram_index: 3,
                input_offset: 1,
                len: 2,
                expected_wkc: 1,
            }
        );

        let mut plan = FramePlan::<1>::new();
        assert_eq!(
            registry.append_frame_plan(0, &mut plan),
            Err(DomainRegistryError::Plan(PlanError::CapacityExceeded))
        );
        assert!(plan.is_empty());
    }

    #[test]
    fn activation_builds_multi_rate_schedule_and_locks_registration() {
        let mut registry = DomainRegistry::<2, 1, 1>::new();
        registry
            .register_domain(config(0, 0x1000, 0, 2, 1, 0))
            .unwrap();
        registry
            .register_domain(config(1, 0x2000, 2, 2, 4, 2))
            .unwrap();
        registry
            .register_datagram(
                0,
                DomainDatagramSpec::output(Command::Lwr, 1, 0x1000, 0, 1, 1),
            )
            .unwrap();
        registry
            .register_datagram(
                1,
                DomainDatagramSpec::input(Command::Lrd, 2, 0x2000, 0, 1, 1),
            )
            .unwrap();

        let schedule = registry.activate::<8>(250_000).unwrap();
        assert!(registry.is_active());
        assert_eq!(schedule.hyperperiod_ticks(), 4);
        assert_eq!(schedule.due_mask(0), 0b01);
        assert_eq!(schedule.due_mask(2), 0b11);
        assert_eq!(
            registry.register_pdo(0, pdo(1, 0x6041, PdoDirection::Tx, 16)),
            Err(DomainRegistryError::ConfigurationLocked)
        );
    }

    #[test]
    fn invalid_layouts_and_incomplete_activation_fail_closed() {
        let mut registry = DomainRegistry::<2, 2, 1>::new();
        registry
            .register_domain(config(0, 0x1000, 0, 4, 1, 0))
            .unwrap();
        assert_eq!(
            registry.register_domain(config(0, 0x2000, 4, 4, 1, 0)),
            Err(DomainRegistryError::DuplicateDomainId)
        );
        assert_eq!(
            registry.register_domain(config(1, 0x2000, 2, 4, 1, 0)),
            Err(DomainRegistryError::ProcessImageOverlap)
        );
        assert_eq!(
            registry.register_domain(config(1, 0x1002, 4, 4, 1, 0)),
            Err(DomainRegistryError::LogicalAddressOverlap)
        );

        let first = registry
            .register_pdo(0, pdo(1, 0x6040, PdoDirection::Rx, 16))
            .unwrap();
        assert_eq!(
            registry.register_pdo(0, pdo(1, 0x6040, PdoDirection::Rx, 16)),
            Err(DomainRegistryError::DuplicatePdo)
        );
        assert_eq!(
            registry.register_pdo_at(0, 8, pdo(1, 0x607A, PdoDirection::Rx, 16)),
            Err(DomainRegistryError::PdoBitOverlap)
        );
        assert_eq!(registry.pdo(first).unwrap().entry.bit_offset, 0);
        assert!(matches!(
            registry.activate::<4>(1),
            Err(DomainRegistryError::EmptyDomain)
        ));
    }

    #[test]
    fn datagram_and_pdo_validation_is_atomic() {
        let mut registry = DomainRegistry::<2, 2, 2>::new();
        registry
            .register_domain(config(0, 0x1000, 0, 2, 1, 0))
            .unwrap();
        assert_eq!(
            registry.register_pdo(0, pdo(1, 0x6040, PdoDirection::Rx, 0)),
            Err(DomainRegistryError::InvalidBitLength)
        );
        assert_eq!(
            registry.register_pdo_at(0, 9, pdo(1, 0x607A, PdoDirection::Rx, 8)),
            Err(DomainRegistryError::PdoImageOverflow)
        );
        registry
            .register_datagram(
                0,
                DomainDatagramSpec::output(Command::Lwr, 4, 0x1000, 0, 1, 1),
            )
            .unwrap();
        assert_eq!(
            registry.register_datagram(
                0,
                DomainDatagramSpec::input(Command::Lrd, 4, 0x1000, 1, 1, 1),
            ),
            Err(DomainRegistryError::DuplicateDatagramIndex)
        );
        assert_eq!(
            registry.register_datagram(
                0,
                DomainDatagramSpec::input(Command::Lrd, 5, 0x1000, 2, 1, 1),
            ),
            Err(DomainRegistryError::DatagramImageOverflow)
        );
        assert_eq!(registry.domain(0).unwrap().datagram_count, 1);
    }

    #[test]
    fn schedule_failure_does_not_lock_registry() {
        let mut registry = DomainRegistry::<1, 1, 1>::new();
        registry
            .register_domain(config(0, 0x1000, 0, 1, 3, 0))
            .unwrap();
        registry
            .register_datagram(
                0,
                DomainDatagramSpec::output(Command::Lwr, 1, 0x1000, 0, 1, 1),
            )
            .unwrap();
        assert!(matches!(
            registry.activate::<2>(1),
            Err(DomainRegistryError::Schedule(
                ScheduleError::HyperperiodTooLarge
            ))
        ));
        assert_eq!(registry.phase(), DomainRegistryPhase::Configuring);
        registry
            .register_pdo(0, pdo(1, 0x6040, PdoDirection::Rx, 8))
            .unwrap();
    }

    #[test]
    fn reset_invalidates_handles_from_the_previous_configuration_generation() {
        let mut registry = DomainRegistry::<1, 1, 1>::new();
        registry
            .register_domain(config(0, 0x1000, 0, 1, 1, 0))
            .unwrap();
        let old = registry
            .register_pdo(0, pdo(1, 0x6040, PdoDirection::Rx, 8))
            .unwrap();
        let old_generation = registry.generation();
        registry.reset().unwrap();
        assert_ne!(registry.generation(), old_generation);
        assert_eq!(registry.pdo(old), Err(DomainRegistryError::StalePdoHandle));

        registry
            .register_domain(config(0, 0x2000, 0, 1, 1, 0))
            .unwrap();
        let new = registry
            .register_pdo(0, pdo(1, 0x607A, PdoDirection::Rx, 8))
            .unwrap();
        assert_ne!(old.generation(), new.generation());
        assert_eq!(registry.pdo(new).unwrap().entry.index, 0x607A);
    }
}
