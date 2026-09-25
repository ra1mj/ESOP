"""Regression tests for scheduler-runqueue eBPF qualification evidence."""

import copy
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
VALIDATOR = ROOT / "scripts/validate-ebpf-scheduler-runqueue-qualification.py"
validator = runpy.run_path(str(VALIDATOR))
validate_report = validator["validate_report"]
ATTACH_SCHEDULER_PAIR = 3
BOOT_ID = 0x4553_4F50_5251_5545
LATENCY_THRESHOLD_NS = 5_000_000
BLOCKER_HOLD_NS = 25_000_000
DEGRADED_INCIDENT_FAULT = 0x4542_2001


def qualified_report():
    target_tid = 4321
    cpu_a = 2
    cpu_b = 7
    duration = 30_000_000
    timestamp = 130_000_000
    evidence_id = timestamp
    return {
        "schema_version": 1,
        "status": "qualified",
        "target_tid": target_tid,
        "cpu_a": cpu_a,
        "cpu_b": cpu_b,
        "target_sleep_confirmed": 1,
        "fifo_policy": 1,
        "fifo_priority_min": 1,
        "fifo_priority_max": 99,
        "fifo_priority": 1,
        "scheduler_latency_threshold_ns": LATENCY_THRESHOLD_NS,
        "blocker_hold_ns": BLOCKER_HOLD_NS,
        "futex_wake_count": 1,
        "target_completed_during_hold": 0,
        "runtime_attach_mask": ATTACH_SCHEDULER_PAIR,
        "required_attach_mask": ATTACH_SCHEDULER_PAIR,
        "initial_observation_boot_id": BOOT_ID,
        "initial_observation_agent_epoch": 1,
        "initial_observation_state": 0,
        "initial_observation_attach_mask": ATTACH_SCHEDULER_PAIR,
        "initial_observation_lost_event_count": 0,
        "initial_observation_incident_count": 0,
        "initial_observation_fault_code": 0,
        "initial_observation_heartbeat_seq": 1,
        "baseline_emitted_events": 0,
        "baseline_lost_events": 0,
        "baseline_wakeups": 0,
        "baseline_scheduler_stalls": 0,
        "baseline_scheduler_migrations": 0,
        "records_seen": 1,
        "incidents_emitted": 1,
        "malformed_records": 0,
        "evidence_rejected": 0,
        "newly_reported_lost_events": 0,
        "emitted_events": 1,
        "lost_events": 0,
        "wakeups": 1,
        "scheduler_stalls": 1,
        "scheduler_migrations": 0,
        "dropped_incidents": 0,
        "incident_id": 1,
        "incident_boot_id": BOOT_ID,
        "incident_agent_epoch": 1,
        "incident_code": 0,
        "incident_severity": 2,
        "recommended_action": 2,
        "confidence_percent": 70,
        "incident_first_seen_ns": timestamp,
        "incident_last_seen_ns": timestamp,
        "pid": 0,
        "tid": target_tid,
        "cpu": cpu_a,
        "irq": 0,
        "netdev_ifindex": 0,
        "cycle_first": 42,
        "cycle_last": 42,
        "transition_seq": 9,
        "incident_observed_value": duration,
        "incident_threshold": LATENCY_THRESHOLD_NS,
        "incident_evidence_count": 1,
        "incident_event_count": 1,
        "incident_lost_events": 0,
        "evidence_id": evidence_id,
        "evidence_boot_id": BOOT_ID,
        "evidence_agent_epoch": 1,
        "evidence_timestamp_ns": timestamp,
        "evidence_domain": 0,
        "evidence_kind": 0,
        "evidence_severity": 2,
        "evidence_pid": 0,
        "evidence_tid": target_tid,
        "evidence_cpu": cpu_a,
        "evidence_irq": 0,
        "evidence_netdev_ifindex": 0,
        "evidence_cycle_seq": 42,
        "evidence_transition_seq": 9,
        "evidence_observed_value": duration,
        "evidence_threshold": LATENCY_THRESHOLD_NS,
        "evidence_duration_ns": duration,
        "evidence_event_count": 1,
        "evidence_detail": 0,
        "observation_boot_id": BOOT_ID,
        "observation_agent_epoch": 1,
        "observation_state": 1,
        "observation_attach_mask": ATTACH_SCHEDULER_PAIR,
        "observation_lost_event_count": 0,
        "observation_incident_count": 1,
        "observation_fault_code": DEGRADED_INCIDENT_FAULT,
        "observation_heartbeat_seq": 2,
    }


