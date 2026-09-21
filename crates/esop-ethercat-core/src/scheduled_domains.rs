//! Fixed-capacity receive ownership for a frozen multi-rate Domain schedule.

use crate::control::{
    ControlError, ControlExpiry, ControlRequestPool, ControlRxConsumer, RequestHandle, RequestState,
};
use crate::dc::{DcCyclicError, DcCyclicSync};
use crate::domain::{Domain, DomainError, DomainQuality, DomainSegment};
use crate::engine::{CycleError, CycleReport, EthercatMaster, RxConsumerMux, RxDatagramConsumer};
use crate::frame_pool::FramePoolError;
use crate::mailbox::{MailboxController, MailboxError, MailboxProgress};
use crate::plan::{FramePlan, FramePlanSet};
use crate::port::{EthercatPort, LinkState};
use crate::rx_index::RxMatch;
use crate::schedule::ScheduleTable;
use crate::wire::{Command, DatagramHeader, MAX_ETHERNET_FRAME_LEN};
use core::any::Any;
use core::convert::Infallible;

mod private {
    pub trait Sealed {}
}

/// The receive operations required from a configured Domain. Implemented by
/// `Domain` for any fixed image and segment capacity. External implementations
/// cannot bypass the activation-time segment and receive-state checks.
pub trait ScheduledDomainRx: private::Sealed + RxDatagramConsumer {
    fn as_any(&self) -> &dyn Any;
    fn logical_address(&self) -> u32;
    fn segments(&self) -> &[DomainSegment];
    fn receive_generation(&self) -> Option<u16>;
    fn begin_receive(&mut self, generation: u16) -> Result<(), DomainError>;
    fn finish_receive(&mut self, generation: u16, cycle: u64) -> Result<bool, DomainError>;
    fn quality(&self) -> DomainQuality;
}

impl<const BYTES: usize, const SEGMENTS: usize> private::Sealed for Domain<BYTES, SEGMENTS> {}

impl<const BYTES: usize, const SEGMENTS: usize> ScheduledDomainRx for Domain<BYTES, SEGMENTS> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn logical_address(&self) -> u32 {
        Domain::logical_address(self)
    }

    fn segments(&self) -> &[DomainSegment] {
        Domain::segments(self)
    }

    fn receive_generation(&self) -> Option<u16> {
        Domain::receive_generation(self)
    }

    fn begin_receive(&mut self, generation: u16) -> Result<(), DomainError> {
        Domain::begin_receive(self, generation)
    }

    fn finish_receive(&mut self, generation: u16, cycle: u64) -> Result<bool, DomainError> {
        Domain::finish_receive(self, generation, cycle)
    }

    fn quality(&self) -> DomainQuality {
        Domain::quality(self)
    }
}

