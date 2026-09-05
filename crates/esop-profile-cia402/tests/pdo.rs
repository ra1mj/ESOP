#![cfg(feature = "ethercat")]

use esop_ethercat_core::{PdoDirection, PdoEntry};
use esop_profile_cia402::{
    CONTROLWORD_ENABLE_OPERATION, Cia402MotionGate, Cia402PdoCommand, Cia402PdoField, Cia402PdoMap,
    Cia402Target, OperatingMode,
};

fn entry(field: Cia402PdoField, bit_offset: usize) -> PdoEntry {
    let (bit_length, signed) = match field {
        Cia402PdoField::Controlword | Cia402PdoField::Statusword | Cia402PdoField::ErrorCode => {
            (16, false)
        }
        Cia402PdoField::ModeOfOperation | Cia402PdoField::ModeDisplay => (8, true),
        Cia402PdoField::TargetTorque | Cia402PdoField::ActualTorque => (16, true),
        Cia402PdoField::TargetPosition
        | Cia402PdoField::TargetVelocity
        | Cia402PdoField::ActualPosition
        | Cia402PdoField::ActualVelocity
        | Cia402PdoField::FollowingError => (32, true),
    };
    PdoEntry {
        index: field.object_index(),
        subindex: 0,
        bit_offset,
        bit_length,
        signed,
        direction: field.direction(),
    }
}

fn map() -> Cia402PdoMap {
    Cia402PdoMap::new()
        .with_entry(
            Cia402PdoField::Controlword,
            entry(Cia402PdoField::Controlword, 0),
        )
        .with_entry(
            Cia402PdoField::ModeOfOperation,
            entry(Cia402PdoField::ModeOfOperation, 16),
        )
        .with_entry(
            Cia402PdoField::TargetPosition,
            entry(Cia402PdoField::TargetPosition, 24),
        )
        .with_entry(
            Cia402PdoField::TargetVelocity,
            entry(Cia402PdoField::TargetVelocity, 56),
        )
        .with_entry(
            Cia402PdoField::TargetTorque,
            entry(Cia402PdoField::TargetTorque, 88),
        )
        .with_entry(
            Cia402PdoField::Statusword,
            entry(Cia402PdoField::Statusword, 0),
        )
        .with_entry(
            Cia402PdoField::ModeDisplay,
            entry(Cia402PdoField::ModeDisplay, 16),
        )
        .with_entry(
            Cia402PdoField::ErrorCode,
            entry(Cia402PdoField::ErrorCode, 24),
        )
        .with_entry(
            Cia402PdoField::ActualPosition,
            entry(Cia402PdoField::ActualPosition, 40),
        )
        .with_entry(
            Cia402PdoField::ActualVelocity,
            entry(Cia402PdoField::ActualVelocity, 72),
        )
        .with_entry(
            Cia402PdoField::ActualTorque,
            entry(Cia402PdoField::ActualTorque, 104),
        )
        .with_entry(
            Cia402PdoField::FollowingError,
            entry(Cia402PdoField::FollowingError, 120),
        )
}

#[test]
fn public_adapter_binds_all_three_robot_modes() {
    let map = map();
    assert_eq!(map.validate_all_modes(), Ok(()));
    assert_eq!(Cia402PdoField::Controlword.direction(), PdoDirection::Rx);

    let gate = Cia402MotionGate {
        lifecycle_permit: true,
        mode_confirmed: true,
        operation_enabled: true,
        setpoint_valid: true,
    };
    let mut image = [0_u8; 20];
    for (mode, target) in [
        (OperatingMode::Csp, Cia402Target::Position(-10)),
        (OperatingMode::Csv, Cia402Target::Velocity(20)),
        (OperatingMode::Cst, Cia402Target::Torque(-3)),
    ] {
        map.write_cyclic(
            &mut image,
            Cia402PdoCommand {
                controlword: CONTROLWORD_ENABLE_OPERATION,
                mode,
                target,
            },
            gate,
        )
        .unwrap();
    }

    assert_eq!(
        map.entry(Cia402PdoField::TargetPosition)
            .unwrap()
            .read_signed(&image),
        Ok(-10)
    );
    assert_eq!(
        map.entry(Cia402PdoField::TargetVelocity)
            .unwrap()
            .read_signed(&image),
        Ok(20)
    );
    assert_eq!(
        map.entry(Cia402PdoField::TargetTorque)
            .unwrap()
            .read_signed(&image),
        Ok(-3)
    );
}

#[test]
fn map_can_be_built_from_frozen_sii_entries() {
    let entries = [
        entry(Cia402PdoField::Controlword, 0),
        entry(Cia402PdoField::ModeOfOperation, 16),
        entry(Cia402PdoField::TargetPosition, 24),
        entry(Cia402PdoField::TargetVelocity, 56),
        entry(Cia402PdoField::TargetTorque, 88),
        entry(Cia402PdoField::Statusword, 128),
        entry(Cia402PdoField::ModeDisplay, 144),
        entry(Cia402PdoField::ErrorCode, 152),
        entry(Cia402PdoField::ActualPosition, 168),
        entry(Cia402PdoField::ActualVelocity, 200),
        entry(Cia402PdoField::ActualTorque, 232),
        entry(Cia402PdoField::FollowingError, 248),
    ];
    let map = Cia402PdoMap::from_pdo_entries(&entries).unwrap();
    assert_eq!(map.validate_all_modes(), Ok(()));
}

#[test]
fn map_rejects_duplicate_standard_entries() {
    let entries = [
        entry(Cia402PdoField::Controlword, 0),
        entry(Cia402PdoField::Controlword, 16),
    ];
    assert_eq!(
        Cia402PdoMap::from_pdo_entries(&entries),
        Err(esop_profile_cia402::Cia402PdoError::DuplicateField(
            Cia402PdoField::Controlword
        ))
    );
}
