use esop_lifecycle_guard::{
    GateId, GuardPolicy, LifecycleAction, LifecycleGuard, MotionPermit, StopAction,
};
use esop_procbuf::{CommandPage, ControlMode, JointCommand, ProcBuf};
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
        exit_bad_cycles: 1,
        max_age_cycles: 1,
        stop_action: StopAction::QuickStop,
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
                permit_epoch: 1,
                sequence: 1,
                axis_mask: 0x03,
                expires_at_ns: 100,
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
                permit_epoch: 1,
                sequence: 2,
                axis_mask: 0x03,
                expires_at_ns: 100,
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

    buffer.publish_command(command(7, 2, 10)).unwrap();
    assert_eq!(
        buffer.read_command(10, &mut command_floor),
        Err(esop_procbuf::CommandReadError::Expired)
    );
    guard.update_gate(GateId::Command, false, 3, 0x434D_0001);
    assert_eq!(
        guard.cycle(3, 3),
        LifecycleAction::Stop(StopAction::QuickStop)
    );
    let blocked = cia402.step(0x0027, DriveRequest::Enable, false);
    assert!(!blocked.motion_allowed);
}
