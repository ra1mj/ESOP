"""Regression tests for virtual-veth network-drop qualification evidence."""

import copy
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
VALIDATOR = ROOT / "scripts/validate-ebpf-network-drop-qualification.py"
validator = runpy.run_path(str(VALIDATOR))
validate_report = validator["validate_report"]
ATTACH_NETWORK_DROP = 1 << 5
BOOT_ID = 0x4553_4F50_4E44_5250
DEGRADED_INCIDENT_FAULT = 0x4542_2001


def qualified_report():
    cpu = 7
    rx_ifindex = 102
    timestamp = 123_456_789
    return {
        "schema_version": 1,
        "status": "qualified",
        "tx_interface": "esotx1a2b",
        "rx_interface": "esorx1a2b",
        "tx_ifindex": 101,
        "rx_ifindex": rx_ifindex,
        "tx_mac": "02:00:00:00:00:01",
        "rx_mac": "02:00:00:00:00:02",
        "tx_mtu": 1500,
        "rx_mtu": 1500,
        "tx_link_up": 1,
        "rx_link_up": 1,
        "tx_ipv6_sysctl_present": 1,
        "rx_ipv6_sysctl_present": 1,
        "tx_ipv6_disabled": 1,
        "rx_ipv6_disabled": 1,
        "target_cpu": cpu,
        "packet_socket_protocol": 0,
        "frame_bytes": 64,
        "network_protocol": 0x88A4,
        "control_protocol": 0x88A5,
        "wrong_protocol_frames": 1,
        "wrong_protocol_bytes": 64,
        "reverse_control_frames": 1,
        "reverse_control_bytes": 64,
        "formal_frames": 4,
        "formal_bytes": 256,
        "tx_drops_before_controls": 0,
        "tx_drops_after_controls": 1,
        "control_tx_drop_delta": 1,
        "rx_drops_before_controls": 0,
        "rx_drops_after_controls": 1,
        "control_rx_drop_delta": 1,
        "baseline_tx_drops": 1,
        "final_tx_drops": 1,
        "formal_tx_drop_delta": 0,
        "baseline_rx_drops": 1,
        "final_rx_drops": 5,
        "formal_rx_drop_delta": 4,
        "network_drop_threshold": 4,
        "network_drop_window_ns": 1_000_000_000,
        "runtime_attach_mask": ATTACH_NETWORK_DROP,
        "required_attach_mask": ATTACH_NETWORK_DROP,
        "initial_observation_boot_id": BOOT_ID,
        "initial_observation_agent_epoch": 1,
        "initial_observation_state": 0,
        "initial_observation_attach_mask": ATTACH_NETWORK_DROP,
        "initial_observation_lost_event_count": 0,
        "initial_observation_incident_count": 0,
        "initial_observation_fault_code": 0,
        "initial_observation_heartbeat_seq": 1,
        "control_records_seen": 0,
        "control_incidents_emitted": 0,
        "control_malformed_records": 0,
        "control_evidence_rejected": 0,
        "control_newly_reported_lost_events": 0,
        "control_emitted_events": 0,
        "control_lost_events": 0,
        "control_network_drops": 0,
        "control_network_unattributed": 0,
        "control_network_threshold_events": 0,
        "baseline_emitted_events": 0,
        "baseline_lost_events": 0,
        "baseline_network_drops": 0,
        "baseline_network_unattributed": 0,
        "baseline_network_threshold_events": 0,
        "records_seen": 1,
        "incidents_emitted": 1,
        "malformed_records": 0,
        "evidence_rejected": 0,
        "newly_reported_lost_events": 0,
        "emitted_events": 1,
        "lost_events": 0,
        "network_drops": 4,
        "network_unattributed": 0,
        "network_threshold_events": 1,
        "dropped_incidents": 0,
        "incident_id": 1,
        "incident_boot_id": BOOT_ID,
        "incident_agent_epoch": 1,
        "incident_code": 2,
        "incident_severity": 2,
        "recommended_action": 2,
        "confidence_percent": 75,
        "incident_first_seen_ns": timestamp,
        "incident_last_seen_ns": timestamp,
        "pid": 0,
        "tid": 0,
        "cpu": cpu,
        "irq": 0,
        "netdev_ifindex": rx_ifindex,
        "cycle_first": 42,
        "cycle_last": 42,
        "transition_seq": 9,
        "incident_observed_value": 4,
        "incident_threshold": 4,
        "incident_evidence_count": 1,
        "incident_event_count": 4,
        "incident_lost_events": 0,
        "evidence_id": timestamp,
        "evidence_boot_id": BOOT_ID,
        "evidence_agent_epoch": 1,
        "evidence_timestamp_ns": timestamp,
        "evidence_domain": 2,
        "evidence_kind": 2,
        "evidence_severity": 2,
        "evidence_pid": 0,
        "evidence_tid": 0,
        "evidence_cpu": cpu,
        "evidence_irq": 0,
        "evidence_netdev_ifindex": rx_ifindex,
        "evidence_cycle_seq": 42,
        "evidence_transition_seq": 9,
        "evidence_observed_value": 4,
        "evidence_threshold": 4,
        "evidence_duration_ns": 25_000,
        "evidence_event_count": 4,
        "evidence_detail": 2,
        "observation_boot_id": BOOT_ID,
        "observation_agent_epoch": 1,
        "observation_state": 1,
        "observation_attach_mask": ATTACH_NETWORK_DROP,
        "observation_lost_event_count": 0,
        "observation_incident_count": 1,
        "observation_fault_code": DEGRADED_INCIDENT_FAULT,
        "observation_heartbeat_seq": 2,
        "runtime_detached": 1,
        "packet_socket_closed": 1,
        "links_removed": 1,
        "affinity_restored": 1,
        "cleanup_succeeded": 1,
    }