pub struct ScheduledDomainEntry<'a> {
    pub id: u8,
    pub domain: &'a mut dyn ScheduledDomainRx,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledDomainError {
    InvalidSchedule,
    EmptyDomain(usize),
    DuplicateDatagramIndex(u8),
    CycleOrder,
    CycleInProgress,
    DomainBusy(usize),
    Domain(usize, DomainError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledReceiveError {
    DcIndexConflict(u8),
    DcGenerationMismatch,
    ControlIndexConflict(u8),
    MailboxRequestMismatch,
    Domain(ScheduledDomainError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledServiceTxError {
    CycleOrder,
    InvalidDeadline,
    DcIndexConflict(u8),
    ControlIndexConflict(u8),
    InvalidControlRequest,
    ControlGenerationMismatch,
    ControlDeadlineExpired,
    Dc(DcCyclicError),
}

#[derive(Debug)]
pub enum ScheduledServiceFrameError<E> {
    FramePool(FramePoolError),
    Build(CycleError<Infallible>),
    Transmit(CycleError<E>),
}

#[derive(Debug)]
pub enum ScheduledServiceTxFailure<E> {
    Dc(ScheduledServiceFrameError<E>),
    Control(ScheduledServiceFrameError<E>),
    Deadline,
}

/// The DC pending generation remains open after a failed submission so the
/// shared RX owner retires it as missing. A failed control submission becomes
/// terminal immediately; its service must consume and release that request.
#[derive(Debug)]
pub struct ScheduledServiceTxReport<E> {
    pub dc_sent: bool,
    pub control_sent: bool,
    pub failure: Option<ScheduledServiceTxFailure<E>>,
    pub post_tx_deadline_met: bool,
}

/// Even an RX port error returns a cycle report tied to the real master
/// cycle. The conservative budget miss blocks motion while retaining the
/// precise transport error for diagnostics; `generation` is the generation
/// actually finalized by the bank. Domain and DC pending state have already
/// been finalized so they cannot leak into the next generation.
#[derive(Debug)]
pub struct ScheduledReceiveReport<E, const DOMAINS: usize> {
    pub report: CycleReport,
    pub generation: u16,
    pub qualities: [DomainQuality; DOMAINS],
    pub dc_result: Result<(), DcCyclicError>,
    pub transport_error: Option<CycleError<E>>,
    /// Newly expired control requests, empty for `receive_with_dc`.
    /// Their owners must consume and release the failed requests.
    pub control_expiry: ControlExpiry,
}

/// A terminal mailbox request is consumed after the shared RX finalizer, so
/// its service FSM sees the completed response or the precise failure reason.
/// `mailbox_progress` is None while its request remains Prepared/InFlight or
/// when no mailbox request was supplied. Its error must not be ignored by the
/// lifecycle owner when determining non-bus readiness.
#[derive(Debug)]
pub struct ScheduledMailboxReceiveReport<E, const DOMAINS: usize> {
    pub received: ScheduledReceiveReport<E, DOMAINS>,
    pub mailbox_progress: Option<Result<MailboxProgress, MailboxError>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledMailboxTxError {
    Mailbox(MailboxError),
    Control(ControlError),
    Service(ScheduledServiceTxError),
    RequestMismatch,
}

/// Carries the mailbox request across cycles until the shared RX owner
/// consumes it. A None request means no mailbox action was due or a Prepared
/// request was released because it never reached the wire.
#[derive(Debug)]
pub struct ScheduledMailboxTxReport<E> {
    pub service: ScheduledServiceTxReport<E>,
    pub request: Option<RequestHandle>,
}

/// Result of the bounded mailbox service stage spanning service TX through
/// the common Domain/DC/control RX finalizer. `request` is the only handle
/// the caller may carry into the next service cycle; it is cleared once a
/// terminal mailbox result has been consumed. The deadline observation ends
/// after RX and is not the final production-cycle deadline after process
/// outputs and lifecycle publication.
#[derive(Debug)]
pub struct ScheduledMailboxCycleReport<E, const DOMAINS: usize> {
    pub tx: ScheduledMailboxTxReport<E>,
    pub receive: ScheduledMailboxReceiveReport<E, DOMAINS>,
    pub request: Option<RequestHandle>,
    pub post_receive_deadline_met: bool,
}

/// A submit rejection has not prepared DC and leaves any caller-owned request
/// with the caller. A receive rejection after a valid submit retains the TX
/// report (including its request handle), but represents a violated cyclic
/// invariant that must fault or reinitialize the service stage rather than
/// continue motion with an open DC generation.
#[derive(Debug)]
pub enum ScheduledMailboxCycleError<E> {
    Submit(ScheduledMailboxTxError),
    Receive {
        tx: ScheduledMailboxTxReport<E>,
        error: ScheduledReceiveError,
    },
}

/// Frozen process image and frame plans for one scheduled Domain. The image
/// is read-only on the cyclic path; callers publish a new immutable binding
/// only while the production cycle is stopped.
#[derive(Clone, Copy)]
pub struct ScheduledProcessInputEntry<'a, const FRAMES: usize, const DATAGRAMS: usize> {
    pub id: u8,
    pub image: &'a [u8],
    pub plans: &'a FramePlanSet<FRAMES, DATAGRAMS>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledProcessInputPlanError {
    InvalidSchedule,
    DomainMismatch { expected: u8, actual: u8 },
    InvalidPlan(u8),
    DuplicateIndex(u8),
    OverlappingWrite(u8, u8),
}

/// Activation-time binding between the frozen multi-rate schedule and every
/// Domain frame submitted before the common RX stage. Runtime traversal is
/// bounded by the const capacities and performs no plan search or allocation.
pub struct ScheduledProcessInputs<
    'a,
    const DOMAINS: usize,
    const SCHEDULE_SLOTS: usize,
    const FRAMES: usize,
    const DATAGRAMS: usize,
> {
    schedule: &'a ScheduleTable<DOMAINS, SCHEDULE_SLOTS>,
    entries: [ScheduledProcessInputEntry<'a, FRAMES, DATAGRAMS>; DOMAINS],
}

#[derive(Debug)]
pub enum ScheduledProcessFrameError<E> {
    InvalidDeadline,
    FramePool(FramePoolError),
    Build(CycleError<Infallible>),
    Transmit(CycleError<E>),
}

#[derive(Debug)]
pub struct ScheduledProcessTxFailure<E> {
    pub domain_id: u8,
    pub frame_index: usize,
    pub error: ScheduledProcessFrameError<E>,
}

/// Evidence for the process-Domain submission stage preceding DC/control TX.
/// `generation` and `rx_deadline_ns` identify the future shared RX armed by
/// this submission. `expected_frames` is derived from the frozen due mask. A
/// failure identifies the first frame not accepted by the port; later due
/// frames are not tried.
#[derive(Debug)]
pub struct ScheduledProcessTxReport<E> {
    pub cycle: u64,
    pub generation: u16,
    pub rx_deadline_ns: u64,
    pub due_mask: u64,
    pub expected_frames: usize,
    pub sent_frames: usize,
    pub failure: Option<ScheduledProcessTxFailure<E>>,
    pub post_tx_deadline_met: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledProcessTxError {
    InvalidBinding,
    CycleOrder,
}

impl<
    'a,
    const DOMAINS: usize,
    const SCHEDULE_SLOTS: usize,
    const FRAMES: usize,
    const DATAGRAMS: usize,
> ScheduledProcessInputs<'a, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>
{
    pub fn new(
        bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        schedule: &'a ScheduleTable<DOMAINS, SCHEDULE_SLOTS>,
        entries: [ScheduledProcessInputEntry<'a, FRAMES, DATAGRAMS>; DOMAINS],
    ) -> Result<Self, ScheduledProcessInputPlanError> {
        if !bank.uses_schedule(schedule) || schedule.domain_count() != DOMAINS {
            return Err(ScheduledProcessInputPlanError::InvalidSchedule);
        }

        let mut indices = [false; 256];
        for (slot, (configured, entry)) in schedule.domains().iter().zip(&entries).enumerate() {
            if entry.id != configured.id {
                return Err(ScheduledProcessInputPlanError::DomainMismatch {
                    expected: configured.id,
                    actual: entry.id,
                });
            }
            if entry.plans.is_empty() || !bank.matches_frame_plans(entry.id, entry.plans) {
                return Err(ScheduledProcessInputPlanError::InvalidPlan(entry.id));
            }
            for (plan_index, plan) in entry.plans.plans().iter().enumerate() {
                for (datagram_index, datagram) in plan.datagrams().iter().enumerate() {
                    if !matches!(datagram.command, Command::Lrd | Command::Lwr | Command::Lrw)
                        || datagram.payload_len == 0
                        || datagram.expected_wkc == 0
                        || datagram
                            .payload_offset
                            .checked_add(datagram.payload_len)
                            .is_none_or(|end| end > entry.image.len())
                    {
                        return Err(ScheduledProcessInputPlanError::InvalidPlan(entry.id));
                    }
                    if indices[datagram.index as usize] {
                        return Err(ScheduledProcessInputPlanError::DuplicateIndex(
                            datagram.index,
                        ));
                    }
                    indices[datagram.index as usize] = true;
                    if !matches!(datagram.command, Command::Lwr | Command::Lrw) {
                        continue;
                    }

                    for prior in &entries[..slot] {
                        for earlier in prior.plans.plans().iter().flat_map(|plan| plan.datagrams())
                        {
                            if process_writable_overlap(datagram, earlier) {
                                return Err(ScheduledProcessInputPlanError::OverlappingWrite(
                                    entry.id, prior.id,
                                ));
                            }
                        }
                    }
                    for (earlier_plan_index, earlier_plan) in
                        entry.plans.plans()[..=plan_index].iter().enumerate()
                    {
                        let limit = if earlier_plan_index == plan_index {
                            datagram_index
                        } else {
                            earlier_plan.len()
                        };
                        for earlier in &earlier_plan.datagrams()[..limit] {
                            if process_writable_overlap(datagram, earlier) {
                                return Err(ScheduledProcessInputPlanError::OverlappingWrite(
                                    entry.id, entry.id,
                                ));
                            }
                        }
                    }
                }
            }
        }

        Ok(Self { schedule, entries })
    }

    pub const fn schedule(&self) -> &ScheduleTable<DOMAINS, SCHEDULE_SLOTS> {
        self.schedule
    }
}

fn process_writable_overlap(
    left: &crate::plan::DatagramPlan,
    right: &crate::plan::DatagramPlan,
) -> bool {
    matches!(right.command, Command::Lwr | Command::Lrw)
        && left.address < right.address.saturating_add(right.payload_len as u32)
        && right.address < left.address.saturating_add(left.payload_len as u32)
}

/// Owns the receive borrow for all configured Domains. A single master RX
/// session dispatches only verified datagrams to Domains due on this tick.
/// Use `receive_with_dc` when DC is enabled so a port error cannot skip
/// Domain or DC receive finalization. `receive_with_dc_and_control` also
/// dispatches in-flight control requests through that RX session. For other
/// receive arrangements the
/// caller must pair `begin_due` and `finish_due`, including on RX errors.
/// The caller still owns verified TX plans, control service progression, and
/// the final cycle deadline. Pass the resulting qualities to the lifecycle schedule
/// projection in the same order.
pub struct ScheduledDomainBank<'a, const DOMAINS: usize, const SLOTS: usize> {
    schedule: &'a ScheduleTable<DOMAINS, SLOTS>,
    domains: [ScheduledDomainEntry<'a>; DOMAINS],
    index_owner: [u8; 256],
    active: Option<(u64, u16, u64)>,
    last_cycle: u64,
    last_generation: u16,
}

impl<'a, const DOMAINS: usize, const SLOTS: usize> ScheduledDomainBank<'a, DOMAINS, SLOTS> {
    /// One bounded master RX step for every due Domain and the same-generation
    /// DC response. Requires `dc.prepare` and arming its frozen FRMW plan
    /// before this call. No response, link loss, or port error may skip either
    /// receive finalizer. The caller still owns TX and lifecycle publication.
    pub fn receive_with_dc<P: EthercatPort, const FRAMES: usize, const MTU: usize>(
        &mut self,
        master: &mut EthercatMaster<FRAMES, MTU>,
        port: &mut P,
        scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN],
        generation: u16,
        dc: &mut DcCyclicSync,
    ) -> Result<ScheduledReceiveReport<P::Error, DOMAINS>, ScheduledReceiveError> {
        self.receive_with_dc_consumer(master, port, scratch, generation, dc, &mut ())
    }

    /// Share the same bounded RX poll with in-flight control requests. The
    /// control owner remains responsible for interpreting completed/failed
    /// requests and consuming `control_expiry` before reusing their slots.
    /// A concurrent control index must never
    /// alias a Domain, DC, or another in-flight control request.
    pub fn receive_with_dc_and_control<
        P: EthercatPort,
        const FRAMES: usize,
        const MTU: usize,
        const REQUESTS: usize,
    >(
        &mut self,
        master: &mut EthercatMaster<FRAMES, MTU>,
        port: &mut P,
        scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN],
        generation: u16,
        dc: &mut DcCyclicSync,
        controls: &mut ControlRequestPool<REQUESTS>,
    ) -> Result<ScheduledReceiveReport<P::Error, DOMAINS>, ScheduledReceiveError> {
        if let Some(index) = self.control_index_conflict(dc.datagram_plan().index, controls, None) {
            return Err(ScheduledReceiveError::ControlIndexConflict(index));
        }
        let mut received = {
            let mut control_consumer = ControlRxConsumer::new(controls);
            self.receive_with_dc_consumer(
                master,
                port,
                scratch,
                generation,
                dc,
                &mut control_consumer,
            )?
        };
        let now_ns = port.now_ns();
        received.control_expiry = controls.expire_in_flight(now_ns);
        if !received.control_expiry.is_empty() {
            master.reap_expired_rx_before_tx(now_ns);
        }
        Ok(received)
    }

    /// Complete one mailbox request after the common Domain/DC/control RX.
    /// Check action ownership before starting RX; an unrelated pool handle
    /// must not fault the mailbox FSM or consume a different service's slot.
    /// Prepared requests (e.g. skipped after a DC TX failure) remain owned by
    /// the caller, and an in-flight request is retained until it completes or
    /// expires. A terminal request is consumed and released by the mailbox.
    #[allow(clippy::too_many_arguments)]
    pub fn receive_with_dc_and_mailbox<
        P: EthercatPort,
        const FRAMES: usize,
        const MTU: usize,
        const REQUESTS: usize,
    >(
        &mut self,
        master: &mut EthercatMaster<FRAMES, MTU>,
        port: &mut P,
        scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN],
        generation: u16,
        dc: &mut DcCyclicSync,
        controls: &mut ControlRequestPool<REQUESTS>,
        mailbox: &mut MailboxController,
        request: Option<RequestHandle>,
    ) -> Result<ScheduledMailboxReceiveReport<P::Error, DOMAINS>, ScheduledReceiveError> {
        if let Some(handle) = request {
            if !mailbox_request_matches(mailbox, controls, handle) {
                return Err(ScheduledReceiveError::MailboxRequestMismatch);
            }
        }
        let received =
            self.receive_with_dc_and_control(master, port, scratch, generation, dc, controls)?;
        let mailbox_progress = request.and_then(|handle| {
            let state = controls.get(handle)?.state;
            matches!(state, RequestState::Complete | RequestState::Failed)
                .then(|| mailbox.accept_completed(controls, handle, port.now_ns()))
        });
        Ok(ScheduledMailboxReceiveReport {
            received,
            mailbox_progress,
        })
    }

    /// Prepare at most one mailbox request and submit it with this cycle's DC
    /// sample. An existing InFlight request is retained without retransmit.
    /// A request created here is released on preflight failure. After a valid
    /// service attempt, any request left Prepared is released and returned as
    /// None; the mailbox action remains pending and can be re-enqueued. A
    /// caller-owned request is preserved when preflight returns an error.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_dc_and_mailbox<
        P: EthercatPort,
        const FRAMES: usize,
        const MTU: usize,
        const REQUESTS: usize,
    >(
        &self,
        master: &mut EthercatMaster<FRAMES, MTU>,
        port: &mut P,
        dc: &mut DcCyclicSync,
        dc_image: &mut [u8],
        application_time_ns: u64,
        controls: &mut ControlRequestPool<REQUESTS>,
        mailbox: &mut MailboxController,
        request: Option<RequestHandle>,
        generation: u16,
        rx_deadline_ns: u64,
        cycle_deadline_ns: u64,
    ) -> Result<ScheduledMailboxTxReport<P::Error>, ScheduledMailboxTxError> {
        let mut created = false;
        let mut request = if let Some(handle) = request {
            if !mailbox_request_matches(mailbox, controls, handle)
                || !matches!(
                    controls.get(handle).map(|item| item.state),
                    Some(RequestState::Prepared | RequestState::InFlight)
                )
            {
                return Err(ScheduledMailboxTxError::RequestMismatch);
            }
            Some(handle)
        } else if mailbox
            .next_action(port.now_ns())
            .map_err(ScheduledMailboxTxError::Mailbox)?
            .is_some()
        {
            created = true;
            Some(
                mailbox
                    .enqueue_pending(controls)
                    .map_err(ScheduledMailboxTxError::Control)?,
            )
        } else {
            None
        };
        let prepared = request.filter(|handle| {
            controls
                .get(*handle)
                .is_some_and(|item| item.state == RequestState::Prepared)
        });
        let service = match self.submit_dc_and_control(
            master,
            port,
            dc,
            dc_image,
            application_time_ns,
            controls,
            prepared,
            generation,
            rx_deadline_ns,
            cycle_deadline_ns,
        ) {
            Ok(report) => report,
            Err(error) => {
                if created && let Some(handle) = prepared {
                    controls
                        .release(handle)
                        .map_err(ScheduledMailboxTxError::Control)?;
                }
                return Err(ScheduledMailboxTxError::Service(error));
            }
        };
        if let Some(handle) = request {
            if controls
                .get(handle)
                .is_some_and(|item| item.state == RequestState::Prepared)
            {
                controls
                    .release(handle)
                    .map_err(ScheduledMailboxTxError::Control)?;
                request = None;
            }
        }
        Ok(ScheduledMailboxTxReport { service, request })
    }

    /// Own one bounded mailbox service stage from DC/control submission through
    /// the shared RX finalizer. Process-Domain frames for this generation must
    /// already be armed and submitted before entry. Once submission succeeds,
    /// RX is always attempted even when the returned TX report contains a send
    /// failure, ensuring a prepared DC generation is retired as missing rather
    /// than leaking into the next cycle.
    ///
    /// This method does not submit process outputs, project service failures
    /// into lifecycle safety facts, publish State/events, or establish the
    /// production cycle's final deadline. The caller must perform those stages
    /// and treat either TX failure or a false post-RX deadline as unsafe.
    #[allow(clippy::too_many_arguments)]
    pub fn run_dc_and_mailbox_cycle<
        P: EthercatPort,
        const FRAMES: usize,
        const MTU: usize,
        const REQUESTS: usize,
    >(
        &mut self,
        master: &mut EthercatMaster<FRAMES, MTU>,
        port: &mut P,
        scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN],
        dc: &mut DcCyclicSync,
        dc_image: &mut [u8],
        application_time_ns: u64,
        controls: &mut ControlRequestPool<REQUESTS>,
        mailbox: &mut MailboxController,
        request: Option<RequestHandle>,
        generation: u16,
        rx_deadline_ns: u64,
        cycle_deadline_ns: u64,
    ) -> Result<ScheduledMailboxCycleReport<P::Error, DOMAINS>, ScheduledMailboxCycleError<P::Error>>
    {
        let mut tx = self
            .submit_dc_and_mailbox(
                master,
                port,
                dc,
                dc_image,
                application_time_ns,
                controls,
                mailbox,
                request,
                generation,
                rx_deadline_ns,
                cycle_deadline_ns,
            )
            .map_err(ScheduledMailboxCycleError::Submit)?;
        let receive = match self.receive_with_dc_and_mailbox(
            master, port, scratch, generation, dc, controls, mailbox, tx.request,
        ) {
            Ok(receive) => receive,
            Err(error) => return Err(ScheduledMailboxCycleError::Receive { tx, error }),
        };
        if receive.mailbox_progress.is_some() {
            tx.request = None;
        }
        let request = tx.request;
        let post_receive_deadline_met =
            tx.service.post_tx_deadline_met && port.now_ns() < cycle_deadline_ns;
        Ok(ScheduledMailboxCycleReport {
            tx,
            receive,
            request,
            post_receive_deadline_met,
        })
    }

    /// Submit the DC sample and optionally one prepared control request for
    /// the next shared RX generation. Domain output frames are submitted by
    /// their lifecycle owner; this method verifies service indices against
    /// that same bank before any TX or DC preparation. The caller must always
    /// run the shared RX finalizer after a reported send failure, so a missing
    /// DC response cannot reuse the previous lock. Feed a failed deadline or
    /// transmission into the lifecycle budget/safety facts before motion TX.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_dc_and_control<
        P: EthercatPort,
        const FRAMES: usize,
        const MTU: usize,
        const REQUESTS: usize,
    >(
        &self,
        master: &mut EthercatMaster<FRAMES, MTU>,
        port: &mut P,
        dc: &mut DcCyclicSync,
        dc_image: &mut [u8],
        application_time_ns: u64,
        controls: &mut ControlRequestPool<REQUESTS>,
        request: Option<RequestHandle>,
        generation: u16,
        rx_deadline_ns: u64,
        cycle_deadline_ns: u64,
    ) -> Result<ScheduledServiceTxReport<P::Error>, ScheduledServiceTxError> {
        if self.active.is_some() || self.last_cycle != master.cycle_number() {
            return Err(ScheduledServiceTxError::CycleOrder);
        }
        let now_ns = port.now_ns();
        if rx_deadline_ns == 0
            || cycle_deadline_ns == 0
            || now_ns >= rx_deadline_ns
            || now_ns >= cycle_deadline_ns
        {
            return Err(ScheduledServiceTxError::InvalidDeadline);
        }
        let dc_index = dc.datagram_plan().index;
        if self.index_owner[dc_index as usize] != 0 {
            return Err(ScheduledServiceTxError::DcIndexConflict(dc_index));
        }
        if dc.pending_generation().is_some() {
            return Err(ScheduledServiceTxError::Dc(DcCyclicError::Busy));
        }
        let control_deadline_ns = if let Some(handle) = request {
            let control = controls
                .get(handle)
                .ok_or(ScheduledServiceTxError::InvalidControlRequest)?;
            if control.state != RequestState::Prepared {
                return Err(ScheduledServiceTxError::InvalidControlRequest);
            }
            if control.generation != generation {
                return Err(ScheduledServiceTxError::ControlGenerationMismatch);
            }
            if control.deadline_ns <= now_ns {
                return Err(ScheduledServiceTxError::ControlDeadlineExpired);
            }
            Some(control.deadline_ns)
        } else {
            None
        };
        if let Some(index) = self.control_index_conflict(dc_index, controls, request) {
            return Err(ScheduledServiceTxError::ControlIndexConflict(index));
        }
        let mut dc_plan = FramePlan::<1>::new();
        dc_plan
            .push(dc.datagram_plan())
            .map_err(|_| ScheduledServiceTxError::Dc(DcCyclicError::InvalidConfiguration))?;
        dc.prepare(generation, application_time_ns, dc_image)
            .map_err(ScheduledServiceTxError::Dc)?;
        master.reap_expired_rx_before_tx(now_ns);

        let mut report = ScheduledServiceTxReport {
            dc_sent: false,
            control_sent: false,
            failure: None,
            post_tx_deadline_met: false,
        };
        match submit_dc_frame(master, port, generation, rx_deadline_ns, &dc_plan, dc_image) {
            Ok(()) => report.dc_sent = true,
            Err(error) => report.failure = Some(ScheduledServiceTxFailure::Dc(error)),
        }
        if report.failure.is_none() && port.now_ns() >= cycle_deadline_ns {
            report.failure = Some(ScheduledServiceTxFailure::Deadline);
        }
        if report.failure.is_none() {
            if let Some((handle, control_deadline_ns)) = request.zip(control_deadline_ns) {
                if port.now_ns() >= control_deadline_ns {
                    // The request is still Prepared; the service can consume
                    // the terminal error without waiting for an RX timeout.
                    let _ = controls.fail_transmit(handle);
                    report.failure = Some(ScheduledServiceTxFailure::Deadline);
                } else {
                    master.reap_expired_rx_before_tx(port.now_ns());
                    let sent = match master.acquire_frame(generation, control_deadline_ns) {
                        Ok(frame) => match master.build_control_request(controls, handle, frame) {
                            Ok(_) => master
                                .submit_frame(port, frame)
                                .map_err(ScheduledServiceFrameError::Transmit),
                            Err(error) => {
                                let _ = master.release_unarmed_frame(frame);
                                Err(ScheduledServiceFrameError::Build(error))
                            }
                        },
                        Err(error) => Err(ScheduledServiceFrameError::FramePool(error)),
                    };
                    match sent {
                        Ok(()) => report.control_sent = true,
                        Err(error) => {
                            let _ = controls.fail_transmit(handle);
                            report.failure = Some(ScheduledServiceTxFailure::Control(error));
                        }
                    }
                }
            }
        }
        let final_now_ns = port.now_ns();
        report.post_tx_deadline_met = final_now_ns < cycle_deadline_ns;
        if !report.post_tx_deadline_met && report.failure.is_none() {
            report.failure = Some(ScheduledServiceTxFailure::Deadline);
        }
        Ok(report)
    }

    fn control_index_conflict<const REQUESTS: usize>(
        &self,
        dc_index: u8,
        controls: &ControlRequestPool<REQUESTS>,
        prepared: Option<RequestHandle>,
    ) -> Option<u8> {
        let mut claimed = [false; 256];
        for slot in 0..REQUESTS.min(64) {
            let Some(handle) = RequestHandle::from_index(slot) else {
                continue;
            };
            let Some(request) = controls.get(handle) else {
                continue;
            };
            if request.state != RequestState::InFlight && Some(handle) != prepared {
                continue;
            }
            let index = request.datagram_index as usize;
            if self.index_owner[index] != 0 || index == dc_index as usize || claimed[index] {
                return Some(index as u8);
            }
            claimed[index] = true;
        }
        None
    }

    fn receive_with_dc_consumer<
        P: EthercatPort,
        C: RxDatagramConsumer,
        const FRAMES: usize,
        const MTU: usize,
    >(
        &mut self,
        master: &mut EthercatMaster<FRAMES, MTU>,
        port: &mut P,
        scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN],
        generation: u16,
        dc: &mut DcCyclicSync,
        control: &mut C,
    ) -> Result<ScheduledReceiveReport<P::Error, DOMAINS>, ScheduledReceiveError> {
        let dc_index = dc.datagram_plan().index;
        if self.index_owner[dc_index as usize] != 0 {
            return Err(ScheduledReceiveError::DcIndexConflict(dc_index));
        }
        if dc.pending_generation() != Some(generation) {
            return Err(ScheduledReceiveError::DcGenerationMismatch);
        }
        let cycle = master.cycle_number().wrapping_add(1);
        self.begin_due(cycle, generation)
            .map_err(ScheduledReceiveError::Domain)?;
        let rx = {
            let mut domain_dc = RxConsumerMux::new(&mut *self, &mut *dc);
            let mut consumers = RxConsumerMux::new(&mut domain_dc, control);
            master.cycle_receive_with_consumer(port, scratch, generation, &mut consumers)
        };
        let (report, transport_error) = match rx {
            Ok(report) => (report, None),
            Err(error) => {
                let mut report = CycleReport::new(master.cycle_number());
                report.budget_exhausted = true;
                report.link_down = port.link_state() == LinkState::Down;
                (report, Some(error))
            }
        };
        let dc_result = dc.finish_receive(report.cycle, generation);
        let qualities = self
            .finish_due(report.cycle, generation)
            .map_err(ScheduledReceiveError::Domain)?;
        Ok(ScheduledReceiveReport {
            report,
            generation,
            qualities,
            dc_result,
            transport_error,
            control_expiry: ControlExpiry::EMPTY,
        })
    }

    pub fn uses_schedule(&self, schedule: &ScheduleTable<DOMAINS, SLOTS>) -> bool {
        core::ptr::eq(self.schedule, schedule)
    }

    /// Submit every process-Domain frame due for the next shared RX cycle.
    /// A frame failure stops later submissions but remains a reportable stage
    /// outcome so the service owner can still execute the common RX finalizer
    /// and invalidate every missing due Domain.
    pub fn submit_due_process_inputs<
        P: EthercatPort,
        const MASTER_FRAMES: usize,
        const MTU: usize,
        const PROCESS_FRAMES: usize,
        const DATAGRAMS: usize,
    >(
        &self,
        inputs: &ScheduledProcessInputs<'_, DOMAINS, SLOTS, PROCESS_FRAMES, DATAGRAMS>,
        master: &mut EthercatMaster<MASTER_FRAMES, MTU>,
        port: &mut P,
        generation: u16,
        rx_deadline_ns: u64,
        cycle_deadline_ns: u64,
    ) -> Result<ScheduledProcessTxReport<P::Error>, ScheduledProcessTxError> {
        if !core::ptr::eq(self.schedule, inputs.schedule) {
            return Err(ScheduledProcessTxError::InvalidBinding);
        }
        if self.active.is_some() || self.last_cycle != master.cycle_number() {
            return Err(ScheduledProcessTxError::CycleOrder);
        }

        let cycle = master.cycle_number().wrapping_add(1);
        if cycle == 0 {
            return Err(ScheduledProcessTxError::CycleOrder);
        }
        let tick = (cycle - 1) % u64::from(self.schedule.hyperperiod_ticks());
        let due_mask = self.schedule.due_mask(tick as u32);
        let expected_frames = inputs
            .entries
            .iter()
            .filter(|entry| due_mask & (1u64 << entry.id) != 0)
            .map(|entry| entry.plans.frame_count())
            .sum();
        let mut report = ScheduledProcessTxReport {
            cycle,
            generation,
            rx_deadline_ns,
            due_mask,
            expected_frames,
            sent_frames: 0,
            failure: None,
            post_tx_deadline_met: false,
        };

        'domains: for entry in &inputs.entries {
            if due_mask & (1u64 << entry.id) == 0 {
                continue;
            }
            for (frame_index, plan) in entry.plans.plans().iter().enumerate() {
                let now_ns = port.now_ns();
                let result = if rx_deadline_ns == 0
                    || cycle_deadline_ns == 0
                    || now_ns >= rx_deadline_ns
                    || now_ns >= cycle_deadline_ns
                {
                    Err(ScheduledProcessFrameError::InvalidDeadline)
                } else {
                    master.reap_expired_rx_before_tx(now_ns);
                    match master.acquire_frame(generation, rx_deadline_ns) {
                        Err(error) => Err(ScheduledProcessFrameError::FramePool(error)),
                        Ok(frame) => {
                            match master.build_and_arm_frame_from_plan(frame, plan, entry.image) {
                                Err(error) => {
                                    let _ = master.release_unarmed_frame(frame);
                                    Err(ScheduledProcessFrameError::Build(error))
                                }
                                Ok(_) => master
                                    .submit_frame(port, frame)
                                    .map_err(ScheduledProcessFrameError::Transmit),
                            }
                        }
                    }
                };
                if let Err(error) = result {
                    report.failure = Some(ScheduledProcessTxFailure {
                        domain_id: entry.id,
                        frame_index,
                        error,
                    });
                    break 'domains;
                }
                report.sent_frames += 1;
            }
        }
        let now_ns = port.now_ns();
        report.post_tx_deadline_met = rx_deadline_ns != 0
            && cycle_deadline_ns != 0
            && now_ns < rx_deadline_ns
            && now_ns < cycle_deadline_ns;
        Ok(report)
    }

    /// Confirm that a process submission report belongs to this bank's most
    /// recently finalized cycle and exactly matches the frozen due traversal.
    pub fn confirms_process_tx<E, const PROCESS_FRAMES: usize, const DATAGRAMS: usize>(
        &self,
        inputs: &ScheduledProcessInputs<'_, DOMAINS, SLOTS, PROCESS_FRAMES, DATAGRAMS>,
        report: &ScheduledProcessTxReport<E>,
    ) -> bool {
        if self.active.is_some()
            || self.last_cycle == 0
            || self.last_cycle != report.cycle
            || self.last_generation != report.generation
            || !core::ptr::eq(self.schedule, inputs.schedule)
        {
            return false;
        }
        self.process_tx_shape_matches(inputs, report)
    }

    /// Validate a process submission before starting the next shared RX.
    /// This is used by the production-cycle owner to record initial priming
    /// without pretending the corresponding receive has already completed.
    pub fn accepts_process_tx<E, const PROCESS_FRAMES: usize, const DATAGRAMS: usize>(
        &self,
        inputs: &ScheduledProcessInputs<'_, DOMAINS, SLOTS, PROCESS_FRAMES, DATAGRAMS>,
        report: &ScheduledProcessTxReport<E>,
    ) -> bool {
        self.active.is_none()
            && report.cycle != 0
            && self.last_cycle.checked_add(1) == Some(report.cycle)
            && core::ptr::eq(self.schedule, inputs.schedule)
            && self.process_tx_shape_matches(inputs, report)
    }

    fn process_tx_shape_matches<E, const PROCESS_FRAMES: usize, const DATAGRAMS: usize>(
        &self,
        inputs: &ScheduledProcessInputs<'_, DOMAINS, SLOTS, PROCESS_FRAMES, DATAGRAMS>,
        report: &ScheduledProcessTxReport<E>,
    ) -> bool {
        if report.rx_deadline_ns == 0 {
            return false;
        }
        let tick = (report.cycle - 1) % u64::from(self.schedule.hyperperiod_ticks());
        let due_mask = self.schedule.due_mask(tick as u32);
        let expected_frames: usize = inputs
            .entries
            .iter()
            .filter(|entry| due_mask & (1u64 << entry.id) != 0)
            .map(|entry| entry.plans.frame_count())
            .sum();
        if report.due_mask != due_mask
            || report.expected_frames != expected_frames
            || report.sent_frames > expected_frames
        {
            return false;
        }

        let Some(failure) = report.failure.as_ref() else {
            return report.sent_frames == expected_frames;
        };
        if matches!(failure.error, ScheduledProcessFrameError::InvalidDeadline)
            && report.post_tx_deadline_met
        {
            return false;
        }
        let mut ordinal = 0usize;
        for entry in &inputs.entries {
            if due_mask & (1u64 << entry.id) == 0 {
                continue;
            }
            if entry.id == failure.domain_id {
                return failure.frame_index < entry.plans.frame_count()
                    && report.sent_frames == ordinal.saturating_add(failure.frame_index);
            }
            ordinal = ordinal.saturating_add(entry.plans.frame_count());
        }
        false
    }

    /// Confirm that a shared RX report still describes this bank's most
    /// recently finalized cycle and its actual Domain qualities.
    pub fn confirms_receive<E>(&self, received: &ScheduledReceiveReport<E, DOMAINS>) -> bool {
        self.active.is_none()
            && self.last_cycle != 0
            && received.report.cycle == self.last_cycle
            && received.generation == self.last_generation
            && self
                .domains
                .iter()
                .zip(received.qualities)
                .all(|(entry, quality)| entry.domain.quality() == quality)
    }

    /// Validate the ID order and exclusive datagram-index ownership once at
    /// activation, not on the cyclic RX path.
    pub fn new(
        schedule: &'a ScheduleTable<DOMAINS, SLOTS>,
        domains: [ScheduledDomainEntry<'a>; DOMAINS],
    ) -> Result<Self, ScheduledDomainError> {
        if DOMAINS == 0
            || DOMAINS != schedule.domain_count()
            || domains
                .iter()
                .zip(schedule.domains())
                .any(|(domain, configured)| domain.id != configured.id)
        {
            return Err(ScheduledDomainError::InvalidSchedule);
        }
        let mut index_owner = [0u8; 256];
        for (slot, entry) in domains.iter().enumerate() {
            if entry.domain.segments().is_empty() {
                return Err(ScheduledDomainError::EmptyDomain(slot));
            }
            if entry.domain.receive_generation().is_some() {
                return Err(ScheduledDomainError::DomainBusy(slot));
            }
            for segment in entry.domain.segments() {
                let owner = &mut index_owner[segment.datagram_index as usize];
                if *owner != 0 {
                    return Err(ScheduledDomainError::DuplicateDatagramIndex(
                        segment.datagram_index,
                    ));
                }
                *owner = slot as u8 + 1;
            }
        }
        Ok(Self {
            schedule,
            domains,
            index_owner,
            active: None,
            last_cycle: 0,
            last_generation: 0,
        })
    }

    /// Read the actual committed image of a bound Domain after `finish_due`.
    /// The caller must supply its configured image and segment capacities;
    /// mismatched IDs or shapes cannot be cast into a motion Domain.
    pub fn domain<const BYTES: usize, const SEGMENTS: usize>(
        &self,
        id: u8,
    ) -> Option<&Domain<BYTES, SEGMENTS>> {
        if self.active.is_some() {
            return None;
        }
        self.domains
            .iter()
            .find(|entry| entry.id == id)?
            .domain
            .as_any()
            .downcast_ref()
    }

    /// Bind every TX datagram to exactly one actual RX segment of this Domain.
    /// A split frame plan may rearrange frame boundaries but not addresses,
    /// indices, offsets, lengths, or WKC ownership.
    pub fn matches_frame_plans<const FRAMES: usize, const DATAGRAMS: usize>(
        &self,
        id: u8,
        plans: &FramePlanSet<FRAMES, DATAGRAMS>,
    ) -> bool {
        let Some(entry) = self.domains.iter().find(|entry| entry.id == id) else {
            return false;
        };
        let segments = entry.domain.segments();
        plans.datagram_count() == segments.len()
            && segments.iter().all(|segment| {
                plans
                    .plans()
                    .iter()
                    .flat_map(|plan| plan.datagrams())
                    .any(|datagram| {
                        datagram.index == segment.datagram_index
                            && datagram.payload_offset == segment.input_offset
                            && datagram.payload_len == segment.len
                            && datagram.expected_wkc == segment.expected_wkc
                            && u32::try_from(segment.input_offset).ok().and_then(|offset| {
                                entry.domain.logical_address().checked_add(offset)
                            }) == Some(datagram.address)
                    })
            })
    }

    pub fn matches_frame_plan<const DATAGRAMS: usize>(
        &self,
        id: u8,
        plan: &FramePlan<DATAGRAMS>,
    ) -> bool {
        let Some(entry) = self.domains.iter().find(|entry| entry.id == id) else {
            return false;
        };
        let segments = entry.domain.segments();
        plan.len() == segments.len()
            && segments.iter().all(|segment| {
                plan.datagrams().iter().any(|datagram| {
                    datagram.index == segment.datagram_index
                        && datagram.payload_offset == segment.input_offset
                        && datagram.payload_len == segment.len
                        && datagram.expected_wkc == segment.expected_wkc
                        && u32::try_from(segment.input_offset)
                            .ok()
                            .and_then(|offset| entry.domain.logical_address().checked_add(offset))
                            == Some(datagram.address)
                })
            })
    }

    pub fn begin_due(&mut self, cycle: u64, generation: u16) -> Result<(), ScheduledDomainError> {
        if self.active.is_some() {
            return Err(ScheduledDomainError::CycleInProgress);
        }
        if cycle == 0 || cycle != self.last_cycle.saturating_add(1) {
            return Err(ScheduledDomainError::CycleOrder);
        }
        let tick = (cycle - 1) % u64::from(self.schedule.hyperperiod_ticks());
        let due_mask = self.schedule.due_mask(tick as u32);
        for (slot, entry) in self.domains.iter().enumerate() {
            if due_mask & (1u64 << entry.id) != 0 && entry.domain.receive_generation().is_some() {
                return Err(ScheduledDomainError::DomainBusy(slot));
            }
        }
        for (slot, entry) in self.domains.iter_mut().enumerate() {
            if due_mask & (1u64 << entry.id) != 0 {
                entry
                    .domain
                    .begin_receive(generation)
                    .map_err(|error| ScheduledDomainError::Domain(slot, error))?;
            }
        }
        self.active = Some((cycle, generation, due_mask));
        Ok(())
    }

    /// Even a missing due response is finished, invalidating that Domain's
    /// quality rather than carrying its previous committed image as current.
    pub fn finish_due(
        &mut self,
        cycle: u64,
        generation: u16,
    ) -> Result<[DomainQuality; DOMAINS], ScheduledDomainError> {
        let Some((active_cycle, active_generation, due_mask)) = self.active else {
            return Err(ScheduledDomainError::CycleOrder);
        };
        if cycle != active_cycle || generation != active_generation {
            return Err(ScheduledDomainError::CycleOrder);
        }
        self.active = None;
        for (slot, entry) in self.domains.iter_mut().enumerate() {
            if due_mask & (1u64 << entry.id) != 0 {
                entry
                    .domain
                    .finish_receive(generation, cycle)
                    .map_err(|error| ScheduledDomainError::Domain(slot, error))?;
            }
        }
        self.last_cycle = cycle;
        self.last_generation = generation;
        Ok(core::array::from_fn(|slot| {
            self.domains[slot].domain.quality()
        }))
    }
}

fn mailbox_request_matches<const REQUESTS: usize>(
    mailbox: &MailboxController,
    controls: &ControlRequestPool<REQUESTS>,
    handle: RequestHandle,
) -> bool {
    let Some(action) = mailbox.pending() else {
        return false;
    };
    let Some(item) = controls.get(handle) else {
        return false;
    };
    item.datagram_index == action.datagram_index
        && item.generation == action.generation
        && item.address == action.address
        && item.operation == action.operation
        && item.response_length == action.datagram_len()
        && item.deadline_ns == action.deadline_ns
        && (!matches!(item.state, RequestState::Prepared | RequestState::InFlight)
            || (item.payload().len() == action.datagram_len()
                && item.payload().starts_with(action.payload())
                && item.payload()[action.payload().len()..]
                    .iter()
                    .all(|byte| *byte == 0)))
}

fn submit_dc_frame<P: EthercatPort, const FRAMES: usize, const MTU: usize>(
    master: &mut EthercatMaster<FRAMES, MTU>,
    port: &mut P,
    generation: u16,
    rx_deadline_ns: u64,
    plan: &FramePlan<1>,
    image: &[u8],
) -> Result<(), ScheduledServiceFrameError<P::Error>> {
    let frame = master
        .acquire_frame(generation, rx_deadline_ns)
        .map_err(ScheduledServiceFrameError::FramePool)?;
    if let Err(error) = master.build_and_arm_frame_from_plan(frame, plan, image) {
        let _ = master.release_unarmed_frame(frame);
        return Err(ScheduledServiceFrameError::Build(error));
    }
    master
        .submit_frame(port, frame)
        .map_err(ScheduledServiceFrameError::Transmit)
}

impl<const DOMAINS: usize, const SLOTS: usize> RxDatagramConsumer
    for ScheduledDomainBank<'_, DOMAINS, SLOTS>
{
    fn accept(
        &mut self,
        cycle: u64,
        received_at_ns: u64,
        completion: RxMatch,
        header: DatagramHeader,
        payload: &[u8],
    ) -> bool {
        let Some((active_cycle, generation, due_mask)) = self.active else {
            return false;
        };
        if cycle != active_cycle || completion.generation != generation {
            return false;
        }
        let owner = self.index_owner[header.index as usize];
        if owner == 0 {
            return false;
        }
        let entry = &mut self.domains[(owner - 1) as usize];
        due_mask & (1u64 << entry.id) != 0
            && entry
                .domain
                .accept(cycle, received_at_ns, completion, header, payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{DatagramPlan, FramePlanSet};
    use crate::schedule::ScheduleDomain;
    use crate::wire::Command;

    fn domain<const BYTES: usize>(index: u8) -> Domain<BYTES, 1> {
        let mut domain = Domain::new(0x1000 + u32::from(index));
        domain
            .add_segment(DomainSegment {
                datagram_index: index,
                input_offset: 0,
                len: BYTES,
                expected_wkc: 1,
            })
            .unwrap();
        domain
    }

    #[test]
    fn schedule_binding_rejects_wrong_order_duplicate_indices_and_empty_domains() {
        let schedule = ScheduleTable::<2, 2>::build(
            100_000,
            &[
                ScheduleDomain {
                    id: 9,
                    period_ticks: 1,
                    phase_ticks: 0,
                },
                ScheduleDomain {
                    id: 10,
                    period_ticks: 2,
                    phase_ticks: 0,
                },
            ],
        )
        .unwrap();
        let mut first = domain::<2>(12);
        let mut second = domain::<1>(12);
        assert!(matches!(
            ScheduledDomainBank::new(
                &schedule,
                [
                    ScheduledDomainEntry {
                        id: 10,
                        domain: &mut first,
                    },
                    ScheduledDomainEntry {
                        id: 9,
                        domain: &mut second,
                    },
                ],
            ),
            Err(ScheduledDomainError::InvalidSchedule)
        ));
        assert!(matches!(
            ScheduledDomainBank::new(
                &schedule,
                [
                    ScheduledDomainEntry {
                        id: 9,
                        domain: &mut first,
                    },
                    ScheduledDomainEntry {
                        id: 10,
                        domain: &mut second,
                    },
                ],
            ),
            Err(ScheduledDomainError::DuplicateDatagramIndex(12))
        ));
        let mut empty = Domain::<2, 1>::new(0x2000);
        assert!(matches!(
            ScheduledDomainBank::new(
                &schedule,
                [
                    ScheduledDomainEntry {
                        id: 9,
                        domain: &mut first,
                    },
                    ScheduledDomainEntry {
                        id: 10,
                        domain: &mut empty,
                    },
                ],
            ),
            Err(ScheduledDomainError::EmptyDomain(1))
        ));
    }

    #[test]
    fn non_due_and_wrong_generation_datagrams_cannot_change_domain_quality() {
        let schedule = ScheduleTable::<2, 2>::build(
            100_000,
            &[
                ScheduleDomain {
                    id: 9,
                    period_ticks: 1,
                    phase_ticks: 0,
                },
                ScheduleDomain {
                    id: 10,
                    period_ticks: 2,
                    phase_ticks: 0,
                },
            ],
        )
        .unwrap();
        let mut first = domain::<2>(12);
        let mut second = domain::<1>(13);
        let mut bank = ScheduledDomainBank::new(
            &schedule,
            [
                ScheduledDomainEntry {
                    id: 9,
                    domain: &mut first,
                },
                ScheduledDomainEntry {
                    id: 10,
                    domain: &mut second,
                },
            ],
        )
        .unwrap();
        let completion = |generation| RxMatch {
            slot_id: 0,
            generation,
            working_counter: 1,
        };
        let header = |index| DatagramHeader {
            command: Command::Lrw,
            index,
            address: 0x1000,
            length: 1,
            last: true,
        };
        assert_eq!(bank.begin_due(2, 1), Err(ScheduledDomainError::CycleOrder));
        bank.begin_due(1, 1).unwrap();
        assert_eq!(
            bank.begin_due(1, 1),
            Err(ScheduledDomainError::CycleInProgress)
        );
        assert!(!bank.accept(1, 100, completion(2), header(13), &[9]));
        assert!(bank.accept(1, 100, completion(1), header(13), &[8]));
        assert!(bank.accept(1, 100, completion(1), header(12), &[1, 2]));
        assert_eq!(bank.finish_due(1, 2), Err(ScheduledDomainError::CycleOrder));
        let qualities = bank.finish_due(1, 1).unwrap();
        assert!(qualities.iter().all(|quality| quality.valid));
        bank.begin_due(2, 2).unwrap();
        assert!(!bank.accept(2, 200, completion(2), header(13), &[3]));
        assert!(bank.accept(2, 200, completion(2), header(12), &[4, 5]));
        let qualities = bank.finish_due(2, 2).unwrap();
        assert_eq!(qualities[1].last_valid_cycle, 1);
        assert_eq!(qualities[1].actual_wkc, 1);
        bank.begin_due(3, 3).unwrap();
        assert!(bank.accept(3, 300, completion(3), header(12), &[6, 7]));
        let qualities = bank.finish_due(3, 3).unwrap();
        assert!(!qualities[1].valid);
        assert!(!qualities[1].complete);
        assert_eq!(qualities[1].last_valid_cycle, 1);
    }

    #[test]
    fn split_output_plans_must_match_every_bound_domain_segment() {
        let schedule = ScheduleTable::<2, 2>::build(
            100_000,
            &[
                ScheduleDomain {
                    id: 9,
                    period_ticks: 1,
                    phase_ticks: 0,
                },
                ScheduleDomain {
                    id: 10,
                    period_ticks: 2,
                    phase_ticks: 0,
                },
            ],
        )
        .unwrap();
        let mut motion = domain::<2>(12);
        let mut auxiliary = Domain::<2, 2>::new(0x2000);
        for (index, offset) in [(13, 0), (14, 1)] {
            auxiliary
                .add_segment(DomainSegment {
                    datagram_index: index,
                    input_offset: offset,
                    len: 1,
                    expected_wkc: 1,
                })
                .unwrap();
        }
        let bank = ScheduledDomainBank::new(
            &schedule,
            [
                ScheduledDomainEntry {
                    id: 9,
                    domain: &mut motion,
                },
                ScheduledDomainEntry {
                    id: 10,
                    domain: &mut auxiliary,
                },
            ],
        )
        .unwrap();
        let mut plans = FramePlanSet::<2, 1>::new();
        for (index, offset) in [(13, 0), (14, 1)] {
            plans
                .push(DatagramPlan {
                    command: Command::Lrw,
                    index,
                    address: 0x2000 + offset as u32,
                    payload_offset: offset,
                    payload_len: 1,
                    expected_wkc: 1,
                })
                .unwrap();
        }
        assert_eq!(plans.frame_count(), 2);
        assert!(bank.matches_frame_plans(10, &plans));
        assert!(!bank.matches_frame_plans(9, &plans));
        let mut wrong = FramePlanSet::<2, 1>::new();
        wrong
            .push(DatagramPlan {
                address: 0x2002,
                ..plans.plan(0).unwrap().datagrams()[0]
            })
            .unwrap();
        wrong.push(plans.plan(1).unwrap().datagrams()[0]).unwrap();
        assert!(!bank.matches_frame_plans(10, &wrong));
    }

    #[test]
    fn process_inputs_bind_schedule_order_images_and_non_overlapping_writes() {
        let schedule = ScheduleTable::<2, 1>::build(
            100_000,
            &[
                ScheduleDomain {
                    id: 9,
                    period_ticks: 1,
                    phase_ticks: 0,
                },
                ScheduleDomain {
                    id: 10,
                    period_ticks: 1,
                    phase_ticks: 0,
                },
            ],
        )
        .unwrap();
        let mut motion = Domain::<2, 1>::new(0x1000);
        motion
            .add_segment(DomainSegment {
                datagram_index: 12,
                input_offset: 0,
                len: 2,
                expected_wkc: 1,
            })
            .unwrap();
        let mut auxiliary = Domain::<1, 1>::new(0x2000);
        auxiliary
            .add_segment(DomainSegment {
                datagram_index: 13,
                input_offset: 0,
                len: 1,
                expected_wkc: 1,
            })
            .unwrap();
        let bank = ScheduledDomainBank::new(
            &schedule,
            [
                ScheduledDomainEntry {
                    id: 9,
                    domain: &mut motion,
                },
                ScheduledDomainEntry {
                    id: 10,
                    domain: &mut auxiliary,
                },
            ],
        )
        .unwrap();
        let mut motion_plans = FramePlanSet::<1, 1>::new();
        motion_plans
            .push(DatagramPlan {
                command: Command::Lrw,
                index: 12,
                address: 0x1000,
                payload_offset: 0,
                payload_len: 2,
                expected_wkc: 1,
            })
            .unwrap();
        let mut auxiliary_plans = FramePlanSet::<1, 1>::new();
        auxiliary_plans
            .push(DatagramPlan {
                command: Command::Lrw,
                index: 13,
                address: 0x2000,
                payload_offset: 0,
                payload_len: 1,
                expected_wkc: 1,
            })
            .unwrap();
        let motion_image = [0u8; 2];
        let auxiliary_image = [0u8; 1];
        assert!(
            ScheduledProcessInputs::new(
                &bank,
                &schedule,
                [
                    ScheduledProcessInputEntry {
                        id: 9,
                        image: &motion_image,
                        plans: &motion_plans,
                    },
                    ScheduledProcessInputEntry {
                        id: 10,
                        image: &auxiliary_image,
                        plans: &auxiliary_plans,
                    },
                ],
            )
            .is_ok()
        );
        assert!(matches!(
            ScheduledProcessInputs::new(
                &bank,
                &schedule,
                [
                    ScheduledProcessInputEntry {
                        id: 10,
                        image: &motion_image,
                        plans: &motion_plans,
                    },
                    ScheduledProcessInputEntry {
                        id: 9,
                        image: &auxiliary_image,
                        plans: &auxiliary_plans,
                    },
                ],
            ),
            Err(ScheduledProcessInputPlanError::DomainMismatch {
                expected: 9,
                actual: 10
            })
        ));
        assert!(matches!(
            ScheduledProcessInputs::new(
                &bank,
                &schedule,
                [
                    ScheduledProcessInputEntry {
                        id: 9,
                        image: &auxiliary_image,
                        plans: &motion_plans,
                    },
                    ScheduledProcessInputEntry {
                        id: 10,
                        image: &auxiliary_image,
                        plans: &auxiliary_plans,
                    },
                ],
            ),
            Err(ScheduledProcessInputPlanError::InvalidPlan(9))
        ));
    }
}
