"""Regression tests for scheduler-migration eBPF qualification evidence."""

import copy
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
VALIDATOR = ROOT / "scripts/validate-ebpf-scheduler-migration-qualification.py"
validator = runpy.run_path(str(VALIDATOR))
validate_report = validator["validate_report"]
ATTACH_SCHED_MIGRATE_TASK = 1 << 11
BOOT_ID = 0x4553_4F50_4D49_4752
MIGRATION_WINDOW_NS = 10_000_000_000
DEGRADED_INCIDENT_FAULT = 0x4542_2001


def qualified_report():
    worker_tid = 4321
    cpu_a = 2
    cpu_b = 7
    timestamp = 123_456_789
    return {
        "schema_version": 1,
        "status": "qualified",
        "worker_tid": worker_tid,
        "cpu_a": cpu_a,
        "cpu_b": cpu_b,
        "migration_threshold": 2,
        "migration_window_ns": MIGRATION_WINDOW_NS,
        "runtime_attach_mask": ATTACH_SCHED_MIGRATE_TASK,
        "required_attach_mask": ATTACH_SCHED_MIGRATE_TASK,
        "initial_observation_boot_id": BOOT_ID,
        "initial_observation_agent_epoch": 1,
        "initial_observation_state": 0,
        "initial_observation_attach_mask": ATTACH_SCHED_MIGRATE_TASK,
        "initial_observation_lost_event_count": 0,
        "initial_observation_incident_count": 0,
        "initial_observation_fault_code": 0,
        "initial_observation_heartbeat_seq": 1,
        "baseline_emitted_events": 0,
        "baseline_lost_events": 0,
        "baseline_scheduler_migrations": 0,
        "baseline_threshold_events": 0,
        "first_records_seen": 0,
        "first_incidents_emitted": 0,
        "first_malformed_records": 0,
        "first_evidence_rejected": 0,
        "first_newly_reported_lost_events": 0,
        "first_emitted_events": 0,
        "first_lost_events": 0,
        "first_scheduler_migrations": 1,
        "first_threshold_events": 0,
        "records_seen": 1,
        "incidents_emitted": 1,
        "malformed_records": 0,
        "evidence_rejected": 0,
        "newly_reported_lost_events": 0,
        "emitted_events": 1,
        "lost_events": 0,
        "scheduler_migrations": 2,
        "scheduler_migration_threshold_events": 1,
        "dropped_incidents": 0,
        "incident_id": 1,
        "incident_boot_id": BOOT_ID,
        "incident_agent_epoch": 1,
        "incident_code": 0,
        "incident_severity": 1,
        "recommended_action": 1,
        "confidence_percent": 60,
        "incident_first_seen_ns": timestamp,
        "incident_last_seen_ns": timestamp,
        "pid": 0,
        "tid": worker_tid,
        "cpu": cpu_a,
        "irq": cpu_b,
        "netdev_ifindex": 0,
        "cycle_first": 42,
        "cycle_last": 42,
        "transition_seq": 9,
        "incident_observed_value": 2,
        "incident_threshold": 2,
        "incident_evidence_count": 1,
        "incident_event_count": 2,
        "incident_lost_events": 0,
        "evidence_id": 987_654,
        "evidence_boot_id": BOOT_ID,
        "evidence_agent_epoch": 1,
        "evidence_timestamp_ns": timestamp,
        "evidence_domain": 0,
        "evidence_kind": 10,
        "evidence_severity": 1,
        "evidence_pid": 0,
        "evidence_tid": worker_tid,
        "evidence_cpu": cpu_a,
        "evidence_irq": cpu_b,
        "evidence_netdev_ifindex": 0,
        "evidence_cycle_seq": 42,
        "evidence_transition_seq": 9,
        "evidence_observed_value": 2,
        "evidence_threshold": 2,
        "evidence_duration_ns": 1_000,
        "evidence_event_count": 2,
        "evidence_detail": 120,
        "observation_boot_id": BOOT_ID,
        "observation_agent_epoch": 1,
        "observation_state": 1,
        "observation_attach_mask": ATTACH_SCHED_MIGRATE_TASK,
        "observation_lost_event_count": 0,
        "observation_incident_count": 1,
        "observation_fault_code": DEGRADED_INCIDENT_FAULT,
        "observation_heartbeat_seq": 2,
    }


