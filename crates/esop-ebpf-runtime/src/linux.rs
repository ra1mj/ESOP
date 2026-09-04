//! Linux implementation for loading the ESOP eBPF observation bundle.

use std::convert::TryFrom;
use std::fmt;
use std::fs;
use std::path::Path;

use aya::maps::{Array, MapData, MapError, PerCpuArray, RingBuf};
use aya::programs::{ProgramError, TracePoint};
use aya::{Ebpf, EbpfError, Pod};
use esop_ebpf_agent::{
    CAPABILITY_BTF, CAPABILITY_PERMISSION, CAPABILITY_RINGBUF, CAPABILITY_VERIFIER,
    CapabilitySnapshot, CorrelatorError, CycleContext, EvidenceDomain, EvidenceKind,
    IncidentSeverity, RuntimeAgent, RuntimeEvidence,
};

pub const MAP_EVENTS: &str = "ESOP_EVENTS";
pub const MAP_CONTEXT: &str = "ESOP_CONTEXT";
pub const MAP_STATS: &str = "ESOP_STATS";

pub const ATTACH_SCHED_WAKEUP: u64 = 1 << 0;
pub const ATTACH_SCHED_SWITCH: u64 = 1 << 1;
pub const ATTACH_PROCESS_EXIT: u64 = 1 << 2;
pub const ATTACH_PAGE_FAULT: u64 = 1 << 3;
pub const ATTACH_OOM_KILL: u64 = 1 << 4;
pub const ATTACH_NETWORK_DROP: u64 = 1 << 5;
pub const ATTACH_ALL: u64 = ATTACH_SCHED_WAKEUP
    | ATTACH_SCHED_SWITCH
    | ATTACH_PROCESS_EXIT
    | ATTACH_PAGE_FAULT
    | ATTACH_OOM_KILL
    | ATTACH_NETWORK_DROP;

const TRACEFS_EVENT_ROOTS: [&str; 2] = [
    "/sys/kernel/tracing/events",
    "/sys/kernel/debug/tracing/events",
];
const EVENT_RECORD_BYTES: usize = 96;
const CAP_BPF: u32 = 39;
const CAP_PERFMON: u32 = 38;
const CAP_SYS_ADMIN: u32 = 21;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AttachPoint {
    SchedulerWakeup = 0,
    SchedulerSwitch = 1,
    ProcessExit = 2,
    PageFault = 3,
    OomKill = 4,
    NetworkDrop = 5,
}

