//! Caller-driven startup orchestration for the minimum EtherCAT master.
//!
//! This layer deliberately does not own a thread or transport. It composes
//! the scan, SII and AL state machines into one bounded sequence that a
//! scheduler can submit through the existing control request pool.

use crate::al::{
    AlAction, AlError, AlErrorAcknowledgePolicy, AlErrorAcknowledgeStatus, AlPhase, AlProgress,
    AlTransitionController, AlTransitionRequest, AlTransitionTimeouts,
    ETG1020_DEFAULT_TRANSITION_TIMEOUTS_V1,
};
use crate::control::{
    ControlError, ControlRequestPool, MAX_CONTROL_PAYLOAD, RegisterOperation, RequestHandle,
    RequestState,
};
use crate::dc::{DcTopology, DcTopologyError};
use crate::mailbox::{MailboxConfig, MailboxConfigError};
use crate::rx_index::RxWorkingCounterPolicy;
use crate::scan::{
    ScanAction, ScanController, ScanDcCapabilities, ScanError, ScanPhase, ScanProgress,
};
use crate::sii::{
    SII_STANDARD_MAILBOX_WORD_COUNT, SII_STANDARD_RECEIVE_MAILBOX_OFFSET_WORD, SiiAction,
    SiiBlockReader, SiiBlockRequest, SiiError, SiiIdentityReader, SiiMailboxError, SiiPhase,
    SiiProgress, SiiStandardMailbox,
};
use crate::sii_config::{SiiConfigurationSignature, SiiConfigurationSignatureError};
use crate::sii_discovery::{
    SiiDiscoveryError, SiiDiscoveryPhase, SiiStreamDiscoveryController, SiiStreamDiscoveryRequest,
};
use crate::sii_stream::{SiiCategoryStreamProgress, SiiCategoryStreamRequest};
use crate::slave::{
    EthercatState, SlaveIdentity, SlaveRecord, SlaveTable, SlaveTableError, next_state,
};
use crate::{
    OpOnlyProfileError, OpOnlySyncManagerAction, OpOnlySyncManagerController,
    OpOnlySyncManagerError, OpOnlySyncManagerPhase, OpOnlySyncManagerProfile,
    OpOnlySyncManagerProgress,
};

pub const STARTUP_SII_IMAGE_WORD_CAPACITY: usize = 4096;
pub const STARTUP_SII_IMAGE_BYTE_CAPACITY: usize = STARTUP_SII_IMAGE_WORD_CAPACITY * 2;
pub const STARTUP_SII_PDO_ENTRY_CAPACITY: usize = 256;

type StartupSiiDiscovery = SiiStreamDiscoveryController<
    STARTUP_SII_IMAGE_WORD_CAPACITY,
    { crate::MAX_ESC_SYNC_MANAGERS },
    0,
    STARTUP_SII_PDO_ENTRY_CAPACITY,
    STARTUP_SII_PDO_ENTRY_CAPACITY,
>;

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
pub struct StartupSlaveProfile {
    pub position: u16,
    pub dc_requirement: StartupDcRequirement,
    pub transition_timeouts: AlTransitionTimeouts,
    pub op_only_outputs: OpOnlySyncManagerProfile,
    pub expected_mailbox: Option<MailboxConfig>,
    pub expected_sii: Option<SiiConfigurationSignature>,
}

impl StartupSlaveProfile {
    pub const EMPTY: Self = Self {
        position: 0,
        dc_requirement: StartupDcRequirement::None,
        transition_timeouts: ETG1020_DEFAULT_TRANSITION_TIMEOUTS_V1,
        op_only_outputs: OpOnlySyncManagerProfile::EMPTY,
        expected_mailbox: None,
        expected_sii: None,
    };

    pub const fn new(position: u16) -> Self {
        Self {
            position,
            ..Self::EMPTY
        }
    }

    pub const fn with_transition_timeouts(
        mut self,
        transition_timeouts: AlTransitionTimeouts,
    ) -> Self {
        self.transition_timeouts = transition_timeouts;
        self
    }

    pub const fn with_dc_requirement(mut self, dc_requirement: StartupDcRequirement) -> Self {
        self.dc_requirement = dc_requirement;
        self
    }

    pub const fn with_op_only_outputs(mut self, op_only_outputs: OpOnlySyncManagerProfile) -> Self {
        self.op_only_outputs = op_only_outputs;
        self
    }

    pub const fn with_expected_mailbox(mut self, expected_mailbox: MailboxConfig) -> Self {
        self.expected_mailbox = Some(expected_mailbox);
        self
    }

