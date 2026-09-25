#!/usr/bin/env python3
"""Validate privileged scheduler-migration eBPF runtime qualification evidence."""

import json
import pathlib
import sys


ATTACH_SCHED_MIGRATE_TASK = 1 << 11
BOOT_ID = 0x4553_4F50_4D49_4752
AGENT_EPOCH = 1
CYCLE_SEQ = 42
TRANSITION_SEQ = 9
MIGRATION_THRESHOLD = 2
MIGRATION_WINDOW_NS = 10_000_000_000
DEGRADED_INCIDENT_FAULT = 0x4542_2001
U64_MAX = (1 << 64) - 1
REQUIRED_KEYS = {
    "schema_version",
    "status",
    "worker_tid",
    "cpu_a",
    "cpu_b",
    "migration_threshold",
    "migration_window_ns",
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
    "baseline_scheduler_migrations",
    "baseline_threshold_events",
    "first_records_seen",
    "first_incidents_emitted",
    "first_malformed_records",
    "first_evidence_rejected",
    "first_newly_reported_lost_events",
    "first_emitted_events",
    "first_lost_events",
    "first_scheduler_migrations",
    "first_threshold_events",
    "records_seen",
    "incidents_emitted",
    "malformed_records",
    "evidence_rejected",
    "newly_reported_lost_events",
    "emitted_events",
    "lost_events",
    "scheduler_migrations",
    "scheduler_migration_threshold_events",
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
    require_value(report, "migration_threshold", MIGRATION_THRESHOLD)
    require_value(report, "migration_window_ns", MIGRATION_WINDOW_NS)
    for key in (
        "runtime_attach_mask",
        "required_attach_mask",
        "initial_observation_attach_mask",
        "observation_attach_mask",
    ):
        require_value(report, key, ATTACH_SCHED_MIGRATE_TASK)

    worker_tid = report["worker_tid"]
    if worker_tid == 0 or worker_tid > 0x7FFF_FFFF:
        fail("worker_tid must be a positive signed-32-bit Linux TID")
    cpu_a = report["cpu_a"]
    cpu_b = report["cpu_b"]
    if cpu_a > 0xFFFF or cpu_b > 0xFFFF:
        fail("qualification CPUs must fit in u16")
    if cpu_a == cpu_b:
        fail("cpu_a and cpu_b must be distinct")

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

    for key in (
        "baseline_emitted_events",
        "baseline_lost_events",
        "baseline_scheduler_migrations",
        "baseline_threshold_events",
        "first_records_seen",
        "first_incidents_emitted",
        "first_malformed_records",
        "first_evidence_rejected",
        "first_newly_reported_lost_events",
        "first_emitted_events",
        "first_lost_events",
        "first_threshold_events",
    ):
        require_value(report, key, 0)
    require_value(report, "first_scheduler_migrations", 1)

    for key in (
        "records_seen",
        "incidents_emitted",
        "emitted_events",
        "scheduler_migration_threshold_events",
        "incident_evidence_count",
        "observation_incident_count",
    ):
        require_value(report, key, 1)
    for key in (
        "malformed_records",
        "evidence_rejected",
        "newly_reported_lost_events",
        "lost_events",
        "dropped_incidents",
        "pid",
        "netdev_ifindex",
        "incident_lost_events",
        "evidence_pid",
        "evidence_netdev_ifindex",
        "observation_lost_event_count",
    ):
        require_value(report, key, 0)
    for key in (
        "scheduler_migrations",
        "incident_observed_value",
        "incident_threshold",
        "incident_event_count",
        "evidence_observed_value",
        "evidence_threshold",
        "evidence_event_count",
    ):
        require_value(report, key, MIGRATION_THRESHOLD)

    expected_values = {
        "incident_boot_id": BOOT_ID,
        "incident_agent_epoch": AGENT_EPOCH,
        "incident_code": 0,
        "incident_severity": 1,
        "recommended_action": 1,
        "confidence_percent": 60,
        "cycle_first": CYCLE_SEQ,
        "cycle_last": CYCLE_SEQ,
        "transition_seq": TRANSITION_SEQ,
        "evidence_boot_id": BOOT_ID,
        "evidence_agent_epoch": AGENT_EPOCH,
        "evidence_domain": 0,
        "evidence_kind": 10,
        "evidence_severity": 1,
        "evidence_cycle_seq": CYCLE_SEQ,
        "evidence_transition_seq": TRANSITION_SEQ,
        "observation_boot_id": BOOT_ID,
        "observation_agent_epoch": AGENT_EPOCH,
        "observation_state": 1,
        "observation_fault_code": DEGRADED_INCIDENT_FAULT,
        "observation_heartbeat_seq": 2,
    }
    for key, expected in expected_values.items():
        require_value(report, key, expected)

    for key in (
        "tid",
        "evidence_tid",
    ):
        require_value(report, key, worker_tid)
    for key in ("cpu", "evidence_cpu"):
        require_value(report, key, cpu_a)
    for key in ("irq", "evidence_irq"):
        require_value(report, key, cpu_b)

    for key in (
        "incident_id",
        "incident_first_seen_ns",
        "incident_last_seen_ns",
        "evidence_id",
        "evidence_timestamp_ns",
    ):
        if report[key] == 0:
            fail(f"{key} must be nonzero")
    evidence_timestamp = report["evidence_timestamp_ns"]
    for key in ("incident_first_seen_ns", "incident_last_seen_ns"):
        if report[key] != evidence_timestamp:
            fail(f"{key} must equal evidence_timestamp_ns")

    duration = report["evidence_duration_ns"]
    if duration == 0 or duration >= MIGRATION_WINDOW_NS:
        fail("evidence_duration_ns must be positive and below migration_window_ns")
    if report["evidence_detail"] > 0xFF:
        fail("evidence_detail must fit in u8")


def main() -> int:
    if len(sys.argv) != 2:
        print(
            "usage: validate-ebpf-scheduler-migration-qualification.py <report.json>",
            file=sys.stderr,
        )
        return 2
    path = pathlib.Path(sys.argv[1])
    try:
        with path.open(encoding="utf-8") as report_file:
            validate_report(json.load(report_file))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"scheduler migration eBPF qualification invalid: {error}", file=sys.stderr)
        return 1
    print(f"scheduler migration eBPF qualification valid: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
