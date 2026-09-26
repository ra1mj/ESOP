//! Fixed-capacity CoE PDO assignment/mapping configuration.
//!
//! The plan is built before activation and contains only expedited SDO write
//! values. The controller downloads each value and then uploads the same
//! object for exact readback verification. It advances one mailbox transaction
//! at a time, so it can share the existing asynchronous mailbox budget without
//! touching the cyclic PDO path.

use crate::coe::{SdoError, SdoTransfer};
use crate::mailbox::{MAX_MAILBOX_BYTES, MailboxConfig, MailboxController, MailboxError};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
pub struct PdoConfigJob<const OPS: usize> {
    station_address: u16,
    plan: PdoConfigPlan<OPS>,
    mailbox_config: MailboxConfig,
}

impl<const OPS: usize> PdoConfigJob<OPS> {
    pub const EMPTY: Self = Self {
        station_address: 0,
        plan: PdoConfigPlan::new(),
        mailbox_config: MailboxConfig::new(0, 0, 0, 0),
    };

    pub const fn new(
        station_address: u16,
        plan: PdoConfigPlan<OPS>,
        mailbox_config: MailboxConfig,
    ) -> Self {
        Self {
            station_address,
            plan,
            mailbox_config,
        }
    }

    pub const fn station_address(&self) -> u16 {
        self.station_address
    }

    pub const fn plan(&self) -> &PdoConfigPlan<OPS> {
        &self.plan
    }

