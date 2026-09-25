"""Regression tests for memcg OOM eBPF runtime qualification evidence."""

import copy
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
VALIDATOR = ROOT / "scripts/validate-ebpf-oom-qualification.py"
validator = runpy.run_path(str(VALIDATOR))
validate_report = validator["validate_report"]
ATTACH_OOM_KILL = 1 << 4
BOOT_ID = 0x4553_4F50_4F4F_4D51
LATCHED_INCIDENT_FAULT = 0x4542_2002


def qualified_report():
    pid = 4321
    cpu = 7
    timestamp = 123_456_789
    page_size = 4096
    mapping_bytes = 128 * 1024 * 1024
    return {
        "schema_version": 1,
        "status": "qualified",
        "cgroup_version": 2,
        "memory_controller_enabled": 1,
        "memory_limit_bytes": 32 * 1024 * 1024,
        "mapping_bytes": mapping_bytes,
        "page_size": page_size,
        "mapping_pages": mapping_bytes // page_size,
        "swap_limit_present": 1,
        "swap_limit_bytes": 0,
        "oom_group": 0,
        "tracked_child_pid": pid,
        "initial_member_count": 1,
        "final_member_count": 0,
        "child_signal": 9,
        "cleanup_succeeded": 1,
        "baseline_memory_max_events": 0,
        "baseline_memory_oom_events": 0,
        "baseline_memory_oom_kill_events": 0,
        "baseline_memory_oom_group_kill_events": 0,
        "memory_max_events": 513,
        "memory_oom_events": 1,
        "memory_oom_kill_events": 1,
        "memory_oom_group_kill_events": 0,
        "memory_max_delta": 513,
        "memory_oom_delta": 1,
        "memory_oom_kill_delta": 1,
        "memory_oom_group_kill_delta": 0,
        "runtime_attach_mask": ATTACH_OOM_KILL,
        "required_attach_mask": ATTACH_OOM_KILL,
        "initial_observation_boot_id": BOOT_ID,
        "initial_observation_agent_epoch": 1,
        "initial_observation_state": 0,
        "initial_observation_attach_mask": ATTACH_OOM_KILL,
        "initial_observation_lost_event_count": 0,
        "initial_observation_incident_count": 0,
        "initial_observation_fault_code": 0,
        "initial_observation_heartbeat_seq": 1,
        "baseline_emitted_events": 0,
        "baseline_lost_events": 0,
        "baseline_process_exits": 0,
        "baseline_oom_events": 0,
        "records_seen": 1,
        "incidents_emitted": 1,
        "malformed_records": 0,
        "evidence_rejected": 0,
        "newly_reported_lost_events": 0,
        "emitted_events": 1,
        "lost_events": 0,
        "process_exits": 0,
        "oom_events": 1,
        "dropped_incidents": 0,
        "incident_id": 1,
        "incident_boot_id": BOOT_ID,
        "incident_agent_epoch": 1,
        "incident_code": 4,
        "incident_severity": 3,
        "recommended_action": 3,
        "confidence_percent": 100,
        "incident_first_seen_ns": timestamp,
        "incident_last_seen_ns": timestamp,
        "pid": pid,
        "tid": pid,
        "cpu": cpu,
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
        "evidence_timestamp_ns": timestamp,
        "evidence_domain": 3,
        "evidence_kind": 4,
        "evidence_severity": 3,
        "evidence_pid": pid,
        "evidence_tid": pid,
        "evidence_cpu": cpu,
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
        "observation_attach_mask": ATTACH_OOM_KILL,
        "observation_lost_event_count": 0,
        "observation_incident_count": 1,
        "observation_fault_code": LATCHED_INCIDENT_FAULT,
        "observation_heartbeat_seq": 2,
    }


class OomQualificationTests(unittest.TestCase):
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
        del report["cleanup_succeeded"]
        self.assert_rejected(report, "missing keys")

        report = qualified_report()
        report["unexpected"] = 1
        self.assert_rejected(report, "unknown keys")

        for key in ("schema_version", "memory_oom_delta", "evidence_id"):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = True
                self.assert_rejected(report, f"{key} must be an integer")

    def test_cgroup_geometry_membership_and_cleanup_are_exact(self):
        for key, value, reason in (
            ("cgroup_version", 1, "cgroup_version must be 2"),
            ("memory_limit_bytes", 1, "memory_limit_bytes must be"),
            ("mapping_bytes", 1024, "mapping_bytes must be"),
            ("page_size", 5000, "page_size must be a bounded power"),
            ("mapping_pages", 1, "mapping_pages must be"),
            ("initial_member_count", 2, "initial_member_count must be 1"),
            ("final_member_count", 1, "final_member_count must be 0"),
            ("child_signal", 15, "child_signal must be 9"),
            ("cleanup_succeeded", 0, "cleanup_succeeded must be 1"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_memcg_counters_require_one_local_kill(self):
        for key, value, reason in (
            ("baseline_memory_oom_events", 1, "must be 0"),
            ("memory_max_delta", 0, "memory_max_delta must be at least 1"),
            ("memory_oom_delta", 0, "memory_oom_delta must be at least 1"),
            ("memory_oom_kill_delta", 2, "memory_oom_kill_delta must be 1"),
            ("memory_oom_group_kill_delta", 1, "must be 0"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                if key.endswith("_delta"):
                    final_key = {
                        "memory_max_delta": "memory_max_events",
                        "memory_oom_delta": "memory_oom_events",
                        "memory_oom_kill_delta": "memory_oom_kill_events",
                        "memory_oom_group_kill_delta": "memory_oom_group_kill_events",
                    }[key]
                    report[final_key] = value
                self.assert_rejected(report, reason)

    def test_attach_statistics_and_identity_are_fail_closed(self):
        for key, value, reason in (
            ("runtime_attach_mask", 0, "runtime_attach_mask must be 16"),
            ("records_seen", 2, "records_seen must be 1"),
            ("oom_events", 0, "oom_events must be 1"),
            ("lost_events", 1, "lost_events must be 0"),
            ("pid", 4322, "pid must be 4321"),
            ("evidence_cpu", 8, "evidence_cpu must be 7"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_incident_and_health_must_preserve_hard_fact_policy(self):
        for key, value, reason in (
            ("incident_code", 5, "incident_code must be 4"),
            ("incident_severity", 2, "incident_severity must be 3"),
            ("recommended_action", 2, "recommended_action must be 3"),
            ("confidence_percent", 99, "confidence_percent must be 100"),
            ("evidence_domain", 4, "evidence_domain must be 3"),
            ("evidence_kind", 5, "evidence_kind must be 4"),
            ("cycle_first", 1, "cycle_first must be 0"),
            ("evidence_duration_ns", 1, "evidence_duration_ns must be 0"),
            ("observation_state", 1, "observation_state must be 2"),
            ("observation_fault_code", 0, "observation_fault_code must be"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)


if __name__ == "__main__":
    unittest.main()