    pub const fn with_expected_sii(mut self, expected_sii: SiiConfigurationSignature) -> Self {
        self.expected_sii = Some(expected_sii);
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StartupDcRequirement {
    None = 0,
    SystemTime = 1,
    ReferenceClock = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupReferenceClock {
    pub position: u16,
    pub station_address: u16,
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
    pub sii_configuration_timeout_ns: u64,
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
            sii_configuration_timeout_ns: 30_000_000_000,
            transition_timeout_ns: 0,
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
    ReadingMailbox,
    ReadingConfiguration,
    TransitioningAl,
    AwaitingConfiguration,
    Ready,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupAction {
    Scan(ScanAction),
    Sii(SiiAction),
    SiiMailbox(SiiAction),
    SiiConfiguration(SiiAction),
    Al(AlAction),
    OpOnly(OpOnlySyncManagerAction),
}

impl StartupAction {
    pub const fn token(self) -> u8 {
        match self {
            Self::Scan(action) => action.token,
            Self::Sii(action) => action.token,
            Self::SiiMailbox(action) => action.token,
            Self::SiiConfiguration(action) => action.token,
            Self::Al(action) => action.token,
            Self::OpOnly(action) => action.token,
        }
    }

    pub const fn datagram_index(self) -> u8 {
        match self {
            Self::Scan(action) => action.datagram_index,
            Self::Sii(action) => action.datagram_index,
            Self::SiiMailbox(action) => action.datagram_index,
            Self::SiiConfiguration(action) => action.datagram_index,
            Self::Al(action) => action.datagram_index,
            Self::OpOnly(action) => action.datagram_index,
        }
    }

    pub const fn generation(self) -> u16 {
        match self {
            Self::Scan(action) => action.generation,
            Self::Sii(action) => action.generation,
            Self::SiiMailbox(action) => action.generation,
            Self::SiiConfiguration(action) => action.generation,
            Self::Al(action) => action.generation,
            Self::OpOnly(action) => action.generation,
        }
    }

    pub const fn address(self) -> u32 {
        match self {
            Self::Scan(action) => action.address,
            Self::Sii(action) => action.address,
            Self::SiiMailbox(action) => action.address,
            Self::SiiConfiguration(action) => action.address,
            Self::Al(action) => action.address,
            Self::OpOnly(action) => action.address,
        }
    }

    pub const fn operation(self) -> RegisterOperation {
        match self {
            Self::Scan(action) => action.operation,
            Self::Sii(action) => action.operation,
            Self::SiiMailbox(action) => action.operation,
            Self::SiiConfiguration(action) => action.operation,
            Self::Al(action) => action.operation,
            Self::OpOnly(action) => action.operation,
        }
    }

    pub fn payload(&self) -> &[u8] {
        match self {
            Self::Scan(action) => action.payload(),
            Self::Sii(action) => action.payload(),
            Self::SiiMailbox(action) => action.payload(),
            Self::SiiConfiguration(action) => action.payload(),
            Self::Al(action) => action.payload(),
            Self::OpOnly(action) => action.payload(),
        }
    }

    pub const fn deadline_ns(self) -> u64 {
        match self {
            Self::Scan(action) => action.deadline_ns,
            Self::Sii(action) => action.deadline_ns,
            Self::SiiMailbox(action) => action.deadline_ns,
            Self::SiiConfiguration(action) => action.deadline_ns,
            Self::Al(action) => action.deadline_ns,
            Self::OpOnly(action) => action.deadline_ns,
        }
    }

    pub const fn expected_wkc(self) -> u16 {
        match self {
            Self::Scan(action) => action.expected_wkc,
            Self::Sii(action) => action.expected_wkc,
            Self::SiiMailbox(action) => action.expected_wkc,
            Self::SiiConfiguration(action) => action.expected_wkc,
            Self::Al(action) => action.expected_wkc,
            Self::OpOnly(action) => action.expected_wkc,
        }
    }

    pub const fn working_counter_policy(self) -> RxWorkingCounterPolicy {
        match self {
            Self::Scan(action) => action.working_counter_policy,
            Self::Sii(_)
            | Self::SiiMailbox(_)
            | Self::SiiConfiguration(_)
            | Self::Al(_)
            | Self::OpOnly(_) => RxWorkingCounterPolicy::Exact,
        }
    }

    pub const fn datagram_len(self) -> usize {
        match self {
            Self::Scan(action) => action.datagram_len(),
            Self::Sii(action) => action.datagram_len(),
            Self::SiiMailbox(action) => action.datagram_len(),
            Self::SiiConfiguration(action) => action.datagram_len(),
            Self::Al(action) => action.datagram_len(),
            Self::OpOnly(action) => action.datagram_len(),
        }
    }

    pub const fn response_len(self) -> usize {
        match self {
            Self::Scan(action) => action.read_len as usize,
            Self::Sii(action) => action.read_len as usize,
            Self::SiiMailbox(action) => action.read_len as usize,
            Self::SiiConfiguration(action) => action.read_len as usize,
            Self::Al(action) => action.read_len as usize,
            Self::OpOnly(action) => action.response_len(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupProgress {
    Advanced,
    SlaveDiscovered(usize),
    IdentityVerified(usize),
    MailboxVerified(usize),
    SiiConfigurationVerified(usize),
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
    ProfileCountMismatch,
    DuplicateProfilePosition(u16),
    MissingProfilePosition(u16),
    MissingScanPosition(u16),
    DcSystemTimeRequired {
        position: u16,
        requirement: StartupDcRequirement,
    },
    DcPropagationDelayRequired {
        position: u16,
        requirement: StartupDcRequirement,
    },
    DcTopology(DcTopologyError),
    MultipleReferenceClocks {
        first_position: u16,
        second_position: u16,
    },
    InvalidTransitionTimeoutProfile(u16),
    InvalidMailboxProfile {
        position: u16,
        error: MailboxConfigError,
    },
    OpOnlyProfile {
        position: u16,
        error: OpOnlyProfileError,
    },
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
    SiiMailbox(SiiMailboxError),
    SiiConfiguration(SiiDiscoveryError),
    SiiConfigurationSignature(SiiConfigurationSignatureError),
    SiiConfigurationMismatch {
        position: u16,
        expected: SiiConfigurationSignature,
        observed: SiiConfigurationSignature,
    },
    MailboxMismatch {
        position: u16,
        expected: MailboxConfig,
        observed: MailboxConfig,
    },
    Al(AlError),
    OpOnly(OpOnlySyncManagerError),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupTransitionStage {
    Idle,
    OpOnlyBeforeAl,
    Al,
    OpOnlyAfterAl,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OpOnlyGateState {
    Unknown,
    DisabledVerified,
    EnabledVerified,
}

pub struct StartupController<const MAX_SLAVES: usize> {
    phase: StartupPhase,
    config: StartupConfig,
    generation: u16,
    station_address_base: u16,
    expected: [ExpectedSlave; MAX_SLAVES],
    profiles: [StartupSlaveProfile; MAX_SLAVES],
    expected_count: usize,
    scan: ScanController<MAX_SLAVES>,
    sii: SiiIdentityReader,
    sii_mailbox: SiiBlockReader<SII_STANDARD_MAILBOX_WORD_COUNT>,
    sii_configuration: StartupSiiDiscovery,
    sii_configuration_scratch: [u8; STARTUP_SII_IMAGE_BYTE_CAPACITY],
    al: AlTransitionController,
    op_only: OpOnlySyncManagerController,
    table: SlaveTable<MAX_SLAVES>,
    device_emulation: [bool; MAX_SLAVES],
    verified_mailboxes: [Option<MailboxConfig>; MAX_SLAVES],
    verified_sii: [Option<SiiConfigurationSignature>; MAX_SLAVES],
    selected_reference_clock: Option<StartupReferenceClock>,
    dc_topology: Option<DcTopology<MAX_SLAVES>>,
    op_only_gate: [OpOnlyGateState; MAX_SLAVES],
    current_index: usize,
    stage_target: EthercatState,
    configuration_released: bool,
    transition_stage: StartupTransitionStage,
    step_deadline_ns: u64,
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
            profiles: [StartupSlaveProfile::EMPTY; MAX_SLAVES],
            expected_count: 0,
            scan: ScanController::new(station_address_base),
            sii: SiiIdentityReader::new(),
            sii_mailbox: SiiBlockReader::new(),
            sii_configuration: SiiStreamDiscoveryController::new(),
            sii_configuration_scratch: [0; STARTUP_SII_IMAGE_BYTE_CAPACITY],
            al: AlTransitionController::new(),
            op_only: OpOnlySyncManagerController::new(),
            table: SlaveTable::new(),
            device_emulation: [false; MAX_SLAVES],
            verified_mailboxes: [None; MAX_SLAVES],
            verified_sii: [None; MAX_SLAVES],
            selected_reference_clock: None,
            dc_topology: None,
            op_only_gate: [OpOnlyGateState::Unknown; MAX_SLAVES],
            current_index: 0,
            stage_target: EthercatState::Op,
            configuration_released: false,
            transition_stage: StartupTransitionStage::Idle,
            step_deadline_ns: 0,
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

    pub fn verified_mailbox(&self, position: u16) -> Option<MailboxConfig> {
        self.table
            .records()
            .iter()
            .position(|record| record.position == position)
            .and_then(|index| self.verified_mailboxes[index])
    }

    pub fn verified_sii(&self, position: u16) -> Option<SiiConfigurationSignature> {
        self.table
            .records()
            .iter()
            .position(|record| record.position == position)
            .and_then(|index| self.verified_sii[index])
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

    pub fn dc_capabilities(&self, position: u16) -> Option<ScanDcCapabilities> {
        self.scan
            .records()
            .iter()
            .find(|record| record.position == position)
            .map(|record| record.dc)
    }

    pub const fn selected_reference_clock(&self) -> Option<StartupReferenceClock> {
        self.selected_reference_clock
    }

    pub const fn dc_topology(&self) -> Option<&DcTopology<MAX_SLAVES>> {
        self.dc_topology.as_ref()
    }

    pub fn pending_action(&self) -> Option<StartupAction> {
        match self.phase {
            StartupPhase::Scanning => self.scan.pending().map(StartupAction::Scan),
            StartupPhase::ReadingIdentity => self.sii.pending().map(StartupAction::Sii),
            StartupPhase::ReadingMailbox => {
                self.sii_mailbox.pending().map(StartupAction::SiiMailbox)
            }
            StartupPhase::ReadingConfiguration => self
                .sii_configuration
                .pending()
                .map(StartupAction::SiiConfiguration),
            StartupPhase::TransitioningAl => match self.transition_stage {
                StartupTransitionStage::Al => self.al.pending().map(StartupAction::Al),
                StartupTransitionStage::OpOnlyBeforeAl | StartupTransitionStage::OpOnlyAfterAl => {
                    self.op_only.pending().map(StartupAction::OpOnly)
                }
                StartupTransitionStage::Idle => None,
            },
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
        self.start_inner(generation, now_ns, config, expected, None)
    }

    pub fn start_with_profiles(
        &mut self,
        generation: u16,
        now_ns: u64,
        config: StartupConfig,
        expected: &[ExpectedSlave],
        profiles: &[StartupSlaveProfile],
    ) -> Result<(), StartupError> {
        self.start_inner(generation, now_ns, config, expected, Some(profiles))
    }

    fn start_inner(
        &mut self,
        generation: u16,
        now_ns: u64,
        config: StartupConfig,
        expected: &[ExpectedSlave],
        profiles: Option<&[StartupSlaveProfile]>,
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
        if let Some(profiles) = profiles {
            if profiles.len() != expected.len() {
                return Err(StartupError::ProfileCountMismatch);
            }
            for (index, profile) in profiles.iter().copied().enumerate() {
                if profiles[..index]
                    .iter()
                    .any(|existing| existing.position == profile.position)
                {
                    return Err(StartupError::DuplicateProfilePosition(profile.position));
                }
                if !expected
                    .iter()
                    .any(|item| item.position == profile.position)
                {
                    return Err(StartupError::MissingExpectedPosition);
                }
                if !profile.transition_timeouts.is_valid() {
                    return Err(StartupError::InvalidTransitionTimeoutProfile(
                        profile.position,
                    ));
                }
                if let Err(error) = profile.op_only_outputs.validate() {
                    return Err(StartupError::OpOnlyProfile {
                        position: profile.position,
                        error,
                    });
                }
                if let Some(mailbox) = profile.expected_mailbox {
                    if let Err(error) = mailbox.validate() {
                        return Err(StartupError::InvalidMailboxProfile {
                            position: profile.position,
                            error,
                        });
                    }
                }
            }
            for item in expected {
                if !profiles
                    .iter()
                    .any(|profile| profile.position == item.position)
                {
                    return Err(StartupError::MissingProfilePosition(item.position));
                }
            }
        }

        self.phase = StartupPhase::Scanning;
        self.config = config;
        self.generation = generation;
        self.expected = [ExpectedSlave::EMPTY; MAX_SLAVES];
        self.expected[..expected.len()].copy_from_slice(expected);
        self.profiles = [StartupSlaveProfile::EMPTY; MAX_SLAVES];
        match profiles {
            Some(profiles) => self.profiles[..profiles.len()].copy_from_slice(profiles),
            None => {
                for (index, item) in expected.iter().enumerate() {
                    self.profiles[index] = StartupSlaveProfile::new(item.position);
                }
            }
        }
        self.expected_count = expected.len();
        self.scan = ScanController::new(self.station_address_base);
        self.sii = SiiIdentityReader::new();
        self.sii_mailbox = SiiBlockReader::new();
        self.sii_configuration = SiiStreamDiscoveryController::new();
        self.sii_configuration_scratch = [0; STARTUP_SII_IMAGE_BYTE_CAPACITY];
        self.al = AlTransitionController::new();
        self.op_only = OpOnlySyncManagerController::new();
        self.table = SlaveTable::new();
        self.device_emulation = [false; MAX_SLAVES];
        self.verified_mailboxes = [None; MAX_SLAVES];
        self.verified_sii = [None; MAX_SLAVES];
        self.selected_reference_clock = None;
        self.dc_topology = None;
        self.op_only_gate = [OpOnlyGateState::Unknown; MAX_SLAVES];
        self.current_index = 0;
        self.stage_target = if config.configuration_services.is_empty() {
            config.target_state
        } else {
            EthercatState::PreOp
        };
        self.configuration_released = false;
        self.transition_stage = StartupTransitionStage::Idle;
        self.step_deadline_ns = 0;
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
                StartupPhase::ReadingMailbox => {
                    self.start_mailbox_reader(now_ns)?;
                    match self.sii_mailbox.next_action(now_ns) {
                        Ok(Some(action)) => return Ok(Some(StartupAction::SiiMailbox(action))),
                        Ok(None) => return Ok(None),
                        Err(error) => {
                            return self
                                .fail(StartupError::SiiMailbox(SiiMailboxError::Block(error)));
                        }
                    }
                }
                StartupPhase::ReadingConfiguration => {
                    self.start_sii_configuration_reader(now_ns)?;
                    match self.sii_configuration.next_action(now_ns) {
                        Ok(Some(action)) => {
                            return Ok(Some(StartupAction::SiiConfiguration(action)));
                        }
                        Ok(None)
                            if self.sii_configuration.phase() == SiiDiscoveryPhase::Projecting =>
                        {
                            self.finish_sii_configuration(now_ns)?;
                        }
                        Ok(None) => return Ok(None),
                        Err(error) => {
                            return self.fail(StartupError::SiiConfiguration(error));
                        }
                    }
                }
                StartupPhase::TransitioningAl => match self.transition_stage {
                    StartupTransitionStage::Al => match self.al.next_action(now_ns) {
                        Ok(Some(action)) => return Ok(Some(StartupAction::Al(action))),
                        Ok(None) => return Ok(None),
                        Err(error) => {
                            self.capture_al_fault();
                            return self.fail(StartupError::Al(error));
                        }
                    },
                    StartupTransitionStage::OpOnlyBeforeAl
                    | StartupTransitionStage::OpOnlyAfterAl => {
                        match self.op_only.next_action(now_ns) {
                            Ok(Some(action)) => return Ok(Some(StartupAction::OpOnly(action))),
                            Ok(None) => return Ok(None),
                            Err(error) => return self.fail(StartupError::OpOnly(error)),
                        }
                    }
                    StartupTransitionStage::Idle => return Ok(None),
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
        pool.acquire_with_response_len_and_wkc_policy(
            action.datagram_index(),
            action.generation(),
            action.address(),
            action.operation(),
            action.payload(),
            action.datagram_len(),
            action.deadline_ns(),
            action.working_counter_policy(),
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
            StartupAction::SiiMailbox(action) => {
                if self.phase != StartupPhase::ReadingMailbox {
                    return self.fail(StartupError::NoPendingAction);
                }
                let progress = match self.sii_mailbox.accept(
                    action.token,
                    generation,
                    payload,
                    working_counter,
                    now_ns,
                ) {
                    Ok(progress) => progress,
                    Err(error) => {
                        return self.fail(StartupError::SiiMailbox(SiiMailboxError::Block(error)));
                    }
                };
                if progress == SiiProgress::Complete {
                    return self.finish_mailbox(now_ns);
                }
                Ok(StartupProgress::Advanced)
            }
            StartupAction::SiiConfiguration(action) => {
                if self.phase != StartupPhase::ReadingConfiguration {
                    return self.fail(StartupError::NoPendingAction);
                }
                let progress = match self.sii_configuration.accept(
                    action.token,
                    generation,
                    payload,
                    working_counter,
                    now_ns,
                ) {
                    Ok(progress) => progress,
                    Err(error) => return self.fail(StartupError::SiiConfiguration(error)),
                };
                if matches!(progress, SiiCategoryStreamProgress::Complete { .. }) {
                    return self.finish_sii_configuration(now_ns);
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
            StartupAction::OpOnly(action) => {
                if self.phase != StartupPhase::TransitioningAl
                    || !matches!(
                        self.transition_stage,
                        StartupTransitionStage::OpOnlyBeforeAl
                            | StartupTransitionStage::OpOnlyAfterAl
                    )
                {
                    return self.fail(StartupError::NoPendingAction);
                }
                let progress =
                    match self
                        .op_only
                        .accept(action, generation, payload, working_counter, now_ns)
                    {
                        Ok(progress) => progress,
                        Err(error) => return self.fail(StartupError::OpOnly(error)),
                    };
                if progress == OpOnlySyncManagerProgress::Complete {
                    return self.finish_op_only(now_ns);
                }
                Ok(StartupProgress::Advanced)
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
                ) || request.working_counter_policy() != action.working_counter_policy()
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
                if !request.matches_action(
                    action.datagram_index(),
                    action.generation(),
                    action.address(),
                    action.operation(),
                    action.payload(),
                    action.datagram_len(),
                    action.deadline_ns(),
                ) || request.working_counter_policy() != action.working_counter_policy()
                {
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
            StartupAction::SiiMailbox(action) => {
                match self.sii_mailbox.timeout(action.token, now_ns) {
                    Ok(()) => self.fail(StartupError::SiiMailbox(SiiMailboxError::Block(
                        crate::sii::SiiBlockError::Timeout,
                    ))),
                    Err(error) => {
                        let error = StartupError::SiiMailbox(SiiMailboxError::Block(error));
                        if self.sii_mailbox.phase() == SiiPhase::Faulted {
                            self.fail(error)
                        } else {
                            Err(error)
                        }
                    }
                }
            }
            StartupAction::SiiConfiguration(action) => {
                match self.sii_configuration.timeout(action.token, now_ns) {
                    Ok(()) => Ok(StartupProgress::Advanced),
                    Err(error) => self.fail(StartupError::SiiConfiguration(error)),
                }
            }
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
            StartupAction::OpOnly(action) => match self.op_only.timeout(action, now_ns) {
                Ok(_) => self.fail(StartupError::OpOnly(OpOnlySyncManagerError::Timeout)),
                Err(error) => {
                    if self.op_only.phase() == OpOnlySyncManagerPhase::Faulted {
                        self.fail(StartupError::OpOnly(error))
                    } else {
                        Err(StartupError::OpOnly(error))
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
        self.op_only = OpOnlySyncManagerController::new();
        self.transition_stage = StartupTransitionStage::Idle;
        self.step_deadline_ns = 0;
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
        self.profiles = [StartupSlaveProfile::EMPTY; MAX_SLAVES];
        for (index, item) in expected.iter().enumerate() {
            self.profiles[index] = StartupSlaveProfile::new(item.position);
        }
        self.expected_count = expected.len();
        self.generation = generation;
        self.scan = ScanController::new(self.station_address_base);
        self.sii = SiiIdentityReader::new();
        self.sii_mailbox = SiiBlockReader::new();
        self.sii_configuration = SiiStreamDiscoveryController::new();
        self.sii_configuration_scratch = [0; STARTUP_SII_IMAGE_BYTE_CAPACITY];
        self.al = AlTransitionController::new();
        self.op_only = OpOnlySyncManagerController::new();
        self.table = SlaveTable::new();
        self.device_emulation = [false; MAX_SLAVES];
        self.verified_mailboxes = [None; MAX_SLAVES];
        self.verified_sii = [None; MAX_SLAVES];
        self.selected_reference_clock = None;
        self.dc_topology = None;
        self.op_only_gate = [OpOnlyGateState::Unknown; MAX_SLAVES];
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
        self.transition_stage = StartupTransitionStage::Idle;
        self.step_deadline_ns = 0;
        self.last_error = None;
        self.last_al_fault = None;
        Ok(())
    }

    fn enter_identity_phase(&mut self) -> Result<(), StartupError> {
        if self.scan.len() != self.expected_count {
            return self.fail(StartupError::ExpectedCountMismatch);
        }

        let mut fallback_reference = None;
        for record in self.scan.records().iter().copied() {
            if fallback_reference.is_none() && record.dc.can_be_reference_clock() {
                fallback_reference = Some(StartupReferenceClock {
                    position: record.position,
                    station_address: record.station_address,
                });
            }
        }

        let mut explicit_reference: Option<StartupReferenceClock> = None;
        for profile in self.profiles.iter().take(self.expected_count).copied() {
            let record = match self
                .scan
                .records()
                .iter()
                .find(|record| record.position == profile.position)
                .copied()
            {
                Some(record) => record,
                None => return self.fail(StartupError::MissingScanPosition(profile.position)),
            };
            if profile.dc_requirement != StartupDcRequirement::None
                && !record.dc.can_be_reference_clock()
            {
                return self.fail(StartupError::DcSystemTimeRequired {
                    position: profile.position,
                    requirement: profile.dc_requirement,
                });
            }
            if profile.dc_requirement == StartupDcRequirement::ReferenceClock {
                let candidate = StartupReferenceClock {
                    position: record.position,
                    station_address: record.station_address,
                };
                if let Some(first) = explicit_reference {
                    return self.fail(StartupError::MultipleReferenceClocks {
                        first_position: first.position,
                        second_position: candidate.position,
                    });
                }
                explicit_reference = Some(candidate);
            }
        }

        let selected_reference_clock = explicit_reference.or(fallback_reference);
        let topology = match DcTopology::build(
            self.scan.records(),
            selected_reference_clock.map(|reference| reference.position),
        ) {
            Ok(topology) => topology,
            Err(error) => return self.fail(StartupError::DcTopology(error)),
        };
        for profile in self.profiles.iter().take(self.expected_count).copied() {
            if profile.dc_requirement != StartupDcRequirement::None
                && topology.transmission_delay_ns(profile.position).is_none()
            {
                return self.fail(StartupError::DcPropagationDelayRequired {
                    position: profile.position,
                    requirement: profile.dc_requirement,
                });
            }
        }

        self.selected_reference_clock = selected_reference_clock;
        self.dc_topology = Some(topology);
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

    fn start_mailbox_reader(&mut self, now_ns: u64) -> Result<(), StartupError> {
        if !matches!(
            self.sii_mailbox.phase(),
            SiiPhase::Idle | SiiPhase::Complete | SiiPhase::Faulted
        ) {
            return Ok(());
        }
        let record = self
            .table
            .records()
            .get(self.current_index)
            .copied()
            .ok_or(StartupError::ExpectedCountMismatch)?;
        let request = SiiBlockRequest {
            station_address: record.station_address,
            start_word: SII_STANDARD_RECEIVE_MAILBOX_OFFSET_WORD,
            word_count: SII_STANDARD_MAILBOX_WORD_COUNT,
            generation: self.generation,
            now_ns,
            timeout_ns: self.config.identity_timeout_ns,
            request_timeout_ns: self.config.request_timeout_ns,
        };
        match self.sii_mailbox.start(request) {
            Ok(()) => Ok(()),
            Err(error) => self.fail(StartupError::SiiMailbox(SiiMailboxError::Block(error))),
        }
    }

    fn start_sii_configuration_reader(&mut self, now_ns: u64) -> Result<(), StartupError> {
        if !matches!(
            self.sii_configuration.phase(),
            SiiDiscoveryPhase::Idle | SiiDiscoveryPhase::Ready | SiiDiscoveryPhase::Faulted
        ) {
            return Ok(());
        }
        let record = self
            .table
            .records()
            .get(self.current_index)
            .copied()
            .ok_or(StartupError::ExpectedCountMismatch)?;
        let request = SiiStreamDiscoveryRequest {
            stream: SiiCategoryStreamRequest::standard(
                record.station_address,
                self.generation,
                now_ns,
                self.config.sii_configuration_timeout_ns,
                self.config.request_timeout_ns,
            ),
            signed: false,
        };
        match self.sii_configuration.start(request) {
            Ok(()) => Ok(()),
            Err(error) => self.fail(StartupError::SiiConfiguration(error)),
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
        let profile = self.profile_for_position(scan_record.position)?;
        if profile.expected_mailbox.is_some() {
            self.phase = StartupPhase::ReadingMailbox;
            return Ok(StartupProgress::IdentityVerified(self.current_index));
        }
        if profile.expected_sii.is_some() {
            self.phase = StartupPhase::ReadingConfiguration;
            return Ok(StartupProgress::IdentityVerified(self.current_index));
        }
        self.phase = StartupPhase::TransitioningAl;
        self.start_al_for_current(now_ns)
    }

    fn finish_mailbox(&mut self, now_ns: u64) -> Result<StartupProgress, StartupError> {
        let record = self
            .table
            .records()
            .get(self.current_index)
            .copied()
            .ok_or(StartupError::ExpectedCountMismatch)?;
        let expected = self
            .profile_for_position(record.position)?
            .expected_mailbox
            .ok_or(StartupError::NoPendingAction)?;
        let descriptor = match SiiStandardMailbox::from_completed_block(&self.sii_mailbox) {
            Ok(descriptor) => descriptor,
            Err(error) => return self.fail(StartupError::SiiMailbox(error)),
        };
        let observed = match descriptor.coe_mailbox_config() {
            Ok(config) => config,
            Err(error) => return self.fail(StartupError::SiiMailbox(error)),
        };
        if !expected.has_same_layout(observed) {
            return self.fail(StartupError::MailboxMismatch {
                position: record.position,
                expected,
                observed,
            });
        }
        self.verified_mailboxes[self.current_index] = Some(observed);
        if self
            .profile_for_position(record.position)?
            .expected_sii
            .is_some()
        {
            self.phase = StartupPhase::ReadingConfiguration;
            return Ok(StartupProgress::MailboxVerified(self.current_index));
        }
        self.phase = StartupPhase::TransitioningAl;
        match self.start_al_for_current(now_ns)? {
            StartupProgress::IdentityVerified(index) => Ok(StartupProgress::MailboxVerified(index)),
            progress => Ok(progress),
        }
    }

    fn finish_sii_configuration(&mut self, now_ns: u64) -> Result<StartupProgress, StartupError> {
        if let Err(error) = self
            .sii_configuration
            .finalize(&mut self.sii_configuration_scratch)
        {
            return self.fail(StartupError::SiiConfiguration(error));
        }
        let observed = match self
            .sii_configuration
            .candidate()
            .ok_or(StartupError::NoPendingAction)?
            .signature()
        {
            Ok(signature) => signature,
            Err(error) => return self.fail(StartupError::SiiConfigurationSignature(error)),
        };
        let record = self
            .table
            .records()
            .get(self.current_index)
            .copied()
            .ok_or(StartupError::ExpectedCountMismatch)?;
        let expected = self
            .profile_for_position(record.position)?
            .expected_sii
            .ok_or(StartupError::NoPendingAction)?;
        if observed != expected {
            return self.fail(StartupError::SiiConfigurationMismatch {
                position: record.position,
                expected,
                observed,
            });
        }
        self.verified_sii[self.current_index] = Some(observed);
        self.phase = StartupPhase::TransitioningAl;
        match self.start_al_for_current(now_ns)? {
            StartupProgress::IdentityVerified(index) => {
                Ok(StartupProgress::SiiConfigurationVerified(index))
            }
            progress => Ok(progress),
        }
    }

    fn start_al_for_current(&mut self, now_ns: u64) -> Result<StartupProgress, StartupError> {
        let record = self
            .table
            .records()
            .get(self.current_index)
            .copied()
            .ok_or(StartupError::ExpectedCountMismatch)?;
        let profile = self.profile_for_position(record.position)?;
        let expected_state = if record.al_status.state == self.stage_target {
            record.al_status.state
        } else {
            next_state(record.al_status.state, self.stage_target)
                .ok_or(StartupError::Al(AlError::InvalidTransition))?
        };
        let timeout_ns = if self.config.transition_timeout_ns != 0 {
            self.config.transition_timeout_ns
        } else {
            profile
                .transition_timeouts
                .for_step(record.al_status.state, expected_state)
                .ok_or(StartupError::InvalidTransitionTimeoutProfile(
                    record.position,
                ))?
        };
        self.step_deadline_ns = now_ns.saturating_add(timeout_ns);

        let needs_disabled =
            record.al_status.state != EthercatState::Op || self.stage_target != EthercatState::Op;
        if !profile.op_only_outputs.is_empty()
            && needs_disabled
            && self.op_only_gate[self.current_index] != OpOnlyGateState::DisabledVerified
        {
            return self.start_op_only(
                record.station_address,
                profile.op_only_outputs,
                false,
                StartupTransitionStage::OpOnlyBeforeAl,
                now_ns,
            );
        }
        self.start_al_transport(record, now_ns)
    }

    fn start_al_transport(
        &mut self,
        record: SlaveRecord,
        now_ns: u64,
    ) -> Result<StartupProgress, StartupError> {
        if now_ns >= self.step_deadline_ns {
            return self.fail(StartupError::Al(AlError::Timeout));
        }
        let policy = if self.device_emulation[self.current_index] {
            AlErrorAcknowledgePolicy::Disabled
        } else {
            AlErrorAcknowledgePolicy::Enabled
        };
        self.transition_stage = StartupTransitionStage::Al;
        if let Err(error) = self.al.start_with_status(
            AlTransitionRequest {
                station_address: record.station_address,
                current_state: record.al_status.state,
                requested_state: self.stage_target,
                generation: self.generation,
                now_ns,
                timeout_ns: self.step_deadline_ns.saturating_sub(now_ns),
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

    fn start_op_only(
        &mut self,
        station_address: u16,
        profile: OpOnlySyncManagerProfile,
        enabled: bool,
        stage: StartupTransitionStage,
        now_ns: u64,
    ) -> Result<StartupProgress, StartupError> {
        if now_ns >= self.step_deadline_ns {
            return self.fail(StartupError::OpOnly(OpOnlySyncManagerError::Timeout));
        }
        if let Err(error) = self.op_only.start(
            station_address,
            self.generation,
            self.step_deadline_ns,
            self.config.request_timeout_ns,
            profile,
            enabled,
        ) {
            return self.fail(StartupError::OpOnly(error));
        }
        self.transition_stage = stage;
        Ok(StartupProgress::Advanced)
    }

    fn finish_op_only(&mut self, now_ns: u64) -> Result<StartupProgress, StartupError> {
        match self.transition_stage {
            StartupTransitionStage::OpOnlyBeforeAl => {
                self.op_only_gate[self.current_index] = OpOnlyGateState::DisabledVerified;
                let record = self
                    .table
                    .records()
                    .get(self.current_index)
                    .copied()
                    .ok_or(StartupError::ExpectedCountMismatch)?;
                self.start_al_transport(record, now_ns)
            }
            StartupTransitionStage::OpOnlyAfterAl => {
                self.op_only_gate[self.current_index] = OpOnlyGateState::EnabledVerified;
                self.complete_current_slave(now_ns)
            }
            StartupTransitionStage::Idle | StartupTransitionStage::Al => {
                self.fail(StartupError::NoPendingAction)
            }
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

        let profile = self.profile_for_position(position)?;
        if status.state == EthercatState::Op
            && !profile.op_only_outputs.is_empty()
            && self.op_only_gate[self.current_index] != OpOnlyGateState::EnabledVerified
        {
            return self.start_op_only(
                self.table.records()[self.current_index].station_address,
                profile.op_only_outputs,
                true,
                StartupTransitionStage::OpOnlyAfterAl,
                now_ns,
            );
        }
        self.complete_current_slave(now_ns)
    }

    fn complete_current_slave(&mut self, now_ns: u64) -> Result<StartupProgress, StartupError> {
        self.transition_stage = StartupTransitionStage::Idle;
        self.step_deadline_ns = 0;
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

    fn profile_for_position(&self, position: u16) -> Result<StartupSlaveProfile, StartupError> {
        self.profiles
            .iter()
            .take(self.expected_count)
            .find(|profile| profile.position == position)
            .copied()
            .ok_or(StartupError::MissingProfilePosition(position))
    }

    fn fail<T>(&mut self, error: StartupError) -> Result<T, StartupError> {
        self.last_error = Some(error);
        self.selected_reference_clock = None;
        self.dc_topology = None;
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
    use crate::op_only::{SYNC_MANAGER_ENABLE_FLAG, SYNC_MANAGER_OP_ONLY_FLAG};
    use crate::registers::{
        ESC_AL_STATUS, ESC_CONFIGURATION, ESC_EEPROM_CONTROL, ESC_EEPROM_DATA, ESC_TYPE,
        auto_increment_address, fixed_address, register_from_address,
    };
    use crate::sii::{
        SII_CATEGORY_END, SII_CATEGORY_RX_PDO, SII_CATEGORY_SYNC_MANAGER, SII_CATEGORY_TX_PDO,
    };
    use crate::sii_config::SiiConfigurationSignatureBuilder;
    use crate::sii_stream::SII_CATEGORY_START_WORD;
    use crate::slave::AL_ERROR_FLAG;

    fn status(state: EthercatState) -> [u8; 6] {
        let mut bytes = [0; 6];
        bytes[0..2].copy_from_slice(&(state as u16).to_le_bytes());
        bytes
    }

    fn basic_info() -> [u8; crate::BASIC_ESC_INFO_LEN as usize] {
        [0x88, 0x02, 3, 4, 1, 2, 0x20, 0xE4, 0, 0, 0, 0]
    }

    fn basic_info_with_features(features: u16) -> [u8; crate::BASIC_ESC_INFO_LEN as usize] {
        let mut bytes = basic_info();
        bytes[8..10].copy_from_slice(&features.to_le_bytes());
        bytes
    }

    fn dc_receive_times(
        values: [u32; crate::ESC_PORT_COUNT],
    ) -> [u8; crate::ESC_DC_RECEIVE_TIME_LEN as usize] {
        let mut bytes = [0; crate::ESC_DC_RECEIVE_TIME_LEN as usize];
        for (port, value) in values.iter().copied().enumerate() {
            bytes[port * 4..port * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
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

    fn op_only_profile(position: u16, timeouts: AlTransitionTimeouts) -> StartupSlaveProfile {
        let mut outputs = OpOnlySyncManagerProfile::new();
        outputs
            .add(
                2,
                SYNC_MANAGER_OP_ONLY_FLAG | SYNC_MANAGER_ENABLE_FLAG | 0x20,
            )
            .unwrap();
        StartupSlaveProfile::new(position)
            .with_transition_timeouts(timeouts)
            .with_op_only_outputs(outputs)
    }

    fn prepared_transition(
        current: EthercatState,
        target: EthercatState,
        mut config: StartupConfig,
        profile: StartupSlaveProfile,
        now_ns: u64,
    ) -> StartupController<1> {
        let identity = SlaveIdentity {
            vendor_id: 1,
            product_code: 2,
            revision: 3,
            serial: 4,
        };
        let expected = ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity,
        };
        config.target_state = target;
        let mut startup = StartupController::<1>::new(0x1000);
        startup.phase = StartupPhase::TransitioningAl;
        startup.config = config;
        startup.generation = 7;
        startup.expected[0] = expected;
        startup.profiles[0] = profile;
        startup.expected_count = 1;
        startup
            .table
            .add(expected.position, expected.station_address, identity)
            .unwrap();
        startup
            .table
            .observe_status(
                expected.position,
                crate::slave::AlStatus::new(current as u16, 0),
                0,
            )
            .unwrap();
        startup
            .table
            .verify_identity(expected.position, identity)
            .unwrap();
        startup.current_index = 0;
        startup.stage_target = target;
        startup.start_al_for_current(now_ns).unwrap();
        startup
    }

    fn accept_op_only<const MAX_SLAVES: usize>(
        startup: &mut StartupController<MAX_SLAVES>,
        activation: u8,
        now_ns: u64,
    ) -> StartupProgress {
        let write = startup.next_action(now_ns).unwrap().unwrap();
        assert!(matches!(write, StartupAction::OpOnly(_)));
        assert_eq!(write.payload(), &[activation]);
        assert_eq!(
            accept_action(startup, write, &[], 1, now_ns + 1),
            StartupProgress::Advanced
        );
        let read = startup.next_action(now_ns + 2).unwrap().unwrap();
        assert!(matches!(read, StartupAction::OpOnly(_)));
        accept_action(startup, read, &[activation], 1, now_ns + 3)
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

    fn accept_scanned_slave<const MAX_SLAVES: usize>(
        startup: &mut StartupController<MAX_SLAVES>,
        features: u16,
        system_time: Option<u64>,
        now_ns: &mut u64,
    ) {
        let dl_status = if startup.scan.len() + 1 < startup.expected_count {
            0x1500
        } else {
            0x5500
        };
        accept_scanned_slave_with_evidence(
            startup,
            features,
            system_time,
            [0, 0, 0, 200],
            dl_status,
            now_ns,
        );
    }

    fn accept_scanned_slave_with_evidence<const MAX_SLAVES: usize>(
        startup: &mut StartupController<MAX_SLAVES>,
        features: u16,
        system_time: Option<u64>,
        receive_time_values: [u32; crate::ESC_PORT_COUNT],
        dl_status_value: u16,
        now_ns: &mut u64,
    ) {
        let probe = startup.next_action(*now_ns).unwrap().unwrap();
        accept_action(startup, probe, &[0x88, 0x02], 1, *now_ns + 1);
        let basic = startup.next_action(*now_ns + 2).unwrap().unwrap();
        accept_action(
            startup,
            basic,
            &basic_info_with_features(features),
            1,
            *now_ns + 3,
        );
        let assign = startup.next_action(*now_ns + 4).unwrap().unwrap();
        accept_action(startup, assign, &[], 1, *now_ns + 5);
        let mut offset = 6;
        if features & crate::ESC_FEATURE_DC_SUPPORTED != 0 {
            let dc = startup.next_action(*now_ns + offset).unwrap().unwrap();
            assert_eq!(
                crate::register_from_address(dc.address()),
                crate::ESC_DC_SYSTEM_TIME
            );
            match system_time {
                Some(system_time) if dc.response_len() == 8 => {
                    accept_action(
                        startup,
                        dc,
                        &system_time.to_le_bytes(),
                        1,
                        *now_ns + offset + 1,
                    );
                }
                Some(system_time) => {
                    accept_action(
                        startup,
                        dc,
                        &(system_time as u32).to_le_bytes(),
                        1,
                        *now_ns + offset + 1,
                    );
                }
                None => {
                    accept_action(startup, dc, &[], 0, *now_ns + offset + 1);
                }
            }
            offset += 2;

            let receive_times = startup.next_action(*now_ns + offset).unwrap().unwrap();
            assert_eq!(
                crate::register_from_address(receive_times.address()),
                crate::ESC_DC_TIME0
            );
            accept_action(
                startup,
                receive_times,
                &dc_receive_times(receive_time_values),
                1,
                *now_ns + offset + 1,
            );
            offset += 2;
        }
        let dl_status = startup.next_action(*now_ns + offset).unwrap().unwrap();
        assert_eq!(
            crate::register_from_address(dl_status.address()),
            crate::ESC_DL_STATUS
        );
        accept_action(
            startup,
            dl_status,
            &dl_status_value.to_le_bytes(),
            1,
            *now_ns + offset + 1,
        );
        offset += 2;
        let configuration = startup.next_action(*now_ns + offset).unwrap().unwrap();
        accept_action(startup, configuration, &[0], 1, *now_ns + offset + 1);
        let status_action = startup.next_action(*now_ns + offset + 2).unwrap().unwrap();
        accept_action(
            startup,
            status_action,
            &status(EthercatState::Init),
            1,
            *now_ns + offset + 3,
        );
        *now_ns += offset + 4;
    }

    fn finish_scan<const MAX_SLAVES: usize>(
        startup: &mut StartupController<MAX_SLAVES>,
        now_ns: u64,
    ) -> Result<StartupProgress, StartupError> {
        match startup.next_action(now_ns)? {
            Some(end_probe @ StartupAction::Scan(_)) => {
                startup.accept(end_probe, end_probe.generation(), &[], 0, now_ns + 1)
            }
            Some(_) | None => Ok(StartupProgress::Advanced),
        }
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

    fn accept_mailbox_words<const MAX_SLAVES: usize>(
        startup: &mut StartupController<MAX_SLAVES>,
        words: [u16; SII_STANDARD_MAILBOX_WORD_COUNT],
        now_ns: &mut u64,
    ) -> Result<StartupProgress, StartupError> {
        let mut progress = StartupProgress::Advanced;
        for (index, word) in words.into_iter().enumerate() {
            let address = startup.next_action(*now_ns)?.unwrap();
            let StartupAction::SiiMailbox(address_action) = address else {
                panic!("expected SII mailbox address action");
            };
            assert_eq!(
                address_action.word_address,
                SII_STANDARD_RECEIVE_MAILBOX_OFFSET_WORD + index as u16
            );
            startup.accept(address, address.generation(), &[], 1, *now_ns + 1)?;

            let issue = startup.next_action(*now_ns + 2)?.unwrap();
            assert!(matches!(issue, StartupAction::SiiMailbox(_)));
            startup.accept(issue, issue.generation(), &[], 1, *now_ns + 3)?;

            let poll = startup.next_action(*now_ns + 4)?.unwrap();
            assert!(matches!(poll, StartupAction::SiiMailbox(_)));
            startup.accept(poll, poll.generation(), &[0, 0], 1, *now_ns + 5)?;

            let data = startup.next_action(*now_ns + 6)?.unwrap();
            assert!(matches!(data, StartupAction::SiiMailbox(_)));
            progress =
                startup.accept(data, data.generation(), &word.to_le_bytes(), 1, *now_ns + 7)?;
            *now_ns += 8;
        }
        Ok(progress)
    }

    fn prepared_mailbox_verification(expected_mailbox: MailboxConfig) -> StartupController<1> {
        let identity = SlaveIdentity {
            vendor_id: 1,
            product_code: 2,
            revision: 3,
            serial: 4,
        };
        let mut config = StartupConfig::new(EthercatState::Op);
        config.identity_timeout_ns = 10_000;
        config.request_timeout_ns = 1_000;
        let mut startup = StartupController::<1>::new(0x1000);
        startup.phase = StartupPhase::ReadingMailbox;
        startup.config = config;
        startup.generation = 7;
        startup.expected[0] = ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity,
        };
        startup.profiles[0] = StartupSlaveProfile::new(0).with_expected_mailbox(expected_mailbox);
        startup.expected_count = 1;
        startup
            .table
            .add(0, 0x1000, identity)
            .and_then(|_| {
                startup.table.observe_status(
                    0,
                    crate::slave::AlStatus::new(EthercatState::SafeOp as u16, 0),
                    0,
                )
            })
            .and_then(|_| startup.table.verify_identity(0, identity))
            .unwrap();
        startup.stage_target = EthercatState::Op;
        startup
    }

    fn append_sii_category(bytes: &mut std::vec::Vec<u8>, kind: u16, payload: &[u8]) {
        assert_eq!(payload.len() % 2, 0);
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&((payload.len() / 2) as u16).to_le_bytes());
        bytes.extend_from_slice(payload);
    }

    fn startup_sii_image(rx_object: u16) -> std::vec::Vec<u8> {
        let mut image = std::vec::Vec::new();
        append_sii_category(
            &mut image,
            SII_CATEGORY_SYNC_MANAGER,
            &[
                0x00, 0x10, 0x02, 0x00, 0x26, 0x00, 0x01, 0x00, 0x00, 0x11, 0x02, 0x00, 0x22, 0x00,
                0x01, 0x00,
            ],
        );
        for (kind, pdo_index, sync_manager, object_index) in [
            (SII_CATEGORY_RX_PDO, 0x1600u16, 0u8, rx_object),
            (SII_CATEGORY_TX_PDO, 0x1A00u16, 1u8, 0x6041u16),
        ] {
            let mut pdo = [0u8; 16];
            pdo[0..2].copy_from_slice(&pdo_index.to_le_bytes());
            pdo[2] = 1;
            pdo[3] = sync_manager;
            pdo[8..10].copy_from_slice(&object_index.to_le_bytes());
            pdo[10] = 0;
            pdo[12] = 16;
            append_sii_category(&mut image, kind, &pdo);
        }
        image.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
        image.extend_from_slice(&0u16.to_le_bytes());
        image
    }

    fn startup_sii_signature(rx_object: u16) -> SiiConfigurationSignature {
        let mut builder = SiiConfigurationSignatureBuilder::new(2, 0b11, 0).unwrap();
        builder
            .begin_pdo(crate::PdoDirection::Rx, 0x1600, 0)
            .unwrap();
        builder.entry(rx_object, 0, 16).unwrap();
        builder.end_pdo().unwrap();
        builder
            .begin_pdo(crate::PdoDirection::Tx, 0x1A00, 1)
            .unwrap();
        builder.entry(0x6041, 0, 16).unwrap();
        builder.end_pdo().unwrap();
        builder.finish().unwrap()
    }

    fn prepared_sii_verification(expected_sii: SiiConfigurationSignature) -> StartupController<1> {
        let identity = SlaveIdentity {
            vendor_id: 1,
            product_code: 2,
            revision: 3,
            serial: 4,
        };
        let mut config = StartupConfig::new(EthercatState::Op);
        config.sii_configuration_timeout_ns = 100_000;
        config.request_timeout_ns = 1_000;
        let mut startup = StartupController::<1>::new(0x1000);
        startup.phase = StartupPhase::ReadingConfiguration;
        startup.config = config;
        startup.generation = 7;
        startup.expected[0] = ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity,
        };
        startup.profiles[0] = StartupSlaveProfile::new(0).with_expected_sii(expected_sii);
        startup.expected_count = 1;
        startup
            .table
            .add(0, 0x1000, identity)
            .and_then(|_| {
                startup.table.observe_status(
                    0,
                    crate::slave::AlStatus::new(EthercatState::SafeOp as u16, 0),
                    0,
                )
            })
            .and_then(|_| startup.table.verify_identity(0, identity))
            .unwrap();
        startup.stage_target = EthercatState::Op;
        startup
    }

    fn prepared_multi_sii_verification(
        expected_sii: [SiiConfigurationSignature; 2],
        identities: [SlaveIdentity; 2],
    ) -> StartupController<2> {
        let mut config = StartupConfig::new(EthercatState::Op);
        config.sii_configuration_timeout_ns = 100_000;
        config.request_timeout_ns = 1_000;
        let mut startup = StartupController::<2>::new(0x1000);
        startup.phase = StartupPhase::ReadingConfiguration;
        startup.config = config;
        startup.generation = 7;
        startup.expected_count = 2;
        startup.stage_target = EthercatState::Op;
        for index in 0..2 {
            startup.expected[index] = ExpectedSlave {
                position: index as u16,
                station_address: 0x1000 + index as u16,
                identity: identities[index],
            };
            startup.profiles[index] =
                StartupSlaveProfile::new(index as u16).with_expected_sii(expected_sii[index]);
            startup
                .table
                .add(index as u16, 0x1000 + index as u16, identities[index])
                .and_then(|_| {
                    startup.table.observe_status(
                        index as u16,
                        crate::slave::AlStatus::new(EthercatState::SafeOp as u16, 0),
                        0,
                    )
                })
                .and_then(|_| {
                    startup
                        .table
                        .verify_identity(index as u16, identities[index])
                })
                .unwrap();
        }
        startup
    }

    fn drive_sii_configuration<const MAX_SLAVES: usize>(
        startup: &mut StartupController<MAX_SLAVES>,
        image: &[u8],
        now_ns: &mut u64,
    ) -> Result<StartupProgress, StartupError> {
        let mut progress = StartupProgress::Advanced;
        while startup.phase() == StartupPhase::ReadingConfiguration {
            let action = startup.next_action(*now_ns)?.unwrap();
            let StartupAction::SiiConfiguration(inner) = action else {
                panic!("expected SII configuration action");
            };
            let payload = if inner.read_len == 0 {
                std::vec::Vec::new()
            } else {
                match register_from_address(inner.address) {
                    ESC_EEPROM_CONTROL => std::vec::Vec::from([0, 0]),
                    ESC_EEPROM_DATA => {
                        let offset = usize::from(inner.word_address - SII_CATEGORY_START_WORD) * 2;
                        image[offset..offset + inner.read_len as usize].to_vec()
                    }
                    register => panic!("unexpected EEPROM register {register:#06x}"),
                }
            };
            progress = startup.accept(
                action,
                action.generation(),
                &payload,
                inner.expected_wkc,
                *now_ns + 1,
            )?;
            *now_ns += 2;
        }
        Ok(progress)
    }

    #[test]
    fn startup_verifies_complete_sii_stream_before_first_al_action() {
        let expected = startup_sii_signature(0x6040);
        let mut startup = prepared_sii_verification(expected);
        let mut now_ns = 1;
        assert_eq!(
            drive_sii_configuration(&mut startup, &startup_sii_image(0x6040), &mut now_ns),
            Ok(StartupProgress::SiiConfigurationVerified(0))
        );
        assert_eq!(startup.verified_sii(0), Some(expected));
        assert_eq!(startup.phase(), StartupPhase::TransitioningAl);
        assert!(matches!(
            startup.next_action(now_ns),
            Ok(Some(StartupAction::Al(_)))
        ));
    }

    #[test]
    fn startup_fails_closed_on_sii_configuration_mismatch() {
        let expected = startup_sii_signature(0x6040);
        let observed = startup_sii_signature(0x607A);
        let mut startup = prepared_sii_verification(expected);
        let mut now_ns = 1;
        assert_eq!(
            drive_sii_configuration(&mut startup, &startup_sii_image(0x607A), &mut now_ns),
            Err(StartupError::SiiConfigurationMismatch {
                position: 0,
                expected,
                observed,
            })
        );
        assert_eq!(startup.phase(), StartupPhase::Faulted);
        assert_eq!(startup.verified_sii(0), None);
        assert_eq!(startup.next_action(now_ns), Ok(None));
    }

    #[test]
    fn sii_configuration_actions_and_deadlines_fail_closed() {
        let expected = startup_sii_signature(0x6040);
        let mut crossed = prepared_sii_verification(expected);
        let pending = crossed.next_action(1).unwrap().unwrap();
        let StartupAction::SiiConfiguration(inner) = pending else {
            panic!("expected SII configuration action");
        };
        assert_eq!(
            crossed.accept(StartupAction::Sii(inner), inner.generation, &[], 1, 2),
            Err(StartupError::ActionMismatch)
        );
        assert_eq!(crossed.phase(), StartupPhase::Faulted);
        assert_eq!(crossed.verified_sii(0), None);

        let mut timed_out = prepared_sii_verification(expected);
        timed_out.config.sii_configuration_timeout_ns = 2;
        timed_out.config.request_timeout_ns = 100;
        let pending = timed_out.next_action(10).unwrap().unwrap();
        assert_eq!(
            timed_out.timeout(pending, pending.deadline_ns()),
            Err(StartupError::SiiConfiguration(SiiDiscoveryError::Stream(
                crate::SiiCategoryStreamError::Block(crate::sii::SiiBlockError::Timeout,)
            )))
        );
        assert_eq!(timed_out.phase(), StartupPhase::Faulted);
        assert_eq!(timed_out.verified_sii(0), None);
        assert_eq!(timed_out.next_action(pending.deadline_ns() + 1), Ok(None));
    }

    #[test]
    fn sii_verification_reuses_workspace_in_slave_order_and_restart_clears_evidence() {
        let signatures = [startup_sii_signature(0x6040), startup_sii_signature(0x607A)];
        let identities = [
            SlaveIdentity {
                vendor_id: 1,
                product_code: 2,
                revision: 3,
                serial: 4,
            },
            SlaveIdentity {
                vendor_id: 5,
                product_code: 6,
                revision: 7,
                serial: 8,
            },
        ];
        let mut startup = prepared_multi_sii_verification(signatures, identities);
        let mut now_ns = 1;
        assert_eq!(
            drive_sii_configuration(&mut startup, &startup_sii_image(0x6040), &mut now_ns),
            Ok(StartupProgress::SiiConfigurationVerified(0))
        );
        assert_eq!(startup.verified_sii(0), Some(signatures[0]));
        assert_eq!(startup.verified_sii(1), None);
        assert_eq!(
            accept_al_state(&mut startup, EthercatState::Op, now_ns),
            StartupProgress::SlaveReady(0)
        );
        now_ns += 4;
        assert_eq!(startup.phase(), StartupPhase::ReadingIdentity);
        assert_eq!(startup.current_index(), 1);
        startup.phase = StartupPhase::ReadingConfiguration;
        assert_eq!(
            drive_sii_configuration(&mut startup, &startup_sii_image(0x607A), &mut now_ns),
            Ok(StartupProgress::SiiConfigurationVerified(1))
        );
        assert_eq!(startup.verified_sii(0), Some(signatures[0]));
        assert_eq!(startup.verified_sii(1), Some(signatures[1]));

        assert_eq!(
            accept_al_state(&mut startup, EthercatState::Op, now_ns),
            StartupProgress::Ready
        );
        let expected = startup.expected;
        let profiles = startup.profiles;
        startup
            .start_with_profiles(
                8,
                now_ns + 4,
                StartupConfig::new(EthercatState::Op),
                &expected[..2],
                &profiles[..2],
            )
            .unwrap();
        assert_eq!(startup.phase(), StartupPhase::Scanning);
        assert_eq!(startup.verified_sii(0), None);
        assert_eq!(startup.verified_sii(1), None);
    }

    #[test]
    fn startup_uses_profile_deadline_and_uniform_override_precedence() {
        let timeouts = AlTransitionTimeouts::new(300, 700, 500, 200);
        let mut config = StartupConfig::new(EthercatState::Op);
        config.request_timeout_ns = 10_000;
        let mut startup = prepared_transition(
            EthercatState::SafeOp,
            EthercatState::Op,
            config,
            StartupSlaveProfile::new(0).with_transition_timeouts(timeouts),
            100,
        );
        assert_eq!(
            startup.next_action(101).unwrap().unwrap().deadline_ns(),
            800
        );

        config.transition_timeout_ns = 33;
        let mut startup = prepared_transition(
            EthercatState::SafeOp,
            EthercatState::Op,
            config,
            StartupSlaveProfile::new(0).with_transition_timeouts(timeouts),
            100,
        );
        assert_eq!(
            startup.next_action(101).unwrap().unwrap().deadline_ns(),
            133
        );
    }

    #[test]
    fn startup_disables_op_only_before_non_op_and_enables_only_after_op() {
        let timeouts = AlTransitionTimeouts::new(300, 700, 500, 200);
        let mut config = StartupConfig::new(EthercatState::Op);
        config.request_timeout_ns = 10_000;
        let mut startup = prepared_transition(
            EthercatState::SafeOp,
            EthercatState::Op,
            config,
            op_only_profile(0, timeouts),
            100,
        );

        let disable = startup.next_action(101).unwrap().unwrap();
        assert!(matches!(disable, StartupAction::OpOnly(_)));
        assert_eq!(disable.deadline_ns(), 800);
        assert_eq!(disable.payload(), &[0x28]);
        accept_action(&mut startup, disable, &[], 1, 102);
        let verify = startup.next_action(103).unwrap().unwrap();
        assert_eq!(verify.deadline_ns(), 800);
        assert_eq!(
            accept_action(&mut startup, verify, &[0x28], 1, 104),
            StartupProgress::IdentityVerified(0)
        );

        let al_write = startup.next_action(105).unwrap().unwrap();
        assert!(matches!(al_write, StartupAction::Al(_)));
        assert_eq!(al_write.deadline_ns(), 800);
        accept_action(&mut startup, al_write, &[], 1, 106);
        let al_read = startup.next_action(107).unwrap().unwrap();
        assert_eq!(
            accept_action(&mut startup, al_read, &status(EthercatState::Op), 1, 108,),
            StartupProgress::Advanced
        );
        assert_eq!(startup.phase(), StartupPhase::TransitioningAl);

        assert_eq!(
            accept_op_only(&mut startup, 0x29, 109),
            StartupProgress::Ready
        );
        assert_eq!(startup.phase(), StartupPhase::Ready);
    }

    #[test]
    fn startup_disables_op_only_before_leaving_operational() {
        let timeouts = AlTransitionTimeouts::new(300, 700, 500, 200);
        let mut config = StartupConfig::new(EthercatState::SafeOp);
        config.request_timeout_ns = 10_000;
        let mut startup = prepared_transition(
            EthercatState::Op,
            EthercatState::SafeOp,
            config,
            op_only_profile(0, timeouts),
            50,
        );
        let first = startup.next_action(51).unwrap().unwrap();
        assert!(matches!(first, StartupAction::OpOnly(_)));
        assert_eq!(first.deadline_ns(), 250);
        assert_eq!(first.payload(), &[0x28]);
        accept_action(&mut startup, first, &[], 1, 52);
        let verify = startup.next_action(53).unwrap().unwrap();
        accept_action(&mut startup, verify, &[0x28], 1, 54);
        assert!(matches!(
            startup.next_action(55).unwrap().unwrap(),
            StartupAction::Al(_)
        ));
    }

    #[test]
    fn op_only_readback_failure_never_releases_ready() {
        let mut config = StartupConfig::new(EthercatState::Op);
        config.request_timeout_ns = 10_000;
        let mut startup = prepared_transition(
            EthercatState::SafeOp,
            EthercatState::Op,
            config,
            op_only_profile(0, AlTransitionTimeouts::uniform(1_000)),
            0,
        );
        let write = startup.next_action(1).unwrap().unwrap();
        accept_action(&mut startup, write, &[], 1, 2);
        let read = startup.next_action(3).unwrap().unwrap();
        assert_eq!(
            startup.accept(read, read.generation(), &[0], 1, 4),
            Err(StartupError::OpOnly(
                OpOnlySyncManagerError::ReadbackMismatch
            ))
        );
        assert_eq!(startup.phase(), StartupPhase::Faulted);
        assert_eq!(startup.next_action(5), Ok(None));
    }

    #[test]
    fn profile_validation_happens_before_startup_mutation() {
        let expected = [ExpectedSlave {
            position: 2,
            station_address: 0x1002,
            identity: SlaveIdentity::EMPTY,
        }];
        let invalid =
            [StartupSlaveProfile::new(2)
                .with_transition_timeouts(AlTransitionTimeouts::uniform(0))];
        let mut startup = StartupController::<1>::new(0x1000);
        assert_eq!(
            startup.start_with_profiles(
                1,
                0,
                StartupConfig::new(EthercatState::PreOp),
                &expected,
                &invalid,
            ),
            Err(StartupError::InvalidTransitionTimeoutProfile(2))
        );
        assert_eq!(startup.phase(), StartupPhase::Idle);

        let invalid_mailbox = [StartupSlaveProfile::new(2)
            .with_expected_mailbox(MailboxConfig::new(0, 32, 0x1100, 32))];
        assert_eq!(
            startup.start_with_profiles(
                1,
                0,
                StartupConfig::new(EthercatState::PreOp),
                &expected,
                &invalid_mailbox,
            ),
            Err(StartupError::InvalidMailboxProfile {
                position: 2,
                error: MailboxConfigError::AddressZero(crate::mailbox::MailboxDirection::Send),
            })
        );
        assert_eq!(startup.phase(), StartupPhase::Idle);
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
        let mailbox = MailboxConfig::new(0x1000, 64, 0x1100, 64);
        let profiles = [StartupSlaveProfile::new(0).with_expected_mailbox(mailbox)];
        let mut startup = StartupController::<2>::new(0x1000);
        startup
            .start_with_profiles(
                7,
                0,
                StartupConfig::new(EthercatState::Op),
                &expected,
                &profiles,
            )
            .unwrap();

        let probe = startup.next_action(1).unwrap().unwrap();
        assert!(matches!(probe, StartupAction::Scan(_)));
        assert_eq!(probe.address(), auto_increment_address(0, ESC_TYPE));
        accept_action(&mut startup, probe, &[0x88, 0x02], 1, 2);

        let basic = startup.next_action(3).unwrap().unwrap();
        accept_action(&mut startup, basic, &basic_info(), 1, 4);
        let assign = startup.next_action(5).unwrap().unwrap();
        accept_action(&mut startup, assign, &[], 1, 6);
        let scan_status = startup.next_action(7).unwrap().unwrap();
        assert_eq!(
            scan_status.address(),
            fixed_address(0x1000, crate::ESC_DL_STATUS)
        );
        accept_action(&mut startup, scan_status, &0x5500u16.to_le_bytes(), 1, 8);
        let scan_status = startup.next_action(9).unwrap().unwrap();
        assert_eq!(
            scan_status.address(),
            fixed_address(0x1000, ESC_CONFIGURATION)
        );
        accept_action(&mut startup, scan_status, &[0], 1, 10);
        let scan_status = startup.next_action(11).unwrap().unwrap();
        assert_eq!(scan_status.address(), fixed_address(0x1000, ESC_AL_STATUS));
        accept_action(
            &mut startup,
            scan_status,
            &status(EthercatState::SafeOp),
            1,
            12,
        );
        let end_probe = startup.next_action(13).unwrap().unwrap();
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

        assert_eq!(startup.phase(), StartupPhase::ReadingMailbox);
        let mut mailbox_now = 18;
        assert_eq!(
            accept_mailbox_words(
                &mut startup,
                [0x1000, 64, 0x1100, 64, crate::sii::SII_MAILBOX_PROTOCOL_COE],
                &mut mailbox_now,
            ),
            Ok(StartupProgress::MailboxVerified(0))
        );
        assert_eq!(startup.verified_mailbox(0), Some(mailbox));
        assert_eq!(startup.phase(), StartupPhase::TransitioningAl);
        let write = startup.next_action(mailbox_now).unwrap().unwrap();
        assert!(matches!(write, StartupAction::Al(_)));
        accept_action(&mut startup, write, &[], 1, mailbox_now + 1);
        let read = startup.next_action(mailbox_now + 2).unwrap().unwrap();
        assert_eq!(read.address(), fixed_address(0x1000, ESC_AL_STATUS));
        assert_eq!(
            accept_action(
                &mut startup,
                read,
                &status(EthercatState::Op),
                1,
                mailbox_now + 3,
            ),
            StartupProgress::Ready
        );
        assert_eq!(startup.phase(), StartupPhase::Ready);
        assert_eq!(startup.records().len(), 1);
        assert_eq!(startup.device_emulation(0), Some(false));
        assert_eq!(startup.records()[0].identity, identity);
        assert_eq!(startup.records()[0].al_status.state, EthercatState::Op);
    }

    #[test]
    fn startup_selects_explicit_reference_and_retains_position_evidence() {
        let expected = [
            ExpectedSlave {
                position: 0,
                station_address: 0x1000,
                identity: SlaveIdentity::EMPTY,
            },
            ExpectedSlave {
                position: 1,
                station_address: 0x1001,
                identity: SlaveIdentity::EMPTY,
            },
        ];
        let profiles = [
            StartupSlaveProfile::new(0).with_dc_requirement(StartupDcRequirement::SystemTime),
            StartupSlaveProfile::new(1).with_dc_requirement(StartupDcRequirement::ReferenceClock),
        ];
        let mut startup = StartupController::<2>::new(0x1000);
        startup
            .start_with_profiles(
                31,
                0,
                StartupConfig::new(EthercatState::PreOp),
                &expected,
                &profiles,
            )
            .unwrap();
        let mut now_ns = 1;
        accept_scanned_slave(
            &mut startup,
            crate::ESC_FEATURE_DC_SUPPORTED,
            Some(10),
            &mut now_ns,
        );
        accept_scanned_slave(
            &mut startup,
            crate::ESC_FEATURE_DC_SUPPORTED | crate::ESC_FEATURE_DC_64_BIT,
            Some(20),
            &mut now_ns,
        );
        finish_scan(&mut startup, now_ns).unwrap();

        assert_eq!(startup.phase(), StartupPhase::ReadingIdentity);
        assert_eq!(
            startup.selected_reference_clock(),
            Some(StartupReferenceClock {
                position: 1,
                station_address: 0x1001,
            })
        );
        assert_eq!(startup.dc_capabilities(0).unwrap().system_time, Some(10));
        assert_eq!(
            startup.dc_capabilities(1).unwrap().range,
            crate::EscDcRange::Bits64
        );
        let topology = startup.dc_topology().unwrap();
        assert_eq!(topology.reference_position(), Some(1));
        assert_eq!(topology.transmission_delay_ns(1), Some(0));
        assert_eq!(topology.transmission_delay_ns(0), Some(100));
    }

    #[test]
    fn startup_uses_first_capable_reference_when_none_is_explicit() {
        let expected = [
            ExpectedSlave {
                position: 0,
                station_address: 0x1000,
                identity: SlaveIdentity::EMPTY,
            },
            ExpectedSlave {
                position: 1,
                station_address: 0x1001,
                identity: SlaveIdentity::EMPTY,
            },
        ];
        let mut startup = StartupController::<2>::new(0x1000);
        startup
            .start(32, 0, StartupConfig::new(EthercatState::PreOp), &expected)
            .unwrap();
        let mut now_ns = 1;
        accept_scanned_slave(
            &mut startup,
            crate::ESC_FEATURE_DC_SUPPORTED,
            Some(10),
            &mut now_ns,
        );
        accept_scanned_slave(
            &mut startup,
            crate::ESC_FEATURE_DC_SUPPORTED,
            Some(20),
            &mut now_ns,
        );
        finish_scan(&mut startup, now_ns).unwrap();

        assert_eq!(
            startup.selected_reference_clock(),
            Some(StartupReferenceClock {
                position: 0,
                station_address: 0x1000,
            })
        );
    }

    #[test]
    fn required_dc_mismatch_faults_before_identity_and_publishes_no_reference() {
        let expected = [ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity: SlaveIdentity::EMPTY,
        }];
        let profiles =
            [StartupSlaveProfile::new(0).with_dc_requirement(StartupDcRequirement::SystemTime)];
        let mut startup = StartupController::<1>::new(0x1000);
        startup
            .start_with_profiles(
                33,
                0,
                StartupConfig::new(EthercatState::PreOp),
                &expected,
                &profiles,
            )
            .unwrap();
        let mut now_ns = 1;
        accept_scanned_slave(
            &mut startup,
            crate::ESC_FEATURE_DC_SUPPORTED,
            None,
            &mut now_ns,
        );

        assert_eq!(
            finish_scan(&mut startup, now_ns),
            Err(StartupError::DcSystemTimeRequired {
                position: 0,
                requirement: StartupDcRequirement::SystemTime,
            })
        );
        assert_eq!(startup.phase(), StartupPhase::Faulted);
        assert_eq!(startup.selected_reference_clock(), None);
        assert_eq!(startup.dc_topology(), None);
        assert_eq!(startup.pending_action(), None);
    }

    #[test]
    fn multiple_explicit_references_fail_transactionally() {
        let expected = [
            ExpectedSlave {
                position: 0,
                station_address: 0x1000,
                identity: SlaveIdentity::EMPTY,
            },
            ExpectedSlave {
                position: 1,
                station_address: 0x1001,
                identity: SlaveIdentity::EMPTY,
            },
        ];
        let profiles = [
            StartupSlaveProfile::new(0).with_dc_requirement(StartupDcRequirement::ReferenceClock),
            StartupSlaveProfile::new(1).with_dc_requirement(StartupDcRequirement::ReferenceClock),
        ];
        let mut startup = StartupController::<2>::new(0x1000);
        startup
            .start_with_profiles(
                34,
                0,
                StartupConfig::new(EthercatState::PreOp),
                &expected,
                &profiles,
            )
            .unwrap();
        let mut now_ns = 1;
        accept_scanned_slave(
            &mut startup,
            crate::ESC_FEATURE_DC_SUPPORTED,
            Some(10),
            &mut now_ns,
        );
        accept_scanned_slave(
            &mut startup,
            crate::ESC_FEATURE_DC_SUPPORTED,
            Some(20),
            &mut now_ns,
        );

        assert_eq!(
            finish_scan(&mut startup, now_ns),
            Err(StartupError::MultipleReferenceClocks {
                first_position: 0,
                second_position: 1,
            })
        );
        assert_eq!(startup.selected_reference_clock(), None);
        assert_eq!(startup.dc_topology(), None);
    }

    #[test]
    fn startup_rejects_required_unmeasurable_dc_before_identity() {
        let expected = [
            ExpectedSlave {
                position: 0,
                station_address: 0x1000,
                identity: SlaveIdentity::EMPTY,
            },
            ExpectedSlave {
                position: 1,
                station_address: 0x1001,
                identity: SlaveIdentity::EMPTY,
            },
            ExpectedSlave {
                position: 2,
                station_address: 0x1002,
                identity: SlaveIdentity::EMPTY,
            },
        ];
        let profiles = [
            StartupSlaveProfile::new(0),
            StartupSlaveProfile::new(1).with_dc_requirement(StartupDcRequirement::ReferenceClock),
            StartupSlaveProfile::new(2).with_dc_requirement(StartupDcRequirement::SystemTime),
        ];
        let mut startup = StartupController::<3>::new(0x1000);
        startup
            .start_with_profiles(
                37,
                0,
                StartupConfig::new(EthercatState::PreOp),
                &expected,
                &profiles,
            )
            .unwrap();
        let mut now_ns = 1;
        accept_scanned_slave_with_evidence(&mut startup, 0, None, [0; 4], 0x1100, &mut now_ns);
        accept_scanned_slave_with_evidence(
            &mut startup,
            crate::ESC_FEATURE_DC_SUPPORTED,
            Some(10),
            [0; 4],
            0x5500,
            &mut now_ns,
        );
        accept_scanned_slave_with_evidence(
            &mut startup,
            crate::ESC_FEATURE_DC_SUPPORTED,
            Some(20),
            [0; 4],
            0x5500,
            &mut now_ns,
        );

        assert_eq!(
            finish_scan(&mut startup, now_ns),
            Err(StartupError::DcPropagationDelayRequired {
                position: 2,
                requirement: StartupDcRequirement::SystemTime,
            })
        );
        assert_eq!(startup.phase(), StartupPhase::Faulted);
        assert_eq!(startup.selected_reference_clock(), None);
        assert_eq!(startup.dc_topology(), None);
        assert_eq!(startup.pending_action(), None);
    }

    #[test]
    fn startup_rejects_malformed_port_topology_transactionally() {
        let expected = [ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity: SlaveIdentity::EMPTY,
        }];
        let mut startup = StartupController::<1>::new(0x1000);
        startup
            .start(38, 0, StartupConfig::new(EthercatState::PreOp), &expected)
            .unwrap();
        let mut now_ns = 1;
        accept_scanned_slave_with_evidence(&mut startup, 0, None, [0; 4], 0x1500, &mut now_ns);

        assert_eq!(
            finish_scan(&mut startup, now_ns),
            Err(StartupError::DcTopology(DcTopologyError::TopologyOverrun {
                position: 0,
                port: 3,
            }))
        );
        assert_eq!(startup.phase(), StartupPhase::Faulted);
        assert_eq!(startup.selected_reference_clock(), None);
        assert_eq!(startup.dc_topology(), None);
    }

    #[test]
    fn startup_restart_clears_dc_evidence_immediately() {
        let expected = [ExpectedSlave {
            position: 0,
            station_address: 0x1000,
            identity: SlaveIdentity::EMPTY,
        }];
        let mut startup = StartupController::<1>::new(0x1000);
        startup
            .start(35, 0, StartupConfig::new(EthercatState::PreOp), &expected)
            .unwrap();
        let mut now_ns = 1;
        accept_scanned_slave(
            &mut startup,
            crate::ESC_FEATURE_DC_SUPPORTED,
            Some(10),
            &mut now_ns,
        );
        finish_scan(&mut startup, now_ns).unwrap();
        assert!(startup.selected_reference_clock().is_some());
        assert!(startup.dc_topology().is_some());

        startup.phase = StartupPhase::Ready;
        startup
            .start(
                36,
                now_ns + 10,
                StartupConfig::new(EthercatState::PreOp),
                &expected,
            )
            .unwrap();
        assert_eq!(startup.selected_reference_clock(), None);
        assert_eq!(startup.dc_topology(), None);
        assert_eq!(startup.dc_capabilities(0), None);
    }

    #[test]
    fn mailbox_verification_ignores_policy_and_fails_closed_on_layout_or_protocol() {
        let mut policy_expected = MailboxConfig::new(0x1000, 64, 0x1100, 64)
            .with_retry_policy(crate::mailbox::MailboxRetryPolicy::new(2, 10))
            .with_status_bit(crate::mailbox::MailboxStatusBit::new(0x1200, 0x08, true));
        policy_expected.timeout_ns = 123;
        let mut startup = prepared_mailbox_verification(policy_expected);
        let mut now_ns = 1;
        assert_eq!(
            accept_mailbox_words(
                &mut startup,
                [0x1000, 64, 0x1100, 64, crate::sii::SII_MAILBOX_PROTOCOL_COE],
                &mut now_ns,
            ),
            Ok(StartupProgress::MailboxVerified(0))
        );
        assert_eq!(
            startup.verified_mailbox(0),
            Some(MailboxConfig::new(0x1000, 64, 0x1100, 64))
        );
        assert!(matches!(
            startup.next_action(now_ns),
            Ok(Some(StartupAction::Al(_)))
        ));

        let expected = MailboxConfig::new(0x1000, 64, 0x1100, 64);
        let mut mismatch = prepared_mailbox_verification(expected);
        let mut now_ns = 1;
        let observed = MailboxConfig::new(0x1000, 32, 0x1100, 64);
        assert_eq!(
            accept_mailbox_words(
                &mut mismatch,
                [0x1000, 32, 0x1100, 64, crate::sii::SII_MAILBOX_PROTOCOL_COE],
                &mut now_ns,
            ),
            Err(StartupError::MailboxMismatch {
                position: 0,
                expected,
                observed,
            })
        );
        assert_eq!(mismatch.phase(), StartupPhase::Faulted);
        assert_eq!(mismatch.verified_mailbox(0), None);
        assert_eq!(mismatch.next_action(now_ns), Ok(None));

        let mut no_coe = prepared_mailbox_verification(expected);
        let mut now_ns = 1;
        assert_eq!(
            accept_mailbox_words(&mut no_coe, [0x1000, 64, 0x1100, 64, 0], &mut now_ns,),
            Err(StartupError::SiiMailbox(SiiMailboxError::CoeUnsupported))
        );
        assert_eq!(no_coe.phase(), StartupPhase::Faulted);
        assert_eq!(no_coe.verified_mailbox(0), None);
    }

    #[test]
    fn mailbox_actions_are_separate_and_timeout_fail_closed() {
        let mailbox = MailboxConfig::new(0x1000, 64, 0x1100, 64);
        let mut crossed = prepared_mailbox_verification(mailbox);
        let pending = crossed.next_action(1).unwrap().unwrap();
        let StartupAction::SiiMailbox(inner) = pending else {
            panic!("expected SII mailbox action");
        };
        assert_eq!(
            crossed.accept(StartupAction::Sii(inner), inner.generation, &[], 1, 2),
            Err(StartupError::ActionMismatch)
        );
        assert_eq!(crossed.phase(), StartupPhase::Faulted);

        let mut timed_out = prepared_mailbox_verification(mailbox);
        let pending = timed_out.next_action(1).unwrap().unwrap();
        assert_eq!(
            timed_out.timeout(pending, pending.deadline_ns()),
            Err(StartupError::SiiMailbox(SiiMailboxError::Block(
                crate::sii::SiiBlockError::Timeout,
            )))
        );
        assert_eq!(timed_out.phase(), StartupPhase::Faulted);
        assert_eq!(timed_out.verified_mailbox(0), None);
    }

    #[test]
    fn mailbox_verification_retains_evidence_in_slave_order() {
        let mailboxes = [
            MailboxConfig::new(0x1000, 64, 0x1100, 64),
            MailboxConfig::new(0x1200, 32, 0x1300, 32),
        ];
        let identities = [
            SlaveIdentity {
                vendor_id: 1,
                product_code: 2,
                revision: 3,
                serial: 4,
            },
            SlaveIdentity {
                vendor_id: 5,
                product_code: 6,
                revision: 7,
                serial: 8,
            },
        ];
        let mut startup = StartupController::<2>::new(0x1000);
        startup.phase = StartupPhase::ReadingMailbox;
        startup.config = StartupConfig::new(EthercatState::Op);
        startup.generation = 9;
        startup.expected_count = 2;
        startup.stage_target = EthercatState::Op;
        for index in 0..2 {
            startup.expected[index] = ExpectedSlave {
                position: index as u16,
                station_address: 0x1000 + index as u16,
                identity: identities[index],
            };
            startup.profiles[index] =
                StartupSlaveProfile::new(index as u16).with_expected_mailbox(mailboxes[index]);
            startup
                .table
                .add(index as u16, 0x1000 + index as u16, identities[index])
                .and_then(|_| {
                    startup.table.observe_status(
                        index as u16,
                        crate::slave::AlStatus::new(EthercatState::SafeOp as u16, 0),
                        0,
                    )
                })
                .and_then(|_| {
                    startup
                        .table
                        .verify_identity(index as u16, identities[index])
                })
                .unwrap();
        }

        let mut now_ns = 1;
        assert_eq!(
            accept_mailbox_words(
                &mut startup,
                [0x1000, 64, 0x1100, 64, crate::sii::SII_MAILBOX_PROTOCOL_COE],
                &mut now_ns,
            ),
            Ok(StartupProgress::MailboxVerified(0))
        );
        assert_eq!(startup.verified_mailbox(0), Some(mailboxes[0]));
        assert_eq!(startup.verified_mailbox(1), None);
        assert_eq!(
            accept_al_state(&mut startup, EthercatState::Op, now_ns),
            StartupProgress::SlaveReady(0)
        );
        assert_eq!(startup.phase(), StartupPhase::ReadingIdentity);

        // The second mailbox gate is entered only after that slave's identity
        // phase has completed; the retained table state models that boundary.
        startup.phase = StartupPhase::ReadingMailbox;
        now_ns += 4;
        assert_eq!(
            accept_mailbox_words(
                &mut startup,
                [0x1200, 32, 0x1300, 32, crate::sii::SII_MAILBOX_PROTOCOL_COE],
                &mut now_ns,
            ),
            Ok(StartupProgress::MailboxVerified(1))
        );
        assert_eq!(startup.verified_mailbox(0), Some(mailboxes[0]));
        assert_eq!(startup.verified_mailbox(1), Some(mailboxes[1]));
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
        accept_action(&mut startup, basic, &basic_info(), 1, 4);
        let assign = startup.next_action(5).unwrap().unwrap();
        accept_action(&mut startup, assign, &[], 1, 6);
        let scan_status = startup.next_action(7).unwrap().unwrap();
        accept_action(&mut startup, scan_status, &0x5500u16.to_le_bytes(), 1, 8);
        let scan_status = startup.next_action(9).unwrap().unwrap();
        accept_action(&mut startup, scan_status, &[0], 1, 10);
        let scan_status = startup.next_action(11).unwrap().unwrap();
        accept_action(
            &mut startup,
            scan_status,
            &status(EthercatState::Init),
            1,
            12,
        );
        let end_probe = startup.next_action(13).unwrap().unwrap();
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
        for index in 0..expected.len() {
            let probe = startup.next_action(now_ns).unwrap().unwrap();
            accept_action(&mut startup, probe, &[0x88, 0x02], 1, now_ns + 1);
            let basic = startup.next_action(now_ns + 2).unwrap().unwrap();
            accept_action(&mut startup, basic, &basic_info(), 1, now_ns + 3);
            let assign = startup.next_action(now_ns + 4).unwrap().unwrap();
            accept_action(&mut startup, assign, &[], 1, now_ns + 5);
            let scan_status = startup.next_action(now_ns + 6).unwrap().unwrap();
            let dl_status = if index + 1 < expected.len() {
                0x1500u16
            } else {
                0x5500u16
            };
            accept_action(
                &mut startup,
                scan_status,
                &dl_status.to_le_bytes(),
                1,
                now_ns + 7,
            );
            let scan_status = startup.next_action(now_ns + 8).unwrap().unwrap();
            accept_action(&mut startup, scan_status, &[0], 1, now_ns + 9);
            let scan_status = startup.next_action(now_ns + 10).unwrap().unwrap();
            accept_action(
                &mut startup,
                scan_status,
                &status(EthercatState::Init),
                1,
                now_ns + 11,
            );
            now_ns += 12;
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
        accept_action(
            &mut startup,
            basic,
            &[0; crate::BASIC_ESC_INFO_LEN as usize],
            1,
            4,
        );
        let assign = startup.next_action(5).unwrap().unwrap();
        accept_action(&mut startup, assign, &[], 1, 6);
        let dl_status = startup.next_action(7).unwrap().unwrap();
        accept_action(&mut startup, dl_status, &0x5500u16.to_le_bytes(), 1, 8);
        let configuration = startup.next_action(9).unwrap().unwrap();
        assert_eq!(
            configuration.address(),
            fixed_address(0x1000, ESC_CONFIGURATION)
        );
        accept_action(
            &mut startup,
            configuration,
            &[crate::registers::ESC_DEVICE_EMULATION],
            1,
            10,
        );
        let status_action = startup.next_action(11).unwrap().unwrap();
        accept_action(
            &mut startup,
            status_action,
            &status_with_code(EthercatState::Init, true, 0x0011),
            1,
            12,
        );
        let end_probe = startup.next_action(13).unwrap().unwrap();
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
            StartupAction::Sii(_)
            | StartupAction::SiiMailbox(_)
            | StartupAction::SiiConfiguration(_)
            | StartupAction::Al(_)
            | StartupAction::OpOnly(_) => 0,
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
        accept_action(
            &mut startup,
            basic,
            &[0; crate::BASIC_ESC_INFO_LEN as usize],
            1,
            4,
        );
        let assign = startup.next_action(5).unwrap().unwrap();
        accept_action(&mut startup, assign, &[], 1, 6);
        let scan_status_action = startup.next_action(7).unwrap().unwrap();
        accept_action(
            &mut startup,
            scan_status_action,
            &0x5500u16.to_le_bytes(),
            1,
            8,
        );
        let scan_status_action = startup.next_action(9).unwrap().unwrap();
        accept_action(&mut startup, scan_status_action, &[0], 1, 10);
        let scan_status_action = startup.next_action(11).unwrap().unwrap();
        accept_action(
            &mut startup,
            scan_status_action,
            &status(EthercatState::SafeOp),
            1,
            12,
        );
        let end_probe = startup.next_action(13).unwrap().unwrap();
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
    fn completed_mailbox_control_request_preserves_action_ownership() {
        let mut startup = prepared_mailbox_verification(MailboxConfig::new(0x1000, 64, 0x1100, 64));
        let action = startup.next_action(1).unwrap().unwrap();
        assert!(matches!(action, StartupAction::SiiMailbox(_)));

        let mut pool = ControlRequestPool::<1>::new();
        let handle = startup.enqueue_pending(&mut pool).unwrap();
        let request = pool.get(handle).unwrap();
        assert_eq!(request.length, 4);
        assert_eq!(request.response_length, 4);
        assert_eq!(action.response_len(), 0);
        let mut frame = [0; crate::wire::MAX_ETHERNET_FRAME_LEN];
        pool.build_into_buffer(handle, &mut frame, [0xFF; 6], [1, 2, 3, 4, 5, 6])
            .unwrap();
        pool.complete(
            handle,
            action.generation(),
            action.address(),
            action.payload(),
            1,
        )
        .unwrap();
        assert_eq!(
            startup.accept_completed(&mut pool, handle, 2),
            Ok(StartupProgress::Advanced)
        );
        assert_eq!(pool.in_use(), 0);
        assert_eq!(startup.phase(), StartupPhase::ReadingMailbox);
        assert!(matches!(
            startup.next_action(3),
            Ok(Some(StartupAction::SiiMailbox(_)))
        ));
    }

    #[test]
    fn completed_sii_configuration_request_preserves_action_ownership() {
        let mut startup = prepared_sii_verification(startup_sii_signature(0x6040));
        let action = startup.next_action(1).unwrap().unwrap();
        assert!(matches!(action, StartupAction::SiiConfiguration(_)));

        let mut pool = ControlRequestPool::<1>::new();
        let handle = startup.enqueue_pending(&mut pool).unwrap();
        let request = pool.get(handle).unwrap();
        assert_eq!(request.length, action.datagram_len());
        assert_eq!(request.response_length, action.datagram_len());
        assert_eq!(action.response_len(), 0);
        let mut frame = [0; crate::wire::MAX_ETHERNET_FRAME_LEN];
        pool.build_into_buffer(handle, &mut frame, [0xFF; 6], [1, 2, 3, 4, 5, 6])
            .unwrap();
        pool.complete(
            handle,
            action.generation(),
            action.address(),
            action.payload(),
            1,
        )
        .unwrap();
        assert_eq!(
            startup.accept_completed(&mut pool, handle, 2),
            Ok(StartupProgress::Advanced)
        );
        assert_eq!(pool.in_use(), 0);
        assert_eq!(startup.phase(), StartupPhase::ReadingConfiguration);
        assert!(matches!(
            startup.next_action(3),
            Ok(Some(StartupAction::SiiConfiguration(_)))
        ));
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