impl AttachPoint {
    pub const fn mask(self) -> u64 {
        match self {
            Self::SchedulerWakeup => ATTACH_SCHED_WAKEUP,
            Self::SchedulerSwitch => ATTACH_SCHED_SWITCH,
            Self::ProcessExit => ATTACH_PROCESS_EXIT,
            Self::PageFault => ATTACH_PAGE_FAULT,
            Self::OomKill => ATTACH_OOM_KILL,
            Self::NetworkDrop => ATTACH_NETWORK_DROP,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeConfig {
    /// Tracepoints that should be requested from the BPF object.
    pub enabled_attach_mask: u64,
    /// Missing bits cause a degraded capability snapshot, rather than granting
    /// an observation healthy lease.
    pub required_attach_mask: u64,
    /// Zero observes scheduler latency for all tasks. Production deployments
    /// should normally restrict this to the EtherCAT/gateway RT process.
    pub tracked_pid: u32,
    pub scheduler_latency_threshold_ns: u64,
    pub network_drop_threshold: u64,
    pub boot_id: u64,
    pub agent_epoch: u64,
}

impl RuntimeConfig {
    pub const fn valid(self) -> bool {
        self.enabled_attach_mask & !ATTACH_ALL == 0
            && self.required_attach_mask & !self.enabled_attach_mask == 0
            && self.scheduler_latency_threshold_ns > 0
            && self.network_drop_threshold > 0
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            enabled_attach_mask: ATTACH_ALL,
            required_attach_mask: ATTACH_SCHED_WAKEUP | ATTACH_SCHED_SWITCH | ATTACH_PROCESS_EXIT,
            tracked_pid: 0,
            scheduler_latency_threshold_ns: 1_000_000,
            network_drop_threshold: 1,
            boot_id: 0,
            agent_epoch: 0,
        }
    }
}

/// Context copied by the eBPF program into every evidence record.
///
/// This is deliberately independent from `CycleContext`: it is a stable BPF
/// map ABI and only carries fields the kernel program needs to correlate an
/// observation with a currently active ESOP cycle.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KernelContext {
    pub boot_id: u64,
    pub agent_epoch: u64,
    pub cycle_seq: u64,
    pub transition_seq: u64,
    pub tracked_pid: u32,
    pub reserved: u32,
    pub scheduler_latency_threshold_ns: u64,
    pub network_drop_threshold: u64,
}

// SAFETY: The BPF map value is an all-integer C-compatible record without
// padding or invalid bit patterns.
unsafe impl Pod for KernelContext {}

impl KernelContext {
    pub const fn from_config(config: RuntimeConfig) -> Self {
        Self {
            boot_id: config.boot_id,
            agent_epoch: config.agent_epoch,
            cycle_seq: 0,
            transition_seq: 0,
            tracked_pid: config.tracked_pid,
            reserved: 0,
            scheduler_latency_threshold_ns: config.scheduler_latency_threshold_ns,
            network_drop_threshold: config.network_drop_threshold,
        }
    }

    pub const fn with_cycle(self, cycle: CycleContext) -> Self {
        Self {
            boot_id: cycle.boot_id,
            agent_epoch: self.agent_epoch,
            cycle_seq: cycle.cycle_seq,
            transition_seq: cycle.transition_seq,
            tracked_pid: self.tracked_pid,
            reserved: 0,
            scheduler_latency_threshold_ns: self.scheduler_latency_threshold_ns,
            network_drop_threshold: self.network_drop_threshold,
        }
    }
}

/// Per-CPU counters maintained by the BPF bundle.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KernelStats {
    pub emitted_events: u64,
    pub lost_events: u64,
    pub wakeups: u64,
    pub scheduler_stalls: u64,
    pub page_faults: u64,
    pub process_exits: u64,
    pub oom_events: u64,
}

// SAFETY: The BPF map value is an all-u64 C-compatible record without padding
// or invalid bit patterns.
unsafe impl Pod for KernelStats {}

