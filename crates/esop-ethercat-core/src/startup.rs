//! Caller-driven startup orchestration for the minimum EtherCAT master.
//!
//! This layer deliberately does not own a thread or transport. It composes
//! the scan, SII and AL state machines into one bounded sequence that a
//! scheduler can submit through the existing control request pool.

use crate::al::{
    AlAction, AlError, AlErrorAcknowledgePolicy, AlErrorAcknowledgeStatus, AlPhase, AlProgress,
    AlTransitionController, AlTransitionRequest,
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
pub struct StartupConfigurationServices(u8);

impl StartupConfigurationServices {
    const PDO_CONFIGURATION: u8 = 1 << 0;
    const MAPPING: u8 = 1 << 1;
    const DC_CONFIGURATION: u8 = 1 << 2;

    pub const NONE: Self = Self(0);

    pub const fn new() -> Self {
        Self::NONE
    }

    pub const fn with_pdo_configuration(mut self) -> Self {
        self.0 |= Self::PDO_CONFIGURATION;
        self
    }

    pub const fn with_mapping(mut self) -> Self {
        self.0 |= Self::MAPPING;
        self
    }

    pub const fn with_dc_configuration(mut self) -> Self {
        self.0 |= Self::DC_CONFIGURATION;
        self
    }

    pub const fn requires_pdo_configuration(self) -> bool {
        self.0 & Self::PDO_CONFIGURATION != 0
    }

    pub const fn requires_mapping(self) -> bool {
        self.0 & Self::MAPPING != 0
    }

    pub const fn requires_dc_configuration(self) -> bool {
        self.0 & Self::DC_CONFIGURATION != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl Default for StartupConfigurationServices {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupConfig {
    pub scan_timeout_ns: u64,
    pub identity_timeout_ns: u64,
    pub transition_timeout_ns: u64,
    pub request_timeout_ns: u64,
    pub target_state: EthercatState,
    pub configuration_services: StartupConfigurationServices,
}

impl StartupConfig {
    pub const fn new(target_state: EthercatState) -> Self {
        Self {
            scan_timeout_ns: 1_000_000_000,
            identity_timeout_ns: 1_000_000_000,
            transition_timeout_ns: 1_000_000_000,
            request_timeout_ns: 1_000_000,
            target_state,
            configuration_services: StartupConfigurationServices::NONE,
        }
    }

    pub const fn with_configuration_services(
        mut self,
        configuration_services: StartupConfigurationServices,
    ) -> Self {
        self.configuration_services = configuration_services;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupPhase {
    Idle,
    Scanning,
    ReadingIdentity,
    TransitioningAl,
    AwaitingConfiguration,
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
    AwaitingConfiguration,
    ConfigurationReleased,
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
    InvalidConfigurationBarrier,
    ConfigurationNotPending,
    AlErrorCode(u16),
    Control(ControlError),
    Scan(ScanError),
    Sii(SiiError),
    Al(AlError),
    Table(SlaveTableError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupAlFault {
    pub position: u16,
    pub station_address: u16,
    pub requested_state: EthercatState,
    pub actual_state: EthercatState,
    pub status_code: u16,
    pub device_emulation: bool,
    pub acknowledgement: AlErrorAcknowledgeStatus,
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
    device_emulation: [bool; MAX_SLAVES],
    current_index: usize,
    stage_target: EthercatState,
    configuration_released: bool,
    last_error: Option<StartupError>,
    last_al_fault: Option<StartupAlFault>,
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
            device_emulation: [false; MAX_SLAVES],
            current_index: 0,
            stage_target: EthercatState::Op,
            configuration_released: false,
            last_error: None,
            last_al_fault: None,
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

    pub const fn last_al_fault(&self) -> Option<StartupAlFault> {
        self.last_al_fault
    }

    pub fn device_emulation(&self, position: u16) -> Option<bool> {
        self.table
            .records()
            .iter()
            .position(|record| record.position == position)
            .map(|index| self.device_emulation[index])
    }

    pub const fn expected_count(&self) -> usize {
        self.expected_count
    }

    pub const fn configuration_services(&self) -> StartupConfigurationServices {
        self.config.configuration_services
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
            StartupPhase::Idle
            | StartupPhase::AwaitingConfiguration
            | StartupPhase::Ready
            | StartupPhase::Faulted => None,
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
            StartupPhase::Idle
                | StartupPhase::AwaitingConfiguration
                | StartupPhase::Ready
                | StartupPhase::Faulted
        ) {
            return Err(StartupError::Busy);
        }
        if expected.len() > MAX_SLAVES || MAX_SLAVES == 0 {
            return Err(StartupError::CapacityExceeded);
        }
        if matches!(config.target_state, EthercatState::Unknown) {
            return Err(StartupError::UnknownState);
        }
        if !config.configuration_services.is_empty()
            && (expected.is_empty()
                || !matches!(
                    config.target_state,
                    EthercatState::SafeOp | EthercatState::Op
                ))
        {
            return Err(StartupError::InvalidConfigurationBarrier);
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
        self.device_emulation = [false; MAX_SLAVES];
        self.current_index = 0;
        self.stage_target = if config.configuration_services.is_empty() {
            config.target_state
        } else {
            EthercatState::PreOp
        };
        self.configuration_released = false;
        self.last_error = None;
        self.last_al_fault = None;
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
                    Err(error) => {
                        self.capture_al_fault();
                        return self.fail(StartupError::Al(error));
                    }
                },
                StartupPhase::AwaitingConfiguration
                | StartupPhase::Ready
                | StartupPhase::Faulted
                | StartupPhase::Idle => {
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
                        Err(error) => {
                            self.capture_al_fault();
                            return self.fail(StartupError::Al(error));
                        }
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
                if !request.matches_action(
                    action.datagram_index(),
                    action.generation(),
                    action.address(),
                    action.operation(),
                    action.payload(),
                    action.datagram_len(),
                    action.deadline_ns(),
                ) || request.length < action.response_len()
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
                if !request.matches_action(
                    action.datagram_index(),
                    action.generation(),
                    action.address(),
                    action.operation(),
                    action.payload(),
                    action.datagram_len(),
                    action.deadline_ns(),
                ) {
                    let _ = pool.release(handle);
                    return self.fail(StartupError::ActionMismatch);
                }
                let error = request.last_error().unwrap_or(ControlError::InvalidState);
                if let Err(release_error) = pool.release(handle) {
                    return self.fail(StartupError::Control(release_error));
                }
                if error == ControlError::Timeout {
                    return self.timeout(action, now_ns);
                }
                return self.fail(StartupError::Control(error));
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
                        self.capture_al_fault();
                        self.fail(StartupError::Al(error))
                    } else {
                        Err(StartupError::Al(error))
                    }
                }
            },
        }
    }

    pub(crate) fn release_configuration(
        &mut self,
        now_ns: u64,
    ) -> Result<StartupProgress, StartupError> {
        if self.phase != StartupPhase::AwaitingConfiguration {
            return self.fail(StartupError::ConfigurationNotPending);
        }
        self.configuration_released = true;
        self.stage_target = self.config.target_state;
        self.current_index = 0;
        self.al = AlTransitionController::new();
        self.phase = StartupPhase::TransitioningAl;
        self.start_al_for_current(now_ns)?;
        Ok(StartupProgress::ConfigurationReleased)
    }

    #[cfg(test)]
    pub(crate) fn enter_configuration_barrier_for_test(
        &mut self,
        generation: u16,
        config: StartupConfig,
        expected: &[ExpectedSlave],
    ) -> Result<(), StartupError> {
        if expected.is_empty()
            || expected.len() > MAX_SLAVES
            || config.configuration_services.is_empty()
            || !matches!(
                config.target_state,
                EthercatState::SafeOp | EthercatState::Op
            )
        {
            return Err(StartupError::InvalidConfigurationBarrier);
        }
        self.phase = StartupPhase::AwaitingConfiguration;
        self.config = config;
        self.expected = [ExpectedSlave::EMPTY; MAX_SLAVES];
        self.expected[..expected.len()].copy_from_slice(expected);
        self.expected_count = expected.len();
        self.generation = generation;
        self.scan = ScanController::new(self.station_address_base);
        self.sii = SiiIdentityReader::new();
        self.al = AlTransitionController::new();
        self.table = SlaveTable::new();
        self.device_emulation = [false; MAX_SLAVES];
        for item in expected.iter().copied() {
            self.table
                .add(item.position, item.station_address, item.identity)
                .map_err(StartupError::Table)?;
            self.table
                .observe_status(
                    item.position,
                    crate::slave::AlStatus::new(EthercatState::PreOp as u16, 0),
                    0,
                )
                .map_err(StartupError::Table)?;
            self.table
                .verify_identity(item.position, item.identity)
                .map_err(StartupError::Table)?;
        }
        self.current_index = expected.len();
        self.stage_target = EthercatState::PreOp;
        self.configuration_released = false;
        self.last_error = None;
        self.last_al_fault = None;
        Ok(())
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
        self.device_emulation[self.current_index] = scan_record.device_emulation;
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
        let policy = if self.device_emulation[self.current_index] {
            AlErrorAcknowledgePolicy::Disabled
        } else {
            AlErrorAcknowledgePolicy::Enabled
        };
        if let Err(error) = self.al.start_with_status(
            AlTransitionRequest {
                station_address: record.station_address,
                current_state: record.al_status.state,
                requested_state: self.stage_target,
                generation: self.generation,
                now_ns,
                timeout_ns: self.config.transition_timeout_ns,
                request_timeout_ns: self.config.request_timeout_ns,
            },
            record.al_status,
            policy,
        ) {
            self.capture_al_fault();
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
        if status.state != self.stage_target {
            return self.start_al_for_current(now_ns);
        }

        let ready_index = self.current_index;
        self.current_index += 1;
        if self.current_index >= self.expected_count {
            if !self.config.configuration_services.is_empty() && !self.configuration_released {
                self.phase = StartupPhase::AwaitingConfiguration;
                Ok(StartupProgress::AwaitingConfiguration)
            } else {
                self.phase = StartupPhase::Ready;
                Ok(StartupProgress::Ready)
            }
        } else if self.configuration_released {
            self.phase = StartupPhase::TransitioningAl;
            self.start_al_for_current(now_ns)
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

    fn capture_al_fault(&mut self) {
        if self.last_al_fault.is_some() {
            return;
        }
        let Some(fault) = self.al.fault_record() else {
            return;
        };
        let Some(record) = self.table.records().get(self.current_index).copied() else {
            return;
        };
        self.last_al_fault = Some(StartupAlFault {
            position: record.position,
            station_address: record.station_address,
            requested_state: fault.requested_state,
            actual_state: fault.status.state,
            status_code: fault.status.code,
            device_emulation: self.device_emulation[self.current_index],
            acknowledgement: fault.acknowledgement,
        });
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
    use crate::registers::{
        ESC_AL_STATUS, ESC_CONFIGURATION, ESC_TYPE, auto_increment_address, fixed_address,
    };
    use crate::slave::AL_ERROR_FLAG;

    fn status(state: EthercatState) -> [u8; 6] {
        let mut bytes = [0; 6];
        bytes[0..2].copy_from_slice(&(state as u16).to_le_bytes());
        bytes
    }

    fn status_with_code(state: EthercatState, error: bool, code: u16) -> [u8; 6] {
        let mut bytes = status(state);
        if error {
            bytes[0] |= 0x10;
        }
        bytes[4..6].copy_from_slice(&code.to_le_bytes());
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

    fn accept_al_state<const MAX_SLAVES: usize>(
        startup: &mut StartupController<MAX_SLAVES>,
        state: EthercatState,
        now_ns: u64,
    ) -> StartupProgress {
        let write = startup.next_action(now_ns).unwrap().unwrap();
        assert!(matches!(write, StartupAction::Al(_)));
        accept_action(startup, write, &[], 1, now_ns + 1);
        let read = startup.next_action(now_ns + 2).unwrap().unwrap();
        assert!(matches!(read, StartupAction::Al(_)));
        accept_action(startup, read, &status(state), 1, now_ns + 3)
    }

    fn accept_identity<const MAX_SLAVES: usize>(
        startup: &mut StartupController<MAX_SLAVES>,
        identity: SlaveIdentity,
        now_ns: &mut u64,
    ) {
        for word in [
            identity.vendor_id as u16,
            (identity.vendor_id >> 16) as u16,
            identity.product_code as u16,
            (identity.product_code >> 16) as u16,
            identity.revision as u16,
            (identity.revision >> 16) as u16,
            identity.serial as u16,
            (identity.serial >> 16) as u16,
        ] {
            let address = startup.next_action(*now_ns).unwrap().unwrap();
            accept_action(startup, address, &[], 1, *now_ns + 1);
            let issue = startup.next_action(*now_ns + 2).unwrap().unwrap();
            accept_action(startup, issue, &[], 1, *now_ns + 3);
            let poll = startup.next_action(*now_ns + 4).unwrap().unwrap();
            accept_action(startup, poll, &[0, 0], 1, *now_ns + 5);
            let data = startup.next_action(*now_ns + 6).unwrap().unwrap();
            accept_action(startup, data, &word.to_le_bytes(), 1, *now_ns + 7);
            *now_ns += 8;
        }
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
        assert_eq!(
            scan_status.address(),
            fixed_address(0x1000, ESC_CONFIGURATION)
        );
        accept_action(&mut startup, scan_status, &[0], 1, 8);
        let scan_status = startup.next_action(9).unwrap().unwrap();
        assert_eq!(scan_status.address(), fixed_address(0x1000, ESC_AL_STATUS));
        accept_action(
            &mut startup,
            scan_status,
            &status(EthercatState::SafeOp),
            1,
            10,
        );
        let end_probe = startup.next_action(11).unwrap().unwrap();
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
        assert_eq!(startup.device_emulation(0), Some(false));
        assert_eq!(startup.records()[0].identity, identity);
        assert_eq!(startup.records()[0].al_status.state, EthercatState::Op);
    }

    #[test]
    fn startup_enters_preop_barrier_and_resumes_without_rediscovery() {
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
        let requirements = StartupConfigurationServices::new().with_pdo_configuration();
        let mut startup = StartupController::<2>::new(0x1000);
        startup
            .start(
                7,
                0,
                StartupConfig::new(EthercatState::Op).with_configuration_services(requirements),
                &expected,
            )
            .unwrap();

        let probe = startup.next_action(1).unwrap().unwrap();
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
        accept_action(&mut startup, scan_status, &[0], 1, 8);
        let scan_status = startup.next_action(9).unwrap().unwrap();
        accept_action(
            &mut startup,
            scan_status,
            &status(EthercatState::Init),
            1,
            10,
        );
        let end_probe = startup.next_action(11).unwrap().unwrap();
        startup
            .timeout(end_probe, end_probe_deadline(end_probe))
            .unwrap();

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

        assert_eq!(startup.al.expected_state(), EthercatState::PreOp);
        assert_eq!(
            accept_al_state(&mut startup, EthercatState::PreOp, 20),
            StartupProgress::AwaitingConfiguration
        );
        assert_eq!(startup.phase(), StartupPhase::AwaitingConfiguration);
        assert_eq!(startup.next_action(24), Ok(None));
        assert_eq!(startup.records().len(), 1);
        assert_eq!(startup.records()[0].identity, identity);
        assert_eq!(startup.records()[0].al_status.state, EthercatState::PreOp);

        assert_eq!(
            startup.release_configuration(25),
            Ok(StartupProgress::ConfigurationReleased)
        );
        assert_eq!(startup.al.expected_state(), EthercatState::SafeOp);
        accept_al_state(&mut startup, EthercatState::SafeOp, 26);
        assert_eq!(startup.al.expected_state(), EthercatState::Op);
        assert_eq!(
            accept_al_state(&mut startup, EthercatState::Op, 30),
            StartupProgress::Ready
        );
        assert_eq!(startup.phase(), StartupPhase::Ready);
        assert_eq!(startup.records()[0].identity, identity);
        assert_eq!(startup.records()[0].al_status.state, EthercatState::Op);
    }

    #[test]
    fn every_expected_slave_reaches_preop_before_configuration_barrier() {
        let expected = [
            ExpectedSlave {
                position: 0,
                station_address: 0x1000,
                identity: SlaveIdentity {
                    vendor_id: 0x1122_3344,
                    product_code: 0x5566_7788,
                    revision: 0x99AA_BBCC,
                    serial: 0xDDEE_FF00,
                },
            },
            ExpectedSlave {
                position: 1,
                station_address: 0x1001,
                identity: SlaveIdentity {
                    vendor_id: 0x0102_0304,
                    product_code: 0x0506_0708,
                    revision: 0x090A_0B0C,
                    serial: 0x0D0E_0F10,
                },
            },
        ];
        let mut startup = StartupController::<3>::new(0x1000);
        startup
            .start(
                9,
                0,
                StartupConfig::new(EthercatState::Op).with_configuration_services(
                    StartupConfigurationServices::new().with_mapping(),
                ),
                &expected,
            )
            .unwrap();

        let mut now_ns = 1;
        for _ in expected {
            let probe = startup.next_action(now_ns).unwrap().unwrap();
            accept_action(&mut startup, probe, &[0x88, 0x02], 1, now_ns + 1);
            let basic = startup.next_action(now_ns + 2).unwrap().unwrap();
            accept_action(
                &mut startup,
                basic,
                &[0x88, 0x02, 3, 4, 1, 2, 0x00, 0x20, 1],
                1,
                now_ns + 3,
            );
            let assign = startup.next_action(now_ns + 4).unwrap().unwrap();
            accept_action(&mut startup, assign, &[], 1, now_ns + 5);
            let scan_status = startup.next_action(now_ns + 6).unwrap().unwrap();
            accept_action(&mut startup, scan_status, &[0], 1, now_ns + 7);
            let scan_status = startup.next_action(now_ns + 8).unwrap().unwrap();
            accept_action(
                &mut startup,
                scan_status,
                &status(EthercatState::Init),
                1,
                now_ns + 9,
            );
            now_ns += 10;
        }
        let end_probe = startup.next_action(now_ns).unwrap().unwrap();
        startup
            .timeout(end_probe, end_probe_deadline(end_probe))
            .unwrap();

        accept_identity(&mut startup, expected[0].identity, &mut now_ns);
        assert_eq!(startup.al.expected_state(), EthercatState::PreOp);
        assert_eq!(
            accept_al_state(&mut startup, EthercatState::PreOp, now_ns),
            StartupProgress::SlaveReady(0)
        );
        now_ns += 4;
        assert_eq!(startup.phase(), StartupPhase::ReadingIdentity);
        assert_eq!(startup.records().len(), 1);
        assert_eq!(startup.records()[0].al_status.state, EthercatState::PreOp);

        accept_identity(&mut startup, expected[1].identity, &mut now_ns);
        assert_eq!(startup.al.expected_state(), EthercatState::PreOp);
        assert_eq!(
            accept_al_state(&mut startup, EthercatState::PreOp, now_ns),
            StartupProgress::AwaitingConfiguration
        );
        assert_eq!(startup.phase(), StartupPhase::AwaitingConfiguration);
        assert_eq!(startup.records().len(), expected.len());
        assert!(
            startup
                .records()
                .iter()
                .all(|record| record.al_status.state == EthercatState::PreOp)
        );
    }

    #[test]
    fn configuration_release_transitions_every_retained_slave_through_safeop() {
        let expected = [
            ExpectedSlave {
                position: 0,
                station_address: 0x1000,
                identity: SlaveIdentity {
                    vendor_id: 1,
                    product_code: 2,
                    revision: 3,
                    serial: 4,
                },
            },
            ExpectedSlave {
                position: 1,
                station_address: 0x1001,
                identity: SlaveIdentity {
                    vendor_id: 5,
                    product_code: 6,
                    revision: 7,
                    serial: 8,
                },
            },
        ];
        let requirements = StartupConfigurationServices::new()
            .with_pdo_configuration()
            .with_mapping()
            .with_dc_configuration();
        let mut startup = StartupController::<2>::new(0x1000);
        startup
            .enter_configuration_barrier_for_test(
                21,
                StartupConfig::new(EthercatState::Op).with_configuration_services(requirements),
                &expected,
            )
            .unwrap();

        startup.release_configuration(0).unwrap();
        assert_eq!(startup.al.expected_state(), EthercatState::SafeOp);
        accept_al_state(&mut startup, EthercatState::SafeOp, 1);
        assert_eq!(startup.al.expected_state(), EthercatState::Op);
        accept_al_state(&mut startup, EthercatState::Op, 5);
        assert_eq!(startup.al.expected_state(), EthercatState::SafeOp);
        accept_al_state(&mut startup, EthercatState::SafeOp, 9);
        assert_eq!(startup.al.expected_state(), EthercatState::Op);
        assert_eq!(
            accept_al_state(&mut startup, EthercatState::Op, 13),
            StartupProgress::Ready
        );

        assert_eq!(startup.phase(), StartupPhase::Ready);
        assert_eq!(startup.records().len(), 2);
        for (record, item) in startup.records().iter().zip(expected) {
            assert_eq!(record.position, item.position);
            assert_eq!(record.station_address, item.station_address);
            assert_eq!(record.identity, item.identity);
            assert_eq!(record.al_status.state, EthercatState::Op);
        }
    }

    #[test]
    fn invalid_configuration_barrier_fails_before_startup_mutates() {
        let expected = [ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity: SlaveIdentity::EMPTY,
        }];
        let requirements = StartupConfigurationServices::new().with_mapping();
        let mut invalid_target = StartupController::<1>::new(0x1000);
        assert_eq!(
            invalid_target.start(
                1,
                0,
                StartupConfig::new(EthercatState::PreOp).with_configuration_services(requirements),
                &expected,
            ),
            Err(StartupError::InvalidConfigurationBarrier)
        );
        assert_eq!(invalid_target.phase(), StartupPhase::Idle);
        assert!(invalid_target.records().is_empty());

        let mut missing_slaves = StartupController::<1>::new(0x1000);
        assert_eq!(
            missing_slaves.start(
                1,
                0,
                StartupConfig::new(EthercatState::Op).with_configuration_services(requirements),
                &[],
            ),
            Err(StartupError::InvalidConfigurationBarrier)
        );
        assert_eq!(missing_slaves.phase(), StartupPhase::Idle);
    }

    #[test]
    fn restart_from_configuration_barrier_discards_retained_state() {
        let expected = [ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity: SlaveIdentity::EMPTY,
        }];
        let mut startup = StartupController::<1>::new(0x1000);
        startup
            .enter_configuration_barrier_for_test(
                1,
                StartupConfig::new(EthercatState::Op).with_configuration_services(
                    StartupConfigurationServices::new().with_mapping(),
                ),
                &expected,
            )
            .unwrap();
        assert_eq!(startup.records().len(), 1);

        startup
            .start(2, 10, StartupConfig::new(EthercatState::SafeOp), &expected)
            .unwrap();
        assert_eq!(startup.phase(), StartupPhase::Scanning);
        assert!(startup.records().is_empty());
        assert!(startup.configuration_services().is_empty());
        assert_eq!(startup.last_error(), None);
    }

    #[test]
    fn post_configuration_al_error_and_timeout_fail_closed() {
        let expected = [ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity: SlaveIdentity::EMPTY,
        }];
        let config = StartupConfig::new(EthercatState::Op).with_configuration_services(
            StartupConfigurationServices::new().with_dc_configuration(),
        );
        let mut al_error = StartupController::<1>::new(0x1000);
        al_error
            .enter_configuration_barrier_for_test(3, config, &expected)
            .unwrap();
        al_error.release_configuration(0).unwrap();
        let write = al_error.next_action(1).unwrap().unwrap();
        accept_action(&mut al_error, write, &[], 1, 2);
        let read = al_error.next_action(3).unwrap().unwrap();
        assert_eq!(
            al_error.accept(
                read,
                read.generation(),
                &status_with_code(EthercatState::PreOp, true, 0x001B),
                1,
                4,
            ),
            Ok(StartupProgress::Advanced)
        );
        let acknowledge = al_error.next_action(5).unwrap().unwrap();
        assert_eq!(acknowledge.payload(), &[0x12, 0x00]);
        accept_action(&mut al_error, acknowledge, &[], 1, 6);
        let verify = al_error.next_action(7).unwrap().unwrap();
        assert_eq!(
            al_error.accept(
                verify,
                verify.generation(),
                &status(EthercatState::PreOp),
                1,
                8,
            ),
            Err(StartupError::Al(AlError::AlErrorCode(0x001B)))
        );
        assert_eq!(al_error.phase(), StartupPhase::Faulted);
        assert_eq!(
            al_error.last_al_fault(),
            Some(StartupAlFault {
                position: 0,
                station_address: 0x1000,
                requested_state: EthercatState::Op,
                actual_state: EthercatState::PreOp,
                status_code: 0x001B,
                device_emulation: false,
                acknowledgement: AlErrorAcknowledgeStatus::Complete,
            })
        );

        let mut emulated = StartupController::<1>::new(0x1000);
        emulated
            .enter_configuration_barrier_for_test(5, config, &expected)
            .unwrap();
        emulated.device_emulation[0] = true;
        emulated.release_configuration(0).unwrap();
        let write = emulated.next_action(1).unwrap().unwrap();
        assert_eq!(write.payload()[0] & AL_ERROR_FLAG as u8, 0);
        accept_action(&mut emulated, write, &[], 1, 2);
        let read = emulated.next_action(3).unwrap().unwrap();
        assert_eq!(
            emulated.accept(
                read,
                read.generation(),
                &status_with_code(EthercatState::PreOp, true, 0x001B),
                1,
                4,
            ),
            Err(StartupError::Al(AlError::AlErrorCode(0x001B)))
        );
        assert_eq!(emulated.next_action(5), Ok(None));
        assert_eq!(
            emulated.last_al_fault().unwrap().acknowledgement,
            AlErrorAcknowledgeStatus::NotAttempted
        );

        let mut timeout = StartupController::<1>::new(0x1000);
        timeout
            .enter_configuration_barrier_for_test(4, config, &expected)
            .unwrap();
        timeout.release_configuration(0).unwrap();
        let action = timeout.next_action(1).unwrap().unwrap();
        assert_eq!(
            timeout.timeout(action, action.deadline_ns()),
            Err(StartupError::Al(AlError::Timeout))
        );
        assert_eq!(timeout.phase(), StartupPhase::Faulted);
    }

    #[test]
    fn scanned_device_emulation_suppresses_initial_error_acknowledgement() {
        let expected = [ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity: SlaveIdentity::EMPTY,
        }];
        let mut startup = StartupController::<2>::new(0x1000);
        startup
            .start(8, 0, StartupConfig::new(EthercatState::Op), &expected)
            .unwrap();

        let probe = startup.next_action(1).unwrap().unwrap();
        accept_action(&mut startup, probe, &[0x88, 0x02], 1, 2);
        let basic = startup.next_action(3).unwrap().unwrap();
        accept_action(&mut startup, basic, &[0; 9], 1, 4);
        let assign = startup.next_action(5).unwrap().unwrap();
        accept_action(&mut startup, assign, &[], 1, 6);
        let configuration = startup.next_action(7).unwrap().unwrap();
        assert_eq!(
            configuration.address(),
            fixed_address(0x1000, ESC_CONFIGURATION)
        );
        accept_action(
            &mut startup,
            configuration,
            &[crate::registers::ESC_DEVICE_EMULATION],
            1,
            8,
        );
        let status_action = startup.next_action(9).unwrap().unwrap();
        accept_action(
            &mut startup,
            status_action,
            &status_with_code(EthercatState::Init, true, 0x0011),
            1,
            10,
        );
        let end_probe = startup.next_action(11).unwrap().unwrap();
        startup
            .timeout(end_probe, end_probe_deadline(end_probe))
            .unwrap();

        let mut now_ns = 12;
        for word_index in 0..8 {
            let address = startup.next_action(now_ns).unwrap().unwrap();
            accept_action(&mut startup, address, &[], 1, now_ns + 1);
            let issue = startup.next_action(now_ns + 2).unwrap().unwrap();
            accept_action(&mut startup, issue, &[], 1, now_ns + 3);
            let poll = startup.next_action(now_ns + 4).unwrap().unwrap();
            accept_action(&mut startup, poll, &[0, 0], 1, now_ns + 5);
            let data = startup.next_action(now_ns + 6).unwrap().unwrap();
            if word_index == 7 {
                assert_eq!(
                    startup.accept(data, data.generation(), &[0, 0], 1, now_ns + 7),
                    Err(StartupError::Al(AlError::AlErrorCode(0x0011)))
                );
            } else {
                accept_action(&mut startup, data, &[0, 0], 1, now_ns + 7);
            }
            now_ns += 8;
        }

        assert_eq!(startup.phase(), StartupPhase::Faulted);
        assert_eq!(startup.device_emulation(0), Some(true));
        assert_eq!(startup.next_action(now_ns), Ok(None));
        assert_eq!(
            startup.last_al_fault(),
            Some(StartupAlFault {
                position: 0,
                station_address: 0x1000,
                requested_state: EthercatState::Op,
                actual_state: EthercatState::Init,
                status_code: 0x0011,
                device_emulation: true,
                acknowledgement: AlErrorAcknowledgeStatus::NotAttempted,
            })
        );

        startup
            .start(9, now_ns, StartupConfig::new(EthercatState::Op), &expected)
            .unwrap();
        assert_eq!(startup.last_al_fault(), None);
        assert_eq!(startup.device_emulation(0), None);
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
        accept_action(&mut startup, scan_status_action, &[0], 1, 8);
        let scan_status_action = startup.next_action(9).unwrap().unwrap();
        accept_action(
            &mut startup,
            scan_status_action,
            &status(EthercatState::SafeOp),
            1,
            10,
        );
        let end_probe = startup.next_action(11).unwrap().unwrap();
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

    #[test]
    fn expired_control_request_reaches_startup_as_timeout_and_releases_pool_slot() {
        let expected = [ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity: SlaveIdentity::EMPTY,
        }];
        let mut startup = StartupController::<2>::new(0x1000);
        startup
            .start(3, 0, StartupConfig::new(EthercatState::PreOp), &expected)
            .unwrap();
        let probe = startup.next_action(1).unwrap().unwrap();
        startup
            .accept(probe, probe.generation(), &[0x88, 0x02], 1, 2)
            .unwrap();
        let action = startup.next_action(3).unwrap().unwrap();

        let mut pool = ControlRequestPool::<1>::new();
        let handle = startup.enqueue_pending(&mut pool).unwrap();
        let mut frame = [0; crate::wire::MAX_ETHERNET_FRAME_LEN];
        pool.build_into_buffer(handle, &mut frame, [0xFF; 6], [1, 2, 3, 4, 5, 6])
            .unwrap();
        let expired_at = action.deadline_ns().saturating_add(1);
        assert!(pool.expire_in_flight(expired_at).contains(handle));

        assert_eq!(
            startup.accept_completed(&mut pool, handle, expired_at),
            Err(StartupError::Scan(ScanError::Timeout))
        );
        assert_eq!(pool.in_use(), 0);
        assert_eq!(startup.phase(), StartupPhase::Faulted);
    }
}
