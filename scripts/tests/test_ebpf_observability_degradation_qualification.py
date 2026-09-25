"""Regression tests for eBPF observability degradation qualification evidence."""

import copy
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
VALIDATOR = ROOT / "scripts/validate-ebpf-observability-degradation-qualification.py"
validator = runpy.run_path(str(VALIDATOR))
validate_report = validator["validate_report"]
ATTACH_PAGE_FAULT = 1 << 3
BOOT_ID = 0x4553_4F50_4445_4752
EVENT_LOSS_FAULT = 0x4542_1004


def qualified_report():
    page_size = 4096
    chunk_bytes = 64 << 20
    chunk_pages = chunk_bytes // page_size
    batches = 3
    pages = batches * chunk_pages
    saturation_lost = 8852
    return {
        "schema_version": 1,
        "status": "qualified",
        "architecture": "x86_64",
        "boot_id": BOOT_ID,
        "tracked_pid": 4321,
        "page_size": page_size,
        "ringbuf_bytes": 1 << 22,
        "chunk_bytes": chunk_bytes,
        "chunk_pages": chunk_pages,
        "max_batches": 8,
        "max_pages": chunk_pages * 8,
        "batches_run": batches,
        "pages_touched": pages,
        "preflight_available_attach_mask": ATTACH_PAGE_FAULT,
        "preflight_capability_mask": 15,
        "preflight_missing_capabilities": 0,
        "missing_btf_state": 1,
        "missing_btf_fault": 0x4542_1001,
        "missing_ringbuf_state": 1,
        "missing_ringbuf_fault": 0x4542_1001,
        "missing_attach_state": 1,
        "missing_attach_fault": 0x4542_1003,
        "missing_verifier_state": 2,
        "missing_verifier_fault": 0x4542_1002,
        "missing_permission_state": 2,
        "missing_permission_fault": 0x4542_1002,
        "first_agent_epoch": 1,
        "first_runtime_attach_mask": ATTACH_PAGE_FAULT,
        "first_runtime_required_attach_mask": ATTACH_PAGE_FAULT,
        "first_runtime_capability_mask": 15,
        "initial_state": 0,
        "initial_attach_mask": ATTACH_PAGE_FAULT,
        "initial_lost_events": 0,
        "initial_incidents": 0,
        "initial_fault": 0,
        "initial_heartbeat_seq": 1,
        "baseline_emitted_events": 10,
        "baseline_lost_events": 0,
        "baseline_page_faults": 10,
        "baseline_page_fault_threshold_events": 10,
        "saturation_emitted_events": 40310,
        "saturation_lost_events": saturation_lost,
        "saturation_page_faults": 49162,
        "saturation_page_fault_threshold_events": 40310,
        "saturation_records_seen": 1,
        "saturation_incidents_emitted": 0,
        "saturation_malformed_records": 0,
        "saturation_evidence_rejected": 0,
        "saturation_newly_reported_lost_events": saturation_lost,
        "loss_state": 1,
        "loss_attach_mask": ATTACH_PAGE_FAULT,
        "loss_lost_events": saturation_lost,
        "loss_incidents": 0,
        "loss_fault": EVENT_LOSS_FAULT,
        "loss_heartbeat_seq": 2,
        "reapplied_state": 1,
        "reapplied_lost_events": saturation_lost,
        "reapplied_fault": EVENT_LOSS_FAULT,
        "reapplied_heartbeat_seq": 3,
        "first_runtime_detached": 1,
        "second_agent_epoch": 2,
        "restarting_state": 1,
        "restarting_attach_mask": 0,
        "restarting_lost_events": 0,
        "restarting_incidents": 0,
        "restarting_fault": 0,
        "restarting_heartbeat_seq": 1,
        "second_runtime_attach_mask": ATTACH_PAGE_FAULT,
        "second_runtime_required_attach_mask": ATTACH_PAGE_FAULT,
        "second_runtime_capability_mask": 15,
        "recovered_state": 0,
        "recovered_attach_mask": ATTACH_PAGE_FAULT,
        "recovered_lost_events": 0,
        "recovered_incidents": 0,
        "recovered_fault": 0,
        "recovered_heartbeat_seq": 2,
        "recovery_pages_touched": chunk_pages,
        "recovery_records_seen": 64,
        "recovery_incidents_emitted": 0,
        "recovery_malformed_records": 0,
        "recovery_evidence_rejected": 0,
        "recovery_newly_reported_lost_events": 0,
        "recovery_emitted_events": chunk_pages,
        "recovery_lost_events": 0,
        "recovery_page_faults": chunk_pages,
        "final_state": 0,
        "final_attach_mask": ATTACH_PAGE_FAULT,
        "final_lost_events": 0,
        "final_incidents": 0,
        "final_fault": 0,
        "final_heartbeat_seq": 3,
        "second_runtime_detached": 1,
        "cleanup_succeeded": 1,
    }


