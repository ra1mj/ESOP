use esop_ebpf_agent::{CAPABILITY_BTF, RuntimeAgent};
use esop_lifecycle_guard::{
    GateId, GuardPolicy, LifecycleAction, LifecycleError, LifecycleGuard, LifecycleState,
    MotionPermit, PermitError, StopAction, StopFeedback, procbuf::lifecycle_to_procbuf,
};
use esop_procbuf::{
    CommandPage, ControlMode, JointCommand, LifecycleTransitionRecord, ProcBuf, StatePage,
};
use esop_profile_cia402::{Cia402Controller, DriveRequest, ModeSupervisor, OperatingMode};

type Buffer = ProcBuf<2, 0, 1, 2>;

fn command(boot_id: u64, sequence: u64, deadline_ns: u64) -> CommandPage<2, 0> {
    CommandPage {
        boot_id,
        sequence,
        deadline_ns,
        source_id: 11,
        permit_epoch: 1,
        permit_expires_at_ns: deadline_ns,
        axis_mask: 0x03,
        requested_mode: ControlMode::Csp,
        motion_enable_request: 1,
        authority: 1,
        reserved: 0,
        axes: [JointCommand::EMPTY; 2],
        io: [],
    }
}

#[test]
fn procbuf_command_expiry_stops_mlg_and_blocks_cia402_enable() {
    let buffer = Buffer::new(1, 7);
    let policy = GuardPolicy {
        enter_good_cycles: 1,
        exit_bad_cycles: 2,
        max_age_cycles: 1,
        stop_timeout_cycles: 1_000,
        stop_action: StopAction::QuickStop,
        authorized_source_id: 11,
        minimum_authority: 1,
        permit_policy_version: 1,
        allowed_axis_mask: 0x03,
    };
    let required = GateId::Platform.bit() | GateId::Link.bit() | GateId::Command.bit();
    let mut guard = LifecycleGuard::new(required, 7, policy);
    for gate in [GateId::Platform, GateId::Link, GateId::Command] {
        guard.update_gate(gate, true, 1, 0);
    }
    guard
        .accept_permit(
            MotionPermit {
                boot_id: 7,
                source_id: 11,
                permit_epoch: 1,
                sequence: 1,
                axis_mask: 0x03,
                expires_at_ns: 100,
                authority: 1,
                reserved: [0; 3],
                policy_version: 1,
            },
            1,
        )
        .unwrap();

    buffer.publish_command(command(7, 1, 100)).unwrap();
    let mut command_floor = 0;
    let snapshot = buffer.read_command(2, &mut command_floor).unwrap();
    assert_eq!(snapshot.command.requested_mode, ControlMode::Csp);
    assert_eq!(
        guard.request_rearm(
            MotionPermit {
                boot_id: 7,
                source_id: 11,
                permit_epoch: 1,
                sequence: 2,
                axis_mask: 0x03,
                expires_at_ns: 100,
                authority: 1,
                reserved: [0; 3],
                policy_version: 1,
            },
            1,
            2,
        ),
        Ok(LifecycleAction::EnableAllowed)
    );

    let mut mode = ModeSupervisor::new(1);
    mode.request(OperatingMode::Csp, 1, 4).unwrap();
    let mode_output = mode.step(
        OperatingMode::Csp.raw(),
        esop_profile_cia402::DriveState::OperationEnabled,
        2,
    );
    assert!(mode_output.cyclic_allowed);
    let mut cia402 = Cia402Controller::new();
    let enabled = cia402.step(
        0x0027,
        DriveRequest::Enable,
        guard.cycle(2, 2) == LifecycleAction::EnableAllowed && mode_output.cyclic_allowed,
    );
    assert!(enabled.motion_allowed);
    let active = lifecycle_to_procbuf(guard.snapshot(2, 2), 2_000_000);
    assert_eq!(active.gates_ready, 1);
    assert_eq!(active.motion_permit, 1);

    buffer.publish_command(command(7, 2, 10)).unwrap();
    assert_eq!(
        buffer.read_command(10, &mut command_floor),
        Err(esop_procbuf::CommandReadError::Expired)
    );
    guard.update_gate(GateId::Platform, true, 3, 0);
    guard.update_gate(GateId::Link, true, 3, 0);
    guard.update_gate(GateId::Command, false, 3, 0x434D_0001);
    assert_eq!(
        guard.cycle(3, 3),
        LifecycleAction::Stop(StopAction::QuickStop)
    );
    let blocked = cia402.step(0x0027, DriveRequest::Enable, false);
    assert!(!blocked.motion_allowed);

    let wrong_boot = MotionPermit {
        boot_id: 8,
        source_id: 11,
        permit_epoch: 2,
        sequence: 3,
        axis_mask: 0x03,
        expires_at_ns: 100,
        authority: 1,
        reserved: [0; 3],
        policy_version: 1,
    };
    assert_eq!(
        guard.accept_permit(wrong_boot, 10),
        Err(PermitError::BootMismatch)
    );

    let snapshot = guard.snapshot(3, 10);
    let mut state = StatePage::new(7);
    state.sequence = 3;
    state.lifecycle = lifecycle_to_procbuf(snapshot, snapshot.transition_cycle * 1_000_000);
    for index in 0..guard.transition_count() {
        let transition = guard.transition_at(index).unwrap();
        state.lifecycle_history.push(LifecycleTransitionRecord {
            sequence: transition.sequence,
            timestamp_ns: transition.cycle * 1_000_000,
            from_state: transition.from as u8,
            to_state: transition.to as u8,
            reserved: 0,
            fault_code: transition.fault_code,
        });
    }
    buffer.publish_state(state).unwrap();
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.lifecycle.state, snapshot.state as u8);
    assert_eq!(published.lifecycle.stop_action, snapshot.stop_action as u8);
    assert_eq!(published.lifecycle.gates_ready, 0);
    // Entering Stopping revokes the permit before this snapshot is published.
    assert_eq!(published.lifecycle.motion_permit, 0);
    assert_eq!(published.lifecycle.required_gate_mask, required);
    assert_eq!(published.lifecycle.first_blocking_code, 0x434D_0001);
    assert_ne!(
        published.lifecycle.qualified_gate_mask & GateId::Command.bit(),
        0
    );
    assert_eq!(
        published.lifecycle.valid_gate_mask,
        snapshot.valid_gate_mask
    );
    assert_eq!(
        published.lifecycle.qualified_gate_mask,
        snapshot.qualified_gate_mask
    );
    assert_eq!(
        published.lifecycle.ready_gate_mask,
        snapshot.ready_gate_mask
    );
    assert_eq!(published.lifecycle.permit_epoch, snapshot.permit_epoch);
    assert_eq!(
        published.lifecycle.permit_expires_at_ns,
        snapshot.permit_expires_at_ns
    );
    assert_eq!(
        published.lifecycle.transition_cycle,
        snapshot.transition_cycle
    );
    assert_eq!(published.lifecycle.permit_audit_sequence, 1);
    assert_eq!(
        published.lifecycle.transition_sequence,
        snapshot.transition_sequence
    );
    assert_eq!(published.lifecycle_history.len(), guard.transition_count());
    assert_eq!(
        published.lifecycle_history.get(1).unwrap().fault_code,
        0x434D_0001
    );
    assert_eq!(
        published.lifecycle_history.get(1).unwrap().to_state,
        guard.transition_at(1).unwrap().to as u8
    );
}

