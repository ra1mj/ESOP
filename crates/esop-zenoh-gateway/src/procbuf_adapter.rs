//! Zenoh-facing compatibility wrapper around the shared hosted payload adapter.

use esop_ipc::payloads::{ProcBufProjector as SharedProjector, RobotId};
use esop_procbuf::ProcBuf;
use esop_proto::v1::{DiagnosticEvent, RobotState};

use crate::KeySpace;

pub use esop_ipc::payloads::ProjectionError;

/// Preserves the Zenoh constructor while all projection logic remains owned by
/// the shared `esop-ipc` payload adapter.
pub struct ProcBufProjector(SharedProjector);

impl ProcBufProjector {
    pub fn new(key_space: KeySpace, numeric_robot_id: u64, boot_id: u64) -> Self {
        let robot = core::str::from_utf8(key_space.robot())
            .expect("KeySpace accepts ASCII identifiers only");
        let robot = RobotId::new(robot).expect("KeySpace robot identifier is bounded and valid");
        Self(SharedProjector::new(robot, numeric_robot_id, boot_id))
    }

    pub fn read_state<
        const AXES: usize,
        const IO: usize,
        const DOMAINS: usize,
        const EVENTS: usize,
    >(
        &mut self,
        buffer: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
    ) -> Result<Option<RobotState>, ProjectionError> {
        self.0.read_state(buffer)
    }

    pub fn pop_event<
        const AXES: usize,
        const IO: usize,
        const DOMAINS: usize,
        const EVENTS: usize,
    >(
        &mut self,
        buffer: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
    ) -> Result<Option<DiagnosticEvent>, ProjectionError> {
        self.0.pop_event(buffer)
    }
}
