//! Project validated, current-cycle CiA 402 feedback into stop evidence.

use esop_profile_cia402::{Cia402PdoInputs, DriveState};

use crate::StopFeedback;

/// Call only after the input Domain has passed complete-frame, WKC and age
/// checks. A missing velocity sample or unknown drive state cannot confirm a stop.
pub fn stop_feedback_from_cia402(
    cycle: u64,
    axis_index: u8,
    inputs: Cia402PdoInputs,
    max_stationary_velocity: u32,
) -> Option<StopFeedback> {
    let axis_bit = 1u32.checked_shl(axis_index as u32)?;
    let drive_state = DriveState::from_statusword(inputs.statusword);
    let stationary = inputs
        .actual_velocity
        .is_some_and(|velocity| velocity.unsigned_abs() <= max_stationary_velocity);
    let non_enabled = !matches!(
        drive_state,
        DriveState::OperationEnabled | DriveState::FaultReactionActive | DriveState::Unknown
    );
    Some(StopFeedback {
        cycle,
        observed_axis_mask: axis_bit,
        stationary_axis_mask: if stationary { axis_bit } else { 0 },
        non_enabled_axis_mask: if non_enabled { axis_bit } else { 0 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use esop_profile_cia402::OperatingMode;

    fn inputs(statusword: u16, actual_velocity: Option<i32>) -> Cia402PdoInputs {
        Cia402PdoInputs {
            statusword,
            actual_mode: OperatingMode::Csp,
            error_code: 0,
            actual_position: None,
            actual_velocity,
            actual_torque: None,
            following_error: None,
        }
    }

    #[test]
    fn missing_motion_feedback_and_active_drive_cannot_confirm_stop() {
        let disabled = stop_feedback_from_cia402(7, 1, inputs(0x0040, None), 3).unwrap();
        assert_eq!(disabled.observed_axis_mask, 2);
        assert_eq!(disabled.stationary_axis_mask, 0);
        assert_eq!(disabled.non_enabled_axis_mask, 2);

        let active = stop_feedback_from_cia402(7, 1, inputs(0x0027, Some(0)), 3).unwrap();
        assert_eq!(active.non_enabled_axis_mask, 0);
        assert_eq!(active.stationary_axis_mask, 2);
        let unknown = stop_feedback_from_cia402(7, 1, inputs(0xFFFF, Some(0)), 3).unwrap();
        assert_eq!(unknown.non_enabled_axis_mask, 0);
        assert_eq!(
            stop_feedback_from_cia402(7, 32, inputs(0x0040, Some(0)), 3),
            None
        );
    }

    #[test]
    fn velocity_threshold_is_inclusive_and_safe_for_minimum_i32() {
        let within = stop_feedback_from_cia402(8, 31, inputs(0x0040, Some(-3)), 3).unwrap();
        assert_eq!(within.stationary_axis_mask, 1u32 << 31);
        let moving = stop_feedback_from_cia402(8, 31, inputs(0x0040, Some(i32::MIN)), 3).unwrap();
        assert_eq!(moving.stationary_axis_mask, 0);
    }
}
