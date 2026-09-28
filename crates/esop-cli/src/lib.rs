use std::fmt::Write as _;
use std::path::PathBuf;

use esop_proto::v1::{
    IncidentSeverity, LifecycleState, OperationalStatus, QueryReply, RobotState,
    RuntimeObservationStatus,
};
use esop_zenoh_gateway::runtime::MAX_QUERY_RECORDS;

pub const DEFAULT_LIMIT: u32 = 16;
pub const DEFAULT_TIMEOUT_MS: u64 = 2_000;
pub const DEFAULT_WATCH_INTERVAL_MS: u64 = 1_000;
pub const MIN_WATCH_INTERVAL_MS: u64 = 50;
pub const MAX_WATCH_INTERVAL_MS: u64 = 60_000;
pub const MAX_TIMEOUT_MS: u64 = 60_000;
pub const MAX_WATCH_ITERATIONS: u32 = 1_000_000;

pub const USAGE: &str = "usage: esop --robot <id> [--fleet <id>] [--boot-id <id>] [--limit <1..32>] [--config <file>] [--timeout-ms <1..60000>] <command>\n\
commands:\n\
  status\n\
  domain list\n\
  dc\n\
  lifecycle\n\
  incident list\n\
  doctor\n\
  watch [status|dc|lifecycle|doctor|domain list|incident list] [--interval-ms <50..60000>] [--iterations <count>]";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum View {
    Status,
    DomainList,
    Dc,
    Lifecycle,
    IncidentList,
    Doctor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WatchPolicy {
    pub interval_ms: u64,
    pub iterations: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cli {
    pub fleet: String,
    pub robot: String,
    pub boot_id: u64,
    pub limit: u32,
    pub config: Option<PathBuf>,
    pub timeout_ms: u64,
    pub view: View,
    pub watch: Option<WatchPolicy>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseOutcome {
    Help,
    Run(Cli),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseError {
    MissingCommand,
    MissingRobot,
    MissingValue(String),
    InvalidNumber { option: String, value: String },
    OutOfRange { option: String, value: u64 },
    UnknownOption(String),
    UnknownCommand(Vec<String>),
    WatchOptionWithoutWatch(&'static str),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingCommand => formatter.write_str("missing command"),
            Self::MissingRobot => formatter.write_str("missing --robot"),
            Self::MissingValue(option) => write!(formatter, "missing value for {option}"),
            Self::InvalidNumber { option, value } => {
                write!(formatter, "invalid numeric value {value:?} for {option}")
            }
            Self::OutOfRange { option, value } => {
                write!(formatter, "value {value} is out of range for {option}")
            }
            Self::UnknownOption(option) => write!(formatter, "unknown option {option:?}"),
            Self::UnknownCommand(command) => write!(formatter, "unknown command {command:?}"),
            Self::WatchOptionWithoutWatch(option) => {
                write!(formatter, "{option} is valid only with watch")
            }
        }
    }
}

impl std::error::Error for ParseError {}

pub fn parse_args<I, S>(arguments: I) -> Result<ParseOutcome, ParseError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let arguments: Vec<String> = arguments.into_iter().map(Into::into).collect();
    if arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help" | "help"))
    {
        return Ok(ParseOutcome::Help);
    }

    let mut fleet = "default".to_owned();
    let mut robot = None;
    let mut boot_id = 0;
    let mut limit = DEFAULT_LIMIT;
    let mut config = None;
    let mut timeout_ms = DEFAULT_TIMEOUT_MS;
    let mut interval_ms = DEFAULT_WATCH_INTERVAL_MS;
    let mut iterations = None;
    let mut interval_set = false;
    let mut iterations_set = false;
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        if !argument.starts_with('-') {
            positionals.push(argument.clone());
            index += 1;
            continue;
        }
        if !matches!(
            argument.as_str(),
            "--fleet"
                | "--robot"
                | "--config"
                | "--boot-id"
                | "--limit"
                | "--timeout-ms"
                | "--interval-ms"
                | "--iterations"
        ) {
            return Err(ParseError::UnknownOption(argument.clone()));
        }
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| ParseError::MissingValue(argument.clone()))?;
        match argument.as_str() {
            "--fleet" => fleet = value.clone(),
            "--robot" => robot = Some(value.clone()),
            "--config" => config = Some(PathBuf::from(value)),
            "--boot-id" => boot_id = parse_u64(argument, value)?,
            "--limit" => {
                let parsed = parse_u64(argument, value)?;
                if !(1..=u64::from(MAX_QUERY_RECORDS)).contains(&parsed) {
                    return Err(ParseError::OutOfRange {
                        option: argument.clone(),
                        value: parsed,
                    });
                }
                limit = parsed as u32;
            }
            "--timeout-ms" => {
                timeout_ms = parse_u64(argument, value)?;
                if !(1..=MAX_TIMEOUT_MS).contains(&timeout_ms) {
                    return Err(ParseError::OutOfRange {
                        option: argument.clone(),
                        value: timeout_ms,
                    });
                }
            }
            "--interval-ms" => {
                interval_ms = parse_u64(argument, value)?;
                interval_set = true;
                if !(MIN_WATCH_INTERVAL_MS..=MAX_WATCH_INTERVAL_MS).contains(&interval_ms) {
                    return Err(ParseError::OutOfRange {
                        option: argument.clone(),
                        value: interval_ms,
                    });
                }
            }
            "--iterations" => {
                let parsed = parse_u64(argument, value)?;
                iterations_set = true;
                if !(1..=u64::from(MAX_WATCH_ITERATIONS)).contains(&parsed) {
                    return Err(ParseError::OutOfRange {
                        option: argument.clone(),
                        value: parsed,
                    });
                }
                iterations = Some(parsed as u32);
            }
            _ => return Err(ParseError::UnknownOption(argument.clone())),
        }
        index += 2;
    }

    if positionals.is_empty() {
        return Err(ParseError::MissingCommand);
    }
    let watching = positionals.first().map(String::as_str) == Some("watch");
    if interval_set && !watching {
        return Err(ParseError::WatchOptionWithoutWatch("--interval-ms"));
    }
    if iterations_set && !watching {
        return Err(ParseError::WatchOptionWithoutWatch("--iterations"));
    }
    let view = parse_view(if watching {
        &positionals[1..]
    } else {
        &positionals
    })?;
    let robot = robot.ok_or(ParseError::MissingRobot)?;
    Ok(ParseOutcome::Run(Cli {
        fleet,
        robot,
        boot_id,
        limit,
        config,
        timeout_ms,
        view,
        watch: watching.then_some(WatchPolicy {
            interval_ms,
            iterations,
        }),
    }))
}