impl KernelStats {
    fn saturating_add_assign(&mut self, other: Self) {
        self.emitted_events = self.emitted_events.saturating_add(other.emitted_events);
        self.lost_events = self.lost_events.saturating_add(other.lost_events);
        self.wakeups = self.wakeups.saturating_add(other.wakeups);
        self.scheduler_stalls = self.scheduler_stalls.saturating_add(other.scheduler_stalls);
        self.page_faults = self.page_faults.saturating_add(other.page_faults);
        self.process_exits = self.process_exits.saturating_add(other.process_exits);
        self.oom_events = self.oom_events.saturating_add(other.oom_events);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimePreflight {
    pub snapshot: CapabilitySnapshot,
    pub available_attach_mask: u64,
    pub kernel_major: u16,
    pub kernel_minor: u16,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PollReport {
    pub records_seen: usize,
    pub incidents_emitted: usize,
    pub malformed_records: usize,
    pub evidence_rejected: usize,
    pub newly_reported_lost_events: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceDecodeError {
    UnexpectedSize { actual: usize },
    InvalidDomain(u8),
    InvalidKind(u8),
    InvalidSeverity(u8),
}

#[derive(Debug)]
pub enum RuntimeError {
    InvalidConfiguration,
    Load(EbpfError),
    Map(MapError),
    Program(ProgramError),
    MissingMap(&'static str),
    MissingProgram(&'static str),
    Evidence(EvidenceDecodeError),
    Correlator(CorrelatorError),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration => formatter.write_str("invalid eBPF runtime configuration"),
            Self::Load(error) => write!(formatter, "failed to load BPF object: {error}"),
            Self::Map(error) => write!(formatter, "BPF map operation failed: {error}"),
            Self::Program(error) => write!(formatter, "BPF program operation failed: {error}"),
            Self::MissingMap(name) => write!(formatter, "required BPF map `{name}` is missing"),
            Self::MissingProgram(name) => {
                write!(
                    formatter,
                    "required BPF tracepoint program `{name}` is missing"
                )
            }
            Self::Evidence(error) => write!(formatter, "malformed BPF evidence: {error:?}"),
            Self::Correlator(error) => {
                write!(formatter, "runtime correlator rejected evidence: {error:?}")
            }
        }
    }
}

impl std::error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Load(error) => Some(error),
            Self::Map(error) => Some(error),
            Self::Program(error) => Some(error),
            _ => None,
        }
    }
}

impl From<EbpfError> for RuntimeError {
    fn from(error: EbpfError) -> Self {
        Self::Load(error)
    }
}

impl From<MapError> for RuntimeError {
    fn from(error: MapError) -> Self {
        Self::Map(error)
    }
}

impl From<ProgramError> for RuntimeError {
    fn from(error: ProgramError) -> Self {
        Self::Program(error)
    }
}

impl From<CorrelatorError> for RuntimeError {
    fn from(error: CorrelatorError) -> Self {
        Self::Correlator(error)
    }
}

#[derive(Clone, Copy)]
struct AttachSpec {
    point: AttachPoint,
    program: &'static str,
    category: &'static str,
    event: &'static str,
}

const ATTACH_SPECS: [AttachSpec; 6] = [
    AttachSpec {
        point: AttachPoint::SchedulerWakeup,
        program: "esop_sched_wakeup",
        category: "sched",
        event: "sched_wakeup",
    },
    AttachSpec {
        point: AttachPoint::SchedulerSwitch,
        program: "esop_sched_switch",
        category: "sched",
        event: "sched_switch",
    },
    AttachSpec {
        point: AttachPoint::ProcessExit,
        program: "esop_process_exit",
        category: "sched",
        event: "sched_process_exit",
    },
    AttachSpec {
        point: AttachPoint::PageFault,
        program: "esop_page_fault_user",
        category: "exceptions",
        event: "page_fault_user",
    },
    AttachSpec {
        point: AttachPoint::OomKill,
        program: "esop_oom_kill",
        category: "oom",
        event: "mark_victim",
    },
    AttachSpec {
        point: AttachPoint::NetworkDrop,
        program: "esop_network_drop",
        category: "skb",
        event: "kfree_skb",
    },
];

/// A loaded eBPF bundle. Dropping this value drops BPF maps and tracepoint
/// links, so unloading cannot leave the agent attached to the kernel.
pub struct BpfRuntime {
    bpf: Ebpf,
    events: RingBuf<MapData>,
    context: Array<MapData, KernelContext>,
    stats: PerCpuArray<MapData, KernelStats>,
    kernel_context: KernelContext,
    snapshot: CapabilitySnapshot,
    attach_mask: u64,
    reported_lost_events: u64,
}

impl BpfRuntime {
    /// Inspect kernel prerequisites without loading or attaching a BPF object.
    pub fn preflight(required_attach_mask: u64) -> RuntimePreflight {
        let release = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
        let (kernel_major, kernel_minor) = parse_kernel_version(&release);
        let available_attach_mask = available_attach_mask();
        let mut capability_mask = CAPABILITY_VERIFIER;
        if Path::new("/sys/kernel/btf/vmlinux").is_file() {
            capability_mask |= CAPABILITY_BTF;
        }
        if kernel_at_least(kernel_major, kernel_minor, 5, 8) {
            capability_mask |= CAPABILITY_RINGBUF;
        }
        if has_bpf_permissions() {
            capability_mask |= CAPABILITY_PERMISSION;
        }
        let snapshot = CapabilitySnapshot::new(
            stable_hash(release.trim().as_bytes()),
            available_attach_mask,
            required_attach_mask,
            capability_mask,
        );
        RuntimePreflight {
            snapshot,
            available_attach_mask,
            kernel_major,
            kernel_minor,
        }
    }