    pub const fn mailbox_config(&self) -> MailboxConfig {
        self.mailbox_config
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdoConfigBatchPlan<const JOBS: usize, const OPS: usize> {
    jobs: [PdoConfigJob<OPS>; JOBS],
    count: usize,
}

impl<const JOBS: usize, const OPS: usize> PdoConfigBatchPlan<JOBS, OPS> {
    pub const fn new() -> Self {
        Self {
            jobs: [PdoConfigJob::EMPTY; JOBS],
            count: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn jobs(&self) -> &[PdoConfigJob<OPS>] {
        &self.jobs[..self.count]
    }

    pub fn push(&mut self, job: PdoConfigJob<OPS>) -> Result<(), PdoConfigBatchPlanError> {
        if self.count >= JOBS {
            return Err(PdoConfigBatchPlanError::CapacityExceeded);
        }
        if self.jobs[..self.count]
            .iter()
            .any(|existing| existing.station_address == job.station_address)
        {
            return Err(PdoConfigBatchPlanError::DuplicateStationAddress {
                station_address: job.station_address,
            });
        }
        self.jobs[self.count] = job;
        self.count += 1;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), PdoConfigBatchPlanError> {
        if self.is_empty() {
            Err(PdoConfigBatchPlanError::Empty)
        } else {
            Ok(())
        }
    }
}

impl<const JOBS: usize, const OPS: usize> Default for PdoConfigBatchPlan<JOBS, OPS> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdoConfigBatchPlanError {
    Empty,
    CapacityExceeded,
    DuplicateStationAddress { station_address: u16 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdoConfigBatchPhase {
    Idle,
    Configuring,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdoConfigBatchStatus {
    pub phase: PdoConfigBatchPhase,
    pub current_index: usize,
    pub job_count: usize,
    pub station_address: Option<u16>,
    pub generation: Option<u16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdoConfigBatchError {
    Busy,
    NotStarted,
    GenerationOutOfBounds,
    Plan(PdoConfigBatchPlanError),
    Configuration(PdoConfigError),
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
pub enum PdoConfigStep {
    Download,
    VerifyUpload,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdoConfigAction {
    pub token: u8,
    pub generation: u16,
    pub station_address: u16,
    pub operation_index: u16,
    pub step: PdoConfigStep,
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
    ReadbackLengthMismatch {
        expected: u8,
        actual: u8,
    },
    ReadbackValueMismatch {
        byte_index: u8,
        expected: u8,
        actual: u8,
    },
    Mailbox(MailboxError),
    Plan(PdoConfigPlanError),
    Sdo(SdoError),
}

pub struct PdoConfigController<const OPS: usize> {
    phase: PdoConfigPhase,
    plan: PdoConfigPlan<OPS>,
    operation_index: usize,
    step: PdoConfigStep,
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
            step: PdoConfigStep::Download,
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

    pub const fn step(&self) -> PdoConfigStep {
        self.step
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
        self.step = PdoConfigStep::Download;
        self.station_address = station_address;
        self.generation = generation;
        self.configuration_deadline_ns = now_ns.saturating_add(timeout_ns);
        self.request_timeout_ns = request_timeout_ns;
        self.transfer = SdoTransfer::new();
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
            step: self.step,
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
                match self.step {
                    PdoConfigStep::Download => {
                        self.step = PdoConfigStep::VerifyUpload;
                        if let Err(error) = self.start_current_transfer() {
                            return self.fail(error);
                        }
                        self.phase = PdoConfigPhase::Sending;
                        Ok(PdoConfigProgress::Advanced)
                    }
                    PdoConfigStep::VerifyUpload => {
                        self.verify_readback()?;
                        self.operation_index += 1;
                        self.step = PdoConfigStep::Download;
                        if self.operation_index >= self.plan.len() {
                            self.phase = PdoConfigPhase::Complete;
                            Ok(PdoConfigProgress::Complete)
                        } else {
                            if let Err(error) = self.start_current_transfer() {
                                return self.fail(error);
                            }
                            self.phase = PdoConfigPhase::Sending;
                            Ok(PdoConfigProgress::Advanced)
                        }
                    }
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

    pub fn mailbox_failed(
        &mut self,
        action: PdoConfigAction,
        error: MailboxError,
    ) -> Result<PdoConfigProgress, PdoConfigError> {
        if self.pending != Some(action) {
            return self.fail(PdoConfigError::ActionMismatch);
        }
        self.fail(PdoConfigError::Mailbox(error))
    }

    fn start_current_transfer(&mut self) -> Result<(), PdoConfigError> {
        let write = self.plan.writes()[self.operation_index];
        match self.step {
            PdoConfigStep::Download => self
                .transfer
                .start_download(
                    write.index,
                    write.subindex,
                    &write.data[..write.data_len as usize],
                    false,
                )
                .map_err(PdoConfigError::Sdo),
            PdoConfigStep::VerifyUpload => self
                .transfer
                .start_upload(write.index, write.subindex, false)
                .map_err(PdoConfigError::Sdo),
        }
    }

    fn verify_readback(&mut self) -> Result<(), PdoConfigError> {
        let write = self.plan.writes()[self.operation_index];
        let actual_len = self.transfer.data_len();
        if actual_len != write.data_len as usize {
            return self.fail(PdoConfigError::ReadbackLengthMismatch {
                expected: write.data_len,
                actual: actual_len.min(u8::MAX as usize) as u8,
            });
        }
        for byte_index in 0..actual_len {
            let expected = write.data[byte_index];
            let actual = self.transfer.data()[byte_index];
            if actual != expected {
                return self.fail(PdoConfigError::ReadbackValueMismatch {
                    byte_index: byte_index as u8,
                    expected,
                    actual,
                });
            }
        }
        Ok(())
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

pub struct PdoConfigBatch<const JOBS: usize, const OPS: usize> {
    phase: PdoConfigBatchPhase,
    plan: PdoConfigBatchPlan<JOBS, OPS>,
    current_index: usize,
    base_generation: u16,
    timeout_ns: u64,
    request_timeout_ns: u64,
    controller: PdoConfigController<OPS>,
    mailbox: MailboxController,
    last_error: Option<PdoConfigBatchError>,
}

impl<const JOBS: usize, const OPS: usize> PdoConfigBatch<JOBS, OPS> {
    pub const fn new() -> Self {
        Self {
            phase: PdoConfigBatchPhase::Idle,
            plan: PdoConfigBatchPlan::new(),
            current_index: 0,
            base_generation: 0,
            timeout_ns: 0,
            request_timeout_ns: 0,
            controller: PdoConfigController::new(),
            mailbox: MailboxController::new(),
            last_error: None,
        }
    }

    pub fn phase(&self) -> PdoConfigBatchPhase {
        if self.phase == PdoConfigBatchPhase::Configuring
            && self.controller.phase() == PdoConfigPhase::Faulted
        {
            PdoConfigBatchPhase::Faulted
        } else {
            self.phase
        }
    }

    pub const fn current_index(&self) -> usize {
        self.current_index
    }

    pub const fn controller(&self) -> &PdoConfigController<OPS> {
        &self.controller
    }

    pub fn controller_mut(&mut self) -> &mut PdoConfigController<OPS> {
        &mut self.controller
    }

    pub const fn mailbox(&self) -> &MailboxController {
        &self.mailbox
    }

    pub fn mailbox_mut(&mut self) -> &mut MailboxController {
        &mut self.mailbox
    }

    pub const fn current_mailbox_config(&self) -> Option<MailboxConfig> {
        if self.current_index < self.plan.count {
            Some(self.plan.jobs[self.current_index].mailbox_config)
        } else {
            None
        }
    }

    pub const fn last_error(&self) -> Option<PdoConfigBatchError> {
        match self.last_error {
            Some(error) => Some(error),
            None => match self.controller.last_error() {
                Some(error) => Some(PdoConfigBatchError::Configuration(error)),
                None => None,
            },
        }
    }

    pub fn status(&self) -> PdoConfigBatchStatus {
        let current = if self.current_index < self.plan.count {
            Some(self.plan.jobs[self.current_index])
        } else {
            None
        };
        PdoConfigBatchStatus {
            phase: self.phase(),
            current_index: self.current_index,
            job_count: self.plan.count,
            station_address: current.map(|job| job.station_address),
            generation: current
                .map(|_| self.base_generation.wrapping_add(self.current_index as u16)),
        }
    }

    pub fn start(
        &mut self,
        plan: PdoConfigBatchPlan<JOBS, OPS>,
        base_generation: u16,
        now_ns: u64,
        timeout_ns: u64,
        request_timeout_ns: u64,
    ) -> Result<(), PdoConfigBatchError> {
        if self.phase() == PdoConfigBatchPhase::Configuring {
            return Err(PdoConfigBatchError::Busy);
        }
        plan.validate().map_err(PdoConfigBatchError::Plan)?;
        let last_offset = u16::try_from(plan.count - 1)
            .map_err(|_| PdoConfigBatchError::GenerationOutOfBounds)?;
        base_generation
            .checked_add(last_offset)
            .ok_or(PdoConfigBatchError::GenerationOutOfBounds)?;

        self.phase = PdoConfigBatchPhase::Configuring;
        self.plan = plan;
        self.current_index = 0;
        self.base_generation = base_generation;
        self.timeout_ns = timeout_ns;
        self.request_timeout_ns = request_timeout_ns;
        self.controller = PdoConfigController::new();
        self.mailbox = MailboxController::new();
        self.last_error = None;
        self.start_current_or_finish(now_ns)
    }

    pub fn advance(&mut self, now_ns: u64) -> Result<PdoConfigBatchStatus, PdoConfigBatchError> {
        match self.phase() {
            PdoConfigBatchPhase::Idle => return Err(PdoConfigBatchError::NotStarted),
            PdoConfigBatchPhase::Complete => return Ok(self.status()),
            PdoConfigBatchPhase::Faulted => {
                let error = self
                    .last_error()
                    .unwrap_or(PdoConfigBatchError::Configuration(
                        PdoConfigError::NoPendingAction,
                    ));
                self.phase = PdoConfigBatchPhase::Faulted;
                self.last_error = Some(error);
                return Err(error);
            }
            PdoConfigBatchPhase::Configuring => {}
        }
        if self.controller.phase() != PdoConfigPhase::Complete {
            return Ok(self.status());
        }
        self.current_index += 1;
        self.start_current_or_finish(now_ns)?;
        Ok(self.status())
    }

    fn start_current_or_finish(&mut self, now_ns: u64) -> Result<(), PdoConfigBatchError> {
        let mut remaining = JOBS;
        while self.current_index < self.plan.count && remaining != 0 {
            let job = self.plan.jobs[self.current_index];
            let generation = self
                .base_generation
                .checked_add(self.current_index as u16)
                .ok_or(PdoConfigBatchError::GenerationOutOfBounds)?;
            if let Err(error) = self.controller.start(
                job.plan,
                job.station_address,
                generation,
                now_ns,
                self.timeout_ns,
                self.request_timeout_ns,
            ) {
                let error = PdoConfigBatchError::Configuration(error);
                self.phase = PdoConfigBatchPhase::Faulted;
                self.last_error = Some(error);
                return Err(error);
            }
            self.phase = PdoConfigBatchPhase::Configuring;
            if self.controller.phase() != PdoConfigPhase::Complete {
                return Ok(());
            }
            self.current_index += 1;
            remaining -= 1;
        }
        self.phase = PdoConfigBatchPhase::Complete;
        Ok(())
    }
}

impl<const JOBS: usize, const OPS: usize> Default for PdoConfigBatch<JOBS, OPS> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coe::{CoeHeader, CoeService};

    fn response_header(dst: &mut [u8]) {
        CoeHeader {
            number: 0,
            service: CoeService::SdoResponse,
        }
        .encode(dst)
        .unwrap();
    }

    fn download_response(action: PdoConfigAction) -> [u8; 6] {
        let mut response = [0; 6];
        response_header(&mut response);
        response[2] = 0x60;
        response[3..5].copy_from_slice(&action.sdo_index.to_le_bytes());
        response[5] = action.sdo_subindex;
        response
    }

    fn expedited_upload_response(action: PdoConfigAction, data: &[u8]) -> [u8; 10] {
        assert!((1..=4).contains(&data.len()));
        let mut response = [0; 10];
        response_header(&mut response);
        response[2] = 0x43 | (((4 - data.len()) as u8) << 2);
        response[3..5].copy_from_slice(&action.sdo_index.to_le_bytes());
        response[5] = action.sdo_subindex;
        response[6..6 + data.len()].copy_from_slice(data);
        response
    }

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
    fn controller_downloads_and_verifies_every_pdo_write() {
        let mut plan = PdoConfigPlan::<3>::new();
        plan.append_assignment(0x1C12, &[0x1600]).unwrap();
        let mut controller = PdoConfigController::<3>::new();
        controller.start(plan, 0x1000, 9, 0, 10_000, 100).unwrap();
        let expected_values: [&[u8]; 3] = [&[0], &0x1600u16.to_le_bytes(), &[1]];

        for (expected_operation, expected_value) in expected_values.iter().enumerate() {
            let download = controller
                .next_action(1 + expected_operation as u64)
                .unwrap()
                .unwrap();
            assert_eq!(download.operation_index, expected_operation as u16);
            assert_eq!(download.step, PdoConfigStep::Download);
            assert_eq!(controller.step(), PdoConfigStep::Download);
            assert_eq!(
                controller
                    .accept(download, 9, &download_response(download), 2)
                    .unwrap(),
                PdoConfigProgress::Advanced
            );
            assert_eq!(controller.operation_index(), expected_operation);

            let verify = controller.next_action(3).unwrap().unwrap();
            assert_eq!(verify.operation_index, expected_operation as u16);
            assert_eq!(verify.step, PdoConfigStep::VerifyUpload);
            assert_eq!(verify.payload()[2], 0x40);
            let response = expedited_upload_response(verify, expected_value);
            let progress = controller.accept(verify, 9, &response, 4).unwrap();
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
    fn controller_accepts_segmented_verification_upload() {
        let mut plan = PdoConfigPlan::<1>::new();
        plan.push(PdoSdoWrite::new(0x6040, 0, &[0x06, 0]).unwrap())
            .unwrap();
        let mut controller = PdoConfigController::<1>::new();
        controller.start(plan, 0x1000, 9, 0, 10_000, 100).unwrap();

        let download = controller.next_action(1).unwrap().unwrap();
        controller
            .accept(download, 9, &download_response(download), 2)
            .unwrap();
        let initiate = controller.next_action(3).unwrap().unwrap();
        let mut initiate_response = [0; 10];
        response_header(&mut initiate_response);
        initiate_response[2] = 0x41;
        initiate_response[3..5].copy_from_slice(&initiate.sdo_index.to_le_bytes());
        initiate_response[5] = initiate.sdo_subindex;
        initiate_response[6..10].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(
            controller
                .accept(initiate, 9, &initiate_response, 4)
                .unwrap(),
            PdoConfigProgress::Advanced
        );

        let segment = controller.next_action(5).unwrap().unwrap();
        assert_eq!(segment.step, PdoConfigStep::VerifyUpload);
        assert_eq!(segment.payload(), &[0, 0x20, 0x60]);
        let mut segment_response = [0; 10];
        response_header(&mut segment_response);
        segment_response[2] = 0x0B;
        segment_response[3..5].copy_from_slice(&[0x06, 0]);
        assert_eq!(
            controller.accept(segment, 9, &segment_response, 6).unwrap(),
            PdoConfigProgress::Complete
        );
        assert_eq!(controller.operation_index(), 1);
    }

    #[test]
    fn readback_length_mismatch_faults_without_advancing() {
        let mut plan = PdoConfigPlan::<1>::new();
        plan.push(PdoSdoWrite::new(0x6040, 0, &[0x06, 0]).unwrap())
            .unwrap();
        let mut controller = PdoConfigController::<1>::new();
        controller.start(plan, 0x1000, 9, 0, 10_000, 100).unwrap();
        let download = controller.next_action(1).unwrap().unwrap();
        controller
            .accept(download, 9, &download_response(download), 2)
            .unwrap();
        let verify = controller.next_action(3).unwrap().unwrap();

        assert_eq!(
            controller.accept(verify, 9, &expedited_upload_response(verify, &[0x06]), 4,),
            Err(PdoConfigError::ReadbackLengthMismatch {
                expected: 2,
                actual: 1,
            })
        );
        assert_eq!(controller.phase(), PdoConfigPhase::Faulted);
        assert_eq!(controller.operation_index(), 0);
    }

    #[test]
    fn readback_value_mismatch_reports_first_changed_byte() {
        let mut plan = PdoConfigPlan::<1>::new();
        plan.push(PdoSdoWrite::new(0x6040, 0, &[0x06, 0]).unwrap())
            .unwrap();
        let mut controller = PdoConfigController::<1>::new();
        controller.start(plan, 0x1000, 9, 0, 10_000, 100).unwrap();
        let download = controller.next_action(1).unwrap().unwrap();
        controller
            .accept(download, 9, &download_response(download), 2)
            .unwrap();
        let verify = controller.next_action(3).unwrap().unwrap();

        assert_eq!(
            controller.accept(
                verify,
                9,
                &expedited_upload_response(verify, &[0x06, 0x7F]),
                4,
            ),
            Err(PdoConfigError::ReadbackValueMismatch {
                byte_index: 1,
                expected: 0,
                actual: 0x7F,
            })
        );
        assert_eq!(controller.operation_index(), 0);
    }

    #[test]
    fn verification_action_matching_generation_and_deadline_fail_closed() {
        let mut plan = PdoConfigPlan::<1>::new();
        plan.push(PdoSdoWrite::new(0x6040, 0, &[0x06, 0]).unwrap())
            .unwrap();
        let mut controller = PdoConfigController::<1>::new();
        controller.start(plan, 0x1000, 9, 0, 10_000, 100).unwrap();
        let download = controller.next_action(1).unwrap().unwrap();
        controller
            .accept(download, 9, &download_response(download), 2)
            .unwrap();
        let verify = controller.next_action(3).unwrap().unwrap();
        let mut modified = verify;
        modified.step = PdoConfigStep::Download;
        assert_eq!(
            controller.accept(modified, 9, &[0; 6], 4),
            Err(PdoConfigError::ActionMismatch)
        );
        assert_eq!(controller.operation_index(), 0);

        let mut plan = PdoConfigPlan::<1>::new();
        plan.push(PdoSdoWrite::new(0x6040, 0, &[0x06, 0]).unwrap())
            .unwrap();
        controller.start(plan, 0x1000, 9, 0, 10_000, 100).unwrap();
        let download = controller.next_action(1).unwrap().unwrap();
        controller
            .accept(download, 9, &download_response(download), 2)
            .unwrap();
        let verify = controller.next_action(3).unwrap().unwrap();
        assert_eq!(
            controller.accept(verify, 10, &[0; 6], 4),
            Err(PdoConfigError::GenerationMismatch)
        );
        assert_eq!(controller.operation_index(), 0);

        let mut plan = PdoConfigPlan::<1>::new();
        plan.push(PdoSdoWrite::new(0x6040, 0, &[0x06, 0]).unwrap())
            .unwrap();
        controller.start(plan, 0x1000, 9, 0, 10_000, 100).unwrap();
        let download = controller.next_action(1).unwrap().unwrap();
        controller
            .accept(download, 9, &download_response(download), 2)
            .unwrap();
        let verify = controller.next_action(3).unwrap().unwrap();
        assert_eq!(
            controller.accept(verify, 9, &[], 4),
            Err(PdoConfigError::PayloadLengthMismatch)
        );
        assert_eq!(controller.operation_index(), 0);

        let mut plan = PdoConfigPlan::<1>::new();
        plan.push(PdoSdoWrite::new(0x6040, 0, &[0x06, 0]).unwrap())
            .unwrap();
        controller.start(plan, 0x1000, 9, 0, 10_000, 100).unwrap();
        let download = controller.next_action(1).unwrap().unwrap();
        controller
            .accept(download, 9, &download_response(download), 2)
            .unwrap();
        let verify = controller.next_action(3).unwrap().unwrap();
        assert_eq!(
            controller.timeout(verify, verify.deadline_ns),
            Err(PdoConfigError::Timeout)
        );
        assert_eq!(controller.operation_index(), 0);
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

    #[test]
    fn controller_latches_matching_mailbox_failure_without_advancing() {
        let mut plan = PdoConfigPlan::<1>::new();
        plan.push(PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap())
            .unwrap();
        let mut controller = PdoConfigController::new();
        controller.start(plan, 0x1000, 7, 0, 100, 20).unwrap();
        let action = controller.next_action(1).unwrap().unwrap();

        assert_eq!(
            controller.mailbox_failed(action, MailboxError::Timeout),
            Err(PdoConfigError::Mailbox(MailboxError::Timeout))
        );
        assert_eq!(controller.phase(), PdoConfigPhase::Faulted);
        assert_eq!(controller.operation_index(), 0);
        assert_eq!(controller.pending(), None);
        assert_eq!(
            controller.last_error(),
            Some(PdoConfigError::Mailbox(MailboxError::Timeout))
        );

        let mut restart_plan = PdoConfigPlan::<1>::new();
        restart_plan
            .push(PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap())
            .unwrap();
        controller
            .start(restart_plan, 0x1000, 8, 2, 100, 20)
            .unwrap();
        assert_eq!(controller.phase(), PdoConfigPhase::Sending);
        assert_eq!(controller.last_error(), None);
    }

    #[test]
    fn controller_rejects_mailbox_failure_for_modified_action() {
        let mut plan = PdoConfigPlan::<1>::new();
        plan.push(PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap())
            .unwrap();
        let mut controller = PdoConfigController::new();
        controller.start(plan, 0x1000, 7, 0, 100, 20).unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        let mut modified = action;
        modified.token = modified.token.wrapping_add(1);

        assert_eq!(
            controller.mailbox_failed(modified, MailboxError::Timeout),
            Err(PdoConfigError::ActionMismatch)
        );
        assert_eq!(controller.phase(), PdoConfigPhase::Faulted);
        assert_eq!(controller.operation_index(), 0);
        assert_eq!(controller.pending(), None);
    }

    fn one_write_plan(index: u16, value: u8) -> PdoConfigPlan<1> {
        let mut plan = PdoConfigPlan::new();
        plan.push(PdoSdoWrite::new(index, 0, &[value]).unwrap())
            .unwrap();
        plan
    }

    fn complete_batch_job<const JOBS: usize>(
        batch: &mut PdoConfigBatch<JOBS, 1>,
        generation: u16,
        value: u8,
        now_ns: u64,
    ) {
        let download = batch.controller_mut().next_action(now_ns).unwrap().unwrap();
        assert_eq!(download.generation, generation);
        batch
            .controller_mut()
            .accept(
                download,
                generation,
                &download_response(download),
                now_ns + 1,
            )
            .unwrap();
        let verify = batch
            .controller_mut()
            .next_action(now_ns + 2)
            .unwrap()
            .unwrap();
        assert_eq!(verify.generation, generation);
        assert_eq!(
            batch.controller_mut().accept(
                verify,
                generation,
                &expedited_upload_response(verify, &[value]),
                now_ns + 3,
            ),
            Ok(PdoConfigProgress::Complete)
        );
    }

    #[test]
    fn batch_plan_rejects_duplicate_stations_and_capacity_transactionally() {
        let config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        let mut plan = PdoConfigBatchPlan::<1, 1>::new();
        assert_eq!(plan.validate(), Err(PdoConfigBatchPlanError::Empty));
        plan.push(PdoConfigJob::new(0x1001, one_write_plan(0x1C12, 0), config))
            .unwrap();
        assert_eq!(
            plan.push(PdoConfigJob::new(0x1001, one_write_plan(0x1C13, 0), config,)),
            Err(PdoConfigBatchPlanError::CapacityExceeded)
        );
        assert_eq!(plan.len(), 1);

        let mut duplicate = PdoConfigBatchPlan::<2, 1>::new();
        duplicate
            .push(PdoConfigJob::new(0x1001, PdoConfigPlan::new(), config))
            .unwrap();
        assert_eq!(
            duplicate.push(PdoConfigJob::new(0x1001, PdoConfigPlan::new(), config)),
            Err(PdoConfigBatchPlanError::DuplicateStationAddress {
                station_address: 0x1001,
            })
        );
        assert_eq!(duplicate.len(), 1);
    }

    #[test]
    fn batch_advances_jobs_in_order_with_distinct_generations() {
        let first_config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        let second_config = MailboxConfig::new(0x1200, 32, 0x1300, 32);
        let mut plan = PdoConfigBatchPlan::<2, 1>::new();
        plan.push(PdoConfigJob::new(
            0x1001,
            one_write_plan(0x1C12, 1),
            first_config,
        ))
        .unwrap();
        plan.push(PdoConfigJob::new(
            0x1002,
            one_write_plan(0x1C13, 2),
            second_config,
        ))
        .unwrap();
        let mut batch = PdoConfigBatch::new();
        batch.start(plan, 41, 0, 1_000, 100).unwrap();

        assert_eq!(
            batch.status(),
            PdoConfigBatchStatus {
                phase: PdoConfigBatchPhase::Configuring,
                current_index: 0,
                job_count: 2,
                station_address: Some(0x1001),
                generation: Some(41),
            }
        );
        assert_eq!(batch.current_mailbox_config(), Some(first_config));
        complete_batch_job(&mut batch, 41, 1, 1);
        assert_eq!(batch.controller().phase(), PdoConfigPhase::Complete);

        assert_eq!(
            batch.advance(10).unwrap(),
            PdoConfigBatchStatus {
                phase: PdoConfigBatchPhase::Configuring,
                current_index: 1,
                job_count: 2,
                station_address: Some(0x1002),
                generation: Some(42),
            }
        );
        assert_eq!(batch.current_mailbox_config(), Some(second_config));
        complete_batch_job(&mut batch, 42, 2, 11);
        assert_eq!(
            batch.advance(20).unwrap().phase,
            PdoConfigBatchPhase::Complete
        );
        assert_eq!(batch.current_index(), 2);
        assert_eq!(batch.current_mailbox_config(), None);
    }

    #[test]
    fn batch_skips_empty_jobs_and_rejects_generation_overflow() {
        let config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        let mut plan = PdoConfigBatchPlan::<2, 0>::new();
        plan.push(PdoConfigJob::new(0x1001, PdoConfigPlan::new(), config))
            .unwrap();
        plan.push(PdoConfigJob::new(0x1002, PdoConfigPlan::new(), config))
            .unwrap();
        let mut batch = PdoConfigBatch::new();
        batch.start(plan, 7, 0, 100, 10).unwrap();
        assert_eq!(batch.phase(), PdoConfigBatchPhase::Complete);
        assert_eq!(batch.current_index(), 2);
        assert_eq!(batch.controller().pending(), None);

        let mut overflow_plan = PdoConfigBatchPlan::<2, 0>::new();
        overflow_plan
            .push(PdoConfigJob::new(0x1001, PdoConfigPlan::new(), config))
            .unwrap();
        overflow_plan
            .push(PdoConfigJob::new(0x1002, PdoConfigPlan::new(), config))
            .unwrap();
        assert_eq!(
            batch.start(overflow_plan, u16::MAX, 0, 100, 10),
            Err(PdoConfigBatchError::GenerationOutOfBounds)
        );
        assert_eq!(batch.phase(), PdoConfigBatchPhase::Complete);
    }

    #[test]
    fn batch_fault_retains_the_exact_job_until_explicit_restart() {
        let config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        let mut plan = PdoConfigBatchPlan::<2, 1>::new();
        plan.push(PdoConfigJob::new(0x1001, one_write_plan(0x1C12, 1), config))
            .unwrap();
        plan.push(PdoConfigJob::new(0x1002, one_write_plan(0x1C13, 2), config))
            .unwrap();
        let mut batch = PdoConfigBatch::new();
        batch.start(plan, 20, 0, 10, 5).unwrap();
        assert_eq!(
            batch.controller_mut().next_action(10),
            Err(PdoConfigError::Timeout)
        );
        assert_eq!(batch.phase(), PdoConfigBatchPhase::Faulted);
        assert_eq!(batch.current_index(), 0);
        assert_eq!(
            batch.advance(11),
            Err(PdoConfigBatchError::Configuration(PdoConfigError::Timeout))
        );
        assert_eq!(batch.current_index(), 0);

        let mut restart = PdoConfigBatchPlan::<2, 1>::new();
        restart
            .push(PdoConfigJob::new(0x1002, one_write_plan(0x1C13, 3), config))
            .unwrap();
        batch.start(restart, 30, 12, 100, 10).unwrap();
        assert_eq!(batch.phase(), PdoConfigBatchPhase::Configuring);
        assert_eq!(batch.current_index(), 0);
        assert_eq!(batch.status().station_address, Some(0x1002));
        assert_eq!(batch.status().generation, Some(30));
        assert_eq!(batch.last_error(), None);
    }
}
