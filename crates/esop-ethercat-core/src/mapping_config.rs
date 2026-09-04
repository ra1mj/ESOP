//! Fixed-capacity SyncManager/FMMU configuration transactions.
//!
//! Configuration is deliberately driven by the caller. Each table entry is
//! written once and read back once before the controller advances, so a
//! partially applied mapping cannot be reported as ready.

use crate::control::{
    ControlError, ControlRequestPool, MAX_CONTROL_PAYLOAD, RegisterOperation, RequestHandle,
    RequestState,
};
use crate::mapping::{
    FMMU_IMAGE_LEN, FmmuConfig, MappingError, MappingTable, SYNC_MANAGER_IMAGE_LEN,
    SyncManagerConfig,
};
use crate::registers::fixed_address;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MappingConfigPhase {
    Idle,
    WritingSyncManager,
    VerifyingSyncManager,
    WritingFmmu,
    VerifyingFmmu,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MappingConfigItem {
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
        if !matches!(
            self.phase,
            MappingConfigPhase::Idle | MappingConfigPhase::Complete | MappingConfigPhase::Faulted
        ) {
            return Err(MappingConfigError::Busy);
        }
        self.sync_manager_count = table.sync_manager_count();
        self.fmmu_count = table.fmmu_count();
        self.sync_managers[..self.sync_manager_count].copy_from_slice(table.sync_managers());
        self.fmmus[..self.fmmu_count].copy_from_slice(table.fmmus());
        self.station_address = station_address;
        self.generation = generation;
        self.configuration_deadline_ns = now_ns.saturating_add(timeout_ns);
        self.request_timeout_ns = request_timeout_ns;
        self.item_index = 0;
        self.pending = None;
        self.next_token = 1;
        self.next_datagram_index = 1;
        self.last_error = None;
        self.phase = if self.sync_manager_count != 0 {
            MappingConfigPhase::WritingSyncManager
        } else if self.fmmu_count != 0 {
            MappingConfigPhase::WritingFmmu
        } else {
            MappingConfigPhase::Complete
        };
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
                config
                    .encode(&mut encoded[..SYNC_MANAGER_IMAGE_LEN])
                    .map_err(MappingConfigError::Mapping)?;
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
            MappingConfigPhase::WritingSyncManager => {
                self.phase = MappingConfigPhase::VerifyingSyncManager;
                MappingConfigProgress::Advanced
            }
            MappingConfigPhase::VerifyingSyncManager => {
                let mut expected = [0; FMMU_IMAGE_LEN];
                self.sync_managers[self.item_index]
                    .encode(&mut expected[..SYNC_MANAGER_IMAGE_LEN])
                    .map_err(MappingConfigError::Mapping)?;
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
                if request.datagram_index != action.datagram_index
                    || request.generation != action.generation
                    || request.address != action.address
                    || request.response_length != action.datagram_len()
                    || request.length < action.response_len()
                {
                    let _ = pool.release(handle);
                    return self.fail(MappingConfigError::ActionMismatch);
                }
                let mut response = [0; MAX_CONTROL_PAYLOAD];
                response[..request.length].copy_from_slice(request.payload());
                (request.generation, request.actual_wkc, response)
            }
            Some(request) if request.state == RequestState::Failed => {
                let _ = pool.release(handle);
                return self.fail(MappingConfigError::Control(ControlError::InvalidState));
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
    use crate::mapping::{ESC_FMMU_BASE, ESC_SYNC_MANAGER_BASE};

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
}
