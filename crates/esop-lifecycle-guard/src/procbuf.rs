//! Fixed-size projection for the optional ProcBuf ABI boundary.

use crate::{CyclicQuality, LifecycleSnapshot};
use esop_procbuf::{CyclicQualityMask, LifecycleSummary, QualityFact, StatePage};

/// Write the raw cycle owner's facts into the unpublished State page. Bind
/// their sequence to that same page so the reader can reject stale quality.
pub fn cyclic_quality_to_procbuf<const AXES: usize, const IO: usize, const DOMAINS: usize>(
    state: &mut StatePage<AXES, IO, DOMAINS>,
    quality: CyclicQuality,
) {
    let observations = [
        (QualityFact::Platform, quality.platform_ready),
        (QualityFact::Configuration, quality.coe_ready),
        (QualityFact::Topology, quality.topology_valid),
        (
            QualityFact::DistributedClock,
            quality.distributed_clock_locked,
        ),
        (QualityFact::Drive, quality.drive_ready),
        (QualityFact::Domain, quality.domain_valid),
        (QualityFact::Wkc, quality.wkc_valid),
        (QualityFact::Command, quality.command_current),
        (QualityFact::Supervisor, quality.supervisor_healthy),
        (QualityFact::ExternalSafety, quality.external_safety_clear),
        (QualityFact::CycleBudget, quality.cycle_within_budget),
    ];
    let mut good_mask = 0;
    for (fact, good) in observations {
        if good {
            good_mask |= fact.bit();
        }
    }
    state.quality.cyclic = CyclicQualityMask {
        known_mask: QualityFact::ALL_MASK,
        good_mask,
    };
    state.quality.sequence = state.sequence;
}

/// `transition_time_ns` must come from the recorded transition's monotonic
/// timestamp, not from the current cycle when publishing a later snapshot.
pub fn lifecycle_to_procbuf(
    snapshot: LifecycleSnapshot,
    transition_time_ns: u64,
) -> LifecycleSummary {
    LifecycleSummary {
        state: snapshot.state as u8,
        stop_action: snapshot.stop_action as u8,
        gates_ready: ((snapshot.ready_gate_mask & snapshot.required_gate_mask)
            == snapshot.required_gate_mask) as u8,
        motion_permit: snapshot.motion_permit_current as u8,
        required_gate_mask: snapshot.required_gate_mask,
        valid_gate_mask: snapshot.valid_gate_mask,
        qualified_gate_mask: snapshot.qualified_gate_mask,
        ready_gate_mask: snapshot.ready_gate_mask,
        first_blocking_code: snapshot.first_blocking_code,
        latched_fault_code: snapshot.latched_fault_code,
        permit_epoch: snapshot.permit_epoch,
        permit_expires_at_ns: snapshot.permit_expires_at_ns,
        transition_sequence: snapshot.transition_sequence,
        transition_cycle: snapshot.transition_cycle,
        transition_time_ns,
        recovery_count: snapshot.recovery_count,
        permit_audit_sequence: snapshot.permit_audit_sequence,
    }
}