fn parse_u64(option: &str, value: &str) -> Result<u64, ParseError> {
    value.parse().map_err(|_| ParseError::InvalidNumber {
        option: option.to_owned(),
        value: value.to_owned(),
    })
}

fn parse_view(command: &[String]) -> Result<View, ParseError> {
    match command
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["status"] => Ok(View::Status),
        ["domain", "list"] => Ok(View::DomainList),
        ["dc"] => Ok(View::Dc),
        ["lifecycle"] => Ok(View::Lifecycle),
        ["incident", "list"] => Ok(View::IncidentList),
        ["doctor"] => Ok(View::Doctor),
        _ => Err(ParseError::UnknownCommand(command.to_vec())),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rendered {
    pub text: String,
    pub healthy: bool,
    pub latest_sequence: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    MissingState,
    UnknownLifecycleState(i32),
    UnknownIncidentSeverity(i32),
    InvalidDomainOrder { expected: u32, actual: u32 },
    InvalidObservationHealth(u32),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingState => formatter.write_str("query reply contains no state"),
            Self::UnknownLifecycleState(value) => {
                write!(formatter, "unknown lifecycle state {value}")
            }
            Self::UnknownIncidentSeverity(value) => {
                write!(formatter, "unknown incident severity {value}")
            }
            Self::InvalidDomainOrder { expected, actual } => write!(
                formatter,
                "domain order is invalid: expected {expected}, received {actual}"
            ),
            Self::InvalidObservationHealth(value) => {
                write!(formatter, "unknown runtime observation health {value}")
            }
        }
    }
}

impl std::error::Error for RenderError {}

pub fn render(view: View, reply: &QueryReply) -> Result<Rendered, RenderError> {
    if view == View::IncidentList {
        return render_incidents(reply);
    }
    let state = reply.states.last().ok_or(RenderError::MissingState)?;
    validate_state(state)?;
    match view {
        View::Status => render_status(reply, state),
        View::DomainList => render_domains(state),
        View::Dc => render_dc(state),
        View::Lifecycle => render_lifecycle(state),
        View::Doctor => render_doctor(reply, state),
        View::IncidentList => unreachable!(),
    }
}

