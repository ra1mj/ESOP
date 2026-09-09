//! Generated ESOP Protobuf bindings for the non-real-time supervision domain.
//!
//! The source of truth is `proto/esop/v1/esop.proto`. This crate must not be
//! linked into the EtherCAT cycle, MLG decision, or ProcBuf implementation.

pub use prost::{DecodeError, Message};

/// Version carried by every top-level message in the current ESOP contract.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchemaCompatibilityError {
    Missing,
    Unsupported(u32),
}

pub const fn validate_schema_version(version: u32) -> Result<(), SchemaCompatibilityError> {
    match version {
        0 => Err(SchemaCompatibilityError::Missing),
        CURRENT_SCHEMA_VERSION => Ok(()),
        other => Err(SchemaCompatibilityError::Unsupported(other)),
    }
}

pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/esop.v1.rs"));
}

#[cfg(test)]
mod tests {
    use super::v1::{MotionCommand, RobotState};
    use super::{
        CURRENT_SCHEMA_VERSION, Message, SchemaCompatibilityError, validate_schema_version,
    };

    #[test]
    fn generated_v1_bindings_round_trip_known_fields() {
        let command = MotionCommand {
            robot_id: "robot_01".to_owned(),
            boot_id: 7,
            schema_version: CURRENT_SCHEMA_VERSION,
            source_id: 42,
            permit_epoch: 3,
            sequence: 9,
            deadline_ns: 100,
            axis_mask: 0x03,
            authority: 2,
            policy_version: 1,
            ..MotionCommand::default()
        };
        let encoded = command.encode_to_vec();
        let decoded = MotionCommand::decode(encoded.as_slice()).unwrap();
        assert_eq!(decoded, command);
    }

    #[test]
    fn generated_state_stays_forward_compatible_with_unknown_fields() {
        let mut encoded = RobotState {
            robot_id: "robot_01".to_owned(),
            boot_id: 7,
            schema_version: CURRENT_SCHEMA_VERSION,
            ..RobotState::default()
        }
        .encode_to_vec();
        encoded.extend_from_slice(&[0xA0, 0x01, 0x01]);
        let decoded = RobotState::decode(encoded.as_slice()).unwrap();
        assert_eq!(decoded.robot_id, "robot_01");
        assert_eq!(decoded.boot_id, 7);
        assert_eq!(decoded.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn schema_version_accepts_v1_and_rejects_missing_or_unknown_versions() {
        assert_eq!(validate_schema_version(1), Ok(()));
        assert_eq!(
            validate_schema_version(0),
            Err(SchemaCompatibilityError::Missing)
        );
        assert_eq!(
            validate_schema_version(2),
            Err(SchemaCompatibilityError::Unsupported(2))
        );
    }
}
