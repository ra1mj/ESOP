//! Fixed-capacity SyncManager/FMMU configuration transactions.
//!
//! Configuration is deliberately driven by the caller. Each table entry is
//! written once and read back once before the controller advances, so a
//! partially applied mapping cannot be reported as ready.

use crate::control::{
    ControlError, ControlRequestPool, MAX_CONTROL_PAYLOAD, RegisterOperation, RequestHandle,
    RequestState,
};
use crate::fmmu_discovery::FmmuRegisterBank;
use crate::mapping::{
    ESC_FMMU_BASE, ESC_FMMU_STRIDE, FMMU_IMAGE_LEN, FmmuConfig, MAX_ESC_FMMUS, MappingError,
    MappingTable, SYNC_MANAGER_IMAGE_LEN, SyncManagerConfig,
};
use crate::op_only::{MAX_ESC_SYNC_MANAGERS, OpOnlySyncManagerProfile};
use crate::registers::fixed_address;
use crate::sync_manager_discovery::SyncManagerRegisterBank;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MappingConfigPhase {
    Idle,
    ClearingSyncManager,
    VerifyingSyncManagerClear,
    ClearingFmmu,
    VerifyingFmmuClear,
    WritingSyncManager,
    VerifyingSyncManager,
    WritingFmmu,
    VerifyingFmmu,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MappingConfigItem {
    SyncManagerReset(u8),
    FmmuReset(u8),
    SyncManager(u8),
    Fmmu(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MappingConfigAction {
    pub token: u8,
    pub datagram_index: u8,
    pub generation: u16,
    pub station_address: u16,
    pub item: MappingConfigItem,
    pub operation: RegisterOperation,
    pub address: u32,
    pub read_len: u16,
    pub write_payload: [u8; FMMU_IMAGE_LEN],
    pub write_len: u8,
    pub deadline_ns: u64,
    pub expected_wkc: u16,
}

impl MappingConfigAction {
    pub fn payload(&self) -> &[u8] {
        &self.write_payload[..self.write_len as usize]
    }

    pub const fn datagram_len(&self) -> usize {
        let read_len = self.read_len as usize;
        let write_len = self.write_len as usize;
        if read_len > write_len {
            read_len
        } else {
            write_len
        }
    }

    pub const fn response_len(&self) -> usize {
        self.read_len as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MappingConfigProgress {
    Advanced,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MappingConfigError {
    Busy,
    NotStarted,
    NoPendingAction,
    ActionMismatch,
    TokenMismatch,
    GenerationMismatch,
    PayloadLengthMismatch,
    UnexpectedWorkingCounter,
    Timeout,
    ReadbackMismatch,
    SyncManagerBankCapacityExceeded,
    RegisterBankPositionMismatch {
        fmmu: u16,
        sync_manager: u16,
    },
    SyncManagerBankStationMismatch {
        expected: u16,
        observed: u16,
    },
    ConfiguredSyncManagerCountExceedsDiscovered {
        configured: usize,
        discovered: usize,
    },
    SyncManagerIndexOutsideDiscoveredBank {
        index: u8,
        discovered: usize,
    },
    FmmuBankCapacityExceeded,
    FmmuBankStationMismatch {
        expected: u16,
        observed: u16,
    },
    ConfiguredFmmuCountExceedsDiscovered {
        configured: usize,
        discovered: usize,
    },
    FmmuIndexOutsideDiscoveredBank {
        index: u8,
        discovered: usize,
    },
    Mapping(MappingError),
    Control(ControlError),
}

pub struct MappingConfigController<const SMS: usize, const FMMUS: usize> {
    phase: MappingConfigPhase,
    generation: u16,
    station_address: u16,
    configuration_deadline_ns: u64,
    request_timeout_ns: u64,
    sync_managers: [SyncManagerConfig; SMS],
    sync_manager_count: usize,
    fmmus: [FmmuConfig; FMMUS],
    fmmu_count: usize,
    discovered_sync_manager_count: usize,
    discovered_fmmu_count: usize,
    op_only_outputs: OpOnlySyncManagerProfile,
    item_index: usize,
    pending: Option<MappingConfigAction>,
    next_token: u8,
    next_datagram_index: u8,
    last_error: Option<MappingConfigError>,
}

impl<const SMS: usize, const FMMUS: usize> MappingConfigController<SMS, FMMUS> {
    pub const fn new() -> Self {
        Self {
            phase: MappingConfigPhase::Idle,
            generation: 0,
            station_address: 0,
            configuration_deadline_ns: 0,
            request_timeout_ns: 0,
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
            discovered_sync_manager_count: 0,
            discovered_fmmu_count: 0,
            op_only_outputs: OpOnlySyncManagerProfile::EMPTY,
            item_index: 0,
            pending: None,
            next_token: 1,
            next_datagram_index: 1,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> MappingConfigPhase {
        self.phase
    }

    pub const fn pending(&self) -> Option<MappingConfigAction> {
        self.pending
    }

    pub const fn last_error(&self) -> Option<MappingConfigError> {
        self.last_error
    }

    pub const fn sync_manager_count(&self) -> usize {
        self.sync_manager_count
    }

    pub const fn fmmu_count(&self) -> usize {
        self.fmmu_count
    }

    pub fn start(
        &mut self,
        station_address: u16,
        generation: u16,
        now_ns: u64,
        timeout_ns: u64,
        request_timeout_ns: u64,
        table: &MappingTable<SMS, FMMUS>,
    ) -> Result<(), MappingConfigError> {
        self.start_inner(
            station_address,
            generation,
            now_ns,
            timeout_ns,
            request_timeout_ns,
            None,
            None,
            table,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_with_verified_fmmus(
        &mut self,
        station_address: u16,
        generation: u16,
        now_ns: u64,
        timeout_ns: u64,
        request_timeout_ns: u64,
        bank: FmmuRegisterBank,
        table: &MappingTable<SMS, FMMUS>,
    ) -> Result<(), MappingConfigError> {
        self.start_inner(
            station_address,
            generation,
            now_ns,
            timeout_ns,
            request_timeout_ns,
            Some(bank),
            None,
            table,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_with_verified_registers(
        &mut self,
        station_address: u16,
        generation: u16,
        now_ns: u64,
        timeout_ns: u64,
        request_timeout_ns: u64,
        sync_manager_bank: SyncManagerRegisterBank,
        fmmu_bank: FmmuRegisterBank,
        table: &MappingTable<SMS, FMMUS>,
    ) -> Result<(), MappingConfigError> {
        self.start_inner(
            station_address,
            generation,
            now_ns,
            timeout_ns,
            request_timeout_ns,
            Some(fmmu_bank),
            Some(sync_manager_bank),
            table,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn start_inner(
        &mut self,
        station_address: u16,
        generation: u16,
        now_ns: u64,
        timeout_ns: u64,
        request_timeout_ns: u64,
        fmmu_bank: Option<FmmuRegisterBank>,
        sync_manager_bank: Option<SyncManagerRegisterBank>,
        table: &MappingTable<SMS, FMMUS>,
    ) -> Result<(), MappingConfigError> {
        if !matches!(
            self.phase,
            MappingConfigPhase::Idle | MappingConfigPhase::Complete | MappingConfigPhase::Faulted
        ) {
            return Err(MappingConfigError::Busy);
        }

        let sync_manager_count = table.sync_manager_count();
        let fmmu_count = table.fmmu_count();
        let discovered_sync_manager_count = if let Some(bank) = sync_manager_bank {
            let discovered = bank.descriptor_count();
            if discovered > MAX_ESC_SYNC_MANAGERS {
                return Err(MappingConfigError::SyncManagerBankCapacityExceeded);
            }
            if bank.station_address() != station_address {
                return Err(MappingConfigError::SyncManagerBankStationMismatch {
                    expected: station_address,
                    observed: bank.station_address(),
                });
            }
            if sync_manager_count > discovered {
                return Err(
                    MappingConfigError::ConfiguredSyncManagerCountExceedsDiscovered {
                        configured: sync_manager_count,
                        discovered,
                    },
                );
            }
            for config in table.sync_managers() {
                if usize::from(config.index) >= discovered {
                    return Err(MappingConfigError::SyncManagerIndexOutsideDiscoveredBank {
                        index: config.index,
                        discovered,
                    });
                }
            }
            discovered
        } else {
            0
        };
        let discovered_fmmu_count = if let Some(bank) = fmmu_bank {
            let discovered = bank.descriptor_count();
            if discovered > MAX_ESC_FMMUS {
                return Err(MappingConfigError::FmmuBankCapacityExceeded);
            }
            if bank.station_address() != station_address {
                return Err(MappingConfigError::FmmuBankStationMismatch {
                    expected: station_address,
                    observed: bank.station_address(),
                });
            }
            if fmmu_count > discovered {
                return Err(MappingConfigError::ConfiguredFmmuCountExceedsDiscovered {
                    configured: fmmu_count,
                    discovered,
                });
            }
            for config in table.fmmus() {
                if usize::from(config.index) >= discovered {
                    return Err(MappingConfigError::FmmuIndexOutsideDiscoveredBank {
                        index: config.index,
                        discovered,
                    });
                }
            }
            discovered
        } else {
            0
        };
        if let (Some(fmmu_bank), Some(sync_manager_bank)) = (fmmu_bank, sync_manager_bank)
            && fmmu_bank.position() != sync_manager_bank.position()
        {
            return Err(MappingConfigError::RegisterBankPositionMismatch {
                fmmu: fmmu_bank.position(),
                sync_manager: sync_manager_bank.position(),
            });
        }

        self.sync_manager_count = sync_manager_count;
        self.fmmu_count = fmmu_count;
        self.discovered_sync_manager_count = discovered_sync_manager_count;
        self.discovered_fmmu_count = discovered_fmmu_count;
        self.sync_managers[..self.sync_manager_count].copy_from_slice(table.sync_managers());
        self.fmmus[..self.fmmu_count].copy_from_slice(table.fmmus());
        self.op_only_outputs = table.op_only_outputs();
        self.station_address = station_address;
        self.generation = generation;
        self.configuration_deadline_ns = now_ns.saturating_add(timeout_ns);
        self.request_timeout_ns = request_timeout_ns;
        self.item_index = 0;
        self.pending = None;
        self.next_token = 1;
        self.next_datagram_index = 1;
        self.last_error = None;
        self.phase = self.initial_reset_phase();
        Ok(())
    }

    pub fn next_action(
        &mut self,
        now_ns: u64,
    ) -> Result<Option<MappingConfigAction>, MappingConfigError> {
        if self.phase == MappingConfigPhase::Idle {
            return Err(MappingConfigError::NotStarted);
        }
        if matches!(
            self.phase,
            MappingConfigPhase::Complete | MappingConfigPhase::Faulted
        ) {
            return Ok(None);
        }
        if let Some(action) = self.pending {
            return Ok(Some(action));
        }
        if now_ns >= self.configuration_deadline_ns {
            return self.fail(MappingConfigError::Timeout);
        }

        let (item, operation, address, read_len, payload, write_len) = match self.phase {
            MappingConfigPhase::ClearingSyncManager => {
                if self.item_index >= self.discovered_sync_manager_count {
                    self.item_index = 0;
                    self.phase = self.phase_after_sync_manager_clear();
                    return self.next_action(now_ns);
                }
                let index = self.item_index as u8;
                (
                    MappingConfigItem::SyncManagerReset(index),
                    RegisterOperation::Write,
                    fixed_address(
                        self.station_address,
                        crate::ESC_SYNC_MANAGER_BASE
                            + u16::from(index) * crate::ESC_SYNC_MANAGER_STRIDE,
                    ),
                    0,
                    [0; FMMU_IMAGE_LEN],
                    SYNC_MANAGER_IMAGE_LEN,
                )
            }
            MappingConfigPhase::VerifyingSyncManagerClear => {
                let index = self.item_index as u8;
                (
                    MappingConfigItem::SyncManagerReset(index),
                    RegisterOperation::Read,
                    fixed_address(
                        self.station_address,
                        crate::ESC_SYNC_MANAGER_BASE
                            + u16::from(index) * crate::ESC_SYNC_MANAGER_STRIDE,
                    ),
                    SYNC_MANAGER_IMAGE_LEN,
                    [0; FMMU_IMAGE_LEN],
                    0,
                )
            }
            MappingConfigPhase::ClearingFmmu => {
                if self.item_index >= self.discovered_fmmu_count {
                    self.item_index = 0;
                    self.phase = self.initial_mapping_phase();
                    return self.next_action(now_ns);
                }
                let index = self.item_index as u8;
                (
                    MappingConfigItem::FmmuReset(index),
                    RegisterOperation::Write,
                    fixed_address(
                        self.station_address,
                        ESC_FMMU_BASE + u16::from(index) * ESC_FMMU_STRIDE,
                    ),
                    0,
                    [0; FMMU_IMAGE_LEN],
                    FMMU_IMAGE_LEN,
                )
            }
            MappingConfigPhase::VerifyingFmmuClear => {
                let index = self.item_index as u8;
                (
                    MappingConfigItem::FmmuReset(index),
                    RegisterOperation::Read,
                    fixed_address(
                        self.station_address,
                        ESC_FMMU_BASE + u16::from(index) * ESC_FMMU_STRIDE,
                    ),
                    FMMU_IMAGE_LEN,
                    [0; FMMU_IMAGE_LEN],
                    0,
                )
            }
            MappingConfigPhase::WritingSyncManager => {
                if self.item_index >= self.sync_manager_count {
                    self.phase = if self.fmmu_count != 0 {
                        MappingConfigPhase::WritingFmmu
                    } else {
                        MappingConfigPhase::Complete
                    };
                    self.item_index = 0;
                    return self.next_action(now_ns);
                }
                let config = self.sync_managers[self.item_index];
                let mut encoded = [0; FMMU_IMAGE_LEN];
                self.encode_sync_manager(config, &mut encoded[..SYNC_MANAGER_IMAGE_LEN])?;
                (
                    MappingConfigItem::SyncManager(config.index),
                    RegisterOperation::Write,
                    fixed_address(self.station_address, config.register_address()),
                    0,
                    encoded,
                    SYNC_MANAGER_IMAGE_LEN,
                )
            }
            MappingConfigPhase::VerifyingSyncManager => {
                let config = self.sync_managers[self.item_index];
                (
                    MappingConfigItem::SyncManager(config.index),
                    RegisterOperation::Read,
                    fixed_address(self.station_address, config.register_address()),
                    SYNC_MANAGER_IMAGE_LEN,
                    [0; FMMU_IMAGE_LEN],
                    0,
                )
            }
            MappingConfigPhase::WritingFmmu => {
                if self.item_index >= self.fmmu_count {
                    self.phase = MappingConfigPhase::Complete;
                    return Ok(None);
                }
                let config = self.fmmus[self.item_index];
                let mut encoded = [0; FMMU_IMAGE_LEN];
                config
                    .encode(&mut encoded)
                    .map_err(MappingConfigError::Mapping)?;
                (
                    MappingConfigItem::Fmmu(config.index),
                    RegisterOperation::Write,
                    fixed_address(self.station_address, config.register_address()),
                    0,
                    encoded,
                    FMMU_IMAGE_LEN,
                )
            }
            MappingConfigPhase::VerifyingFmmu => {
                let config = self.fmmus[self.item_index];
                (
                    MappingConfigItem::Fmmu(config.index),
                    RegisterOperation::Read,
                    fixed_address(self.station_address, config.register_address()),
                    FMMU_IMAGE_LEN,
                    [0; FMMU_IMAGE_LEN],
                    0,
                )
            }
            MappingConfigPhase::Idle
            | MappingConfigPhase::Complete
            | MappingConfigPhase::Faulted => return Ok(None),
        };
        let deadline_ns = now_ns
            .saturating_add(self.request_timeout_ns)
            .min(self.configuration_deadline_ns);
        let action = MappingConfigAction {
            token: self.next_token,
            datagram_index: self.next_datagram_index,
            generation: self.generation,
            station_address: self.station_address,
            item,
            operation,
            address,
            read_len: read_len as u16,
            write_payload: payload,
            write_len: write_len as u8,
            deadline_ns,
            expected_wkc: 1,
        };
        self.next_token = self.next_token.wrapping_add(1).max(1);
        self.next_datagram_index = self.next_datagram_index.wrapping_add(1).max(1);
        self.pending = Some(action);
        Ok(Some(action))
    }

    pub fn enqueue_pending<const REQUESTS: usize>(
        &self,
        pool: &mut ControlRequestPool<REQUESTS>,
    ) -> Result<RequestHandle, ControlError> {
        let action = self.pending.ok_or(ControlError::InvalidState)?;
        pool.acquire_with_response_len(
            action.datagram_index,
            action.generation,
            action.address,
            action.operation,
            action.payload(),
            action.datagram_len(),
            action.deadline_ns,
        )
    }

    pub fn accept(
        &mut self,
        action: MappingConfigAction,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<MappingConfigProgress, MappingConfigError> {
        if self.pending != Some(action) {
            return self.fail(MappingConfigError::ActionMismatch);
        }
        if action.generation != generation {
            return self.fail(MappingConfigError::GenerationMismatch);
        }
        if now_ns > action.deadline_ns {
            return self.fail(MappingConfigError::Timeout);
        }
        if working_counter != action.expected_wkc {
            return self.fail(MappingConfigError::UnexpectedWorkingCounter);
        }
        if payload.len() != action.response_len() {
            return self.fail(MappingConfigError::PayloadLengthMismatch);
        }

        let progress = match self.phase {
            MappingConfigPhase::ClearingSyncManager => {
                self.phase = MappingConfigPhase::VerifyingSyncManagerClear;
                MappingConfigProgress::Advanced
            }
            MappingConfigPhase::VerifyingSyncManagerClear => {
                if payload != [0; SYNC_MANAGER_IMAGE_LEN] {
                    return self.fail(MappingConfigError::ReadbackMismatch);
                }
                self.item_index += 1;
                if self.item_index < self.discovered_sync_manager_count {
                    self.phase = MappingConfigPhase::ClearingSyncManager;
                    MappingConfigProgress::Advanced
                } else {
                    self.item_index = 0;
                    self.phase = self.phase_after_sync_manager_clear();
                    if self.phase == MappingConfigPhase::Complete {
                        MappingConfigProgress::Complete
                    } else {
                        MappingConfigProgress::Advanced
                    }
                }
            }
            MappingConfigPhase::ClearingFmmu => {
                self.phase = MappingConfigPhase::VerifyingFmmuClear;
                MappingConfigProgress::Advanced
            }
            MappingConfigPhase::VerifyingFmmuClear => {
                if payload != [0; FMMU_IMAGE_LEN] {
                    return self.fail(MappingConfigError::ReadbackMismatch);
                }
                self.item_index += 1;
                if self.item_index < self.discovered_fmmu_count {
                    self.phase = MappingConfigPhase::ClearingFmmu;
                    MappingConfigProgress::Advanced
                } else {
                    self.item_index = 0;
                    self.phase = self.initial_mapping_phase();
                    if self.phase == MappingConfigPhase::Complete {
                        MappingConfigProgress::Complete
                    } else {
                        MappingConfigProgress::Advanced
                    }
                }
            }
            MappingConfigPhase::WritingSyncManager => {
                self.phase = MappingConfigPhase::VerifyingSyncManager;
                MappingConfigProgress::Advanced
            }
            MappingConfigPhase::VerifyingSyncManager => {
                let mut expected = [0; FMMU_IMAGE_LEN];
                self.encode_sync_manager(
                    self.sync_managers[self.item_index],
                    &mut expected[..SYNC_MANAGER_IMAGE_LEN],
                )?;
                if payload != &expected[..SYNC_MANAGER_IMAGE_LEN] {
                    return self.fail(MappingConfigError::ReadbackMismatch);
                }
                self.item_index += 1;
                self.phase = if self.item_index < self.sync_manager_count {
                    MappingConfigPhase::WritingSyncManager
                } else if self.fmmu_count != 0 {
                    self.item_index = 0;
                    MappingConfigPhase::WritingFmmu
                } else {
                    MappingConfigPhase::Complete
                };
                if self.phase == MappingConfigPhase::Complete {
                    MappingConfigProgress::Complete
                } else {
                    MappingConfigProgress::Advanced
                }
            }
            MappingConfigPhase::WritingFmmu => {
                self.phase = MappingConfigPhase::VerifyingFmmu;
                MappingConfigProgress::Advanced
            }
            MappingConfigPhase::VerifyingFmmu => {
                let mut expected = [0; FMMU_IMAGE_LEN];
                self.fmmus[self.item_index]
                    .encode(&mut expected)
                    .map_err(MappingConfigError::Mapping)?;
                if payload != expected {
                    return self.fail(MappingConfigError::ReadbackMismatch);
                }
                self.item_index += 1;
                if self.item_index < self.fmmu_count {
                    self.phase = MappingConfigPhase::WritingFmmu;
                    MappingConfigProgress::Advanced
                } else {
                    self.phase = MappingConfigPhase::Complete;
                    MappingConfigProgress::Complete
                }
            }
            MappingConfigPhase::Idle
            | MappingConfigPhase::Complete
            | MappingConfigPhase::Faulted => {
                return self.fail(MappingConfigError::NoPendingAction);
            }
        };
        self.pending = None;
        Ok(progress)
    }

    pub fn accept_completed<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
        handle: RequestHandle,
        now_ns: u64,
    ) -> Result<MappingConfigProgress, MappingConfigError> {
        let action = match self.pending {
            Some(action) => action,
            None => return self.fail(MappingConfigError::NoPendingAction),
        };
        let (generation, actual_wkc, response) = match pool.get(handle) {
            Some(request) if request.state == RequestState::Complete => {
                if !request.matches_action(
                    action.datagram_index,
                    action.generation,
                    action.address,
                    action.operation,
                    action.payload(),
                    action.datagram_len(),
                    action.deadline_ns,
                ) || request.length < action.response_len()
                {
                    let _ = pool.release(handle);
                    return self.fail(MappingConfigError::ActionMismatch);
                }
                let mut response = [0; MAX_CONTROL_PAYLOAD];
                response[..request.length].copy_from_slice(request.payload());
                (request.generation, request.actual_wkc, response)
            }
            Some(request) if request.state == RequestState::Failed => {
                if !request.matches_action(
                    action.datagram_index,
                    action.generation,
                    action.address,
                    action.operation,
                    action.payload(),
                    action.datagram_len(),
                    action.deadline_ns,
                ) {
                    let _ = pool.release(handle);
                    return self.fail(MappingConfigError::ActionMismatch);
                }
                let error = request.last_error().unwrap_or(ControlError::InvalidState);
                if let Err(release_error) = pool.release(handle) {
                    return self.fail(MappingConfigError::Control(release_error));
                }
                if error == ControlError::Timeout {
                    return self.timeout(action, now_ns);
                }
                return self.fail(MappingConfigError::Control(error));
            }
            Some(_) => return Err(MappingConfigError::Control(ControlError::InvalidState)),
            None => return self.fail(MappingConfigError::Control(ControlError::InvalidHandle)),
        };
        let progress = self.accept(
            action,
            generation,
            &response[..action.response_len()],
            actual_wkc,
            now_ns,
        );
        let release = pool.release(handle);
        match (progress, release) {
            (Ok(progress), Ok(())) => Ok(progress),
            (Ok(_), Err(error)) => self.fail(MappingConfigError::Control(error)),
            (Err(error), _) => Err(error),
        }
    }

    pub fn timeout(
        &mut self,
        action: MappingConfigAction,
        now_ns: u64,
    ) -> Result<MappingConfigProgress, MappingConfigError> {
        if self.pending != Some(action) {
            return self.fail(MappingConfigError::ActionMismatch);
        }
        if now_ns < action.deadline_ns {
            return Err(MappingConfigError::Timeout);
        }
        self.fail(MappingConfigError::Timeout)
    }

    fn encode_sync_manager(
        &self,
        config: SyncManagerConfig,
        dst: &mut [u8],
    ) -> Result<(), MappingConfigError> {
        config.encode(dst).map_err(MappingConfigError::Mapping)?;
        if let Some(activation) = self.op_only_outputs.activation_for(config.index, false) {
            dst[6] = activation;
        }
        Ok(())
    }

    const fn initial_mapping_phase(&self) -> MappingConfigPhase {
        if self.sync_manager_count != 0 {
            MappingConfigPhase::WritingSyncManager
        } else if self.fmmu_count != 0 {
            MappingConfigPhase::WritingFmmu
        } else {
            MappingConfigPhase::Complete
        }
    }

    const fn initial_reset_phase(&self) -> MappingConfigPhase {
        if self.discovered_sync_manager_count != 0 {
            MappingConfigPhase::ClearingSyncManager
        } else {
            self.phase_after_sync_manager_clear()
        }
    }

    const fn phase_after_sync_manager_clear(&self) -> MappingConfigPhase {
        if self.discovered_fmmu_count != 0 {
            MappingConfigPhase::ClearingFmmu
        } else {
            self.initial_mapping_phase()
        }
    }

    fn fail<T>(&mut self, error: MappingConfigError) -> Result<T, MappingConfigError> {
        self.last_error = Some(error);
        self.pending = None;
        self.phase = MappingConfigPhase::Faulted;
        Err(error)
    }
}

impl<const SMS: usize, const FMMUS: usize> Default for MappingConfigController<SMS, FMMUS> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fmmu_discovery::FmmuRegisterDescriptor;
    use crate::mapping::{ESC_FMMU_BASE, ESC_SYNC_MANAGER_BASE};
    use crate::sync_manager_discovery::SyncManagerRegisterDescriptor;

    fn verified_bank(
        position: u16,
        station_address: u16,
        descriptor_count: u8,
    ) -> FmmuRegisterBank {
        FmmuRegisterBank::from_parts(
            position,
            station_address,
            descriptor_count,
            [FmmuRegisterDescriptor::RESET; MAX_ESC_FMMUS],
        )
    }

    fn verified_sync_manager_bank(
        position: u16,
        station_address: u16,
        descriptor_count: u8,
    ) -> SyncManagerRegisterBank {
        SyncManagerRegisterBank::from_parts(
            position,
            station_address,
            descriptor_count,
            [SyncManagerRegisterDescriptor::RESET; MAX_ESC_SYNC_MANAGERS],
        )
    }

    fn mapping_table() -> MappingTable<1, 1> {
        let mut table = MappingTable::new();
        table
            .add_sync_manager(SyncManagerConfig {
                index: 2,
                physical_start: 0x1000,
                length: 8,
                control: 0x26,
                status: 0,
                enable: true,
            })
            .unwrap();
        table
            .add_fmmu(FmmuConfig {
                index: 0,
                logical_start: 0x2000,
                length: 8,
                logical_start_bit: 0,
                logical_end_bit: 7,
                physical_start: 0x1000,
                physical_start_bit: 0,
                fmmu_type: 2,
                enable: true,
            })
            .unwrap();
        table
    }

    #[test]
    fn writes_and_reads_back_every_mapping_entry() {
        let table = mapping_table();
        let mut controller = MappingConfigController::<1, 1>::new();
        controller.start(0x1000, 5, 0, 1_000, 100, &table).unwrap();

        let write_sm = controller.next_action(1).unwrap().unwrap();
        assert_eq!(write_sm.item, MappingConfigItem::SyncManager(2));
        assert_eq!(write_sm.address, 0x1000_0810);
        assert_eq!(write_sm.operation, RegisterOperation::Write);
        assert_eq!(write_sm.datagram_len(), SYNC_MANAGER_IMAGE_LEN);
        controller.accept(write_sm, 5, &[], 1, 2).unwrap();

        let read_sm = controller.next_action(3).unwrap().unwrap();
        let mut sm_image = [0; SYNC_MANAGER_IMAGE_LEN];
        table
            .sync_manager(2)
            .unwrap()
            .encode(&mut sm_image)
            .unwrap();
        assert_eq!(
            controller.accept(read_sm, 5, &sm_image, 1, 4),
            Ok(MappingConfigProgress::Advanced)
        );

        let write_fmmu = controller.next_action(5).unwrap().unwrap();
        assert_eq!(write_fmmu.item, MappingConfigItem::Fmmu(0));
        assert_eq!(write_fmmu.address, 0x1000_0600);
        assert_eq!(write_fmmu.operation, RegisterOperation::Write);
        controller.accept(write_fmmu, 5, &[], 1, 6).unwrap();

        let read_fmmu = controller.next_action(7).unwrap().unwrap();
        let mut fmmu_image = [0; FMMU_IMAGE_LEN];
        table.fmmu(0).unwrap().encode(&mut fmmu_image).unwrap();
        assert_eq!(
            controller.accept(read_fmmu, 5, &fmmu_image, 1, 8),
            Ok(MappingConfigProgress::Complete)
        );
        assert_eq!(controller.phase(), MappingConfigPhase::Complete);
        assert_eq!(controller.next_action(9), Ok(None));
    }

    #[test]
    fn readback_mismatch_latches_configuration_fault() {
        let table = mapping_table();
        let mut controller = MappingConfigController::<1, 1>::new();
        controller.start(0x1000, 5, 0, 1_000, 100, &table).unwrap();
        let write = controller.next_action(1).unwrap().unwrap();
        controller.accept(write, 5, &[], 1, 2).unwrap();
        let read = controller.next_action(3).unwrap().unwrap();
        assert_eq!(
            controller.accept(read, 5, &[0; SYNC_MANAGER_IMAGE_LEN], 1, 4),
            Err(MappingConfigError::ReadbackMismatch)
        );
        assert_eq!(controller.phase(), MappingConfigPhase::Faulted);
        assert_eq!(
            controller.last_error(),
            Some(MappingConfigError::ReadbackMismatch)
        );
    }

    #[test]
    fn op_only_sync_manager_is_configured_disabled_in_preop() {
        let mut table = mapping_table();
        table.mark_op_only_output(2, 0x09).unwrap();
        let mut controller = MappingConfigController::<1, 1>::new();
        controller.start(0x1000, 5, 0, 1_000, 100, &table).unwrap();

        let write = controller.next_action(1).unwrap().unwrap();
        assert_eq!(write.payload()[6], 0x08);
        controller.accept(write, 5, &[], 1, 2).unwrap();
        let read = controller.next_action(3).unwrap().unwrap();
        let mut image = [0; SYNC_MANAGER_IMAGE_LEN];
        table.sync_manager(2).unwrap().encode(&mut image).unwrap();
        image[6] = 0x08;
        assert_eq!(
            controller.accept(read, 5, &image, 1, 4),
            Ok(MappingConfigProgress::Advanced)
        );
    }

    #[test]
    fn completed_readback_uses_control_pool_and_releases_slot() {
        let table = mapping_table();
        let mut controller = MappingConfigController::<1, 1>::new();
        controller.start(0x1000, 5, 0, 1_000, 100, &table).unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        let mut pool = ControlRequestPool::<1>::new();
        let handle = controller.enqueue_pending(&mut pool).unwrap();
        assert_eq!(pool.get(handle).unwrap().length, SYNC_MANAGER_IMAGE_LEN);
        assert_eq!(
            pool.get(handle).unwrap().response_length,
            SYNC_MANAGER_IMAGE_LEN
        );
        pool.get_mut(handle).unwrap().state = RequestState::Complete;
        pool.get_mut(handle).unwrap().actual_wkc = 1;
        assert_eq!(
            controller.accept_completed(&mut pool, handle, 2),
            Ok(MappingConfigProgress::Advanced)
        );
        assert_eq!(pool.in_use(), 0);
        assert_eq!(
            action.address,
            fixed_address(0x1000, ESC_SYNC_MANAGER_BASE + 16)
        );
        assert_eq!(
            controller.next_action(3).unwrap().unwrap().address,
            fixed_address(0x1000, ESC_SYNC_MANAGER_BASE + 16)
        );
        assert_eq!(ESC_FMMU_BASE, 0x0600);
    }

    #[test]
    fn verified_bank_clears_every_discovered_slot_before_mapping() {
        let table = mapping_table();
        let mut controller = MappingConfigController::<1, 1>::new();
        controller
            .start_with_verified_fmmus(
                0x1000,
                5,
                0,
                1_000,
                100,
                verified_bank(0, 0x1000, 2),
                &table,
            )
            .unwrap();

        for index in 0..2u8 {
            let write = controller
                .next_action(1 + u64::from(index) * 4)
                .unwrap()
                .unwrap();
            assert_eq!(write.item, MappingConfigItem::FmmuReset(index));
            assert_eq!(write.operation, RegisterOperation::Write);
            assert_eq!(write.payload(), &[0; FMMU_IMAGE_LEN]);
            assert_eq!(
                write.address,
                fixed_address(0x1000, ESC_FMMU_BASE + u16::from(index) * ESC_FMMU_STRIDE)
            );
            controller
                .accept(write, 5, &[], 1, 2 + u64::from(index) * 4)
                .unwrap();

            let read = controller
                .next_action(3 + u64::from(index) * 4)
                .unwrap()
                .unwrap();
            assert_eq!(read.item, MappingConfigItem::FmmuReset(index));
            assert_eq!(read.operation, RegisterOperation::Read);
            assert_eq!(read.response_len(), FMMU_IMAGE_LEN);
            controller
                .accept(read, 5, &[0; FMMU_IMAGE_LEN], 1, 4 + u64::from(index) * 4)
                .unwrap();
        }

        let write_sm = controller.next_action(9).unwrap().unwrap();
        assert_eq!(write_sm.item, MappingConfigItem::SyncManager(2));
        controller.accept(write_sm, 5, &[], 1, 10).unwrap();
        let read_sm = controller.next_action(11).unwrap().unwrap();
        let mut sm_image = [0; SYNC_MANAGER_IMAGE_LEN];
        table
            .sync_manager(2)
            .unwrap()
            .encode(&mut sm_image)
            .unwrap();
        controller.accept(read_sm, 5, &sm_image, 1, 12).unwrap();

        let write_fmmu = controller.next_action(13).unwrap().unwrap();
        assert_eq!(write_fmmu.item, MappingConfigItem::Fmmu(0));
        controller.accept(write_fmmu, 5, &[], 1, 14).unwrap();
        let read_fmmu = controller.next_action(15).unwrap().unwrap();
        let mut image = [0; FMMU_IMAGE_LEN];
        table.fmmu(0).unwrap().encode(&mut image).unwrap();
        assert_eq!(
            controller.accept(read_fmmu, 5, &image, 1, 16),
            Ok(MappingConfigProgress::Complete)
        );
    }

    #[test]
    fn verified_bank_preflight_rejects_mismatch_without_mutation() {
        let table = mapping_table();
        let mut controller = MappingConfigController::<1, 1>::new();
        assert_eq!(
            controller.start_with_verified_fmmus(
                0x1000,
                5,
                0,
                1_000,
                100,
                verified_bank(0, 0x1001, 2),
                &table,
            ),
            Err(MappingConfigError::FmmuBankStationMismatch {
                expected: 0x1000,
                observed: 0x1001,
            })
        );
        assert_eq!(controller.phase(), MappingConfigPhase::Idle);
        assert_eq!(
            controller.next_action(1),
            Err(MappingConfigError::NotStarted)
        );

        assert_eq!(
            controller.start_with_verified_fmmus(
                0x1000,
                5,
                0,
                1_000,
                100,
                verified_bank(0, 0x1000, 0),
                &table,
            ),
            Err(MappingConfigError::ConfiguredFmmuCountExceedsDiscovered {
                configured: 1,
                discovered: 0,
            })
        );

        let mut sparse = MappingTable::<0, 1>::new();
        sparse
            .add_fmmu(FmmuConfig {
                index: 1,
                logical_start: 0x2000,
                length: 8,
                logical_start_bit: 0,
                logical_end_bit: 7,
                physical_start: 0x1000,
                physical_start_bit: 0,
                fmmu_type: 2,
                enable: true,
            })
            .unwrap();
        let mut sparse_controller = MappingConfigController::<0, 1>::new();
        assert_eq!(
            sparse_controller.start_with_verified_fmmus(
                0x1000,
                5,
                0,
                1_000,
                100,
                verified_bank(0, 0x1000, 1),
                &sparse,
            ),
            Err(MappingConfigError::FmmuIndexOutsideDiscoveredBank {
                index: 1,
                discovered: 1,
            })
        );

        assert_eq!(
            controller.start_with_verified_fmmus(
                0x1000,
                5,
                0,
                1_000,
                100,
                verified_bank(0, 0x1000, (MAX_ESC_FMMUS + 1) as u8),
                &table,
            ),
            Err(MappingConfigError::FmmuBankCapacityExceeded)
        );
        assert_eq!(controller.phase(), MappingConfigPhase::Idle);
    }

    #[test]
    fn verified_bank_clear_readback_mismatch_latches_fault() {
        let table = mapping_table();
        let mut controller = MappingConfigController::<1, 1>::new();
        controller
            .start_with_verified_fmmus(
                0x1000,
                5,
                0,
                1_000,
                100,
                verified_bank(0, 0x1000, 2),
                &table,
            )
            .unwrap();
        let write = controller.next_action(1).unwrap().unwrap();
        controller.accept(write, 5, &[], 1, 2).unwrap();
        let read = controller.next_action(3).unwrap().unwrap();
        let mut stale = [0; FMMU_IMAGE_LEN];
        stale[12] = 1;
        assert_eq!(
            controller.accept(read, 5, &stale, 1, 4),
            Err(MappingConfigError::ReadbackMismatch)
        );
        assert_eq!(controller.phase(), MappingConfigPhase::Faulted);
        assert_eq!(controller.next_action(5), Ok(None));
    }

    #[test]
    fn verified_register_banks_clear_sync_managers_before_fmmus_and_mapping() {
        let table = mapping_table();
        let mut controller = MappingConfigController::<1, 1>::new();
        controller
            .start_with_verified_registers(
                0x1000,
                5,
                0,
                10_000,
                100,
                verified_sync_manager_bank(0, 0x1000, 3),
                verified_bank(0, 0x1000, 2),
                &table,
            )
            .unwrap();

        let mut now = 1;
        for index in 0..3u8 {
            let write = controller.next_action(now).unwrap().unwrap();
            assert_eq!(write.item, MappingConfigItem::SyncManagerReset(index));
            assert_eq!(write.operation, RegisterOperation::Write);
            assert_eq!(write.payload(), &[0; SYNC_MANAGER_IMAGE_LEN]);
            assert_eq!(
                write.address,
                fixed_address(
                    0x1000,
                    ESC_SYNC_MANAGER_BASE + u16::from(index) * crate::ESC_SYNC_MANAGER_STRIDE,
                )
            );
            controller.accept(write, 5, &[], 1, now + 1).unwrap();
            now += 2;

            let read = controller.next_action(now).unwrap().unwrap();
            assert_eq!(read.item, MappingConfigItem::SyncManagerReset(index));
            assert_eq!(read.operation, RegisterOperation::Read);
            assert_eq!(read.response_len(), SYNC_MANAGER_IMAGE_LEN);
            controller
                .accept(read, 5, &[0; SYNC_MANAGER_IMAGE_LEN], 1, now + 1)
                .unwrap();
            now += 2;
        }

        for index in 0..2u8 {
            let write = controller.next_action(now).unwrap().unwrap();
            assert_eq!(write.item, MappingConfigItem::FmmuReset(index));
            controller.accept(write, 5, &[], 1, now + 1).unwrap();
            now += 2;
            let read = controller.next_action(now).unwrap().unwrap();
            assert_eq!(read.item, MappingConfigItem::FmmuReset(index));
            controller
                .accept(read, 5, &[0; FMMU_IMAGE_LEN], 1, now + 1)
                .unwrap();
            now += 2;
        }

        let write_sm = controller.next_action(now).unwrap().unwrap();
        assert_eq!(write_sm.item, MappingConfigItem::SyncManager(2));
        controller.accept(write_sm, 5, &[], 1, now + 1).unwrap();
        now += 2;
        let read_sm = controller.next_action(now).unwrap().unwrap();
        let mut sm_image = [0; SYNC_MANAGER_IMAGE_LEN];
        table
            .sync_manager(2)
            .unwrap()
            .encode(&mut sm_image)
            .unwrap();
        controller
            .accept(read_sm, 5, &sm_image, 1, now + 1)
            .unwrap();
        now += 2;

        let write_fmmu = controller.next_action(now).unwrap().unwrap();
        assert_eq!(write_fmmu.item, MappingConfigItem::Fmmu(0));
        controller.accept(write_fmmu, 5, &[], 1, now + 1).unwrap();
        now += 2;
        let read_fmmu = controller.next_action(now).unwrap().unwrap();
        let mut fmmu_image = [0; FMMU_IMAGE_LEN];
        table.fmmu(0).unwrap().encode(&mut fmmu_image).unwrap();
        assert_eq!(
            controller.accept(read_fmmu, 5, &fmmu_image, 1, now + 1),
            Ok(MappingConfigProgress::Complete)
        );
    }

    #[test]
    fn verified_register_bank_preflight_rejects_sync_manager_drift() {
        let table = mapping_table();
        let mut controller = MappingConfigController::<1, 1>::new();
        assert_eq!(
            controller.start_with_verified_registers(
                0x1000,
                5,
                0,
                1_000,
                100,
                verified_sync_manager_bank(0, 0x1001, 3),
                verified_bank(0, 0x1000, 2),
                &table,
            ),
            Err(MappingConfigError::SyncManagerBankStationMismatch {
                expected: 0x1000,
                observed: 0x1001,
            })
        );
        assert_eq!(controller.phase(), MappingConfigPhase::Idle);

        assert_eq!(
            controller.start_with_verified_registers(
                0x1000,
                5,
                0,
                1_000,
                100,
                verified_sync_manager_bank(1, 0x1000, 3),
                verified_bank(0, 0x1000, 2),
                &table,
            ),
            Err(MappingConfigError::RegisterBankPositionMismatch {
                fmmu: 0,
                sync_manager: 1,
            })
        );

        assert_eq!(
            controller.start_with_verified_registers(
                0x1000,
                5,
                0,
                1_000,
                100,
                verified_sync_manager_bank(0, 0x1000, 0),
                verified_bank(0, 0x1000, 2),
                &table,
            ),
            Err(
                MappingConfigError::ConfiguredSyncManagerCountExceedsDiscovered {
                    configured: 1,
                    discovered: 0,
                }
            )
        );

        assert_eq!(
            controller.start_with_verified_registers(
                0x1000,
                5,
                0,
                1_000,
                100,
                verified_sync_manager_bank(0, 0x1000, 2),
                verified_bank(0, 0x1000, 2),
                &table,
            ),
            Err(MappingConfigError::SyncManagerIndexOutsideDiscoveredBank {
                index: 2,
                discovered: 2,
            })
        );

        assert_eq!(
            controller.start_with_verified_registers(
                0x1000,
                5,
                0,
                1_000,
                100,
                verified_sync_manager_bank(0, 0x1000, (MAX_ESC_SYNC_MANAGERS + 1) as u8,),
                verified_bank(0, 0x1000, 2),
                &table,
            ),
            Err(MappingConfigError::SyncManagerBankCapacityExceeded)
        );
        assert_eq!(controller.phase(), MappingConfigPhase::Idle);
    }

    #[test]
    fn sync_manager_clear_readback_mismatch_latches_fault() {
        let table = mapping_table();
        let mut controller = MappingConfigController::<1, 1>::new();
        controller
            .start_with_verified_registers(
                0x1000,
                5,
                0,
                1_000,
                100,
                verified_sync_manager_bank(0, 0x1000, 3),
                verified_bank(0, 0x1000, 2),
                &table,
            )
            .unwrap();
        let write = controller.next_action(1).unwrap().unwrap();
        controller.accept(write, 5, &[], 1, 2).unwrap();
        let read = controller.next_action(3).unwrap().unwrap();
        let mut stale = [0; SYNC_MANAGER_IMAGE_LEN];
        stale[6] = 1;
        assert_eq!(
            controller.accept(read, 5, &stale, 1, 4),
            Err(MappingConfigError::ReadbackMismatch)
        );
        assert_eq!(controller.phase(), MappingConfigPhase::Faulted);
        assert_eq!(controller.next_action(5), Ok(None));
    }
}
