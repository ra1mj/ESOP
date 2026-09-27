//! Unified observation and fixed-capacity diagnostics for explicit recovery.
//!
//! The operation-specific controllers remain the protocol authorities. These
//! types only project their immutable status and result values through one
//! allocation-free public surface.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::reconfigure::{
    ReconfigureSlaveError, ReconfigureSlavePhase, ReconfigureSlaveResult, ReconfigureSlaveStatus,
};
use crate::rescan::{RescanError, RescanPhase, RescanResult, RescanStatus};
use crate::ring::SpscRing;
use crate::state_request::{
    StateRequestError, StateRequestPhase, StateRequestResult, StateRequestStatus,
};

const RECOVERY_KIND_COUNT: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ExplicitRecoveryKind {
    StateRequest = 0,
    Rescan = 1,
    ReconfigureSlave = 2,
}

impl ExplicitRecoveryKind {
    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ExplicitRecoveryPhase {
    Idle = 0,
    Active = 1,
    Complete = 2,
    Faulted = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplicitRecoveryFault {
    StateRequest(StateRequestError),
    Rescan(RescanError),
    ReconfigureSlave(ReconfigureSlaveError),
}

impl From<StateRequestError> for ExplicitRecoveryFault {
    fn from(value: StateRequestError) -> Self {
        Self::StateRequest(value)
    }
}

impl From<RescanError> for ExplicitRecoveryFault {
    fn from(value: RescanError) -> Self {
        Self::Rescan(value)
    }
}

impl From<ReconfigureSlaveError> for ExplicitRecoveryFault {
    fn from(value: ReconfigureSlaveError) -> Self {
        Self::ReconfigureSlave(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplicitRecoveryStatus {
    StateRequest(StateRequestStatus),
    Rescan(RescanStatus),
    ReconfigureSlave(ReconfigureSlaveStatus),
}

impl ExplicitRecoveryStatus {
    pub const fn kind(self) -> ExplicitRecoveryKind {
        match self {
            Self::StateRequest(_) => ExplicitRecoveryKind::StateRequest,
            Self::Rescan(_) => ExplicitRecoveryKind::Rescan,
            Self::ReconfigureSlave(_) => ExplicitRecoveryKind::ReconfigureSlave,
        }
    }

    pub const fn sequence(self) -> u32 {
        match self {
            Self::StateRequest(status) => status.handle.sequence(),
            Self::Rescan(status) => status.handle.sequence(),
            Self::ReconfigureSlave(status) => status.handle.sequence(),
        }
    }

    pub const fn phase(self) -> ExplicitRecoveryPhase {
        match self {
            Self::StateRequest(status) => match status.phase {
                StateRequestPhase::Idle => ExplicitRecoveryPhase::Idle,
                StateRequestPhase::Transitioning => ExplicitRecoveryPhase::Active,
                StateRequestPhase::Complete => ExplicitRecoveryPhase::Complete,
                StateRequestPhase::Faulted => ExplicitRecoveryPhase::Faulted,
            },
            Self::Rescan(status) => match status.phase {
                RescanPhase::Idle => ExplicitRecoveryPhase::Idle,
                RescanPhase::Active => ExplicitRecoveryPhase::Active,
                RescanPhase::Complete => ExplicitRecoveryPhase::Complete,
                RescanPhase::Faulted => ExplicitRecoveryPhase::Faulted,
            },
            Self::ReconfigureSlave(status) => match status.phase {
                ReconfigureSlavePhase::Idle => ExplicitRecoveryPhase::Idle,
                ReconfigureSlavePhase::Complete => ExplicitRecoveryPhase::Complete,
                ReconfigureSlavePhase::Faulted => ExplicitRecoveryPhase::Faulted,
                ReconfigureSlavePhase::DisablingOpOnly
                | ReconfigureSlavePhase::MovingToPreOp
                | ReconfigureSlavePhase::ConfiguringPdo
                | ReconfigureSlavePhase::ConfiguringWatchdog
                | ReconfigureSlavePhase::ConfiguringMapping
                | ReconfigureSlavePhase::ConfiguringDcClock
                | ReconfigureSlavePhase::ConfiguringDcSync => ExplicitRecoveryPhase::Active,
            },
        }
    }

    pub const fn generation(self) -> u16 {
        match self {
            Self::StateRequest(status) => status.generation,
            Self::Rescan(status) => status.generation,
            Self::ReconfigureSlave(status) => status.generation,
        }
    }

    pub const fn deadline_ns(self) -> u64 {
        match self {
            Self::StateRequest(status) => status.deadline_ns,
            Self::Rescan(status) => status.deadline_ns,
            Self::ReconfigureSlave(status) => status.deadline_ns,
        }
    }

    pub const fn position(self) -> Option<u16> {
        match self {
            Self::StateRequest(status) => Some(status.position),
            Self::Rescan(_) => None,
            Self::ReconfigureSlave(status) => Some(status.position),
        }
    }

    pub const fn station_address(self) -> Option<u16> {
        match self {
            Self::StateRequest(status) => Some(status.station_address),
            Self::Rescan(_) => None,
            Self::ReconfigureSlave(status) => Some(status.station_address),
        }
    }

    pub const fn fault(self) -> Option<ExplicitRecoveryFault> {
        match self {
            Self::StateRequest(status) => match status.error {
                Some(error) => Some(ExplicitRecoveryFault::StateRequest(error)),
                None => None,
            },
            Self::Rescan(status) => match status.error {
                Some(error) => Some(ExplicitRecoveryFault::Rescan(error)),
                None => None,
            },
            Self::ReconfigureSlave(status) => match status.error {
                Some(error) => Some(ExplicitRecoveryFault::ReconfigureSlave(error)),
                None => None,
            },
        }
    }
}

impl From<StateRequestStatus> for ExplicitRecoveryStatus {
    fn from(value: StateRequestStatus) -> Self {
        Self::StateRequest(value)
    }
}

impl From<RescanStatus> for ExplicitRecoveryStatus {
    fn from(value: RescanStatus) -> Self {
        Self::Rescan(value)
    }
}

impl From<ReconfigureSlaveStatus> for ExplicitRecoveryStatus {
    fn from(value: ReconfigureSlaveStatus) -> Self {
        Self::ReconfigureSlave(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplicitRecoveryResult {
    StateRequest(StateRequestResult),
    Rescan(RescanResult),
    ReconfigureSlave(ReconfigureSlaveResult),
}

impl ExplicitRecoveryResult {
    pub const fn kind(self) -> ExplicitRecoveryKind {
        match self {
            Self::StateRequest(_) => ExplicitRecoveryKind::StateRequest,
            Self::Rescan(_) => ExplicitRecoveryKind::Rescan,
            Self::ReconfigureSlave(_) => ExplicitRecoveryKind::ReconfigureSlave,
        }
    }

    pub const fn sequence(self) -> u32 {
        match self {
            Self::StateRequest(result) => result.observation.handle.sequence(),
            Self::Rescan(result) => result.handle.sequence(),
            Self::ReconfigureSlave(result) => result.handle.sequence(),
        }
    }

    pub const fn phase(self) -> ExplicitRecoveryPhase {
        match self {
            Self::Rescan(result) if result.error.is_some() => ExplicitRecoveryPhase::Faulted,
            Self::StateRequest(_) | Self::Rescan(_) | Self::ReconfigureSlave(_) => {
                ExplicitRecoveryPhase::Complete
            }
        }
    }

    pub const fn generation(self) -> Option<u16> {
        match self {
            Self::StateRequest(result) => Some(result.observation.generation),
            Self::Rescan(result) => Some(result.generation),
            Self::ReconfigureSlave(_) => None,
        }
    }

    pub const fn deadline_ns(self) -> Option<u64> {
        match self {
            Self::StateRequest(result) => Some(result.observation.deadline_ns),
            Self::Rescan(result) => Some(result.deadline_ns),
            Self::ReconfigureSlave(_) => None,
        }
    }

    pub const fn completed_at_ns(self) -> Option<u64> {
        match self {
            Self::StateRequest(result) => Some(result.completed_at_ns),
            Self::Rescan(_) => None,
            Self::ReconfigureSlave(result) => Some(result.completed_at_ns),
        }
    }

    pub const fn position(self) -> Option<u16> {
        match self {
            Self::StateRequest(result) => Some(result.observation.position),
            Self::Rescan(_) => None,
            Self::ReconfigureSlave(result) => Some(result.position),
        }
    }

    pub const fn station_address(self) -> Option<u16> {
        match self {
            Self::StateRequest(result) => Some(result.observation.station_address),
            Self::Rescan(_) => None,
            Self::ReconfigureSlave(result) => Some(result.station_address),
        }
    }

    pub const fn fault(self) -> Option<ExplicitRecoveryFault> {
        match self {
            Self::Rescan(result) => match result.error {
                Some(error) => Some(ExplicitRecoveryFault::Rescan(error)),
                None => None,
            },
            Self::StateRequest(_) | Self::ReconfigureSlave(_) => None,
        }
    }
}

impl From<StateRequestResult> for ExplicitRecoveryResult {
    fn from(value: StateRequestResult) -> Self {
        Self::StateRequest(value)
    }
}

impl From<RescanResult> for ExplicitRecoveryResult {
    fn from(value: RescanResult) -> Self {
        Self::Rescan(value)
    }
}

impl From<ReconfigureSlaveResult> for ExplicitRecoveryResult {
    fn from(value: ReconfigureSlaveResult) -> Self {
        Self::ReconfigureSlave(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ExplicitRecoveryDiagnosticCode {
    Submitted = 1,
    Progress = 2,
    Completed = 3,
    Faulted = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExplicitRecoveryDiagnosticEvent {
    pub cycle: u64,
    pub timestamp_ns: u64,
    pub code: ExplicitRecoveryDiagnosticCode,
    pub status: ExplicitRecoveryStatus,
}

pub struct ExplicitRecoveryDiagnostics<const EVENTS: usize = 64> {
    events: SpscRing<ExplicitRecoveryDiagnosticEvent, EVENTS>,
    last: [Option<ExplicitRecoveryStatus>; RECOVERY_KIND_COUNT],
    lost_events: AtomicUsize,
}

impl<const EVENTS: usize> ExplicitRecoveryDiagnostics<EVENTS> {
    pub const fn new() -> Self {
        Self {
            events: SpscRing::new(),
            last: [None; RECOVERY_KIND_COUNT],
            lost_events: AtomicUsize::new(0),
        }
    }

    /// Observe one immutable controller status and enqueue only a new
    /// operation or status transition. A full ring records one lost event and
    /// still advances the observation cursor so an unchanged status cannot
    /// amplify overflow on every cycle.
    pub fn observe(
        &mut self,
        cycle: u64,
        timestamp_ns: u64,
        status: ExplicitRecoveryStatus,
    ) -> bool {
        let index = status.kind().index();
        let previous = self.last[index];
        if previous == Some(status) {
            return false;
        }
        let code = match status.phase() {
            ExplicitRecoveryPhase::Complete => ExplicitRecoveryDiagnosticCode::Completed,
            ExplicitRecoveryPhase::Faulted => ExplicitRecoveryDiagnosticCode::Faulted,
            ExplicitRecoveryPhase::Idle | ExplicitRecoveryPhase::Active
                if previous.is_none_or(|previous| previous.sequence() != status.sequence()) =>
            {
                ExplicitRecoveryDiagnosticCode::Submitted
            }
            ExplicitRecoveryPhase::Idle | ExplicitRecoveryPhase::Active => {
                ExplicitRecoveryDiagnosticCode::Progress
            }
        };
        self.last[index] = Some(status);
        let event = ExplicitRecoveryDiagnosticEvent {
            cycle,
            timestamp_ns,
            code,
            status,
        };
        let (producer, _) = self.events.split();
        if producer.push(event).is_ok() {
            true
        } else {
            self.lost_events.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    pub fn pop(&self) -> Option<ExplicitRecoveryDiagnosticEvent> {
        let (_, consumer) = self.events.split();
        consumer.pop()
    }

    pub fn pending(&self) -> usize {
        self.events.len()
    }

    pub fn lost_events(&self) -> usize {
        self.lost_events.load(Ordering::Acquire)
    }
}

impl<const EVENTS: usize> Default for ExplicitRecoveryDiagnostics<EVENTS> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::al::{AlErrorAcknowledgePolicy, AlTransitionTimeouts};
    use crate::slave::{AlStatus, EthercatState};
    use crate::state_request::{StateRequestConfig, StateRequestController, StateRequestProgress};

    fn request(
        current: EthercatState,
        target: EthercatState,
        now_ns: u64,
        deadline_ns: u64,
    ) -> StateRequestConfig {
        StateRequestConfig {
            position: 2,
            station_address: 0x1002,
            observed_status: AlStatus::new(current as u16, 0),
            requested_state: target,
            generation: 9,
            now_ns,
            deadline_ns,
            request_timeout_ns: 100,
            transition_timeouts: AlTransitionTimeouts::uniform(1_000),
            error_acknowledge_policy: AlErrorAcknowledgePolicy::Enabled,
        }
    }

    fn status_payload(state: EthercatState) -> [u8; 6] {
        let raw = (state as u16).to_le_bytes();
        [raw[0], raw[1], 0, 0, 0, 0]
    }

    fn finish_step(
        controller: &mut StateRequestController,
        state: EthercatState,
        now_ns: u64,
    ) -> StateRequestProgress {
        let write = controller.next_action(now_ns).unwrap().unwrap();
        controller.accept(write, 9, &[], 1, now_ns + 1).unwrap();
        let read = controller.next_action(now_ns + 2).unwrap().unwrap();
        controller
            .accept(read, 9, &status_payload(state), 1, now_ns + 3)
            .unwrap()
    }

    #[test]
    fn unified_state_request_snapshots_preserve_common_and_typed_detail() {
        let mut controller = StateRequestController::new();
        let handle = controller
            .start(request(
                EthercatState::SafeOp,
                EthercatState::SafeOp,
                10,
                100,
            ))
            .unwrap();
        let status = ExplicitRecoveryStatus::from(controller.status(handle).unwrap());
        let result = ExplicitRecoveryResult::from(controller.result(handle).unwrap());

        assert_eq!(status.kind(), ExplicitRecoveryKind::StateRequest);
        assert_eq!(status.sequence(), handle.sequence());
        assert_eq!(status.phase(), ExplicitRecoveryPhase::Complete);
        assert_eq!(status.generation(), 9);
        assert_eq!(status.deadline_ns(), 100);
        assert_eq!(status.position(), Some(2));
        assert_eq!(status.station_address(), Some(0x1002));
        assert_eq!(status.fault(), None);
        assert_eq!(result.kind(), ExplicitRecoveryKind::StateRequest);
        assert_eq!(result.sequence(), handle.sequence());
        assert_eq!(result.generation(), Some(9));
        assert_eq!(result.deadline_ns(), Some(100));
        assert_eq!(result.completed_at_ns(), Some(10));
    }

    #[test]
    fn diagnostics_deduplicate_progress_and_preserve_typed_faults() {
        let mut controller = StateRequestController::new();
        let handle = controller
            .start(request(EthercatState::Init, EthercatState::Op, 0, 10_000))
            .unwrap();
        let mut diagnostics = ExplicitRecoveryDiagnostics::<8>::new();

        let submitted = controller.status(handle).unwrap().into();
        assert!(diagnostics.observe(1, 10, submitted));
        assert!(!diagnostics.observe(1, 11, submitted));
        assert_eq!(
            diagnostics.pop().unwrap().code,
            ExplicitRecoveryDiagnosticCode::Submitted
        );

        assert!(matches!(
            finish_step(&mut controller, EthercatState::PreOp, 20),
            StateRequestProgress::StepReached(_)
        ));
        let progressed = controller.status(handle).unwrap().into();
        assert!(diagnostics.observe(2, 20, progressed));
        assert_eq!(
            diagnostics.pop().unwrap().code,
            ExplicitRecoveryDiagnosticCode::Progress
        );

        finish_step(&mut controller, EthercatState::SafeOp, 30);
        let result = finish_step(&mut controller, EthercatState::Op, 40);
        assert!(matches!(result, StateRequestProgress::Complete(_)));
        let completed = controller.status(handle).unwrap().into();
        assert!(diagnostics.observe(4, 40, completed));
        assert_eq!(
            diagnostics.pop().unwrap().code,
            ExplicitRecoveryDiagnosticCode::Completed
        );

        let fault_handle = controller
            .start(request(EthercatState::SafeOp, EthercatState::Op, 50, 500))
            .unwrap();
        let active = controller.status(fault_handle).unwrap().into();
        assert!(diagnostics.observe(5, 50, active));
        assert_eq!(
            diagnostics.pop().unwrap().code,
            ExplicitRecoveryDiagnosticCode::Submitted
        );
        let mut action = controller.next_action(51).unwrap().unwrap();
        action.token = action.token.wrapping_add(1);
        assert_eq!(
            controller.accept(action, 9, &[], 1, 52),
            Err(StateRequestError::ActionMismatch)
        );
        let faulted = controller.status(fault_handle).unwrap().into();
        assert!(diagnostics.observe(6, 52, faulted));
        let event = diagnostics.pop().unwrap();
        assert_eq!(event.code, ExplicitRecoveryDiagnosticCode::Faulted);
        assert_eq!(
            event.status.fault(),
            Some(ExplicitRecoveryFault::StateRequest(
                StateRequestError::ActionMismatch
            ))
        );
    }

    #[test]
    fn diagnostic_overflow_counts_once_for_an_unchanged_lost_transition() {
        let mut controller = StateRequestController::new();
        let handle = controller
            .start(request(EthercatState::Init, EthercatState::Op, 0, 10_000))
            .unwrap();
        let mut diagnostics = ExplicitRecoveryDiagnostics::<1>::new();

        assert!(diagnostics.observe(1, 10, controller.status(handle).unwrap().into()));
        finish_step(&mut controller, EthercatState::PreOp, 20);
        let progress = controller.status(handle).unwrap().into();
        assert!(!diagnostics.observe(2, 20, progress));
        assert_eq!(diagnostics.lost_events(), 1);
        assert!(!diagnostics.observe(3, 30, progress));
        assert_eq!(diagnostics.lost_events(), 1);
        assert_eq!(diagnostics.pending(), 1);
        assert_eq!(
            diagnostics.pop().unwrap().code,
            ExplicitRecoveryDiagnosticCode::Submitted
        );
    }
}
