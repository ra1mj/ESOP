#!/usr/bin/env python3
"""Validate privileged controlled-loopback softirq qualification evidence."""

import json
import pathlib
import sys


ATTACH_SOFTIRQ = (1 << 8) | (1 << 9)
BOOT_ID = 0x4553_4F50_534F_4654
AGENT_EPOCH = 1
CALIBRATION_CYCLE_SEQ = 41
FORMAL_CYCLE_SEQ = 42
TRANSITION_SEQ = 9
NET_RX_VECTOR = 3
UDP_SEGMENT_BYTES = 1_200
UDP_SEGMENT_COUNT = 54
UDP_PAYLOAD_BYTES = UDP_SEGMENT_BYTES * UDP_SEGMENT_COUNT
CALIBRATION_DIVISOR = 8
CALIBRATION_THRESHOLD_NS = 1
DEGRADED_INCIDENT_FAULT = 0x4542_2001
U64_MAX = (1 << 64) - 1
REQUIRED_KEYS = {
    "schema_version",
    "status",
    "fixture_pid",
    "fixture_tid",
    "target_cpu",
    "quiet_net_rx_before",
    "quiet_net_rx_after",
    "quiet_net_rx_delta",
    "softirq_vector",
    "udp_segment_bytes",
    "udp_segment_count",
    "udp_payload_bytes",
    "calibration_divisor",
    "calibration_threshold_ns",
    "calibration_duration_ns",
    "calibration_sent_bytes",
    "calibration_received_datagrams",
    "calibration_net_rx_before",
    "calibration_net_rx_after",
    "calibration_net_rx_delta",
    "calibration_runtime_attach_mask",
    "calibration_required_attach_mask",
    "calibration_baseline_emitted_events",
    "calibration_baseline_lost_events",
    "calibration_baseline_irq_samples",
    "calibration_baseline_irq_overruns",
    "calibration_baseline_softirq_samples",
    "calibration_baseline_softirq_overruns",
    "calibration_records_seen",
    "calibration_incidents_emitted",
    "calibration_malformed_records",
    "calibration_evidence_rejected",
    "calibration_newly_reported_lost_events",
    "calibration_emitted_events",
    "calibration_lost_events",
    "calibration_irq_samples",
    "calibration_irq_overruns",
    "calibration_softirq_samples",
    "calibration_softirq_overruns",
    "calibration_dropped_incidents",
    "formal_threshold_ns",
    "formal_sent_bytes",
    "formal_received_datagrams",
    "formal_net_rx_before",
    "formal_net_rx_after",
    "formal_net_rx_delta",
    "runtime_attach_mask",
    "required_attach_mask",
    "interrupt_filter_cpu",
    "interrupt_filter_vector",
    "initial_observation_boot_id",
    "initial_observation_agent_epoch",
    "initial_observation_state",
    "initial_observation_attach_mask",
    "initial_observation_lost_event_count",
    "initial_observation_incident_count",
    "initial_observation_fault_code",
    "initial_observation_heartbeat_seq",
    "baseline_emitted_events",
    "baseline_lost_events",
    "baseline_irq_samples",
    "baseline_irq_overruns",
    "baseline_softirq_samples",
    "baseline_softirq_overruns",
    "records_seen",
    "incidents_emitted",
    "malformed_records",
    "evidence_rejected",
    "newly_reported_lost_events",
    "emitted_events",
    "lost_events",
    "irq_samples",
    "irq_overruns",
    "softirq_samples",
    "softirq_overruns",
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
}
INTEGER_KEYS = REQUIRED_KEYS - {"status"}


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


