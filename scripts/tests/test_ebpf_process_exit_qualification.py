"""Regression tests for process-exit eBPF runtime qualification evidence."""

import copy
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
VALIDATOR = ROOT / "scripts/validate-ebpf-process-exit-qualification.py"
validator = runpy.run_path(str(VALIDATOR))
validate_report = validator["validate_report"]
ATTACH_PROCESS_EXIT = 1 << 2
BOOT_ID = 0x4553_4F50_5058_5155
LATCHED_INCIDENT_FAULT = 0x4542_2002


def qualified_report():
    pid = 4321
    return {
        "schema_version": 1,
        "status": "qualified",
        "tracked_child_pid": pid,
        "runtime_attach_mask": ATTACH_PROCESS_EXIT,
        "required_attach_mask": ATTACH_PROCESS_EXIT,
        "worker_records_seen": 0,
        "worker_incidents_emitted": 0,
        "worker_malformed_records": 0,
        "worker_evidence_rejected": 0,
        "worker_newly_reported_lost_events": 0,
        "worker_thread_exits_ignored": 1,
        "worker_observation_state": 0,
        "worker_observation_attach_mask": ATTACH_PROCESS_EXIT,
        "worker_observation_lost_event_count": 0,
        "worker_observation_incident_count": 0,
        "worker_observation_fault_code": 0,
        "worker_observation_heartbeat_seq": 1,
        "records_seen": 1,
        "incidents_emitted": 1,
        "malformed_records": 0,
        "evidence_rejected": 0,
        "newly_reported_lost_events": 0,
        "emitted_events": 1,
        "lost_events": 0,
        "process_exits": 1,
        "thread_exits_ignored": 1,
        "oom_events": 0,
        "dropped_incidents": 0,
        "incident_id": 1,
        "incident_boot_id": BOOT_ID,
        "incident_agent_epoch": 1,
        "incident_code": 5,
        "incident_severity": 3,
        "recommended_action": 3,
        "confidence_percent": 100,
        "pid": pid,
        "tid": pid,
        "cpu": 2,
        "irq": 0,
        "netdev_ifindex": 0,
        "cycle_first": 0,
        "cycle_last": 0,
        "transition_seq": 0,
        "incident_observed_value": 1,
        "incident_threshold": 1,
        "incident_evidence_count": 1,
        "incident_event_count": 1,
        "incident_lost_events": 0,
        "evidence_id": 987_654,
        "evidence_boot_id": BOOT_ID,
        "evidence_agent_epoch": 1,
        "evidence_timestamp_ns": 123_456_789,
        "evidence_domain": 4,
        "evidence_kind": 5,
        "evidence_severity": 3,
        "evidence_pid": pid,
        "evidence_tid": pid,
        "evidence_cpu": 2,
        "evidence_irq": 0,
        "evidence_netdev_ifindex": 0,
        "evidence_cycle_seq": 0,
        "evidence_transition_seq": 0,
        "evidence_observed_value": 1,
        "evidence_threshold": 1,
        "evidence_duration_ns": 0,
        "evidence_event_count": 1,
        "evidence_detail": 0,
        "observation_boot_id": BOOT_ID,
        "observation_agent_epoch": 1,
        "observation_state": 2,
        "observation_attach_mask": ATTACH_PROCESS_EXIT,
        "observation_lost_event_count": 0,
        "observation_incident_count": 1,
        "observation_fault_code": LATCHED_INCIDENT_FAULT,
        "observation_heartbeat_seq": 2,
    }


class ProcessExitQualificationTests(unittest.TestCase):
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

        for key in ("schema_version", "worker_records_seen", "evidence_id"):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = True
                self.assert_rejected(report, f"{key} must be an integer")

    def test_attach_worker_and_poll_contracts_are_fail_closed(self):
        for key, value, reason in (
            ("runtime_attach_mask", 0, "runtime_attach_mask must be 4"),
            ("required_attach_mask", 5, "required_attach_mask must be 4"),
            ("worker_records_seen", 1, "worker_records_seen must be 0"),
            ("worker_incidents_emitted", 1, "worker_incidents_emitted must be 0"),
            ("worker_thread_exits_ignored", 0, "worker_thread_exits_ignored must be 1"),
            ("worker_observation_state", 2, "worker_observation_state must be 0"),
            ("records_seen", 0, "records_seen must be 1"),
            ("incidents_emitted", 2, "incidents_emitted must be 1"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_count_loss_and_identity_mismatches_are_rejected(self):
        for key in (
            "malformed_records",
            "evidence_rejected",
            "newly_reported_lost_events",
            "lost_events",
            "oom_events",
            "dropped_incidents",
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = 1
                self.assert_rejected(report, f"{key} must be 0")

        for key in ("pid", "tid", "evidence_pid", "evidence_tid"):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] += 1
                self.assert_rejected(report, f"{key} must equal tracked_child_pid")

    def test_wrong_incident_or_evidence_semantics_are_rejected(self):
        for key, value, reason in (
            ("incident_code", 4, "incident_code must be 5"),
            ("incident_severity", 2, "incident_severity must be 3"),
            ("recommended_action", 2, "recommended_action must be 3"),
            ("confidence_percent", 99, "confidence_percent must be 100"),
            ("evidence_domain", 3, "evidence_domain must be 4"),
            ("evidence_kind", 4, "evidence_kind must be 5"),
            ("evidence_severity", 2, "evidence_severity must be 3"),
            ("evidence_cycle_seq", 1, "evidence_cycle_seq must be 0"),
            ("transition_seq", 1, "transition_seq must be 0"),
            ("evidence_duration_ns", 1, "evidence_duration_ns must be 0"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_observer_mismatch_never_qualifies(self):
        for key, value, reason in (
            ("observation_state", 1, "observation_state must be 2"),
            ("observation_attach_mask", 0, "observation_attach_mask must be 4"),
            ("observation_lost_event_count", 1, "observation_lost_event_count must be 0"),
            ("observation_incident_count", 0, "observation_incident_count must be 1"),
            ("observation_fault_code", 0, "observation_fault_code must be"),
            ("observation_heartbeat_seq", 1, "observation_heartbeat_seq must be 2"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)


if __name__ == "__main__":
    unittest.main()