    /// Load a prebuilt CO-RE object and attach every enabled tracepoint that is
    /// available. Missing optional points keep the runtime alive but result in
    /// a degraded [`CapabilitySnapshot`].
    pub fn load(
        object_path: impl AsRef<Path>,
        config: RuntimeConfig,
    ) -> Result<Self, RuntimeError> {
        if !config.valid() {
            return Err(RuntimeError::InvalidConfiguration);
        }

        let preflight = Self::preflight(config.required_attach_mask);
        let mut bpf = Ebpf::load_file(object_path)?;
        let events = RingBuf::try_from(
            bpf.take_map(MAP_EVENTS)
                .ok_or(RuntimeError::MissingMap(MAP_EVENTS))?,
        )?;
        let context = Array::try_from(
            bpf.take_map(MAP_CONTEXT)
                .ok_or(RuntimeError::MissingMap(MAP_CONTEXT))?,
        )?;
        let stats = PerCpuArray::try_from(
            bpf.take_map(MAP_STATS)
                .ok_or(RuntimeError::MissingMap(MAP_STATS))?,
        )?;

        let mut runtime = Self {
            bpf,
            events,
            context,
            stats,
            kernel_context: KernelContext::from_config(config),
            snapshot: preflight.snapshot,
            attach_mask: 0,
            reported_lost_events: 0,
        };
        runtime.context.set(0, runtime.kernel_context, 0)?;
        runtime.attach_enabled(config.enabled_attach_mask, config.required_attach_mask)?;
        runtime.snapshot = CapabilitySnapshot::new(
            preflight.snapshot.kernel_release_hash,
            runtime.attach_mask,
            config.required_attach_mask,
            preflight.snapshot.capability_mask,
        );
        Ok(runtime)
    }

    pub const fn capability_snapshot(&self) -> CapabilitySnapshot {
        self.snapshot
    }

    pub const fn attach_mask(&self) -> u64 {
        self.attach_mask
    }

    pub const fn kernel_context(&self) -> KernelContext {
        self.kernel_context
    }

    /// Project loader health into the observation agent. This only changes the
    /// host-observation lease state; it cannot grant a motion permit.
    pub fn apply_capability_snapshot<const INCIDENTS: usize>(
        &self,
        agent: &mut RuntimeAgent<INCIDENTS>,
    ) {
        agent.health_mut().set_capability_snapshot(self.snapshot);
    }

    /// Copy the current cycle identity into the BPF map. This is an ordinary
    /// control-plane update and must not be called from a hard realtime path.
    pub fn update_cycle_context(&mut self, context: CycleContext) -> Result<(), RuntimeError> {
        self.kernel_context = self.kernel_context.with_cycle(context);
        self.context.set(0, self.kernel_context, 0)?;
        Ok(())
    }

    pub fn update_tracking(
        &mut self,
        tracked_pid: u32,
        scheduler_latency_threshold_ns: u64,
        network_drop_threshold: u64,
    ) -> Result<(), RuntimeError> {
        if scheduler_latency_threshold_ns == 0 || network_drop_threshold == 0 {
            return Err(RuntimeError::InvalidConfiguration);
        }
        self.kernel_context.tracked_pid = tracked_pid;
        self.kernel_context.scheduler_latency_threshold_ns = scheduler_latency_threshold_ns;
        self.kernel_context.network_drop_threshold = network_drop_threshold;
        self.context.set(0, self.kernel_context, 0)?;
        Ok(())
    }