fn validate_state(state: &RobotState) -> Result<(), RenderError> {
    if let Some(lifecycle) = &state.lifecycle {
        LifecycleState::try_from(lifecycle.state)
            .map_err(|_| RenderError::UnknownLifecycleState(lifecycle.state))?;
    }
    if let Some(operational) = &state.operational {
        for (expected, domain) in operational.domains.iter().enumerate() {
            if domain.domain != expected as u32 {
                return Err(RenderError::InvalidDomainOrder {
                    expected: expected as u32,
                    actual: domain.domain,
                });
            }
        }
        if let Some(observation) = &operational.runtime_observation
            && observation.health > 2
        {
            return Err(RenderError::InvalidObservationHealth(observation.health));
        }
    }
    Ok(())
}

fn render_status(reply: &QueryReply, state: &RobotState) -> Result<Rendered, RenderError> {
    let checks = doctor_checks(reply, state)?;
    let healthy = checks.iter().all(|check| check.passed);
    let mut text = String::new();
    writeln!(
        text,
        "robot={} boot={} sequence={} time_ns={} health={}",
        state.robot_id,
        state.boot_id,
        state.sequence,
        state.monotonic_time_ns,
        if healthy { "ok" } else { "degraded" }
    )
    .unwrap();
    if let Some(operational) = &state.operational {
        writeln!(
            text,
            "link={} al={} fault_bitmap=0x{:08x} command_age_cycles={} deadline_misses={}",
            up_down(operational.link_up),
            al_state_name(operational.al_state),
            operational.fault_bitmap,
            operational.command_age_cycles,
            operational.deadline_misses
        )
        .unwrap();
        writeln!(
            text,
            "dc={} offset_ns={} domains={} invalid_domains={}",
            locked_unlocked(operational.dc_locked),
            operational.dc_offset_ns,
            operational.domains.len(),
            operational
                .domains
                .iter()
                .filter(|domain| {
                    !domain.valid || !domain.complete || domain.actual_wkc != domain.expected_wkc
                })
                .count()
        )
        .unwrap();
        render_observation_line(&mut text, operational.runtime_observation.as_ref());
    } else {
        text.push_str("operational=unavailable\n");
    }
    if let Some(lifecycle) = &state.lifecycle {
        writeln!(
            text,
            "lifecycle={} permit={} first_blocking_code={} latched_fault_code={}",
            lifecycle_name(lifecycle.state)?,
            yes_no(lifecycle.motion_permit_current),
            lifecycle.first_blocking_code,
            lifecycle.latched_fault_code
        )
        .unwrap();
    } else {
        text.push_str("lifecycle=unavailable\n");
    }
    writeln!(
        text,
        "incidents={} truncated={}",
        reply.incidents.len(),
        yes_no(reply.truncated)
    )
    .unwrap();
    Ok(rendered(text, healthy, state.sequence))
}

fn render_domains(state: &RobotState) -> Result<Rendered, RenderError> {
    let mut text = String::new();
    let Some(operational) = &state.operational else {
        text.push_str("domain_status=unavailable\n");
        return Ok(rendered(text, false, state.sequence));
    };
    text.push_str("domain expected_wkc actual_wkc valid complete mismatch_streak last_valid_cycle input_age_cycles\n");
    let mut healthy = !operational.domains.is_empty();
    for domain in &operational.domains {
        writeln!(
            text,
            "{} {} {} {} {} {} {} {}",
            domain.domain,
            domain.expected_wkc,
            domain.actual_wkc,
            yes_no(domain.valid),
            yes_no(domain.complete),
            domain.consecutive_wkc_mismatches,
            domain.last_valid_cycle,
            domain.input_age_cycles
        )
        .unwrap();
        healthy &= domain.valid && domain.complete && domain.expected_wkc == domain.actual_wkc;
    }
    Ok(rendered(text, healthy, state.sequence))
}

fn render_dc(state: &RobotState) -> Result<Rendered, RenderError> {
    let (text, healthy) = if let Some(operational) = &state.operational {
        (
            format!(
                "dc={} offset_ns={} quality_gate={}\n",
                locked_unlocked(operational.dc_locked),
                operational.dc_offset_ns,
                state
                    .quality
                    .as_ref()
                    .map(|quality| yes_no(quality.distributed_clock_locked))
                    .unwrap_or("unavailable")
            ),
            operational.dc_locked
                && state
                    .quality
                    .as_ref()
                    .is_some_and(|quality| quality.distributed_clock_locked),
        )
    } else {
        ("dc=unavailable\n".to_owned(), false)
    };
    Ok(rendered(text, healthy, state.sequence))
}

