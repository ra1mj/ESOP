//! Fixed-priority production service scheduling over the shared cyclic RX.
//!
//! Startup, mapping, DC configuration, and mailbox controllers retain their
//! own state machines. This scheduler owns the single request handle admitted
//! to the production service slot, keeps it across cycles while it is in
//! flight, and consumes or rebuilds it without allowing a lower-priority
//! service to overtake the active transaction.

use crate::control::{ControlError, ControlRequestPool, RequestHandle, RequestState};
use crate::dc::{DcController, DcCyclicSync, DcError, DcPhase, DcProgress};
use crate::engine::EthercatMaster;
use crate::mailbox::{MailboxController, MailboxError, MailboxPhase, MailboxProgress};
use crate::mapping_config::{
    MappingConfigController, MappingConfigError, MappingConfigPhase, MappingConfigProgress,
};
use crate::port::EthercatPort;
use crate::scheduled_domains::{
    ScheduledControlCycleError, ScheduledControlCycleReport, ScheduledDomainBank,
    ScheduledMailboxCycleError, ScheduledMailboxCycleReport, ScheduledReceiveReport,
};
use crate::startup::{StartupController, StartupError, StartupPhase, StartupProgress};
use crate::wire::MAX_ETHERNET_FRAME_LEN;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledProductionServiceKind {
    Idle,
    Startup,
    Mapping,
    DcConfiguration,
    Mailbox,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledProductionServiceProgress {
    Idle,
    Waiting,
    Startup(StartupProgress),
    Mapping(MappingConfigProgress),
    DcConfiguration(DcProgress),
    Mailbox(MailboxProgress),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledProductionServiceFault {
    Control(ControlError),
    Startup(StartupError),
    Mapping(MappingConfigError),
    DcConfiguration(DcError),
    Mailbox(MailboxError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledProductionServiceRecovery {
    None,
    AwaitingResponse,
    RebuildRequest,
    Faulted,
}

pub struct ScheduledProductionServices<
    'a,
    const MAX_SLAVES: usize,
    const SMS: usize,
    const FMMUS: usize,
> {
    pub startup: Option<&'a mut StartupController<MAX_SLAVES>>,
    pub mapping: Option<&'a mut MappingConfigController<SMS, FMMUS>>,
    pub dc_configuration: Option<&'a mut DcController>,
    pub mailbox: Option<&'a mut MailboxController>,
}

impl<'a, const MAX_SLAVES: usize, const SMS: usize, const FMMUS: usize>
    ScheduledProductionServices<'a, MAX_SLAVES, SMS, FMMUS>
{
    pub const fn new(
        startup: Option<&'a mut StartupController<MAX_SLAVES>>,
        mapping: Option<&'a mut MappingConfigController<SMS, FMMUS>>,
        dc_configuration: Option<&'a mut DcController>,
        mailbox: Option<&'a mut MailboxController>,
    ) -> Self {
        Self {
            startup,
            mapping,
            dc_configuration,
            mailbox,
        }
    }
}

#[derive(Debug)]
enum ScheduledProductionServiceTransport<E, const DOMAINS: usize> {
    Control(ScheduledControlCycleReport<E, DOMAINS>),
    Mailbox(ScheduledMailboxCycleReport<E, DOMAINS>),
}

#[derive(Debug)]
pub struct ScheduledProductionServiceCycleReport<E, const DOMAINS: usize> {
    selected: ScheduledProductionServiceKind,
    progress: ScheduledProductionServiceProgress,
    fault: Option<ScheduledProductionServiceFault>,
    recovery: ScheduledProductionServiceRecovery,
    request: Option<RequestHandle>,
    service_ready: bool,
    transport: ScheduledProductionServiceTransport<E, DOMAINS>,
}

impl<E, const DOMAINS: usize> ScheduledProductionServiceCycleReport<E, DOMAINS> {
    pub const fn selected(&self) -> ScheduledProductionServiceKind {
        self.selected
    }

    pub const fn progress(&self) -> ScheduledProductionServiceProgress {
        self.progress
    }

    pub const fn fault(&self) -> Option<ScheduledProductionServiceFault> {
        self.fault
    }

    pub const fn recovery(&self) -> ScheduledProductionServiceRecovery {
        self.recovery
    }

    pub const fn request(&self) -> Option<RequestHandle> {
        self.request
    }

    pub const fn service_ready(&self) -> bool {
        self.service_ready
    }

    pub const fn received(&self) -> &ScheduledReceiveReport<E, DOMAINS> {
        match &self.transport {
            ScheduledProductionServiceTransport::Control(cycle) => cycle.received(),
            ScheduledProductionServiceTransport::Mailbox(cycle) => &cycle.receive.received,
        }
    }

    pub const fn post_receive_deadline_met(&self) -> bool {
        match &self.transport {
            ScheduledProductionServiceTransport::Control(cycle) => {
                cycle.post_receive_deadline_met()
            }
            ScheduledProductionServiceTransport::Mailbox(cycle) => cycle.post_receive_deadline_met,
        }
    }

    pub const fn control_cycle(&self) -> Option<&ScheduledControlCycleReport<E, DOMAINS>> {
        match &self.transport {
            ScheduledProductionServiceTransport::Control(cycle) => Some(cycle),
            ScheduledProductionServiceTransport::Mailbox(_) => None,
        }
    }

    pub const fn mailbox_cycle(&self) -> Option<&ScheduledMailboxCycleReport<E, DOMAINS>> {
        match &self.transport {
            ScheduledProductionServiceTransport::Control(_) => None,
            ScheduledProductionServiceTransport::Mailbox(cycle) => Some(cycle),
        }
    }

    pub(crate) fn confirmed_by<const SCHEDULE_SLOTS: usize>(
        &self,
        bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
    ) -> bool {
        match &self.transport {
            ScheduledProductionServiceTransport::Control(cycle) => {
                bank.confirms_control_cycle(cycle)
            }
            ScheduledProductionServiceTransport::Mailbox(cycle) => {
                bank.confirms_mailbox_cycle(cycle)
            }
        }
    }
}

#[derive(Debug)]
pub enum ScheduledProductionServiceCycleError<E, const DOMAINS: usize> {
    MissingController(ScheduledProductionServiceKind),
    RequestMismatch(ScheduledProductionServiceKind),
    Control(ControlError),
    ControlCycle(ScheduledControlCycleError<E, DOMAINS>),
    MailboxCycle(ScheduledMailboxCycleError<E>),
}

pub struct ScheduledProductionServiceScheduler {
    active: ScheduledProductionServiceKind,
    request: Option<RequestHandle>,
}

struct ScheduledProductionEnqueueOutcome {
    request: Option<RequestHandle>,
    progress: Option<ScheduledProductionServiceProgress>,
    fault: Option<ScheduledProductionServiceFault>,
}

impl ScheduledProductionEnqueueOutcome {
    const EMPTY: Self = Self {
        request: None,
        progress: None,
        fault: None,
    };
}

impl ScheduledProductionServiceScheduler {
    pub const fn new() -> Self {
        Self {
            active: ScheduledProductionServiceKind::Idle,
            request: None,
        }
    }

    pub const fn active(&self) -> ScheduledProductionServiceKind {
        self.active
    }

    pub const fn request(&self) -> Option<RequestHandle> {
        self.request
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run_cycle<
        P: EthercatPort,
        const MASTER_FRAMES: usize,
        const MTU: usize,
        const REQUESTS: usize,
        const DOMAINS: usize,
        const SCHEDULE_SLOTS: usize,
        const MAX_SLAVES: usize,
        const SMS: usize,
        const FMMUS: usize,
    >(
        &mut self,
        bank: &mut ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        master: &mut EthercatMaster<MASTER_FRAMES, MTU>,
        port: &mut P,
        scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN],
        dc: &mut DcCyclicSync,
        dc_image: &mut [u8],
        application_time_ns: u64,
        controls: &mut ControlRequestPool<REQUESTS>,
        services: &mut ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS>,
        generation: u16,
        rx_deadline_ns: u64,
        cycle_deadline_ns: u64,
    ) -> Result<
        ScheduledProductionServiceCycleReport<P::Error, DOMAINS>,
        ScheduledProductionServiceCycleError<P::Error, DOMAINS>,
    > {
        self.refresh_selection(services);
        let selected = self.active;
        self.ensure_request_matches(controls, services)
            .map_err(ScheduledProductionServiceCycleError::RequestMismatch)?;
        let mut pre_progress = None;
        let mut pre_fault = None;
        if self.request.is_none() {
            let outcome = self
                .enqueue_due(port.now_ns(), controls, services)
                .map_err(ScheduledProductionServiceCycleError::Control)?;
            self.request = outcome.request;
            pre_progress = outcome.progress;
            pre_fault = outcome.fault;
        }

        if selected == ScheduledProductionServiceKind::Mailbox {
            let mailbox = services.mailbox.as_deref_mut().ok_or(
                ScheduledProductionServiceCycleError::MissingController(selected),
            )?;
            let cycle = match bank.run_dc_and_mailbox_cycle(
                master,
                port,
                scratch,
                dc,
                dc_image,
                application_time_ns,
                controls,
                mailbox,
                self.request,
                generation,
                rx_deadline_ns,
                cycle_deadline_ns,
            ) {
                Ok(cycle) => cycle,
                Err(error) => {
                    self.release_prepared(controls)
                        .map_err(ScheduledProductionServiceCycleError::Control)?;
                    return Err(ScheduledProductionServiceCycleError::MailboxCycle(error));
                }
            };
            self.request = cycle.request;
            let progress = cycle
                .receive
                .mailbox_progress
                .and_then(Result::ok)
                .map(ScheduledProductionServiceProgress::Mailbox)
                .or(pre_progress)
                .unwrap_or(ScheduledProductionServiceProgress::Waiting);
            let fault = pre_fault.or_else(|| {
                cycle
                    .receive
                    .mailbox_progress
                    .and_then(Result::err)
                    .map(ScheduledProductionServiceFault::Mailbox)
                    .or_else(|| {
                        mailbox
                            .last_error()
                            .map(ScheduledProductionServiceFault::Mailbox)
                    })
            });
            let recovery = self.recovery(controls, progress, fault);
            let service_ready = fault.is_none()
                && !matches!(
                    progress,
                    ScheduledProductionServiceProgress::Mailbox(MailboxProgress::RetryScheduled)
                )
                && cycle.tx.service.failure.is_none();
            return Ok(ScheduledProductionServiceCycleReport {
                selected,
                progress,
                fault,
                recovery,
                request: self.request,
                service_ready,
                transport: ScheduledProductionServiceTransport::Mailbox(cycle),
            });
        }

        let cycle = match bank.run_dc_and_control_cycle(
            master,
            port,
            scratch,
            dc,
            dc_image,
            application_time_ns,
            controls,
            self.request,
            generation,
            rx_deadline_ns,
            cycle_deadline_ns,
        ) {
            Ok(cycle) => cycle,
            Err(error) => {
                self.release_prepared(controls)
                    .map_err(ScheduledProductionServiceCycleError::Control)?;
                return Err(ScheduledProductionServiceCycleError::ControlCycle(error));
            }
        };
        let mut progress = pre_progress.unwrap_or(ScheduledProductionServiceProgress::Waiting);
        let mut fault = pre_fault;
        if let Some(handle) = self.request {
            match controls.get(handle).map(|request| request.state) {
                Some(RequestState::Complete | RequestState::Failed) => {
                    let result = self.consume_terminal(controls, services, handle, port.now_ns());
                    self.request = None;
                    match result {
                        Ok(value) => progress = value,
                        Err(error) => fault = Some(error),
                    }
                }
                Some(RequestState::Prepared) => {
                    controls
                        .release(handle)
                        .map_err(ScheduledProductionServiceCycleError::Control)?;
                    self.request = None;
                }
                Some(RequestState::InFlight) => {}
                Some(RequestState::Free) | None => {
                    return Err(ScheduledProductionServiceCycleError::RequestMismatch(
                        selected,
                    ));
                }
            }
        } else if selected == ScheduledProductionServiceKind::Idle {
            progress = ScheduledProductionServiceProgress::Idle;
        }
        fault = fault.or_else(|| self.controller_fault(services));
        let recovery = self.recovery(controls, progress, fault);
        let service_ready =
            cycle.service().failure.is_none() && fault.is_none() && self.controller_ready(services);
        Ok(ScheduledProductionServiceCycleReport {
            selected,
            progress,
            fault,
            recovery,
            request: self.request,
            service_ready,
            transport: ScheduledProductionServiceTransport::Control(cycle),
        })
    }

    fn refresh_selection<const MAX_SLAVES: usize, const SMS: usize, const FMMUS: usize>(
        &mut self,
        services: &ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS>,
    ) {
        if self.request.is_some() || self.service_active(services, self.active) {
            return;
        }
        self.active = [
            ScheduledProductionServiceKind::Startup,
            ScheduledProductionServiceKind::Mapping,
            ScheduledProductionServiceKind::DcConfiguration,
            ScheduledProductionServiceKind::Mailbox,
        ]
        .into_iter()
        .find(|kind| self.service_active(services, *kind))
        .unwrap_or(ScheduledProductionServiceKind::Idle);
    }

    fn service_active<const MAX_SLAVES: usize, const SMS: usize, const FMMUS: usize>(
        &self,
        services: &ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS>,
        kind: ScheduledProductionServiceKind,
    ) -> bool {
        match kind {
            ScheduledProductionServiceKind::Idle => false,
            ScheduledProductionServiceKind::Startup => {
                services.startup.as_deref().is_some_and(|controller| {
                    !matches!(controller.phase(), StartupPhase::Idle | StartupPhase::Ready)
                })
            }
            ScheduledProductionServiceKind::Mapping => {
                services.mapping.as_deref().is_some_and(|controller| {
                    !matches!(
                        controller.phase(),
                        MappingConfigPhase::Idle | MappingConfigPhase::Complete
                    )
                })
            }
            ScheduledProductionServiceKind::DcConfiguration => services
                .dc_configuration
                .as_deref()
                .is_some_and(|controller| {
                    !matches!(controller.phase(), DcPhase::Idle | DcPhase::Complete)
                }),
            ScheduledProductionServiceKind::Mailbox => {
                services.mailbox.as_deref().is_some_and(|controller| {
                    !matches!(
                        controller.phase(),
                        MailboxPhase::Idle | MailboxPhase::Complete
                    )
                })
            }
        }
    }

    fn ensure_request_matches<
        const REQUESTS: usize,
        const MAX_SLAVES: usize,
        const SMS: usize,
        const FMMUS: usize,
    >(
        &self,
        controls: &ControlRequestPool<REQUESTS>,
        services: &ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS>,
    ) -> Result<(), ScheduledProductionServiceKind> {
        let Some(handle) = self.request else {
            return Ok(());
        };
        let request = controls.get(handle).ok_or(self.active)?;
        let matches = match self.active {
            ScheduledProductionServiceKind::Startup => services
                .startup
                .as_deref()
                .and_then(StartupController::pending_action)
                .is_some_and(|action| {
                    request.matches_action(
                        action.datagram_index(),
                        action.generation(),
                        action.address(),
                        action.operation(),
                        action.payload(),
                        action.datagram_len(),
                        action.deadline_ns(),
                    )
                }),
            ScheduledProductionServiceKind::Mapping => services
                .mapping
                .as_deref()
                .and_then(MappingConfigController::pending)
                .is_some_and(|action| {
                    request.matches_action(
                        action.datagram_index,
                        action.generation,
                        action.address,
                        action.operation,
                        action.payload(),
                        action.datagram_len(),
                        action.deadline_ns,
                    )
                }),
            ScheduledProductionServiceKind::DcConfiguration => services
                .dc_configuration
                .as_deref()
                .and_then(DcController::pending)
                .is_some_and(|action| {
                    request.matches_action(
                        action.datagram_index,
                        action.generation,
                        action.address,
                        action.operation,
                        action.payload(),
                        action.datagram_len(),
                        action.deadline_ns,
                    )
                }),
            ScheduledProductionServiceKind::Mailbox => services
                .mailbox
                .as_deref()
                .and_then(MailboxController::pending)
                .is_some_and(|action| {
                    request.matches_action(
                        action.datagram_index,
                        action.generation,
                        action.address,
                        action.operation,
                        action.payload(),
                        action.datagram_len(),
                        action.deadline_ns,
                    )
                }),
            ScheduledProductionServiceKind::Idle => false,
        };
        if matches { Ok(()) } else { Err(self.active) }
    }

    fn enqueue_due<
        const REQUESTS: usize,
        const MAX_SLAVES: usize,
        const SMS: usize,
        const FMMUS: usize,
    >(
        &mut self,
        now_ns: u64,
        controls: &mut ControlRequestPool<REQUESTS>,
        services: &mut ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS>,
    ) -> Result<ScheduledProductionEnqueueOutcome, ControlError> {
        match self.active {
            ScheduledProductionServiceKind::Idle => Ok(ScheduledProductionEnqueueOutcome::EMPTY),
            ScheduledProductionServiceKind::Startup => {
                let controller = services
                    .startup
                    .as_deref_mut()
                    .ok_or(ControlError::InvalidState)?;
                let action = match controller.next_action(now_ns) {
                    Ok(action) => action,
                    Err(error) => {
                        return Ok(ScheduledProductionEnqueueOutcome {
                            fault: Some(ScheduledProductionServiceFault::Startup(error)),
                            ..ScheduledProductionEnqueueOutcome::EMPTY
                        });
                    }
                };
                let Some(action) = action else {
                    return Ok(ScheduledProductionEnqueueOutcome::EMPTY);
                };
                if action.deadline_ns() <= now_ns {
                    return Ok(match controller.timeout(action, now_ns) {
                        Ok(progress) => ScheduledProductionEnqueueOutcome {
                            progress: Some(ScheduledProductionServiceProgress::Startup(progress)),
                            ..ScheduledProductionEnqueueOutcome::EMPTY
                        },
                        Err(error) => ScheduledProductionEnqueueOutcome {
                            fault: Some(ScheduledProductionServiceFault::Startup(error)),
                            ..ScheduledProductionEnqueueOutcome::EMPTY
                        },
                    });
                }
                Ok(ScheduledProductionEnqueueOutcome {
                    request: Some(controller.enqueue_pending(controls)?),
                    ..ScheduledProductionEnqueueOutcome::EMPTY
                })
            }
            ScheduledProductionServiceKind::Mapping => {
                let controller = services
                    .mapping
                    .as_deref_mut()
                    .ok_or(ControlError::InvalidState)?;
                let action = match controller.next_action(now_ns) {
                    Ok(action) => action,
                    Err(error) => {
                        return Ok(ScheduledProductionEnqueueOutcome {
                            fault: Some(ScheduledProductionServiceFault::Mapping(error)),
                            ..ScheduledProductionEnqueueOutcome::EMPTY
                        });
                    }
                };
                let Some(action) = action else {
                    return Ok(ScheduledProductionEnqueueOutcome::EMPTY);
                };
                if action.deadline_ns <= now_ns {
                    return Ok(match controller.timeout(action, now_ns) {
                        Ok(progress) => ScheduledProductionEnqueueOutcome {
                            progress: Some(ScheduledProductionServiceProgress::Mapping(progress)),
                            ..ScheduledProductionEnqueueOutcome::EMPTY
                        },
                        Err(error) => ScheduledProductionEnqueueOutcome {
                            fault: Some(ScheduledProductionServiceFault::Mapping(error)),
                            ..ScheduledProductionEnqueueOutcome::EMPTY
                        },
                    });
                }
                Ok(ScheduledProductionEnqueueOutcome {
                    request: Some(controller.enqueue_pending(controls)?),
                    ..ScheduledProductionEnqueueOutcome::EMPTY
                })
            }
            ScheduledProductionServiceKind::DcConfiguration => {
                let controller = services
                    .dc_configuration
                    .as_deref_mut()
                    .ok_or(ControlError::InvalidState)?;
                let action = match controller.next_action(now_ns) {
                    Ok(action) => action,
                    Err(error) => {
                        return Ok(ScheduledProductionEnqueueOutcome {
                            fault: Some(ScheduledProductionServiceFault::DcConfiguration(error)),
                            ..ScheduledProductionEnqueueOutcome::EMPTY
                        });
                    }
                };
                let Some(action) = action else {
                    return Ok(ScheduledProductionEnqueueOutcome::EMPTY);
                };
                if action.deadline_ns <= now_ns {
                    return Ok(match controller.timeout(action, now_ns) {
                        Ok(progress) => ScheduledProductionEnqueueOutcome {
                            progress: Some(ScheduledProductionServiceProgress::DcConfiguration(
                                progress,
                            )),
                            ..ScheduledProductionEnqueueOutcome::EMPTY
                        },
                        Err(error) => ScheduledProductionEnqueueOutcome {
                            fault: Some(ScheduledProductionServiceFault::DcConfiguration(error)),
                            ..ScheduledProductionEnqueueOutcome::EMPTY
                        },
                    });
                }
                Ok(ScheduledProductionEnqueueOutcome {
                    request: Some(controller.enqueue_pending(controls)?),
                    ..ScheduledProductionEnqueueOutcome::EMPTY
                })
            }
            ScheduledProductionServiceKind::Mailbox => {
                let controller = services
                    .mailbox
                    .as_deref_mut()
                    .ok_or(ControlError::InvalidState)?;
                let action = match controller.next_action(now_ns) {
                    Ok(action) => action,
                    Err(error) => {
                        return Ok(ScheduledProductionEnqueueOutcome {
                            fault: Some(ScheduledProductionServiceFault::Mailbox(error)),
                            ..ScheduledProductionEnqueueOutcome::EMPTY
                        });
                    }
                };
                let Some(action) = action else {
                    return Ok(ScheduledProductionEnqueueOutcome::EMPTY);
                };
                if action.deadline_ns <= now_ns {
                    return Ok(match controller.timeout(action, now_ns) {
                        Ok(progress) => ScheduledProductionEnqueueOutcome {
                            progress: Some(ScheduledProductionServiceProgress::Mailbox(progress)),
                            ..ScheduledProductionEnqueueOutcome::EMPTY
                        },
                        Err(error) => ScheduledProductionEnqueueOutcome {
                            fault: Some(ScheduledProductionServiceFault::Mailbox(error)),
                            ..ScheduledProductionEnqueueOutcome::EMPTY
                        },
                    });
                }
                Ok(ScheduledProductionEnqueueOutcome {
                    request: Some(controller.enqueue_pending(controls)?),
                    ..ScheduledProductionEnqueueOutcome::EMPTY
                })
            }
        }
    }

    fn release_prepared<const REQUESTS: usize>(
        &mut self,
        controls: &mut ControlRequestPool<REQUESTS>,
    ) -> Result<(), ControlError> {
        let Some(handle) = self.request else {
            return Ok(());
        };
        if controls
            .get(handle)
            .is_some_and(|request| request.state == RequestState::Prepared)
        {
            controls.release(handle)?;
            self.request = None;
        }
        Ok(())
    }

    fn consume_terminal<
        const REQUESTS: usize,
        const MAX_SLAVES: usize,
        const SMS: usize,
        const FMMUS: usize,
    >(
        &mut self,
        controls: &mut ControlRequestPool<REQUESTS>,
        services: &mut ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS>,
        handle: RequestHandle,
        now_ns: u64,
    ) -> Result<ScheduledProductionServiceProgress, ScheduledProductionServiceFault> {
        match self.active {
            ScheduledProductionServiceKind::Startup => services
                .startup
                .as_deref_mut()
                .ok_or(ScheduledProductionServiceFault::Control(
                    ControlError::InvalidState,
                ))?
                .accept_completed(controls, handle, now_ns)
                .map(ScheduledProductionServiceProgress::Startup)
                .map_err(ScheduledProductionServiceFault::Startup),
            ScheduledProductionServiceKind::Mapping => services
                .mapping
                .as_deref_mut()
                .ok_or(ScheduledProductionServiceFault::Control(
                    ControlError::InvalidState,
                ))?
                .accept_completed(controls, handle, now_ns)
                .map(ScheduledProductionServiceProgress::Mapping)
                .map_err(ScheduledProductionServiceFault::Mapping),
            ScheduledProductionServiceKind::DcConfiguration => services
                .dc_configuration
                .as_deref_mut()
                .ok_or(ScheduledProductionServiceFault::Control(
                    ControlError::InvalidState,
                ))?
                .accept_completed(controls, handle, now_ns)
                .map(ScheduledProductionServiceProgress::DcConfiguration)
                .map_err(ScheduledProductionServiceFault::DcConfiguration),
            ScheduledProductionServiceKind::Idle | ScheduledProductionServiceKind::Mailbox => Err(
                ScheduledProductionServiceFault::Control(ControlError::InvalidState),
            ),
        }
    }

    fn controller_fault<const MAX_SLAVES: usize, const SMS: usize, const FMMUS: usize>(
        &self,
        services: &ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS>,
    ) -> Option<ScheduledProductionServiceFault> {
        match self.active {
            ScheduledProductionServiceKind::Idle => None,
            ScheduledProductionServiceKind::Startup => services
                .startup
                .as_deref()
                .and_then(StartupController::last_error)
                .map(ScheduledProductionServiceFault::Startup),
            ScheduledProductionServiceKind::Mapping => services
                .mapping
                .as_deref()
                .and_then(MappingConfigController::last_error)
                .map(ScheduledProductionServiceFault::Mapping),
            ScheduledProductionServiceKind::DcConfiguration => services
                .dc_configuration
                .as_deref()
                .and_then(DcController::last_error)
                .map(ScheduledProductionServiceFault::DcConfiguration),
            ScheduledProductionServiceKind::Mailbox => services
                .mailbox
                .as_deref()
                .and_then(MailboxController::last_error)
                .map(ScheduledProductionServiceFault::Mailbox),
        }
    }

    fn controller_ready<const MAX_SLAVES: usize, const SMS: usize, const FMMUS: usize>(
        &self,
        services: &ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS>,
    ) -> bool {
        match self.active {
            ScheduledProductionServiceKind::Idle => true,
            ScheduledProductionServiceKind::Startup => services
                .startup
                .as_deref()
                .is_some_and(|controller| controller.phase() == StartupPhase::Ready),
            ScheduledProductionServiceKind::Mapping => services
                .mapping
                .as_deref()
                .is_some_and(|controller| controller.phase() == MappingConfigPhase::Complete),
            ScheduledProductionServiceKind::DcConfiguration => services
                .dc_configuration
                .as_deref()
                .is_some_and(|controller| controller.phase() == DcPhase::Complete),
            ScheduledProductionServiceKind::Mailbox => services
                .mailbox
                .as_deref()
                .is_some_and(|controller| controller.phase() != MailboxPhase::Faulted),
        }
    }

    fn recovery<const REQUESTS: usize>(
        &self,
        controls: &ControlRequestPool<REQUESTS>,
        progress: ScheduledProductionServiceProgress,
        fault: Option<ScheduledProductionServiceFault>,
    ) -> ScheduledProductionServiceRecovery {
        if fault.is_some() {
            return ScheduledProductionServiceRecovery::Faulted;
        }
        if self.request.is_some_and(|handle| {
            controls
                .get(handle)
                .is_some_and(|request| request.state == RequestState::InFlight)
        }) {
            return ScheduledProductionServiceRecovery::AwaitingResponse;
        }
        if progress == ScheduledProductionServiceProgress::Waiting
            && self.active != ScheduledProductionServiceKind::Idle
        {
            ScheduledProductionServiceRecovery::RebuildRequest
        } else {
            ScheduledProductionServiceRecovery::None
        }
    }
}

impl Default for ScheduledProductionServiceScheduler {
    fn default() -> Self {
        Self::new()
    }
}