#[test]
fn ebpf_health_heartbeat_can_qualify_then_stop_motion() {
    let policy = GuardPolicy {
        enter_good_cycles: 1,
        exit_bad_cycles: 1,
        max_age_cycles: 1,
        stop_timeout_cycles: 1_000,
        stop_action: StopAction::QuickStop,
        authorized_source_id: 1,
        minimum_authority: 1,
        permit_policy_version: 1,
        allowed_axis_mask: 1,
    };
    let mut guard = LifecycleGuard::new(GateId::HostObservation.bit(), 7, policy);
    let mut agent = RuntimeAgent::<2>::new(7, 1, 1_000);
    agent
        .health_mut()
        .set_capabilities(CAPABILITY_BTF, CAPABILITY_BTF, true);

    let healthy = agent.heartbeat(100);
    guard.update_host_observation(healthy, 1, 100, 10).unwrap();
    guard
        .accept_permit(
            MotionPermit {
                boot_id: 7,
                source_id: 1,
                permit_epoch: 1,
                sequence: 1,
                axis_mask: 1,
                expires_at_ns: 1_000,
                authority: 1,
                reserved: [0; 3],
                policy_version: 1,
            },
            100,
        )
        .unwrap();
    assert_eq!(guard.cycle(1, 100), LifecycleAction::Hold);
    assert_eq!(
        guard.request_rearm(
            MotionPermit {
                boot_id: 7,
                source_id: 1,
                permit_epoch: 1,
                sequence: 2,
                axis_mask: 1,
                expires_at_ns: 1_000,
                authority: 1,
                reserved: [0; 3],
                policy_version: 1,
            },
            1,
            100,
        ),
        Ok(LifecycleAction::EnableAllowed)
    );

    agent.health_mut().record_event_loss(2);
    agent
        .health_mut()
        .set_capabilities(CAPABILITY_BTF, CAPABILITY_BTF, true);
    let degraded = agent.heartbeat(101);
    assert_eq!(
        degraded.state,
        esop_lifecycle_guard::ObservationState::Degraded
    );
    assert_eq!(degraded.lost_event_count, 2);
    assert_eq!(degraded.fault_code, 0x4542_1004);
    guard.update_host_observation(degraded, 2, 101, 10).unwrap();
    assert_eq!(
        guard.cycle(2, 101),
        LifecycleAction::Stop(StopAction::QuickStop)
    );
}

