#!/usr/bin/env python3
"""Validate privileged process-exit eBPF runtime qualification evidence."""

import json
import pathlib
import sys


ATTACH_PROCESS_EXIT = 1 << 2
BOOT_ID = 0x4553_4F50_5058_5155
AGENT_EPOCH = 1
LATCHED_INCIDENT_FAULT = 0x4542_2002
REQUIRED_KEYS = {
    "schema_version",
    "status",
    "tracked_child_pid",
    "runtime_attach_mask",
    "required_attach_mask",
    "worker_records_seen",
    "worker_incidents_emitted",
    "worker_malformed_records",
    "worker_evidence_rejected",
    "worker_newly_reported_lost_events",
    "worker_thread_exits_ignored",
    "worker_observation_state",
    "worker_observation_attach_mask",
    "worker_observation_lost_event_count",
    "worker_observation_incident_count",
    "worker_observation_fault_code",
    "worker_observation_heartbeat_seq",
    "records_seen",
    "incidents_emitted",
    "malformed_records",
    "evidence_rejected",
    "newly_reported_lost_events",
    "emitted_events",
    "lost_events",
    "process_exits",
    "thread_exits_ignored",
    "oom_events",
    "dropped_incidents",
    "incident_id",
    "incident_boot_id",
    "incident_agent_epoch",
    "incident_code",
    "incident_severity",
    "recommended_action",
    "confidence_percent",
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
U64_MAX = (1 << 64) - 1


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
    if report["tracked_child_pid"] == 0:
        fail("tracked_child_pid must be nonzero")
    for key in (
        "runtime_attach_mask",
        "required_attach_mask",
        "worker_observation_attach_mask",
        "observation_attach_mask",
    ):
        require_value(report, key, ATTACH_PROCESS_EXIT)

    for key in (
        "worker_records_seen",
        "worker_incidents_emitted",
        "worker_malformed_records",
        "worker_evidence_rejected",
        "worker_newly_reported_lost_events",
        "worker_observation_state",
        "worker_observation_lost_event_count",
        "worker_observation_incident_count",
        "worker_observation_fault_code",
    ):
        require_value(report, key, 0)
    require_value(report, "worker_thread_exits_ignored", 1)
    require_value(report, "worker_observation_heartbeat_seq", 1)

    for key in (
        "records_seen",
        "incidents_emitted",
        "emitted_events",
        "process_exits",
        "thread_exits_ignored",
        "incident_observed_value",
        "incident_threshold",
        "incident_evidence_count",
        "incident_event_count",
        "evidence_observed_value",
        "evidence_threshold",
        "evidence_event_count",
        "observation_incident_count",
    ):
        require_value(report, key, 1)
    for key in (
        "malformed_records",
        "evidence_rejected",
        "newly_reported_lost_events",
        "lost_events",
        "oom_events",
        "dropped_incidents",
        "irq",
        "netdev_ifindex",
        "cycle_first",
        "cycle_last",
        "transition_seq",
        "incident_lost_events",
        "evidence_irq",
        "evidence_netdev_ifindex",
        "evidence_cycle_seq",
        "evidence_transition_seq",
        "evidence_duration_ns",
        "evidence_detail",
        "observation_lost_event_count",
    ):
        require_value(report, key, 0)

    expected_values = {
        "incident_boot_id": BOOT_ID,
        "incident_agent_epoch": AGENT_EPOCH,
        "incident_code": 5,
        "incident_severity": 3,
        "recommended_action": 3,
        "confidence_percent": 100,
        "evidence_boot_id": BOOT_ID,
        "evidence_agent_epoch": AGENT_EPOCH,
        "evidence_domain": 4,
        "evidence_kind": 5,
        "evidence_severity": 3,
        "observation_boot_id": BOOT_ID,
        "observation_agent_epoch": AGENT_EPOCH,
        "observation_state": 2,
        "observation_fault_code": LATCHED_INCIDENT_FAULT,
        "observation_heartbeat_seq": 2,
    }
    for key, expected in expected_values.items():
        require_value(report, key, expected)

    for key in ("incident_id", "evidence_id", "evidence_timestamp_ns"):
        if report[key] == 0:
            fail(f"{key} must be nonzero")
    for key in ("tracked_child_pid", "pid", "tid", "evidence_pid", "evidence_tid"):
        if report[key] > 0xFFFF_FFFF:
            fail(f"{key} must fit in u32")
    for key in ("cpu", "evidence_cpu"):
        if report[key] > 0xFFFF:
            fail(f"{key} must fit in u16")

    tracked_pid = report["tracked_child_pid"]
    for key in ("pid", "tid", "evidence_pid", "evidence_tid"):
        if report[key] != tracked_pid:
            fail(f"{key} must equal tracked_child_pid")
    if report["cpu"] != report["evidence_cpu"]:
        fail("cpu must equal evidence_cpu")


def main() -> int:
    if len(sys.argv) != 2:
        print(
            "usage: validate-ebpf-process-exit-qualification.py <report.json>",
            file=sys.stderr,
        )
        return 2
    path = pathlib.Path(sys.argv[1])
    try:
        with path.open(encoding="utf-8") as report_file:
            validate_report(json.load(report_file))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"process-exit eBPF qualification invalid: {error}", file=sys.stderr)
        return 1
    print(f"process-exit eBPF qualification valid: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
