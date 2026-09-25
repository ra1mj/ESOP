"""Regression tests for controlled-loopback softirq qualification evidence."""

import copy
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
VALIDATOR = ROOT / "scripts/validate-ebpf-softirq-qualification.py"
validator = runpy.run_path(str(VALIDATOR))
validate_report = validator["validate_report"]
ATTACH_SOFTIRQ = 768
BOOT_ID = 0x4553_4F50_534F_4654
PAYLOAD_BYTES = 64_800
DEGRADED_INCIDENT_FAULT = 0x4542_2001
INTERRUPT_GATE_CLOSED_VECTOR = (1 << 32) - 2


def qualified_report():
    fixture_pid = 4_200
    fixture_tid = 4_201
    target_cpu = 7
    calibration_duration = 80_000
    formal_threshold = calibration_duration // 8
    duration = 72_000
    timestamp = 130_000_000
    report = {
        "schema_version": 1,
        "status": "qualified",
        "fixture_pid": fixture_pid,
        "fixture_tid": fixture_tid,
        "target_cpu": target_cpu,
        "quiet_net_rx_before": 100,
        "quiet_net_rx_after": 100,
        "quiet_net_rx_delta": 0,
        "softirq_vector": 3,
        "interrupt_gate_closed_vector": INTERRUPT_GATE_CLOSED_VECTOR,
        "udp_segment_bytes": 1_200,
        "udp_segment_count": 54,
        "udp_payload_bytes": PAYLOAD_BYTES,
        "calibration_divisor": 8,
        "calibration_threshold_ns": 1,
        "calibration_duration_ns": calibration_duration,
        "calibration_sent_bytes": PAYLOAD_BYTES,
        "calibration_received_datagrams": 54,
        "calibration_net_rx_before": 101,
        "calibration_net_rx_after": 102,
        "calibration_net_rx_delta": 1,
        "calibration_filter_vector_before": INTERRUPT_GATE_CLOSED_VECTOR,
        "calibration_filter_vector_active": 3,
        "calibration_filter_vector_after": INTERRUPT_GATE_CLOSED_VECTOR,
        "calibration_runtime_attach_mask": ATTACH_SOFTIRQ,
        "calibration_required_attach_mask": ATTACH_SOFTIRQ,
        "calibration_baseline_emitted_events": 0,
        "calibration_baseline_lost_events": 0,
        "calibration_baseline_irq_samples": 0,
        "calibration_baseline_irq_overruns": 0,
        "calibration_baseline_softirq_samples": 0,
        "calibration_baseline_softirq_overruns": 0,
        "calibration_records_seen": 1,
        "calibration_incidents_emitted": 1,
        "calibration_malformed_records": 0,
        "calibration_evidence_rejected": 0,
        "calibration_newly_reported_lost_events": 0,
        "calibration_emitted_events": 1,
        "calibration_lost_events": 0,
        "calibration_irq_samples": 0,
        "calibration_irq_overruns": 0,
        "calibration_softirq_samples": 1,
        "calibration_softirq_overruns": 1,
        "calibration_dropped_incidents": 0,
        "formal_threshold_ns": formal_threshold,
        "formal_sent_bytes": PAYLOAD_BYTES,
        "formal_received_datagrams": 54,
        "formal_net_rx_before": 103,
        "formal_net_rx_after": 104,
        "formal_net_rx_delta": 1,
        "filter_vector_before": INTERRUPT_GATE_CLOSED_VECTOR,
        "filter_vector_active": 3,
        "filter_vector_after": INTERRUPT_GATE_CLOSED_VECTOR,
        "runtime_attach_mask": ATTACH_SOFTIRQ,
        "required_attach_mask": ATTACH_SOFTIRQ,
        "interrupt_filter_cpu": target_cpu,
        "interrupt_filter_vector": 3,
        "initial_observation_boot_id": BOOT_ID,
        "initial_observation_agent_epoch": 1,
        "initial_observation_state": 0,
        "initial_observation_attach_mask": ATTACH_SOFTIRQ,
        "initial_observation_lost_event_count": 0,
        "initial_observation_incident_count": 0,
        "initial_observation_fault_code": 0,
        "initial_observation_heartbeat_seq": 1,
        "baseline_emitted_events": 0,
        "baseline_lost_events": 0,
        "baseline_irq_samples": 0,
        "baseline_irq_overruns": 0,
        "baseline_softirq_samples": 0,
        "baseline_softirq_overruns": 0,
        "records_seen": 1,
        "incidents_emitted": 1,
        "malformed_records": 0,
        "evidence_rejected": 0,
        "newly_reported_lost_events": 0,
        "emitted_events": 1,
        "lost_events": 0,
        "irq_samples": 0,
        "irq_overruns": 0,
        "softirq_samples": 1,
        "softirq_overruns": 1,
        "dropped_incidents": 0,
        "incident_id": 1,
        "incident_boot_id": BOOT_ID,
        "incident_agent_epoch": 1,
        "incident_code": 1,
        "incident_severity": 2,
        "recommended_action": 2,
        "confidence_percent": 70,
        "incident_first_seen_ns": timestamp,
        "incident_last_seen_ns": timestamp,
        "pid": fixture_pid,
        "tid": fixture_tid,
        "cpu": target_cpu,
        "irq": 3,
        "netdev_ifindex": 0,
        "cycle_first": 42,
        "cycle_last": 42,
        "transition_seq": 9,
        "incident_observed_value": duration,
        "incident_threshold": formal_threshold,
        "incident_evidence_count": 1,
        "incident_event_count": 1,
        "incident_lost_events": 0,
        "evidence_id": timestamp,
        "evidence_boot_id": BOOT_ID,
        "evidence_agent_epoch": 1,
        "evidence_timestamp_ns": timestamp,
        "evidence_domain": 1,
        "evidence_kind": 9,
        "evidence_severity": 2,
        "evidence_pid": fixture_pid,
        "evidence_tid": fixture_tid,
        "evidence_cpu": target_cpu,
        "evidence_irq": 3,
        "evidence_netdev_ifindex": 0,
        "evidence_cycle_seq": 42,
        "evidence_transition_seq": 9,
        "evidence_observed_value": duration,
        "evidence_threshold": formal_threshold,
        "evidence_duration_ns": duration,
        "evidence_event_count": 1,
        "evidence_detail": 0,
        "observation_boot_id": BOOT_ID,
        "observation_agent_epoch": 1,
        "observation_state": 1,
        "observation_attach_mask": ATTACH_SOFTIRQ,
        "observation_lost_event_count": 0,
        "observation_incident_count": 1,
        "observation_fault_code": DEGRADED_INCIDENT_FAULT,
        "observation_heartbeat_seq": 2,
    }
    return report