#[test]
fn external_inhibit_latches_only_after_stop_and_publishes_fault_reason() {
    let buffer = Buffer::new(1, 7);
    let mut guard = LifecycleGuard::new(
        GateId::ExternalSafety.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            allowed_axis_mask: 0x03,
            ..GuardPolicy::conservative()
        },
    );
    guard.update_gate(GateId::ExternalSafety, true, 1, 0);
    guard
        .request_rearm(
            MotionPermit {
                boot_id: 7,
                source_id: 1,
                permit_epoch: 1,
                sequence: 1,
                axis_mask: 0x03,
                expires_at_ns: 100,
                authority: 1,
                reserved: [0; 3],
                policy_version: 1,
            },
            1,
            1,
        )
        .unwrap();
    guard.update_gate(GateId::ExternalSafety, false, 2, 0x5341_0001);
    let mut decision = guard.cycle_axes(2, 2);
    assert_eq!(
        decision.action(),
        LifecycleAction::Stop(StopAction::QuickStop)
    );
    decision.mark_stop_transmitted().unwrap();
    assert_eq!(guard.state(), LifecycleState::Stopping);
    assert_eq!(guard.clear_fault(2), Err(LifecycleError::InvalidState));

    let mut stopping = StatePage::new(7);
    stopping.sequence = 2;
    stopping.lifecycle = lifecycle_to_procbuf(guard.snapshot(2, 2), 2_000_000);
    buffer.publish_state(stopping).unwrap();
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.lifecycle.motion_permit, 0);
    assert_eq!(published.lifecycle.first_blocking_code, 0x5341_0001);
    assert_eq!(published.lifecycle.latched_fault_code, 0);

    guard
        .acknowledge_stopped(
            3,
            StopFeedback {
                cycle: 3,
                observed_axis_mask: 0x03,
                stationary_axis_mask: 0x03,
                non_enabled_axis_mask: 0x03,
            },
        )
        .unwrap();
    assert_eq!(guard.state(), LifecycleState::FaultLatched);
    assert_eq!(guard.cycle(3, 3), LifecycleAction::FaultLatched);
    let mut latched = StatePage::new(7);
    latched.sequence = 3;
    latched.lifecycle = lifecycle_to_procbuf(guard.snapshot(3, 3), 3_000_000);
    buffer.publish_state(latched).unwrap();
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.lifecycle.latched_fault_code, 0x5341_0001);
    assert_eq!(published.lifecycle.stop_action, StopAction::Disable as u8);
}