def require_counter_delta(report: dict, prefix: str, expected: int | None = None) -> None:
    before = report[f"{prefix}_before"]
    after = report[f"{prefix}_after"]
    delta = report[f"{prefix}_delta"]
    if after < before or after - before != delta:
        fail(f"{prefix} counters must be monotonic and match their delta")
    if expected is not None and delta != expected:
        fail(f"{prefix}_delta must be {expected}")


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

    for key in ("fixture_pid", "fixture_tid"):
        if report[key] == 0 or report[key] > 0x7FFF_FFFF:
            fail(f"{key} must be a positive signed-32-bit Linux task ID")
    target_cpu = report["target_cpu"]
    if target_cpu >= 0xFFFF:
        fail("target_cpu must fit in u16 and not use the all-CPU sentinel")

    require_counter_delta(report, "quiet_net_rx")
    require_counter_delta(report, "calibration_net_rx", 1)
    require_counter_delta(report, "formal_net_rx", 1)
    constants = {
        "softirq_vector": NET_RX_VECTOR,
        "udp_segment_bytes": UDP_SEGMENT_BYTES,
        "udp_segment_count": UDP_SEGMENT_COUNT,
        "udp_payload_bytes": UDP_PAYLOAD_BYTES,
        "calibration_divisor": CALIBRATION_DIVISOR,
        "calibration_threshold_ns": CALIBRATION_THRESHOLD_NS,
        "calibration_sent_bytes": UDP_PAYLOAD_BYTES,
        "calibration_received_datagrams": UDP_SEGMENT_COUNT,
        "formal_sent_bytes": UDP_PAYLOAD_BYTES,
        "formal_received_datagrams": UDP_SEGMENT_COUNT,
        "interrupt_filter_cpu": target_cpu,
        "interrupt_filter_vector": NET_RX_VECTOR,
    }
    for key, expected in constants.items():
        require_value(report, key, expected)

    calibration_duration = report["calibration_duration_ns"]
    if calibration_duration <= CALIBRATION_THRESHOLD_NS:
        fail("calibration_duration_ns must be strictly above its threshold")
    formal_threshold = max(1, calibration_duration // CALIBRATION_DIVISOR)
    require_value(report, "formal_threshold_ns", formal_threshold)

    for key in (
        "calibration_runtime_attach_mask",
        "calibration_required_attach_mask",
        "runtime_attach_mask",
        "required_attach_mask",
        "initial_observation_attach_mask",
        "observation_attach_mask",
    ):
        require_value(report, key, ATTACH_SOFTIRQ)

    for prefix in ("calibration_baseline", "baseline"):
        for suffix in (
            "emitted_events",
            "lost_events",
            "irq_samples",
            "irq_overruns",
            "softirq_samples",
            "softirq_overruns",
        ):
            require_value(report, f"{prefix}_{suffix}", 0)

    for prefix in ("calibration_", ""):
        for suffix in (
            "records_seen",
            "incidents_emitted",
            "emitted_events",
            "softirq_samples",
            "softirq_overruns",
        ):
            require_value(report, f"{prefix}{suffix}", 1)
        for suffix in (
            "malformed_records",
            "evidence_rejected",
            "newly_reported_lost_events",
            "lost_events",
            "irq_samples",
            "irq_overruns",
            "dropped_incidents",
        ):
            require_value(report, f"{prefix}{suffix}", 0)

    initial_values = {
        "initial_observation_boot_id": BOOT_ID,
        "initial_observation_agent_epoch": AGENT_EPOCH,
        "initial_observation_state": 0,
        "initial_observation_lost_event_count": 0,
        "initial_observation_incident_count": 0,
        "initial_observation_fault_code": 0,
        "initial_observation_heartbeat_seq": 1,
    }
    for key, expected in initial_values.items():
        require_value(report, key, expected)

    expected_values = {
        "incident_boot_id": BOOT_ID,
        "incident_agent_epoch": AGENT_EPOCH,
        "incident_code": 1,
        "incident_severity": 2,
        "recommended_action": 2,
        "confidence_percent": 70,
        "irq": NET_RX_VECTOR,
        "netdev_ifindex": 0,
        "cycle_first": FORMAL_CYCLE_SEQ,
        "cycle_last": FORMAL_CYCLE_SEQ,
        "transition_seq": TRANSITION_SEQ,
        "incident_threshold": formal_threshold,
        "incident_evidence_count": 1,
        "incident_event_count": 1,
        "incident_lost_events": 0,
        "evidence_boot_id": BOOT_ID,
        "evidence_agent_epoch": AGENT_EPOCH,
        "evidence_domain": 1,
        "evidence_kind": 9,
        "evidence_severity": 2,
        "evidence_irq": NET_RX_VECTOR,
        "evidence_netdev_ifindex": 0,
        "evidence_cycle_seq": FORMAL_CYCLE_SEQ,
        "evidence_transition_seq": TRANSITION_SEQ,
        "evidence_threshold": formal_threshold,
        "evidence_event_count": 1,
        "evidence_detail": 0,
        "observation_boot_id": BOOT_ID,
        "observation_agent_epoch": AGENT_EPOCH,
        "observation_state": 1,
        "observation_lost_event_count": 0,
        "observation_incident_count": 1,
        "observation_fault_code": DEGRADED_INCIDENT_FAULT,
        "observation_heartbeat_seq": 2,
    }
    for key, expected in expected_values.items():
        require_value(report, key, expected)

    for key in ("pid", "evidence_pid"):
        require_value(report, key, report["fixture_pid"])
    for key in ("tid", "evidence_tid"):
        require_value(report, key, report["fixture_tid"])
    for key in ("cpu", "evidence_cpu"):
        require_value(report, key, target_cpu)

    for key in (
        "incident_id",
        "incident_first_seen_ns",
        "incident_last_seen_ns",
        "evidence_id",
        "evidence_timestamp_ns",
    ):
        if report[key] == 0:
            fail(f"{key} must be nonzero")
    timestamp = report["evidence_timestamp_ns"]
    if report["evidence_id"] != timestamp:
        fail("evidence_id must equal evidence_timestamp_ns for a fallback producer ID")
    for key in ("incident_first_seen_ns", "incident_last_seen_ns"):
        if report[key] != timestamp:
            fail(f"{key} must equal evidence_timestamp_ns")

    duration = report["evidence_duration_ns"]
    if duration <= formal_threshold:
        fail("evidence_duration_ns must be strictly above formal_threshold_ns")
    for key in (
        "incident_observed_value",
        "evidence_observed_value",
    ):
        if report[key] != duration:
            fail(f"{key} must equal evidence_duration_ns")


def main() -> int:
    if len(sys.argv) != 2:
        print(
            "usage: validate-ebpf-softirq-qualification.py <report.json>",
            file=sys.stderr,
        )
        return 2
    path = pathlib.Path(sys.argv[1])
    try:
        with path.open(encoding="utf-8") as report_file:
            validate_report(json.load(report_file))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"softirq eBPF qualification invalid: {error}", file=sys.stderr)
        return 1
    print(f"softirq eBPF qualification valid: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
