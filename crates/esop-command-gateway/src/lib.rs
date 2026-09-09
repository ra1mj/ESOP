#![no_std]

use esop_lifecycle_guard::MotionPermit;

pub const MAX_AUTHORIZED_SOURCES: usize = 4;
pub const MAX_INGRESS_AUDITS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ExternalMotionCommand {
    pub boot_id: u64,
    pub source_id: u64,
    pub permit_epoch: u64,
    pub sequence: u64,
    pub deadline_ns: u64,
    pub axis_mask: u32,
    pub authority: u8,
    pub reserved: [u8; 3],
    pub policy_version: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct IngressPolicy {
    pub authorized_sources: [u64; MAX_AUTHORIZED_SOURCES],
    pub authorized_source_count: u8,
    pub minimum_authority: u8,
    pub reserved: [u8; 2],
    pub permit_policy_version: u32,
    pub allowed_axis_mask: u32,
    pub max_ttl_ns: u64,
    pub rate_window_ns: u64,
    pub max_commands_per_window: u16,
    pub reserved_tail: [u8; 6],
}

impl IngressPolicy {
    pub const fn conservative(source_id: u64, allowed_axis_mask: u32) -> Self {
        Self {
            authorized_sources: [source_id, 0, 0, 0],
            authorized_source_count: 1,
            minimum_authority: 1,
            reserved: [0; 2],
            permit_policy_version: 1,
            allowed_axis_mask,
            max_ttl_ns: 10_000_000,
            rate_window_ns: 1_000_000,
            max_commands_per_window: 100,
            reserved_tail: [0; 6],
        }
    }

    const fn normalized(self) -> Self {
        Self {
            authorized_sources: self.authorized_sources,
            authorized_source_count: if self.authorized_source_count as usize
                > MAX_AUTHORIZED_SOURCES
            {
                MAX_AUTHORIZED_SOURCES as u8
            } else {
                self.authorized_source_count
            },
            minimum_authority: if self.minimum_authority == 0 {
                1
            } else {
                self.minimum_authority
            },
            reserved: [0; 2],
            permit_policy_version: if self.permit_policy_version == 0 {
                1
            } else {
                self.permit_policy_version
            },
            allowed_axis_mask: self.allowed_axis_mask,
            max_ttl_ns: if self.max_ttl_ns == 0 {
                1
            } else {
                self.max_ttl_ns
            },
            rate_window_ns: if self.rate_window_ns == 0 {
                1
            } else {
                self.rate_window_ns
            },
            max_commands_per_window: if self.max_commands_per_window == 0 {
                1
            } else {
                self.max_commands_per_window
            },
            reserved_tail: [0; 6],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum IngressError {
    BootMismatch = 1,
    SourceUnauthorized = 2,
    AuthorityInsufficient = 3,
    PolicyVersionMismatch = 4,
    DeadlineExpired = 5,
    TtlTooLong = 6,
    ZeroSequence = 7,
    ZeroEpoch = 8,
    EmptyAxisMask = 9,
    AxisOutsidePolicy = 10,
    EpochReplayed = 11,
    SequenceReplayed = 12,
    RateLimited = 13,
}

impl IngressError {
    pub const fn code(self) -> u32 {
        0x4741_0000 | self as u32
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum IngressDecision {
    Accepted = 0,
    Rejected = 1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct IngressAudit {
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub source_id: u64,
    pub permit_epoch: u64,
    pub command_sequence: u64,
    pub decision: IngressDecision,
    pub error_code: u32,
    pub reserved: [u8; 3],
}

impl IngressAudit {
    pub const EMPTY: Self = Self {
        sequence: 0,
        timestamp_ns: 0,
        source_id: 0,
        permit_epoch: 0,
        command_sequence: 0,
        decision: IngressDecision::Rejected,
        error_code: 0,
        reserved: [0; 3],
    };
}

#[derive(Clone, Copy)]
struct SourceState {
    last_epoch: u64,
    last_sequence: u64,
    window_start_ns: u64,
    accepted_in_window: u16,
}

impl SourceState {
    const EMPTY: Self = Self {
        last_epoch: 0,
        last_sequence: 0,
        window_start_ns: 0,
        accepted_in_window: 0,
    };
}

pub struct CommandIngress {
    boot_id: u64,
    policy: IngressPolicy,
    sources: [SourceState; MAX_AUTHORIZED_SOURCES],
    audits: [IngressAudit; MAX_INGRESS_AUDITS],
    audit_head: usize,
    audit_count: usize,
    audit_sequence: u64,
}

impl CommandIngress {
    pub const fn new(boot_id: u64, policy: IngressPolicy) -> Self {
        Self {
            boot_id,
            policy: policy.normalized(),
            sources: [SourceState::EMPTY; MAX_AUTHORIZED_SOURCES],
            audits: [IngressAudit::EMPTY; MAX_INGRESS_AUDITS],
            audit_head: 0,
            audit_count: 0,
            audit_sequence: 0,
        }
    }

    pub const fn boot_id(&self) -> u64 {
        self.boot_id
    }

    pub const fn policy(&self) -> IngressPolicy {
        self.policy
    }

    pub const fn audit_count(&self) -> usize {
        self.audit_count
    }

    /// Return command decisions from oldest to newest.
    pub fn audit_at(&self, index: usize) -> Option<IngressAudit> {
        if index >= self.audit_count {
            return None;
        }
        let oldest = (self.audit_head + MAX_INGRESS_AUDITS - self.audit_count) % MAX_INGRESS_AUDITS;
        Some(self.audits[(oldest + index) % MAX_INGRESS_AUDITS])
    }

    /// Validate an external command and convert it to the only RT-facing
    /// capability: a fixed-size motion permit.
    pub fn admit(
        &mut self,
        command: ExternalMotionCommand,
        now_ns: u64,
    ) -> Result<MotionPermit, IngressError> {
        let result = self.preflight(command, now_ns);
        match result {
            Ok(source_index) => {
                let source = &mut self.sources[source_index];
                if now_ns.saturating_sub(source.window_start_ns) >= self.policy.rate_window_ns {
                    source.window_start_ns = now_ns;
                    source.accepted_in_window = 0;
                }
                source.last_epoch = command.permit_epoch;
                source.last_sequence = command.sequence;
                source.accepted_in_window = source.accepted_in_window.saturating_add(1);
                self.record_audit(command, now_ns, IngressDecision::Accepted, 0);
                Ok(MotionPermit {
                    boot_id: command.boot_id,
                    source_id: command.source_id,
                    permit_epoch: command.permit_epoch,
                    sequence: command.sequence,
                    expires_at_ns: command.deadline_ns,
                    axis_mask: command.axis_mask,
                    authority: command.authority,
                    reserved: [0; 3],
                    policy_version: command.policy_version,
                })
            }
            Err(error) => {
                self.record_audit(command, now_ns, IngressDecision::Rejected, error.code());
                Err(error)
            }
        }
    }

    fn preflight(
        &self,
        command: ExternalMotionCommand,
        now_ns: u64,
    ) -> Result<usize, IngressError> {
        if command.boot_id != self.boot_id {
            return Err(IngressError::BootMismatch);
        }
        let source_index = self
            .source_index(command.source_id)
            .ok_or(IngressError::SourceUnauthorized)?;
        if command.authority < self.policy.minimum_authority {
            return Err(IngressError::AuthorityInsufficient);
        }
        if command.policy_version != self.policy.permit_policy_version {
            return Err(IngressError::PolicyVersionMismatch);
        }
        if command.deadline_ns <= now_ns {
            return Err(IngressError::DeadlineExpired);
        }
        if command.deadline_ns.saturating_sub(now_ns) > self.policy.max_ttl_ns {
            return Err(IngressError::TtlTooLong);
        }
        if command.sequence == 0 {
            return Err(IngressError::ZeroSequence);
        }
        if command.permit_epoch == 0 {
            return Err(IngressError::ZeroEpoch);
        }
        if command.axis_mask == 0 {
            return Err(IngressError::EmptyAxisMask);
        }
        if command.axis_mask & !self.policy.allowed_axis_mask != 0 {
            return Err(IngressError::AxisOutsidePolicy);
        }

        let source = self.sources[source_index];
        if command.permit_epoch < source.last_epoch {
            return Err(IngressError::EpochReplayed);
        }
        if command.permit_epoch == source.last_epoch && command.sequence <= source.last_sequence {
            return Err(IngressError::SequenceReplayed);
        }
        let window_active =
            now_ns.saturating_sub(source.window_start_ns) < self.policy.rate_window_ns;
        if window_active && source.accepted_in_window >= self.policy.max_commands_per_window {
            return Err(IngressError::RateLimited);
        }
        Ok(source_index)
    }

    fn source_index(&self, source_id: u64) -> Option<usize> {
        (0..self.policy.authorized_source_count as usize)
            .find(|&index| self.policy.authorized_sources[index] == source_id)
    }

    fn record_audit(
        &mut self,
        command: ExternalMotionCommand,
        now_ns: u64,
        decision: IngressDecision,
        error_code: u32,
    ) {
        self.audit_sequence = self.audit_sequence.saturating_add(1);
        self.audits[self.audit_head] = IngressAudit {
            sequence: self.audit_sequence,
            timestamp_ns: now_ns,
            source_id: command.source_id,
            permit_epoch: command.permit_epoch,
            command_sequence: command.sequence,
            decision,
            error_code,
            reserved: [0; 3],
        };
        self.audit_head = (self.audit_head + 1) % MAX_INGRESS_AUDITS;
        self.audit_count = (self.audit_count + 1).min(MAX_INGRESS_AUDITS);
    }
}