class SoftirqQualificationTests(unittest.TestCase):
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

    def test_attach_filter_and_gso_contract_is_exact(self):
        for key, value, reason in (
            ("runtime_attach_mask", 256, "runtime_attach_mask must be 768"),
            ("calibration_required_attach_mask", 512, "must be 768"),
            ("target_cpu", 0xFFFF, "target_cpu must fit"),
            ("interrupt_filter_cpu", 8, "interrupt_filter_cpu must be 7"),
            ("interrupt_filter_vector", 2, "must be 3"),
            ("udp_segment_bytes", 1_199, "must be 1200"),
            ("udp_segment_count", 53, "must be 54"),
            ("formal_received_datagrams", 53, "must be 54"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_interrupt_gate_sequence_is_exact(self):
        for key, value, reason in (
            ("interrupt_gate_closed_vector", 3, "must be 4294967294"),
            ("calibration_filter_vector_before", 3, "must be 4294967294"),
            (
                "calibration_filter_vector_active",
                INTERRUPT_GATE_CLOSED_VECTOR,
                "must be 3",
            ),
            ("calibration_filter_vector_after", 3, "must be 4294967294"),
            ("filter_vector_before", 3, "must be 4294967294"),
            ("filter_vector_active", INTERRUPT_GATE_CLOSED_VECTOR, "must be 3"),
            ("filter_vector_after", 3, "must be 4294967294"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_counter_deltas_and_calibration_formula_are_exact(self):
        for key, value, reason in (
            ("calibration_net_rx_after", 103, "match their delta"),
            ("formal_net_rx_delta", 2, "match their delta"),
            ("calibration_threshold_ns", 2, "must be 1"),
            ("calibration_duration_ns", 1, "strictly above"),
            ("formal_threshold_ns", 9_999, "formal_threshold_ns must be 10000"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_baselines_counts_and_loss_are_exact(self):
        for key, value, reason in (
            ("calibration_baseline_softirq_samples", 1, "must be 0"),
            ("calibration_records_seen", 2, "must be 1"),
            ("calibration_irq_samples", 1, "must be 0"),
            ("baseline_emitted_events", 1, "must be 0"),
            ("records_seen", 2, "must be 1"),
            ("softirq_samples", 2, "must be 1"),
            ("irq_overruns", 1, "must be 0"),
            ("lost_events", 1, "must be 0"),
            ("dropped_incidents", 1, "must be 0"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_identity_classification_cycle_and_timing_are_exact(self):
        for key, value, reason in (
            ("pid", 4_199, "pid must be 4200"),
            ("evidence_tid", 4_202, "evidence_tid must be 4201"),
            ("evidence_cpu", 8, "evidence_cpu must be 7"),
            ("incident_code", 0, "incident_code must be 1"),
            ("evidence_kind", 1, "evidence_kind must be 9"),
            ("cycle_first", 41, "cycle_first must be 42"),
            ("transition_seq", 8, "transition_seq must be 9"),
            ("incident_first_seen_ns", 129_999_999, "must equal evidence_timestamp_ns"),
            ("evidence_id", 129_999_999, "must equal evidence_timestamp_ns"),
            ("evidence_duration_ns", 10_000, "strictly above formal_threshold_ns"),
            ("incident_observed_value", 72_001, "must equal evidence_duration_ns"),
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
