#!/usr/bin/env python3
"""Validate privileged gateway eBPF runtime qualification evidence."""

import json
import pathlib
import sys


ATTACH_GATEWAY = sum(1 << bit for bit in range(12, 16))
PUBLISH_REQUEST_ID = 101
CALLBACK_REQUEST_ID = 202
PUBLISH_DETAIL = 2
CALLBACK_DETAIL = 3
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
    "gateway_probe_begins",
    "gateway_probe_completions",
    "gateway_stalls",
    "gateway_probe_mismatches",
    "dropped_incidents",
    "incident_code",
    "incident_severity",
    "recommended_action",
    "confidence_percent",
    "pid",
    "tid",
    "cycle_first",
    "cycle_last",
    "incident_evidence_count",
    "incident_event_count",
    "incident_lost_events",
    "publish_request_id",
    "publish_domain",
    "publish_kind",
    "publish_severity",
    "publish_detail",
    "publish_pid",
    "publish_tid",
    "publish_cycle_seq",
    "publish_event_count",
    "publish_observed_value",
    "publish_evidence_threshold",
    "publish_duration_ns",
    "callback_request_id",
    "callback_domain",
    "callback_kind",
    "callback_severity",
    "callback_detail",
    "callback_pid",
    "callback_tid",
    "callback_cycle_seq",
    "callback_event_count",
    "callback_observed_value",
    "callback_evidence_threshold",
    "callback_duration_ns",
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


def validate_evidence(report: dict, prefix: str, request_id: int, detail: int) -> None:
    if report[f"{prefix}_request_id"] != request_id:
        fail(f"{prefix}_request_id must be {request_id}")
    if report[f"{prefix}_domain"] != 7:
        fail(f"{prefix}_domain must be 7")
    if report[f"{prefix}_kind"] != 7:
        fail(f"{prefix}_kind must be 7")
    if report[f"{prefix}_severity"] != 2:
        fail(f"{prefix}_severity must be 2")
    if report[f"{prefix}_detail"] != detail:
        fail(f"{prefix}_detail must be {detail}")
    if report[f"{prefix}_pid"] != report["pid"]:
        fail(f"{prefix}_pid must equal pid")
    if report[f"{prefix}_tid"] != report["tid"]:
        fail(f"{prefix}_tid must equal tid")
    if report[f"{prefix}_cycle_seq"] != 1:
        fail(f"{prefix}_cycle_seq must be 1")
    if report[f"{prefix}_event_count"] != 1:
        fail(f"{prefix}_event_count must be 1")
    if report[f"{prefix}_evidence_threshold"] != report["threshold_ns"]:
        fail(f"{prefix}_evidence_threshold must equal threshold_ns")
    if report[f"{prefix}_duration_ns"] <= report[f"{prefix}_evidence_threshold"]:
        fail(f"{prefix}_duration_ns must strictly exceed its threshold")
    if report[f"{prefix}_observed_value"] != report[f"{prefix}_duration_ns"]:
        fail(f"{prefix}_observed_value must equal {prefix}_duration_ns")


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
    if report["runtime_attach_mask"] != ATTACH_GATEWAY:
        fail("runtime_attach_mask must contain both complete gateway pairs only")
    if report["required_attach_mask"] != ATTACH_GATEWAY:
        fail("required_attach_mask must contain both complete gateway pairs only")

    expected_twos = (
        "records_seen",
        "incidents_emitted",
        "gateway_probe_begins",
        "gateway_probe_completions",
        "gateway_stalls",
        "incident_evidence_count",
        "incident_event_count",
    )
    for key in expected_twos:
        if report[key] != 2:
            fail(f"{key} must be 2")

    expected_zeroes = (
        "malformed_records",
        "evidence_rejected",
        "newly_reported_lost_events",
        "lost_events",
        "gateway_probe_mismatches",
        "dropped_incidents",
        "incident_lost_events",
    )
    for key in expected_zeroes:
        if report[key] != 0:
            fail(f"{key} must be zero")

    expected_values = {
        "incident_code": 7,
        "incident_severity": 2,
        "recommended_action": 2,
        "confidence_percent": 75,
        "cycle_first": 1,
        "cycle_last": 1,
    }
    for key, expected in expected_values.items():
        if report[key] != expected:
            fail(f"{key} must be {expected}")
    for key in ("pid", "tid"):
        if report[key] == 0:
            fail(f"{key} must be nonzero")

    validate_evidence(report, "publish", PUBLISH_REQUEST_ID, PUBLISH_DETAIL)
    validate_evidence(report, "callback", CALLBACK_REQUEST_ID, CALLBACK_DETAIL)
    if report["publish_request_id"] == report["callback_request_id"]:
        fail("publish and callback request IDs must be distinct")


def main() -> int:
    if len(sys.argv) != 2:
        print(
            "usage: validate-ebpf-gateway-qualification.py <report.json>",
            file=sys.stderr,
        )
        return 2
    path = pathlib.Path(sys.argv[1])
    try:
        with path.open(encoding="utf-8") as report_file:
            validate_report(json.load(report_file))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"gateway eBPF qualification invalid: {error}", file=sys.stderr)
        return 1
    print(f"gateway eBPF qualification valid: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