    /// Consume at most `max_records` fixed-size ring-buffer records. Invalid
    /// records are counted and discarded; they never enter the correlator.
    pub fn poll<const INCIDENTS: usize>(
        &mut self,
        agent: &mut RuntimeAgent<INCIDENTS>,
        max_records: usize,
    ) -> Result<PollReport, RuntimeError> {
        let mut report = PollReport::default();
        for _ in 0..max_records {
            let Some(record) = self.events.next() else {
                break;
            };
            report.records_seen += 1;
            match decode_evidence(&record) {
                Ok(evidence) => match agent.ingest(evidence) {
                    Ok(Some(_)) => report.incidents_emitted += 1,
                    Ok(None) => {}
                    Err(_) => report.evidence_rejected += 1,
                },
                Err(_) => report.malformed_records += 1,
            }
        }

        let stats = self.statistics()?;
        let delta = stats.lost_events.saturating_sub(self.reported_lost_events);
        self.reported_lost_events = stats.lost_events;
        if delta != 0 {
            let count = u32::try_from(delta).unwrap_or(u32::MAX);
            agent.health_mut().record_event_loss(count);
            report.newly_reported_lost_events = count;
        }
        Ok(report)
    }

    pub fn statistics(&self) -> Result<KernelStats, RuntimeError> {
        let values = self.stats.get(&0, 0)?;
        let mut aggregate = KernelStats::default();
        for value in values.iter() {
            aggregate.saturating_add_assign(*value);
        }
        Ok(aggregate)
    }

