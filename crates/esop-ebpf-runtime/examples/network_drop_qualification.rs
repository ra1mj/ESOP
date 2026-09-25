#![cfg(target_os = "linux")]

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use esop_ebpf_agent::{
    AgentState, CycleContext, EvidenceDomain, EvidenceKind, IncidentCode, IncidentSeverity,
    RecommendedAction, RuntimeAgent,
};
use esop_ebpf_runtime::{
    ATTACH_NETWORK_DROP, BpfRuntime, KernelStats, NETWORK_PROTOCOL_ETHERCAT, PollReport,
    RuntimeConfig,
};

const BOOT_ID: u64 = 0x4553_4f50_4e44_5250;
const AGENT_EPOCH: u64 = 1;
const CYCLE_SEQ: u64 = 42;
const TRANSITION_SEQ: u64 = 9;
const INCIDENT_WINDOW_NS: u64 = 1_000_000_000;
const NETWORK_DROP_THRESHOLD: u64 = 4;
const NETWORK_DROP_WINDOW_NS: u64 = 1_000_000_000;
const CONTROL_PROTOCOL: u16 = NETWORK_PROTOCOL_ETHERCAT + 1;
const FRAME_BYTES: usize = 64;
const CONTROL_FRAMES: u64 = 1;
const FORMAL_FRAMES: u64 = NETWORK_DROP_THRESHOLD;
const POLL_SLEEP: Duration = Duration::from_millis(2);
const POLL_DEADLINE: Duration = Duration::from_secs(2);
const COUNTER_DEADLINE: Duration = Duration::from_secs(2);
const CLEANUP_DEADLINE: Duration = Duration::from_secs(2);
const CONTROL_QUIET: Duration = Duration::from_millis(50);
const FINAL_QUIET: Duration = Duration::from_millis(50);
const OBSERVATION_HEALTHY: u8 = 0;
const OBSERVATION_DEGRADED: u8 = 1;
const DEGRADED_INCIDENT_FAULT: u32 = 0x4542_2001;

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn report_temp_path(output: &Path) -> PathBuf {
    let mut path = output.as_os_str().to_os_string();
    path.push(".tmp");
    PathBuf::from(path)
}

fn add_poll_report(total: &mut PollReport, current: PollReport) {
    total.records_seen = total.records_seen.saturating_add(current.records_seen);
    total.incidents_emitted = total
        .incidents_emitted
        .saturating_add(current.incidents_emitted);
    total.malformed_records = total
        .malformed_records
        .saturating_add(current.malformed_records);
    total.evidence_rejected = total
        .evidence_rejected
        .saturating_add(current.evidence_rejected);
    total.newly_reported_lost_events = total
        .newly_reported_lost_events
        .saturating_add(current.newly_reported_lost_events);
}

fn read_trimmed(path: impl AsRef<Path>) -> io::Result<String> {
    Ok(fs::read_to_string(path)?.trim().to_owned())
}

fn read_u64(path: impl AsRef<Path>, label: &str) -> io::Result<u64> {
    read_trimmed(path)?
        .parse::<u64>()
        .map_err(|_| invalid_data(format!("{label} was not an unsigned integer")))
}

fn read_u32(path: impl AsRef<Path>, label: &str) -> io::Result<u32> {
    let value = read_u64(path, label)?;
    u32::try_from(value).map_err(|_| invalid_data(format!("{label} did not fit u32")))
}

fn read_hex_u64(path: impl AsRef<Path>, label: &str) -> io::Result<u64> {
    let text = read_trimmed(path)?;
    u64::from_str_radix(text.trim_start_matches("0x"), 16)
        .map_err(|_| invalid_data(format!("{label} was not hexadecimal")))
}

fn network_path(interface: &str, suffix: &str) -> PathBuf {
    Path::new("/sys/class/net").join(interface).join(suffix)
}

fn receive_drops(interface: &str) -> io::Result<u64> {
    read_u64(
        network_path(interface, "statistics/rx_dropped"),
        "receive-drop counter",
    )
}

fn interface_is_up(interface: &str) -> io::Result<bool> {
    let flags = read_hex_u64(network_path(interface, "flags"), "interface flags")?;
    Ok(flags & libc::IFF_UP as u64 != 0)
}

fn parse_mac(text: &str) -> io::Result<[u8; 6]> {
    let mut mac = [0u8; 6];
    let mut parts = text.split(':');
    for octet in &mut mac {
        let part = parts
            .next()
            .ok_or_else(|| invalid_data("interface MAC address was too short"))?;
        if part.len() != 2 {
            return Err(invalid_data("interface MAC address octet was malformed"));
        }
        *octet = u8::from_str_radix(part, 16)
            .map_err(|_| invalid_data("interface MAC address was malformed"))?;
    }
    if parts.next().is_some() || mac == [0; 6] || mac == [0xff; 6] || mac[0] & 1 != 0 {
        return Err(invalid_data(
            "interface MAC address was not a valid unicast address",
        ));
    }
    Ok(mac)
}

