//! Generated ESOP Protobuf bindings for the non-real-time supervision domain.
//!
//! The source of truth is `proto/esop/v1/esop.proto`. This crate must not be
//! linked into the EtherCAT cycle, MLG decision, or ProcBuf implementation.

pub use prost::{DecodeError, Message};

pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/esop.v1.rs"));
}

#[cfg(test)]
mod tests {
    use super::Message;
    use super::v1::{MotionCommand, RobotState};

    #[test]
    fn generated_v1_bindings_round_trip_known_fields() {
        let command = MotionCommand {
            robot_id: "robot_01".to_owned(),
            boot_id: 7,
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
            ..RobotState::default()
        }
        .encode_to_vec();
        encoded.extend_from_slice(&[0x68, 0x01]);
        let decoded = RobotState::decode(encoded.as_slice()).unwrap();
        assert_eq!(decoded.robot_id, "robot_01");
        assert_eq!(decoded.boot_id, 7);
    }
}