    fn attach_enabled(
        &mut self,
        enabled_attach_mask: u64,
        required_attach_mask: u64,
    ) -> Result<(), RuntimeError> {
        for spec in ATTACH_SPECS {
            if enabled_attach_mask & spec.point.mask() == 0 {
                continue;
            }
            let required = required_attach_mask & spec.point.mask() != 0;
            let Some(program) = self.bpf.program_mut(spec.program) else {
                if required {
                    return Err(RuntimeError::MissingProgram(spec.program));
                }
                continue;
            };
            let tracepoint: &mut TracePoint = match program.try_into() {
                Ok(tracepoint) => tracepoint,
                Err(error) if required => return Err(RuntimeError::Program(error)),
                Err(_) => continue,
            };
            if let Err(error) = tracepoint.load() {
                if required {
                    return Err(RuntimeError::Program(error));
                }
                continue;
            }
            if let Err(error) = tracepoint.attach(spec.category, spec.event) {
                if required {
                    return Err(RuntimeError::Program(error));
                }
                continue;
            }
            self.attach_mask |= spec.point.mask();
        }
        Ok(())
    }
}

pub fn decode_evidence(bytes: &[u8]) -> Result<RuntimeEvidence, EvidenceDecodeError> {
    if bytes.len() != EVENT_RECORD_BYTES {
        return Err(EvidenceDecodeError::UnexpectedSize {
            actual: bytes.len(),
        });
    }
    let domain = decode_domain(bytes[92])?;
    let kind = decode_kind(bytes[93])?;
    let severity = decode_severity(bytes[94])?;
    Ok(RuntimeEvidence {
        evidence_id: read_u64(bytes, 0),
        boot_id: read_u64(bytes, 8),
        agent_epoch: read_u64(bytes, 16),
        timestamp_ns: read_u64(bytes, 24),
        cycle_seq: read_u64(bytes, 32),
        transition_seq: read_u64(bytes, 40),
        pid: read_u32(bytes, 48),
        tid: read_u32(bytes, 52),
        cpu: read_u16(bytes, 56),
        irq: read_u16(bytes, 58),
        netdev_ifindex: read_u32(bytes, 60),
        observed_value: read_u64(bytes, 64),
        threshold: read_u64(bytes, 72),
        duration_ns: read_u64(bytes, 80),
        count: read_u32(bytes, 88),
        domain,
        kind,
        severity,
        reserved: bytes[95],
    })
}

fn decode_domain(value: u8) -> Result<EvidenceDomain, EvidenceDecodeError> {
    match value {
        0 => Ok(EvidenceDomain::KernelScheduler),
        1 => Ok(EvidenceDomain::KernelIrq),
        2 => Ok(EvidenceDomain::KernelNetwork),
        3 => Ok(EvidenceDomain::KernelMemory),
        4 => Ok(EvidenceDomain::KernelProcess),
        5 => Ok(EvidenceDomain::UserEsop),
        6 => Ok(EvidenceDomain::UserRos),
        7 => Ok(EvidenceDomain::UserZenoh),
        8 => Ok(EvidenceDomain::Correlator),
        _ => Err(EvidenceDecodeError::InvalidDomain(value)),
    }
}

fn decode_kind(value: u8) -> Result<EvidenceKind, EvidenceDecodeError> {
    match value {
        0 => Ok(EvidenceKind::SchedulerRunqueueLatency),
        1 => Ok(EvidenceKind::IrqCpuTime),
        2 => Ok(EvidenceKind::NetworkDrop),
        3 => Ok(EvidenceKind::PageFault),
        4 => Ok(EvidenceKind::OomKill),
        5 => Ok(EvidenceKind::ProcessExit),
        6 => Ok(EvidenceKind::CpuThrottle),
        7 => Ok(EvidenceKind::GatewayStall),
        8 => Ok(EvidenceKind::AgentCapabilityFailure),
        _ => Err(EvidenceDecodeError::InvalidKind(value)),
    }
}

fn decode_severity(value: u8) -> Result<IncidentSeverity, EvidenceDecodeError> {
    match value {
        0 => Ok(IncidentSeverity::Info),
        1 => Ok(IncidentSeverity::Warning),
        2 => Ok(IncidentSeverity::Error),
        3 => Ok(IncidentSeverity::Critical),
        _ => Err(EvidenceDecodeError::InvalidSeverity(value)),
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn available_attach_mask() -> u64 {
    ATTACH_SPECS.iter().fold(0, |mask, spec| {
        if tracepoint_available(spec.category, spec.event) {
            mask | spec.point.mask()
        } else {
            mask
        }
    })
}

fn tracepoint_available(category: &str, event: &str) -> bool {
    TRACEFS_EVENT_ROOTS.iter().any(|root| {
        Path::new(root)
            .join(category)
            .join(event)
            .join("id")
            .is_file()
    })
}

fn parse_kernel_version(release: &str) -> (u16, u16) {
    let mut parts = release.split('.');
    let major = parts
        .next()
        .and_then(|part| leading_number(part).parse().ok())
        .unwrap_or(0);
    let minor = parts
        .next()
        .and_then(|part| leading_number(part).parse().ok())
        .unwrap_or(0);
    (major, minor)
}

fn leading_number(value: &str) -> &str {
    let length = value.bytes().take_while(u8::is_ascii_digit).count();
    &value[..length]
}

fn kernel_at_least(major: u16, minor: u16, required_major: u16, required_minor: u16) -> bool {
    major > required_major || (major == required_major && minor >= required_minor)
}

fn has_bpf_permissions() -> bool {
    let Ok(status) = fs::read_to_string("/proc/self/status") else {
        return false;
    };
    let effective_uid_is_root = status.lines().any(|line| {
        line.strip_prefix("Uid:")
            .and_then(|value| value.split_whitespace().nth(1))
            == Some("0")
    });
    if effective_uid_is_root {
        return true;
    }
    let Some(capabilities) = status
        .lines()
        .find_map(|line| line.strip_prefix("CapEff:"))
        .and_then(|value| u64::from_str_radix(value.trim(), 16).ok())
    else {
        return false;
    };
    has_capability(capabilities, CAP_BPF)
        || has_capability(capabilities, CAP_PERFMON)
        || has_capability(capabilities, CAP_SYS_ADMIN)
}

fn has_capability(mask: u64, capability: u32) -> bool {
    capability < u64::BITS && mask & (1_u64 << capability) != 0
}

fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn decodes_the_fixed_bpf_event_abi_without_transmuting_enums() {
        let mut bytes = [0_u8; EVENT_RECORD_BYTES];
        put_u64(&mut bytes, 0, 1);
        put_u64(&mut bytes, 8, 2);
        put_u64(&mut bytes, 16, 3);
        put_u64(&mut bytes, 24, 4);
        put_u64(&mut bytes, 32, 5);
        put_u64(&mut bytes, 40, 6);
        put_u32(&mut bytes, 48, 7);
        put_u32(&mut bytes, 52, 8);
        put_u16(&mut bytes, 56, 9);
        put_u16(&mut bytes, 58, 10);
        put_u32(&mut bytes, 60, 11);
        put_u64(&mut bytes, 64, 12);
        put_u64(&mut bytes, 72, 13);
        put_u64(&mut bytes, 80, 14);
        put_u32(&mut bytes, 88, 15);
        bytes[92] = EvidenceDomain::KernelScheduler as u8;
        bytes[93] = EvidenceKind::SchedulerRunqueueLatency as u8;
        bytes[94] = IncidentSeverity::Error as u8;

        let evidence = decode_evidence(&bytes).unwrap();
        assert_eq!(evidence.evidence_id, 1);
        assert_eq!(evidence.boot_id, 2);
        assert_eq!(evidence.agent_epoch, 3);
        assert_eq!(evidence.pid, 7);
        assert_eq!(evidence.cpu, 9);
        assert_eq!(evidence.duration_ns, 14);
        assert_eq!(evidence.domain, EvidenceDomain::KernelScheduler);
        assert_eq!(evidence.kind, EvidenceKind::SchedulerRunqueueLatency);
    }

    #[test]
    fn rejects_invalid_fixed_event_fields() {
        let mut bytes = [0_u8; EVENT_RECORD_BYTES];
        bytes[92] = 99;
        assert_eq!(
            decode_evidence(&bytes),
            Err(EvidenceDecodeError::InvalidDomain(99))
        );
        assert_eq!(
            decode_evidence(&bytes[..EVENT_RECORD_BYTES - 1]),
            Err(EvidenceDecodeError::UnexpectedSize {
                actual: EVENT_RECORD_BYTES - 1
            })
        );
    }

    #[test]
    fn parses_kernel_versions_without_accepting_suffixes_as_numbers() {
        assert_eq!(parse_kernel_version("6.8.0-138-generic"), (6, 8));
        assert_eq!(parse_kernel_version("5.15"), (5, 15));
        assert_eq!(parse_kernel_version("invalid"), (0, 0));
        assert!(kernel_at_least(6, 8, 5, 8));
        assert!(!kernel_at_least(5, 4, 5, 8));
    }

    #[test]
    fn runtime_config_rejects_unrequested_required_points_and_zero_thresholds() {
        let mut config = RuntimeConfig::default();
        assert!(config.valid());
        config.required_attach_mask = ATTACH_ALL << 1;
        assert!(!config.valid());
        config.required_attach_mask = ATTACH_SCHED_WAKEUP;
        config.network_drop_threshold = 0;
        assert!(!config.valid());
    }

    #[test]
    fn kernel_context_preserves_tracking_when_cycle_context_changes() {
        let config = RuntimeConfig {
            tracked_pid: 42,
            scheduler_latency_threshold_ns: 100,
            network_drop_threshold: 3,
            boot_id: 5,
            agent_epoch: 7,
            ..RuntimeConfig::default()
        };
        let context = KernelContext::from_config(config).with_cycle(CycleContext {
            boot_id: 5,
            cycle_seq: 11,
            transition_seq: 13,
            ..CycleContext::EMPTY
        });
        assert_eq!(context.agent_epoch, 7);
        assert_eq!(context.tracked_pid, 42);
        assert_eq!(context.cycle_seq, 11);
        assert_eq!(context.transition_seq, 13);
    }
}
