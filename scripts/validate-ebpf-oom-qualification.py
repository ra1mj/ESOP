#!/usr/bin/env python3
"""Validate privileged controlled memcg OOM qualification evidence."""

import json
import pathlib
import sys


ATTACH_OOM_KILL = 1 << 4
BOOT_ID = 0x4553_4F50_4F4F_4D51
AGENT_EPOCH = 1
MEMORY_LIMIT_BYTES = 32 * 1024 * 1024
MAPPING_BYTES = 128 * 1024 * 1024
SIGKILL = 9
LATCHED_INCIDENT_FAULT = 0x4542_2002
U64_MAX = (1 << 64) - 1
REQUIRED_KEYS = {
    "schema_version",
    "status",
    "cgroup_version",
    "memory_controller_enabled",
    "memory_limit_bytes",
    "mapping_bytes",
    "page_size",
    "mapping_pages",
    "swap_limit_present",
    "swap_limit_bytes",
    "oom_group",
    "tracked_child_pid",
    "initial_member_count",
    "final_member_count",
    "child_signal",
    "cleanup_succeeded",
    "baseline_memory_max_events",
    "baseline_memory_oom_events",
    "baseline_memory_oom_kill_events",
    "baseline_memory_oom_group_kill_events",
    "memory_max_events",
    "memory_oom_events",
    "memory_oom_kill_events",
    "memory_oom_group_kill_events",
    "memory_max_delta",
    "memory_oom_delta",
    "memory_oom_kill_delta",
    "memory_oom_group_kill_delta",
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
    "baseline_process_exits",
    "baseline_oom_events",
    "records_seen",
    "incidents_emitted",
    "malformed_records",
    "evidence_rejected",
    "newly_reported_lost_events",
    "emitted_events",
    "lost_events",
    "process_exits",
    "oom_events",
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
    require_value(report, "cgroup_version", 2)
    require_value(report, "memory_controller_enabled", 1)
    require_value(report, "memory_limit_bytes", MEMORY_LIMIT_BYTES)
    require_value(report, "mapping_bytes", MAPPING_BYTES)
    if report["mapping_bytes"] <= report["memory_limit_bytes"]:
        fail("mapping_bytes must exceed memory_limit_bytes")

    page_size = report["page_size"]
    if page_size < 4096 or page_size > (1 << 30) or page_size & (page_size - 1):
        fail("page_size must be a bounded power of two of at least 4096")
    if report["mapping_bytes"] % page_size:
        fail("mapping_bytes must be page aligned")
    require_value(report, "mapping_pages", report["mapping_bytes"] // page_size)
    if report["swap_limit_present"] not in (0, 1):
        fail("swap_limit_present must be 0 or 1")
    require_value(report, "swap_limit_bytes", 0)
    require_value(report, "oom_group", 0)

    tracked_pid = report["tracked_child_pid"]
    if tracked_pid == 0 or tracked_pid > 0x7FFF_FFFF:
        fail("tracked_child_pid must be a positive signed-32-bit Linux task ID")
    require_value(report, "initial_member_count", 1)
    require_value(report, "final_member_count", 0)
    require_value(report, "child_signal", SIGKILL)
    require_value(report, "cleanup_succeeded", 1)

    baseline_keys = (
        "baseline_memory_max_events",
        "baseline_memory_oom_events",
        "baseline_memory_oom_kill_events",
        "baseline_memory_oom_group_kill_events",
    )
    for key in baseline_keys:
        require_value(report, key, 0)

    counter_pairs = (
        ("memory_max_events", "baseline_memory_max_events", "memory_max_delta"),
        ("memory_oom_events", "baseline_memory_oom_events", "memory_oom_delta"),
        (
            "memory_oom_kill_events",
            "baseline_memory_oom_kill_events",
            "memory_oom_kill_delta",
        ),
        (
            "memory_oom_group_kill_events",
            "baseline_memory_oom_group_kill_events",
            "memory_oom_group_kill_delta",
        ),
    )
    for final_key, baseline_key, delta_key in counter_pairs:
        if report[final_key] < report[baseline_key]:
            fail(f"{final_key} must not regress")
        if report[final_key] - report[baseline_key] != report[delta_key]:
            fail(f"{delta_key} must match the counter difference")
    if report["memory_max_delta"] < 1:
        fail("memory_max_delta must be at least 1")
    if report["memory_oom_delta"] < 1:
        fail("memory_oom_delta must be at least 1")
    require_value(report, "memory_oom_kill_delta", 1)
    require_value(report, "memory_oom_group_kill_delta", 0)

    for key in (
        "runtime_attach_mask",
        "required_attach_mask",
        "initial_observation_attach_mask",
        "observation_attach_mask",
    ):
        require_value(report, key, ATTACH_OOM_KILL)

    for key in (
        "initial_observation_lost_event_count",
        "initial_observation_incident_count",
        "initial_observation_fault_code",
        "baseline_emitted_events",
        "baseline_lost_events",
        "baseline_process_exits",
        "baseline_oom_events",
        "malformed_records",
        "evidence_rejected",
        "newly_reported_lost_events",
        "lost_events",
        "process_exits",
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

    for key in (
        "records_seen",
        "incidents_emitted",
        "emitted_events",
        "oom_events",
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

    expected_values = {
        "initial_observation_boot_id": BOOT_ID,
        "initial_observation_agent_epoch": AGENT_EPOCH,
        "initial_observation_state": 0,
        "initial_observation_heartbeat_seq": 1,
        "incident_boot_id": BOOT_ID,
        "incident_agent_epoch": AGENT_EPOCH,
        "incident_code": 4,
        "incident_severity": 3,
        "recommended_action": 3,
        "confidence_percent": 100,
        "evidence_boot_id": BOOT_ID,
        "evidence_agent_epoch": AGENT_EPOCH,
        "evidence_domain": 3,
        "evidence_kind": 4,
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
    for key in ("pid", "tid", "evidence_pid", "evidence_tid"):
        require_value(report, key, tracked_pid)
    if report["cpu"] > 0xFFFF or report["evidence_cpu"] > 0xFFFF:
        fail("CPU fields must fit in u16")
    require_value(report, "evidence_cpu", report["cpu"])
    require_value(report, "incident_first_seen_ns", report["evidence_timestamp_ns"])
    require_value(report, "incident_last_seen_ns", report["evidence_timestamp_ns"])


def main() -> int:
    if len(sys.argv) != 2:
        print(
            "usage: validate-ebpf-oom-qualification.py <report.json>",
            file=sys.stderr,
        )
        return 2
    path = pathlib.Path(sys.argv[1])
    try:
        with path.open(encoding="utf-8") as report_file:
            validate_report(json.load(report_file))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"OOM eBPF qualification invalid: {error}", file=sys.stderr)
        return 1
    print(f"OOM eBPF qualification valid: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
