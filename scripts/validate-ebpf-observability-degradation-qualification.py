#!/usr/bin/env python3
"""Validate privileged eBPF observability degradation qualification evidence."""

import json
import pathlib
import sys


ATTACH_PAGE_FAULT = 1 << 3
BOOT_ID = 0x4553_4F50_4445_4752
COMPLETE_CAPABILITIES = (1 << 4) - 1
RINGBUF_BYTES = 1 << 22
CHUNK_BYTES = 64 << 20
MAX_BATCHES = 8
EVENT_LOSS_FAULT = 0x4542_1004
CAPABILITY_FAULT = 0x4542_1001
LOAD_FAULT = 0x4542_1002
ATTACH_FAULT = 0x4542_1003
U32_MAX = (1 << 32) - 1
U64_MAX = (1 << 64) - 1
REQUIRED_KEYS = {
    "schema_version",
    "status",
    "architecture",
    "boot_id",
    "tracked_pid",
    "page_size",
    "ringbuf_bytes",
    "chunk_bytes",
    "chunk_pages",
    "max_batches",
    "max_pages",
    "batches_run",
    "pages_touched",
    "preflight_available_attach_mask",
    "preflight_capability_mask",
    "preflight_missing_capabilities",
    "missing_btf_state",
    "missing_btf_fault",
    "missing_ringbuf_state",
    "missing_ringbuf_fault",
    "missing_attach_state",
    "missing_attach_fault",
    "missing_verifier_state",
    "missing_verifier_fault",
    "missing_permission_state",
    "missing_permission_fault",
    "first_agent_epoch",
    "first_runtime_attach_mask",
    "first_runtime_required_attach_mask",
    "first_runtime_capability_mask",
    "initial_state",
    "initial_attach_mask",
    "initial_lost_events",
    "initial_incidents",
    "initial_fault",
    "initial_heartbeat_seq",
    "baseline_emitted_events",
    "baseline_lost_events",
    "baseline_page_faults",
    "baseline_page_fault_threshold_events",
    "saturation_emitted_events",
    "saturation_lost_events",
    "saturation_page_faults",
    "saturation_page_fault_threshold_events",
    "saturation_records_seen",
    "saturation_incidents_emitted",
    "saturation_malformed_records",
    "saturation_evidence_rejected",
    "saturation_newly_reported_lost_events",
    "loss_state",
    "loss_attach_mask",
    "loss_lost_events",
    "loss_incidents",
    "loss_fault",
    "loss_heartbeat_seq",
    "reapplied_state",
    "reapplied_lost_events",
    "reapplied_fault",
    "reapplied_heartbeat_seq",
    "first_runtime_detached",
    "second_agent_epoch",
    "restarting_state",
    "restarting_attach_mask",
    "restarting_lost_events",
    "restarting_incidents",
    "restarting_fault",
    "restarting_heartbeat_seq",
    "second_runtime_attach_mask",
    "second_runtime_required_attach_mask",
    "second_runtime_capability_mask",
    "recovered_state",
    "recovered_attach_mask",
    "recovered_lost_events",
    "recovered_incidents",
    "recovered_fault",
    "recovered_heartbeat_seq",
    "recovery_pages_touched",
    "recovery_records_seen",
    "recovery_incidents_emitted",
    "recovery_malformed_records",
    "recovery_evidence_rejected",
    "recovery_newly_reported_lost_events",
    "recovery_emitted_events",
    "recovery_lost_events",
    "recovery_page_faults",
    "final_state",
    "final_attach_mask",
    "final_lost_events",
    "final_incidents",
    "final_fault",
    "final_heartbeat_seq",
    "second_runtime_detached",
    "cleanup_succeeded",
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
    require_value(report, "boot_id", BOOT_ID)
    if report["tracked_pid"] == 0 or report["tracked_pid"] > 0x7FFF_FFFF:
        fail("tracked_pid must be a positive signed-32-bit Linux task ID")

    page_size = report["page_size"]
    if page_size < 4096 or page_size > (1 << 30) or page_size & (page_size - 1):
        fail("page_size must be a bounded power of two of at least 4096")
    require_value(report, "ringbuf_bytes", RINGBUF_BYTES)
    require_value(report, "chunk_bytes", CHUNK_BYTES)
    if CHUNK_BYTES % page_size:
        fail("chunk_bytes must be page aligned")
    chunk_pages = CHUNK_BYTES // page_size
    require_value(report, "chunk_pages", chunk_pages)
    require_value(report, "max_batches", MAX_BATCHES)
    require_value(report, "max_pages", chunk_pages * MAX_BATCHES)
    batches = report["batches_run"]
    if batches == 0 or batches > MAX_BATCHES:
        fail("batches_run must be within the bounded saturation budget")
    require_value(report, "pages_touched", batches * chunk_pages)

    if report["preflight_available_attach_mask"] & ATTACH_PAGE_FAULT == 0:
        fail("preflight must expose page_fault_user")
    require_value(report, "preflight_capability_mask", COMPLETE_CAPABILITIES)
    require_value(report, "preflight_missing_capabilities", 0)

    expected_policy = {
        "missing_btf_state": 1,
        "missing_btf_fault": CAPABILITY_FAULT,
        "missing_ringbuf_state": 1,
        "missing_ringbuf_fault": CAPABILITY_FAULT,
        "missing_attach_state": 1,
        "missing_attach_fault": ATTACH_FAULT,
        "missing_verifier_state": 2,
        "missing_verifier_fault": LOAD_FAULT,
        "missing_permission_state": 2,
        "missing_permission_fault": LOAD_FAULT,
    }
    for key, value in expected_policy.items():
        require_value(report, key, value)

    for key in (
        "first_runtime_attach_mask",
        "first_runtime_required_attach_mask",
        "initial_attach_mask",
        "loss_attach_mask",
        "second_runtime_attach_mask",
        "second_runtime_required_attach_mask",
        "recovered_attach_mask",
        "final_attach_mask",
    ):
        require_value(report, key, ATTACH_PAGE_FAULT)
    for key in ("first_runtime_capability_mask", "second_runtime_capability_mask"):
        require_value(report, key, COMPLETE_CAPABILITIES)
    require_value(report, "first_agent_epoch", 1)
    require_value(report, "second_agent_epoch", 2)

    expected_zero = (
        "initial_lost_events",
        "initial_incidents",
        "initial_fault",
        "baseline_lost_events",
        "saturation_incidents_emitted",
        "saturation_malformed_records",
        "saturation_evidence_rejected",
        "loss_incidents",
        "restarting_attach_mask",
        "restarting_lost_events",
        "restarting_incidents",
        "restarting_fault",
        "recovered_lost_events",
        "recovered_incidents",
        "recovered_fault",
        "recovery_incidents_emitted",
        "recovery_malformed_records",
        "recovery_evidence_rejected",
        "recovery_newly_reported_lost_events",
        "recovery_lost_events",
        "final_lost_events",
        "final_incidents",
        "final_fault",
    )
    for key in expected_zero:
        require_value(report, key, 0)

    expected_state = {
        "initial_state": 0,
        "initial_heartbeat_seq": 1,
        "loss_state": 1,
        "loss_fault": EVENT_LOSS_FAULT,
        "loss_heartbeat_seq": 2,
        "reapplied_state": 1,
        "reapplied_fault": EVENT_LOSS_FAULT,
        "reapplied_heartbeat_seq": 3,
        "restarting_state": 1,
        "restarting_heartbeat_seq": 1,
        "recovered_state": 0,
        "recovered_heartbeat_seq": 2,
        "final_state": 0,
        "final_heartbeat_seq": 3,
    }
    for key, value in expected_state.items():
        require_value(report, key, value)

    if report["saturation_emitted_events"] <= report["baseline_emitted_events"]:
        fail("saturation_emitted_events must advance")
    if report["saturation_lost_events"] == 0:
        fail("saturation_lost_events must be positive")
    minimum_faults = report["baseline_page_faults"] + report["pages_touched"]
    if report["saturation_page_faults"] < minimum_faults:
        fail("saturation_page_faults must cover every touched page")
    if (
        report["saturation_page_fault_threshold_events"]
        != report["saturation_emitted_events"]
    ):
        fail("page-fault threshold events must match successful emissions")
    require_value(report, "saturation_records_seen", 1)
    projected_loss = min(report["saturation_lost_events"], U32_MAX)
    require_value(report, "saturation_newly_reported_lost_events", projected_loss)
    require_value(report, "loss_lost_events", projected_loss)
    require_value(report, "reapplied_lost_events", projected_loss)

    for key in (
        "first_runtime_detached",
        "second_runtime_detached",
        "cleanup_succeeded",
    ):
        require_value(report, key, 1)
    require_value(report, "recovery_pages_touched", chunk_pages)
    if report["recovery_records_seen"] == 0 or report["recovery_records_seen"] > 64:
        fail("recovery_records_seen must be within one bounded poll")
    if report["recovery_page_faults"] < chunk_pages:
        fail("recovery_page_faults must cover the recovery chunk")
    if report["recovery_emitted_events"] == 0:
        fail("recovery_emitted_events must be positive")


def main() -> int:
    if len(sys.argv) != 2:
        print(
            "usage: validate-ebpf-observability-degradation-qualification.py <report.json>",
            file=sys.stderr,
        )
        return 2
    path = pathlib.Path(sys.argv[1])
    try:
        with path.open(encoding="utf-8") as report_file:
            validate_report(json.load(report_file))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(
            f"observability degradation eBPF qualification invalid: {error}",
            file=sys.stderr,
        )
        return 1
    print(f"observability degradation eBPF qualification valid: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
