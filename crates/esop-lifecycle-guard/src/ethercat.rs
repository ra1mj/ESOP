//! Allocation-free projection of verified EtherCAT cycle evidence into MLG facts.

use crate::CyclicQuality;
use esop_ethercat_core::{CycleReport, DcCyclicSync, DomainQuality};

/// Facts owned by the cycle caller, not inferable from EtherCAT RX or DC.
/// Callers must sample each from its actual owner for the same cycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OtherCycleFacts {
    pub platform_ready: bool,
    pub coe_ready: bool,
    pub topology_valid: bool,
    pub drive_ready: bool,
    pub command_current: bool,
    pub supervisor_healthy: bool,
    pub external_safety_clear: bool,
    pub deadline_met: bool,
}

/// Collect after `Domain::finish_receive` for every required, scheduled
/// Domain. `due_domains` must contain *all* Domains required in this cycle;
/// omitted or unscheduled Domains cannot be inferred from a receive report.
/// DC-enabled configurations must pass their actual cyclic sync instance.
/// A no-DC configuration must explicitly waive the DC gate in its policy.
pub fn cyclic_quality_from_ethercat(
    report: CycleReport,
    due_domains: &[DomainQuality],
    dc: &DcCyclicSync,
    other: OtherCycleFacts,
) -> CyclicQuality {
    cyclic_quality_from_domains(report, due_domains.iter(), dc, other)
}

