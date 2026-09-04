//! Caller-driven startup orchestration for the minimum EtherCAT master.
//!
//! This layer deliberately does not own a thread or transport. It composes
//! the scan, SII and AL state machines into one bounded sequence that a
//! scheduler can submit through the existing control request pool.

use crate::al::{
    AlAction, AlError, AlPhase, AlProgress, AlTransitionController, AlTransitionRequest,
};
use crate::control::{
    ControlError, ControlRequestPool, MAX_CONTROL_PAYLOAD, RegisterOperation, RequestHandle,
    RequestState,
};
use crate::scan::{ScanAction, ScanController, ScanError, ScanPhase, ScanProgress};
use crate::sii::{SiiAction, SiiError, SiiIdentityReader, SiiPhase, SiiProgress};
use crate::slave::{EthercatState, SlaveIdentity, SlaveRecord, SlaveTable, SlaveTableError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpectedSlave {
    pub position: u16,
    pub station_address: u16,
    pub identity: SlaveIdentity,
}

impl ExpectedSlave {
    pub const EMPTY: Self = Self {
        position: 0,
        station_address: 0,
        identity: SlaveIdentity::EMPTY,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupConfig {
    pub scan_timeout_ns: u64,
    pub identity_timeout_ns: u64,
    pub transition_timeout_ns: u64,
    pub request_timeout_ns: u64,
    pub target_state: EthercatState,
}

impl StartupConfig {
    pub const fn new(target_state: EthercatState) -> Self {
        Self {
            scan_timeout_ns: 1_000_000_000,
            identity_timeout_ns: 1_000_000_000,
            transition_timeout_ns: 1_000_000_000,
            request_timeout_ns: 1_000_000,
            target_state,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupPhase {
    Idle,
    Scanning,
    ReadingIdentity,
    TransitioningAl,
    Ready,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupAction {
    Scan(ScanAction),
    Sii(SiiAction),
    Al(AlAction),
}

impl StartupAction {
    pub const fn token(self) -> u8 {
        match self {
            Self::Scan(action) => action.token,
            Self::Sii(action) => action.token,
            Self::Al(action) => action.token,
        }
    }

    pub const fn datagram_index(self) -> u8 {
        match self {
            Self::Scan(action) => action.datagram_index,
            Self::Sii(action) => action.datagram_index,
            Self::Al(action) => action.datagram_index,
        }
    }

    pub const fn generation(self) -> u16 {
        match self {
            Self::Scan(action) => action.generation,
            Self::Sii(action) => action.generation,
            Self::Al(action) => action.generation,
        }
    }

    pub const fn address(self) -> u32 {
        match self {
            Self::Scan(action) => action.address,
            Self::Sii(action) => action.address,
            Self::Al(action) => action.address,
        }
    }

    pub const fn operation(self) -> RegisterOperation {
        match self {
            Self::Scan(action) => action.operation,
            Self::Sii(action) => action.operation,
            Self::Al(action) => action.operation,
        }
    }

    pub fn payload(&self) -> &[u8] {
        match self {
            Self::Scan(action) => action.payload(),
            Self::Sii(action) => action.payload(),
            Self::Al(action) => action.payload(),
        }
    }

    pub const fn deadline_ns(self) -> u64 {
        match self {
            Self::Scan(action) => action.deadline_ns,
            Self::Sii(action) => action.deadline_ns,
            Self::Al(action) => action.deadline_ns,
        }
    }

    pub const fn expected_wkc(self) -> u16 {
        match self {
            Self::Scan(action) => action.expected_wkc,
            Self::Sii(action) => action.expected_wkc,
            Self::Al(action) => action.expected_wkc,
        }
    }

    pub const fn datagram_len(self) -> usize {
        match self {
            Self::Scan(action) => action.datagram_len(),
            Self::Sii(action) => action.datagram_len(),
            Self::Al(action) => action.datagram_len(),
        }
    }

    pub const fn response_len(self) -> usize {
        match self {
            Self::Scan(action) => action.read_len as usize,
            Self::Sii(action) => action.read_len as usize,
            Self::Al(action) => action.read_len as usize,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupProgress {
    Advanced,
    SlaveDiscovered(usize),
    IdentityVerified(usize),
    SlaveReady(usize),
    Ready,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupError {
    Busy,
    NotStarted,
    NoPendingAction,
    CapacityExceeded,
    DuplicateExpectedPosition,
    ExpectedCountMismatch,
    MissingExpectedPosition,
    StationAddressMismatch,
    IdentityMismatch,
    ActionMismatch,
    UnknownState,
    AlErrorCode(u16),
    Control(ControlError),
    Scan(ScanError),
    Sii(SiiError),
    Al(AlError),
    Table(SlaveTableError),
}

pub struct StartupController<const MAX_SLAVES: usize> {
    phase: StartupPhase,
    config: StartupConfig,
    generation: u16,
    station_address_base: u16,
    expected: [ExpectedSlave; MAX_SLAVES],
    expected_count: usize,
    scan: ScanController<MAX_SLAVES>,
    sii: SiiIdentityReader,
    al: AlTransitionController,
    table: SlaveTable<MAX_SLAVES>,
    current_index: usize,
    last_error: Option<StartupError>,
}

impl<const MAX_SLAVES: usize> StartupController<MAX_SLAVES> {
    pub const fn new(station_address_base: u16) -> Self {
        Self {
            phase: StartupPhase::Idle,
            config: StartupConfig::new(EthercatState::Op),
            generation: 0,
            station_address_base,
            expected: [ExpectedSlave::EMPTY; MAX_SLAVES],
            expected_count: 0,
            scan: ScanController::new(station_address_base),
            sii: SiiIdentityReader::new(),
            al: AlTransitionController::new(),
            table: SlaveTable::new(),
            current_index: 0,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> StartupPhase {
        self.phase
    }

    pub const fn current_index(&self) -> usize {
        self.current_index
    }

    pub const fn last_error(&self) -> Option<StartupError> {
        self.last_error
    }

    pub const fn expected_count(&self) -> usize {
        self.expected_count
    }

    pub fn records(&self) -> &[SlaveRecord] {
        self.table.records()
    }

    pub fn scan_records(&self) -> &[crate::scan::ScanRecord] {
        self.scan.records()
    }

    pub fn pending_action(&self) -> Option<StartupAction> {
        match self.phase {
            StartupPhase::Scanning => self.scan.pending().map(StartupAction::Scan),
            StartupPhase::ReadingIdentity => self.sii.pending().map(StartupAction::Sii),
            StartupPhase::TransitioningAl => self.al.pending().map(StartupAction::Al),
            StartupPhase::Idle | StartupPhase::Ready | StartupPhase::Faulted => None,
        }
    }

    pub fn start(
        &mut self,
        generation: u16,
        now_ns: u64,
        config: StartupConfig,
        expected: &[ExpectedSlave],
    ) -> Result<(), StartupError> {
        if !matches!(
            self.phase,
            StartupPhase::Idle | StartupPhase::Ready | StartupPhase::Faulted
        ) {
            return Err(StartupError::Busy);
        }
        if expected.len() > MAX_SLAVES || MAX_SLAVES == 0 {
            return Err(StartupError::CapacityExceeded);
        }
        if matches!(config.target_state, EthercatState::Unknown) {
            return Err(StartupError::UnknownState);
        }
        for (index, item) in expected.iter().copied().enumerate() {
            if expected[..index]
                .iter()
                .any(|existing| existing.position == item.position)
            {
                return Err(StartupError::DuplicateExpectedPosition);
            }
        }

        self.phase = StartupPhase::Scanning;
        self.config = config;
        self.generation = generation;
        self.expected = [ExpectedSlave::EMPTY; MAX_SLAVES];
        self.expected[..expected.len()].copy_from_slice(expected);
        self.expected_count = expected.len();
        self.scan = ScanController::new(self.station_address_base);
        self.sii = SiiIdentityReader::new();
        self.al = AlTransitionController::new();
        self.table = SlaveTable::new();
        self.current_index = 0;
        self.last_error = None;
        match self.scan.start(
            generation,
            now_ns,
            config.scan_timeout_ns,
            config.request_timeout_ns,
        ) {
            Ok(()) => Ok(()),
            Err(error) => self.fail(StartupError::Scan(error)),
        }
    }

    pub fn next_action(&mut self, now_ns: u64) -> Result<Option<StartupAction>, StartupError> {
        loop {
            match self.phase {
                StartupPhase::Scanning => match self.scan.next_action(now_ns) {
                    Ok(Some(action)) => return Ok(Some(StartupAction::Scan(action))),
                    Ok(None) if self.scan.phase() == ScanPhase::Complete => {
                        self.enter_identity_phase()?;
                    }
                    Ok(None) => return Ok(None),
                    Err(error) => return self.fail(StartupError::Scan(error)),
                },
                StartupPhase::ReadingIdentity => {
                    self.start_identity_reader(now_ns)?;
                    match self.sii.next_action(now_ns) {
                        Ok(Some(action)) => return Ok(Some(StartupAction::Sii(action))),
                        Ok(None) => return Ok(None),
                        Err(error) => return self.fail(StartupError::Sii(error)),
                    }
                }
                StartupPhase::TransitioningAl => match self.al.next_action(now_ns) {
                    Ok(Some(action)) => return Ok(Some(StartupAction::Al(action))),
                    Ok(None) => return Ok(None),
                    Err(error) => return self.fail(StartupError::Al(error)),
                },
                StartupPhase::Ready | StartupPhase::Faulted | StartupPhase::Idle => {
                    return Ok(None);
                }
            }
        }
    }

    pub fn enqueue_pending<const REQUESTS: usize>(
        &self,
        pool: &mut ControlRequestPool<REQUESTS>,
    ) -> Result<RequestHandle, ControlError> {
        let action = self.pending_action().ok_or(ControlError::InvalidState)?;
        pool.acquire_with_response_len(
            action.datagram_index(),
            action.generation(),
            action.address(),
            action.operation(),
            action.payload(),
            action.datagram_len(),
            action.deadline_ns(),
        )
    }

    pub fn accept(
        &mut self,
        action: StartupAction,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<StartupProgress, StartupError> {
        if self.pending_action() != Some(action) {
            return self.fail(StartupError::ActionMismatch);
        }
        match action {
            StartupAction::Scan(action) => {
                if self.phase != StartupPhase::Scanning {
                    return self.fail(StartupError::NoPendingAction);
                }
                let progress = match self.scan.accept(
                    action.token,
                    generation,
                    payload,
                    working_counter,
                    now_ns,
                ) {
                    Ok(progress) => progress,
                    Err(error) => return self.fail(StartupError::Scan(error)),
                };
                if self.scan.phase() == ScanPhase::Complete {
                    self.enter_identity_phase()?;
                }
                Ok(match progress {
                    ScanProgress::DeviceDiscovered(index) => {
                        StartupProgress::SlaveDiscovered(index)
                    }
                    ScanProgress::Advanced | ScanProgress::Complete => StartupProgress::Advanced,
                })
            }
            StartupAction::Sii(action) => {
                if self.phase != StartupPhase::ReadingIdentity {
                    return self.fail(StartupError::NoPendingAction);
                }
                let progress = match self.sii.accept(
                    action.token,
                    generation,
                    payload,
                    working_counter,
                    now_ns,
                ) {
                    Ok(progress) => progress,
                    Err(error) => return self.fail(StartupError::Sii(error)),
                };
                if progress == SiiProgress::Complete {
                    return self.finish_identity(now_ns);
                }
                Ok(StartupProgress::Advanced)
            }
            StartupAction::Al(action) => {
                if self.phase != StartupPhase::TransitioningAl {
                    return self.fail(StartupError::NoPendingAction);
                }
                let progress =
                    match self
                        .al
                        .accept(action.token, generation, payload, working_counter, now_ns)
                    {
                        Ok(progress) => progress,
                        Err(error) => return self.fail(StartupError::Al(error)),
                    };
                match progress {
                    AlProgress::Reached(_) => self.finish_al_step(now_ns),
                    AlProgress::ControlWritten | AlProgress::Polling => {
                        Ok(StartupProgress::Advanced)
                    }
                }
            }
        }
    }

    /// Consume a completed control-plane request and advance the matching
    /// startup FSM. The wire response retains the full EtherCAT data area;
    /// write actions intentionally pass an empty response to their FSM.
    pub fn accept_completed<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
        handle: RequestHandle,
        now_ns: u64,
    ) -> Result<StartupProgress, StartupError> {
        let action = match self.pending_action() {
            Some(action) => action,
            None => return self.fail(StartupError::NoPendingAction),
        };
        let (generation, actual_wkc, wire_length, response) = match pool.get(handle) {
            Some(request) if request.state == RequestState::Complete => {
                if request.datagram_index != action.datagram_index()
                    || request.generation != action.generation()
                    || request.address != action.address()
                    || request.response_length != action.datagram_len()
                    || request.length < action.response_len()
                {
                    let _ = pool.release(handle);
                    return self.fail(StartupError::ActionMismatch);
                }
                let mut response = [0; MAX_CONTROL_PAYLOAD];
                response[..request.length].copy_from_slice(request.payload());
                (
                    request.generation,
                    request.actual_wkc,
                    request.length,
                    response,
                )
            }
            Some(request) if request.state == RequestState::Failed => {
                let _ = pool.release(handle);
                return self.fail(StartupError::Control(ControlError::InvalidState));
            }
            Some(_) => return Err(StartupError::Control(ControlError::InvalidState)),
            None => return self.fail(StartupError::Control(ControlError::InvalidHandle)),
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
            (Ok(progress), Ok(())) => {
                debug_assert!(wire_length >= action.response_len());
                Ok(progress)
            }
            (Ok(_), Err(error)) => self.fail(StartupError::Control(error)),
            (Err(error), _) => Err(error),
        }
    }

    pub fn timeout(
        &mut self,
        action: StartupAction,
        now_ns: u64,
    ) -> Result<StartupProgress, StartupError> {
        if self.pending_action() != Some(action) {
            return self.fail(StartupError::ActionMismatch);
        }
        match action {
            StartupAction::Scan(action) => {
                let progress = match self.scan.timeout(action.token, now_ns) {
                    Ok(progress) => progress,
                    Err(error) => {
                        if self.scan.phase() == ScanPhase::Faulted {
                            return self.fail(StartupError::Scan(error));
                        }
                        return Err(StartupError::Scan(error));
                    }
                };
                if progress == ScanProgress::Complete {
                    self.enter_identity_phase()?;
                }
                Ok(StartupProgress::Advanced)
            }
            StartupAction::Sii(action) => match self.sii.timeout(action.token, now_ns) {
                Ok(()) => self.fail(StartupError::Sii(SiiError::Timeout)),
                Err(error) => {
                    if self.sii.phase() == SiiPhase::Faulted {
                        self.fail(StartupError::Sii(error))
                    } else {
                        Err(StartupError::Sii(error))
                    }
                }
            },
            StartupAction::Al(action) => match self.al.timeout(action.token, now_ns) {
                Ok(()) => self.fail(StartupError::Al(AlError::Timeout)),
                Err(error) => {
                    if self.al.phase() == AlPhase::Faulted {
                        self.fail(StartupError::Al(error))
                    } else {
                        Err(StartupError::Al(error))
                    }
                }
            },
        }
    }

    fn enter_identity_phase(&mut self) -> Result<(), StartupError> {
        if self.scan.len() != self.expected_count {
            return self.fail(StartupError::ExpectedCountMismatch);
        }
        if self.expected_count == 0 {
            self.phase = StartupPhase::Ready;
            return Ok(());
        }
        self.current_index = 0;
        self.phase = StartupPhase::ReadingIdentity;
        Ok(())
    }

    fn start_identity_reader(&mut self, now_ns: u64) -> Result<(), StartupError> {
        if !matches!(
            self.sii.phase(),
            SiiPhase::Idle | SiiPhase::Complete | SiiPhase::Faulted
        ) {
            return Ok(());
        }
        let record = self
            .scan
            .records()
            .get(self.current_index)
            .copied()
            .ok_or(StartupError::ExpectedCountMismatch)?;
        match self.sii.start(
            record.station_address,
            self.generation,
            now_ns,
            self.config.identity_timeout_ns,
            self.config.request_timeout_ns,
        ) {
            Ok(()) => Ok(()),
            Err(error) => self.fail(StartupError::Sii(error)),
        }
    }

    fn finish_identity(&mut self, now_ns: u64) -> Result<StartupProgress, StartupError> {
        let scan_record = self
            .scan
            .records()
            .get(self.current_index)
            .copied()
            .ok_or(StartupError::ExpectedCountMismatch)?;
        let expected = self
            .expected
            .iter()
            .take(self.expected_count)
            .find(|item| item.position == scan_record.position)
            .copied()
            .ok_or(StartupError::MissingExpectedPosition)?;
        if expected.station_address != scan_record.station_address {
            return self.fail(StartupError::StationAddressMismatch);
        }
        let identity = self.sii.identity().ok_or(StartupError::IdentityMismatch)?;
        if !identity.matches(expected.identity) {
            return self.fail(StartupError::IdentityMismatch);
        }
        if let Err(error) =
            self.table
                .add(scan_record.position, scan_record.station_address, identity)
        {
            return self.fail(StartupError::Table(error));
        }
        if let Err(error) =
            self.table
                .observe_status(scan_record.position, scan_record.al_status, 0)
        {
            return self.fail(StartupError::Table(error));
        }
        if let Err(error) = self
            .table
            .verify_identity(scan_record.position, expected.identity)
        {
            return self.fail(StartupError::Table(error));
        }
        self.phase = StartupPhase::TransitioningAl;
        self.start_al_for_current(now_ns)
    }

    fn start_al_for_current(&mut self, now_ns: u64) -> Result<StartupProgress, StartupError> {
        let record = self
            .table
            .records()
            .get(self.current_index)
            .copied()
            .ok_or(StartupError::ExpectedCountMismatch)?;
        if record.al_status.error {
            return self.fail(StartupError::AlErrorCode(record.al_status.code));
        }
        if let Err(error) = self.al.start(AlTransitionRequest {
            station_address: record.station_address,
            current_state: record.al_status.state,
            requested_state: self.config.target_state,
            generation: self.generation,
            now_ns,
            timeout_ns: self.config.transition_timeout_ns,
            request_timeout_ns: self.config.request_timeout_ns,
        }) {
            return self.fail(StartupError::Al(error));
        }
        if self.al.phase() == AlPhase::Complete {
            self.finish_al_step(now_ns)
        } else {
            Ok(StartupProgress::IdentityVerified(self.current_index))
        }
    }

    fn finish_al_step(&mut self, now_ns: u64) -> Result<StartupProgress, StartupError> {
        let position = self
            .table
            .records()
            .get(self.current_index)
            .copied()
            .ok_or(StartupError::ExpectedCountMismatch)?
            .position;
        if let Err(error) = self
            .table
            .observe_status(position, self.al.observed_status(), 0)
        {
            return self.fail(StartupError::Table(error));
        }
        let status = self.al.observed_status();
        if status.error {
            return self.fail(StartupError::AlErrorCode(status.code));
        }
        if status.state != self.config.target_state {
            return self.start_al_for_current(now_ns);
        }

        let ready_index = self.current_index;
        self.current_index += 1;
        if self.current_index >= self.expected_count {
            self.phase = StartupPhase::Ready;
            Ok(StartupProgress::Ready)
        } else {
            self.phase = StartupPhase::ReadingIdentity;
            Ok(StartupProgress::SlaveReady(ready_index))
        }
    }

    fn fail<T>(&mut self, error: StartupError) -> Result<T, StartupError> {
        self.last_error = Some(error);
        self.phase = StartupPhase::Faulted;
        Err(error)
    }
}

impl<const MAX_SLAVES: usize> Default for StartupController<MAX_SLAVES> {
    fn default() -> Self {
        Self::new(0x1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registers::{ESC_AL_STATUS, ESC_TYPE, auto_increment_address, fixed_address};

    fn status(state: EthercatState) -> [u8; 6] {
        let mut bytes = [0; 6];
        bytes[0..2].copy_from_slice(&(state as u16).to_le_bytes());
        bytes
    }

    fn accept_action<const MAX_SLAVES: usize>(
        startup: &mut StartupController<MAX_SLAVES>,
        action: StartupAction,
        payload: &[u8],
        wkc: u16,
        now_ns: u64,
    ) -> StartupProgress {
        startup
            .accept(action, action.generation(), payload, wkc, now_ns)
            .unwrap()
    }

    #[test]
    fn startup_scans_verifies_identity_and_reaches_operational() {
        let identity = SlaveIdentity {
            vendor_id: 0x1122_3344,
            product_code: 0x5566_7788,
            revision: 0x99AA_BBCC,
            serial: 0xDDEE_FF00,
        };
        let expected = [ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity,
        }];
        let mut startup = StartupController::<2>::new(0x1000);
        startup
            .start(7, 0, StartupConfig::new(EthercatState::Op), &expected)
            .unwrap();

        let probe = startup.next_action(1).unwrap().unwrap();
        assert!(matches!(probe, StartupAction::Scan(_)));
        assert_eq!(probe.address(), auto_increment_address(0, ESC_TYPE));
        accept_action(&mut startup, probe, &[0x88, 0x02], 1, 2);

        let basic = startup.next_action(3).unwrap().unwrap();
        accept_action(
            &mut startup,
            basic,
            &[0x88, 0x02, 3, 4, 1, 2, 0x00, 0x20, 1],
            1,
            4,
        );
        let assign = startup.next_action(5).unwrap().unwrap();
        accept_action(&mut startup, assign, &[], 1, 6);
        let scan_status = startup.next_action(7).unwrap().unwrap();
        assert_eq!(scan_status.address(), fixed_address(0x1000, ESC_AL_STATUS));
        accept_action(
            &mut startup,
            scan_status,
            &status(EthercatState::SafeOp),
            1,
            8,
        );
        let end_probe = startup.next_action(9).unwrap().unwrap();
        assert_eq!(
            startup
                .timeout(end_probe, end_probe_deadline(end_probe))
                .unwrap(),
            StartupProgress::Advanced
        );
        assert_eq!(startup.phase(), StartupPhase::ReadingIdentity);

        for word in [
            0x3344u16, 0x1122, 0x7788, 0x5566, 0xBBCC, 0x99AA, 0xFF00, 0xDDEE,
        ] {
            let address = startup.next_action(10).unwrap().unwrap();
            accept_action(&mut startup, address, &[], 1, 11);
            let issue = startup.next_action(12).unwrap().unwrap();
            accept_action(&mut startup, issue, &[], 1, 13);
            let poll = startup.next_action(14).unwrap().unwrap();
            accept_action(&mut startup, poll, &[0, 0], 1, 15);
            let data = startup.next_action(16).unwrap().unwrap();
            accept_action(&mut startup, data, &word.to_le_bytes(), 1, 17);
        }

        assert_eq!(startup.phase(), StartupPhase::TransitioningAl);
        let write = startup.next_action(18).unwrap().unwrap();
        accept_action(&mut startup, write, &[], 1, 19);
        let read = startup.next_action(20).unwrap().unwrap();
        assert_eq!(read.address(), fixed_address(0x1000, ESC_AL_STATUS));
        assert_eq!(
            accept_action(&mut startup, read, &status(EthercatState::Op), 1, 21),
            StartupProgress::Ready
        );
        assert_eq!(startup.phase(), StartupPhase::Ready);
        assert_eq!(startup.records().len(), 1);
        assert_eq!(startup.records()[0].identity, identity);
        assert_eq!(startup.records()[0].al_status.state, EthercatState::Op);
    }

    fn end_probe_deadline(action: StartupAction) -> u64 {
        match action {
            StartupAction::Scan(action) => action.deadline_ns,
            StartupAction::Sii(_) | StartupAction::Al(_) => 0,
        }
    }

    #[test]
    fn startup_rejects_identity_and_topology_mismatch_before_al() {
        let expected = [ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity: SlaveIdentity {
                vendor_id: 9,
                product_code: 9,
                revision: 9,
                serial: 0,
            },
        }];
        let mut startup = StartupController::<2>::new(0x1000);
        startup
            .start(1, 0, StartupConfig::new(EthercatState::Op), &expected)
            .unwrap();
        let probe = startup.next_action(1).unwrap().unwrap();
        accept_action(&mut startup, probe, &[0x01, 0x00], 1, 2);
        let basic = startup.next_action(3).unwrap().unwrap();
        accept_action(&mut startup, basic, &[0; 9], 1, 4);
        let assign = startup.next_action(5).unwrap().unwrap();
        accept_action(&mut startup, assign, &[], 1, 6);
        let scan_status_action = startup.next_action(7).unwrap().unwrap();
        accept_action(
            &mut startup,
            scan_status_action,
            &status(EthercatState::SafeOp),
            1,
            8,
        );
        let end_probe = startup.next_action(9).unwrap().unwrap();
        startup
            .timeout(end_probe, end_probe_deadline(end_probe))
            .unwrap();
        for word in 0..8 {
            let address = startup.next_action(10).unwrap().unwrap();
            accept_action(&mut startup, address, &[], 1, 11);
            let issue = startup.next_action(12).unwrap().unwrap();
            accept_action(&mut startup, issue, &[], 1, 13);
            let poll = startup.next_action(14).unwrap().unwrap();
            accept_action(&mut startup, poll, &[0, 0], 1, 15);
            let data = startup.next_action(16).unwrap().unwrap();
            let value = (word as u16).to_le_bytes();
            if word == 7 {
                assert_eq!(
                    startup.accept(data, data.generation(), &value, 1, 17),
                    Err(StartupError::IdentityMismatch)
                );
            } else {
                accept_action(&mut startup, data, &value, 1, 17);
            }
        }
        assert_eq!(startup.phase(), StartupPhase::Faulted);
        assert_eq!(startup.next_action(18), Ok(None));
    }

    #[test]
    fn stale_startup_action_faults_the_lifecycle() {
        let expected = [ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity: SlaveIdentity::EMPTY,
        }];
        let mut startup = StartupController::<2>::new(0x1000);
        startup
            .start(3, 0, StartupConfig::new(EthercatState::PreOp), &expected)
            .unwrap();
        let action = startup.next_action(1).unwrap().unwrap();
        let mut stale_action = action;
        if let StartupAction::Scan(scan_action) = &mut stale_action {
            scan_action.datagram_index = scan_action.datagram_index.wrapping_add(1);
        }
        assert_eq!(
            startup.accept(stale_action, action.generation(), &[], 1, 2),
            Err(StartupError::ActionMismatch)
        );
        assert_eq!(startup.phase(), StartupPhase::Faulted);
        assert_eq!(startup.last_error(), Some(StartupError::ActionMismatch));
        assert_eq!(startup.next_action(3), Ok(None));
    }

    #[test]
    fn completed_control_request_advances_startup_and_releases_pool_slot() {
        let expected = [ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity: SlaveIdentity::EMPTY,
        }];
        let mut startup = StartupController::<2>::new(0x1000);
        startup
            .start(3, 0, StartupConfig::new(EthercatState::PreOp), &expected)
            .unwrap();
        let action = startup.next_action(1).unwrap().unwrap();
        assert!(matches!(action, StartupAction::Scan(_)));

        let mut pool = ControlRequestPool::<1>::new();
        let handle = startup.enqueue_pending(&mut pool).unwrap();
        let request = pool.get(handle).unwrap();
        assert_eq!(request.length, 2);
        assert_eq!(request.response_length, 2);
        assert_eq!(request.payload(), &[0, 0]);

        let mut frame = [0; crate::wire::MAX_ETHERNET_FRAME_LEN];
        pool.build_into_buffer(handle, &mut frame, [0xFF; 6], [1, 2, 3, 4, 5, 6])
            .unwrap();
        pool.complete(
            handle,
            action.generation(),
            action.address(),
            &[0x88, 0x02],
            1,
        )
        .unwrap();
        assert_eq!(
            startup.accept_completed(&mut pool, handle, 2),
            Ok(StartupProgress::Advanced)
        );
        assert_eq!(pool.in_use(), 0);
        assert_eq!(startup.phase(), StartupPhase::Scanning);
    }
}
