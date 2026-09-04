//! Fixed-capacity CoE PDO assignment/mapping configuration.
//!
//! The plan is built before activation and contains only expedited SDO
//! downloads. The controller advances one mailbox transaction at a time, so
//! it can share the existing asynchronous mailbox budget without touching the
//! cyclic PDO path.

use crate::coe::{SdoError, SdoTransfer};
use crate::mailbox::MAX_MAILBOX_BYTES;

pub const MAX_PDO_SDO_DATA: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdoEntrySpec {
    pub index: u16,
    pub subindex: u8,
    pub bit_length: u8,
}

impl PdoEntrySpec {
    pub const fn new(index: u16, subindex: u8, bit_length: u8) -> Self {
        Self {
            index,
            subindex,
            bit_length,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdoSdoWrite {
    pub index: u16,
    pub subindex: u8,
    pub data: [u8; MAX_PDO_SDO_DATA],
    pub data_len: u8,
}

impl PdoSdoWrite {
    pub const EMPTY: Self = Self {
        index: 0,
        subindex: 0,
        data: [0; MAX_PDO_SDO_DATA],
        data_len: 0,
    };

    pub fn new(index: u16, subindex: u8, data: &[u8]) -> Result<Self, PdoConfigPlanError> {
        if data.is_empty() || data.len() > MAX_PDO_SDO_DATA {
            return Err(PdoConfigPlanError::DataLengthOutOfBounds);
        }
        let mut write = Self {
            index,
            subindex,
            data: [0; MAX_PDO_SDO_DATA],
            data_len: data.len() as u8,
        };
        write.data[..data.len()].copy_from_slice(data);
        Ok(write)
    }
}

pub struct PdoConfigPlan<const OPS: usize> {
    writes: [PdoSdoWrite; OPS],
    count: usize,
}

impl<const OPS: usize> PdoConfigPlan<OPS> {
    pub const fn new() -> Self {
        Self {
            writes: [PdoSdoWrite::EMPTY; OPS],
            count: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn writes(&self) -> &[PdoSdoWrite] {
        &self.writes[..self.count]
    }

    pub fn push(&mut self, write: PdoSdoWrite) -> Result<(), PdoConfigPlanError> {
        if self.count >= OPS {
            return Err(PdoConfigPlanError::CapacityExceeded);
        }
        self.writes[self.count] = write;
        self.count += 1;
        Ok(())
    }

    /// Append the standard sequence for one PDO mapping object:
    /// clear count, write entries, then publish the final count.
    pub fn append_mapping(
        &mut self,
        mapping_index: u16,
        entries: &[PdoEntrySpec],
    ) -> Result<(), PdoConfigPlanError> {
        if entries.len() > u8::MAX as usize {
            return Err(PdoConfigPlanError::CountOutOfBounds);
        }
        if entries
            .iter()
            .any(|entry| !(1..=64).contains(&entry.bit_length))
        {
            return Err(PdoConfigPlanError::InvalidBitLength);
        }
        let required = entries.len().saturating_add(2);
        if self.count.saturating_add(required) > OPS {
            return Err(PdoConfigPlanError::CapacityExceeded);
        }
        self.push(PdoSdoWrite::new(mapping_index, 0, &[0])?)?;
        for (offset, entry) in entries.iter().enumerate() {
            let subindex =
                u8::try_from(offset + 1).map_err(|_| PdoConfigPlanError::CountOutOfBounds)?;
            let packed = (entry.index as u32)
                | ((entry.subindex as u32) << 16)
                | ((entry.bit_length as u32) << 24);
            self.push(PdoSdoWrite::new(
                mapping_index,
                subindex,
                &packed.to_le_bytes(),
            )?)?;
        }
        self.push(PdoSdoWrite::new(mapping_index, 0, &[entries.len() as u8])?)?;
        Ok(())
    }

    /// Append the standard sequence for one assignment object:
    /// clear count, write mapping object indexes, then publish the count.
    pub fn append_assignment(
        &mut self,
        assignment_index: u16,
        mapping_indexes: &[u16],
    ) -> Result<(), PdoConfigPlanError> {
        if mapping_indexes.len() > u8::MAX as usize {
            return Err(PdoConfigPlanError::CountOutOfBounds);
        }
        let required = mapping_indexes.len().saturating_add(2);
        if self.count.saturating_add(required) > OPS {
            return Err(PdoConfigPlanError::CapacityExceeded);
        }
        self.push(PdoSdoWrite::new(assignment_index, 0, &[0])?)?;
        for (offset, mapping_index) in mapping_indexes.iter().enumerate() {
            let subindex =
                u8::try_from(offset + 1).map_err(|_| PdoConfigPlanError::CountOutOfBounds)?;
            self.push(PdoSdoWrite::new(
                assignment_index,
                subindex,
                &mapping_index.to_le_bytes(),
            )?)?;
        }
        self.push(PdoSdoWrite::new(
            assignment_index,
            0,
            &[mapping_indexes.len() as u8],
        )?)?;
        Ok(())
    }
}

impl<const OPS: usize> Default for PdoConfigPlan<OPS> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdoConfigPlanError {
    CapacityExceeded,
    CountOutOfBounds,
    DataLengthOutOfBounds,
    InvalidBitLength,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdoConfigPhase {
    Idle,
    Sending,
    AwaitingResponse,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdoConfigAction {
    pub token: u8,
    pub generation: u16,
    pub station_address: u16,
    pub operation_index: u16,
    pub sdo_index: u16,
    pub sdo_subindex: u8,
    pub request_payload: [u8; MAX_MAILBOX_BYTES],
    pub request_len: u8,
    pub deadline_ns: u64,
}

impl PdoConfigAction {
    pub fn payload(&self) -> &[u8] {
        &self.request_payload[..self.request_len as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdoConfigProgress {
    Advanced,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdoConfigError {
    Busy,
    NotStarted,
    NoPendingAction,
    ActionMismatch,
    GenerationMismatch,
    PayloadLengthMismatch,
    Timeout,
    RequestTooLarge,
    Plan(PdoConfigPlanError),
    Sdo(SdoError),
}

pub struct PdoConfigController<const OPS: usize> {
    phase: PdoConfigPhase,
    plan: PdoConfigPlan<OPS>,
    operation_index: usize,
    station_address: u16,
    generation: u16,
    configuration_deadline_ns: u64,
    request_timeout_ns: u64,
    transfer: SdoTransfer,
    pending: Option<PdoConfigAction>,
    next_token: u8,
    last_error: Option<PdoConfigError>,
}

impl<const OPS: usize> PdoConfigController<OPS> {
    pub const fn new() -> Self {
        Self {
            phase: PdoConfigPhase::Idle,
            plan: PdoConfigPlan::new(),
            operation_index: 0,
            station_address: 0,
            generation: 0,
            configuration_deadline_ns: 0,
            request_timeout_ns: 0,
            transfer: SdoTransfer::new(),
            pending: None,
            next_token: 1,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> PdoConfigPhase {
        self.phase
    }

    pub const fn pending(&self) -> Option<PdoConfigAction> {
        self.pending
    }

    pub const fn operation_index(&self) -> usize {
        self.operation_index
    }

    pub const fn last_error(&self) -> Option<PdoConfigError> {
        self.last_error
    }

    pub fn start(
        &mut self,
        plan: PdoConfigPlan<OPS>,
        station_address: u16,
        generation: u16,
        now_ns: u64,
        timeout_ns: u64,
        request_timeout_ns: u64,
    ) -> Result<(), PdoConfigError> {
        if !matches!(
            self.phase,
            PdoConfigPhase::Idle | PdoConfigPhase::Complete | PdoConfigPhase::Faulted
        ) {
            return Err(PdoConfigError::Busy);
        }
        self.plan = plan;
        self.operation_index = 0;
        self.station_address = station_address;
        self.generation = generation;
        self.configuration_deadline_ns = now_ns.saturating_add(timeout_ns);
        self.request_timeout_ns = request_timeout_ns;
        self.pending = None;
        self.next_token = 1;
        self.last_error = None;
        if self.plan.is_empty() {
            self.phase = PdoConfigPhase::Complete;
            return Ok(());
        }
        self.start_current_transfer()?;
        self.phase = PdoConfigPhase::Sending;
        Ok(())
    }

    pub fn next_action(&mut self, now_ns: u64) -> Result<Option<PdoConfigAction>, PdoConfigError> {
        if self.phase == PdoConfigPhase::Idle {
            return Err(PdoConfigError::NotStarted);
        }
        if matches!(
            self.phase,
            PdoConfigPhase::Complete | PdoConfigPhase::Faulted
        ) {
            return Ok(None);
        }
        if let Some(action) = self.pending {
            return Ok(Some(action));
        }
        if now_ns >= self.configuration_deadline_ns {
            return self.fail(PdoConfigError::Timeout);
        }
        if self.phase != PdoConfigPhase::Sending {
            return self.fail(PdoConfigError::NoPendingAction);
        }
        let request = match self.transfer.request() {
            Some(request) => request,
            None => return self.fail(PdoConfigError::Sdo(SdoError::InvalidState)),
        };
        if request.len() > MAX_MAILBOX_BYTES {
            return self.fail(PdoConfigError::RequestTooLarge);
        }
        let write = self.plan.writes()[self.operation_index];
        let mut request_payload = [0; MAX_MAILBOX_BYTES];
        request_payload[..request.len()].copy_from_slice(request);
        let action = PdoConfigAction {
            token: self.next_token,
            generation: self.generation,
            station_address: self.station_address,
            operation_index: self.operation_index as u16,
            sdo_index: write.index,
            sdo_subindex: write.subindex,
            request_payload,
            request_len: request.len() as u8,
            deadline_ns: now_ns
                .saturating_add(self.request_timeout_ns)
                .min(self.configuration_deadline_ns),
        };
        self.next_token = self.next_token.wrapping_add(1).max(1);
        self.pending = Some(action);
        self.phase = PdoConfigPhase::AwaitingResponse;
        Ok(Some(action))
    }

    pub fn accept(
        &mut self,
        action: PdoConfigAction,
        generation: u16,
        response: &[u8],
        now_ns: u64,
    ) -> Result<PdoConfigProgress, PdoConfigError> {
        if self.pending != Some(action) {
            return self.fail(PdoConfigError::ActionMismatch);
        }
        if action.generation != generation {
            return self.fail(PdoConfigError::GenerationMismatch);
        }
        if now_ns > action.deadline_ns {
            return self.fail(PdoConfigError::Timeout);
        }
        if response.is_empty() || response.len() > MAX_MAILBOX_BYTES {
            return self.fail(PdoConfigError::PayloadLengthMismatch);
        }
        match self.transfer.accept_response(response) {
            Ok(crate::coe::SdoProgress::Advanced) => {
                self.pending = None;
                self.phase = PdoConfigPhase::Sending;
                Ok(PdoConfigProgress::Advanced)
            }
            Ok(crate::coe::SdoProgress::Complete) => {
                self.pending = None;
                self.operation_index += 1;
                if self.operation_index >= self.plan.len() {
                    self.phase = PdoConfigPhase::Complete;
                    Ok(PdoConfigProgress::Complete)
                } else {
                    self.start_current_transfer()?;
                    self.phase = PdoConfigPhase::Sending;
                    Ok(PdoConfigProgress::Advanced)
                }
            }
            Err(error) => self.fail(PdoConfigError::Sdo(error)),
        }
    }

    pub fn timeout(
        &mut self,
        action: PdoConfigAction,
        now_ns: u64,
    ) -> Result<PdoConfigProgress, PdoConfigError> {
        if self.pending != Some(action) {
            return self.fail(PdoConfigError::ActionMismatch);
        }
        if now_ns < action.deadline_ns {
            return Err(PdoConfigError::Timeout);
        }
        self.fail(PdoConfigError::Timeout)
    }

    fn start_current_transfer(&mut self) -> Result<(), PdoConfigError> {
        let write = self.plan.writes()[self.operation_index];
        self.transfer
            .start_download(
                write.index,
                write.subindex,
                &write.data[..write.data_len as usize],
                false,
            )
            .map_err(PdoConfigError::Sdo)
    }

    fn fail<T>(&mut self, error: PdoConfigError) -> Result<T, PdoConfigError> {
        self.last_error = Some(error);
        self.pending = None;
        self.phase = PdoConfigPhase::Faulted;
        Err(error)
    }
}

impl<const OPS: usize> Default for PdoConfigController<OPS> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coe::{CoeHeader, CoeService};

    #[test]
    fn plan_emits_clear_entries_and_final_counts_in_order() {
        let mut plan = PdoConfigPlan::<8>::new();
        plan.append_mapping(
            0x1600,
            &[
                PdoEntrySpec::new(0x6040, 0, 16),
                PdoEntrySpec::new(0x607A, 0, 32),
            ],
        )
        .unwrap();
        plan.append_assignment(0x1C12, &[0x1600]).unwrap();

        assert_eq!(plan.len(), 7);
        assert_eq!(plan.writes()[0], PdoSdoWrite::new(0x1600, 0, &[0]).unwrap());
        assert_eq!(
            plan.writes()[1],
            PdoSdoWrite::new(0x1600, 1, &0x1000_6040u32.to_le_bytes()).unwrap()
        );
        assert_eq!(
            plan.writes()[2],
            PdoSdoWrite::new(0x1600, 2, &0x2000_607Au32.to_le_bytes()).unwrap()
        );
        assert_eq!(plan.writes()[3], PdoSdoWrite::new(0x1600, 0, &[2]).unwrap());
        assert_eq!(plan.writes()[4], PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap());
        assert_eq!(
            plan.writes()[5],
            PdoSdoWrite::new(0x1C12, 1, &0x1600u16.to_le_bytes()).unwrap()
        );
        assert_eq!(plan.writes()[6], PdoSdoWrite::new(0x1C12, 0, &[1]).unwrap());
    }

    #[test]
    fn controller_runs_each_pdo_write_through_expedited_sdo() {
        let mut plan = PdoConfigPlan::<3>::new();
        plan.append_assignment(0x1C12, &[0x1600]).unwrap();
        let mut controller = PdoConfigController::<3>::new();
        controller.start(plan, 0x1000, 9, 0, 10_000, 100).unwrap();

        for expected_operation in 0..3 {
            let action = controller
                .next_action(1 + expected_operation as u64)
                .unwrap()
                .unwrap();
            assert_eq!(action.operation_index, expected_operation as u16);
            let mut response = [0; 6];
            CoeHeader {
                number: 0,
                service: CoeService::SdoResponse,
            }
            .encode(&mut response)
            .unwrap();
            response[2] = 0x60;
            response[3..5].copy_from_slice(&action.sdo_index.to_le_bytes());
            response[5] = action.sdo_subindex;
            let progress = controller.accept(action, 9, &response, 2).unwrap();
            let expected_progress = if expected_operation == 2 {
                PdoConfigProgress::Complete
            } else {
                PdoConfigProgress::Advanced
            };
            assert_eq!(progress, expected_progress);
        }

        assert_eq!(controller.phase(), PdoConfigPhase::Complete);
        assert_eq!(controller.next_action(3), Ok(None));
    }

    #[test]
    fn capacity_failure_does_not_leave_a_partial_mapping() {
        let mut plan = PdoConfigPlan::<3>::new();
        plan.push(PdoSdoWrite::new(0x6040, 0, &[0x06, 0]).unwrap())
            .unwrap();

        assert_eq!(
            plan.append_mapping(0x1600, &[PdoEntrySpec::new(0x6040, 0, 16)]),
            Err(PdoConfigPlanError::CapacityExceeded)
        );
        assert_eq!(plan.len(), 1);
        assert_eq!(plan.writes()[0].index, 0x6040);
    }

    #[test]
    fn invalid_bit_length_does_not_leave_a_partial_mapping() {
        let mut plan = PdoConfigPlan::<4>::new();
        assert_eq!(
            plan.append_mapping(0x1600, &[PdoEntrySpec::new(0x6040, 0, 0)]),
            Err(PdoConfigPlanError::InvalidBitLength)
        );
        assert!(plan.is_empty());
    }

    #[test]
    fn controller_rejects_stale_action_without_advancing_plan() {
        let mut plan = PdoConfigPlan::<1>::new();
        plan.push(PdoSdoWrite::new(0x6040, 0, &[0x06, 0]).unwrap())
            .unwrap();
        let mut controller = PdoConfigController::<1>::new();
        controller.start(plan, 0x1000, 9, 0, 10_000, 100).unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        let mut stale = action;
        stale.token = stale.token.wrapping_add(1).max(1);
        assert_eq!(
            controller.accept(stale, 9, &[0; 6], 2),
            Err(PdoConfigError::ActionMismatch)
        );
        assert_eq!(controller.phase(), PdoConfigPhase::Faulted);
        assert_eq!(controller.operation_index(), 0);
    }

    #[test]
    fn controller_latches_when_the_sdo_transfer_is_missing() {
        let mut plan = PdoConfigPlan::<1>::new();
        plan.push(PdoSdoWrite::new(0x6040, 0, &[0x06, 0]).unwrap())
            .unwrap();
        let mut controller = PdoConfigController::<1>::new();
        controller.start(plan, 0x1000, 9, 0, 10_000, 100).unwrap();
        controller.transfer = SdoTransfer::new();

        assert_eq!(
            controller.next_action(1),
            Err(PdoConfigError::Sdo(SdoError::InvalidState))
        );
        assert_eq!(controller.phase(), PdoConfigPhase::Faulted);
    }
}
