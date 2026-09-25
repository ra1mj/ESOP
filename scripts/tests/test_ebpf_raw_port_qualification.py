"""Regression tests for raw-port eBPF runtime qualification evidence."""

import copy
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
VALIDATOR = ROOT / "scripts/validate-ebpf-raw-port-qualification.py"
validate_report = runpy.run_path(str(VALIDATOR))["validate_report"]
ATTACH_RAW_PORT = (1 << 16) | (1 << 17)


def qualified_report():
    return {
        "schema_version": 1,
        "status": "qualified",
        "threshold_ns": 5_000_000,
        "injected_delay_ns": 25_000_000,
        "runtime_attach_mask": ATTACH_RAW_PORT,
        "required_attach_mask": ATTACH_RAW_PORT,
        "records_seen": 1,
        "incidents_emitted": 1,
        "malformed_records": 0,
        "evidence_rejected": 0,
        "newly_reported_lost_events": 0,
        "lost_events": 0,
        "raw_port_probe_begins": 1,
        "raw_port_probe_completions": 1,
        "raw_port_stalls": 1,
        "raw_port_probe_mismatches": 0,
        "incident_code": 9,
        "incident_severity": 2,
        "recommended_action": 2,
        "confidence_percent": 80,
        "pid": 1234,
        "tid": 1234,
        "netdev_ifindex": 7,
        "cycle_seq": 1,
        "detail": 1,
        "incident_evidence_count": 1,
        "event_count": 1,
        "evidence_id": 123_456,
        "observed_value": 25_000_000,
        "evidence_threshold": 5_000_000,
        "duration_ns": 25_000_000,
    }


class RawPortQualificationTests(unittest.TestCase):
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
        del report["duration_ns"]
        self.assert_rejected(report, "missing keys")

        report = qualified_report()
        report["unexpected"] = 1
        self.assert_rejected(report, "unknown keys")

        for key in ("schema_version", "records_seen", "duration_ns"):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = True
                self.assert_rejected(report, f"{key} must be an integer")

    def test_attach_and_poll_contract_is_fail_closed(self):
        for key, value, reason in (
            ("runtime_attach_mask", 1 << 16, "complete raw-port pair"),
            ("required_attach_mask", 1 << 17, "complete raw-port pair"),
            ("records_seen", 0, "records_seen must be positive"),
            ("incidents_emitted", 0, "incidents_emitted must be 1"),
            ("malformed_records", 1, "malformed_records must be zero"),
            ("evidence_rejected", 1, "evidence_rejected must be zero"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_timing_mismatch_and_loss_never_qualify(self):
        report = qualified_report()
        report["duration_ns"] = report["evidence_threshold"]
        report["observed_value"] = report["duration_ns"]
        self.assert_rejected(report, "strictly exceed")

        report = qualified_report()
        report["observed_value"] += 1
        self.assert_rejected(report, "observed_value must equal duration_ns")

        for key in (
            "newly_reported_lost_events",
            "lost_events",
            "raw_port_probe_mismatches",
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = 1
                self.assert_rejected(report, f"{key} must be zero")

    def test_wrong_incident_or_evidence_semantics_are_rejected(self):
        for key, value, reason in (
            ("incident_code", 7, "incident_code must be 9"),
            ("incident_severity", 1, "incident_severity must be 2"),
            ("recommended_action", 1, "recommended_action must be 2"),
            ("confidence_percent", 75, "confidence_percent must be 80"),
            ("detail", 0, "detail must be 1"),
            ("event_count", 0, "event_count must be 1"),
            ("evidence_id", 0, "evidence_id must be nonzero"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)


if __name__ == "__main__":
    unittest.main()