fn render_lifecycle(state: &RobotState) -> Result<Rendered, RenderError> {
    let Some(lifecycle) = &state.lifecycle else {
        return Ok(rendered(
            "lifecycle=unavailable\n".to_owned(),
            false,
            state.sequence,
        ));
    };
    let state_name = lifecycle_name(lifecycle.state)?;
    let healthy = matches!(
        LifecycleState::try_from(lifecycle.state),
        Ok(LifecycleState::Ready | LifecycleState::Active)
    ) && lifecycle.latched_fault_code == 0;
    let mut text = String::new();
    writeln!(
        text,
        "state={} permit={} permit_epoch={} permit_expires_at_ns={}",
        state_name,
        yes_no(lifecycle.motion_permit_current),
        lifecycle.permit_epoch,
        lifecycle.permit_expires_at_ns
    )
    .unwrap();
    writeln!(
        text,
        "gates required=0x{:04x} valid=0x{:04x} qualified=0x{:04x} ready=0x{:04x}",
        lifecycle.required_gate_mask,
        lifecycle.valid_gate_mask,
        lifecycle.qualified_gate_mask,
        lifecycle.ready_gate_mask
    )
    .unwrap();
    writeln!(
        text,
        "first_blocking_code={} latched_fault_code={} transition_sequence={} recovery_count={}",
        lifecycle.first_blocking_code,
        lifecycle.latched_fault_code,
        lifecycle.transition_sequence,
        lifecycle.recovery_count
    )
    .unwrap();
    Ok(rendered(text, healthy, state.sequence))
}

fn render_incidents(reply: &QueryReply) -> Result<Rendered, RenderError> {
    let mut text = String::new();
    let mut healthy = true;
    if reply.incidents.is_empty() {
        text.push_str("incidents=none\n");
    }
    for incident in &reply.incidents {
        let severity = IncidentSeverity::try_from(incident.severity)
            .map_err(|_| RenderError::UnknownIncidentSeverity(incident.severity))?;
        healthy &= !matches!(
            severity,
            IncidentSeverity::Error | IncidentSeverity::Critical
        );
        writeln!(
            text,
            "id={} severity={} reason={} confidence={} component={} cycles={}..{} events={} lost={} action={}",
            one_line(&incident.incident_id),
            incident_severity_name(severity),
            incident.reason_code,
            incident.confidence_percent,
            one_line(&incident.affected_component),
            incident.cycle_first,
            incident.cycle_last,
            incident.event_count,
            incident.lost_event_count,
            one_line(&incident.suggested_action)
        )
        .unwrap();
    }
    Ok(Rendered {
        text,
        healthy,
        latest_sequence: reply.states.last().map(|state| state.sequence),
    })
}

fn render_doctor(reply: &QueryReply, state: &RobotState) -> Result<Rendered, RenderError> {
    let checks = doctor_checks(reply, state)?;
    let healthy = checks.iter().all(|check| check.passed);
    let mut text = String::new();
    for check in checks {
        writeln!(
            text,
            "{} {} {}",
            if check.passed { "PASS" } else { "FAIL" },
            check.name,
            check.detail
        )
        .unwrap();
    }
    writeln!(
        text,
        "result={}",
        if healthy { "healthy" } else { "degraded" }
    )
    .unwrap();
    Ok(rendered(text, healthy, state.sequence))
}

struct Check {
    name: &'static str,
    passed: bool,
    detail: String,
}

