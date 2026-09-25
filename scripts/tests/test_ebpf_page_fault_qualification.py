"""Regression tests for page-fault eBPF runtime qualification evidence."""

import copy
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
VALIDATOR = ROOT / "scripts/validate-ebpf-page-fault-qualification.py"
validator = runpy.run_path(str(VALIDATOR))
validate_report = validator["validate_report"]
ATTACH_PAGE_FAULT = 1 << 3
BOOT_ID = 0x4553_4F50_5046_4155
DEGRADED_INCIDENT_FAULT = 0x4542_2001


def qualified_report():
    pid = 4321
    cpu = 7
    timestamp = 123_456_789
    page_size = 4096
    return {
        "schema_version": 1,
        "status": "qualified",
        "architecture": "x86_64",
        "tracked_child_pid": pid,
        "target_cpu": cpu,
        "ready_cpu": cpu,
        "result_cpu": cpu,
        "page_size": page_size,
        "page_count": 16,
        "guarded_mapping_bytes": page_size * 2,
        "minor_faults_before": 200,
        "minor_faults_after": 216,
        "minor_faults_delta": 16,
        "page_fault_threshold": 16,
        "page_fault_window_ns": 1_000_000_000,
        "runtime_attach_mask": ATTACH_PAGE_FAULT,
        "required_attach_mask": ATTACH_PAGE_FAULT,
        "initial_observation_boot_id": BOOT_ID,
        "initial_observation_agent_epoch": 1,
        "initial_observation_state": 0,
        "initial_observation_attach_mask": ATTACH_PAGE_FAULT,
        "initial_observation_lost_event_count": 0,
        "initial_observation_incident_count": 0,
        "initial_observation_fault_code": 0,
        "initial_observation_heartbeat_seq": 1,
        "baseline_emitted_events": 0,
        "baseline_lost_events": 0,
        "baseline_page_faults": 0,
        "baseline_page_fault_threshold_events": 0,
        "records_seen": 1,
        "incidents_emitted": 1,
        "malformed_records": 0,
        "evidence_rejected": 0,
        "newly_reported_lost_events": 0,
        "emitted_events": 1,
        "lost_events": 0,
        "page_faults": 16,
        "page_fault_threshold_events": 1,
        "dropped_incidents": 0,
        "incident_id": 1,
        "incident_boot_id": BOOT_ID,
        "incident_agent_epoch": 1,
        "incident_code": 3,
        "incident_severity": 1,
        "recommended_action": 1,
        "confidence_percent": 65,
        "incident_first_seen_ns": timestamp,
        "incident_last_seen_ns": timestamp,
        "pid": pid,
        "tid": pid,
        "cpu": cpu,
        "irq": 0,
        "netdev_ifindex": 0,
        "cycle_first": 42,
        "cycle_last": 42,
        "transition_seq": 9,
        "incident_observed_value": 16,
        "incident_threshold": 16,
        "incident_evidence_count": 1,
        "incident_event_count": 16,
        "incident_lost_events": 0,
        "evidence_id": timestamp,
        "evidence_boot_id": BOOT_ID,
        "evidence_agent_epoch": 1,
        "evidence_timestamp_ns": timestamp,
        "evidence_domain": 3,
        "evidence_kind": 3,
        "evidence_severity": 1,
        "evidence_pid": pid,
        "evidence_tid": pid,
        "evidence_cpu": cpu,
        "evidence_irq": 0,
        "evidence_netdev_ifindex": 0,
        "evidence_cycle_seq": 42,
        "evidence_transition_seq": 9,
        "evidence_observed_value": 16,
        "evidence_threshold": 16,
        "evidence_duration_ns": 25_000,
        "evidence_event_count": 16,
        "evidence_detail": 6,
        "observation_boot_id": BOOT_ID,
        "observation_agent_epoch": 1,
        "observation_state": 1,
        "observation_attach_mask": ATTACH_PAGE_FAULT,
        "observation_lost_event_count": 0,
        "observation_incident_count": 1,
        "observation_fault_code": DEGRADED_INCIDENT_FAULT,
        "observation_heartbeat_seq": 2,
    }


class PageFaultQualificationTests(unittest.TestCase):
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

    def test_schema_rejects_missing_unknown_and_mistyped_fields(self):
        report = qualified_report()
        del report["evidence_detail"]
        self.assert_rejected(report, "missing keys")

        report = qualified_report()
        report["unexpected"] = 1
        self.assert_rejected(report, "unknown keys")

        for key in ("schema_version", "target_cpu", "evidence_duration_ns"):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = True
                self.assert_rejected(report, f"{key} must be an integer")

    def test_architecture_page_geometry_and_attach_are_exact(self):
        for key, value, reason in (
            ("architecture", "aarch64", "architecture must be x86_64"),
            ("target_cpu", 0xFFFF, "target_cpu must fit"),
            ("ready_cpu", 8, "ready_cpu must be 7"),
            ("page_size", 5000, "page_size must be a bounded power"),
            ("page_count", 15, "page_count must be 16"),
            ("guarded_mapping_bytes", 4096, "guarded_mapping_bytes must be 8192"),
            ("runtime_attach_mask", 4, "runtime_attach_mask must be 8"),
            ("required_attach_mask", 9, "required_attach_mask must be 8"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_child_and_kernel_counts_are_exact(self):
        for key, value, reason in (
            ("minor_faults_after", 215, "minor_faults_delta must match"),
            ("minor_faults_delta", 15, "minor_faults_delta must match"),
            ("baseline_page_faults", 1, "baseline_page_faults must be 0"),
            ("records_seen", 2, "records_seen must be 1"),
            ("page_faults", 17, "page_faults must be 16"),
            ("page_fault_threshold_events", 2, "must be 1"),
            ("lost_events", 1, "lost_events must be 0"),
            ("dropped_incidents", 1, "dropped_incidents must be 0"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_identity_classification_cycle_and_timing_are_exact(self):
        for key, value, reason in (
            ("pid", 4322, "pid must be 4321"),
            ("evidence_cpu", 8, "evidence_cpu must be 7"),
            ("incident_code", 4, "incident_code must be 3"),
            ("incident_severity", 2, "incident_severity must be 1"),
            ("recommended_action", 2, "recommended_action must be 1"),
            ("confidence_percent", 64, "confidence_percent must be 65"),
            ("cycle_first", 41, "cycle_first must be 42"),
            ("transition_seq", 8, "transition_seq must be 9"),
            ("evidence_detail", 4, "evidence_detail must be 6"),
            ("incident_first_seen_ns", 123, "must equal evidence_timestamp_ns"),
            ("evidence_id", 123, "must equal evidence_timestamp_ns"),
            ("evidence_duration_ns", 0, "must be positive"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_observer_transition_must_be_healthy_to_degraded(self):
        for key, value, reason in (
            ("initial_observation_state", 1, "must be 0"),
            ("observation_state", 0, "must be 1"),
            ("observation_incident_count", 0, "must be 1"),
            ("observation_fault_code", 0, "observation_fault_code must be"),
            ("observation_heartbeat_seq", 1, "must be 2"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)


if __name__ == "__main__":
    unittest.main()
