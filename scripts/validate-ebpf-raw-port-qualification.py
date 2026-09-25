#!/usr/bin/env python3
"""Validate privileged raw-port eBPF runtime qualification evidence."""

import json
import pathlib
import sys


ATTACH_RAW_PORT = (1 << 16) | (1 << 17)
REQUIRED_KEYS = {
    "schema_version",
    "status",
    "threshold_ns",
    "injected_delay_ns",
    "runtime_attach_mask",
    "required_attach_mask",
    "records_seen",
    "incidents_emitted",
    "malformed_records",
    "evidence_rejected",
    "newly_reported_lost_events",
    "lost_events",
    "raw_port_probe_begins",
    "raw_port_probe_completions",
    "raw_port_stalls",
    "raw_port_probe_mismatches",
    "incident_code",
    "incident_severity",
    "recommended_action",
    "confidence_percent",
    "pid",
    "tid",
    "netdev_ifindex",
    "cycle_seq",
    "detail",
    "incident_evidence_count",
    "event_count",
    "evidence_id",
    "observed_value",
    "evidence_threshold",
    "duration_ns",
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

    if report["schema_version"] != 1:
        fail("schema_version must be 1")
    if report["status"] != "qualified":
        fail("status must be qualified")
    if report["threshold_ns"] == 0:
        fail("threshold_ns must be nonzero")
    if report["injected_delay_ns"] <= report["threshold_ns"]:
        fail("injected_delay_ns must exceed threshold_ns")
    if report["runtime_attach_mask"] != ATTACH_RAW_PORT:
        fail("runtime_attach_mask must contain the complete raw-port pair only")
    if report["required_attach_mask"] != ATTACH_RAW_PORT:
        fail("required_attach_mask must contain the complete raw-port pair only")
    if report["records_seen"] < 1:
        fail("records_seen must be positive")
    if report["incidents_emitted"] != 1:
        fail("incidents_emitted must be 1")

    for key in (
        "malformed_records",
        "evidence_rejected",
        "newly_reported_lost_events",
        "lost_events",
        "raw_port_probe_mismatches",
    ):
        if report[key] != 0:
            fail(f"{key} must be zero")
    for key in (
        "raw_port_probe_begins",
        "raw_port_probe_completions",
        "raw_port_stalls",
        "incident_evidence_count",
        "event_count",
    ):
        if report[key] != 1:
            fail(f"{key} must be 1")

    expected_values = {
        "incident_code": 9,
        "incident_severity": 2,
        "recommended_action": 2,
        "confidence_percent": 80,
        "netdev_ifindex": 7,
        "cycle_seq": 1,
        "detail": 1,
    }
    for key, expected in expected_values.items():
        if report[key] != expected:
            fail(f"{key} must be {expected}")
    for key in ("pid", "tid", "evidence_id"):
        if report[key] == 0:
            fail(f"{key} must be nonzero")
    if report["evidence_threshold"] != report["threshold_ns"]:
        fail("evidence_threshold must equal threshold_ns")
    if report["duration_ns"] <= report["evidence_threshold"]:
        fail("duration_ns must strictly exceed evidence_threshold")
    if report["observed_value"] != report["duration_ns"]:
        fail("observed_value must equal duration_ns")


def main() -> int:
    if len(sys.argv) != 2:
        print(
            "usage: validate-ebpf-raw-port-qualification.py <report.json>",
            file=sys.stderr,
        )
        return 2
    path = pathlib.Path(sys.argv[1])
    try:
        with path.open(encoding="utf-8") as report_file:
            validate_report(json.load(report_file))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"raw-port eBPF qualification invalid: {error}", file=sys.stderr)
        return 1
    print(f"raw-port eBPF qualification valid: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
