//! Bounded asynchronous runtime EtherCAT state requests.
//!
//! The controller owns one explicit request and composes the existing AL
//! transition FSM for each legal ESM step. It does not scan, reconfigure, or
//! retry after an uncertain wire result.

use crate::al::{
    AlAction, AlError, AlErrorAcknowledgePolicy, AlFaultRecord, AlPhase, AlProgress,
    AlTransitionController, AlTransitionRequest, AlTransitionTimeouts,
};
use crate::control::{
    ControlError, ControlRequestPool, MAX_CONTROL_PAYLOAD, RequestHandle, RequestState,
};
use crate::op_only::{
    OpOnlySyncManagerAction, OpOnlySyncManagerController, OpOnlySyncManagerError,
    OpOnlySyncManagerProfile, OpOnlySyncManagerProgress,
};
use crate::slave::{AlStatus, EthercatState, next_state};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateRequestAction {
    Al(AlAction),
    OpOnly(OpOnlySyncManagerAction),
}

impl StateRequestAction {
    pub const fn deadline_ns(self) -> u64 {
        match self {
            Self::Al(action) => action.deadline_ns,
            Self::OpOnly(action) => action.deadline_ns,
        }
    }

    pub(crate) fn matches_request(self, request: &crate::control::ControlRequest) -> bool {
        match self {
            Self::Al(action) => request.matches_action(
                action.datagram_index,
                action.generation,
                action.address,
                action.operation,
                action.payload(),
                action.datagram_len(),
                action.deadline_ns,
            ),
            Self::OpOnly(action) => request.matches_action(
                action.datagram_index,
                action.generation,
                action.address,
                action.operation,
                action.payload(),
                action.datagram_len(),
                action.deadline_ns,
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransitionStage {
    Al,
    Disable,
    Enable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StateRequestHandle {
    sequence: u32,
}

impl StateRequestHandle {
    pub const fn sequence(self) -> u32 {
        self.sequence
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StateRequestPhase {
    Idle = 0,
    Transitioning = 1,
    Complete = 2,
    Faulted = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StateRequestConfig {
    pub position: u16,
    pub station_address: u16,
    pub observed_status: AlStatus,
    pub requested_state: EthercatState,
    pub generation: u16,
    pub now_ns: u64,
    pub deadline_ns: u64,
    pub request_timeout_ns: u64,
    pub transition_timeouts: AlTransitionTimeouts,
    pub error_acknowledge_policy: AlErrorAcknowledgePolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StateRequestObservation {
    pub handle: StateRequestHandle,
    pub position: u16,
    pub station_address: u16,
    pub requested_state: EthercatState,
    pub observed_status: AlStatus,
    pub generation: u16,
    pub deadline_ns: u64,
    pub observed_at_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StateRequestResult {
    pub observation: StateRequestObservation,
    pub completed_at_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateRequestProgress {
    OpOnly(OpOnlySyncManagerProgress),
    ControlWritten,
    Polling,
    StepReached(StateRequestObservation),
    Complete(StateRequestResult),
}

impl StateRequestProgress {
    pub const fn observation(self) -> Option<StateRequestObservation> {
        match self {
            Self::StepReached(observation) => Some(observation),
            Self::Complete(result) => Some(result.observation),
            Self::ControlWritten | Self::Polling | Self::OpOnly(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateRequestError {
    Busy,
    FaultLatched,
    InvalidHandle,
    ResultNotReady,
    InvalidStationAddress,
    InvalidDeadline,
    InvalidRequestTimeout,
    InvalidTransitionTimeouts,
    InvalidTransition,
    ActionMismatch,
    StartupNotReady,
    UnknownPosition(u16),
    SlaveOffline(u16),
    SlaveUnconfigured(u16),
    OpOnlyUnsupported(u16),
    RetainedStationMismatch {
        position: u16,
        expected: u16,
        observed: u16,
    },
    RetainedStatusMismatch {
        position: u16,
        expected: AlStatus,
        observed: AlStatus,
    },
    RetainedResultMismatch(u16),
    Control(ControlError),
    Al(AlError),
    OpOnly(OpOnlySyncManagerError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StateRequestStatus {
    pub handle: StateRequestHandle,
    pub phase: StateRequestPhase,
    pub position: u16,
    pub station_address: u16,
    pub requested_state: EthercatState,
    pub observed_status: AlStatus,
    pub generation: u16,
    pub deadline_ns: u64,
    pub latest_observation: Option<StateRequestObservation>,
    pub error: Option<StateRequestError>,
}

pub struct StateRequestController {
    phase: StateRequestPhase,
    handle: Option<StateRequestHandle>,
    next_sequence: u32,
    position: u16,
    station_address: u16,
    requested_state: EthercatState,
    observed_status: AlStatus,
    generation: u16,
    deadline_ns: u64,
    request_timeout_ns: u64,
    transition_timeouts: AlTransitionTimeouts,
    error_acknowledge_policy: AlErrorAcknowledgePolicy,
    al: AlTransitionController,
    op_only: OpOnlySyncManagerController,
    op_only_profile: OpOnlySyncManagerProfile,
    transition_stage: TransitionStage,
    step_deadline_ns: u64,
    latest_observation: Option<StateRequestObservation>,
    observation_pending: bool,
    result: Option<StateRequestResult>,
    last_error: Option<StateRequestError>,
}

impl StateRequestController {
    pub const fn new() -> Self {
        Self {
            phase: StateRequestPhase::Idle,
            handle: None,
            next_sequence: 1,
            position: 0,
            station_address: 0,
            requested_state: EthercatState::Unknown,
            observed_status: AlStatus::new(0, 0),
            generation: 0,
            deadline_ns: 0,
            request_timeout_ns: 0,
            transition_timeouts: AlTransitionTimeouts::uniform(1),
            error_acknowledge_policy: AlErrorAcknowledgePolicy::Disabled,
            al: AlTransitionController::new(),
            op_only: OpOnlySyncManagerController::new(),
            op_only_profile: OpOnlySyncManagerProfile::EMPTY,
            transition_stage: TransitionStage::Al,
            step_deadline_ns: 0,
            latest_observation: None,
            observation_pending: false,
            result: None,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> StateRequestPhase {
        self.phase
    }

    pub const fn active_handle(&self) -> Option<StateRequestHandle> {
        self.handle
    }

    pub const fn pending(&self) -> Option<AlAction> {
        match self.transition_stage {
            TransitionStage::Al => self.al.pending(),
            _ => None,
        }
    }

    pub fn control_pending(&self) -> Option<StateRequestAction> {
        match self.transition_stage {
            TransitionStage::Al => self.al.pending().map(StateRequestAction::Al),
            _ => self.op_only.pending().map(StateRequestAction::OpOnly),
        }
    }

    pub const fn last_error(&self) -> Option<StateRequestError> {
        self.last_error
    }

    pub const fn fault_record(&self) -> Option<AlFaultRecord> {
        self.al.fault_record()
    }

    pub const fn latest_observation(&self) -> Option<StateRequestObservation> {
        self.latest_observation
    }

    pub fn status(
        &self,
        handle: StateRequestHandle,
    ) -> Result<StateRequestStatus, StateRequestError> {
        if self.handle != Some(handle) {
            return Err(StateRequestError::InvalidHandle);
        }
        Ok(self.status_unchecked(handle))
    }

    pub fn result(
        &self,
        handle: StateRequestHandle,
    ) -> Result<StateRequestResult, StateRequestError> {
        if self.handle != Some(handle) {
            return Err(StateRequestError::InvalidHandle);
        }
        self.result.ok_or(StateRequestError::ResultNotReady)
    }

    pub fn start(
        &mut self,
        config: StateRequestConfig,
    ) -> Result<StateRequestHandle, StateRequestError> {
        self.start_inner(config, false, OpOnlySyncManagerProfile::EMPTY)
    }

    pub fn start_with_op_only(
        &mut self,
        config: StateRequestConfig,
        profile: OpOnlySyncManagerProfile,
    ) -> Result<StateRequestHandle, StateRequestError> {
        self.start_inner(config, false, profile)
    }

    /// Start a bounded request that lowers SAFEOP/OP directly to PREOP.
    ///
    /// All other state requests retain the normal stepwise AL transition
    /// policy. This is intentionally narrow so callers cannot skip upward
    /// startup transitions.
    pub fn start_direct_preop(
        &mut self,
        config: StateRequestConfig,
    ) -> Result<StateRequestHandle, StateRequestError> {
        self.start_inner(config, true, OpOnlySyncManagerProfile::EMPTY)
    }

    fn start_inner(
        &mut self,
        config: StateRequestConfig,
        direct_preop: bool,
        profile: OpOnlySyncManagerProfile,
    ) -> Result<StateRequestHandle, StateRequestError> {
        match self.phase {
            StateRequestPhase::Transitioning => return Err(StateRequestError::Busy),
            StateRequestPhase::Faulted => return Err(StateRequestError::FaultLatched),
            StateRequestPhase::Idle | StateRequestPhase::Complete => {}
        }
        validate_config(config)?;
        profile
            .validate()
            .map_err(|error| StateRequestError::OpOnly(OpOnlySyncManagerError::Profile(error)))?;

        let handle = StateRequestHandle {
            sequence: self.next_sequence,
        };
        let (al, step_deadline_ns) =
            prepare_al(config, config.observed_status, config.now_ns, direct_preop)?;
        let al_complete = al.phase() == AlPhase::Complete;
        let immediate_complete = al_complete && profile.is_empty();
        let enabled = al_complete && config.requested_state == EthercatState::Op;
        let mut op_only = OpOnlySyncManagerController::new();
        if !profile.is_empty() {
            op_only
                .start(
                    config.station_address,
                    config.generation,
                    step_deadline_ns,
                    config.request_timeout_ns,
                    profile,
                    enabled,
                )
                .map_err(StateRequestError::OpOnly)?;
        }

        self.phase = if immediate_complete {
            StateRequestPhase::Complete
        } else {
            StateRequestPhase::Transitioning
        };
        self.handle = Some(handle);
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        self.position = config.position;
        self.station_address = config.station_address;
        self.requested_state = config.requested_state;
        self.observed_status = config.observed_status;
        self.generation = config.generation;
        self.deadline_ns = config.deadline_ns;
        self.request_timeout_ns = config.request_timeout_ns;
        self.transition_timeouts = config.transition_timeouts;
        self.error_acknowledge_policy = config.error_acknowledge_policy;
        self.al = al;
        self.op_only = op_only;
        self.op_only_profile = profile;
        self.step_deadline_ns = step_deadline_ns;
        self.transition_stage = if profile.is_empty() {
            TransitionStage::Al
        } else if enabled {
            TransitionStage::Enable
        } else {
            TransitionStage::Disable
        };
        self.latest_observation = None;
        self.observation_pending = false;
        self.result = None;
        self.last_error = None;

        if al_complete {
            let observation = self.observation(config.now_ns);
            self.latest_observation = Some(observation);
            if immediate_complete {
                self.result = Some(StateRequestResult {
                    observation,
                    completed_at_ns: config.now_ns,
                });
            }
        }
        Ok(handle)
    }

    pub fn next_action(&mut self, now_ns: u64) -> Result<Option<AlAction>, StateRequestError> {
        if self.transition_stage != TransitionStage::Al
            && self.phase == StateRequestPhase::Transitioning
        {
            return Err(StateRequestError::OpOnlyUnsupported(self.position));
        }
        self.next_control_action(now_ns).map(|action| {
            action.and_then(|action| match action {
                StateRequestAction::Al(action) => Some(action),
                StateRequestAction::OpOnly(_) => None,
            })
        })
    }

    pub fn next_control_action(
        &mut self,
        now_ns: u64,
    ) -> Result<Option<StateRequestAction>, StateRequestError> {
        match self.phase {
            StateRequestPhase::Idle => return Err(StateRequestError::ResultNotReady),
            StateRequestPhase::Complete => return Ok(None),
            StateRequestPhase::Faulted => {
                return Err(self.last_error.unwrap_or(StateRequestError::FaultLatched));
            }
            StateRequestPhase::Transitioning => {}
        }
        if self.transition_stage != TransitionStage::Al {
            return self
                .op_only
                .next_action(now_ns)
                .map(|action| action.map(StateRequestAction::OpOnly))
                .map_err(|error| self.fail(StateRequestError::OpOnly(error)));
        }
        match self.al.next_action(now_ns) {
            Ok(action) => Ok(action.map(StateRequestAction::Al)),
            Err(error) => Err(self.fail(StateRequestError::Al(error))),
        }
    }

    pub fn enqueue_pending<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
    ) -> Result<RequestHandle, StateRequestError> {
        let result = if self.transition_stage == TransitionStage::Al {
            self.al.enqueue_pending(pool)
        } else {
            self.op_only.enqueue_pending(pool)
        };
        match result {
            Ok(handle) => Ok(handle),
            Err(error) => Err(self.fail(StateRequestError::Control(error))),
        }
    }

    pub fn accept(
        &mut self,
        action: AlAction,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<StateRequestProgress, StateRequestError> {
        if self.phase != StateRequestPhase::Transitioning || self.pending() != Some(action) {
            return Err(self.fail(StateRequestError::ActionMismatch));
        }

        let progress =
            match self
                .al
                .accept(action.token, generation, payload, working_counter, now_ns)
            {
                Ok(progress) => progress,
                Err(error) => {
                    if action.read_len != 0
                        && matches!(error, AlError::AlErrorCode(_) | AlError::InvalidResponse)
                    {
                        self.record_observation(now_ns);
                    }
                    return Err(self.fail(StateRequestError::Al(error)));
                }
            };

        match progress {
            AlProgress::ControlWritten => Ok(StateRequestProgress::ControlWritten),
            AlProgress::Polling => {
                if action.read_len != 0 {
                    self.record_observation(now_ns);
                }
                Ok(StateRequestProgress::Polling)
            }
            AlProgress::Reached(_) => {
                let observation = self.record_observation(now_ns);
                if observation.observed_status.state == self.requested_state {
                    if self.requested_state == EthercatState::Op && !self.op_only_profile.is_empty()
                    {
                        if now_ns >= self.step_deadline_ns {
                            return Err(self
                                .fail(StateRequestError::OpOnly(OpOnlySyncManagerError::Timeout)));
                        }
                        self.op_only
                            .start(
                                self.station_address,
                                self.generation,
                                self.step_deadline_ns,
                                self.request_timeout_ns,
                                self.op_only_profile,
                                true,
                            )
                            .map_err(|error| self.fail(StateRequestError::OpOnly(error)))?;
                        self.transition_stage = TransitionStage::Enable;
                        return Ok(StateRequestProgress::StepReached(observation));
                    }
                    let result = StateRequestResult {
                        observation,
                        completed_at_ns: now_ns,
                    };
                    self.result = Some(result);
                    self.phase = StateRequestPhase::Complete;
                    Ok(StateRequestProgress::Complete(result))
                } else {
                    self.al = AlTransitionController::new();
                    self.start_next_step(now_ns)?;
                    Ok(StateRequestProgress::StepReached(observation))
                }
            }
        }
    }

    pub fn accept_completed<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
        handle: RequestHandle,
        now_ns: u64,
    ) -> Result<StateRequestProgress, StateRequestError> {
        if self.transition_stage != TransitionStage::Al {
            if self.phase != StateRequestPhase::Transitioning {
                return Err(self.fail(StateRequestError::ActionMismatch));
            }
            let progress = self
                .op_only
                .accept_completed(pool, handle, now_ns)
                .map_err(|error| self.fail(StateRequestError::OpOnly(error)))?;
            return self.finish_op_only(progress, now_ns);
        }
        let action = match self.pending() {
            Some(action) => action,
            None => return Err(self.fail(StateRequestError::ActionMismatch)),
        };
        let response_len = action.read_len as usize;
        let mut response = [0; MAX_CONTROL_PAYLOAD];
        let (generation, actual_wkc, control_error, matches) = match pool.get(handle) {
            Some(request) if request.state == RequestState::Complete => {
                let matches = request.matches_action(
                    action.datagram_index,
                    action.generation,
                    action.address,
                    action.operation,
                    action.payload(),
                    action.datagram_len(),
                    action.deadline_ns,
                ) && request.length >= response_len;
                if matches {
                    response[..response_len].copy_from_slice(&request.payload()[..response_len]);
                }
                (request.generation, request.actual_wkc, None, matches)
            }
            Some(request) if request.state == RequestState::Failed => {
                let matches = request.matches_action(
                    action.datagram_index,
                    action.generation,
                    action.address,
                    action.operation,
                    action.payload(),
                    action.datagram_len(),
                    action.deadline_ns,
                );
                (
                    request.generation,
                    request.actual_wkc,
                    Some(request.last_error().unwrap_or(ControlError::InvalidState)),
                    matches,
                )
            }
            Some(_) => return Err(StateRequestError::Control(ControlError::InvalidState)),
            None => return Err(self.fail(StateRequestError::Control(ControlError::InvalidHandle))),
        };

        let release = pool.release(handle);
        if !matches {
            return Err(self.fail(StateRequestError::ActionMismatch));
        }
        if let Err(error) = release {
            return Err(self.fail(StateRequestError::Control(error)));
        }
        if let Some(error) = control_error {
            if error == ControlError::Timeout {
                return self.timeout(action, now_ns);
            }
            return Err(self.fail(StateRequestError::Control(error)));
        }

        self.accept(
            action,
            generation,
            &response[..response_len],
            actual_wkc,
            now_ns,
        )
    }

    pub fn accept_control(
        &mut self,
        action: StateRequestAction,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<StateRequestProgress, StateRequestError> {
        if self.phase != StateRequestPhase::Transitioning || self.control_pending() != Some(action)
        {
            return Err(self.fail(StateRequestError::ActionMismatch));
        }
        match action {
            StateRequestAction::Al(action) => {
                self.accept(action, generation, payload, working_counter, now_ns)
            }
            StateRequestAction::OpOnly(action) => {
                let progress = self
                    .op_only
                    .accept(action, generation, payload, working_counter, now_ns)
                    .map_err(|error| self.fail(StateRequestError::OpOnly(error)))?;
                self.finish_op_only(progress, now_ns)
            }
        }
    }

    pub fn timeout_control(
        &mut self,
        action: StateRequestAction,
        now_ns: u64,
    ) -> Result<StateRequestProgress, StateRequestError> {
        if self.phase != StateRequestPhase::Transitioning || self.control_pending() != Some(action)
        {
            return Err(self.fail(StateRequestError::ActionMismatch));
        }
        match action {
            StateRequestAction::Al(action) => self.timeout(action, now_ns),
            StateRequestAction::OpOnly(action) => {
                let error = self
                    .op_only
                    .timeout(action, now_ns)
                    .err()
                    .unwrap_or(OpOnlySyncManagerError::Timeout);
                Err(self.fail(StateRequestError::OpOnly(error)))
            }
        }
    }

    fn finish_op_only(
        &mut self,
        progress: OpOnlySyncManagerProgress,
        now_ns: u64,
    ) -> Result<StateRequestProgress, StateRequestError> {
        if progress != OpOnlySyncManagerProgress::Complete {
            return Ok(StateRequestProgress::OpOnly(progress));
        }
        if self.transition_stage == TransitionStage::Disable && self.al.phase() != AlPhase::Complete
        {
            self.transition_stage = TransitionStage::Al;
            return Ok(StateRequestProgress::OpOnly(progress));
        }
        let observation = self
            .latest_observation
            .ok_or_else(|| self.fail(StateRequestError::ActionMismatch))?;
        let result = StateRequestResult {
            observation,
            completed_at_ns: now_ns,
        };
        self.phase = StateRequestPhase::Complete;
        self.result = Some(result);
        Ok(StateRequestProgress::Complete(result))
    }

    pub fn timeout(
        &mut self,
        action: AlAction,
        now_ns: u64,
    ) -> Result<StateRequestProgress, StateRequestError> {
        if self.phase != StateRequestPhase::Transitioning || self.pending() != Some(action) {
            return Err(self.fail(StateRequestError::ActionMismatch));
        }
        let error = match self.al.timeout(action.token, now_ns) {
            Ok(()) | Err(AlError::Timeout) => StateRequestError::Al(AlError::Timeout),
            Err(error) => StateRequestError::Al(error),
        };
        Err(self.fail(error))
    }

    pub fn take_observation(&mut self) -> Option<StateRequestObservation> {
        if !self.observation_pending {
            return None;
        }
        self.observation_pending = false;
        self.latest_observation
    }

    pub(crate) fn status_unchecked(&self, handle: StateRequestHandle) -> StateRequestStatus {
        StateRequestStatus {
            handle,
            phase: self.phase,
            position: self.position,
            station_address: self.station_address,
            requested_state: self.requested_state,
            observed_status: self.observed_status,
            generation: self.generation,
            deadline_ns: self.deadline_ns,
            latest_observation: self.latest_observation,
            error: self.last_error,
        }
    }

    pub(crate) fn abort(&mut self, error: StateRequestError) -> StateRequestError {
        self.fail(error)
    }

    fn start_next_step(&mut self, now_ns: u64) -> Result<(), StateRequestError> {
        let config = StateRequestConfig {
            position: self.position,
            station_address: self.station_address,
            observed_status: self.observed_status,
            requested_state: self.requested_state,
            generation: self.generation,
            now_ns,
            deadline_ns: self.deadline_ns,
            request_timeout_ns: self.request_timeout_ns,
            transition_timeouts: self.transition_timeouts,
            error_acknowledge_policy: self.error_acknowledge_policy,
        };
        match prepare_al(config, self.observed_status, now_ns, false) {
            Ok((al, deadline)) => {
                self.al = al;
                self.step_deadline_ns = deadline;
                Ok(())
            }
            Err(error) => Err(self.fail(error)),
        }
    }

    fn record_observation(&mut self, now_ns: u64) -> StateRequestObservation {
        self.observed_status = self.al.observed_status();
        let observation = self.observation(now_ns);
        self.latest_observation = Some(observation);
        self.observation_pending = true;
        observation
    }

    fn observation(&self, now_ns: u64) -> StateRequestObservation {
        StateRequestObservation {
            handle: self.handle.unwrap_or(StateRequestHandle { sequence: 0 }),
            position: self.position,
            station_address: self.station_address,
            requested_state: self.requested_state,
            observed_status: self.observed_status,
            generation: self.generation,
            deadline_ns: self.deadline_ns,
            observed_at_ns: now_ns,
        }
    }

    fn fail(&mut self, error: StateRequestError) -> StateRequestError {
        if self.last_error.is_none() {
            self.last_error = Some(error);
        }
        self.phase = StateRequestPhase::Faulted;
        self.result = None;
        self.last_error.unwrap_or(error)
    }
}

fn validate_config(config: StateRequestConfig) -> Result<(), StateRequestError> {
    if config.station_address == 0 {
        return Err(StateRequestError::InvalidStationAddress);
    }
    if config.deadline_ns <= config.now_ns {
        return Err(StateRequestError::InvalidDeadline);
    }
    if config.request_timeout_ns == 0 {
        return Err(StateRequestError::InvalidRequestTimeout);
    }
    if !config.transition_timeouts.is_valid() {
        return Err(StateRequestError::InvalidTransitionTimeouts);
    }
    if matches!(config.observed_status.state, EthercatState::Unknown)
        || matches!(config.requested_state, EthercatState::Unknown)
        || (config.observed_status.state != config.requested_state
            && next_state(config.observed_status.state, config.requested_state).is_none())
    {
        return Err(StateRequestError::InvalidTransition);
    }
    Ok(())
}

fn prepare_al(
    config: StateRequestConfig,
    observed_status: AlStatus,
    now_ns: u64,
    direct_preop: bool,
) -> Result<(AlTransitionController, u64), StateRequestError> {
    let expected_state = if direct_preop {
        if config.requested_state != EthercatState::PreOp
            || !matches!(
                observed_status.state,
                EthercatState::PreOp | EthercatState::SafeOp | EthercatState::Op
            )
        {
            return Err(StateRequestError::InvalidTransition);
        }
        EthercatState::PreOp
    } else if observed_status.state == config.requested_state {
        observed_status.state
    } else {
        next_state(observed_status.state, config.requested_state)
            .ok_or(StateRequestError::InvalidTransition)?
    };
    let remaining_ns = config
        .deadline_ns
        .checked_sub(now_ns)
        .filter(|remaining| *remaining != 0)
        .ok_or(StateRequestError::InvalidDeadline)?;
    let transition_timeout_ns = config
        .transition_timeouts
        .for_step(observed_status.state, expected_state)
        .ok_or(StateRequestError::InvalidTransitionTimeouts)?
        .min(remaining_ns);
    let mut al = AlTransitionController::new();
    let request = AlTransitionRequest {
        station_address: config.station_address,
        current_state: observed_status.state,
        requested_state: config.requested_state,
        generation: config.generation,
        now_ns,
        timeout_ns: transition_timeout_ns,
        request_timeout_ns: config.request_timeout_ns.min(transition_timeout_ns),
    };
    if direct_preop {
        al.start_direct_preop_with_status(request, observed_status, config.error_acknowledge_policy)
    } else {
        al.start_with_status(request, observed_status, config.error_acknowledge_policy)
    }
    .map_err(StateRequestError::Al)?;
    Ok((al, now_ns.saturating_add(transition_timeout_ns)))
}

impl Default for StateRequestController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slave::AL_ERROR_FLAG;

    const TIMEOUTS: AlTransitionTimeouts = AlTransitionTimeouts::new(300, 1_000, 500, 200);

    fn op_only_profile() -> OpOnlySyncManagerProfile {
        let mut profile = OpOnlySyncManagerProfile::new();
        profile.add(2, 0x09).unwrap();
        profile
    }

    fn finish_op_only_pair(
        controller: &mut StateRequestController,
        enabled: bool,
        now: u64,
    ) -> StateRequestProgress {
        let action = controller.next_control_action(now).unwrap().unwrap();
        let StateRequestAction::OpOnly(write) = action else {
            panic!("expected SM write");
        };
        assert_eq!(write.enabled, enabled);
        assert_eq!(write.payload(), &[u8::from(enabled)]);
        assert_eq!(
            write.address,
            crate::registers::fixed_address(0x1003, 0x0816)
        );
        controller
            .accept_control(action, 7, &[], 1, now + 1)
            .unwrap();
        let action = controller.next_control_action(now + 2).unwrap().unwrap();
        let StateRequestAction::OpOnly(read) = action else {
            panic!("expected SM readback");
        };
        assert_eq!(read.read_len, 1);
        controller
            .accept_control(action, 7, &[u8::from(enabled)], 1, now + 3)
            .unwrap()
    }

    #[test]
    fn op_only_disables_before_al_and_enables_only_after_verified_op() {
        let mut controller = StateRequestController::new();
        let handle = controller
            .start_with_op_only(
                config(EthercatState::Init, EthercatState::Op, 0, 2_000),
                op_only_profile(),
            )
            .unwrap();
        assert_eq!(
            controller.next_action(1),
            Err(StateRequestError::OpOnlyUnsupported(3))
        );
        assert_eq!(controller.phase(), StateRequestPhase::Transitioning);
        finish_op_only_pair(&mut controller, false, 1);
        finish_step(&mut controller, EthercatState::PreOp, 5);
        finish_step(&mut controller, EthercatState::SafeOp, 9);
        assert!(matches!(
            finish_step(&mut controller, EthercatState::Op, 13),
            StateRequestProgress::StepReached(_)
        ));
        assert_eq!(
            controller.result(handle),
            Err(StateRequestError::ResultNotReady)
        );
        let observation = controller.latest_observation().unwrap();
        assert_eq!(observation.observed_status.state, EthercatState::Op);
        let StateRequestProgress::Complete(result) = finish_op_only_pair(&mut controller, true, 17)
        else {
            panic!("expected completion after SM verification");
        };
        assert_eq!(result.observation, observation);
        assert_eq!(result.completed_at_ns, 20);

        let next = controller
            .start_with_op_only(
                config(EthercatState::Op, EthercatState::SafeOp, 30, 2_000),
                op_only_profile(),
            )
            .unwrap();
        assert_eq!(
            controller.result(handle),
            Err(StateRequestError::InvalidHandle)
        );
        finish_op_only_pair(&mut controller, false, 31);
        assert!(matches!(
            finish_step(&mut controller, EthercatState::SafeOp, 35),
            StateRequestProgress::Complete(_)
        ));
        assert_eq!(
            controller
                .result(next)
                .unwrap()
                .observation
                .observed_status
                .state,
            EthercatState::SafeOp
        );
    }

    #[test]
    fn op_only_same_state_still_verifies_activation_and_readback_failure_latches() {
        let mut controller = StateRequestController::new();
        let handle = controller
            .start_with_op_only(
                config(EthercatState::Op, EthercatState::Op, 0, 500),
                op_only_profile(),
            )
            .unwrap();
        assert_eq!(
            controller.result(handle),
            Err(StateRequestError::ResultNotReady)
        );
        assert!(matches!(
            finish_op_only_pair(&mut controller, true, 1),
            StateRequestProgress::Complete(_)
        ));
        let mut controller = StateRequestController::new();
        controller
            .start_with_op_only(
                config(EthercatState::Op, EthercatState::SafeOp, 0, 500),
                op_only_profile(),
            )
            .unwrap();
        let write = controller.next_control_action(1).unwrap().unwrap();
        controller.accept_control(write, 7, &[], 1, 2).unwrap();
        let read = controller.next_control_action(3).unwrap().unwrap();
        let error = StateRequestError::OpOnly(OpOnlySyncManagerError::ReadbackMismatch);
        assert_eq!(
            controller.accept_control(read, 7, &[0x08], 1, 4),
            Err(error)
        );
        assert_eq!(controller.phase(), StateRequestPhase::Faulted);
        assert_eq!(controller.next_control_action(5), Err(error));
        assert_eq!(
            controller.start(config(EthercatState::Op, EthercatState::SafeOp, 5, 500)),
            Err(StateRequestError::FaultLatched)
        );
        assert_eq!(controller.timeout_control(write, 500), Err(error));
    }

    #[test]
    fn op_only_and_al_share_the_original_step_deadline() {
        let mut controller = StateRequestController::new();
        controller
            .start_with_op_only(
                config(EthercatState::SafeOp, EthercatState::Op, 0, 10),
                op_only_profile(),
            )
            .unwrap();
        finish_op_only_pair(&mut controller, false, 1);
        finish_step(&mut controller, EthercatState::Op, 5);
        let action = controller.next_control_action(9).unwrap().unwrap();
        assert_eq!(action.deadline_ns(), 10);
        controller.accept_control(action, 7, &[], 1, 10).unwrap();
        assert_eq!(
            controller.next_control_action(11),
            Err(StateRequestError::OpOnly(OpOnlySyncManagerError::Timeout))
        );
        assert_eq!(controller.phase(), StateRequestPhase::Faulted);
        assert!(
            controller
                .result(controller.active_handle().unwrap())
                .is_err()
        );
    }

    fn config(
        current: EthercatState,
        target: EthercatState,
        now_ns: u64,
        deadline_ns: u64,
    ) -> StateRequestConfig {
        StateRequestConfig {
            position: 3,
            station_address: 0x1003,
            observed_status: AlStatus::new(current as u16, 0),
            requested_state: target,
            generation: 7,
            now_ns,
            deadline_ns,
            request_timeout_ns: 100,
            transition_timeouts: TIMEOUTS,
            error_acknowledge_policy: AlErrorAcknowledgePolicy::Enabled,
        }
    }

    fn status_payload(state: EthercatState, error: bool, code: u16) -> [u8; 6] {
        let raw = (state as u16) | if error { AL_ERROR_FLAG } else { 0 };
        let raw = raw.to_le_bytes();
        let code = code.to_le_bytes();
        [raw[0], raw[1], 0, 0, code[0], code[1]]
    }

    fn finish_step(
        controller: &mut StateRequestController,
        state: EthercatState,
        now_ns: u64,
    ) -> StateRequestProgress {
        let write = controller.next_action(now_ns).unwrap().unwrap();
        assert_eq!(write.read_len, 0);
        assert_eq!(
            controller.accept(write, 7, &[], 1, now_ns + 1),
            Ok(StateRequestProgress::ControlWritten)
        );
        let read = controller.next_action(now_ns + 2).unwrap().unwrap();
        assert_eq!(read.read_len, 6);
        controller
            .accept(read, 7, &status_payload(state, false, 0), 1, now_ns + 3)
            .unwrap()
    }

    #[test]
    fn immediate_completion_retains_result_and_invalid_start_is_transactional() {
        let mut controller = StateRequestController::new();
        let handle = controller
            .start(config(
                EthercatState::SafeOp,
                EthercatState::SafeOp,
                10,
                100,
            ))
            .unwrap();
        assert_eq!(controller.phase(), StateRequestPhase::Complete);
        assert_eq!(
            controller
                .result(handle)
                .unwrap()
                .observation
                .observed_status
                .state,
            EthercatState::SafeOp
        );

        let mut invalid = config(EthercatState::SafeOp, EthercatState::Op, 20, 20);
        invalid.station_address = 0;
        assert_eq!(
            controller.start(invalid),
            Err(StateRequestError::InvalidStationAddress)
        );
        assert_eq!(
            controller.result(handle).unwrap().observation.handle,
            handle
        );
    }

    #[test]
    fn init_to_op_advances_only_after_each_verified_step() {
        let mut controller = StateRequestController::new();
        let handle = controller
            .start(config(EthercatState::Init, EthercatState::Op, 0, 5_000))
            .unwrap();

        let preop = finish_step(&mut controller, EthercatState::PreOp, 10);
        let preop_observation = match preop {
            StateRequestProgress::StepReached(observation) => observation,
            other => panic!("unexpected progress: {other:?}"),
        };
        assert_eq!(
            preop_observation.observed_status.state,
            EthercatState::PreOp
        );
        assert_eq!(controller.take_observation(), Some(preop_observation));

        let safeop = finish_step(&mut controller, EthercatState::SafeOp, 30);
        assert!(matches!(safeop, StateRequestProgress::StepReached(_)));
        let complete = finish_step(&mut controller, EthercatState::Op, 50);
        let result = match complete {
            StateRequestProgress::Complete(result) => result,
            other => panic!("unexpected progress: {other:?}"),
        };
        assert_eq!(result.observation.observed_status.state, EthercatState::Op);
        assert_eq!(controller.phase(), StateRequestPhase::Complete);
        assert_eq!(controller.result(handle), Ok(result));
    }

    #[test]
    fn overall_deadline_caps_step_and_wire_deadlines() {
        let mut controller = StateRequestController::new();
        let mut request = config(EthercatState::SafeOp, EthercatState::Op, 1_000, 1_050);
        request.request_timeout_ns = 1_000;
        controller.start(request).unwrap();
        let action = controller.next_action(1_000).unwrap().unwrap();
        assert_eq!(action.deadline_ns, 1_050);
        assert_eq!(
            controller.timeout(action, 1_050),
            Err(StateRequestError::Al(AlError::Timeout))
        );
        assert_eq!(controller.phase(), StateRequestPhase::Faulted);

        let mut step_bounded = StateRequestController::new();
        let mut request = config(EthercatState::SafeOp, EthercatState::Op, 2_000, 5_000);
        request.request_timeout_ns = 1_000;
        request.transition_timeouts = AlTransitionTimeouts::uniform(25);
        step_bounded.start(request).unwrap();
        assert_eq!(
            step_bounded
                .next_action(2_000)
                .unwrap()
                .unwrap()
                .deadline_ns,
            2_025
        );
    }

    #[test]
    fn invalid_and_busy_requests_are_rejected_without_replacing_the_operation() {
        let mut controller = StateRequestController::new();
        for (mut request, expected) in [
            (
                config(EthercatState::Unknown, EthercatState::Op, 0, 10),
                StateRequestError::InvalidTransition,
            ),
            (
                config(EthercatState::Init, EthercatState::Bootstrap, 0, 10),
                StateRequestError::InvalidTransition,
            ),
        ] {
            assert_eq!(controller.start(request), Err(expected));
            request.requested_state = EthercatState::Unknown;
            assert_eq!(
                controller.start(request),
                Err(StateRequestError::InvalidTransition)
            );
        }

        let handle = controller
            .start(config(EthercatState::Init, EthercatState::PreOp, 0, 100))
            .unwrap();
        assert_eq!(
            controller.start(config(EthercatState::Init, EthercatState::SafeOp, 1, 100)),
            Err(StateRequestError::Busy)
        );
        assert_eq!(controller.active_handle(), Some(handle));
    }

    #[test]
    fn wkc_generation_and_action_mismatches_latch_the_first_fault() {
        let mut wkc = StateRequestController::new();
        wkc.start(config(EthercatState::Init, EthercatState::PreOp, 0, 100))
            .unwrap();
        let action = wkc.next_action(1).unwrap().unwrap();
        assert_eq!(
            wkc.accept(action, 7, &[], 0, 2),
            Err(StateRequestError::Al(AlError::UnexpectedWorkingCounter))
        );
        assert_eq!(wkc.phase(), StateRequestPhase::Faulted);
        assert_eq!(
            wkc.start(config(EthercatState::Init, EthercatState::PreOp, 3, 100)),
            Err(StateRequestError::FaultLatched)
        );

        let mut generation = StateRequestController::new();
        generation
            .start(config(EthercatState::Init, EthercatState::PreOp, 0, 100))
            .unwrap();
        let action = generation.next_action(1).unwrap().unwrap();
        assert_eq!(
            generation.accept(action, 8, &[], 1, 2),
            Err(StateRequestError::Al(AlError::GenerationMismatch))
        );

        let mut mismatch = StateRequestController::new();
        mismatch
            .start(config(EthercatState::Init, EthercatState::PreOp, 0, 100))
            .unwrap();
        let mut action = mismatch.next_action(1).unwrap().unwrap();
        action.token = action.token.wrapping_add(1);
        assert_eq!(
            mismatch.accept(action, 7, &[], 1, 2),
            Err(StateRequestError::ActionMismatch)
        );
    }

    #[test]
    fn error_acknowledgement_records_fault_but_never_retries_transition() {
        let mut controller = StateRequestController::new();
        controller
            .start(config(EthercatState::Init, EthercatState::PreOp, 0, 1_000))
            .unwrap();
        let write = controller.next_action(1).unwrap().unwrap();
        controller.accept(write, 7, &[], 1, 2).unwrap();
        let read_error = controller.next_action(3).unwrap().unwrap();
        assert_eq!(
            controller.accept(
                read_error,
                7,
                &status_payload(EthercatState::Init, true, 0x001B),
                1,
                4,
            ),
            Ok(StateRequestProgress::Polling)
        );
        assert!(controller.take_observation().unwrap().observed_status.error);

        let acknowledge = controller.next_action(5).unwrap().unwrap();
        controller.accept(acknowledge, 7, &[], 1, 6).unwrap();
        let verify = controller.next_action(7).unwrap().unwrap();
        assert_eq!(
            controller.accept(
                verify,
                7,
                &status_payload(EthercatState::Init, false, 0),
                1,
                8,
            ),
            Err(StateRequestError::Al(AlError::AlErrorCode(0x001B)))
        );
        assert_eq!(controller.phase(), StateRequestPhase::Faulted);
        assert_eq!(controller.fault_record().unwrap().status.code, 0x001B);
    }

    #[test]
    fn starting_a_new_completed_request_invalidates_the_old_handle() {
        let mut controller = StateRequestController::new();
        let first = controller
            .start(config(EthercatState::Init, EthercatState::Init, 0, 100))
            .unwrap();
        let second = controller
            .start(config(EthercatState::SafeOp, EthercatState::SafeOp, 1, 100))
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(
            controller.status(first),
            Err(StateRequestError::InvalidHandle)
        );
        assert_eq!(
            controller
                .result(second)
                .unwrap()
                .observation
                .observed_status
                .state,
            EthercatState::SafeOp
        );
    }
}
