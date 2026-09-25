#!/usr/bin/env python3
"""Validate privileged virtual-veth network-drop qualification evidence."""

import json
import pathlib
import re
import sys


ATTACH_NETWORK_DROP = 1 << 5
BOOT_ID = 0x4553_4F50_4E44_5250
AGENT_EPOCH = 1
CYCLE_SEQ = 42
TRANSITION_SEQ = 9
ETHERCAT_PROTOCOL = 0x88A4
CONTROL_PROTOCOL = ETHERCAT_PROTOCOL + 1
FRAME_BYTES = 64
NETWORK_DROP_THRESHOLD = 4
NETWORK_DROP_WINDOW_NS = 1_000_000_000
DEGRADED_INCIDENT_FAULT = 0x4542_2001
U64_MAX = (1 << 64) - 1
INTERFACE_PATTERN = re.compile(r"^eso(?:tx|rx)[0-9a-f]+$")
MAC_PATTERN = re.compile(r"^(?:[0-9a-f]{2}:){5}[0-9a-f]{2}$")
REQUIRED_KEYS = {
    "schema_version",
    "status",
    "tx_interface",
    "rx_interface",
    "tx_ifindex",
    "rx_ifindex",
    "tx_mac",
    "rx_mac",
    "tx_mtu",
    "rx_mtu",
    "tx_link_up",
    "rx_link_up",
    "tx_ipv6_sysctl_present",
    "rx_ipv6_sysctl_present",
    "tx_ipv6_disabled",
    "rx_ipv6_disabled",
    "target_cpu",
    "packet_socket_protocol",
    "frame_bytes",
    "network_protocol",
    "control_protocol",
    "wrong_protocol_frames",
    "wrong_protocol_bytes",
    "reverse_control_frames",
    "reverse_control_bytes",
    "formal_frames",
    "formal_bytes",
    "tx_drops_before_controls",
    "tx_drops_after_controls",
    "control_tx_drop_delta",
    "rx_drops_before_controls",
    "rx_drops_after_controls",
    "control_rx_drop_delta",
    "baseline_tx_drops",
    "final_tx_drops",
    "formal_tx_drop_delta",
    "baseline_rx_drops",
    "final_rx_drops",
    "formal_rx_drop_delta",
    "network_drop_threshold",
    "network_drop_window_ns",
    "runtime_attach_mask",
    "required_attach_mask",
    "initial_observation_boot_id",
    "initial_observation_agent_epoch",
    "initial_observation_state",
    "initial_observation_attach_mask",
    "initial_observation_lost_event_count",
    "initial_observation_incident_count",
    "initial_observation_fault_code",
    "initial_observation_heartbeat_seq",
    "control_records_seen",
    "control_incidents_emitted",
    "control_malformed_records",
    "control_evidence_rejected",
    "control_newly_reported_lost_events",
    "control_emitted_events",
    "control_lost_events",
    "control_network_drops",
    "control_network_unattributed",
    "control_network_threshold_events",
    "baseline_emitted_events",
    "baseline_lost_events",
    "baseline_network_drops",
    "baseline_network_unattributed",
    "baseline_network_threshold_events",
    "records_seen",
    "incidents_emitted",
    "malformed_records",
    "evidence_rejected",
    "newly_reported_lost_events",
    "emitted_events",
    "lost_events",
    "network_drops",
    "network_unattributed",
    "network_threshold_events",
    "dropped_incidents",
    "incident_id",
    "incident_boot_id",
    "incident_agent_epoch",
    "incident_code",
    "incident_severity",
    "recommended_action",
    "confidence_percent",
    "incident_first_seen_ns",
    "incident_last_seen_ns",
    "pid",
    "tid",
    "cpu",
    "irq",
    "netdev_ifindex",
    "cycle_first",
    "cycle_last",
    "transition_seq",
    "incident_observed_value",
    "incident_threshold",
    "incident_evidence_count",
    "incident_event_count",
    "incident_lost_events",
    "evidence_id",
    "evidence_boot_id",
    "evidence_agent_epoch",
    "evidence_timestamp_ns",
    "evidence_domain",
    "evidence_kind",
    "evidence_severity",
    "evidence_pid",
    "evidence_tid",
    "evidence_cpu",
    "evidence_irq",
    "evidence_netdev_ifindex",
    "evidence_cycle_seq",
    "evidence_transition_seq",
    "evidence_observed_value",
    "evidence_threshold",
    "evidence_duration_ns",
    "evidence_event_count",
    "evidence_detail",
    "observation_boot_id",
    "observation_agent_epoch",
    "observation_state",
    "observation_attach_mask",
    "observation_lost_event_count",
    "observation_incident_count",
    "observation_fault_code",
    "observation_heartbeat_seq",
    "runtime_detached",
    "packet_socket_closed",
    "links_removed",
    "affinity_restored",
    "cleanup_succeeded",
}
STRING_KEYS = {"status", "tx_interface", "rx_interface", "tx_mac", "rx_mac"}
INTEGER_KEYS = REQUIRED_KEYS - STRING_KEYS


