use esop_command_gateway::{
    CommandIngress, ExternalMotionCommand, IngressDecision, IngressError, IngressPolicy,
    MAX_INGRESS_AUDITS,
};
use esop_lifecycle_guard::{
    GateId, GuardPolicy, LifecycleAction, LifecycleGuard, MotionPermit, StopAction,
};

fn command(sequence: u64, source_id: u64, deadline_ns: u64) -> ExternalMotionCommand {
    ExternalMotionCommand {
        boot_id: 7,
        source_id,
        permit_epoch: 1,
        sequence,
        deadline_ns,
        axis_mask: 0x03,
        authority: 2,
        reserved: [0; 3],
        policy_version: 9,
    }
}

fn policy() -> IngressPolicy {
    IngressPolicy {
        authorized_sources: [42, 77, 0, 0],
        authorized_source_count: 2,
        minimum_authority: 2,
        reserved: [0; 2],
        permit_policy_version: 9,
        allowed_axis_mask: 0x03,
        max_ttl_ns: 100,
        rate_window_ns: 1_000,
        max_commands_per_window: 2,
        reserved_tail: [0; 6],
    }
}

#[test]
fn admitted_command_becomes_mlg_permit_without_network_in_rt_path() {
    let mut ingress = CommandIngress::new(7, policy());
    let permit = ingress.admit(command(1, 42, 100), 1).unwrap();
    assert_eq!(
        permit,
        MotionPermit {
            boot_id: 7,
            source_id: 42,
            permit_epoch: 1,
            sequence: 1,
            expires_at_ns: 100,
            axis_mask: 0x03,
            authority: 2,
            reserved: [0; 3],
            policy_version: 9,
        }
    );
    assert_eq!(ingress.audit_count(), 1);
    assert_eq!(
        ingress.audit_at(0).unwrap().decision,
        IngressDecision::Accepted
    );

    let mut guard = LifecycleGuard::new(
        GateId::Platform.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            exit_bad_cycles: 1,
            max_age_cycles: 1,
            stop_action: StopAction::QuickStop,
            authorized_source_id: 42,
            minimum_authority: 2,
            permit_policy_version: 9,
        },
    );
    guard.update_gate(GateId::Platform, true, 1, 0);
    guard.accept_permit(permit, 1).unwrap();
    assert_eq!(guard.cycle(1, 1), LifecycleAction::Hold);
    assert_eq!(
        guard.request_rearm(permit_with_sequence(permit, 2), 1, 1),
        Ok(LifecycleAction::EnableAllowed)
    );
}

fn permit_with_sequence(mut permit: MotionPermit, sequence: u64) -> MotionPermit {
    permit.sequence = sequence;
    permit
}

#[test]
fn ingress_rejects_policy_violations_and_bounds_audit_history() {
    let mut ingress = CommandIngress::new(7, policy());
    assert_eq!(
        ingress.admit(command(1, 99, 100), 1),
        Err(IngressError::SourceUnauthorized)
    );
    let mut bad_version = command(2, 42, 100);
    bad_version.policy_version = 8;
    assert_eq!(
        ingress.admit(bad_version, 2),
        Err(IngressError::PolicyVersionMismatch)
    );
    assert!(ingress.admit(command(1, 42, 100), 3).is_ok());
    assert_eq!(
        ingress.admit(command(1, 42, 100), 4),
        Err(IngressError::SequenceReplayed)
    );
    assert!(ingress.admit(command(2, 42, 100), 5).is_ok());
    assert_eq!(
        ingress.admit(command(3, 42, 100), 6),
        Err(IngressError::RateLimited)
    );

    for sequence in 4..=(MAX_INGRESS_AUDITS as u64 + 3) {
        let mut rejected = command(sequence, 42, 100);
        rejected.policy_version = 8;
        assert_eq!(
            ingress.admit(rejected, sequence),
            Err(IngressError::PolicyVersionMismatch)
        );
    }
    assert_eq!(ingress.audit_count(), MAX_INGRESS_AUDITS);
    assert_eq!(ingress.audit_at(0).unwrap().sequence, 7);
    assert_eq!(
        ingress.audit_at(0).unwrap().error_code,
        IngressError::PolicyVersionMismatch.code()
    );
}
