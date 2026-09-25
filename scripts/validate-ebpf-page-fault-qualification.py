#!/usr/bin/env python3
"""Validate privileged controlled page-fault qualification evidence."""

import json
import pathlib
import sys


ATTACH_PAGE_FAULT = 1 << 3
BOOT_ID = 0x4553_4F50_5046_4155
AGENT_EPOCH = 1
CYCLE_SEQ = 42
TRANSITION_SEQ = 9
PAGE_COUNT = 16
PAGE_FAULT_WINDOW_NS = 1_000_000_000
X86_USER_WRITE_NOT_PRESENT = 6
DEGRADED_INCIDENT_FAULT = 0x4542_2001
U64_MAX = (1 << 64) - 1
REQUIRED_KEYS = {
    "schema_version",
    "status",
    "architecture",
    "tracked_child_pid",
    "target_cpu",
    "ready_cpu",
    "result_cpu",
    "page_size",
    "page_count",
    "guarded_mapping_bytes",
    "minor_faults_before",
    "minor_faults_after",
    "minor_faults_delta",
    "page_fault_threshold",
    "page_fault_window_ns",
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
    "baseline_emitted_events",
    "baseline_lost_events",
    "baseline_page_faults",
    "baseline_page_fault_threshold_events",
    "records_seen",
    "incidents_emitted",
    "malformed_records",
    "evidence_rejected",
    "newly_reported_lost_events",
    "emitted_events",
    "lost_events",
    "page_faults",
    "page_fault_threshold_events",
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
INTEGER_KEYS = REQUIRED_KEYS - {"status", "architecture"}


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
    if report["architecture"] != "x86_64":
        fail("architecture must be x86_64")

    tracked_pid = report["tracked_child_pid"]
    if tracked_pid == 0 or tracked_pid > 0x7FFF_FFFF:
        fail("tracked_child_pid must be a positive signed-32-bit Linux task ID")
    target_cpu = report["target_cpu"]
    if target_cpu >= 0xFFFF:
        fail("target_cpu must fit in u16 and not use the sentinel")
    for key in ("ready_cpu", "result_cpu", "cpu", "evidence_cpu"):
        require_value(report, key, target_cpu)

    page_size = report["page_size"]
    if page_size < 4096 or page_size > (1 << 30) or page_size & (page_size - 1):
        fail("page_size must be a bounded power of two of at least 4096")
    require_value(report, "page_count", PAGE_COUNT)
    require_value(report, "guarded_mapping_bytes", page_size * 2)
    require_value(report, "page_fault_threshold", PAGE_COUNT)
    require_value(report, "page_fault_window_ns", PAGE_FAULT_WINDOW_NS)

    if report["minor_faults_after"] < report["minor_faults_before"]:
        fail("minor-fault counters must be monotonic")
    if (
        report["minor_faults_after"] - report["minor_faults_before"]
        != report["minor_faults_delta"]
    ):
        fail("minor_faults_delta must match the child counter difference")
    require_value(report, "minor_faults_delta", PAGE_COUNT)

    for key in (
        "runtime_attach_mask",
        "required_attach_mask",
        "initial_observation_attach_mask",
        "observation_attach_mask",
    ):
        require_value(report, key, ATTACH_PAGE_FAULT)

    for key in (
        "initial_observation_lost_event_count",
        "initial_observation_incident_count",
        "initial_observation_fault_code",
        "baseline_emitted_events",
        "baseline_lost_events",
        "baseline_page_faults",
        "baseline_page_fault_threshold_events",
        "malformed_records",
        "evidence_rejected",
        "newly_reported_lost_events",
        "lost_events",
        "dropped_incidents",
        "irq",
        "netdev_ifindex",
        "incident_lost_events",
        "evidence_irq",
        "evidence_netdev_ifindex",
        "observation_lost_event_count",
    ):
        require_value(report, key, 0)

    for key in (
        "records_seen",
        "incidents_emitted",
        "emitted_events",
        "page_fault_threshold_events",
        "incident_evidence_count",
        "observation_incident_count",
    ):
        require_value(report, key, 1)
    require_value(report, "page_faults", PAGE_COUNT)

    expected_values = {
        "initial_observation_boot_id": BOOT_ID,
        "initial_observation_agent_epoch": AGENT_EPOCH,
        "initial_observation_state": 0,
        "initial_observation_heartbeat_seq": 1,
        "incident_boot_id": BOOT_ID,
        "incident_agent_epoch": AGENT_EPOCH,
        "incident_code": 3,
        "incident_severity": 1,
        "recommended_action": 1,
        "confidence_percent": 65,
        "cycle_first": CYCLE_SEQ,
        "cycle_last": CYCLE_SEQ,
        "transition_seq": TRANSITION_SEQ,
        "incident_observed_value": PAGE_COUNT,
        "incident_threshold": PAGE_COUNT,
        "incident_event_count": PAGE_COUNT,
        "evidence_boot_id": BOOT_ID,
        "evidence_agent_epoch": AGENT_EPOCH,
        "evidence_domain": 3,
        "evidence_kind": 3,
        "evidence_severity": 1,
        "evidence_cycle_seq": CYCLE_SEQ,
        "evidence_transition_seq": TRANSITION_SEQ,
        "evidence_observed_value": PAGE_COUNT,
        "evidence_threshold": PAGE_COUNT,
        "evidence_event_count": PAGE_COUNT,
        "evidence_detail": X86_USER_WRITE_NOT_PRESENT,
        "observation_boot_id": BOOT_ID,
        "observation_agent_epoch": AGENT_EPOCH,
        "observation_state": 1,
        "observation_fault_code": DEGRADED_INCIDENT_FAULT,
        "observation_heartbeat_seq": 2,
    }
    for key, expected in expected_values.items():
        require_value(report, key, expected)

    for key in ("pid", "tid", "evidence_pid", "evidence_tid"):
        require_value(report, key, tracked_pid)
    for key in (
        "incident_id",
        "incident_first_seen_ns",
        "incident_last_seen_ns",
        "evidence_id",
        "evidence_timestamp_ns",
    ):
        if report[key] == 0:
            fail(f"{key} must be nonzero")
    if report["incident_first_seen_ns"] != report["evidence_timestamp_ns"]:
        fail("incident_first_seen_ns must equal evidence_timestamp_ns")
    if report["incident_last_seen_ns"] != report["evidence_timestamp_ns"]:
        fail("incident_last_seen_ns must equal evidence_timestamp_ns")
    if report["evidence_id"] != report["evidence_timestamp_ns"]:
        fail("evidence_id must equal evidence_timestamp_ns")
    duration = report["evidence_duration_ns"]
    if duration == 0 or duration >= PAGE_FAULT_WINDOW_NS:
        fail("evidence_duration_ns must be positive and inside the count window")


def main() -> int:
    if len(sys.argv) != 2:
        print(
            "usage: validate-ebpf-page-fault-qualification.py <report.json>",
            file=sys.stderr,
        )
        return 2
    path = pathlib.Path(sys.argv[1])
    try:
        with path.open(encoding="utf-8") as report_file:
            validate_report(json.load(report_file))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"page-fault eBPF qualification invalid: {error}", file=sys.stderr)
        return 1
    print(f"page-fault eBPF qualification valid: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
