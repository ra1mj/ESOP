//! Bounded, caller-driven EtherCAT online scan.
//!
//! The scanner only produces the next bounded register request and consumes a
//! completed response. Transport submission, RX index arming and scheduling
//! remain owned by the master/port layer, so a scan cannot block the PDO path.

use crate::control::{ControlError, ControlRequestPool, RegisterOperation, RequestHandle};
use crate::registers::{
    AL_STATUS_WITH_CODE_LEN, BASIC_ESC_INFO_LEN, ESC_AL_STATUS, ESC_STATION_ADDRESS, ESC_TYPE,
    auto_increment_address, fixed_address,
};
use crate::slave::{AlStatus, EthercatState};

const ACTION_PAYLOAD_LEN: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanPhase {
    Idle,
    Probing,
    ReadingBasicInfo,
    AssigningStationAddress,
    ReadingAlStatus,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanError {
    Busy,
    NotStarted,
    NoPendingAction,
    TokenMismatch,
    GenerationMismatch,
    PayloadLengthMismatch,
    UnexpectedWorkingCounter,
    Timeout,
    CapacityExceeded,
    InvalidResponse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanProgress {
    Advanced,
    DeviceDiscovered(usize),
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanAction {
    pub token: u8,
    pub datagram_index: u8,
    pub generation: u16,
    pub position: u16,
    pub station_address: u16,
    pub operation: RegisterOperation,
    pub address: u32,
    pub read_len: u16,
    pub write_payload: [u8; ACTION_PAYLOAD_LEN],
    pub write_len: u8,
    pub deadline_ns: u64,
    pub expected_wkc: u16,
}

impl ScanAction {
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanRecord {
    pub position: u16,
    pub station_address: u16,
    pub esc_type: u16,
    pub revision: u8,
    pub build: u8,
    pub fmmu_count: u8,
    pub sync_manager_count: u8,
    pub ram_size: u16,
    pub port_descriptor: u8,
    pub al_status: AlStatus,
    pub online: bool,
}

impl ScanRecord {
    const EMPTY: Self = Self {
        position: 0,
        station_address: 0,
        esc_type: 0,
        revision: 0,
        build: 0,
        fmmu_count: 0,
        sync_manager_count: 0,
        ram_size: 0,
        port_descriptor: 0,
        al_status: AlStatus {
            state: EthercatState::Unknown,
            error: false,
            raw: 0,
            code: 0,
        },
        online: false,
    };
}

pub struct ScanController<const MAX_SLAVES: usize> {
    phase: ScanPhase,
    generation: u16,
    scan_deadline_ns: u64,
    request_timeout_ns: u64,
    station_address_base: u16,
    next_position: u16,
    record_count: usize,
    current: ScanRecord,
    records: [ScanRecord; MAX_SLAVES],
    pending: Option<ScanAction>,
    next_token: u8,
    next_datagram_index: u8,
    last_error: Option<ScanError>,
}

impl<const MAX_SLAVES: usize> ScanController<MAX_SLAVES> {
    pub const fn new(station_address_base: u16) -> Self {
        Self {
            phase: ScanPhase::Idle,
            generation: 0,
            scan_deadline_ns: 0,
            request_timeout_ns: 0,
            station_address_base,
            next_position: 0,
            record_count: 0,
            current: ScanRecord::EMPTY,
            records: [ScanRecord::EMPTY; MAX_SLAVES],
            pending: None,
            next_token: 1,
            next_datagram_index: 1,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> ScanPhase {
        self.phase
    }

    pub const fn len(&self) -> usize {
        self.record_count
    }

    pub const fn is_empty(&self) -> bool {
        self.record_count == 0
    }

    pub fn records(&self) -> &[ScanRecord] {
        &self.records[..self.record_count]
    }

    pub const fn pending(&self) -> Option<ScanAction> {
        self.pending
    }

    pub const fn last_error(&self) -> Option<ScanError> {
        self.last_error
    }

    pub fn start(
        &mut self,
        generation: u16,
        now_ns: u64,
        timeout_ns: u64,
        request_timeout_ns: u64,
    ) -> Result<(), ScanError> {
        if !matches!(
            self.phase,
            ScanPhase::Idle | ScanPhase::Complete | ScanPhase::Faulted
        ) {
            return Err(ScanError::Busy);
        }
        if MAX_SLAVES == 0 {
            return Err(ScanError::CapacityExceeded);
        }
        self.phase = ScanPhase::Probing;
        self.generation = generation;
        self.scan_deadline_ns = now_ns.saturating_add(timeout_ns);
        self.request_timeout_ns = request_timeout_ns;
        self.next_position = 0;
        self.record_count = 0;
        self.current = ScanRecord::EMPTY;
        self.pending = None;
        self.next_token = 1;
        self.next_datagram_index = 1;
        self.last_error = None;
        Ok(())
    }

    pub fn next_action(&mut self, now_ns: u64) -> Result<Option<ScanAction>, ScanError> {
        if self.phase == ScanPhase::Idle {
            return Err(ScanError::NotStarted);
        }
        if self.phase == ScanPhase::Complete || self.phase == ScanPhase::Faulted {
            return Ok(None);
        }
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        if now_ns >= self.scan_deadline_ns {
            self.fail(ScanError::Timeout);
            return Err(ScanError::Timeout);
        }

        let (operation, address, read_len, payload, write_len, position, station_address) =
            match self.phase {
                ScanPhase::Probing => {
                    if self.next_position as usize >= MAX_SLAVES {
                        self.phase = ScanPhase::Complete;
                        return Ok(None);
                    }
                    (
                        RegisterOperation::AutoIncrementRead,
                        auto_increment_address(self.next_position, ESC_TYPE),
                        2,
                        [0; ACTION_PAYLOAD_LEN],
                        0,
                        self.next_position,
                        self.station_address_base.wrapping_add(self.next_position),
                    )
                }
                ScanPhase::ReadingBasicInfo => (
                    RegisterOperation::AutoIncrementRead,
                    auto_increment_address(self.current.position, ESC_TYPE),
                    BASIC_ESC_INFO_LEN,
                    [0; ACTION_PAYLOAD_LEN],
                    0,
                    self.current.position,
                    self.current.station_address,
                ),
                ScanPhase::AssigningStationAddress => (
                    RegisterOperation::AutoIncrementWrite,
                    auto_increment_address(self.current.position, ESC_STATION_ADDRESS),
                    0,
                    self.current.station_address.to_le_bytes(),
                    2,
                    self.current.position,
                    self.current.station_address,
                ),
                ScanPhase::ReadingAlStatus => (
                    RegisterOperation::Read,
                    fixed_address(self.current.station_address, ESC_AL_STATUS),
                    AL_STATUS_WITH_CODE_LEN,
                    [0; ACTION_PAYLOAD_LEN],
                    0,
                    self.current.position,
                    self.current.station_address,
                ),
                ScanPhase::Idle | ScanPhase::Complete | ScanPhase::Faulted => return Ok(None),
            };

        let deadline_ns = now_ns
            .saturating_add(self.request_timeout_ns)
            .min(self.scan_deadline_ns);
        let action = ScanAction {
            token: self.next_token,
            datagram_index: self.next_datagram_index,
            generation: self.generation,
            position,
            station_address,
            operation,
            address,
            read_len,
            write_payload: payload,
            write_len,
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
        token: u8,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<ScanProgress, ScanError> {
        let action = self.pending.ok_or(ScanError::NoPendingAction)?;
        if action.token != token {
            return Err(ScanError::TokenMismatch);
        }
        if action.generation != generation {
            self.fail(ScanError::GenerationMismatch);
            return Err(ScanError::GenerationMismatch);
        }
        if now_ns > action.deadline_ns {
            self.fail(ScanError::Timeout);
            return Err(ScanError::Timeout);
        }

        if self.phase == ScanPhase::Probing && working_counter == 0 {
            self.pending = None;
            self.phase = ScanPhase::Complete;
            return Ok(ScanProgress::Complete);
        }
        if working_counter != action.expected_wkc {
            self.fail(ScanError::UnexpectedWorkingCounter);
            return Err(ScanError::UnexpectedWorkingCounter);
        }
        if payload.len() != action.read_len as usize {
            self.fail(ScanError::PayloadLengthMismatch);
            return Err(ScanError::PayloadLengthMismatch);
        }

        let progress = match self.phase {
            ScanPhase::Probing => {
                self.current = ScanRecord {
                    position: action.position,
                    station_address: action.station_address,
                    esc_type: u16::from_le_bytes([payload[0], payload[1]]),
                    online: true,
                    ..ScanRecord::EMPTY
                };
                self.phase = ScanPhase::ReadingBasicInfo;
                ScanProgress::Advanced
            }
            ScanPhase::ReadingBasicInfo => {
                self.current.revision = payload[2];
                self.current.build = payload[3];
                self.current.fmmu_count = payload[4];
                self.current.sync_manager_count = payload[5];
                self.current.ram_size = u16::from_le_bytes([payload[6], payload[7]]);
                self.current.port_descriptor = payload.get(8).copied().unwrap_or(0);
                self.phase = ScanPhase::AssigningStationAddress;
                ScanProgress::Advanced
            }
            ScanPhase::AssigningStationAddress => {
                self.phase = ScanPhase::ReadingAlStatus;
                ScanProgress::Advanced
            }
            ScanPhase::ReadingAlStatus => {
                let raw = u16::from_le_bytes([payload[0], payload[1]]);
                let code = u16::from_le_bytes([payload[4], payload[5]]);
                self.current.al_status = AlStatus::new(raw, code);
                let index = self.record_count;
                self.records[index] = self.current;
                self.record_count += 1;
                self.next_position = self.next_position.saturating_add(1);
                self.phase = ScanPhase::Probing;
                ScanProgress::DeviceDiscovered(index)
            }
            ScanPhase::Idle | ScanPhase::Complete | ScanPhase::Faulted => {
                return Err(ScanError::NotStarted);
            }
        };
        self.pending = None;
        Ok(progress)
    }

    pub fn timeout(&mut self, token: u8, now_ns: u64) -> Result<ScanProgress, ScanError> {
        let action = self.pending.ok_or(ScanError::NoPendingAction)?;
        if action.token != token {
            return Err(ScanError::TokenMismatch);
        }
        if now_ns < action.deadline_ns {
            return Err(ScanError::Timeout);
        }
        self.pending = None;
        if self.phase == ScanPhase::Probing {
            self.phase = ScanPhase::Complete;
            Ok(ScanProgress::Complete)
        } else {
            self.fail(ScanError::Timeout);
            Err(ScanError::Timeout)
        }
    }

    fn fail(&mut self, error: ScanError) {
        self.last_error = Some(error);
        self.pending = None;
        self.phase = ScanPhase::Faulted;
    }
}

impl<const MAX_SLAVES: usize> Default for ScanController<MAX_SLAVES> {
    fn default() -> Self {
        Self::new(0x1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registers::{ESC_AL_STATUS, ESC_TYPE, auto_increment_address, fixed_address};

    fn basic_info() -> [u8; 9] {
        [0x88, 0x02, 3, 4, 5, 6, 0x00, 0x20, 0x01]
    }

    #[test]
    fn scan_progresses_through_probe_basic_address_and_al_status() {
        let mut scan = ScanController::<2>::new(0x1000);
        scan.start(7, 0, 1_000, 100).unwrap();

        let probe = scan.next_action(1).unwrap().unwrap();
        assert_eq!(probe.operation, RegisterOperation::AutoIncrementRead);
        assert_eq!(probe.address, auto_increment_address(0, ESC_TYPE));
        scan.accept(probe.token, 7, &[0x88, 0x02], 1, 2).unwrap();

        let basic = scan.next_action(3).unwrap().unwrap();
        assert_eq!(basic.read_len, BASIC_ESC_INFO_LEN);
        scan.accept(basic.token, 7, &basic_info(), 1, 4).unwrap();

        let assign = scan.next_action(5).unwrap().unwrap();
        assert_eq!(assign.operation, RegisterOperation::AutoIncrementWrite);
        assert_eq!(assign.payload(), &[0x00, 0x10]);
        scan.accept(assign.token, 7, &[], 1, 6).unwrap();

        let status = scan.next_action(7).unwrap().unwrap();
        assert_eq!(status.address, fixed_address(0x1000, ESC_AL_STATUS));
        scan.accept(status.token, 7, &[0x04, 0x00, 0, 0, 0, 0], 1, 8)
            .unwrap();

        let next_probe = scan.next_action(9).unwrap().unwrap();
        assert_eq!(next_probe.position, 1);
        scan.timeout(next_probe.token, next_probe.deadline_ns)
            .unwrap();
        assert_eq!(scan.phase(), ScanPhase::Complete);
        assert_eq!(scan.len(), 1);
        assert_eq!(scan.records()[0].al_status.state, EthercatState::SafeOp);
    }

    #[test]
    fn zero_wkc_probe_finishes_without_creating_a_record() {
        let mut scan = ScanController::<4>::new(0x1000);
        scan.start(2, 0, 1_000, 100).unwrap();
        let action = scan.next_action(1).unwrap().unwrap();
        assert_eq!(
            scan.accept(action.token, 2, &[], 0, 2).unwrap(),
            ScanProgress::Complete
        );
        assert!(scan.is_empty());
        assert_eq!(scan.phase(), ScanPhase::Complete);
    }

    #[test]
    fn non_probe_timeout_faults_and_cannot_auto_complete() {
        let mut scan = ScanController::<1>::new(0x1000);
        scan.start(1, 0, 1_000, 10).unwrap();
        let probe = scan.next_action(1).unwrap().unwrap();
        scan.accept(probe.token, 1, &[1, 0], 1, 2).unwrap();
        let basic = scan.next_action(3).unwrap().unwrap();
        assert_eq!(
            scan.timeout(basic.token, basic.deadline_ns),
            Err(ScanError::Timeout)
        );
        assert_eq!(scan.phase(), ScanPhase::Faulted);
        assert_eq!(scan.last_error(), Some(ScanError::Timeout));
    }
}