def fail(message: str) -> None:
    raise ValueError(message)


def require_integer(report: dict, key: str) -> int:
    value = report[key]
    if type(value) is not int:
        fail(f"{key} must be an integer")
    if value < 0 or value > U64_MAX:
        fail(f"{key} must be an unsigned 64-bit integer")
    return value


def require_value(report: dict, key: str, expected: int) -> None:
    if report[key] != expected:
        fail(f"{key} must be {expected}")


def require_counter_delta(
    report: dict, baseline_key: str, final_key: str, delta_key: str
) -> int:
    baseline = report[baseline_key]
    final = report[final_key]
    if final < baseline:
        fail(f"{final_key} must not regress")
    delta = final - baseline
    if report[delta_key] != delta:
        fail(f"{delta_key} must match the counter difference")
    return delta


def validate_mac(value: object, key: str) -> tuple[int, ...]:
    if not isinstance(value, str) or not MAC_PATTERN.fullmatch(value):
        fail(f"{key} must be a canonical lowercase MAC address")
    octets = tuple(int(part, 16) for part in value.split(":"))
    if octets == (0,) * 6 or octets == (0xFF,) * 6 or octets[0] & 1:
        fail(f"{key} must be a nonzero unicast MAC address")
    return octets


def validate_report(report: object) -> None:
    if not isinstance(report, dict):
        fail("report must be an object")
    keys = set(report)
    missing = REQUIRED_KEYS - keys
    unknown = keys - REQUIRED_KEYS
    if missing:
        fail(f"missing keys: {sorted(missing)}")
    if unknown:
        fail(f"unknown keys: {sorted(unknown)}")
    for key in INTEGER_KEYS:
        require_integer(report, key)

    require_value(report, "schema_version", 1)
    if report["status"] != "qualified":
        fail("status must be qualified")
    for key in ("tx_interface", "rx_interface"):
        value = report[key]
        if not isinstance(value, str) or not INTERFACE_PATTERN.fullmatch(value):
            fail(f"{key} must be a generated ESOP veth name")
        if len(value) > 15:
            fail(f"{key} must fit Linux IFNAMSIZ")
    if report["tx_interface"] == report["rx_interface"]:
        fail("veth interface names must be distinct")

    tx_ifindex = report["tx_ifindex"]
    rx_ifindex = report["rx_ifindex"]
    if tx_ifindex == 0 or tx_ifindex > 0x7FFF_FFFF:
        fail("tx_ifindex must be a positive signed-32-bit ifindex")
    if rx_ifindex == 0 or rx_ifindex > 0x7FFF_FFFF:
        fail("rx_ifindex must be a positive signed-32-bit ifindex")
    if tx_ifindex == rx_ifindex:
        fail("veth ifindexes must be distinct")
    if validate_mac(report["tx_mac"], "tx_mac") == validate_mac(
        report["rx_mac"], "rx_mac"
    ):
        fail("veth MAC addresses must be distinct")

    for key in ("tx_mtu", "rx_mtu"):
        if report[key] < FRAME_BYTES or report[key] > 0xFFFF:
            fail(f"{key} must fit the bounded test frame")
    for key in ("tx_link_up", "rx_link_up"):
        require_value(report, key, 1)
    for prefix in ("tx", "rx"):
        present = report[f"{prefix}_ipv6_sysctl_present"]
        disabled = report[f"{prefix}_ipv6_disabled"]
        if present not in (0, 1) or disabled not in (0, 1):
            fail(f"{prefix} IPv6 fields must be binary")
        if disabled != present:
            fail(f"{prefix}_ipv6_disabled must match sysctl presence")

    if report["target_cpu"] > 0xFFFF:
        fail("target_cpu must fit the evidence u16")
    expected_values = {
        "packet_socket_protocol": 0,
        "frame_bytes": FRAME_BYTES,
        "network_protocol": ETHERCAT_PROTOCOL,
        "control_protocol": CONTROL_PROTOCOL,
        "wrong_protocol_frames": 1,
        "wrong_protocol_bytes": FRAME_BYTES,
        "reverse_control_frames": 1,
        "reverse_control_bytes": FRAME_BYTES,
        "formal_frames": NETWORK_DROP_THRESHOLD,
        "formal_bytes": FRAME_BYTES * NETWORK_DROP_THRESHOLD,
        "network_drop_threshold": NETWORK_DROP_THRESHOLD,
        "network_drop_window_ns": NETWORK_DROP_WINDOW_NS,
    }
    for key, expected in expected_values.items():
        require_value(report, key, expected)

    control_tx_delta = require_counter_delta(
        report,
        "tx_drops_before_controls",
        "tx_drops_after_controls",
        "control_tx_drop_delta",
    )
    control_rx_delta = require_counter_delta(
        report,
        "rx_drops_before_controls",
        "rx_drops_after_controls",
        "control_rx_drop_delta",
    )
    if control_tx_delta < 1 or control_rx_delta < 1:
        fail("both control frames must independently advance receive drops")
    require_value(report, "baseline_tx_drops", report["tx_drops_after_controls"])
    require_value(report, "baseline_rx_drops", report["rx_drops_after_controls"])
    formal_tx_delta = require_counter_delta(
        report, "baseline_tx_drops", "final_tx_drops", "formal_tx_drop_delta"
    )
    formal_rx_delta = require_counter_delta(
        report, "baseline_rx_drops", "final_rx_drops", "formal_rx_drop_delta"
    )
    if formal_tx_delta != 0:
        fail("formal_tx_drop_delta must be 0")
    if formal_rx_delta < NETWORK_DROP_THRESHOLD:
        fail("formal_rx_drop_delta must cover every formal frame")

    for key in (
        "runtime_attach_mask",
        "required_attach_mask",
        "initial_observation_attach_mask",
        "observation_attach_mask",
    ):
        require_value(report, key, ATTACH_NETWORK_DROP)

    for key in (
        "initial_observation_lost_event_count",
        "initial_observation_incident_count",
        "initial_observation_fault_code",
        "control_records_seen",
        "control_incidents_emitted",
        "control_malformed_records",
        "control_evidence_rejected",
        "control_newly_reported_lost_events",
        "control_emitted_events",
        "control_lost_events",
        "control_network_drops",
        "control_network_unattributed",
        "control_network_threshold_events",
        "baseline_emitted_events",
        "baseline_lost_events",
        "baseline_network_drops",
        "baseline_network_unattributed",
        "baseline_network_threshold_events",
        "malformed_records",
        "evidence_rejected",
        "newly_reported_lost_events",
        "lost_events",
        "network_unattributed",
        "dropped_incidents",
        "pid",
        "tid",
        "irq",
        "incident_lost_events",
        "evidence_pid",
        "evidence_tid",
        "evidence_irq",
        "observation_lost_event_count",
    ):
        require_value(report, key, 0)

    for key in (
        "records_seen",
        "incidents_emitted",
        "emitted_events",
        "network_threshold_events",
        "incident_evidence_count",
        "observation_incident_count",
    ):
        require_value(report, key, 1)
    require_value(report, "network_drops", NETWORK_DROP_THRESHOLD)

    semantic_values = {
        "initial_observation_boot_id": BOOT_ID,
        "initial_observation_agent_epoch": AGENT_EPOCH,
        "initial_observation_state": 0,
        "initial_observation_heartbeat_seq": 1,
        "incident_boot_id": BOOT_ID,
        "incident_agent_epoch": AGENT_EPOCH,
        "incident_code": 2,
        "incident_severity": 2,
        "recommended_action": 2,
        "confidence_percent": 75,
        "cpu": report["target_cpu"],
        "netdev_ifindex": rx_ifindex,
        "cycle_first": CYCLE_SEQ,
        "cycle_last": CYCLE_SEQ,
        "transition_seq": TRANSITION_SEQ,
        "incident_observed_value": NETWORK_DROP_THRESHOLD,
        "incident_threshold": NETWORK_DROP_THRESHOLD,
        "incident_event_count": NETWORK_DROP_THRESHOLD,
        "evidence_boot_id": BOOT_ID,
        "evidence_agent_epoch": AGENT_EPOCH,
        "evidence_domain": 2,
        "evidence_kind": 2,
        "evidence_severity": 2,
        "evidence_cpu": report["target_cpu"],
        "evidence_netdev_ifindex": rx_ifindex,
        "evidence_cycle_seq": CYCLE_SEQ,
        "evidence_transition_seq": TRANSITION_SEQ,
        "evidence_observed_value": NETWORK_DROP_THRESHOLD,
        "evidence_threshold": NETWORK_DROP_THRESHOLD,
        "evidence_event_count": NETWORK_DROP_THRESHOLD,
        "observation_boot_id": BOOT_ID,
        "observation_agent_epoch": AGENT_EPOCH,
        "observation_state": 1,
        "observation_fault_code": DEGRADED_INCIDENT_FAULT,
        "observation_heartbeat_seq": 2,
    }
    for key, expected in semantic_values.items():
        require_value(report, key, expected)

    for key in (
        "incident_id",
        "incident_first_seen_ns",
        "incident_last_seen_ns",
        "evidence_id",
        "evidence_timestamp_ns",
    ):
        if report[key] == 0:
            fail(f"{key} must be nonzero")
    require_value(report, "incident_first_seen_ns", report["evidence_timestamp_ns"])
    require_value(report, "incident_last_seen_ns", report["evidence_timestamp_ns"])
    require_value(report, "evidence_id", report["evidence_timestamp_ns"])
    duration = report["evidence_duration_ns"]
    if duration == 0 or duration >= NETWORK_DROP_WINDOW_NS:
        fail("evidence_duration_ns must be positive and inside the count window")
    if report["evidence_detail"] == 0 or report["evidence_detail"] > 0xFF:
        fail("evidence_detail must contain a bounded nonzero kernel drop reason")

    for key in (
        "runtime_detached",
        "packet_socket_closed",
        "links_removed",
        "affinity_restored",
        "cleanup_succeeded",
    ):
        require_value(report, key, 1)


def main() -> int:
    if len(sys.argv) != 2:
        print(
            "usage: validate-ebpf-network-drop-qualification.py <report.json>",
            file=sys.stderr,
        )
        return 2
    path = pathlib.Path(sys.argv[1])
    try:
        with path.open(encoding="utf-8") as report_file:
            validate_report(json.load(report_file))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"network-drop eBPF qualification invalid: {error}", file=sys.stderr)
        return 1
    print(f"network-drop eBPF qualification valid: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
