//! Bounded, caller-driven EtherCAT online scan.
//!
//! The scanner only produces the next bounded register request and consumes a
//! completed response. Transport submission, RX index arming and scheduling
//! remain owned by the master/port layer, so a scan cannot block the PDO path.

use crate::control::{ControlError, ControlRequestPool, RegisterOperation, RequestHandle};
use crate::registers::{
    AL_STATUS_WITH_CODE_LEN, BASIC_ESC_INFO_LEN, ESC_AL_STATUS, ESC_CONFIGURATION,
    ESC_DC_RECEIVE_TIME_LEN, ESC_DC_SYSTEM_TIME, ESC_DC_TIME0, ESC_DEVICE_EMULATION, ESC_DL_STATUS,
    ESC_DL_STATUS_LEN, ESC_FEATURE_DC_64_BIT, ESC_FEATURE_DC_SUPPORTED,
    ESC_FEATURE_FMMU_BIT_OPERATION, ESC_PORT_COUNT, ESC_STATION_ADDRESS, ESC_TYPE,
    auto_increment_address, fixed_address,
};
use crate::rx_index::RxWorkingCounterPolicy;
use crate::slave::{AlStatus, EthercatState};

const ACTION_PAYLOAD_LEN: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanPhase {
    Idle,
    Probing,
    ReadingBasicInfo,
    AssigningStationAddress,
    ReadingDcSystemTime,
    ReadingDcReceiveTimes,
    ReadingDataLinkStatus,
    ReadingEscConfiguration,
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
    pub working_counter_policy: RxWorkingCounterPolicy,
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

    pub const fn accepts_working_counter(&self, working_counter: u16) -> bool {
        self.working_counter_policy
            .accepts(self.expected_wkc, working_counter)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EscDcRange {
    Bits32,
    Bits64,
}

impl EscDcRange {
    pub const fn system_time_len(self) -> u16 {
        match self {
            Self::Bits32 => 4,
            Self::Bits64 => 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanDcCapabilities {
    pub raw_features: u16,
    pub fmmu_bit_operation: bool,
    pub supported: bool,
    pub range: EscDcRange,
    pub has_system_time: bool,
    pub system_time: Option<u64>,
    pub receive_times: Option<[u32; ESC_PORT_COUNT]>,
}

impl ScanDcCapabilities {
    pub const NONE: Self = Self {
        raw_features: 0,
        fmmu_bit_operation: false,
        supported: false,
        range: EscDcRange::Bits32,
        has_system_time: false,
        system_time: None,
        receive_times: None,
    };

    const fn from_features(raw_features: u16) -> Self {
        Self {
            raw_features,
            fmmu_bit_operation: raw_features & ESC_FEATURE_FMMU_BIT_OPERATION != 0,
            supported: raw_features & ESC_FEATURE_DC_SUPPORTED != 0,
            range: if raw_features & ESC_FEATURE_DC_64_BIT != 0 {
                EscDcRange::Bits64
            } else {
                EscDcRange::Bits32
            },
            has_system_time: false,
            system_time: None,
            receive_times: None,
        }
    }

    pub const fn can_be_reference_clock(self) -> bool {
        self.supported && self.has_system_time
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanPortLink {
    pub link_up: bool,
    pub loop_closed: bool,
    pub signal_detected: bool,
}

impl ScanPortLink {
    pub const NONE: Self = Self {
        link_up: false,
        loop_closed: false,
        signal_detected: false,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanRecord {
    pub position: u16,
    pub station_address: u16,
    pub esc_type: u8,
    pub revision: u8,
    pub build: u16,
    pub fmmu_count: u8,
    pub sync_manager_count: u8,
    pub ram_size: u16,
    pub port_descriptor: u8,
    pub dl_status: u16,
    pub port_links: [ScanPortLink; ESC_PORT_COUNT],
    pub dc: ScanDcCapabilities,
    pub device_emulation: bool,
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
        dl_status: 0,
        port_links: [ScanPortLink::NONE; ESC_PORT_COUNT],
        dc: ScanDcCapabilities::NONE,
        device_emulation: false,
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
        self.records = [ScanRecord::EMPTY; MAX_SLAVES];
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
                ScanPhase::ReadingDcSystemTime => (
                    RegisterOperation::Read,
                    fixed_address(self.current.station_address, ESC_DC_SYSTEM_TIME),
                    self.current.dc.range.system_time_len(),
                    [0; ACTION_PAYLOAD_LEN],
                    0,
                    self.current.position,
                    self.current.station_address,
                ),
                ScanPhase::ReadingDcReceiveTimes => (
                    RegisterOperation::Read,
                    fixed_address(self.current.station_address, ESC_DC_TIME0),
                    ESC_DC_RECEIVE_TIME_LEN,
                    [0; ACTION_PAYLOAD_LEN],
                    0,
                    self.current.position,
                    self.current.station_address,
                ),
                ScanPhase::ReadingDataLinkStatus => (
                    RegisterOperation::Read,
                    fixed_address(self.current.station_address, ESC_DL_STATUS),
                    ESC_DL_STATUS_LEN,
                    [0; ACTION_PAYLOAD_LEN],
                    0,
                    self.current.position,
                    self.current.station_address,
                ),
                ScanPhase::ReadingEscConfiguration => (
                    RegisterOperation::Read,
                    fixed_address(self.current.station_address, ESC_CONFIGURATION),
                    1,
                    [0; ACTION_PAYLOAD_LEN],
                    0,
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

        let working_counter_policy = if matches!(
            self.phase,
            ScanPhase::Probing | ScanPhase::ReadingDcSystemTime
        ) {
            RxWorkingCounterPolicy::ZeroOrOne
        } else {
            RxWorkingCounterPolicy::Exact
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
            working_counter_policy,
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
        pool.acquire_with_response_len_and_wkc_policy(
            action.datagram_index,
            action.generation,
            action.address,
            action.operation,
            action.payload(),
            action.datagram_len(),
            action.deadline_ns,
            action.working_counter_policy,
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

        if !action.accepts_working_counter(working_counter) {
            self.fail(ScanError::UnexpectedWorkingCounter);
            return Err(ScanError::UnexpectedWorkingCounter);
        }
        if self.phase == ScanPhase::Probing && working_counter == 0 {
            self.pending = None;
            self.phase = ScanPhase::Complete;
            return Ok(ScanProgress::Complete);
        }
        if self.phase == ScanPhase::ReadingDcSystemTime && working_counter == 0 {
            self.pending = None;
            self.phase = ScanPhase::ReadingDcReceiveTimes;
            return Ok(ScanProgress::Advanced);
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
                    online: true,
                    ..ScanRecord::EMPTY
                };
                self.phase = ScanPhase::ReadingBasicInfo;
                ScanProgress::Advanced
            }
            ScanPhase::ReadingBasicInfo => {
                self.current.esc_type = payload[0];
                self.current.revision = payload[1];
                self.current.build = u16::from_le_bytes([payload[2], payload[3]]);
                self.current.fmmu_count = payload[4];
                self.current.sync_manager_count = payload[5];
                self.current.ram_size = payload[6] as u16;
                self.current.port_descriptor = payload[7];
                self.current.dc =
                    ScanDcCapabilities::from_features(u16::from_le_bytes([payload[8], payload[9]]));
                self.phase = ScanPhase::AssigningStationAddress;
                ScanProgress::Advanced
            }
            ScanPhase::AssigningStationAddress => {
                self.phase = if self.current.dc.supported {
                    ScanPhase::ReadingDcSystemTime
                } else {
                    ScanPhase::ReadingDataLinkStatus
                };
                ScanProgress::Advanced
            }
            ScanPhase::ReadingDcSystemTime => {
                self.current.dc.system_time = Some(match self.current.dc.range {
                    EscDcRange::Bits32 => {
                        u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as u64
                    }
                    EscDcRange::Bits64 => u64::from_le_bytes([
                        payload[0], payload[1], payload[2], payload[3], payload[4], payload[5],
                        payload[6], payload[7],
                    ]),
                });
                self.current.dc.has_system_time = true;
                self.phase = ScanPhase::ReadingDcReceiveTimes;
                ScanProgress::Advanced
            }
            ScanPhase::ReadingDcReceiveTimes => {
                let mut receive_times = [0; ESC_PORT_COUNT];
                for (port, receive_time) in receive_times.iter_mut().enumerate() {
                    let offset = port * 4;
                    *receive_time = u32::from_le_bytes([
                        payload[offset],
                        payload[offset + 1],
                        payload[offset + 2],
                        payload[offset + 3],
                    ]);
                }
                self.current.dc.receive_times = Some(receive_times);
                self.phase = ScanPhase::ReadingDataLinkStatus;
                ScanProgress::Advanced
            }
            ScanPhase::ReadingDataLinkStatus => {
                let dl_status = u16::from_le_bytes([payload[0], payload[1]]);
                self.current.dl_status = dl_status;
                for (port, link) in self.current.port_links.iter_mut().enumerate() {
                    *link = ScanPortLink {
                        link_up: dl_status & (1 << (4 + port)) != 0,
                        loop_closed: dl_status & (1 << (8 + port * 2)) != 0,
                        signal_detected: dl_status & (1 << (9 + port * 2)) != 0,
                    };
                }
                self.phase = ScanPhase::ReadingEscConfiguration;
                ScanProgress::Advanced
            }
            ScanPhase::ReadingEscConfiguration => {
                self.current.device_emulation = payload[0] & ESC_DEVICE_EMULATION != 0;
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
    use crate::registers::{
        ESC_AL_STATUS, ESC_CONFIGURATION, ESC_DC_TIME0, ESC_DL_STATUS, ESC_TYPE,
        auto_increment_address, fixed_address,
    };

    const CLOSED_DL_STATUS: [u8; ESC_DL_STATUS_LEN as usize] = 0x5500u16.to_le_bytes();

    fn basic_info() -> [u8; BASIC_ESC_INFO_LEN as usize] {
        [
            0x88, 0x02, 0x34, 0x12, 5, 6, 0x20, 0xE4, 0x01, 0x00, 0xAA, 0xBB,
        ]
    }

    fn basic_info_with_features(features: u16) -> [u8; BASIC_ESC_INFO_LEN as usize] {
        let mut bytes = basic_info();
        bytes[8..10].copy_from_slice(&features.to_le_bytes());
        bytes
    }

    fn receive_times(values: [u32; ESC_PORT_COUNT]) -> [u8; ESC_DC_RECEIVE_TIME_LEN as usize] {
        let mut bytes = [0; ESC_DC_RECEIVE_TIME_LEN as usize];
        for (port, value) in values.iter().copied().enumerate() {
            bytes[port * 4..port * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    fn accept_dl_status<const MAX_SLAVES: usize>(
        scan: &mut ScanController<MAX_SLAVES>,
        generation: u16,
        payload: &[u8],
        now_ns: u64,
    ) {
        let action = scan.next_action(now_ns).unwrap().unwrap();
        assert_eq!(
            action.address,
            fixed_address(action.station_address, ESC_DL_STATUS)
        );
        assert_eq!(action.read_len, ESC_DL_STATUS_LEN);
        scan.accept(action.token, generation, payload, 1, now_ns + 1)
            .unwrap();
    }

    fn accept_receive_times<const MAX_SLAVES: usize>(
        scan: &mut ScanController<MAX_SLAVES>,
        generation: u16,
        values: [u32; ESC_PORT_COUNT],
        now_ns: u64,
    ) {
        let action = scan.next_action(now_ns).unwrap().unwrap();
        assert_eq!(
            action.address,
            fixed_address(action.station_address, ESC_DC_TIME0)
        );
        assert_eq!(action.read_len, ESC_DC_RECEIVE_TIME_LEN);
        assert_eq!(action.working_counter_policy, RxWorkingCounterPolicy::Exact);
        scan.accept(
            action.token,
            generation,
            &receive_times(values),
            1,
            now_ns + 1,
        )
        .unwrap();
    }

    fn advance_to_dc_probe(
        scan: &mut ScanController<1>,
        generation: u16,
        features: u16,
    ) -> ScanAction {
        let probe = scan.next_action(1).unwrap().unwrap();
        scan.accept(probe.token, generation, &[0x88, 0x02], 1, 2)
            .unwrap();
        let basic = scan.next_action(3).unwrap().unwrap();
        scan.accept(
            basic.token,
            generation,
            &basic_info_with_features(features),
            1,
            4,
        )
        .unwrap();
        let assign = scan.next_action(5).unwrap().unwrap();
        scan.accept(assign.token, generation, &[], 1, 6).unwrap();
        scan.next_action(7).unwrap().unwrap()
    }

    fn advance_to_dl_status(scan: &mut ScanController<1>, generation: u16) -> ScanAction {
        let probe = scan.next_action(1).unwrap().unwrap();
        scan.accept(probe.token, generation, &[1, 0], 1, 2).unwrap();
        let basic = scan.next_action(3).unwrap().unwrap();
        scan.accept(basic.token, generation, &basic_info(), 1, 4)
            .unwrap();
        let assign = scan.next_action(5).unwrap().unwrap();
        scan.accept(assign.token, generation, &[], 1, 6).unwrap();
        scan.next_action(7).unwrap().unwrap()
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

        accept_dl_status(&mut scan, 7, &CLOSED_DL_STATUS, 7);

        let configuration = scan.next_action(9).unwrap().unwrap();
        assert_eq!(
            configuration.address,
            fixed_address(0x1000, ESC_CONFIGURATION)
        );
        assert_eq!(configuration.read_len, 1);
        scan.accept(configuration.token, 7, &[ESC_DEVICE_EMULATION], 1, 10)
            .unwrap();

        let status = scan.next_action(11).unwrap().unwrap();
        assert_eq!(status.address, fixed_address(0x1000, ESC_AL_STATUS));
        scan.accept(status.token, 7, &[0x04, 0x00, 0, 0, 0, 0], 1, 12)
            .unwrap();

        let next_probe = scan.next_action(13).unwrap().unwrap();
        assert_eq!(next_probe.position, 1);
        scan.timeout(next_probe.token, next_probe.deadline_ns)
            .unwrap();
        assert_eq!(scan.phase(), ScanPhase::Complete);
        assert_eq!(scan.len(), 1);
        assert!(scan.records()[0].device_emulation);
        assert_eq!(scan.records()[0].al_status.state, EthercatState::SafeOp);
        assert_eq!(scan.records()[0].esc_type, 0x88);
        assert_eq!(scan.records()[0].revision, 0x02);
        assert_eq!(scan.records()[0].build, 0x1234);
        assert_eq!(scan.records()[0].fmmu_count, 5);
        assert_eq!(scan.records()[0].sync_manager_count, 6);
        assert_eq!(scan.records()[0].ram_size, 0x20);
        assert_eq!(scan.records()[0].port_descriptor, 0xE4);
        assert_eq!(scan.records()[0].dl_status, 0x5500);
        assert!(
            scan.records()[0]
                .port_links
                .iter()
                .all(|link| link.loop_closed)
        );
        assert_eq!(scan.records()[0].dc.raw_features, 0x0001);
        assert!(scan.records()[0].dc.fmmu_bit_operation);
        assert!(!scan.records()[0].dc.supported);
    }

    #[test]
    fn esc_configuration_requires_an_exact_response() {
        let mut scan = ScanController::<1>::new(0x1000);
        scan.start(3, 0, 1_000, 100).unwrap();
        let probe = scan.next_action(1).unwrap().unwrap();
        scan.accept(probe.token, 3, &[1, 0], 1, 2).unwrap();
        let basic = scan.next_action(3).unwrap().unwrap();
        scan.accept(basic.token, 3, &basic_info(), 1, 4).unwrap();
        let assign = scan.next_action(5).unwrap().unwrap();
        scan.accept(assign.token, 3, &[], 1, 6).unwrap();
        accept_dl_status(&mut scan, 3, &CLOSED_DL_STATUS, 7);

        let configuration = scan.next_action(9).unwrap().unwrap();
        assert_eq!(
            scan.accept(configuration.token, 3, &[], 1, 8),
            Err(ScanError::PayloadLengthMismatch)
        );
        assert_eq!(scan.phase(), ScanPhase::Faulted);
    }

    #[test]
    fn esc_configuration_timeout_faults_instead_of_assuming_a_policy() {
        let mut scan = ScanController::<1>::new(0x1000);
        scan.start(4, 0, 1_000, 10).unwrap();
        let probe = scan.next_action(1).unwrap().unwrap();
        scan.accept(probe.token, 4, &[1, 0], 1, 2).unwrap();
        let basic = scan.next_action(3).unwrap().unwrap();
        scan.accept(basic.token, 4, &basic_info(), 1, 4).unwrap();
        let assign = scan.next_action(5).unwrap().unwrap();
        scan.accept(assign.token, 4, &[], 1, 6).unwrap();
        accept_dl_status(&mut scan, 4, &CLOSED_DL_STATUS, 7);

        let configuration = scan.next_action(9).unwrap().unwrap();
        assert_eq!(
            scan.timeout(configuration.token, configuration.deadline_ns),
            Err(ScanError::Timeout)
        );
        assert_eq!(scan.phase(), ScanPhase::Faulted);
        assert!(scan.records().is_empty());
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

    #[test]
    fn dc_system_time_probe_decodes_32_bit_reference_capability() {
        let mut scan = ScanController::<1>::new(0x1000);
        scan.start(5, 0, 1_000, 100).unwrap();
        let dc = advance_to_dc_probe(&mut scan, 5, ESC_FEATURE_DC_SUPPORTED);
        assert_eq!(scan.phase(), ScanPhase::ReadingDcSystemTime);
        assert_eq!(dc.address, fixed_address(0x1000, ESC_DC_SYSTEM_TIME));
        assert_eq!(dc.read_len, 4);
        assert_eq!(dc.working_counter_policy, RxWorkingCounterPolicy::ZeroOrOne);
        scan.accept(dc.token, 5, &[0x78, 0x56, 0x34, 0x12], 1, 8)
            .unwrap();
        accept_receive_times(&mut scan, 5, [100, 110, 120, 130], 9);
        accept_dl_status(&mut scan, 5, &CLOSED_DL_STATUS, 11);

        let configuration = scan.next_action(13).unwrap().unwrap();
        scan.accept(configuration.token, 5, &[0], 1, 14).unwrap();
        let status = scan.next_action(15).unwrap().unwrap();
        scan.accept(status.token, 5, &[1, 0, 0, 0, 0, 0], 1, 16)
            .unwrap();

        let record = scan.records()[0];
        assert_eq!(record.dc.range, EscDcRange::Bits32);
        assert_eq!(record.dc.system_time, Some(0x1234_5678));
        assert_eq!(record.dc.receive_times, Some([100, 110, 120, 130]));
        assert!(record.dc.can_be_reference_clock());
    }

    #[test]
    fn dc_system_time_probe_decodes_64_bit_range() {
        let mut scan = ScanController::<1>::new(0x1000);
        scan.start(6, 0, 1_000, 100).unwrap();
        let features = ESC_FEATURE_DC_SUPPORTED | ESC_FEATURE_DC_64_BIT;
        let dc = advance_to_dc_probe(&mut scan, 6, features);
        assert_eq!(dc.read_len, 8);
        scan.accept(dc.token, 6, &0x1122_3344_5566_7788u64.to_le_bytes(), 1, 8)
            .unwrap();
        assert_eq!(scan.current.dc.range, EscDcRange::Bits64);
        assert_eq!(scan.current.dc.system_time, Some(0x1122_3344_5566_7788));
        assert_eq!(scan.phase(), ScanPhase::ReadingDcReceiveTimes);
    }

    #[test]
    fn zero_wkc_dc_probe_is_delay_only_and_continues() {
        let mut scan = ScanController::<1>::new(0x1000);
        scan.start(7, 0, 1_000, 100).unwrap();
        let dc = advance_to_dc_probe(&mut scan, 7, ESC_FEATURE_DC_SUPPORTED);
        assert_eq!(
            scan.accept(dc.token, 7, &[], 0, 8),
            Ok(ScanProgress::Advanced)
        );
        assert_eq!(scan.phase(), ScanPhase::ReadingDcReceiveTimes);
        assert!(scan.current.dc.supported);
        assert!(!scan.current.dc.has_system_time);
        assert_eq!(scan.current.dc.system_time, None);
        assert!(!scan.current.dc.can_be_reference_clock());

        accept_receive_times(&mut scan, 7, [1, 2, 3, 4], 9);
        assert_eq!(scan.current.dc.receive_times, Some([1, 2, 3, 4]));
    }

    #[test]
    fn dc_probe_rejects_invalid_wkc_short_payload_and_timeout() {
        let mut invalid_wkc = ScanController::<1>::new(0x1000);
        invalid_wkc.start(8, 0, 1_000, 100).unwrap();
        let action = advance_to_dc_probe(&mut invalid_wkc, 8, ESC_FEATURE_DC_SUPPORTED);
        assert_eq!(
            invalid_wkc.accept(action.token, 8, &[0; 4], 2, 8),
            Err(ScanError::UnexpectedWorkingCounter)
        );

        let mut short = ScanController::<1>::new(0x1000);
        short.start(9, 0, 1_000, 100).unwrap();
        let action = advance_to_dc_probe(&mut short, 9, ESC_FEATURE_DC_SUPPORTED);
        assert_eq!(
            short.accept(action.token, 9, &[0; 3], 1, 8),
            Err(ScanError::PayloadLengthMismatch)
        );

        let mut timeout = ScanController::<1>::new(0x1000);
        timeout.start(10, 0, 1_000, 100).unwrap();
        let action = advance_to_dc_probe(&mut timeout, 10, ESC_FEATURE_DC_SUPPORTED);
        assert_eq!(
            timeout.timeout(action.token, action.deadline_ns),
            Err(ScanError::Timeout)
        );
        assert_eq!(timeout.phase(), ScanPhase::Faulted);
    }

    #[test]
    fn restart_clears_published_dc_evidence() {
        let mut scan = ScanController::<1>::new(0x1000);
        scan.start(11, 0, 1_000, 100).unwrap();
        let dc = advance_to_dc_probe(&mut scan, 11, ESC_FEATURE_DC_SUPPORTED);
        scan.accept(dc.token, 11, &[1, 0, 0, 0], 1, 8).unwrap();
        accept_receive_times(&mut scan, 11, [1, 2, 3, 4], 9);
        accept_dl_status(&mut scan, 11, &CLOSED_DL_STATUS, 11);
        let configuration = scan.next_action(13).unwrap().unwrap();
        scan.accept(configuration.token, 11, &[0], 1, 14).unwrap();
        let status = scan.next_action(15).unwrap().unwrap();
        scan.accept(status.token, 11, &[1, 0, 0, 0, 0, 0], 1, 16)
            .unwrap();
        assert!(scan.records()[0].dc.can_be_reference_clock());
        assert_eq!(scan.next_action(17).unwrap(), None);
        assert_eq!(scan.phase(), ScanPhase::Complete);

        scan.start(12, 20, 1_000, 100).unwrap();
        assert!(scan.records().is_empty());
        assert_eq!(scan.current.dc, ScanDcCapabilities::NONE);
    }

    #[test]
    fn dl_status_decodes_all_port_flags() {
        let mut scan = ScanController::<1>::new(0x1000);
        scan.start(12, 0, 1_000, 100).unwrap();
        let probe = scan.next_action(1).unwrap().unwrap();
        scan.accept(probe.token, 12, &[1, 0], 1, 2).unwrap();
        let basic = scan.next_action(3).unwrap().unwrap();
        scan.accept(basic.token, 12, &basic_info(), 1, 4).unwrap();
        let assign = scan.next_action(5).unwrap().unwrap();
        scan.accept(assign.token, 12, &[], 1, 6).unwrap();

        let raw = (1 << 4) | (1 << 6) | (1 << 9) | (1 << 10) | (1 << 13) | (1 << 14);
        accept_dl_status(&mut scan, 12, &u16::to_le_bytes(raw), 7);
        assert_eq!(scan.current.dl_status, raw);
        assert_eq!(
            scan.current.port_links,
            [
                ScanPortLink {
                    link_up: true,
                    loop_closed: false,
                    signal_detected: true,
                },
                ScanPortLink {
                    link_up: false,
                    loop_closed: true,
                    signal_detected: false,
                },
                ScanPortLink {
                    link_up: true,
                    loop_closed: false,
                    signal_detected: true,
                },
                ScanPortLink {
                    link_up: false,
                    loop_closed: true,
                    signal_detected: false,
                },
            ]
        );
    }

    #[test]
    fn receive_time_and_dl_reads_fail_closed() {
        let mut short = ScanController::<1>::new(0x1000);
        short.start(13, 0, 1_000, 100).unwrap();
        let dc = advance_to_dc_probe(&mut short, 13, ESC_FEATURE_DC_SUPPORTED);
        short.accept(dc.token, 13, &[0; 4], 1, 8).unwrap();
        let times = short.next_action(9).unwrap().unwrap();
        assert_eq!(
            short.accept(times.token, 13, &[0; 15], 1, 10),
            Err(ScanError::PayloadLengthMismatch)
        );
        assert!(short.records().is_empty());

        let mut invalid_wkc = ScanController::<1>::new(0x1000);
        invalid_wkc.start(14, 0, 1_000, 100).unwrap();
        let dc = advance_to_dc_probe(&mut invalid_wkc, 14, ESC_FEATURE_DC_SUPPORTED);
        invalid_wkc.accept(dc.token, 14, &[0; 4], 1, 8).unwrap();
        let times = invalid_wkc.next_action(9).unwrap().unwrap();
        assert_eq!(
            invalid_wkc.accept(times.token, 14, &[0; 16], 0, 10),
            Err(ScanError::UnexpectedWorkingCounter)
        );

        let mut times_timeout = ScanController::<1>::new(0x1000);
        times_timeout.start(15, 0, 1_000, 10).unwrap();
        let dc = advance_to_dc_probe(&mut times_timeout, 15, ESC_FEATURE_DC_SUPPORTED);
        times_timeout.accept(dc.token, 15, &[0; 4], 1, 8).unwrap();
        let times = times_timeout.next_action(9).unwrap().unwrap();
        assert_eq!(
            times_timeout.timeout(times.token, times.deadline_ns),
            Err(ScanError::Timeout)
        );

        let mut dl_short = ScanController::<1>::new(0x1000);
        dl_short.start(16, 0, 1_000, 100).unwrap();
        let dl = advance_to_dl_status(&mut dl_short, 16);
        assert_eq!(
            dl_short.accept(dl.token, 16, &[0], 1, 8),
            Err(ScanError::PayloadLengthMismatch)
        );

        let mut dl_wkc = ScanController::<1>::new(0x1000);
        dl_wkc.start(17, 0, 1_000, 100).unwrap();
        let dl = advance_to_dl_status(&mut dl_wkc, 17);
        assert_eq!(
            dl_wkc.accept(dl.token, 17, &[0; 2], 0, 8),
            Err(ScanError::UnexpectedWorkingCounter)
        );

        let mut dl_timeout = ScanController::<1>::new(0x1000);
        dl_timeout.start(18, 0, 1_000, 10).unwrap();
        let dl = advance_to_dl_status(&mut dl_timeout, 18);
        assert_eq!(
            dl_timeout.timeout(dl.token, dl.deadline_ns),
            Err(ScanError::Timeout)
        );
        assert!(dl_timeout.records().is_empty());
    }
}