class NetworkDropQualificationTests(unittest.TestCase):
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

    def test_schema_interfaces_and_packet_contract_are_closed(self):
        report = qualified_report()
        del report["cleanup_succeeded"]
        self.assert_rejected(report, "missing keys")

        report = qualified_report()
        report["unexpected"] = 1
        self.assert_rejected(report, "unknown keys")

        for key, value, reason in (
            ("target_cpu", True, "target_cpu must be an integer"),
            ("tx_interface", "eth0", "generated ESOP veth name"),
            ("rx_ifindex", 101, "veth ifindexes must be distinct"),
            ("tx_mac", "ff:ff:ff:ff:ff:ff", "nonzero unicast"),
            ("packet_socket_protocol", 0x88A4, "must be 0"),
            ("formal_bytes", 255, "must be 256"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_controls_and_drop_counters_are_independently_proven(self):
        for key, value, reason in (
            ("control_tx_drop_delta", 0, "must match the counter difference"),
            ("control_records_seen", 1, "control_records_seen must be 0"),
            ("baseline_rx_drops", 0, "baseline_rx_drops must be 1"),
            ("formal_tx_drop_delta", 1, "must match the counter difference"),
            ("formal_rx_drop_delta", 3, "must match the counter difference"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

        report = qualified_report()
        report["final_rx_drops"] = 4
        report["formal_rx_drop_delta"] = 3
        self.assert_rejected(report, "cover every formal frame")

    def test_attach_statistics_and_loss_are_exact(self):
        for key, value, reason in (
            ("runtime_attach_mask", 0, "runtime_attach_mask must be 32"),
            ("records_seen", 2, "records_seen must be 1"),
            ("network_drops", 3, "network_drops must be 4"),
            ("network_unattributed", 1, "network_unattributed must be 0"),
            ("network_threshold_events", 2, "must be 1"),
            ("lost_events", 1, "lost_events must be 0"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_incident_evidence_cycle_and_reason_are_exact(self):
        for key, value, reason in (
            ("incident_code", 3, "incident_code must be 2"),
            ("recommended_action", 1, "recommended_action must be 2"),
            ("confidence_percent", 74, "confidence_percent must be 75"),
            ("cpu", 8, "cpu must be 7"),
            ("netdev_ifindex", 101, "netdev_ifindex must be 102"),
            ("cycle_first", 41, "cycle_first must be 42"),
            ("evidence_duration_ns", 0, "must be positive"),
            ("evidence_detail", 0, "bounded nonzero kernel drop reason"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)

    def test_health_and_cleanup_must_complete(self):
        for key, value, reason in (
            ("initial_observation_state", 1, "must be 0"),
            ("observation_state", 0, "must be 1"),
            ("observation_fault_code", 0, "observation_fault_code must be"),
            ("runtime_detached", 0, "runtime_detached must be 1"),
            ("links_removed", 0, "links_removed must be 1"),
            ("affinity_restored", 0, "affinity_restored must be 1"),
        ):
            with self.subTest(key=key):
                report = qualified_report()
                report[key] = value
                self.assert_rejected(report, reason)


if __name__ == "__main__":
    unittest.main()
