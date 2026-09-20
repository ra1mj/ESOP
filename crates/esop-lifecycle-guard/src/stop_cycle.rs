//! Fixed-capacity stop branch of the EtherCAT cycle owner.
//!
//! The caller owns RX, Domain completion, command admission, and the normal
//! motion branch. This branch consumes only a finished Domain and a real
//! receive report, then publishes stop TX and its evidence for that cycle.

use crate::cia402::step_axis_bank;
use crate::ethercat::{
    OtherCycleFacts, StopFrameError, submit_stopping_frame, verified_ethercat_stop_feedback,
};
use crate::procbuf::{
    AxisEvidenceError, LifecycleEventCursor, LifecycleEventError, axis_stops_to_procbuf,
    ethercat_cycle_to_procbuf, lifecycle_events_to_procbuf, lifecycle_to_procbuf,
};
use crate::{CyclicQuality, LifecycleAction, LifecycleGuard, MAX_MOTION_AXES, StopFeedback};
use esop_ethercat_core::{
    CycleReport, DcCyclicSync, Domain, EthercatMaster, EthercatPort, FramePlan,
};
use esop_procbuf::{HeaderError, ProcBuf, StatePage, StatePublishError};
use esop_profile_cia402::{Cia402AxisBank, Cia402PdoMap, DriveRequest, OperatingMode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopCycleError {
    CycleMismatch,
    AxisCapacityExceeded,
    Header(HeaderError),
    NotStopping(LifecycleAction),
    Evidence(AxisEvidenceError),
}

/// Submission errors do not suppress the State page: `issued_action` stays
/// zero for a failed TX, while a verified response to an *earlier* accepted
/// TX may still acknowledge stopping. A failed State publication leaves the
/// caller-owned page intact for a retry; lifecycle events remain pending.
#[derive(Debug)]
pub struct StopCycleOutcome<E> {
    pub quality: CyclicQuality,
    pub feedback: Option<StopFeedback>,
    pub acknowledged: bool,
    pub transmission: Result<usize, StopFrameError<E>>,
    pub state_publish: Result<u64, StatePublishError>,
    pub event_publish: Option<Result<usize, LifecycleEventError>>,
}

/// All non-CiA 402 outputs in `safe_process_image` must be independently
/// checked by the caller. The Domain has already been finished for `report`;
/// start the next Domain receive only *after* this branch inspects its input.
/// `transition_time_ns` is the timestamp of a previously recorded transition;
/// transitions made during `run` use `now_ns` instead.
pub struct StopCycleContext<
    'a,
    P: EthercatPort,
    const AXES: usize,
    const IO: usize,
    const BYTES: usize,
    const SEGMENTS: usize,
    const DATAGRAMS: usize,
    const SLOTS: usize,
    const MTU: usize,
    const EVENTS: usize,
> {
    pub guard: &'a mut LifecycleGuard,
    pub bank: &'a mut Cia402AxisBank<AXES>,
    pub master: &'a mut EthercatMaster<SLOTS, MTU>,
    pub port: &'a mut P,
    pub domain: &'a Domain<BYTES, SEGMENTS>,
    pub dc: &'a DcCyclicSync,
    pub buffer: &'a ProcBuf<AXES, IO, 1, EVENTS>,
    pub event_cursor: &'a mut LifecycleEventCursor,
    pub state: &'a mut StatePage<AXES, IO, 1>,
    pub report: CycleReport,
    pub other: OtherCycleFacts,
    pub maps: &'a [Cia402PdoMap; AXES],
    pub modes: &'a [OperatingMode; AXES],
    pub max_stationary_velocities: &'a [u32; AXES],
    pub safe_process_image: &'a [u8; BYTES],
    pub plan: &'a FramePlan<DATAGRAMS>,
    pub next_generation: u16,
    pub deadline_ns: u64,
    pub now_ns: u64,
    pub transition_time_ns: u64,
}

impl<
    P: EthercatPort,
    const AXES: usize,
    const IO: usize,
    const BYTES: usize,
    const SEGMENTS: usize,
    const DATAGRAMS: usize,
    const SLOTS: usize,
    const MTU: usize,
    const EVENTS: usize,
> StopCycleContext<'_, P, AXES, IO, BYTES, SEGMENTS, DATAGRAMS, SLOTS, MTU, EVENTS>
{
    /// Project the completed RX cycle, submit the next stop frame, then
    /// publish causally matched State and transition events. `NotStopping`
    /// means the caller must route the cycle through its normal motion or
    /// inhibited-output branch; this stop-only owner does not send that frame.
    pub fn run(&mut self) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        if self.report.cycle == 0
            || self.state.sequence != self.report.cycle
            || self.master.cycle_number() != self.report.cycle
        {
            return Err(StopCycleError::CycleMismatch);
        }
        if AXES > MAX_MOTION_AXES {
            return Err(StopCycleError::AxisCapacityExceeded);
        }
        let axes_mask = if AXES == MAX_MOTION_AXES {
            u32::MAX
        } else {
            (1u32 << AXES) - 1
        };
        if (self.guard.policy.allowed_axis_mask | self.guard.motion_axes_mask) & !axes_mask != 0 {
            return Err(StopCycleError::AxisCapacityExceeded);
        }
        self.buffer
            .validate_header(self.buffer.header().robot_id, self.guard.boot_id)
            .map_err(StopCycleError::Header)?;
        if self.state.boot_id != self.guard.boot_id {
            return Err(StopCycleError::Header(HeaderError::BootIdMismatch));
        }

        let previous_transition_sequence = self.guard.transition_sequence;
        let quality = ethercat_cycle_to_procbuf(
            self.state,
            self.report,
            &[self.domain.quality()],
            &[true],
            self.dc,
            self.other,
        );
        self.guard.update_cyclic_quality(quality, self.report.cycle);
        let mut decision = self.guard.cycle_axes(self.report.cycle, self.now_ns);
        if !matches!(decision.action(), LifecycleAction::Stop(_)) {
            return Err(StopCycleError::NotStopping(decision.action()));
        }

        let statuswords = core::array::from_fn(|axis| {
            if quality.domain_valid && quality.wkc_valid {
                self.maps[axis]
                    .read_inputs_for(self.domain.input(), self.modes[axis])
                    .map_or(u16::MAX, |input| input.statusword)
            } else {
                u16::MAX
            }
        });
        let outputs = step_axis_bank(
            self.bank,
            &decision,
            statuswords,
            [DriveRequest::Disable; AXES],
        );
        let feedback = verified_ethercat_stop_feedback(
            &decision,
            self.report,
            self.domain,
            self.maps,
            self.modes,
            self.max_stationary_velocities,
        );
        let transmission = submit_stopping_frame(
            &mut decision,
            &outputs,
            self.maps,
            self.modes,
            self.safe_process_image,
            self.domain,
            self.plan,
            self.master,
            self.port,
            self.next_generation,
            self.deadline_ns,
        );
        axis_stops_to_procbuf(self.state, &decision, &outputs, feedback)
            .map_err(StopCycleError::Evidence)?;
        let stop_mask = decision.stopping_axis_mask();
        // The borrowed decision must end before acknowledgment or snapshot.
        let acknowledged = feedback
            .filter(|proof| {
                proof.observed_axis_mask & stop_mask == stop_mask
                    && proof.stationary_axis_mask & stop_mask == stop_mask
                    && proof.non_enabled_axis_mask & stop_mask == stop_mask
            })
            .is_some_and(|proof| {
                self.guard
                    .acknowledge_stopped(self.report.cycle, proof)
                    .is_ok()
            });
        self.state.lifecycle = lifecycle_to_procbuf(
            self.guard.snapshot(self.report.cycle, self.now_ns),
            if self.guard.transition_sequence != previous_transition_sequence {
                self.now_ns
            } else {
                self.transition_time_ns
            },
        );
        let state_publish = self.buffer.publish_state(*self.state);
        let event_publish = state_publish.as_ref().ok().map(|_| {
            lifecycle_events_to_procbuf(self.guard, self.buffer, self.event_cursor, self.now_ns)
        });
        Ok(StopCycleOutcome {
            quality,
            feedback,
            acknowledged,
            transmission,
            state_publish,
            event_publish,
        })
    }
}
