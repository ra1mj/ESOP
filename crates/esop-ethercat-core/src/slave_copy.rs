//! Static, quality-carrying PDO copy from one slave to another via the master.

use crate::domain::{Domain, DomainQuality};
use crate::domain_registry::{
    DomainRegistry, DomainRegistryError, DomainRegistryPhase, PdoEntryHandle,
};
use crate::pdo::PdoDirection;
use crate::wire::Command;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlaveCopyError {
    Registry(DomainRegistryError),
    RegistryNotActive,
    SameSlave,
    WrongDirection,
    InvalidWidth,
    UnalignedField,
    QualityFieldOverlap,
    MissingDatagram,
    DomainMismatch,
    ImageBounds,
    TargetNotDue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlaveCopyStatus {
    Valid,
    InvalidSource,
    StaleSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlaveCopyOutcome {
    pub status: SlaveCopyStatus,
    pub source_last_valid_cycle: u64,
    pub source_age_cycles: u64,
    pub target_cycle: u64,
}

/// One immutable link between a source TxPDO and a destination RxPDO.
///
/// The destination quality byte is 1 for fresh verified data, 0 otherwise.
/// A bad source replaces the destination field with the explicitly configured
/// fallback byte. This is ordinary process-data handling, not a safety channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlaveCopyPlan {
    source_domain_id: u8,
    target_domain_id: u8,
    source_address: u32,
    target_address: u32,
    source_offset: usize,
    target_offset: usize,
    quality_offset: usize,
    len: usize,
    source_len: usize,
    target_len: usize,
    source_expected_wkc: u16,
    source_period_ticks: u32,
    source_phase_ticks: u32,
    target_period_ticks: u32,
    target_phase_ticks: u32,
    invalid_fill: u8,
}

impl SlaveCopyPlan {
    /// Bind a plan only after all PDOs and datagrams are frozen by activation.
    /// Source and target fields must be whole bytes of identical width, and
    /// each must be covered by a matching process-data datagram.
    pub fn build<const DOMAINS: usize, const PDOS: usize, const DATAGRAMS: usize>(
        registry: &DomainRegistry<DOMAINS, PDOS, DATAGRAMS>,
        source: PdoEntryHandle,
        target: PdoEntryHandle,
        target_quality: PdoEntryHandle,
        invalid_fill: u8,
    ) -> Result<Self, SlaveCopyError> {
        if registry.phase() != DomainRegistryPhase::Active {
            return Err(SlaveCopyError::RegistryNotActive);
        }
        let source_pdo = registry.pdo(source).map_err(SlaveCopyError::Registry)?;
        let target_pdo = registry.pdo(target).map_err(SlaveCopyError::Registry)?;
        let quality_pdo = registry
            .pdo(target_quality)
            .map_err(SlaveCopyError::Registry)?;
        if source_pdo.slave_position == target_pdo.slave_position {
            return Err(SlaveCopyError::SameSlave);
        }
        if source_pdo.entry.direction != PdoDirection::Tx
            || target_pdo.entry.direction != PdoDirection::Rx
            || quality_pdo.entry.direction != PdoDirection::Rx
            || target.domain_id() != target_quality.domain_id()
            || target_pdo.slave_position != quality_pdo.slave_position
        {
            return Err(SlaveCopyError::WrongDirection);
        }
        let source_field = source_pdo.entry;
        let target_field = target_pdo.entry;
        let quality_field = quality_pdo.entry;
        if source_field.bit_length == 0
            || source_field.bit_length != target_field.bit_length
            || source_field.bit_length % 8 != 0
            || quality_field.bit_length != 8
            || quality_field.signed
        {
            return Err(SlaveCopyError::InvalidWidth);
        }
        if source_field.bit_offset % 8 != 0
            || target_field.bit_offset % 8 != 0
            || quality_field.bit_offset % 8 != 0
        {
            return Err(SlaveCopyError::UnalignedField);
        }
        let len = usize::from(source_field.bit_length / 8);
        let source_offset = source_field.bit_offset / 8;
        let target_offset = target_field.bit_offset / 8;
        let quality_offset = quality_field.bit_offset / 8;
        if (target_offset..target_offset + len).contains(&quality_offset) {
            return Err(SlaveCopyError::QualityFieldOverlap);
        }
        let source_info = registry
            .domain(source.domain_id())
            .map_err(SlaveCopyError::Registry)?;
        let target_info = registry
            .domain(target.domain_id())
            .map_err(SlaveCopyError::Registry)?;
        if source_info.input_expected_wkc == 0
            || !covered_by_datagram(registry, source.domain_id(), source_offset, len, true)?
            || !covered_by_datagram(registry, target.domain_id(), target_offset, len, false)?
            || !covered_by_datagram(registry, target.domain_id(), quality_offset, 1, false)?
        {
            return Err(SlaveCopyError::MissingDatagram);
        }
        Ok(Self {
            source_domain_id: source.domain_id(),
            target_domain_id: target.domain_id(),
            source_address: source_info.config.logical_address,
            target_address: target_info.config.logical_address,
            source_offset,
            target_offset,
            quality_offset,
            len,
            source_len: source_info.config.process_image_len,
            target_len: target_info.config.process_image_len,
            source_expected_wkc: source_info.input_expected_wkc,
            source_period_ticks: source_info.config.period_ticks,
            source_phase_ticks: source_info.config.phase_ticks,
            target_period_ticks: target_info.config.period_ticks,
            target_phase_ticks: target_info.config.phase_ticks,
            invalid_fill,
        })
    }

    pub const fn source_domain_id(&self) -> u8 {
        self.source_domain_id
    }

    pub const fn target_domain_id(&self) -> u8 {
        self.target_domain_id
    }

    /// Call after finishing the source receive and immediately before building
    /// the target frame for `target_cycle`. Errors leave outputs unchanged;
    /// the caller must not transmit the target frame on error.
    pub fn apply_to_domains<
        const SOURCE_BYTES: usize,
        const SOURCE_SEGMENTS: usize,
        const TARGET_BYTES: usize,
        const TARGET_SEGMENTS: usize,
    >(
        &self,
        source: &Domain<SOURCE_BYTES, SOURCE_SEGMENTS>,
        target: &mut Domain<TARGET_BYTES, TARGET_SEGMENTS>,
        target_cycle: u64,
    ) -> Result<SlaveCopyOutcome, SlaveCopyError> {
        self.apply_images(
            source.logical_address(),
            source.input(),
            source.quality(),
            target.logical_address(),
            target.output_mut(),
            target_cycle,
        )
    }

    /// Copy between different slaves mapped into the same Domain.
    pub fn apply_within_domain<const BYTES: usize, const SEGMENTS: usize>(
        &self,
        domain: &mut Domain<BYTES, SEGMENTS>,
        target_cycle: u64,
    ) -> Result<SlaveCopyOutcome, SlaveCopyError> {
        let address = domain.logical_address();
        let quality = domain.quality();
        let (input, output) = domain.process_images_mut();
        self.apply_images(address, input, quality, address, output, target_cycle)
    }

    fn apply_images(
        &self,
        source_address: u32,
        source_image: &[u8],
        source_quality: DomainQuality,
        target_address: u32,
        target_image: &mut [u8],
        target_cycle: u64,
    ) -> Result<SlaveCopyOutcome, SlaveCopyError> {
        if source_address != self.source_address || target_address != self.target_address {
            return Err(SlaveCopyError::DomainMismatch);
        }
        if source_image.len() < self.source_len || target_image.len() < self.target_len {
            return Err(SlaveCopyError::ImageBounds);
        }
        if target_cycle == 0
            || (target_cycle - 1) % u64::from(self.target_period_ticks)
                != u64::from(self.target_phase_ticks)
        {
            return Err(SlaveCopyError::TargetNotDue);
        }

        let age = target_cycle.saturating_sub(source_quality.last_valid_cycle);
        let status = if source_quality.valid
            && source_quality.complete
            && source_quality.expected_wkc == self.source_expected_wkc
            && source_quality.actual_wkc == self.source_expected_wkc
            && source_quality.input_age_cycles == 0
            && source_quality.last_valid_cycle != 0
            && source_quality.last_valid_cycle <= target_cycle
            && (source_quality.last_valid_cycle - 1) % u64::from(self.source_period_ticks)
                == u64::from(self.source_phase_ticks)
        {
            if age <= 1 {
                SlaveCopyStatus::Valid
            } else {
                SlaveCopyStatus::StaleSource
            }
        } else {
            SlaveCopyStatus::InvalidSource
        };
        let destination = &mut target_image[self.target_offset..self.target_offset + self.len];
        if status == SlaveCopyStatus::Valid {
            destination
                .copy_from_slice(&source_image[self.source_offset..self.source_offset + self.len]);
        } else {
            destination.fill(self.invalid_fill);
        }
        target_image[self.quality_offset] = u8::from(status == SlaveCopyStatus::Valid);
        Ok(SlaveCopyOutcome {
            status,
            source_last_valid_cycle: source_quality.last_valid_cycle,
            source_age_cycles: age,
            target_cycle,
        })
    }
}

fn covered_by_datagram<const DOMAINS: usize, const PDOS: usize, const DATAGRAMS: usize>(
    registry: &DomainRegistry<DOMAINS, PDOS, DATAGRAMS>,
    domain_id: u8,
    offset: usize,
    len: usize,
    input: bool,
) -> Result<bool, SlaveCopyError> {
    let info = registry
        .domain(domain_id)
        .map_err(SlaveCopyError::Registry)?;
    let start = info.config.process_image_offset + offset;
    let end = start + len;
    Ok(registry
        .datagrams(domain_id)
        .map_err(SlaveCopyError::Registry)?
        .iter()
        .any(|datagram| {
            let command_ok = if input {
                datagram.input && matches!(datagram.plan.command, Command::Lrd | Command::Lrw)
            } else {
                matches!(datagram.plan.command, Command::Lwr | Command::Lrw)
            };
            command_ok
                && datagram.plan.payload_offset <= start
                && end <= datagram.plan.payload_offset + datagram.plan.payload_len
        }))
}