class ObservabilityDegradationQualificationTests(unittest.TestCase):
    def assert_rejected(self, report, reason):
        with self.assertRaisesRegex(ValueError, reason):
            validate_report(report)

    def test_qualified_report_and_cli_are_accepted(self):
        report = qualified_report()
        validate_report(copy.deepcopy(report))
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "qualification.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            result = subprocess.run(
                [sys.executable, str(VALIDATOR), str(path)],
                check=False,
                capture_output=True,
                text=True,
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("qualification valid", result.stdout)

    def test_schema_and_geometry_are_closed(self):
        report = qualified_report()
        del report["cleanup_succeeded"]
        self.assert_rejected(report, "missing keys")

        report = qualified_report()
        report["unexpected"] = 1
        self.assert_rejected(report, "unknown keys")

        for key, value, reason in (
            ("page_size", 5000, "page_size must be a bounded power"),
            ("ringbuf_bytes", 1024, "ringbuf_bytes must be"),
            ("chunk_pages", 1, "chunk_pages must be"),
            ("batches_run", 0, "batches_run must be within"),
            ("pages_touched", 1, "pages_touched must be"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_capability_matrix_and_runtime_masks_are_exact(self):
        for key, value, reason in (
            ("preflight_available_attach_mask", 0, "must expose page_fault_user"),
            ("preflight_capability_mask", 7, "must be 15"),
            ("missing_btf_state", 2, "must be 1"),
            ("missing_attach_fault", 0, "missing_attach_fault must be"),
            ("missing_permission_state", 1, "must be 2"),
            ("first_runtime_attach_mask", 0, "must be 8"),
            ("second_runtime_capability_mask", 7, "must be 15"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_saturation_requires_real_loss_and_exact_projection(self):
        for key, value, reason in (
            ("saturation_emitted_events", 10, "must advance"),
            ("saturation_lost_events", 0, "must be positive"),
            ("saturation_page_faults", 49_161, "must cover every touched page"),
            ("saturation_page_fault_threshold_events", 40_309, "must match"),
            ("saturation_records_seen", 0, "must be 1"),
            ("saturation_newly_reported_lost_events", 8_851, "must be 8852"),
            ("loss_lost_events", 1, "must be 8852"),
            ("loss_fault", 0, "loss_fault must be"),
            ("reapplied_state", 0, "must be 1"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_restart_must_clear_epoch_local_state_before_recovery(self):
        for key, value, reason in (
            ("second_agent_epoch", 1, "must be 2"),
            ("restarting_attach_mask", ATTACH_PAGE_FAULT, "must be 0"),
            ("restarting_lost_events", 1, "must be 0"),
            ("recovered_state", 1, "must be 0"),
            ("recovered_heartbeat_seq", 1, "must be 2"),
            ("recovery_records_seen", 0, "must be within one bounded poll"),
            ("recovery_lost_events", 1, "must be 0"),
            ("final_state", 1, "must be 0"),
            ("cleanup_succeeded", 0, "must be 1"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_integer_fields_reject_boolean_values(self):
        for key in ("schema_version", "saturation_lost_events", "final_state"):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = True
                self.assert_rejected(report, f"{key} must be an integer")


if __name__ == "__main__":
    unittest.main()