fn doctor_checks(reply: &QueryReply, state: &RobotState) -> Result<Vec<Check>, RenderError> {
    let mut checks = Vec::new();
    let operational = state.operational.as_ref();
    checks.push(Check {
        name: "operational_status",
        passed: operational.is_some(),
        detail: availability(operational.is_some()).to_owned(),
    });
    checks.push(Check {
        name: "link",
        passed: operational.is_some_and(|status| status.link_up),
        detail: operational
            .map(|status| up_down(status.link_up))
            .unwrap_or("unavailable")
            .to_owned(),
    });
    checks.push(Check {
        name: "dc",
        passed: operational.is_some_and(|status| status.dc_locked),
        detail: operational
            .map(|status| {
                format!(
                    "{} offset_ns={}",
                    locked_unlocked(status.dc_locked),
                    status.dc_offset_ns
                )
            })
            .unwrap_or_else(|| "unavailable".to_owned()),
    });
    checks.push(Check {
        name: "domains",
        passed: operational.is_some_and(domains_healthy),
        detail: operational
            .map(|status| format!("count={}", status.domains.len()))
            .unwrap_or_else(|| "unavailable".to_owned()),
    });
    checks.push(Check {
        name: "quality",
        passed: state.quality.as_ref().is_some_and(quality_healthy),
        detail: availability(state.quality.is_some()).to_owned(),
    });
    let lifecycle = state.lifecycle.as_ref();
    let lifecycle_healthy = if let Some(lifecycle) = lifecycle {
        matches!(
            LifecycleState::try_from(lifecycle.state),
            Ok(LifecycleState::Ready | LifecycleState::Active)
        ) && lifecycle.first_blocking_code == 0
            && lifecycle.latched_fault_code == 0
            && (lifecycle.state != LifecycleState::Active as i32 || lifecycle.motion_permit_current)
    } else {
        false
    };
    checks.push(Check {
        name: "lifecycle",
        passed: lifecycle_healthy,
        detail: lifecycle
            .map(|value| lifecycle_name(value.state).map(str::to_owned))
            .transpose()?
            .unwrap_or_else(|| "unavailable".to_owned()),
    });
    let observation = operational.and_then(|status| status.runtime_observation.as_ref());
    checks.push(Check {
        name: "ebpf_observation",
        passed: observation.is_some_and(observation_healthy),
        detail: observation
            .map(observation_detail)
            .unwrap_or_else(|| "unavailable".to_owned()),
    });
    let mut incident_healthy = true;
    for incident in &reply.incidents {
        let severity = IncidentSeverity::try_from(incident.severity)
            .map_err(|_| RenderError::UnknownIncidentSeverity(incident.severity))?;
        incident_healthy &= !matches!(
            severity,
            IncidentSeverity::Error | IncidentSeverity::Critical
        );
    }
    checks.push(Check {
        name: "incidents",
        passed: incident_healthy,
        detail: format!("count={}", reply.incidents.len()),
    });
    Ok(checks)
}

fn quality_healthy(quality: &esop_proto::v1::QualitySummary) -> bool {
    quality.platform_ready
        && quality.configuration_ready
        && quality.topology_valid
        && quality.distributed_clock_locked
        && quality.drive_ready
        && quality.domain_valid
        && quality.wkc_valid
        && quality.command_current
        && quality.supervisor_healthy
        && quality.external_safety_clear
        && quality.cycle_within_budget
        && quality.first_fault_code == 0
}

fn domains_healthy(operational: &OperationalStatus) -> bool {
    !operational.domains.is_empty()
        && operational.domains.iter().all(|domain| {
            domain.valid && domain.complete && domain.expected_wkc == domain.actual_wkc
        })
}

fn observation_healthy(observation: &RuntimeObservationStatus) -> bool {
    observation.agent_epoch != 0 && observation.health == 0 && observation.lost_events == 0
}

fn observation_detail(observation: &RuntimeObservationStatus) -> String {
    format!(
        "{} epoch={} incidents={} lost={}",
        observation_health_name(observation.health),
        observation.agent_epoch,
        observation.incident_count,
        observation.lost_events
    )
}

fn render_observation_line(text: &mut String, observation: Option<&RuntimeObservationStatus>) {
    if let Some(observation) = observation {
        writeln!(text, "ebpf={}", observation_detail(observation)).unwrap();
    } else {
        text.push_str("ebpf=unavailable\n");
    }
}

fn rendered(text: String, healthy: bool, sequence: u64) -> Rendered {
    Rendered {
        text,
        healthy,
        latest_sequence: Some(sequence),
    }
}

fn lifecycle_name(value: i32) -> Result<&'static str, RenderError> {
    let state =
        LifecycleState::try_from(value).map_err(|_| RenderError::UnknownLifecycleState(value))?;
    Ok(match state {
        LifecycleState::Unspecified => "unspecified",
        LifecycleState::Qualifying => "qualifying",
        LifecycleState::Ready => "ready",
        LifecycleState::Active => "active",
        LifecycleState::Stopping => "stopping",
        LifecycleState::FaultLatched => "fault_latched",
        LifecycleState::Maintenance => "maintenance",
    })
}