pub(crate) fn cyclic_quality_from_domains<'a>(
    report: CycleReport,
    mut due_domains: impl Iterator<Item = &'a DomainQuality>,
    dc: &DcCyclicSync,
    other: OtherCycleFacts,
) -> CyclicQuality {
    let mut has_due_domain = false;
    let domain_valid = due_domains.all(|domain| {
        has_due_domain = true;
        domain.valid
            && domain.complete
            && domain.expected_wkc != 0
            && domain.actual_wkc == domain.expected_wkc
            && domain.last_valid_cycle == report.cycle
            && domain.input_age_cycles == 0
    }) && has_due_domain;
    let rx_valid = !report.link_down
        && report.received_frames != 0
        && report.parsed_datagrams != 0
        && report.unmatched_datagrams == 0
        && report.corrupt_frames == 0
        && report.wkc_mismatches == 0
        && report.timed_out_datagrams == 0
        && report.consumer_rejections == 0;

    CyclicQuality {
        platform_ready: other.platform_ready,
        coe_ready: other.coe_ready,
        topology_valid: other.topology_valid,
        distributed_clock_locked: !report.link_down
            && dc.sync_count() != 0
            && dc.last_sync_cycle() == report.cycle
            && dc.last_error().is_none()
            && dc.pending_generation().is_none()
            && dc.monitor().is_locked(),
        drive_ready: other.drive_ready,
        domain_valid,
        wkc_valid: rx_valid && domain_valid,
        command_current: other.command_current,
        supervisor_healthy: other.supervisor_healthy,
        external_safety_clear: other.external_safety_clear,
        cycle_within_budget: other.deadline_met && !report.budget_exhausted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use esop_ethercat_core::wire::{Command, DatagramHeader};
    use esop_ethercat_core::{DcCyclicConfig, DcMonitor, RxMatch};

    fn report() -> CycleReport {
        CycleReport {
            cycle: 7,
            received_frames: 1,
            received_bytes: 64,
            parsed_datagrams: 2,
            unmatched_datagrams: 0,
            corrupt_frames: 0,
            wkc_mismatches: 0,
            timed_out_datagrams: 0,
            consumer_rejections: 0,
            budget_exhausted: false,
            link_down: false,
        }
    }

    fn domain() -> DomainQuality {
        DomainQuality {
            expected_wkc: 1,
            actual_wkc: 1,
            valid: true,
            complete: true,
            last_valid_cycle: 7,
            input_age_cycles: 0,
        }
    }

    fn other() -> OtherCycleFacts {
        OtherCycleFacts {
            platform_ready: true,
            coe_ready: true,
            topology_valid: true,
            drive_ready: true,
            command_current: true,
            supervisor_healthy: true,
            external_safety_clear: true,
            deadline_met: true,
        }
    }

    fn locked_dc() -> DcCyclicSync {
        let mut sync = DcCyclicSync::new(
            DcCyclicConfig::new(0x1000, 13, 0),
            DcMonitor::new(50, 10, 1, 2),
        );
        let mut image = [0; 8];
        sync.prepare(3, 100, &mut image).unwrap();
        let plan = sync.datagram_plan();
        sync.complete(
            7,
            100,
            RxMatch {
                slot_id: 0,
                generation: 3,
                working_counter: 1,
            },
            DatagramHeader {
                command: Command::Frmw,
                index: plan.index,
                address: plan.address,
                length: 8,
                last: true,
            },
            &100u64.to_le_bytes(),
        )
        .unwrap();
        sync
    }

    #[test]
    fn fresh_verified_cycle_produces_complete_good_facts() {
        let facts =
            cyclic_quality_from_ethercat(report(), &[domain(), domain()], &locked_dc(), other());
        assert!(facts.domain_valid && facts.wkc_valid && facts.distributed_clock_locked);
        assert!(facts.cycle_within_budget && facts.command_current);
    }

    #[test]
    fn missing_or_stale_domain_fails_closed() {
        let dc = locked_dc();
        let empty = cyclic_quality_from_ethercat(report(), &[], &dc, other());
        assert!(!empty.domain_valid && !empty.wkc_valid);
        let stale = DomainQuality {
            last_valid_cycle: 6,
            ..domain()
        };
        let facts = cyclic_quality_from_ethercat(report(), &[domain(), stale], &dc, other());
        assert!(!facts.domain_valid && !facts.wkc_valid);
        for faulty in [
            DomainQuality {
                expected_wkc: 0,
                actual_wkc: 0,
                ..domain()
            },
            DomainQuality {
                actual_wkc: 0,
                ..domain()
            },
            DomainQuality {
                valid: false,
                ..domain()
            },
            DomainQuality {
                input_age_cycles: 1,
                ..domain()
            },
            DomainQuality {
                complete: false,
                ..domain()
            },
        ] {
            assert!(!cyclic_quality_from_ethercat(report(), &[faulty], &dc, other()).domain_valid);
        }
    }

    #[test]
    fn receive_errors_and_dc_freshness_cannot_pass() {
        let dc = locked_dc();
        let bad_reports = [
            CycleReport {
                link_down: true,
                ..report()
            },
            CycleReport {
                received_frames: 0,
                ..report()
            },
            CycleReport {
                wkc_mismatches: 1,
                ..report()
            },
            CycleReport {
                timed_out_datagrams: 1,
                ..report()
            },
            CycleReport {
                corrupt_frames: 1,
                ..report()
            },
            CycleReport {
                unmatched_datagrams: 1,
                ..report()
            },
            CycleReport {
                consumer_rejections: 1,
                ..report()
            },
        ];
        for bad in bad_reports {
            assert!(!cyclic_quality_from_ethercat(bad, &[domain()], &dc, other()).wkc_valid);
            if bad.link_down {
                assert!(
                    !cyclic_quality_from_ethercat(bad, &[domain()], &dc, other())
                        .distributed_clock_locked
                );
            }
        }
        let next = CycleReport {
            cycle: 8,
            ..report()
        };
        assert!(
            !cyclic_quality_from_ethercat(next, &[domain()], &dc, other()).distributed_clock_locked
        );
        let mut image = [0; 8];
        let mut pending = locked_dc();
        pending.prepare(4, 101, &mut image).unwrap();
        assert!(
            !cyclic_quality_from_ethercat(report(), &[domain()], &pending, other())
                .distributed_clock_locked
        );
        let plan = pending.datagram_plan();
        assert!(
            pending
                .complete(
                    7,
                    100,
                    RxMatch {
                        slot_id: 0,
                        generation: 4,
                        working_counter: 0,
                    },
                    DatagramHeader {
                        command: Command::Frmw,
                        index: plan.index,
                        address: plan.address,
                        length: 8,
                        last: true,
                    },
                    &100u64.to_le_bytes(),
                )
                .is_err()
        );
        // The monitor still remembers the previous lock; last_error must win.
        assert!(pending.monitor().is_locked());
        assert!(
            !cyclic_quality_from_ethercat(report(), &[domain()], &pending, other())
                .distributed_clock_locked
        );
    }

    #[test]
    fn deadline_and_budget_are_independent_of_rx_health() {
        let dc = locked_dc();
        for (report, other) in [
            (
                CycleReport {
                    budget_exhausted: true,
                    ..report()
                },
                other(),
            ),
            (
                report(),
                OtherCycleFacts {
                    deadline_met: false,
                    ..other()
                },
            ),
        ] {
            let facts = cyclic_quality_from_ethercat(report, &[domain()], &dc, other);
            assert!(!facts.cycle_within_budget);
            assert!(facts.wkc_valid);
        }
    }
}
