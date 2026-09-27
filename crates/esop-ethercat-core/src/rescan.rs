//! Explicit bounded EtherCAT topology rescans.
//!
//! A rescan is a named run of the existing startup state machine. These
//! fixed-size types expose operation identity and status without creating a
//! second owner for scan, SII, or AL protocol state.

use crate::startup::{StartupError, StartupPhase, StartupProgress};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RescanHandle {
    sequence: u32,
}

impl RescanHandle {
    pub const fn sequence(self) -> u32 {
        self.sequence
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RescanPhase {
    Idle = 0,
    Active = 1,
    Complete = 2,
    Faulted = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RescanError {
    Busy,
    InvalidState(StartupPhase),
    InvalidDeadline,
    InvalidHandle,
    ResultNotReady,
    Startup(StartupError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RescanStatus {
    pub handle: RescanHandle,
    pub phase: RescanPhase,
    pub startup_phase: StartupPhase,
    pub generation: u16,
    pub deadline_ns: u64,
    pub expected_count: usize,
    pub discovered_count: usize,
    pub error: Option<RescanError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RescanResult {
    pub handle: RescanHandle,
    pub generation: u16,
    pub deadline_ns: u64,
    pub expected_count: usize,
    pub discovered_count: usize,
    pub final_startup_phase: StartupPhase,
    pub error: Option<RescanError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RescanProgress {
    Advanced,
    SlaveDiscovered(usize),
    IdentityVerified(usize),
    RequestingIdVerified(usize),
    MailboxVerified(usize),
    FmmuRegistersRead(usize),
    SyncManagerRegistersRead(usize),
    SiiConfigurationVerified(usize),
    SlaveReady(usize),
    Complete(RescanResult),
}

impl RescanProgress {
    pub(crate) const fn from_startup(progress: StartupProgress) -> Option<Self> {
        match progress {
            StartupProgress::Advanced => Some(Self::Advanced),
            StartupProgress::SlaveDiscovered(index) => Some(Self::SlaveDiscovered(index)),
            StartupProgress::IdentityVerified(index) => Some(Self::IdentityVerified(index)),
            StartupProgress::RequestingIdVerified(index) => Some(Self::RequestingIdVerified(index)),
            StartupProgress::MailboxVerified(index) => Some(Self::MailboxVerified(index)),
            StartupProgress::FmmuRegistersRead(index) => Some(Self::FmmuRegistersRead(index)),
            StartupProgress::SyncManagerRegistersRead(index) => {
                Some(Self::SyncManagerRegistersRead(index))
            }
            StartupProgress::SiiConfigurationVerified(index) => {
                Some(Self::SiiConfigurationVerified(index))
            }
            StartupProgress::SlaveReady(index) => Some(Self::SlaveReady(index)),
            StartupProgress::Ready
            | StartupProgress::AwaitingConfiguration
            | StartupProgress::ConfigurationReleased => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RescanState {
    phase: RescanPhase,
    handle: Option<RescanHandle>,
    next_sequence: u32,
    generation: u16,
    deadline_ns: u64,
    expected_count: usize,
    result: Option<RescanResult>,
    last_error: Option<RescanError>,
}

impl RescanState {
    pub(crate) const fn new() -> Self {
        Self {
            phase: RescanPhase::Idle,
            handle: None,
            next_sequence: 1,
            generation: 0,
            deadline_ns: 0,
            expected_count: 0,
            result: None,
            last_error: None,
        }
    }

    pub(crate) const fn phase(self) -> RescanPhase {
        self.phase
    }

    pub(crate) const fn handle(self) -> Option<RescanHandle> {
        self.handle
    }

    pub(crate) const fn last_error(self) -> Option<RescanError> {
        self.last_error
    }

    pub(crate) const fn is_active_or_faulted(self) -> bool {
        matches!(self.phase, RescanPhase::Active | RescanPhase::Faulted)
    }

    pub(crate) fn clear(&mut self) {
        self.phase = RescanPhase::Idle;
        self.handle = None;
        self.generation = 0;
        self.deadline_ns = 0;
        self.expected_count = 0;
        self.result = None;
        self.last_error = None;
    }

    pub(crate) fn begin(
        &mut self,
        generation: u16,
        deadline_ns: u64,
        expected_count: usize,
    ) -> RescanHandle {
        let handle = RescanHandle {
            sequence: self.next_sequence,
        };
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        self.phase = RescanPhase::Active;
        self.handle = Some(handle);
        self.generation = generation;
        self.deadline_ns = deadline_ns;
        self.expected_count = expected_count;
        self.result = None;
        self.last_error = None;
        handle
    }

    pub(crate) fn status(
        self,
        handle: RescanHandle,
        startup_phase: StartupPhase,
        discovered_count: usize,
    ) -> Result<RescanStatus, RescanError> {
        if self.handle != Some(handle) {
            return Err(RescanError::InvalidHandle);
        }
        Ok(RescanStatus {
            handle,
            phase: self.phase,
            startup_phase,
            generation: self.generation,
            deadline_ns: self.deadline_ns,
            expected_count: self.expected_count,
            discovered_count,
            error: self.last_error,
        })
    }

    pub(crate) fn result(self, handle: RescanHandle) -> Result<RescanResult, RescanError> {
        if self.handle != Some(handle) {
            return Err(RescanError::InvalidHandle);
        }
        self.result.ok_or(RescanError::ResultNotReady)
    }

    pub(crate) fn complete(
        &mut self,
        final_startup_phase: StartupPhase,
        discovered_count: usize,
    ) -> Option<RescanResult> {
        let handle = self.handle?;
        let result = RescanResult {
            handle,
            generation: self.generation,
            deadline_ns: self.deadline_ns,
            expected_count: self.expected_count,
            discovered_count,
            final_startup_phase,
            error: None,
        };
        self.phase = RescanPhase::Complete;
        self.result = Some(result);
        Some(result)
    }

    pub(crate) fn fail(
        &mut self,
        error: StartupError,
        final_startup_phase: StartupPhase,
        discovered_count: usize,
    ) {
        if self.phase != RescanPhase::Active {
            return;
        }
        let error = RescanError::Startup(error);
        self.last_error = Some(error);
        let Some(handle) = self.handle else {
            return;
        };
        self.result = Some(RescanResult {
            handle,
            generation: self.generation,
            deadline_ns: self.deadline_ns,
            expected_count: self.expected_count,
            discovered_count,
            final_startup_phase,
            error: Some(error),
        });
        self.phase = RescanPhase::Faulted;
    }
}