class SchedulerMigrationQualificationTests(unittest.TestCase):
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

        for key in ("schema_version", "worker_tid", "evidence_duration_ns"):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = True
                self.assert_rejected(report, f"{key} must be an integer")

    def test_cpu_and_attach_prerequisites_are_fail_closed(self):
        for key, value, reason in (
            ("runtime_attach_mask", 0, "runtime_attach_mask must be 2048"),
            ("required_attach_mask", 1, "required_attach_mask must be 2048"),
            ("worker_tid", 0, "worker_tid must be"),
            ("cpu_b", 2, "cpu_a and cpu_b must be distinct"),
            ("cpu_a", 0x1_0000, "qualification CPUs must fit in u16"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_baseline_and_first_migration_must_remain_below_threshold(self):
        for key, value, reason in (
            ("baseline_scheduler_migrations", 1, "must be 0"),
            ("first_records_seen", 1, "must be 0"),
            ("first_incidents_emitted", 1, "must be 0"),
            ("first_emitted_events", 1, "must be 0"),
            ("first_scheduler_migrations", 0, "must be 1"),
            ("first_threshold_events", 1, "must be 0"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_final_counts_and_loss_fields_are_exact(self):
        for key, value, reason in (
            ("records_seen", 2, "records_seen must be 1"),
            ("incidents_emitted", 0, "incidents_emitted must be 1"),
            ("scheduler_migrations", 3, "scheduler_migrations must be 2"),
            (
                "scheduler_migration_threshold_events",
                0,
                "scheduler_migration_threshold_events must be 1",
            ),
            ("lost_events", 1, "lost_events must be 0"),
            ("dropped_incidents", 1, "dropped_incidents must be 0"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_identity_and_a_to_b_to_a_endpoint_must_match(self):
        for key, value, reason in (
            ("tid", 4322, "tid must be 4321"),
            ("evidence_tid", 4322, "evidence_tid must be 4321"),
            ("cpu", 7, "cpu must be 2"),
            ("evidence_cpu", 7, "evidence_cpu must be 2"),
            ("irq", 2, "irq must be 7"),
            ("evidence_irq", 2, "evidence_irq must be 7"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_incident_evidence_and_cycle_semantics_are_exact(self):
        for key, value, reason in (
            ("incident_code", 1, "incident_code must be 0"),
            ("incident_severity", 2, "incident_severity must be 1"),
            ("recommended_action", 2, "recommended_action must be 1"),
            ("confidence_percent", 70, "confidence_percent must be 60"),
            ("evidence_kind", 0, "evidence_kind must be 10"),
            ("evidence_cycle_seq", 41, "evidence_cycle_seq must be 42"),
            ("transition_seq", 8, "transition_seq must be 9"),
            ("incident_event_count", 1, "incident_event_count must be 2"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_duration_timestamps_and_priority_width_are_bounded(self):
        for key, value, reason in (
            ("evidence_duration_ns", 0, "must be positive"),
            ("evidence_duration_ns", MIGRATION_WINDOW_NS, "must be positive"),
            (
                "incident_first_seen_ns",
                123_456_788,
                "must equal evidence_timestamp_ns",
            ),
            ("evidence_detail", 256, "evidence_detail must fit in u8"),
        ):
            with self.subTest(key=key, value=value):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_observer_transition_must_be_healthy_to_degraded(self):
        for key, value, reason in (
            ("initial_observation_state", 1, "initial_observation_state must be 0"),
            ("observation_state", 0, "observation_state must be 1"),
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
