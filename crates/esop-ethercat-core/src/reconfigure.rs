//! Explicit bounded reconfiguration of one retained EtherCAT slave.

use crate::al::{AlAction, AlErrorAcknowledgePolicy, AlTransitionTimeouts};
use crate::control::{ControlError, ControlRequestPool, RegisterOperation, RequestHandle};
use crate::dc::{
    DcClockAction, DcClockConfig, DcClockController, DcClockError, DcClockPhase, DcSyncAction,
    DcSyncConfig, DcSyncController, DcSyncError, DcSyncPhase, DcSyncPlan, DcTopology,
};
use crate::fmmu_discovery::{FmmuRegisterBank, FmmuRegisterDescriptor};
use crate::mailbox::{
    MailboxConfig, MailboxConfigError, MailboxController, MailboxError, MailboxProgress,
};
use crate::mapping::{MAX_ESC_FMMUS, MappingTable};
use crate::mapping_config::{
    MappingConfigAction, MappingConfigController, MappingConfigError, MappingConfigPhase,
};
use crate::op_only::{
    OpOnlySyncManagerAction, OpOnlySyncManagerController, OpOnlySyncManagerError,
    OpOnlySyncManagerPhase, OpOnlySyncManagerProfile,
};
use crate::pdo_config::{
    PdoConfigAction, PdoConfigController, PdoConfigError, PdoConfigPhase, PdoConfigPlan,
};
use crate::slave::{AlStatus, EthercatState, SlaveIdentity};
use crate::state_request::{
    StateRequestConfig, StateRequestController, StateRequestError, StateRequestObservation,
    StateRequestPhase, StateRequestProgress,
};
use crate::sync_manager_discovery::{SyncManagerRegisterBank, SyncManagerRegisterDescriptor};
use crate::watchdog::{
    EscWatchdogConfig, WatchdogAction, WatchdogController, WatchdogControllerConfig, WatchdogError,
    WatchdogPhase, WatchdogPlan, WatchdogPlanEntry,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconfigureSlaveHandle {
    sequence: u32,
}

impl ReconfigureSlaveHandle {
    pub const fn sequence(self) -> u32 {
        self.sequence
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ReconfigureSlavePhase {
    Idle = 0,
    DisablingOpOnly = 1,
    MovingToPreOp = 2,
    ConfiguringPdo = 3,
    ConfiguringWatchdog = 4,
    ConfiguringMapping = 5,
    ConfiguringDcClock = 6,
    ConfiguringDcSync = 7,
    Complete = 8,
    Faulted = 9,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconfigureSlaveTransport {
    None,
    Control,
    Mailbox,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconfigureSlavePlan<
    const MAX_SLAVES: usize,
    const SMS: usize,
    const FMMUS: usize,
    const PDO_OPS: usize,
> {
    pub position: u16,
    pub station_address: u16,
    pub identity: SlaveIdentity,
    pub mailbox: Option<MailboxConfig>,
    pub pdo: PdoConfigPlan<PDO_OPS>,
    pub mapping: MappingTable<SMS, FMMUS>,
    pub watchdog: Option<EscWatchdogConfig>,
    pub dc_clock_required: bool,
    pub dc_sync_required: bool,
    pub dc_sync: DcSyncPlan<MAX_SLAVES>,
    pub dc_sync_delay_ns: u64,
}

impl<const MAX_SLAVES: usize, const SMS: usize, const FMMUS: usize, const PDO_OPS: usize>
    ReconfigureSlavePlan<MAX_SLAVES, SMS, FMMUS, PDO_OPS>
{
    pub const fn new(
        position: u16,
        station_address: u16,
        identity: SlaveIdentity,
        mapping: MappingTable<SMS, FMMUS>,
    ) -> Self {
        Self {
            position,
            station_address,
            identity,
            mailbox: None,
            pdo: PdoConfigPlan::new(),
            mapping,
            watchdog: None,
            dc_clock_required: false,
            dc_sync_required: false,
            dc_sync: DcSyncPlan::new(None),
            dc_sync_delay_ns: crate::DC_SYNC_DELAY_NS,
        }
    }

    pub const fn with_pdo(mut self, mailbox: MailboxConfig, pdo: PdoConfigPlan<PDO_OPS>) -> Self {
        self.mailbox = Some(mailbox);
        self.pdo = pdo;
        self
    }

    pub const fn with_watchdog(mut self, watchdog: EscWatchdogConfig) -> Self {
        self.watchdog = Some(watchdog);
        self
    }

    pub const fn with_dc(
        mut self,
        dc_clock_required: bool,
        dc_sync_required: bool,
        dc_sync: DcSyncPlan<MAX_SLAVES>,
        dc_sync_delay_ns: u64,
    ) -> Self {
        self.dc_clock_required = dc_clock_required;
        self.dc_sync_required = dc_sync_required;
        self.dc_sync = dc_sync;
        self.dc_sync_delay_ns = dc_sync_delay_ns;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconfigureSlaveContext<const MAX_SLAVES: usize> {
    pub observed_status: AlStatus,
    pub generation: u16,
    pub now_ns: u64,
    pub deadline_ns: u64,
    pub request_timeout_ns: u64,
    pub transition_timeouts: AlTransitionTimeouts,
    pub error_acknowledge_policy: AlErrorAcknowledgePolicy,
    pub op_only_outputs: OpOnlySyncManagerProfile,
    pub sync_manager_registers: SyncManagerRegisterBank,
    pub fmmu_registers: FmmuRegisterBank,
    pub dc_topology: Option<DcTopology<MAX_SLAVES>>,
    pub application_time_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconfigureSlaveResult {
    pub handle: ReconfigureSlaveHandle,
    pub position: u16,
    pub station_address: u16,
    pub identity: SlaveIdentity,
    pub observed_status: AlStatus,
    pub completed_at_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconfigureSlaveProgress {
    Advanced(ReconfigureSlavePhase),
    StateObserved(StateRequestObservation),
    Complete(ReconfigureSlaveResult),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconfigureSlaveError {
    Busy,
    FaultLatched,
    StartupNotReady,
    UnknownPosition(u16),
    SlaveOffline(u16),
    SlaveUnconfigured(u16),
    InvalidHandle,
    ResultNotReady,
    InvalidStationAddress,
    InvalidDeadline,
    OperationDeadlineExceeded,
    InvalidRequestTimeout,
    IdentityMismatch,
    StationMismatch,
    MailboxRequired,
    UnexpectedMailbox,
    MailboxMismatch,
    MailboxConfig(MailboxConfigError),
    OpOnlyProfileMismatch,
    MissingDcTopology,
    ActionMismatch,
    RetainedStateMismatch,
    Control(ControlError),
    OpOnly(OpOnlySyncManagerError),
    StateRequest(StateRequestError),
    Pdo(PdoConfigError),
    Watchdog(WatchdogError),
    Mapping(MappingConfigError),
    DcClock(DcClockError),
    DcSync(DcSyncError),
    Mailbox(MailboxError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconfigureSlaveStatus {
    pub handle: ReconfigureSlaveHandle,
    pub phase: ReconfigureSlavePhase,
    pub position: u16,
    pub station_address: u16,
    pub identity: SlaveIdentity,
    pub observed_status: AlStatus,
    pub generation: u16,
    pub deadline_ns: u64,
    pub latest_observation: Option<StateRequestObservation>,
    pub error: Option<ReconfigureSlaveError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconfigureSlaveControlAction {
    OpOnly(OpOnlySyncManagerAction),
    StateRequest(AlAction),
    Watchdog(WatchdogAction),
    Mapping(MappingConfigAction),
    DcClock(DcClockAction),
    DcSync(DcSyncAction),
}

impl ReconfigureSlaveControlAction {
    pub const fn datagram_index(self) -> u8 {
        match self {
            Self::OpOnly(action) => action.datagram_index,
            Self::StateRequest(action) => action.datagram_index,
            Self::Watchdog(action) => action.datagram_index,
            Self::Mapping(action) => action.datagram_index,
            Self::DcClock(action) => action.datagram_index,
            Self::DcSync(action) => action.datagram_index,
        }
    }

    pub const fn generation(self) -> u16 {
        match self {
            Self::OpOnly(action) => action.generation,
            Self::StateRequest(action) => action.generation,
            Self::Watchdog(action) => action.generation,
            Self::Mapping(action) => action.generation,
            Self::DcClock(action) => action.generation,
            Self::DcSync(action) => action.generation,
        }
    }

    pub const fn address(self) -> u32 {
        match self {
            Self::OpOnly(action) => action.address,
            Self::StateRequest(action) => action.address,
            Self::Watchdog(action) => action.address,
            Self::Mapping(action) => action.address,
            Self::DcClock(action) => action.address,
            Self::DcSync(action) => action.address,
        }
    }

    pub const fn operation(self) -> RegisterOperation {
        match self {
            Self::OpOnly(action) => action.operation,
            Self::StateRequest(action) => action.operation,
            Self::Watchdog(action) => action.operation,
            Self::Mapping(action) => action.operation,
            Self::DcClock(action) => action.operation,
            Self::DcSync(action) => action.operation,
        }
    }

    pub fn payload(&self) -> &[u8] {
        match self {
            Self::OpOnly(action) => action.payload(),
            Self::StateRequest(action) => action.payload(),
            Self::Watchdog(action) => action.payload(),
            Self::Mapping(action) => action.payload(),
            Self::DcClock(action) => action.payload(),
            Self::DcSync(action) => action.payload(),
        }
    }

    pub const fn datagram_len(self) -> usize {
        match self {
            Self::OpOnly(action) => action.datagram_len(),
            Self::StateRequest(action) => action.datagram_len(),
            Self::Watchdog(action) => action.datagram_len(),
            Self::Mapping(action) => action.datagram_len(),
            Self::DcClock(action) => action.datagram_len(),
            Self::DcSync(action) => action.datagram_len(),
        }
    }

    pub const fn deadline_ns(self) -> u64 {
        match self {
            Self::OpOnly(action) => action.deadline_ns,
            Self::StateRequest(action) => action.deadline_ns,
            Self::Watchdog(action) => action.deadline_ns,
            Self::Mapping(action) => action.deadline_ns,
            Self::DcClock(action) => action.deadline_ns,
            Self::DcSync(action) => action.deadline_ns,
        }
    }
}

pub struct ReconfigureSlaveController<
    const MAX_SLAVES: usize,
    const SMS: usize,
    const FMMUS: usize,
    const PDO_OPS: usize,
> {
    phase: ReconfigureSlavePhase,
    handle: Option<ReconfigureSlaveHandle>,
    next_sequence: u32,
    plan: ReconfigureSlavePlan<MAX_SLAVES, SMS, FMMUS, PDO_OPS>,
    context: ReconfigureSlaveContext<MAX_SLAVES>,
    observed_status: AlStatus,
    latest_observation: Option<StateRequestObservation>,
    observation_pending: bool,
    result: Option<ReconfigureSlaveResult>,
    last_error: Option<ReconfigureSlaveError>,
    op_only: OpOnlySyncManagerController,
    state_request: StateRequestController,
    pdo: PdoConfigController<PDO_OPS>,
    mailbox: MailboxController,
    pdo_action: Option<PdoConfigAction>,
    watchdog: WatchdogController<1>,
    mapping: MappingConfigController<SMS, FMMUS>,
    dc_clock: DcClockController<MAX_SLAVES>,
    dc_sync: DcSyncController<MAX_SLAVES>,
}

impl<const MAX_SLAVES: usize, const SMS: usize, const FMMUS: usize, const PDO_OPS: usize>
    ReconfigureSlaveController<MAX_SLAVES, SMS, FMMUS, PDO_OPS>
{
    pub const fn new() -> Self {
        Self {
            phase: ReconfigureSlavePhase::Idle,
            handle: None,
            next_sequence: 1,
            plan: ReconfigureSlavePlan::new(0, 0, SlaveIdentity::EMPTY, MappingTable::new()),
            context: ReconfigureSlaveContext {
                observed_status: AlStatus::new(0, 0),
                generation: 0,
                now_ns: 0,
                deadline_ns: 0,
                request_timeout_ns: 0,
                transition_timeouts: AlTransitionTimeouts::uniform(1),
                error_acknowledge_policy: AlErrorAcknowledgePolicy::Disabled,
                op_only_outputs: OpOnlySyncManagerProfile::EMPTY,
                sync_manager_registers: SyncManagerRegisterBank::from_parts(
                    0,
                    0,
                    0,
                    [SyncManagerRegisterDescriptor::RESET; crate::MAX_ESC_SYNC_MANAGERS],
                ),
                fmmu_registers: FmmuRegisterBank::from_parts(
                    0,
                    0,
                    0,
                    [FmmuRegisterDescriptor::RESET; MAX_ESC_FMMUS],
                ),
                dc_topology: None,
                application_time_ns: 0,
            },
            observed_status: AlStatus::new(0, 0),
            latest_observation: None,
            observation_pending: false,
            result: None,
            last_error: None,
            op_only: OpOnlySyncManagerController::new(),
            state_request: StateRequestController::new(),
            pdo: PdoConfigController::new(),
            mailbox: MailboxController::new(),
            pdo_action: None,
            watchdog: WatchdogController::new(),
            mapping: MappingConfigController::new(),
            dc_clock: DcClockController::new(),
            dc_sync: DcSyncController::new(),
        }
    }

    pub const fn phase(&self) -> ReconfigureSlavePhase {
        self.phase
    }

    pub const fn active_handle(&self) -> Option<ReconfigureSlaveHandle> {
        self.handle
    }

    pub const fn last_error(&self) -> Option<ReconfigureSlaveError> {
        self.last_error
    }

    pub const fn is_active_or_faulted(&self) -> bool {
        matches!(
            self.phase,
            ReconfigureSlavePhase::DisablingOpOnly
                | ReconfigureSlavePhase::MovingToPreOp
                | ReconfigureSlavePhase::ConfiguringPdo
                | ReconfigureSlavePhase::ConfiguringWatchdog
                | ReconfigureSlavePhase::ConfiguringMapping
                | ReconfigureSlavePhase::ConfiguringDcClock
                | ReconfigureSlavePhase::ConfiguringDcSync
                | ReconfigureSlavePhase::Faulted
        )
    }

    pub const fn transport(&self) -> ReconfigureSlaveTransport {
        match self.phase {
            ReconfigureSlavePhase::Idle
            | ReconfigureSlavePhase::Complete
            | ReconfigureSlavePhase::Faulted => ReconfigureSlaveTransport::None,
            ReconfigureSlavePhase::ConfiguringPdo => ReconfigureSlaveTransport::Mailbox,
            ReconfigureSlavePhase::DisablingOpOnly
            | ReconfigureSlavePhase::MovingToPreOp
            | ReconfigureSlavePhase::ConfiguringWatchdog
            | ReconfigureSlavePhase::ConfiguringMapping
            | ReconfigureSlavePhase::ConfiguringDcClock
            | ReconfigureSlavePhase::ConfiguringDcSync => ReconfigureSlaveTransport::Control,
        }
    }

    pub fn status(
        &self,
        handle: ReconfigureSlaveHandle,
    ) -> Result<ReconfigureSlaveStatus, ReconfigureSlaveError> {
        if self.handle != Some(handle) {
            return Err(ReconfigureSlaveError::InvalidHandle);
        }
        Ok(self.status_unchecked(handle))
    }

    pub fn result(
        &self,
        handle: ReconfigureSlaveHandle,
    ) -> Result<ReconfigureSlaveResult, ReconfigureSlaveError> {
        if self.handle != Some(handle) {
            return Err(ReconfigureSlaveError::InvalidHandle);
        }
        self.result.ok_or(ReconfigureSlaveError::ResultNotReady)
    }

    pub fn start(
        &mut self,
        plan: ReconfigureSlavePlan<MAX_SLAVES, SMS, FMMUS, PDO_OPS>,
        context: ReconfigureSlaveContext<MAX_SLAVES>,
    ) -> Result<ReconfigureSlaveHandle, ReconfigureSlaveError> {
        match self.phase {
            ReconfigureSlavePhase::DisablingOpOnly
            | ReconfigureSlavePhase::MovingToPreOp
            | ReconfigureSlavePhase::ConfiguringPdo
            | ReconfigureSlavePhase::ConfiguringWatchdog
            | ReconfigureSlavePhase::ConfiguringMapping
            | ReconfigureSlavePhase::ConfiguringDcClock
            | ReconfigureSlavePhase::ConfiguringDcSync => {
                return Err(ReconfigureSlaveError::Busy);
            }
            ReconfigureSlavePhase::Idle
            | ReconfigureSlavePhase::Complete
            | ReconfigureSlavePhase::Faulted => {}
        }
        validate_plan(&plan, context)?;

        let handle = ReconfigureSlaveHandle {
            sequence: self.next_sequence,
        };
        let next_sequence = self.next_sequence.wrapping_add(1).max(1);
        let mut staged = Self::new();
        staged.handle = Some(handle);
        staged.next_sequence = next_sequence;
        staged.plan = plan;
        staged.context = context;
        staged.observed_status = context.observed_status;
        staged.begin(context.now_ns)?;
        *self = staged;
        Ok(handle)
    }

    pub fn pending_control_action(&self) -> Option<ReconfigureSlaveControlAction> {
        match self.phase {
            ReconfigureSlavePhase::DisablingOpOnly => self
                .op_only
                .pending()
                .map(ReconfigureSlaveControlAction::OpOnly),
            ReconfigureSlavePhase::MovingToPreOp => self
                .state_request
                .pending()
                .map(ReconfigureSlaveControlAction::StateRequest),
            ReconfigureSlavePhase::ConfiguringWatchdog => self
                .watchdog
                .pending()
                .map(ReconfigureSlaveControlAction::Watchdog),
            ReconfigureSlavePhase::ConfiguringMapping => self
                .mapping
                .pending()
                .map(ReconfigureSlaveControlAction::Mapping),
            ReconfigureSlavePhase::ConfiguringDcClock => self
                .dc_clock
                .pending()
                .map(ReconfigureSlaveControlAction::DcClock),
            ReconfigureSlavePhase::ConfiguringDcSync => self
                .dc_sync
                .pending()
                .map(ReconfigureSlaveControlAction::DcSync),
            ReconfigureSlavePhase::Idle
            | ReconfigureSlavePhase::ConfiguringPdo
            | ReconfigureSlavePhase::Complete
            | ReconfigureSlavePhase::Faulted => None,
        }
    }

    pub fn next_control_action(
        &mut self,
        now_ns: u64,
    ) -> Result<Option<ReconfigureSlaveControlAction>, ReconfigureSlaveError> {
        self.ensure_deadline(now_ns)?;
        let action = match self.phase {
            ReconfigureSlavePhase::DisablingOpOnly => self
                .op_only
                .next_action(now_ns)
                .map(|action| action.map(ReconfigureSlaveControlAction::OpOnly))
                .map_err(ReconfigureSlaveError::OpOnly),
            ReconfigureSlavePhase::MovingToPreOp => self
                .state_request
                .next_action(now_ns)
                .map(|action| action.map(ReconfigureSlaveControlAction::StateRequest))
                .map_err(ReconfigureSlaveError::StateRequest),
            ReconfigureSlavePhase::ConfiguringWatchdog => self
                .watchdog
                .next_action(now_ns)
                .map(|action| action.map(ReconfigureSlaveControlAction::Watchdog))
                .map_err(ReconfigureSlaveError::Watchdog),
            ReconfigureSlavePhase::ConfiguringMapping => self
                .mapping
                .next_action(now_ns)
                .map(|action| action.map(ReconfigureSlaveControlAction::Mapping))
                .map_err(ReconfigureSlaveError::Mapping),
            ReconfigureSlavePhase::ConfiguringDcClock => self
                .dc_clock
                .next_action(now_ns)
                .map(|action| action.map(ReconfigureSlaveControlAction::DcClock))
                .map_err(ReconfigureSlaveError::DcClock),
            ReconfigureSlavePhase::ConfiguringDcSync => self
                .dc_sync
                .next_action(now_ns)
                .map(|action| action.map(ReconfigureSlaveControlAction::DcSync))
                .map_err(ReconfigureSlaveError::DcSync),
            _ => return Ok(None),
        };
        action.map_err(|error| self.fail_value(error))
    }

    pub fn enqueue_control_pending<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
    ) -> Result<RequestHandle, ReconfigureSlaveError> {
        let result = match self.phase {
            ReconfigureSlavePhase::DisablingOpOnly => self
                .op_only
                .enqueue_pending(pool)
                .map_err(ReconfigureSlaveError::Control),
            ReconfigureSlavePhase::MovingToPreOp => self
                .state_request
                .enqueue_pending(pool)
                .map_err(ReconfigureSlaveError::StateRequest),
            ReconfigureSlavePhase::ConfiguringWatchdog => self
                .watchdog
                .enqueue_pending(pool)
                .map_err(ReconfigureSlaveError::Control),
            ReconfigureSlavePhase::ConfiguringMapping => self
                .mapping
                .enqueue_pending(pool)
                .map_err(ReconfigureSlaveError::Control),
            ReconfigureSlavePhase::ConfiguringDcClock => self
                .dc_clock
                .enqueue_pending(pool)
                .map_err(ReconfigureSlaveError::Control),
            ReconfigureSlavePhase::ConfiguringDcSync => self
                .dc_sync
                .enqueue_pending(pool)
                .map_err(ReconfigureSlaveError::Control),
            _ => Err(ReconfigureSlaveError::ActionMismatch),
        };
        result.map_err(|error| self.fail_value(error))
    }

    pub fn accept_control_completed<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
        handle: RequestHandle,
        now_ns: u64,
    ) -> Result<ReconfigureSlaveProgress, ReconfigureSlaveError> {
        self.ensure_deadline(now_ns)?;
        let complete = match self.phase {
            ReconfigureSlavePhase::DisablingOpOnly => self
                .op_only
                .accept_completed(pool, handle, now_ns)
                .map(|progress| {
                    progress == crate::OpOnlySyncManagerProgress::Complete
                        || self.op_only.phase() == OpOnlySyncManagerPhase::Complete
                })
                .map_err(ReconfigureSlaveError::OpOnly),
            ReconfigureSlavePhase::MovingToPreOp => {
                let progress = self
                    .state_request
                    .accept_completed(pool, handle, now_ns)
                    .map_err(ReconfigureSlaveError::StateRequest);
                if let Some(observation) = self.state_request.take_observation() {
                    self.observed_status = observation.observed_status;
                    self.latest_observation = Some(observation);
                    self.observation_pending = true;
                }
                progress.map(|progress| matches!(progress, StateRequestProgress::Complete(_)))
            }
            ReconfigureSlavePhase::ConfiguringWatchdog => self
                .watchdog
                .accept_completed(pool, handle, now_ns)
                .map(|_| self.watchdog.phase() == WatchdogPhase::Complete)
                .map_err(ReconfigureSlaveError::Watchdog),
            ReconfigureSlavePhase::ConfiguringMapping => self
                .mapping
                .accept_completed(pool, handle, now_ns)
                .map(|_| self.mapping.phase() == MappingConfigPhase::Complete)
                .map_err(ReconfigureSlaveError::Mapping),
            ReconfigureSlavePhase::ConfiguringDcClock => self
                .dc_clock
                .accept_completed(pool, handle, now_ns)
                .map(|_| self.dc_clock.phase() == DcClockPhase::Complete)
                .map_err(ReconfigureSlaveError::DcClock),
            ReconfigureSlavePhase::ConfiguringDcSync => self
                .dc_sync
                .accept_completed(pool, handle, now_ns)
                .map(|_| self.dc_sync.phase() == DcSyncPhase::Complete)
                .map_err(ReconfigureSlaveError::DcSync),
            _ => Err(ReconfigureSlaveError::ActionMismatch),
        };
        let complete = complete.map_err(|error| self.fail_value(error))?;
        if complete {
            self.advance(now_ns)
        } else if let Some(observation) =
            self.latest_observation.filter(|_| self.observation_pending)
        {
            Ok(ReconfigureSlaveProgress::StateObserved(observation))
        } else {
            Ok(ReconfigureSlaveProgress::Advanced(self.phase))
        }
    }

    pub fn timeout_control_action(
        &mut self,
        action: ReconfigureSlaveControlAction,
        now_ns: u64,
    ) -> Result<ReconfigureSlaveProgress, ReconfigureSlaveError> {
        let result = match (self.phase, action) {
            (
                ReconfigureSlavePhase::DisablingOpOnly,
                ReconfigureSlaveControlAction::OpOnly(action),
            ) => self
                .op_only
                .timeout(action, now_ns)
                .map(|_| ())
                .map_err(ReconfigureSlaveError::OpOnly),
            (
                ReconfigureSlavePhase::MovingToPreOp,
                ReconfigureSlaveControlAction::StateRequest(action),
            ) => self
                .state_request
                .timeout(action, now_ns)
                .map(|_| ())
                .map_err(ReconfigureSlaveError::StateRequest),
            (
                ReconfigureSlavePhase::ConfiguringWatchdog,
                ReconfigureSlaveControlAction::Watchdog(action),
            ) => self
                .watchdog
                .timeout(action, now_ns)
                .map(|_| ())
                .map_err(ReconfigureSlaveError::Watchdog),
            (
                ReconfigureSlavePhase::ConfiguringMapping,
                ReconfigureSlaveControlAction::Mapping(action),
            ) => self
                .mapping
                .timeout(action, now_ns)
                .map(|_| ())
                .map_err(ReconfigureSlaveError::Mapping),
            (
                ReconfigureSlavePhase::ConfiguringDcClock,
                ReconfigureSlaveControlAction::DcClock(action),
            ) => self
                .dc_clock
                .timeout(action, now_ns)
                .map(|_| ())
                .map_err(ReconfigureSlaveError::DcClock),
            (
                ReconfigureSlavePhase::ConfiguringDcSync,
                ReconfigureSlaveControlAction::DcSync(action),
            ) => self
                .dc_sync
                .timeout(action, now_ns)
                .map(|_| ())
                .map_err(ReconfigureSlaveError::DcSync),
            _ => Err(ReconfigureSlaveError::ActionMismatch),
        };
        match result {
            Ok(()) => Ok(ReconfigureSlaveProgress::Advanced(self.phase)),
            Err(error) => Err(self.fail_value(error)),
        }
    }

    pub const fn pending_pdo_action(&self) -> Option<PdoConfigAction> {
        self.pdo_action
    }

    pub const fn mailbox(&self) -> &MailboxController {
        &self.mailbox
    }

    pub fn mailbox_mut(&mut self) -> &mut MailboxController {
        &mut self.mailbox
    }

    pub fn prepare_mailbox(
        &mut self,
        now_ns: u64,
    ) -> Result<Option<ReconfigureSlaveProgress>, ReconfigureSlaveError> {
        self.ensure_deadline(now_ns)?;
        if self.phase != ReconfigureSlavePhase::ConfiguringPdo {
            return Err(self.fail_value(ReconfigureSlaveError::ActionMismatch));
        }
        if self.pdo_action.is_some() {
            return Ok(None);
        }
        let action = self
            .pdo
            .next_action(now_ns)
            .map_err(ReconfigureSlaveError::Pdo)
            .map_err(|error| self.fail_value(error))?;
        let Some(action) = action else {
            if self.pdo.phase() == PdoConfigPhase::Complete {
                return self.advance(now_ns).map(Some);
            }
            return Ok(None);
        };
        if action.deadline_ns <= now_ns {
            return match self.pdo.timeout(action, now_ns) {
                Ok(_) => Ok(Some(ReconfigureSlaveProgress::Advanced(self.phase))),
                Err(error) => Err(self.fail_value(ReconfigureSlaveError::Pdo(error))),
            };
        }
        let mut mailbox = self
            .plan
            .mailbox
            .ok_or_else(|| self.fail_value(ReconfigureSlaveError::MailboxRequired))?;
        let remaining_ns = action.deadline_ns.saturating_sub(now_ns);
        mailbox.timeout_ns = mailbox.timeout_ns.min(remaining_ns);
        mailbox.request_timeout_ns = mailbox.request_timeout_ns.min(remaining_ns);
        self.mailbox
            .start(
                mailbox,
                action.station_address,
                action.generation,
                now_ns,
                crate::MailboxProtocol::CoE,
                action.payload(),
            )
            .map_err(ReconfigureSlaveError::Mailbox)
            .map_err(|error| self.fail_value(error))?;
        self.pdo_action = Some(action);
        Ok(None)
    }

    pub fn accept_mailbox_progress(
        &mut self,
        progress: Result<MailboxProgress, MailboxError>,
        now_ns: u64,
    ) -> Result<ReconfigureSlaveProgress, ReconfigureSlaveError> {
        let action = self
            .pdo_action
            .ok_or_else(|| self.fail_value(ReconfigureSlaveError::ActionMismatch))?;
        match progress {
            Ok(MailboxProgress::Complete) => {
                let mut response = [0; crate::MAX_MAILBOX_BYTES];
                let response_len = self
                    .mailbox
                    .response()
                    .map(|(_, payload)| {
                        response[..payload.len()].copy_from_slice(payload);
                        payload.len()
                    })
                    .ok_or_else(|| {
                        self.fail_value(ReconfigureSlaveError::Mailbox(
                            MailboxError::NoPendingAction,
                        ))
                    })?;
                self.pdo_action = None;
                self.pdo
                    .accept(action, action.generation, &response[..response_len], now_ns)
                    .map_err(ReconfigureSlaveError::Pdo)
                    .map_err(|error| self.fail_value(error))?;
                if self.pdo.phase() == PdoConfigPhase::Complete {
                    self.advance(now_ns)
                } else {
                    Ok(ReconfigureSlaveProgress::Advanced(self.phase))
                }
            }
            Ok(_) => Ok(ReconfigureSlaveProgress::Advanced(self.phase)),
            Err(error) => {
                self.pdo_action = None;
                match self.pdo.mailbox_failed(action, error) {
                    Ok(_) => Ok(ReconfigureSlaveProgress::Advanced(self.phase)),
                    Err(error) => Err(self.fail_value(ReconfigureSlaveError::Pdo(error))),
                }
            }
        }
    }

    pub fn take_observation(&mut self) -> Option<StateRequestObservation> {
        if !self.observation_pending {
            return None;
        }
        self.observation_pending = false;
        self.latest_observation
    }

    pub(crate) fn status_unchecked(
        &self,
        handle: ReconfigureSlaveHandle,
    ) -> ReconfigureSlaveStatus {
        ReconfigureSlaveStatus {
            handle,
            phase: self.phase,
            position: self.plan.position,
            station_address: self.plan.station_address,
            identity: self.plan.identity,
            observed_status: self.observed_status,
            generation: self.context.generation,
            deadline_ns: self.context.deadline_ns,
            latest_observation: self.latest_observation,
            error: self.last_error,
        }
    }

    pub(crate) fn abort(&mut self, error: ReconfigureSlaveError) -> ReconfigureSlaveError {
        self.fail_value(error)
    }

    fn begin(&mut self, now_ns: u64) -> Result<ReconfigureSlaveProgress, ReconfigureSlaveError> {
        if self.context.op_only_outputs.is_empty() {
            self.start_state_request(now_ns)?;
        } else {
            let request_timeout_ns = self.request_timeout(now_ns)?;
            self.op_only
                .start(
                    self.plan.station_address,
                    self.context.generation,
                    self.context.deadline_ns,
                    request_timeout_ns,
                    self.context.op_only_outputs,
                    false,
                )
                .map_err(ReconfigureSlaveError::OpOnly)?;
            self.phase = ReconfigureSlavePhase::DisablingOpOnly;
        }
        if self.phase == ReconfigureSlavePhase::MovingToPreOp
            && self.state_request.phase() == StateRequestPhase::Complete
        {
            self.advance(now_ns)
        } else {
            Ok(ReconfigureSlaveProgress::Advanced(self.phase))
        }
    }

    fn start_state_request(&mut self, now_ns: u64) -> Result<(), ReconfigureSlaveError> {
        let request_timeout_ns = self.request_timeout(now_ns)?;
        self.state_request
            .start_direct_preop(StateRequestConfig {
                position: self.plan.position,
                station_address: self.plan.station_address,
                observed_status: self.observed_status,
                requested_state: EthercatState::PreOp,
                generation: self.context.generation,
                now_ns,
                deadline_ns: self.context.deadline_ns,
                request_timeout_ns,
                transition_timeouts: self.context.transition_timeouts,
                error_acknowledge_policy: self.context.error_acknowledge_policy,
            })
            .map_err(ReconfigureSlaveError::StateRequest)?;
        self.phase = ReconfigureSlavePhase::MovingToPreOp;
        Ok(())
    }

    fn advance(&mut self, now_ns: u64) -> Result<ReconfigureSlaveProgress, ReconfigureSlaveError> {
        self.ensure_deadline(now_ns)?;
        for _ in 0..8 {
            match self.phase {
                ReconfigureSlavePhase::DisablingOpOnly => {
                    self.start_state_request(now_ns)?;
                    if self.state_request.phase() != StateRequestPhase::Complete {
                        return Ok(ReconfigureSlaveProgress::Advanced(self.phase));
                    }
                }
                ReconfigureSlavePhase::MovingToPreOp => {
                    if self.observed_status.state != EthercatState::PreOp {
                        return Err(self.fail_value(ReconfigureSlaveError::RetainedStateMismatch));
                    }
                    if !self.plan.pdo.is_empty() {
                        let timeout_ns = self.remaining(now_ns)?;
                        let request_timeout_ns = self.request_timeout(now_ns)?;
                        self.pdo
                            .start(
                                self.plan.pdo,
                                self.plan.station_address,
                                self.context.generation,
                                now_ns,
                                timeout_ns,
                                request_timeout_ns,
                            )
                            .map_err(ReconfigureSlaveError::Pdo)
                            .map_err(|error| self.fail_value(error))?;
                        self.phase = ReconfigureSlavePhase::ConfiguringPdo;
                        return Ok(ReconfigureSlaveProgress::Advanced(self.phase));
                    }
                    self.phase = ReconfigureSlavePhase::ConfiguringPdo;
                }
                ReconfigureSlavePhase::ConfiguringPdo => {
                    if let Some(config) = self.plan.watchdog {
                        let mut watchdog = WatchdogPlan::new();
                        watchdog
                            .push(WatchdogPlanEntry {
                                position: self.plan.position,
                                station_address: self.plan.station_address,
                                config,
                            })
                            .map_err(|_| {
                                ReconfigureSlaveError::Watchdog(WatchdogError::InvalidConfiguration)
                            })?;
                        let timeout_ns = self.remaining(now_ns)?;
                        let request_timeout_ns = self.request_timeout(now_ns)?;
                        self.watchdog
                            .start(
                                WatchdogControllerConfig {
                                    timeout_ns,
                                    request_timeout_ns,
                                },
                                &watchdog,
                                self.context.generation,
                                now_ns,
                            )
                            .map_err(ReconfigureSlaveError::Watchdog)
                            .map_err(|error| self.fail_value(error))?;
                        self.phase = ReconfigureSlavePhase::ConfiguringWatchdog;
                        return Ok(ReconfigureSlaveProgress::Advanced(self.phase));
                    }
                    self.phase = ReconfigureSlavePhase::ConfiguringWatchdog;
                }
                ReconfigureSlavePhase::ConfiguringWatchdog => {
                    let timeout_ns = self.remaining(now_ns)?;
                    let request_timeout_ns = self.request_timeout(now_ns)?;
                    self.mapping
                        .start_with_verified_registers(
                            self.plan.station_address,
                            self.context.generation,
                            now_ns,
                            timeout_ns,
                            request_timeout_ns,
                            self.context.sync_manager_registers,
                            self.context.fmmu_registers,
                            &self.plan.mapping,
                        )
                        .map_err(ReconfigureSlaveError::Mapping)
                        .map_err(|error| self.fail_value(error))?;
                    self.phase = ReconfigureSlavePhase::ConfiguringMapping;
                    if self.mapping.phase() != MappingConfigPhase::Complete {
                        return Ok(ReconfigureSlaveProgress::Advanced(self.phase));
                    }
                }
                ReconfigureSlavePhase::ConfiguringMapping => {
                    if self.plan.dc_clock_required {
                        let topology = match self.context.dc_topology {
                            Some(topology) => topology,
                            None => {
                                return Err(
                                    self.fail_value(ReconfigureSlaveError::MissingDcTopology)
                                );
                            }
                        };
                        let timeout_ns = self.remaining(now_ns)?;
                        let request_timeout_ns = self.request_timeout(now_ns)?;
                        self.dc_clock
                            .start_target(
                                DcClockConfig {
                                    timeout_ns,
                                    request_timeout_ns,
                                },
                                &topology,
                                self.plan.position,
                                self.context.generation,
                                self.context.application_time_ns,
                                now_ns,
                            )
                            .map_err(ReconfigureSlaveError::DcClock)
                            .map_err(|error| self.fail_value(error))?;
                        self.phase = ReconfigureSlavePhase::ConfiguringDcClock;
                        return Ok(ReconfigureSlaveProgress::Advanced(self.phase));
                    }
                    self.phase = ReconfigureSlavePhase::ConfiguringDcClock;
                }
                ReconfigureSlavePhase::ConfiguringDcClock => {
                    if self.plan.dc_sync_required {
                        let topology = match self.context.dc_topology {
                            Some(topology) => topology,
                            None => {
                                return Err(
                                    self.fail_value(ReconfigureSlaveError::MissingDcTopology)
                                );
                            }
                        };
                        let timeout_ns = self.remaining(now_ns)?;
                        let request_timeout_ns = self.request_timeout(now_ns)?;
                        self.dc_sync
                            .start_target(
                                DcSyncConfig {
                                    sync_delay_ns: self.plan.dc_sync_delay_ns,
                                    timeout_ns,
                                    request_timeout_ns,
                                },
                                &self.plan.dc_sync,
                                &topology,
                                self.plan.position,
                                self.context.generation,
                                now_ns,
                            )
                            .map_err(ReconfigureSlaveError::DcSync)
                            .map_err(|error| self.fail_value(error))?;
                        self.phase = ReconfigureSlavePhase::ConfiguringDcSync;
                        return Ok(ReconfigureSlaveProgress::Advanced(self.phase));
                    }
                    self.phase = ReconfigureSlavePhase::ConfiguringDcSync;
                }
                ReconfigureSlavePhase::ConfiguringDcSync => return self.complete(now_ns),
                ReconfigureSlavePhase::Complete => {
                    return Ok(ReconfigureSlaveProgress::Complete(
                        self.result.ok_or(ReconfigureSlaveError::ResultNotReady)?,
                    ));
                }
                ReconfigureSlavePhase::Idle | ReconfigureSlavePhase::Faulted => {
                    return Err(self.fail_value(ReconfigureSlaveError::ActionMismatch));
                }
            }
        }
        Err(self.fail_value(ReconfigureSlaveError::ActionMismatch))
    }

    fn complete(&mut self, now_ns: u64) -> Result<ReconfigureSlaveProgress, ReconfigureSlaveError> {
        if self.observed_status.state != EthercatState::PreOp || self.observed_status.error {
            return Err(self.fail_value(ReconfigureSlaveError::RetainedStateMismatch));
        }
        let result = ReconfigureSlaveResult {
            handle: self.handle.ok_or(ReconfigureSlaveError::InvalidHandle)?,
            position: self.plan.position,
            station_address: self.plan.station_address,
            identity: self.plan.identity,
            observed_status: self.observed_status,
            completed_at_ns: now_ns,
        };
        self.result = Some(result);
        self.phase = ReconfigureSlavePhase::Complete;
        Ok(ReconfigureSlaveProgress::Complete(result))
    }

    fn remaining(&mut self, now_ns: u64) -> Result<u64, ReconfigureSlaveError> {
        self.context
            .deadline_ns
            .checked_sub(now_ns)
            .filter(|remaining| *remaining != 0)
            .ok_or_else(|| self.fail_value(ReconfigureSlaveError::OperationDeadlineExceeded))
    }

    fn request_timeout(&mut self, now_ns: u64) -> Result<u64, ReconfigureSlaveError> {
        Ok(self.context.request_timeout_ns.min(self.remaining(now_ns)?))
    }

    fn ensure_deadline(&mut self, now_ns: u64) -> Result<(), ReconfigureSlaveError> {
        if now_ns >= self.context.deadline_ns {
            return Err(self.fail_value(ReconfigureSlaveError::OperationDeadlineExceeded));
        }
        Ok(())
    }

    fn fail_value(&mut self, error: ReconfigureSlaveError) -> ReconfigureSlaveError {
        if self.last_error.is_none() {
            self.last_error = Some(error);
        }
        self.phase = ReconfigureSlavePhase::Faulted;
        self.result = None;
        self.pdo_action = None;
        self.last_error.unwrap_or(error)
    }
}

impl<const MAX_SLAVES: usize, const SMS: usize, const FMMUS: usize, const PDO_OPS: usize> Default
    for ReconfigureSlaveController<MAX_SLAVES, SMS, FMMUS, PDO_OPS>
{
    fn default() -> Self {
        Self::new()
    }
}

fn validate_plan<
    const MAX_SLAVES: usize,
    const SMS: usize,
    const FMMUS: usize,
    const PDO_OPS: usize,
>(
    plan: &ReconfigureSlavePlan<MAX_SLAVES, SMS, FMMUS, PDO_OPS>,
    context: ReconfigureSlaveContext<MAX_SLAVES>,
) -> Result<(), ReconfigureSlaveError> {
    if plan.station_address == 0 {
        return Err(ReconfigureSlaveError::InvalidStationAddress);
    }
    if context.deadline_ns <= context.now_ns {
        return Err(ReconfigureSlaveError::InvalidDeadline);
    }
    if context.request_timeout_ns == 0 {
        return Err(ReconfigureSlaveError::InvalidRequestTimeout);
    }
    if plan.pdo.is_empty() && plan.mailbox.is_some() {
        return Err(ReconfigureSlaveError::UnexpectedMailbox);
    }
    if !plan.pdo.is_empty() {
        plan.mailbox
            .ok_or(ReconfigureSlaveError::MailboxRequired)?
            .validate()
            .map_err(ReconfigureSlaveError::MailboxConfig)?;
    }
    if let Some(watchdog) = plan.watchdog {
        watchdog
            .validate()
            .map_err(|_| ReconfigureSlaveError::Watchdog(WatchdogError::InvalidConfiguration))?;
    }
    if plan.mapping.op_only_outputs() != context.op_only_outputs {
        return Err(ReconfigureSlaveError::OpOnlyProfileMismatch);
    }
    if (plan.dc_clock_required || plan.dc_sync_required) && context.dc_topology.is_none() {
        return Err(ReconfigureSlaveError::MissingDcTopology);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FMMU_IMAGE_LEN;
    use crate::coe::{CoeHeader, CoeService};
    use crate::mailbox::{MailboxHeader, MailboxProtocol};
    use crate::mapping::{FmmuConfig, SyncManagerConfig};
    use crate::op_only::{SYNC_MANAGER_ENABLE_FLAG, SYNC_MANAGER_OP_ONLY_FLAG};
    use crate::pdo_config::{PdoConfigStep, PdoSdoWrite};

    const IDENTITY: SlaveIdentity = SlaveIdentity {
        vendor_id: 1,
        product_code: 2,
        revision: 3,
        serial: 4,
    };

    fn context<const MAX_SLAVES: usize>(
        state: EthercatState,
    ) -> ReconfigureSlaveContext<MAX_SLAVES> {
        ReconfigureSlaveContext {
            observed_status: AlStatus::new(state as u16, 0),
            generation: 7,
            now_ns: 10,
            deadline_ns: 10_000,
            request_timeout_ns: 100,
            transition_timeouts: AlTransitionTimeouts::uniform(1_000),
            error_acknowledge_policy: AlErrorAcknowledgePolicy::Enabled,
            op_only_outputs: OpOnlySyncManagerProfile::EMPTY,
            sync_manager_registers: SyncManagerRegisterBank::from_parts(
                0,
                0x1000,
                0,
                [SyncManagerRegisterDescriptor::RESET; crate::MAX_ESC_SYNC_MANAGERS],
            ),
            fmmu_registers: FmmuRegisterBank::from_parts(
                0,
                0x1000,
                0,
                [FmmuRegisterDescriptor::RESET; MAX_ESC_FMMUS],
            ),
            dc_topology: None,
            application_time_ns: 0,
        }
    }

    fn empty_plan() -> ReconfigureSlavePlan<1, 0, 0, 0> {
        ReconfigureSlavePlan::new(0, 0x1000, IDENTITY, MappingTable::new())
    }

    fn complete_control_action<
        const MAX_SLAVES: usize,
        const SMS: usize,
        const FMMUS: usize,
        const PDO_OPS: usize,
    >(
        controller: &mut ReconfigureSlaveController<MAX_SLAVES, SMS, FMMUS, PDO_OPS>,
        pool: &mut ControlRequestPool<1>,
        response: &[u8],
        now_ns: u64,
    ) -> Result<ReconfigureSlaveProgress, ReconfigureSlaveError> {
        let action = controller.pending_control_action().unwrap();
        let handle = controller.enqueue_control_pending(pool)?;
        let mut frame = [0; crate::wire::MAX_ETHERNET_FRAME_LEN];
        pool.build_into_buffer(handle, &mut frame, [0xFF; 6], [1, 2, 3, 4, 5, 6])
            .unwrap();
        pool.complete(handle, action.generation(), action.address(), response, 1)
            .unwrap();
        controller.accept_control_completed(pool, handle, now_ns)
    }

    fn pdo_download_response(action: PdoConfigAction) -> [u8; 6] {
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
        response
    }

    fn pdo_upload_response(action: PdoConfigAction, data: &[u8]) -> [u8; 10] {
        let mut response = [0; 10];
        CoeHeader {
            number: 0,
            service: CoeService::SdoResponse,
        }
        .encode(&mut response)
        .unwrap();
        response[2] = 0x43 | (((4 - data.len()) as u8) << 2);
        response[3..5].copy_from_slice(&action.sdo_index.to_le_bytes());
        response[5] = action.sdo_subindex;
        response[6..6 + data.len()].copy_from_slice(data);
        response
    }

    fn complete_mailbox_transaction<const SMS: usize, const FMMUS: usize, const PDO_OPS: usize>(
        controller: &mut ReconfigureSlaveController<1, SMS, FMMUS, PDO_OPS>,
        response: &[u8],
        now_ns: u64,
    ) -> ReconfigureSlaveProgress {
        assert_eq!(controller.prepare_mailbox(now_ns).unwrap(), None);
        let send = controller
            .mailbox_mut()
            .next_action(now_ns + 1)
            .unwrap()
            .unwrap();
        assert_eq!(send.station_address, 0x1000);
        assert_eq!(send.operation, RegisterOperation::Write);
        let request_header = MailboxHeader::decode(send.payload()).unwrap();
        let mut echoed = [0; crate::MAX_MAILBOX_BYTES];
        echoed[..send.datagram_len()].copy_from_slice(send.payload());
        assert_eq!(
            controller.mailbox_mut().accept(
                send,
                send.generation,
                &echoed[..send.datagram_len()],
                1,
                now_ns + 2,
            ),
            Ok(MailboxProgress::Advanced)
        );

        let poll = controller
            .mailbox_mut()
            .next_action(now_ns + 3)
            .unwrap()
            .unwrap();
        assert_eq!(poll.station_address, 0x1000);
        assert_eq!(poll.operation, RegisterOperation::Read);
        let mut frame = [0; crate::MAX_MAILBOX_BYTES];
        MailboxHeader {
            length: response.len() as u16,
            address: 0,
            priority: 0,
            protocol: MailboxProtocol::CoE,
            counter: request_header.counter,
        }
        .encode(&mut frame)
        .unwrap();
        frame[6..6 + response.len()].copy_from_slice(response);
        assert_eq!(
            controller.mailbox_mut().accept(
                poll,
                poll.generation,
                &frame[..poll.datagram_len()],
                1,
                now_ns + 4,
            ),
            Ok(MailboxProgress::Complete)
        );
        controller
            .accept_mailbox_progress(Ok(MailboxProgress::Complete), now_ns + 4)
            .unwrap()
    }

    #[test]
    fn complete_operation_retains_result_and_replaces_stale_handle_transactionally() {
        let mut controller = ReconfigureSlaveController::<1, 0, 0, 0>::new();
        let first = controller
            .start(empty_plan(), context(EthercatState::PreOp))
            .unwrap();
        assert_eq!(controller.phase(), ReconfigureSlavePhase::Complete);
        assert_eq!(controller.result(first).unwrap().position, 0);

        let mut invalid = context(EthercatState::PreOp);
        invalid.deadline_ns = invalid.now_ns;
        assert_eq!(
            controller.start(empty_plan(), invalid),
            Err(ReconfigureSlaveError::InvalidDeadline)
        );
        assert_eq!(controller.result(first).unwrap().position, 0);

        let second = controller
            .start(empty_plan(), context(EthercatState::PreOp))
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(
            controller.status(first),
            Err(ReconfigureSlaveError::InvalidHandle)
        );
        assert_eq!(
            controller.result(second).unwrap().observed_status.state,
            EthercatState::PreOp
        );
    }

    #[test]
    fn first_fault_is_retained_until_a_new_explicit_operation() {
        let mut controller = ReconfigureSlaveController::<1, 0, 0, 0>::new();
        let first = controller
            .start(empty_plan(), context(EthercatState::Op))
            .unwrap();
        let action = controller.next_control_action(11).unwrap().unwrap();
        let ReconfigureSlaveControlAction::StateRequest(al) = action else {
            panic!("expected direct PREOP state request");
        };
        assert_eq!(al.payload(), &(EthercatState::PreOp as u16).to_le_bytes());
        let error = controller
            .timeout_control_action(action, action.deadline_ns())
            .unwrap_err();
        assert!(matches!(error, ReconfigureSlaveError::StateRequest(_)));
        assert_eq!(controller.phase(), ReconfigureSlavePhase::Faulted);
        assert_eq!(controller.status(first).unwrap().error, Some(error));
        assert_eq!(
            controller.abort(ReconfigureSlaveError::StationMismatch),
            error
        );

        let second = controller
            .start(empty_plan(), context(EthercatState::PreOp))
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(controller.phase(), ReconfigureSlavePhase::Complete);
        assert_eq!(controller.last_error(), None);
    }

    #[test]
    fn op_only_outputs_are_disabled_and_verified_before_preop_request() {
        let activation = SYNC_MANAGER_OP_ONLY_FLAG | SYNC_MANAGER_ENABLE_FLAG | 0x20;
        let mut mapping = MappingTable::<1, 0>::new();
        mapping
            .add_sync_manager(SyncManagerConfig {
                index: 2,
                physical_start: 0x1200,
                length: 2,
                control: 0x24,
                status: 0,
                enable: true,
            })
            .unwrap();
        mapping.mark_op_only_output(2, activation).unwrap();
        let profile = mapping.op_only_outputs();
        let plan = ReconfigureSlavePlan::<1, 1, 0, 0>::new(0, 0x1000, IDENTITY, mapping);
        let mut operation_context = context(EthercatState::Op);
        operation_context.op_only_outputs = profile;
        operation_context.sync_manager_registers = SyncManagerRegisterBank::from_parts(
            0,
            0x1000,
            3,
            [SyncManagerRegisterDescriptor::RESET; crate::MAX_ESC_SYNC_MANAGERS],
        );
        let mut controller = ReconfigureSlaveController::new();
        controller.start(plan, operation_context).unwrap();

        let first = controller.next_control_action(11).unwrap().unwrap();
        let ReconfigureSlaveControlAction::OpOnly(first) = first else {
            panic!("expected OpOnly write before AL transition");
        };
        assert_eq!(first.operation, RegisterOperation::Write);
        assert_eq!(first.payload(), &[activation & !SYNC_MANAGER_ENABLE_FLAG]);
        let mut pool = ControlRequestPool::<1>::new();
        complete_control_action(
            &mut controller,
            &mut pool,
            &[activation & !SYNC_MANAGER_ENABLE_FLAG],
            12,
        )
        .unwrap();

        let verify = controller.next_control_action(13).unwrap().unwrap();
        let ReconfigureSlaveControlAction::OpOnly(verify) = verify else {
            panic!("expected OpOnly readback before AL transition");
        };
        assert_eq!(verify.operation, RegisterOperation::Read);
        complete_control_action(
            &mut controller,
            &mut pool,
            &[activation & !SYNC_MANAGER_ENABLE_FLAG],
            14,
        )
        .unwrap();

        assert_eq!(controller.phase(), ReconfigureSlavePhase::MovingToPreOp);
        assert!(matches!(
            controller.next_control_action(15).unwrap(),
            Some(ReconfigureSlaveControlAction::StateRequest(_))
        ));
    }

    #[test]
    fn pdo_watchdog_and_mapping_execute_in_order_on_target() {
        let mut pdo = PdoConfigPlan::<1>::new();
        pdo.push(PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap())
            .unwrap();
        let mut mapping = MappingTable::<1, 1>::new();
        mapping
            .add_sync_manager(SyncManagerConfig {
                index: 0,
                physical_start: 0x1000,
                length: 2,
                control: 0x26,
                status: 0,
                enable: true,
            })
            .unwrap();
        mapping
            .add_fmmu(FmmuConfig {
                index: 0,
                logical_start: 0x2000,
                length: 2,
                logical_start_bit: 0,
                logical_end_bit: 7,
                physical_start: 0x1000,
                physical_start_bit: 0,
                fmmu_type: 2,
                enable: true,
            })
            .unwrap();
        let plan = ReconfigureSlavePlan::<1, 1, 1, 1>::new(0, 0x1000, IDENTITY, mapping)
            .with_pdo(MailboxConfig::new(0x1000, 32, 0x1100, 32), pdo)
            .with_watchdog(EscWatchdogConfig::new(Some(2_500), Some(100)));
        let mut operation_context = context(EthercatState::PreOp);
        operation_context.sync_manager_registers = SyncManagerRegisterBank::from_parts(
            0,
            0x1000,
            1,
            [SyncManagerRegisterDescriptor::RESET; crate::MAX_ESC_SYNC_MANAGERS],
        );
        operation_context.fmmu_registers = FmmuRegisterBank::from_parts(
            0,
            0x1000,
            1,
            [FmmuRegisterDescriptor::RESET; MAX_ESC_FMMUS],
        );
        let mut controller = ReconfigureSlaveController::new();
        let handle = controller.start(plan, operation_context).unwrap();
        assert_eq!(controller.phase(), ReconfigureSlavePhase::ConfiguringPdo);

        assert_eq!(controller.prepare_mailbox(10).unwrap(), None);
        let download = controller.pending_pdo_action().unwrap();
        assert_eq!(download.step, PdoConfigStep::Download);
        let progress =
            complete_mailbox_transaction(&mut controller, &pdo_download_response(download), 20);
        assert_eq!(
            progress,
            ReconfigureSlaveProgress::Advanced(ReconfigureSlavePhase::ConfiguringPdo)
        );
        assert_eq!(controller.prepare_mailbox(25).unwrap(), None);
        let upload = controller.pending_pdo_action().unwrap();
        assert_eq!(upload.step, PdoConfigStep::VerifyUpload);
        let progress =
            complete_mailbox_transaction(&mut controller, &pdo_upload_response(upload, &[0]), 30);
        assert_eq!(
            progress,
            ReconfigureSlaveProgress::Advanced(ReconfigureSlavePhase::ConfiguringWatchdog)
        );

        let mut pool = ControlRequestPool::<1>::new();
        let mut watchdog_value = [0; 2];
        while controller.phase() == ReconfigureSlavePhase::ConfiguringWatchdog {
            let action = controller.next_control_action(40).unwrap().unwrap();
            let ReconfigureSlaveControlAction::Watchdog(action) = action else {
                panic!("watchdog phase exposed a non-watchdog action");
            };
            assert_eq!(action.station_address, 0x1000);
            let response = if action.operation == RegisterOperation::Write {
                watchdog_value.copy_from_slice(action.payload());
                action.payload()
            } else {
                &watchdog_value
            };
            complete_control_action(&mut controller, &mut pool, response, 41).unwrap();
        }
        assert_eq!(
            controller.phase(),
            ReconfigureSlavePhase::ConfiguringMapping
        );

        let mut last_mapping = [0; FMMU_IMAGE_LEN];
        let mut last_mapping_len = 0usize;
        while controller.phase() == ReconfigureSlavePhase::ConfiguringMapping {
            let action = controller.next_control_action(50).unwrap().unwrap();
            let ReconfigureSlaveControlAction::Mapping(action) = action else {
                panic!("mapping phase exposed a non-mapping action");
            };
            assert_eq!(action.station_address, 0x1000);
            let response = if action.operation == RegisterOperation::Write {
                last_mapping.fill(0);
                last_mapping[..action.payload().len()].copy_from_slice(action.payload());
                last_mapping_len = action.payload().len();
                action.payload()
            } else {
                &last_mapping[..last_mapping_len]
            };
            complete_control_action(&mut controller, &mut pool, response, 51).unwrap();
        }
        assert_eq!(controller.phase(), ReconfigureSlavePhase::Complete);
        assert_eq!(controller.result(handle).unwrap().position, 0);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn operation_deadline_caps_child_request_and_latches_expiry() {
        let mut operation_context = context(EthercatState::Op);
        operation_context.deadline_ns = 50;
        operation_context.request_timeout_ns = 1_000;
        let mut controller = ReconfigureSlaveController::<1, 0, 0, 0>::new();
        let handle = controller.start(empty_plan(), operation_context).unwrap();
        let action = controller.next_control_action(11).unwrap().unwrap();
        assert_eq!(action.deadline_ns(), 50);
        assert_eq!(
            controller.next_control_action(50),
            Err(ReconfigureSlaveError::OperationDeadlineExceeded)
        );
        assert_eq!(controller.phase(), ReconfigureSlavePhase::Faulted);
        assert_eq!(
            controller.status(handle).unwrap().error,
            Some(ReconfigureSlaveError::OperationDeadlineExceeded)
        );
    }
}