fn incident_severity_name(value: IncidentSeverity) -> &'static str {
    match value {
        IncidentSeverity::Unspecified => "unspecified",
        IncidentSeverity::Info => "info",
        IncidentSeverity::Warning => "warning",
        IncidentSeverity::Error => "error",
        IncidentSeverity::Critical => "critical",
    }
}

fn observation_health_name(value: u32) -> &'static str {
    match value {
        0 => "healthy",
        1 => "degraded",
        2 => "failed",
        _ => "unknown",
    }
}

fn al_state_name(value: u32) -> String {
    let state = match value & 0x0f {
        1 => "init",
        2 => "preop",
        3 => "boot",
        4 => "safeop",
        8 => "op",
        _ => "unknown",
    };
    if value & 0x10 != 0 {
        format!("{state}+error(0x{value:02x})")
    } else {
        format!("{state}(0x{value:02x})")
    }
}

fn one_line(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

const fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

const fn up_down(value: bool) -> &'static str {
    if value { "up" } else { "down" }
}

const fn locked_unlocked(value: bool) -> &'static str {
    if value { "locked" } else { "unlocked" }
}

const fn availability(value: bool) -> &'static str {
    if value { "available" } else { "unavailable" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use esop_proto::CURRENT_SCHEMA_VERSION;
    use esop_proto::v1::{
        DomainStatus, LifecycleSummary, OperationalStatus, QualitySummary, RuntimeIncident,
    };

    fn parse(arguments: &[&str]) -> Result<ParseOutcome, ParseError> {
        parse_args(arguments.iter().copied())
    }

    fn healthy_reply() -> QueryReply {
        QueryReply {
            robot_id: "robot_01".into(),
            boot_id: 7,
            schema_version: CURRENT_SCHEMA_VERSION,
            states: vec![RobotState {
                robot_id: "robot_01".into(),
                boot_id: 7,
                sequence: 9,
                monotonic_time_ns: 1_000,
                schema_version: CURRENT_SCHEMA_VERSION,
                lifecycle: Some(LifecycleSummary {
                    state: LifecycleState::Active as i32,
                    motion_permit_current: true,
                    required_gate_mask: 0x7ff,
                    valid_gate_mask: 0x7ff,
                    qualified_gate_mask: 0x7ff,
                    ready_gate_mask: 0x7ff,
                    permit_epoch: 3,
                    ..Default::default()
                }),
                quality: Some(QualitySummary {
                    platform_ready: true,
                    configuration_ready: true,
                    topology_valid: true,
                    distributed_clock_locked: true,
                    drive_ready: true,
                    domain_valid: true,
                    wkc_valid: true,
                    command_current: true,
                    supervisor_healthy: true,
                    external_safety_clear: true,
                    cycle_within_budget: true,
                    first_fault_code: 0,
                }),
                operational: Some(OperationalStatus {
                    link_up: true,
                    al_state: 8,
                    dc_locked: true,
                    dc_offset_ns: -12,
                    domains: vec![DomainStatus {
                        domain: 0,
                        expected_wkc: 6,
                        actual_wkc: 6,
                        valid: true,
                        complete: true,
                        last_valid_cycle: 9,
                        ..Default::default()
                    }],
                    runtime_observation: Some(RuntimeObservationStatus {
                        agent_epoch: 2,
                        observed_at_ns: 900,
                        observation_window_ns: 500,
                        health: 0,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn parser_covers_commands_and_bounded_watch_options() {
        for (arguments, view) in [
            (vec!["--robot", "r", "status"], View::Status),
            (vec!["domain", "list", "--robot", "r"], View::DomainList),
            (vec!["--robot", "r", "dc"], View::Dc),
            (vec!["--robot", "r", "lifecycle"], View::Lifecycle),
            (vec!["--robot", "r", "incident", "list"], View::IncidentList),
            (vec!["--robot", "r", "doctor"], View::Doctor),
        ] {
            let ParseOutcome::Run(cli) = parse(&arguments).unwrap() else {
                panic!("command should run")
            };
            assert_eq!(cli.view, view);
            assert_eq!(cli.watch, None);
        }
        let ParseOutcome::Run(cli) = parse(&[
            "watch",
            "domain",
            "list",
            "--robot",
            "r",
            "--interval-ms",
            "50",
            "--iterations",
            "2",
        ])
        .unwrap() else {
            panic!("watch should run")
        };
        assert_eq!(cli.view, View::DomainList);
        assert_eq!(
            cli.watch,
            Some(WatchPolicy {
                interval_ms: 50,
                iterations: Some(2)
            })
        );
    }

    #[test]
    fn parser_rejects_missing_unknown_and_unbounded_values() {
        assert_eq!(parse(&[]), Err(ParseError::MissingCommand));
        assert_eq!(parse(&["status"]), Err(ParseError::MissingRobot));
        assert!(matches!(
            parse(&["--robot", "r", "unknown"]),
            Err(ParseError::UnknownCommand(_))
        ));
        assert_eq!(
            parse(&["--robot", "r", "status", "--unknown"]),
            Err(ParseError::UnknownOption("--unknown".to_owned()))
        );
        assert!(matches!(
            parse(&["--robot", "r", "status", "--limit", "33"]),
            Err(ParseError::OutOfRange { .. })
        ));
        assert!(matches!(
            parse(&["--robot", "r", "watch", "--interval-ms", "0"]),
            Err(ParseError::OutOfRange { .. })
        ));
        assert_eq!(
            parse(&["--robot", "r", "status", "--iterations", "1"]),
            Err(ParseError::WatchOptionWithoutWatch("--iterations"))
        );
        assert_eq!(parse(&["--help"]), Ok(ParseOutcome::Help));
    }

    #[test]
    fn renderers_expose_healthy_domain_dc_lifecycle_and_ebpf_state() {
        let reply = healthy_reply();
        let status = render(View::Status, &reply).unwrap();
        assert!(status.healthy);
        assert!(status.text.contains("dc=locked offset_ns=-12"));
        assert!(status.text.contains("ebpf=healthy epoch=2"));
        let domains = render(View::DomainList, &reply).unwrap();
        assert!(domains.healthy);
        assert!(domains.text.contains("0 6 6 yes yes"));
        let lifecycle = render(View::Lifecycle, &reply).unwrap();
        assert!(lifecycle.healthy);
        assert!(lifecycle.text.contains("state=active permit=yes"));
        let doctor = render(View::Doctor, &reply).unwrap();
        assert!(doctor.healthy);
        assert!(doctor.text.contains("result=healthy"));
    }

    #[test]
    fn doctor_fails_closed_on_missing_or_degraded_observation() {
        let mut reply = healthy_reply();
        let state = reply.states.last_mut().unwrap();
        let operational = state.operational.as_mut().unwrap();
        operational.dc_locked = false;
        operational.domains[0].actual_wkc = 5;
        operational
            .runtime_observation
            .as_mut()
            .unwrap()
            .lost_events = 1;
        let doctor = render(View::Doctor, &reply).unwrap();
        assert!(!doctor.healthy);
        assert!(doctor.text.contains("FAIL dc"));
        assert!(doctor.text.contains("FAIL domains"));
        assert!(doctor.text.contains("FAIL ebpf_observation"));

        reply.states[0].operational = None;
        let status = render(View::Status, &reply).unwrap();
        assert!(!status.healthy);
        assert!(status.text.contains("operational=unavailable"));
    }

    #[test]
    fn incidents_render_without_requiring_a_state() {
        let reply = QueryReply {
            incidents: vec![RuntimeIncident {
                incident_id: "incident-1".into(),
                severity: IncidentSeverity::Error as i32,
                reason_code: 7,
                confidence_percent: 75,
                affected_component: "host.network".into(),
                suggested_action: "controlled_stop".into(),
                cycle_first: 8,
                cycle_last: 9,
                event_count: 2,
                ..Default::default()
            }],
            ..Default::default()
        };
        let rendered = render(View::IncidentList, &reply).unwrap();
        assert!(!rendered.healthy);
        assert!(rendered.text.contains("severity=error"));
        assert!(rendered.text.contains("action=controlled_stop"));
    }

    #[test]
    fn status_rejects_unknown_incident_severity() {
        let mut reply = healthy_reply();
        reply.incidents.push(RuntimeIncident {
            severity: 99,
            ..Default::default()
        });
        assert_eq!(
            render(View::Status, &reply),
            Err(RenderError::UnknownIncidentSeverity(99))
        );
    }
}
