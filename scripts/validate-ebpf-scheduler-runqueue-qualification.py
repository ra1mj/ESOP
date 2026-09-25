#!/usr/bin/env python3
"""Validate privileged scheduler-runqueue eBPF runtime qualification evidence."""

import json
import pathlib
import sys


ATTACH_SCHEDULER_PAIR = (1 << 0) | (1 << 1)
BOOT_ID = 0x4553_4F50_5251_5545
AGENT_EPOCH = 1
CYCLE_SEQ = 42
TRANSITION_SEQ = 9
SCHED_FIFO = 1
SCHEDULER_LATENCY_THRESHOLD_NS = 5_000_000
BLOCKER_HOLD_NS = 25_000_000
MAX_QUALIFIED_LATENCY_NS = 1_000_000_000
DEGRADED_INCIDENT_FAULT = 0x4542_2001
U64_MAX = (1 << 64) - 1
REQUIRED_KEYS = {
    "schema_version",
    "status",
    "target_tid",
    "cpu_a",
    "cpu_b",
    "target_sleep_confirmed",
    "fifo_policy",
    "fifo_priority_min",
    "fifo_priority_max",
    "fifo_priority",
    "scheduler_latency_threshold_ns",
    "blocker_hold_ns",
    "futex_wake_count",
    "target_completed_during_hold",
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
    "baseline_wakeups",
    "baseline_scheduler_stalls",
    "baseline_scheduler_migrations",
    "records_seen",
    "incidents_emitted",
    "malformed_records",
    "evidence_rejected",
    "newly_reported_lost_events",
    "emitted_events",
    "lost_events",
    "wakeups",
    "scheduler_stalls",
    "scheduler_migrations",
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
    require_value(
        report,
        "scheduler_latency_threshold_ns",
        SCHEDULER_LATENCY_THRESHOLD_NS,
    )
    require_value(report, "blocker_hold_ns", BLOCKER_HOLD_NS)
    for key in (
        "runtime_attach_mask",
        "required_attach_mask",
        "initial_observation_attach_mask",
        "observation_attach_mask",
    ):
        require_value(report, key, ATTACH_SCHEDULER_PAIR)

    target_tid = report["target_tid"]
    if target_tid == 0 or target_tid > 0x7FFF_FFFF:
        fail("target_tid must be a positive signed-32-bit Linux TID")
    cpu_a = report["cpu_a"]
    cpu_b = report["cpu_b"]
    if cpu_a > 0xFFFF or cpu_b > 0xFFFF:
        fail("qualification CPUs must fit in u16")
    if cpu_a == cpu_b:
        fail("cpu_a and cpu_b must be distinct")
    require_value(report, "target_sleep_confirmed", 1)
    require_value(report, "fifo_policy", SCHED_FIFO)
    priority_min = report["fifo_priority_min"]
    priority_max = report["fifo_priority_max"]
    priority = report["fifo_priority"]
    if priority_min == 0 or priority_min > priority_max:
        fail("FIFO priority range must be positive and ordered")
    if priority != priority_min or priority > priority_max or priority > 0x7FFF_FFFF:
        fail("fifo_priority must equal the valid minimum SCHED_FIFO priority")
    require_value(report, "futex_wake_count", 1)
    require_value(report, "target_completed_during_hold", 0)

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
        "baseline_wakeups",
        "baseline_scheduler_stalls",
        "baseline_scheduler_migrations",
    ):
        require_value(report, key, 0)
    for key in (
        "records_seen",
        "incidents_emitted",
        "emitted_events",
        "wakeups",
        "scheduler_stalls",
        "incident_evidence_count",
        "incident_event_count",
        "evidence_event_count",
        "observation_incident_count",
    ):
        require_value(report, key, 1)
    for key in (
        "malformed_records",
        "evidence_rejected",
        "newly_reported_lost_events",
        "lost_events",
        "scheduler_migrations",
        "dropped_incidents",
        "pid",
        "irq",
        "netdev_ifindex",
        "incident_lost_events",
        "evidence_pid",
        "evidence_irq",
        "evidence_netdev_ifindex",
        "evidence_detail",
        "observation_lost_event_count",
    ):
        require_value(report, key, 0)

    expected_values = {
        "incident_boot_id": BOOT_ID,
        "incident_agent_epoch": AGENT_EPOCH,
        "incident_code": 0,
        "incident_severity": 2,
        "recommended_action": 2,
        "confidence_percent": 70,
        "cycle_first": CYCLE_SEQ,
        "cycle_last": CYCLE_SEQ,
        "transition_seq": TRANSITION_SEQ,
        "incident_threshold": SCHEDULER_LATENCY_THRESHOLD_NS,
        "evidence_boot_id": BOOT_ID,
        "evidence_agent_epoch": AGENT_EPOCH,
        "evidence_domain": 0,
        "evidence_kind": 0,
        "evidence_severity": 2,
        "evidence_cycle_seq": CYCLE_SEQ,
        "evidence_transition_seq": TRANSITION_SEQ,
        "evidence_threshold": SCHEDULER_LATENCY_THRESHOLD_NS,
        "observation_boot_id": BOOT_ID,
        "observation_agent_epoch": AGENT_EPOCH,
        "observation_state": 1,
        "observation_fault_code": DEGRADED_INCIDENT_FAULT,
        "observation_heartbeat_seq": 2,
    }
    for key, expected in expected_values.items():
        require_value(report, key, expected)
    for key in ("tid", "evidence_tid"):
        require_value(report, key, target_tid)
    for key in ("cpu", "evidence_cpu"):
        require_value(report, key, cpu_a)

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
    if report["evidence_id"] >= evidence_timestamp:
        fail("evidence_id must precede evidence_timestamp_ns")
    for key in ("incident_first_seen_ns", "incident_last_seen_ns"):
        if report[key] != evidence_timestamp:
            fail(f"{key} must equal evidence_timestamp_ns")

    duration = report["evidence_duration_ns"]
    if duration < BLOCKER_HOLD_NS or duration >= MAX_QUALIFIED_LATENCY_NS:
        fail("evidence_duration_ns must cover the blocker hold and stay below the maximum")
    if duration <= SCHEDULER_LATENCY_THRESHOLD_NS:
        fail("evidence_duration_ns must be strictly above the latency threshold")
    for key in ("incident_observed_value", "evidence_observed_value"):
        if report[key] != duration:
            fail(f"{key} must equal evidence_duration_ns")


def main() -> int:
    if len(sys.argv) != 2:
        print(
            "usage: validate-ebpf-scheduler-runqueue-qualification.py <report.json>",
            file=sys.stderr,
        )
        return 2
    path = pathlib.Path(sys.argv[1])
    try:
        with path.open(encoding="utf-8") as report_file:
            validate_report(json.load(report_file))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"scheduler runqueue eBPF qualification invalid: {error}", file=sys.stderr)
        return 1
    print(f"scheduler runqueue eBPF qualification valid: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