class SchedulerRunqueueQualificationTests(unittest.TestCase):
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

        for key in ("schema_version", "target_tid", "evidence_duration_ns"):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = True
                self.assert_rejected(report, f"{key} must be an integer")

    def test_cpu_tid_attach_and_sleep_prerequisites_are_fail_closed(self):
        for key, value, reason in (
            ("runtime_attach_mask", 1, "runtime_attach_mask must be 3"),
            ("required_attach_mask", 2, "required_attach_mask must be 3"),
            ("target_tid", 0, "target_tid must be"),
            ("cpu_b", 2, "cpu_a and cpu_b must be distinct"),
            ("cpu_a", 0x1_0000, "qualification CPUs must fit in u16"),
            ("target_sleep_confirmed", 0, "target_sleep_confirmed must be 1"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_fifo_and_futex_contract_is_exact(self):
        for key, value, reason in (
            ("fifo_policy", 0, "fifo_policy must be 1"),
            ("fifo_priority_min", 0, "FIFO priority range"),
            ("fifo_priority", 2, "fifo_priority must equal"),
            ("futex_wake_count", 0, "futex_wake_count must be 1"),
            ("futex_wake_count", 2, "futex_wake_count must be 1"),
            ("target_completed_during_hold", 1, "must be 0"),
        ):
            with self.subTest(key=key, value=value):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_threshold_and_hold_constants_are_exact(self):
        for key, value, reason in (
            (
                "scheduler_latency_threshold_ns",
                LATENCY_THRESHOLD_NS + 1,
                "scheduler_latency_threshold_ns must be 5000000",
            ),
            ("blocker_hold_ns", BLOCKER_HOLD_NS - 1, "blocker_hold_ns must be 25000000"),
            ("incident_threshold", LATENCY_THRESHOLD_NS + 1, "incident_threshold must be 5000000"),
            ("evidence_threshold", LATENCY_THRESHOLD_NS + 1, "evidence_threshold must be 5000000"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_baseline_final_counts_and_loss_fields_are_exact(self):
        for key, value, reason in (
            ("baseline_wakeups", 1, "baseline_wakeups must be 0"),
            ("records_seen", 2, "records_seen must be 1"),
            ("incidents_emitted", 0, "incidents_emitted must be 1"),
            ("wakeups", 2, "wakeups must be 1"),
            ("scheduler_stalls", 0, "scheduler_stalls must be 1"),
            ("scheduler_migrations", 1, "scheduler_migrations must be 0"),
            ("lost_events", 1, "lost_events must be 0"),
            ("dropped_incidents", 1, "dropped_incidents must be 0"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_identity_cycle_and_classification_are_exact(self):
        for key, value, reason in (
            ("tid", 4322, "tid must be 4321"),
            ("evidence_cpu", 7, "evidence_cpu must be 2"),
            ("incident_severity", 1, "incident_severity must be 2"),
            ("recommended_action", 1, "recommended_action must be 2"),
            ("confidence_percent", 60, "confidence_percent must be 70"),
            ("evidence_kind", 10, "evidence_kind must be 0"),
            ("evidence_cycle_seq", 41, "evidence_cycle_seq must be 42"),
            ("transition_seq", 8, "transition_seq must be 9"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_duration_and_timestamps_are_consistent_and_bounded(self):
        for key, value, reason in (
            ("evidence_duration_ns", BLOCKER_HOLD_NS - 1, "cover the blocker hold"),
            ("evidence_duration_ns", 1_000_000_000, "cover the blocker hold"),
            ("evidence_observed_value", 30_000_001, "must equal evidence_duration_ns"),
            ("incident_first_seen_ns", 129_999_999, "must equal evidence_timestamp_ns"),
            ("evidence_id", 129_999_999, "must equal evidence_timestamp_ns"),
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