fn format_mac(mac: [u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

fn run_ip(arguments: &[&str]) -> io::Result<()> {
    let output = Command::new("ip").args(arguments).output()?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(invalid_data(format!(
        "ip {} failed with {}: {}",
        arguments.join(" "),
        output.status,
        stderr.trim()
    )))
}

fn verify_ip_available() -> io::Result<()> {
    run_ip(&["-Version"])
}

fn set_interface_ipv6_disabled(interface: &str) -> io::Result<(bool, bool)> {
    let path = Path::new("/proc/sys/net/ipv6/conf")
        .join(interface)
        .join("disable_ipv6");
    if !path.exists() {
        return Ok((false, false));
    }
    fs::write(&path, b"1\n")?;
    let disabled = read_u64(&path, "per-interface IPv6 policy")? == 1;
    if !disabled {
        return Err(invalid_data("per-interface IPv6 disable did not persist"));
    }
    Ok((true, true))
}

#[derive(Debug)]
struct LinkFixture {
    tx_name: String,
    rx_name: String,
    tx_ifindex: u32,
    rx_ifindex: u32,
    tx_mac: [u8; 6],
    rx_mac: [u8; 6],
    tx_mtu: u32,
    rx_mtu: u32,
    tx_ipv6_present: bool,
    rx_ipv6_present: bool,
    tx_ipv6_disabled: bool,
    rx_ipv6_disabled: bool,
    created: bool,
}

impl LinkFixture {
    fn create() -> Result<Self, Box<dyn Error>> {
        if unsafe { libc::geteuid() } != 0 {
            return Err(invalid_data("network-drop qualification requires root").into());
        }
        verify_ip_available()?;
        let pid = std::process::id();
        let tx_name = format!("esotx{pid:x}");
        let rx_name = format!("esorx{pid:x}");
        if tx_name.len() > libc::IFNAMSIZ - 1 || rx_name.len() > libc::IFNAMSIZ - 1 {
            return Err(invalid_data("generated veth name exceeded IFNAMSIZ").into());
        }
        if network_path(&tx_name, "").exists() || network_path(&rx_name, "").exists() {
            return Err(invalid_data("generated veth name already existed").into());
        }

        run_ip(&[
            "link", "add", &tx_name, "type", "veth", "peer", "name", &rx_name,
        ])?;
        let mut fixture = Self {
            tx_name,
            rx_name,
            tx_ifindex: 0,
            rx_ifindex: 0,
            tx_mac: [0; 6],
            rx_mac: [0; 6],
            tx_mtu: 0,
            rx_mtu: 0,
            tx_ipv6_present: false,
            rx_ipv6_present: false,
            tx_ipv6_disabled: false,
            rx_ipv6_disabled: false,
            created: true,
        };

        let (tx_ipv6_present, tx_ipv6_disabled) = set_interface_ipv6_disabled(&fixture.tx_name)?;
        let (rx_ipv6_present, rx_ipv6_disabled) = set_interface_ipv6_disabled(&fixture.rx_name)?;
        fixture.tx_ipv6_present = tx_ipv6_present;
        fixture.rx_ipv6_present = rx_ipv6_present;
        fixture.tx_ipv6_disabled = tx_ipv6_disabled;
        fixture.rx_ipv6_disabled = rx_ipv6_disabled;

        run_ip(&["link", "set", "dev", &fixture.tx_name, "up"])?;
        run_ip(&["link", "set", "dev", &fixture.rx_name, "up"])?;
        if !interface_is_up(&fixture.tx_name)? || !interface_is_up(&fixture.rx_name)? {
            return Err(invalid_data("veth pair did not enter the UP state").into());
        }

        fixture.tx_ifindex = read_u32(
            network_path(&fixture.tx_name, "ifindex"),
            "transmit ifindex",
        )?;
        fixture.rx_ifindex =
            read_u32(network_path(&fixture.rx_name, "ifindex"), "receive ifindex")?;
        if fixture.tx_ifindex == 0
            || fixture.rx_ifindex == 0
            || fixture.tx_ifindex == fixture.rx_ifindex
        {
            return Err(invalid_data("veth ifindexes were invalid").into());
        }
        fixture.tx_mac = parse_mac(&read_trimmed(network_path(&fixture.tx_name, "address"))?)?;
        fixture.rx_mac = parse_mac(&read_trimmed(network_path(&fixture.rx_name, "address"))?)?;
        if fixture.tx_mac == fixture.rx_mac {
            return Err(invalid_data("veth MAC addresses were not distinct").into());
        }
        fixture.tx_mtu = read_u32(network_path(&fixture.tx_name, "mtu"), "transmit MTU")?;
        fixture.rx_mtu = read_u32(network_path(&fixture.rx_name, "mtu"), "receive MTU")?;
        if fixture.tx_mtu < FRAME_BYTES as u32 || fixture.rx_mtu < FRAME_BYTES as u32 {
            return Err(invalid_data("veth MTU was smaller than the test frame").into());
        }
        Ok(fixture)
    }

    fn remove(&mut self) -> io::Result<()> {
        if !self.created {
            return Ok(());
        }
        run_ip(&["link", "delete", "dev", &self.tx_name])?;
        let started = Instant::now();
        while network_path(&self.tx_name, "").exists() || network_path(&self.rx_name, "").exists() {
            if started.elapsed() >= CLEANUP_DEADLINE {
                return Err(invalid_data("veth pair did not disappear after deletion"));
            }
            thread::sleep(POLL_SLEEP);
        }
        self.created = false;
        Ok(())
    }
}

impl Drop for LinkFixture {
    fn drop(&mut self) {
        if self.created {
            let _ = run_ip(&["link", "delete", "dev", &self.tx_name]);
        }
    }
}

struct AffinityGuard {
    original: libc::cpu_set_t,
    restored: bool,
}

impl AffinityGuard {
    fn pin_first_allowed() -> io::Result<(Self, u16)> {
        let mut original: libc::cpu_set_t = unsafe { zeroed() };
        if unsafe { libc::sched_getaffinity(0, size_of::<libc::cpu_set_t>(), &mut original) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut selected = None;
        for cpu in 0..libc::CPU_SETSIZE as usize {
            if cpu <= usize::from(u16::MAX) && unsafe { libc::CPU_ISSET(cpu, &original) } {
                selected = Some(cpu as u16);
                break;
            }
        }
        let cpu = selected.ok_or_else(|| invalid_data("no allowed CPU fits the evidence ABI"))?;
        let mut pinned: libc::cpu_set_t = unsafe { zeroed() };
        unsafe {
            libc::CPU_ZERO(&mut pinned);
            libc::CPU_SET(usize::from(cpu), &mut pinned);
        }
        if unsafe { libc::sched_setaffinity(0, size_of::<libc::cpu_set_t>(), &pinned) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let guard = Self {
            original,
            restored: false,
        };
        let current = unsafe { libc::sched_getcpu() };
        if current < 0 {
            return Err(io::Error::last_os_error());
        }
        if current as u16 != cpu {
            return Err(invalid_data("fixture did not execute on the selected CPU"));
        }
        Ok((guard, cpu))
    }

    fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }
        if unsafe { libc::sched_setaffinity(0, size_of::<libc::cpu_set_t>(), &self.original) } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut actual: libc::cpu_set_t = unsafe { zeroed() };
        if unsafe { libc::sched_getaffinity(0, size_of::<libc::cpu_set_t>(), &mut actual) } != 0 {
            return Err(io::Error::last_os_error());
        }
        for cpu in 0..libc::CPU_SETSIZE as usize {
            if unsafe { libc::CPU_ISSET(cpu, &actual) }
                != unsafe { libc::CPU_ISSET(cpu, &self.original) }
            {
                return Err(invalid_data("fixture CPU affinity was not restored"));
            }
        }
        self.restored = true;
        Ok(())
    }
}

impl Drop for AffinityGuard {
    fn drop(&mut self) {
        if !self.restored {
            let _ =
                unsafe { libc::sched_setaffinity(0, size_of::<libc::cpu_set_t>(), &self.original) };
        }
    }
}

struct PacketSocket {
    fd: RawFd,
}

impl PacketSocket {
    fn open() -> io::Result<Self> {
        let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW | libc::SOCK_CLOEXEC, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd })
    }

    fn send_frame(
        &self,
        ifindex: u32,
        source: [u8; 6],
        destination: [u8; 6],
        protocol: u16,
        sequence: u8,
    ) -> io::Result<usize> {
        let mut frame = [0u8; FRAME_BYTES];
        frame[0..6].copy_from_slice(&destination);
        frame[6..12].copy_from_slice(&source);
        frame[12..14].copy_from_slice(&protocol.to_be_bytes());
        for (offset, byte) in frame[14..].iter_mut().enumerate() {
            *byte = sequence.wrapping_add(offset as u8);
        }

        let mut address: libc::sockaddr_ll = unsafe { zeroed() };
        address.sll_family = libc::AF_PACKET as u16;
        address.sll_protocol = protocol.to_be();
        address.sll_ifindex = i32::try_from(ifindex)
            .map_err(|_| invalid_data("packet ifindex did not fit signed int"))?;
        address.sll_halen = 6;
        address.sll_addr[..6].copy_from_slice(&destination);
        let sent = unsafe {
            libc::sendto(
                self.fd,
                frame.as_ptr().cast(),
                frame.len(),
                0,
                (&raw const address).cast(),
                size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        };
        if sent < 0 {
            return Err(io::Error::last_os_error());
        }
        let sent = usize::try_from(sent)
            .map_err(|_| invalid_data("packet send length did not fit usize"))?;
        if sent != frame.len() {
            return Err(invalid_data(format!(
                "packet send was partial: expected {}, sent {sent}",
                frame.len()
            )));
        }
        Ok(sent)
    }

    fn close(&mut self) -> io::Result<()> {
        if self.fd < 0 {
            return Ok(());
        }
        let fd = self.fd;
        self.fd = -1;
        if unsafe { libc::close(fd) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for PacketSocket {
    fn drop(&mut self) {
        if self.fd >= 0 {
            let _ = unsafe { libc::close(self.fd) };
            self.fd = -1;
        }
    }
}

fn wait_counter_delta(interface: &str, baseline: u64, minimum: u64) -> io::Result<u64> {
    let started = Instant::now();
    loop {
        let current = receive_drops(interface)?;
        let delta = current
            .checked_sub(baseline)
            .ok_or_else(|| invalid_data("receive-drop counter moved backwards"))?;
        if delta >= minimum {
            return Ok(current);
        }
        if started.elapsed() >= COUNTER_DEADLINE {
            return Err(invalid_data(format!(
                "{interface} receive-drop counter did not advance by {minimum}"
            )));
        }
        thread::sleep(POLL_SLEEP);
    }
}

fn require_empty_poll(report: PollReport, label: &str) -> io::Result<()> {
    if report != PollReport::default() {
        return Err(invalid_data(format!(
            "{label} unexpectedly produced poll activity: {report:?}"
        )));
    }
    Ok(())
}

fn json_number(name: &'static str, value: impl ToString) -> (&'static str, String) {
    (name, value.to_string())
}

fn json_string(name: &'static str, value: &str) -> (&'static str, String) {
    let mut encoded = String::with_capacity(value.len() + 2);
    encoded.push('"');
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            character if character.is_control() => {
                let _ = write!(&mut encoded, "\\u{:04x}", character as u32);
            }
            character => encoded.push(character),
        }
    }
    encoded.push('"');
    (name, encoded)
}

fn write_report(output: &Path, fields: Vec<(&'static str, String)>) -> io::Result<()> {
    let temp = report_temp_path(output);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut report = String::from("{\n");
    let last = fields.len().saturating_sub(1);
    for (index, (name, value)) in fields.into_iter().enumerate() {
        let comma = if index == last { "" } else { "," };
        writeln!(&mut report, "  \"{name}\": {value}{comma}")
            .map_err(|_| invalid_data("failed to format qualification report"))?;
    }
    report.push_str("}\n");
    fs::write(&temp, report)?;
    fs::rename(temp, output)?;
    Ok(())
}

fn run(object_path: PathBuf, output_path: PathBuf) -> Result<(), Box<dyn Error>> {
    let temp_path = report_temp_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&temp_path);

    let (mut affinity, target_cpu) = AffinityGuard::pin_first_allowed()?;
    let mut links = LinkFixture::create()?;
    let tx_mac_text = format_mac(links.tx_mac);
    let rx_mac_text = format_mac(links.rx_mac);
    let mut socket = PacketSocket::open()?;

    let config = RuntimeConfig {
        enabled_attach_mask: ATTACH_NETWORK_DROP,
        required_attach_mask: ATTACH_NETWORK_DROP,
        network_ifindex: links.rx_ifindex,
        network_protocol: NETWORK_PROTOCOL_ETHERCAT,
        network_drop_threshold: NETWORK_DROP_THRESHOLD,
        network_drop_window_ns: NETWORK_DROP_WINDOW_NS,
        boot_id: BOOT_ID,
        agent_epoch: AGENT_EPOCH,
        ..RuntimeConfig::default()
    };
    let mut runtime = BpfRuntime::load(&object_path, config)?;
    let snapshot = runtime.capability_snapshot();
    let context = runtime.kernel_context();
    if runtime.attach_mask() != ATTACH_NETWORK_DROP
        || snapshot.attach_mask != ATTACH_NETWORK_DROP
        || snapshot.required_attach_mask != ATTACH_NETWORK_DROP
        || !snapshot.attach_ready()
        || context.network_ifindex != links.rx_ifindex
        || context.network_protocol != NETWORK_PROTOCOL_ETHERCAT
        || context.network_drop_threshold != NETWORK_DROP_THRESHOLD
        || context.network_drop_window_ns != NETWORK_DROP_WINDOW_NS
    {
        return Err(invalid_data("network-drop tracepoint capability was incomplete").into());
    }

    let mut agent = RuntimeAgent::<8>::new(BOOT_ID, AGENT_EPOCH, INCIDENT_WINDOW_NS);
    runtime.apply_capability_snapshot(&mut agent);
    if agent.health().state() != AgentState::Healthy {
        return Err(invalid_data("network-drop observer did not become healthy").into());
    }
    let initial_observation = agent.heartbeat(1);
    if initial_observation.state as u8 != OBSERVATION_HEALTHY
        || initial_observation.attach_mask != ATTACH_NETWORK_DROP
        || initial_observation.lost_event_count != 0
        || initial_observation.incident_count != 0
        || initial_observation.fault_code != 0
        || initial_observation.heartbeat_seq != 1
    {
        return Err(invalid_data("initial network-drop observation was inconsistent").into());
    }

    let initial_stats = runtime.statistics()?;
    if initial_stats != KernelStats::default() {
        return Err(invalid_data(format!(
            "network-drop initial statistics were not empty: {initial_stats:?}"
        ))
        .into());
    }

    let tx_drops_before_controls = receive_drops(&links.tx_name)?;
    let rx_drops_before_controls = receive_drops(&links.rx_name)?;
    let wrong_protocol_bytes = socket.send_frame(
        links.tx_ifindex,
        links.tx_mac,
        links.rx_mac,
        CONTROL_PROTOCOL,
        0x10,
    )?;
    let rx_drops_after_controls =
        wait_counter_delta(&links.rx_name, rx_drops_before_controls, CONTROL_FRAMES)?;
    let reverse_control_bytes = socket.send_frame(
        links.rx_ifindex,
        links.rx_mac,
        links.tx_mac,
        NETWORK_PROTOCOL_ETHERCAT,
        0x20,
    )?;
    let tx_drops_after_controls =
        wait_counter_delta(&links.tx_name, tx_drops_before_controls, CONTROL_FRAMES)?;
    thread::sleep(CONTROL_QUIET);
    let control_poll = runtime.poll(&mut agent, 64)?;
    require_empty_poll(control_poll, "network-drop controls")?;
    let control_stats = runtime.statistics()?;
    if control_stats != KernelStats::default()
        || !agent.correlator().is_empty()
        || agent.correlator().dropped_incidents() != 0
        || agent.health().state() != AgentState::Healthy
    {
        return Err(invalid_data(format!(
            "network-drop controls were not silent: {control_stats:?}"
        ))
        .into());
    }

    let baseline_tx_drops = tx_drops_after_controls;
    let baseline_rx_drops = rx_drops_after_controls;
    let baseline = runtime.statistics()?;
    if baseline != KernelStats::default() {
        return Err(invalid_data("network-drop formal baseline was not empty").into());
    }

    let cycle = CycleContext {
        boot_id: BOOT_ID,
        cycle_seq: CYCLE_SEQ,
        transition_seq: TRANSITION_SEQ,
        wkc_bad: 1,
        expected_wkc: NETWORK_DROP_THRESHOLD as u16,
        actual_wkc: 0,
        ..CycleContext::EMPTY
    };
    runtime.update_cycle_context(cycle)?;
    agent
        .observe_cycle(cycle)
        .map_err(|error| invalid_data(format!("agent rejected cycle context: {error:?}")))?;

    let mut formal_bytes = 0usize;
    for sequence in 0..FORMAL_FRAMES {
        formal_bytes = formal_bytes.saturating_add(socket.send_frame(
            links.tx_ifindex,
            links.tx_mac,
            links.rx_mac,
            NETWORK_PROTOCOL_ETHERCAT,
            0x40u8.wrapping_add(sequence as u8),
        )?);
    }
    let final_rx_drops = wait_counter_delta(&links.rx_name, baseline_rx_drops, FORMAL_FRAMES)?;
    let final_tx_drops = receive_drops(&links.tx_name)?;
    if final_tx_drops != baseline_tx_drops {
        return Err(invalid_data("formal forward injection changed TX receive drops").into());
    }

    let mut poll = PollReport::default();
    let poll_started = Instant::now();
    while poll_started.elapsed() < POLL_DEADLINE {
        add_poll_report(&mut poll, runtime.poll(&mut agent, 64)?);
        if poll.records_seen >= 1 && poll.incidents_emitted >= 1 {
            break;
        }
        thread::sleep(POLL_SLEEP);
    }
    thread::sleep(FINAL_QUIET);
    add_poll_report(&mut poll, runtime.poll(&mut agent, 64)?);
    if poll.records_seen != 1 || poll.incidents_emitted != 1 {
        return Err(invalid_data(format!(
            "network-drop injection produced records/incidents {}/{}",
            poll.records_seen, poll.incidents_emitted
        ))
        .into());
    }
    if poll.malformed_records != 0
        || poll.evidence_rejected != 0
        || poll.newly_reported_lost_events != 0
        || agent.correlator().len() != 1
        || agent.correlator().dropped_incidents() != 0
    {
        return Err(invalid_data("network-drop poll or retention was inconsistent").into());
    }

    let dropped_incidents = agent.correlator().dropped_incidents();
    let incident = agent
        .pop_incident()
        .ok_or_else(|| invalid_data("network-drop incident was missing"))?;
    if agent.pop_incident().is_some() {
        return Err(invalid_data("unexpected extra network-drop incident").into());
    }
    if incident.incident_id == 0
        || incident.boot_id != BOOT_ID
        || incident.agent_epoch != AGENT_EPOCH
        || incident.code != IncidentCode::HostNicDrop
        || incident.severity != IncidentSeverity::Error
        || incident.recommended_action != RecommendedAction::ControlledStop
        || incident.confidence_percent != 75
        || incident.pid != 0
        || incident.tid != 0
        || incident.cpu != target_cpu
        || incident.irq != 0
        || incident.netdev_ifindex != links.rx_ifindex
        || incident.cycle_first != CYCLE_SEQ
        || incident.cycle_last != CYCLE_SEQ
        || incident.transition_seq != TRANSITION_SEQ
        || incident.observed_value != NETWORK_DROP_THRESHOLD
        || incident.threshold != NETWORK_DROP_THRESHOLD
        || incident.evidence_count != 1
        || incident.count != NETWORK_DROP_THRESHOLD as u32
        || incident.lost_events != 0
    {
        return Err(invalid_data(format!(
            "network-drop incident fields were inconsistent: {incident:?}"
        ))
        .into());
    }
    let evidence = incident.evidence[0];
    if evidence.evidence_id == 0
        || evidence.evidence_id != evidence.timestamp_ns
        || evidence.boot_id != BOOT_ID
        || evidence.agent_epoch != AGENT_EPOCH
        || evidence.pid != 0
        || evidence.tid != 0
        || evidence.cpu != target_cpu
        || evidence.irq != 0
        || evidence.netdev_ifindex != links.rx_ifindex
        || evidence.cycle_seq != CYCLE_SEQ
        || evidence.transition_seq != TRANSITION_SEQ
        || evidence.observed_value != NETWORK_DROP_THRESHOLD
        || evidence.threshold != NETWORK_DROP_THRESHOLD
        || evidence.duration_ns == 0
        || evidence.duration_ns >= NETWORK_DROP_WINDOW_NS
        || evidence.count != NETWORK_DROP_THRESHOLD as u32
        || evidence.domain != EvidenceDomain::KernelNetwork
        || evidence.kind != EvidenceKind::NetworkDrop
        || evidence.severity != IncidentSeverity::Error
        || evidence.detail == 0
        || incident.first_seen_ns != evidence.timestamp_ns
        || incident.last_seen_ns != evidence.timestamp_ns
    {
        return Err(invalid_data(format!(
            "network-drop evidence fields were inconsistent: {evidence:?}"
        ))
        .into());
    }

    let statistics = runtime.statistics()?;
    let expected_statistics = KernelStats {
        emitted_events: 1,
        network_drops: NETWORK_DROP_THRESHOLD,
        network_threshold_events: 1,
        ..KernelStats::default()
    };
    if statistics != expected_statistics {
        return Err(invalid_data(format!(
            "network-drop statistics were inconsistent: {statistics:?}"
        ))
        .into());
    }
    if agent.health().state() != AgentState::Degraded {
        return Err(invalid_data("network-drop incident did not degrade observer health").into());
    }
    let observation = agent.heartbeat(evidence.timestamp_ns.saturating_add(1));
    if observation.boot_id != BOOT_ID
        || observation.agent_epoch != AGENT_EPOCH
        || observation.state as u8 != OBSERVATION_DEGRADED
        || observation.attach_mask != ATTACH_NETWORK_DROP
        || observation.lost_event_count != 0
        || observation.incident_count != 1
        || observation.fault_code != DEGRADED_INCIDENT_FAULT
        || observation.heartbeat_seq != 2
    {
        return Err(invalid_data("final network-drop observation was inconsistent").into());
    }

    let runtime_attach_mask = runtime.attach_mask();
    let required_attach_mask = snapshot.required_attach_mask;
    drop(runtime);
    socket.close()?;
    links.remove()?;
    affinity.restore()?;

    let control_tx_drop_delta = tx_drops_after_controls - tx_drops_before_controls;
    let control_rx_drop_delta = rx_drops_after_controls - rx_drops_before_controls;
    let formal_tx_drop_delta = final_tx_drops - baseline_tx_drops;
    let formal_rx_drop_delta = final_rx_drops - baseline_rx_drops;
    let fields = vec![
        json_number("schema_version", 1),
        json_string("status", "qualified"),
        json_string("tx_interface", &links.tx_name),
        json_string("rx_interface", &links.rx_name),
        json_number("tx_ifindex", links.tx_ifindex),
        json_number("rx_ifindex", links.rx_ifindex),
        json_string("tx_mac", &tx_mac_text),
        json_string("rx_mac", &rx_mac_text),
        json_number("tx_mtu", links.tx_mtu),
        json_number("rx_mtu", links.rx_mtu),
        json_number("tx_link_up", 1),
        json_number("rx_link_up", 1),
        json_number("tx_ipv6_sysctl_present", links.tx_ipv6_present as u8),
        json_number("rx_ipv6_sysctl_present", links.rx_ipv6_present as u8),
        json_number("tx_ipv6_disabled", links.tx_ipv6_disabled as u8),
        json_number("rx_ipv6_disabled", links.rx_ipv6_disabled as u8),
        json_number("target_cpu", target_cpu),
        json_number("packet_socket_protocol", 0),
        json_number("frame_bytes", FRAME_BYTES),
        json_number("network_protocol", NETWORK_PROTOCOL_ETHERCAT),
        json_number("control_protocol", CONTROL_PROTOCOL),
        json_number("wrong_protocol_frames", CONTROL_FRAMES),
        json_number("wrong_protocol_bytes", wrong_protocol_bytes),
        json_number("reverse_control_frames", CONTROL_FRAMES),
        json_number("reverse_control_bytes", reverse_control_bytes),
        json_number("formal_frames", FORMAL_FRAMES),
        json_number("formal_bytes", formal_bytes),
        json_number("tx_drops_before_controls", tx_drops_before_controls),
        json_number("tx_drops_after_controls", tx_drops_after_controls),
        json_number("control_tx_drop_delta", control_tx_drop_delta),
        json_number("rx_drops_before_controls", rx_drops_before_controls),
        json_number("rx_drops_after_controls", rx_drops_after_controls),
        json_number("control_rx_drop_delta", control_rx_drop_delta),
        json_number("baseline_tx_drops", baseline_tx_drops),
        json_number("final_tx_drops", final_tx_drops),
        json_number("formal_tx_drop_delta", formal_tx_drop_delta),
        json_number("baseline_rx_drops", baseline_rx_drops),
        json_number("final_rx_drops", final_rx_drops),
        json_number("formal_rx_drop_delta", formal_rx_drop_delta),
        json_number("network_drop_threshold", NETWORK_DROP_THRESHOLD),
        json_number("network_drop_window_ns", NETWORK_DROP_WINDOW_NS),
        json_number("runtime_attach_mask", runtime_attach_mask),
        json_number("required_attach_mask", required_attach_mask),
        json_number("initial_observation_boot_id", initial_observation.boot_id),
        json_number(
            "initial_observation_agent_epoch",
            initial_observation.agent_epoch,
        ),
        json_number("initial_observation_state", initial_observation.state as u8),
        json_number(
            "initial_observation_attach_mask",
            initial_observation.attach_mask,
        ),
        json_number(
            "initial_observation_lost_event_count",
            initial_observation.lost_event_count,
        ),
        json_number(
            "initial_observation_incident_count",
            initial_observation.incident_count,
        ),
        json_number(
            "initial_observation_fault_code",
            initial_observation.fault_code,
        ),
        json_number(
            "initial_observation_heartbeat_seq",
            initial_observation.heartbeat_seq,
        ),
        json_number("control_records_seen", control_poll.records_seen),
        json_number("control_incidents_emitted", control_poll.incidents_emitted),
        json_number("control_malformed_records", control_poll.malformed_records),
        json_number("control_evidence_rejected", control_poll.evidence_rejected),
        json_number(
            "control_newly_reported_lost_events",
            control_poll.newly_reported_lost_events,
        ),
        json_number("control_emitted_events", control_stats.emitted_events),
        json_number("control_lost_events", control_stats.lost_events),
        json_number("control_network_drops", control_stats.network_drops),
        json_number(
            "control_network_unattributed",
            control_stats.network_unattributed,
        ),
        json_number(
            "control_network_threshold_events",
            control_stats.network_threshold_events,
        ),
        json_number("baseline_emitted_events", baseline.emitted_events),
        json_number("baseline_lost_events", baseline.lost_events),
        json_number("baseline_network_drops", baseline.network_drops),
        json_number(
            "baseline_network_unattributed",
            baseline.network_unattributed,
        ),
        json_number(
            "baseline_network_threshold_events",
            baseline.network_threshold_events,
        ),
        json_number("records_seen", poll.records_seen),
        json_number("incidents_emitted", poll.incidents_emitted),
        json_number("malformed_records", poll.malformed_records),
        json_number("evidence_rejected", poll.evidence_rejected),
        json_number(
            "newly_reported_lost_events",
            poll.newly_reported_lost_events,
        ),
        json_number("emitted_events", statistics.emitted_events),
        json_number("lost_events", statistics.lost_events),
        json_number("network_drops", statistics.network_drops),
        json_number("network_unattributed", statistics.network_unattributed),
        json_number(
            "network_threshold_events",
            statistics.network_threshold_events,
        ),
        json_number("dropped_incidents", dropped_incidents),
        json_number("incident_id", incident.incident_id),
        json_number("incident_boot_id", incident.boot_id),
        json_number("incident_agent_epoch", incident.agent_epoch),
        json_number("incident_code", incident.code as u8),
        json_number("incident_severity", incident.severity as u8),
        json_number("recommended_action", incident.recommended_action as u8),
        json_number("confidence_percent", incident.confidence_percent),
        json_number("incident_first_seen_ns", incident.first_seen_ns),
        json_number("incident_last_seen_ns", incident.last_seen_ns),
        json_number("pid", incident.pid),
        json_number("tid", incident.tid),
        json_number("cpu", incident.cpu),
        json_number("irq", incident.irq),
        json_number("netdev_ifindex", incident.netdev_ifindex),
        json_number("cycle_first", incident.cycle_first),
        json_number("cycle_last", incident.cycle_last),
        json_number("transition_seq", incident.transition_seq),
        json_number("incident_observed_value", incident.observed_value),
        json_number("incident_threshold", incident.threshold),
        json_number("incident_evidence_count", incident.evidence_count),
        json_number("incident_event_count", incident.count),
        json_number("incident_lost_events", incident.lost_events),
        json_number("evidence_id", evidence.evidence_id),
        json_number("evidence_boot_id", evidence.boot_id),
        json_number("evidence_agent_epoch", evidence.agent_epoch),
        json_number("evidence_timestamp_ns", evidence.timestamp_ns),
        json_number("evidence_domain", evidence.domain as u8),
        json_number("evidence_kind", evidence.kind as u8),
        json_number("evidence_severity", evidence.severity as u8),
        json_number("evidence_pid", evidence.pid),
        json_number("evidence_tid", evidence.tid),
        json_number("evidence_cpu", evidence.cpu),
        json_number("evidence_irq", evidence.irq),
        json_number("evidence_netdev_ifindex", evidence.netdev_ifindex),
        json_number("evidence_cycle_seq", evidence.cycle_seq),
        json_number("evidence_transition_seq", evidence.transition_seq),
        json_number("evidence_observed_value", evidence.observed_value),
        json_number("evidence_threshold", evidence.threshold),
        json_number("evidence_duration_ns", evidence.duration_ns),
        json_number("evidence_event_count", evidence.count),
        json_number("evidence_detail", evidence.detail),
        json_number("observation_boot_id", observation.boot_id),
        json_number("observation_agent_epoch", observation.agent_epoch),
        json_number("observation_state", observation.state as u8),
        json_number("observation_attach_mask", observation.attach_mask),
        json_number("observation_lost_event_count", observation.lost_event_count),
        json_number("observation_incident_count", observation.incident_count),
        json_number("observation_fault_code", observation.fault_code),
        json_number("observation_heartbeat_seq", observation.heartbeat_seq),
        json_number("runtime_detached", 1),
        json_number("packet_socket_closed", 1),
        json_number("links_removed", 1),
        json_number("affinity_restored", 1),
        json_number("cleanup_succeeded", 1),
    ];
    write_report(&output_path, fields)?;
    println!(
        "network-drop runtime qualification passed: {}",
        output_path.display()
    );
    Ok(())
}

fn main() {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let result = (|| -> Result<(), Box<dyn Error>> {
        let object_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
            invalid_data(
                "usage: network_drop_qualification <esop_runtime.bpf.o> <qualification.json>",
            )
        })?;
        let output_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
            invalid_data(
                "usage: network_drop_qualification <esop_runtime.bpf.o> <qualification.json>",
            )
        })?;
        if arguments.next().is_some() {
            return Err(invalid_data("unexpected extra qualification argument").into());
        }
        run(object_path, output_path)
    })();
    if let Err(error) = result {
        eprintln!("network-drop runtime qualification failed: {error}");
        std::process::exit(1);
    }
}
