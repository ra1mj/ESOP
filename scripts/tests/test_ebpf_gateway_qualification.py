"""Regression tests for gateway eBPF runtime qualification evidence."""

import copy
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
VALIDATOR = ROOT / "scripts/validate-ebpf-gateway-qualification.py"
validator = runpy.run_path(str(VALIDATOR))
validate_report = validator["validate_report"]
ATTACH_GATEWAY = validator["ATTACH_GATEWAY"]


def qualified_report():
    return {
        "schema_version": 1,
        "status": "qualified",
        "threshold_ns": 5_000_000,
        "injected_delay_ns": 25_000_000,
        "runtime_attach_mask": ATTACH_GATEWAY,
        "required_attach_mask": ATTACH_GATEWAY,
        "records_seen": 2,
        "incidents_emitted": 2,
        "malformed_records": 0,
        "evidence_rejected": 0,
        "newly_reported_lost_events": 0,
        "lost_events": 0,
        "gateway_probe_begins": 2,
        "gateway_probe_completions": 2,
        "gateway_stalls": 2,
        "gateway_probe_mismatches": 0,
        "dropped_incidents": 0,
        "incident_code": 7,
        "incident_severity": 2,
        "recommended_action": 2,
        "confidence_percent": 75,
        "pid": 1234,
        "tid": 1234,
        "cycle_first": 1,
        "cycle_last": 1,
        "incident_evidence_count": 2,
        "incident_event_count": 2,
        "incident_lost_events": 0,
        "publish_request_id": 101,
        "publish_domain": 7,
        "publish_kind": 7,
        "publish_severity": 2,
        "publish_detail": 2,
        "publish_pid": 1234,
        "publish_tid": 1234,
        "publish_cycle_seq": 1,
        "publish_event_count": 1,
        "publish_observed_value": 25_000_001,
        "publish_evidence_threshold": 5_000_000,
        "publish_duration_ns": 25_000_001,
        "callback_request_id": 202,
        "callback_domain": 7,
        "callback_kind": 7,
        "callback_severity": 2,
        "callback_detail": 3,
        "callback_pid": 1234,
        "callback_tid": 1234,
        "callback_cycle_seq": 1,
        "callback_event_count": 1,
        "callback_observed_value": 25_000_002,
        "callback_evidence_threshold": 5_000_000,
        "callback_duration_ns": 25_000_002,
    }


class GatewayQualificationTests(unittest.TestCase):
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
        del report["callback_duration_ns"]
        self.assert_rejected(report, "missing keys")

        report = qualified_report()
        report["unexpected"] = 1
        self.assert_rejected(report, "unknown keys")

        for key in ("schema_version", "records_seen", "publish_duration_ns"):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = True
                self.assert_rejected(report, f"{key} must be an integer")

    def test_attach_poll_and_merge_contract_is_fail_closed(self):
        for key, value, reason in (
            ("runtime_attach_mask", 1 << 12, "both complete gateway pairs"),
            ("required_attach_mask", 1 << 14, "both complete gateway pairs"),
            ("records_seen", 1, "records_seen must be 2"),
            ("incidents_emitted", 1, "incidents_emitted must be 2"),
            ("incident_evidence_count", 1, "incident_evidence_count must be 2"),
            ("incident_event_count", 1, "incident_event_count must be 2"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_timing_mismatch_and_loss_never_qualify(self):
        for prefix in ("publish", "callback"):
            with self.subTest(prefix=prefix, failure="threshold"):
                report = qualified_report()
                report[f"{prefix}_duration_ns"] = report[
                    f"{prefix}_evidence_threshold"
                ]
                report[f"{prefix}_observed_value"] = report[f"{prefix}_duration_ns"]
                self.assert_rejected(report, "strictly exceed")

            with self.subTest(prefix=prefix, failure="duration"):
                report = qualified_report()
                report[f"{prefix}_observed_value"] += 1
                self.assert_rejected(report, "observed_value must equal")

        for key in (
            "newly_reported_lost_events",
            "lost_events",
            "gateway_probe_mismatches",
            "dropped_incidents",
            "incident_lost_events",
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = 1
                self.assert_rejected(report, f"{key} must be zero")

    def test_wrong_incident_route_and_identity_are_rejected(self):
        for key, value, reason in (
            ("incident_code", 9, "incident_code must be 7"),
            ("incident_severity", 1, "incident_severity must be 2"),
            ("recommended_action", 1, "recommended_action must be 2"),
            ("confidence_percent", 80, "confidence_percent must be 75"),
            ("publish_domain", 5, "publish_domain must be 7"),
            ("publish_kind", 11, "publish_kind must be 7"),
            ("callback_severity", 1, "callback_severity must be 2"),
            ("publish_detail", 3, "publish_detail must be 2"),
            ("callback_detail", 4, "callback_detail must be 3"),
            ("publish_pid", 999, "publish_pid must equal pid"),
            ("callback_tid", 999, "callback_tid must equal tid"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

        report = qualified_report()
        report["callback_request_id"] = report["publish_request_id"]
        self.assert_rejected(report, "callback_request_id must be 202")


if __name__ == "__main__":
    unittest.main()
