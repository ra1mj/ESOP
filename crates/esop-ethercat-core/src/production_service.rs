//! Fixed-priority production service scheduling over the shared cyclic RX.
//!
//! Startup, PDO configuration, mapping, DC configuration, and mailbox
//! controllers retain their own state machines. This scheduler owns the single
//! request handle admitted to the production service slot, keeps it across
//! cycles while it is in flight, and consumes or rebuilds it without allowing
//! a lower-priority service to overtake the active transaction.

use crate::control::{ControlError, ControlRequestPool, RequestHandle, RequestState};
use crate::dc::{DcController, DcCyclicSync, DcError, DcPhase, DcProgress};
use crate::engine::EthercatMaster;
use crate::mailbox::{
    MAX_MAILBOX_BYTES, MailboxConfig, MailboxController, MailboxError, MailboxPhase,
    MailboxProgress, MailboxProtocol,
};
use crate::mapping_config::{
    MappingConfigController, MappingConfigError, MappingConfigPhase, MappingConfigProgress,
};
use crate::pdo_config::{
    PdoConfigAction, PdoConfigBatch, PdoConfigBatchError, PdoConfigBatchPhase,
    PdoConfigBatchStatus, PdoConfigController, PdoConfigError, PdoConfigPhase, PdoConfigProgress,
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
    PdoConfiguration,
    Mapping,
    DcConfiguration,
    Mailbox,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledPdoConfigurationProgress {
    Mailbox(MailboxProgress),
    Configuration(PdoConfigProgress),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledProductionServiceProgress {
    Idle,
    Waiting,
    Startup(StartupProgress),
    PdoConfiguration(ScheduledPdoConfigurationProgress),
    Mapping(MappingConfigProgress),
    DcConfiguration(DcProgress),
    Mailbox(MailboxProgress),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledProductionServiceFault {
    Control(ControlError),
    Startup(StartupError),
    PdoConfiguration(PdoConfigError),
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

enum ScheduledPdoConfigurationMode<'a, const OPS: usize, const JOBS: usize> {
    Single {
        controller: &'a mut PdoConfigController<OPS>,
        mailbox: &'a mut MailboxController,
        mailbox_config: MailboxConfig,
    },
    Batch(&'a mut PdoConfigBatch<JOBS, OPS>),
}

pub struct ScheduledPdoConfiguration<'a, const OPS: usize, const JOBS: usize = 1> {
    mode: ScheduledPdoConfigurationMode<'a, OPS, JOBS>,
}

impl<'a, const OPS: usize> ScheduledPdoConfiguration<'a, OPS, 1> {
    pub const fn new(
        controller: &'a mut PdoConfigController<OPS>,
        mailbox: &'a mut MailboxController,
        mailbox_config: MailboxConfig,
    ) -> Self {
        Self {
            mode: ScheduledPdoConfigurationMode::Single {
                controller,
                mailbox,
                mailbox_config,
            },
        }
    }
}

impl<'a, const OPS: usize, const JOBS: usize> ScheduledPdoConfiguration<'a, OPS, JOBS> {
    pub const fn batch(batch: &'a mut PdoConfigBatch<JOBS, OPS>) -> Self {
        Self {
            mode: ScheduledPdoConfigurationMode::Batch(batch),
        }
    }

    fn controller(&self) -> &PdoConfigController<OPS> {
        match &self.mode {
            ScheduledPdoConfigurationMode::Single { controller, .. } => controller,
            ScheduledPdoConfigurationMode::Batch(batch) => batch.controller(),
        }
    }

    fn controller_mut(&mut self) -> &mut PdoConfigController<OPS> {
        match &mut self.mode {
            ScheduledPdoConfigurationMode::Single { controller, .. } => controller,
            ScheduledPdoConfigurationMode::Batch(batch) => batch.controller_mut(),
        }
    }

    fn mailbox(&self) -> &MailboxController {
        match &self.mode {
            ScheduledPdoConfigurationMode::Single { mailbox, .. } => mailbox,
            ScheduledPdoConfigurationMode::Batch(batch) => batch.mailbox(),
        }
    }

    fn mailbox_mut(&mut self) -> &mut MailboxController {
        match &mut self.mode {
            ScheduledPdoConfigurationMode::Single { mailbox, .. } => mailbox,
            ScheduledPdoConfigurationMode::Batch(batch) => batch.mailbox_mut(),
        }
    }

    fn mailbox_config(&self) -> Option<MailboxConfig> {
        match &self.mode {
            ScheduledPdoConfigurationMode::Single { mailbox_config, .. } => Some(*mailbox_config),
            ScheduledPdoConfigurationMode::Batch(batch) => batch.current_mailbox_config(),
        }
    }

    fn is_complete(&self) -> bool {
        match &self.mode {
            ScheduledPdoConfigurationMode::Single { controller, .. } => {
                controller.phase() == PdoConfigPhase::Complete
            }
            ScheduledPdoConfigurationMode::Batch(batch) => {
                batch.phase() == PdoConfigBatchPhase::Complete
            }
        }
    }

    fn is_active(&self) -> bool {
        match &self.mode {
            ScheduledPdoConfigurationMode::Single { controller, .. } => !matches!(
                controller.phase(),
                PdoConfigPhase::Idle | PdoConfigPhase::Complete
            ),
            ScheduledPdoConfigurationMode::Batch(batch) => !matches!(
                batch.phase(),
                PdoConfigBatchPhase::Idle | PdoConfigBatchPhase::Complete
            ),
        }
    }

    fn advance_completed(&mut self, now_ns: u64) -> Result<(), PdoConfigBatchError> {
        if let ScheduledPdoConfigurationMode::Batch(batch) = &mut self.mode
            && batch.controller().phase() == PdoConfigPhase::Complete
        {
            batch.advance(now_ns)?;
        }
        Ok(())
    }

    fn batch_status(&self) -> Option<PdoConfigBatchStatus> {
        match &self.mode {
            ScheduledPdoConfigurationMode::Single { .. } => None,
            ScheduledPdoConfigurationMode::Batch(batch) => Some(batch.status()),
        }
    }
}

pub struct ScheduledProductionServices<
    'a,
    const MAX_SLAVES: usize,
    const SMS: usize,
    const FMMUS: usize,
    const PDO_OPS: usize = 0,
    const PDO_JOBS: usize = 1,
> {
    pub startup: Option<&'a mut StartupController<MAX_SLAVES>>,
    pdo_configuration: Option<ScheduledPdoConfiguration<'a, PDO_OPS, PDO_JOBS>>,
    pub mapping: Option<&'a mut MappingConfigController<SMS, FMMUS>>,
    pub dc_configuration: Option<&'a mut DcController>,
    pub mailbox: Option<&'a mut MailboxController>,
}

impl<
    'a,
    const MAX_SLAVES: usize,
    const SMS: usize,
    const FMMUS: usize,
    const PDO_OPS: usize,
    const PDO_JOBS: usize,
> ScheduledProductionServices<'a, MAX_SLAVES, SMS, FMMUS, PDO_OPS, PDO_JOBS>
{
    pub const fn new(
        startup: Option<&'a mut StartupController<MAX_SLAVES>>,
        mapping: Option<&'a mut MappingConfigController<SMS, FMMUS>>,
        dc_configuration: Option<&'a mut DcController>,
        mailbox: Option<&'a mut MailboxController>,
    ) -> Self {
        Self {
            startup,
            pdo_configuration: None,
            mapping,
            dc_configuration,
            mailbox,
        }
    }

    pub fn with_pdo_configuration(
        mut self,
        pdo_configuration: ScheduledPdoConfiguration<'a, PDO_OPS, PDO_JOBS>,
    ) -> Self {
        self.pdo_configuration = Some(pdo_configuration);
        self
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
    startup_phase: Option<StartupPhase>,
    pdo_batch_status: Option<PdoConfigBatchStatus>,
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

    pub const fn startup_phase(&self) -> Option<StartupPhase> {
        self.startup_phase
    }

    pub const fn pdo_batch_status(&self) -> Option<PdoConfigBatchStatus> {
        self.pdo_batch_status
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
    Startup(StartupError),
    PdoBatch(PdoConfigBatchError),
    Control(ControlError),
    ControlCycle(ScheduledControlCycleError<E, DOMAINS>),
    MailboxCycle(ScheduledMailboxCycleError<E>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupBarrierReleaseError {
    MissingController(ScheduledProductionServiceKind),
    Startup(StartupError),
    PdoBatch(PdoConfigBatchError),
}

pub struct ScheduledProductionServiceScheduler {
    active: ScheduledProductionServiceKind,
    request: Option<RequestHandle>,
    pdo_action: Option<PdoConfigAction>,
}

struct ScheduledProductionEnqueueOutcome {
    request: Option<RequestHandle>,
    progress: Option<ScheduledProductionServiceProgress>,
    fault: Option<ScheduledProductionServiceFault>,
}

struct ScheduledMailboxEnqueueOutcome {
    request: Option<RequestHandle>,
    progress: Option<MailboxProgress>,
    fault: Option<MailboxError>,
}

impl ScheduledMailboxEnqueueOutcome {
    const EMPTY: Self = Self {
        request: None,
        progress: None,
        fault: None,
    };
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
            pdo_action: None,
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
        const PDO_OPS: usize,
        const PDO_JOBS: usize,
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
        services: &mut ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS, PDO_OPS, PDO_JOBS>,
        generation: u16,
        rx_deadline_ns: u64,
        cycle_deadline_ns: u64,
    ) -> Result<
        ScheduledProductionServiceCycleReport<P::Error, DOMAINS>,
        ScheduledProductionServiceCycleError<P::Error, DOMAINS>,
    > {
        self.release_startup_configuration(port.now_ns(), services)
            .map_err(|error| match error {
                StartupBarrierReleaseError::MissingController(kind) => {
                    ScheduledProductionServiceCycleError::MissingController(kind)
                }
                StartupBarrierReleaseError::Startup(error) => {
                    ScheduledProductionServiceCycleError::Startup(error)
                }
                StartupBarrierReleaseError::PdoBatch(error) => {
                    ScheduledProductionServiceCycleError::PdoBatch(error)
                }
            })?;
        self.refresh_selection(services);
        let selected = self.active;
        if selected == ScheduledProductionServiceKind::PdoConfiguration
            && services.pdo_configuration.is_none()
        {
            return Err(ScheduledProductionServiceCycleError::MissingController(
                selected,
            ));
        }
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

        if selected == ScheduledProductionServiceKind::Mailbox
            || (selected == ScheduledProductionServiceKind::PdoConfiguration
                && self.pdo_action.is_some())
        {
            let mailbox = match selected {
                ScheduledProductionServiceKind::PdoConfiguration => services
                    .pdo_configuration
                    .as_mut()
                    .map(ScheduledPdoConfiguration::mailbox_mut),
                ScheduledProductionServiceKind::Mailbox => services.mailbox.as_deref_mut(),
                _ => None,
            }
            .ok_or(ScheduledProductionServiceCycleError::MissingController(
                selected,
            ))?;
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
            if selected == ScheduledProductionServiceKind::PdoConfiguration {
                let binding = services.pdo_configuration.as_mut().ok_or(
                    ScheduledProductionServiceCycleError::MissingController(selected),
                )?;
                let mailbox_progress = cycle.receive.mailbox_progress;
                let mut progress =
                    pre_progress.unwrap_or(ScheduledProductionServiceProgress::Waiting);
                let mut fault = pre_fault;

                match mailbox_progress {
                    Some(Ok(MailboxProgress::Complete)) => {
                        let action = self.pdo_action.ok_or(
                            ScheduledProductionServiceCycleError::RequestMismatch(selected),
                        )?;
                        let mut response = [0; MAX_MAILBOX_BYTES];
                        let response_len = match binding.mailbox().response() {
                            Some((_, payload)) => {
                                response[..payload.len()].copy_from_slice(payload);
                                payload.len()
                            }
                            None => {
                                self.pdo_action = None;
                                match binding
                                    .controller_mut()
                                    .mailbox_failed(action, MailboxError::NoPendingAction)
                                {
                                    Ok(value) => {
                                        progress =
                                            ScheduledProductionServiceProgress::PdoConfiguration(
                                                ScheduledPdoConfigurationProgress::Configuration(
                                                    value,
                                                ),
                                            );
                                    }
                                    Err(error) => {
                                        fault = Some(
                                            ScheduledProductionServiceFault::PdoConfiguration(
                                                error,
                                            ),
                                        );
                                    }
                                }
                                0
                            }
                        };
                        if response_len != 0 {
                            self.pdo_action = None;
                            match binding.controller_mut().accept(
                                action,
                                action.generation,
                                &response[..response_len],
                                port.now_ns(),
                            ) {
                                Ok(value) => {
                                    progress = ScheduledProductionServiceProgress::PdoConfiguration(
                                        ScheduledPdoConfigurationProgress::Configuration(value),
                                    );
                                }
                                Err(error) => {
                                    fault = Some(
                                        ScheduledProductionServiceFault::PdoConfiguration(error),
                                    );
                                }
                            }
                        }
                    }
                    Some(Ok(value)) => {
                        progress = ScheduledProductionServiceProgress::PdoConfiguration(
                            ScheduledPdoConfigurationProgress::Mailbox(value),
                        );
                    }
                    Some(Err(error)) => {
                        let action = self.pdo_action.ok_or(
                            ScheduledProductionServiceCycleError::RequestMismatch(selected),
                        )?;
                        self.pdo_action = None;
                        match binding.controller_mut().mailbox_failed(action, error) {
                            Ok(value) => {
                                progress = ScheduledProductionServiceProgress::PdoConfiguration(
                                    ScheduledPdoConfigurationProgress::Configuration(value),
                                );
                            }
                            Err(error) => {
                                fault =
                                    Some(ScheduledProductionServiceFault::PdoConfiguration(error));
                            }
                        }
                    }
                    None => {}
                }
                binding
                    .advance_completed(port.now_ns())
                    .map_err(ScheduledProductionServiceCycleError::PdoBatch)?;
                fault = fault.or_else(|| {
                    binding
                        .controller()
                        .last_error()
                        .map(ScheduledProductionServiceFault::PdoConfiguration)
                });
                let recovery = self.recovery(controls, progress, fault);
                let service_ready =
                    cycle.tx.service.failure.is_none() && fault.is_none() && binding.is_complete();
                let startup_phase = services.startup.as_deref().map(StartupController::phase);
                let pdo_batch_status = binding.batch_status();
                return Ok(ScheduledProductionServiceCycleReport {
                    selected,
                    progress,
                    fault,
                    recovery,
                    request: self.request,
                    service_ready,
                    startup_phase,
                    pdo_batch_status,
                    transport: ScheduledProductionServiceTransport::Mailbox(cycle),
                });
            }
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
            let startup_phase = services.startup.as_deref().map(StartupController::phase);
            let pdo_batch_status = services
                .pdo_configuration
                .as_ref()
                .and_then(ScheduledPdoConfiguration::batch_status);
            return Ok(ScheduledProductionServiceCycleReport {
                selected,
                progress,
                fault,
                recovery,
                request: self.request,
                service_ready,
                startup_phase,
                pdo_batch_status,
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
        let startup_phase = services.startup.as_deref().map(StartupController::phase);
        let pdo_batch_status = services
            .pdo_configuration
            .as_ref()
            .and_then(ScheduledPdoConfiguration::batch_status);
        Ok(ScheduledProductionServiceCycleReport {
            selected,
            progress,
            fault,
            recovery,
            request: self.request,
            service_ready,
            startup_phase,
            pdo_batch_status,
            transport: ScheduledProductionServiceTransport::Control(cycle),
        })
    }

    fn release_startup_configuration<
        const MAX_SLAVES: usize,
        const SMS: usize,
        const FMMUS: usize,
        const PDO_OPS: usize,
        const PDO_JOBS: usize,
    >(
        &mut self,
        now_ns: u64,
        services: &mut ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS, PDO_OPS, PDO_JOBS>,
    ) -> Result<(), StartupBarrierReleaseError> {
        let requirements = match services.startup.as_deref() {
            Some(startup) if startup.phase() == StartupPhase::AwaitingConfiguration => {
                startup.configuration_services()
            }
            _ => return Ok(()),
        };

        if requirements.requires_pdo_configuration() && services.pdo_configuration.is_none() {
            return Err(StartupBarrierReleaseError::MissingController(
                ScheduledProductionServiceKind::PdoConfiguration,
            ));
        }
        if requirements.requires_mapping() && services.mapping.is_none() {
            return Err(StartupBarrierReleaseError::MissingController(
                ScheduledProductionServiceKind::Mapping,
            ));
        }
        if requirements.requires_dc_configuration() && services.dc_configuration.is_none() {
            return Err(StartupBarrierReleaseError::MissingController(
                ScheduledProductionServiceKind::DcConfiguration,
            ));
        }

        if self.request.is_some() || self.pdo_action.is_some() {
            return Ok(());
        }
        if requirements.requires_pdo_configuration() {
            services
                .pdo_configuration
                .as_mut()
                .ok_or(StartupBarrierReleaseError::MissingController(
                    ScheduledProductionServiceKind::PdoConfiguration,
                ))?
                .advance_completed(now_ns)
                .map_err(StartupBarrierReleaseError::PdoBatch)?;
        }
        let pdo_complete = !requirements.requires_pdo_configuration()
            || services
                .pdo_configuration
                .as_ref()
                .is_some_and(ScheduledPdoConfiguration::is_complete);
        let mapping_complete = !requirements.requires_mapping()
            || services
                .mapping
                .as_deref()
                .is_some_and(|controller| controller.phase() == MappingConfigPhase::Complete);
        let dc_complete = !requirements.requires_dc_configuration()
            || services
                .dc_configuration
                .as_deref()
                .is_some_and(|controller| controller.phase() == DcPhase::Complete);
        if !(pdo_complete && mapping_complete && dc_complete) {
            return Ok(());
        }

        services
            .startup
            .as_deref_mut()
            .ok_or(StartupBarrierReleaseError::MissingController(
                ScheduledProductionServiceKind::Startup,
            ))?
            .release_configuration(now_ns)
            .map_err(StartupBarrierReleaseError::Startup)?;
        Ok(())
    }

    fn refresh_selection<
        const MAX_SLAVES: usize,
        const SMS: usize,
        const FMMUS: usize,
        const PDO_OPS: usize,
        const PDO_JOBS: usize,
    >(
        &mut self,
        services: &ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS, PDO_OPS, PDO_JOBS>,
    ) {
        if self.request.is_some()
            || self.pdo_action.is_some()
            || self.service_active(services, self.active)
        {
            return;
        }
        if let Some(startup) = services
            .startup
            .as_deref()
            .filter(|controller| controller.phase() == StartupPhase::AwaitingConfiguration)
        {
            let requirements = startup.configuration_services();
            self.active = if requirements.requires_pdo_configuration()
                && !services
                    .pdo_configuration
                    .as_ref()
                    .is_some_and(ScheduledPdoConfiguration::is_complete)
            {
                ScheduledProductionServiceKind::PdoConfiguration
            } else if requirements.requires_mapping()
                && !services
                    .mapping
                    .as_deref()
                    .is_some_and(|controller| controller.phase() == MappingConfigPhase::Complete)
            {
                ScheduledProductionServiceKind::Mapping
            } else if requirements.requires_dc_configuration()
                && !services
                    .dc_configuration
                    .as_deref()
                    .is_some_and(|controller| controller.phase() == DcPhase::Complete)
            {
                ScheduledProductionServiceKind::DcConfiguration
            } else {
                ScheduledProductionServiceKind::Startup
            };
            return;
        }
        self.active = [
            ScheduledProductionServiceKind::Startup,
            ScheduledProductionServiceKind::PdoConfiguration,
            ScheduledProductionServiceKind::Mapping,
            ScheduledProductionServiceKind::DcConfiguration,
            ScheduledProductionServiceKind::Mailbox,
        ]
        .into_iter()
        .find(|kind| self.service_active(services, *kind))
        .unwrap_or(ScheduledProductionServiceKind::Idle);
    }

    fn service_active<
        const MAX_SLAVES: usize,
        const SMS: usize,
        const FMMUS: usize,
        const PDO_OPS: usize,
        const PDO_JOBS: usize,
    >(
        &self,
        services: &ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS, PDO_OPS, PDO_JOBS>,
        kind: ScheduledProductionServiceKind,
    ) -> bool {
        match kind {
            ScheduledProductionServiceKind::Idle => false,
            ScheduledProductionServiceKind::Startup => {
                services.startup.as_deref().is_some_and(|controller| {
                    !matches!(
                        controller.phase(),
                        StartupPhase::Idle
                            | StartupPhase::AwaitingConfiguration
                            | StartupPhase::Ready
                    )
                })
            }
            ScheduledProductionServiceKind::PdoConfiguration => services
                .pdo_configuration
                .as_ref()
                .is_some_and(ScheduledPdoConfiguration::is_active),
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
        const PDO_OPS: usize,
        const PDO_JOBS: usize,
    >(
        &self,
        controls: &ControlRequestPool<REQUESTS>,
        services: &ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS, PDO_OPS, PDO_JOBS>,
    ) -> Result<(), ScheduledProductionServiceKind> {
        if self.active == ScheduledProductionServiceKind::PdoConfiguration {
            let binding = services.pdo_configuration.as_ref().ok_or(self.active)?;
            let action_matches = match self.pdo_action {
                Some(action) => {
                    binding.controller().pending() == Some(action)
                        && binding.mailbox().transaction_matches(
                            action.station_address,
                            action.generation,
                            MailboxProtocol::CoE,
                            action.payload(),
                        )
                }
                None => binding.controller().pending().is_none(),
            };
            if !action_matches {
                return Err(self.active);
            }
        }
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
            ScheduledProductionServiceKind::PdoConfiguration => services
                .pdo_configuration
                .as_ref()
                .and_then(|binding| binding.mailbox().pending())
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
        const PDO_OPS: usize,
        const PDO_JOBS: usize,
    >(
        &mut self,
        now_ns: u64,
        controls: &mut ControlRequestPool<REQUESTS>,
        services: &mut ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS, PDO_OPS, PDO_JOBS>,
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
                    if controller.phase() == StartupPhase::AwaitingConfiguration {
                        return Ok(ScheduledProductionEnqueueOutcome {
                            progress: Some(ScheduledProductionServiceProgress::Startup(
                                StartupProgress::AwaitingConfiguration,
                            )),
                            ..ScheduledProductionEnqueueOutcome::EMPTY
                        });
                    }
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
            ScheduledProductionServiceKind::PdoConfiguration => {
                let binding = services
                    .pdo_configuration
                    .as_mut()
                    .ok_or(ControlError::InvalidState)?;
                if self.pdo_action.is_none() {
                    let action = match binding.controller_mut().next_action(now_ns) {
                        Ok(action) => action,
                        Err(error) => {
                            return Ok(ScheduledProductionEnqueueOutcome {
                                fault: Some(ScheduledProductionServiceFault::PdoConfiguration(
                                    error,
                                )),
                                ..ScheduledProductionEnqueueOutcome::EMPTY
                            });
                        }
                    };
                    let Some(action) = action else {
                        return Ok(ScheduledProductionEnqueueOutcome::EMPTY);
                    };
                    if action.deadline_ns <= now_ns {
                        return Ok(match binding.controller_mut().timeout(action, now_ns) {
                            Ok(progress) => ScheduledProductionEnqueueOutcome {
                                progress: Some(
                                    ScheduledProductionServiceProgress::PdoConfiguration(
                                        ScheduledPdoConfigurationProgress::Configuration(progress),
                                    ),
                                ),
                                ..ScheduledProductionEnqueueOutcome::EMPTY
                            },
                            Err(error) => ScheduledProductionEnqueueOutcome {
                                fault: Some(ScheduledProductionServiceFault::PdoConfiguration(
                                    error,
                                )),
                                ..ScheduledProductionEnqueueOutcome::EMPTY
                            },
                        });
                    }
                    let remaining_ns = action.deadline_ns.saturating_sub(now_ns);
                    let mut mailbox_config =
                        binding.mailbox_config().ok_or(ControlError::InvalidState)?;
                    mailbox_config.timeout_ns = mailbox_config.timeout_ns.min(remaining_ns);
                    mailbox_config.request_timeout_ns =
                        mailbox_config.request_timeout_ns.min(remaining_ns);
                    if let Err(error) = binding.mailbox_mut().start(
                        mailbox_config,
                        action.station_address,
                        action.generation,
                        now_ns,
                        MailboxProtocol::CoE,
                        action.payload(),
                    ) {
                        return Ok(
                            match binding.controller_mut().mailbox_failed(action, error) {
                                Ok(progress) => ScheduledProductionEnqueueOutcome {
                                    progress: Some(
                                        ScheduledProductionServiceProgress::PdoConfiguration(
                                            ScheduledPdoConfigurationProgress::Configuration(
                                                progress,
                                            ),
                                        ),
                                    ),
                                    ..ScheduledProductionEnqueueOutcome::EMPTY
                                },
                                Err(error) => ScheduledProductionEnqueueOutcome {
                                    fault: Some(ScheduledProductionServiceFault::PdoConfiguration(
                                        error,
                                    )),
                                    ..ScheduledProductionEnqueueOutcome::EMPTY
                                },
                            },
                        );
                    }
                    self.pdo_action = Some(action);
                }

                let outcome = enqueue_mailbox(now_ns, controls, binding.mailbox_mut())?;
                if let Some(error) = outcome.fault.or_else(|| {
                    (binding.mailbox().phase() == MailboxPhase::Faulted)
                        .then(|| binding.mailbox().last_error())
                        .flatten()
                }) {
                    let action = self.pdo_action.ok_or(ControlError::InvalidState)?;
                    self.pdo_action = None;
                    return Ok(
                        match binding.controller_mut().mailbox_failed(action, error) {
                            Ok(progress) => ScheduledProductionEnqueueOutcome {
                                progress: Some(
                                    ScheduledProductionServiceProgress::PdoConfiguration(
                                        ScheduledPdoConfigurationProgress::Configuration(progress),
                                    ),
                                ),
                                ..ScheduledProductionEnqueueOutcome::EMPTY
                            },
                            Err(error) => ScheduledProductionEnqueueOutcome {
                                fault: Some(ScheduledProductionServiceFault::PdoConfiguration(
                                    error,
                                )),
                                ..ScheduledProductionEnqueueOutcome::EMPTY
                            },
                        },
                    );
                }
                Ok(ScheduledProductionEnqueueOutcome {
                    request: outcome.request,
                    progress: outcome.progress.map(|progress| {
                        ScheduledProductionServiceProgress::PdoConfiguration(
                            ScheduledPdoConfigurationProgress::Mailbox(progress),
                        )
                    }),
                    fault: None,
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
                let outcome = enqueue_mailbox(now_ns, controls, controller)?;
                Ok(ScheduledProductionEnqueueOutcome {
                    request: outcome.request,
                    progress: outcome
                        .progress
                        .map(ScheduledProductionServiceProgress::Mailbox),
                    fault: outcome.fault.map(ScheduledProductionServiceFault::Mailbox),
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
        const PDO_OPS: usize,
        const PDO_JOBS: usize,
    >(
        &mut self,
        controls: &mut ControlRequestPool<REQUESTS>,
        services: &mut ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS, PDO_OPS, PDO_JOBS>,
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
            ScheduledProductionServiceKind::Idle
            | ScheduledProductionServiceKind::PdoConfiguration
            | ScheduledProductionServiceKind::Mailbox => Err(
                ScheduledProductionServiceFault::Control(ControlError::InvalidState),
            ),
        }
    }

    fn controller_fault<
        const MAX_SLAVES: usize,
        const SMS: usize,
        const FMMUS: usize,
        const PDO_OPS: usize,
        const PDO_JOBS: usize,
    >(
        &self,
        services: &ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS, PDO_OPS, PDO_JOBS>,
    ) -> Option<ScheduledProductionServiceFault> {
        match self.active {
            ScheduledProductionServiceKind::Idle => None,
            ScheduledProductionServiceKind::Startup => services
                .startup
                .as_deref()
                .and_then(StartupController::last_error)
                .map(ScheduledProductionServiceFault::Startup),
            ScheduledProductionServiceKind::PdoConfiguration => services
                .pdo_configuration
                .as_ref()
                .and_then(|binding| binding.controller().last_error())
                .map(ScheduledProductionServiceFault::PdoConfiguration),
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

    fn controller_ready<
        const MAX_SLAVES: usize,
        const SMS: usize,
        const FMMUS: usize,
        const PDO_OPS: usize,
        const PDO_JOBS: usize,
    >(
        &self,
        services: &ScheduledProductionServices<'_, MAX_SLAVES, SMS, FMMUS, PDO_OPS, PDO_JOBS>,
    ) -> bool {
        match self.active {
            ScheduledProductionServiceKind::Idle => true,
            ScheduledProductionServiceKind::Startup => services
                .startup
                .as_deref()
                .is_some_and(|controller| controller.phase() == StartupPhase::Ready),
            ScheduledProductionServiceKind::PdoConfiguration => services
                .pdo_configuration
                .as_ref()
                .is_some_and(ScheduledPdoConfiguration::is_complete),
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

fn enqueue_mailbox<const REQUESTS: usize>(
    now_ns: u64,
    controls: &mut ControlRequestPool<REQUESTS>,
    controller: &mut MailboxController,
) -> Result<ScheduledMailboxEnqueueOutcome, ControlError> {
    let action = match controller.next_action(now_ns) {
        Ok(action) => action,
        Err(error) => {
            return Ok(ScheduledMailboxEnqueueOutcome {
                fault: Some(error),
                ..ScheduledMailboxEnqueueOutcome::EMPTY
            });
        }
    };
    let Some(action) = action else {
        return Ok(ScheduledMailboxEnqueueOutcome::EMPTY);
    };
    if action.deadline_ns <= now_ns {
        return Ok(match controller.timeout(action, now_ns) {
            Ok(progress) => ScheduledMailboxEnqueueOutcome {
                progress: Some(progress),
                ..ScheduledMailboxEnqueueOutcome::EMPTY
            },
            Err(error) => ScheduledMailboxEnqueueOutcome {
                fault: Some(error),
                ..ScheduledMailboxEnqueueOutcome::EMPTY
            },
        });
    }
    Ok(ScheduledMailboxEnqueueOutcome {
        request: Some(controller.enqueue_pending(controls)?),
        ..ScheduledMailboxEnqueueOutcome::EMPTY
    })
}

impl Default for ScheduledProductionServiceScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coe::{CoeHeader, CoeService};
    use crate::mapping::MappingTable;
    use crate::pdo_config::{PdoConfigBatchPlan, PdoConfigJob, PdoConfigPlan, PdoSdoWrite};
    use crate::slave::{EthercatState, SlaveIdentity};
    use crate::startup::{
        ExpectedSlave, StartupAction, StartupConfig, StartupConfigurationServices,
    };

    const EXPECTED: [ExpectedSlave; 1] = [ExpectedSlave {
        position: 0,
        station_address: 0x1000,
        identity: SlaveIdentity {
            vendor_id: 1,
            product_code: 2,
            revision: 3,
            serial: 4,
        },
    }];

    fn startup_at_barrier(requirements: StartupConfigurationServices) -> StartupController<1> {
        let mut startup = StartupController::new(0x1000);
        startup
            .enter_configuration_barrier_for_test(
                7,
                StartupConfig::new(EthercatState::Op).with_configuration_services(requirements),
                &EXPECTED,
            )
            .unwrap();
        startup
    }

    fn complete_current_batch_job<const JOBS: usize>(
        batch: &mut PdoConfigBatch<JOBS, 1>,
        now_ns: u64,
    ) {
        let download = batch.controller_mut().next_action(now_ns).unwrap().unwrap();
        let mut download_response = [0; 6];
        CoeHeader {
            number: 0,
            service: CoeService::SdoResponse,
        }
        .encode(&mut download_response)
        .unwrap();
        download_response[2] = 0x60;
        download_response[3..5].copy_from_slice(&download.sdo_index.to_le_bytes());
        download_response[5] = download.sdo_subindex;
        batch
            .controller_mut()
            .accept(
                download,
                download.generation,
                &download_response,
                now_ns + 1,
            )
            .unwrap();

        let upload = batch
            .controller_mut()
            .next_action(now_ns + 2)
            .unwrap()
            .unwrap();
        let mut upload_response = [0; 10];
        CoeHeader {
            number: 0,
            service: CoeService::SdoResponse,
        }
        .encode(&mut upload_response)
        .unwrap();
        upload_response[2] = 0x4F;
        upload_response[3..5].copy_from_slice(&upload.sdo_index.to_le_bytes());
        upload_response[5] = upload.sdo_subindex;
        upload_response[6] = 0;
        assert_eq!(
            batch
                .controller_mut()
                .accept(upload, upload.generation, &upload_response, now_ns + 3,),
            Ok(PdoConfigProgress::Complete)
        );
    }

    #[test]
    fn pdo_binding_rejects_a_substituted_mailbox_without_a_pool_request() {
        let mut plan = PdoConfigPlan::<1>::new();
        plan.push(PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap())
            .unwrap();
        let mut controller = PdoConfigController::<1>::new();
        controller.start(plan, 1, 41, 0, 1_000, 100).unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        let config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        let mut matching_mailbox = MailboxController::new();
        matching_mailbox
            .start(
                config,
                action.station_address,
                action.generation,
                1,
                MailboxProtocol::CoE,
                action.payload(),
            )
            .unwrap();
        let controls = ControlRequestPool::<1>::new();
        let scheduler = ScheduledProductionServiceScheduler {
            active: ScheduledProductionServiceKind::PdoConfiguration,
            request: None,
            pdo_action: Some(action),
        };
        {
            let services = ScheduledProductionServices::<0, 0, 0, 1>::new(None, None, None, None)
                .with_pdo_configuration(ScheduledPdoConfiguration::new(
                    &mut controller,
                    &mut matching_mailbox,
                    config,
                ));
            assert_eq!(
                scheduler.ensure_request_matches(&controls, &services),
                Ok(())
            );
        }

        let mut substituted_mailbox = MailboxController::new();
        substituted_mailbox
            .start(
                config,
                action.station_address,
                action.generation.wrapping_add(1),
                1,
                MailboxProtocol::CoE,
                action.payload(),
            )
            .unwrap();
        let services = ScheduledProductionServices::<0, 0, 0, 1>::new(None, None, None, None)
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut controller,
                &mut substituted_mailbox,
                config,
            ));
        assert_eq!(
            scheduler.ensure_request_matches(&controls, &services),
            Err(ScheduledProductionServiceKind::PdoConfiguration)
        );
        assert_eq!(controller.pending(), Some(action));
        assert_eq!(scheduler.pdo_action, Some(action));
    }

    #[test]
    fn startup_barrier_reports_missing_required_binding() {
        let mut startup = startup_at_barrier(StartupConfigurationServices::new().with_mapping());
        let mut services =
            ScheduledProductionServices::<1, 0, 0, 0>::new(Some(&mut startup), None, None, None);
        let mut scheduler = ScheduledProductionServiceScheduler::new();

        assert_eq!(
            scheduler.release_startup_configuration(1, &mut services),
            Err(StartupBarrierReleaseError::MissingController(
                ScheduledProductionServiceKind::Mapping
            ))
        );
        assert_eq!(startup.phase(), StartupPhase::AwaitingConfiguration);
    }

    #[test]
    fn startup_barrier_selects_idle_required_services_in_fixed_order() {
        let requirements = StartupConfigurationServices::new()
            .with_pdo_configuration()
            .with_mapping()
            .with_dc_configuration();
        let mailbox_config = MailboxConfig::new(0x1000, 32, 0x1100, 32);

        let mut startup = startup_at_barrier(requirements);
        let mut pdo = PdoConfigController::<0>::new();
        let mut pdo_mailbox = MailboxController::new();
        let mut mapping = MappingConfigController::<0, 0>::new();
        let mut dc = DcController::new();
        let mut scheduler = ScheduledProductionServiceScheduler::new();
        {
            let services = ScheduledProductionServices::new(
                Some(&mut startup),
                Some(&mut mapping),
                Some(&mut dc),
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            ));
            scheduler.refresh_selection(&services);
            assert_eq!(
                scheduler.active(),
                ScheduledProductionServiceKind::PdoConfiguration
            );
        }
        {
            let mut services = ScheduledProductionServices::new(
                Some(&mut startup),
                Some(&mut mapping),
                Some(&mut dc),
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            ));
            let mut controls = ControlRequestPool::<1>::new();
            let outcome = scheduler
                .enqueue_due(1, &mut controls, &mut services)
                .unwrap();
            assert_eq!(
                outcome.fault,
                Some(ScheduledProductionServiceFault::PdoConfiguration(
                    PdoConfigError::NotStarted
                ))
            );
            assert_eq!(outcome.request, None);
        }

        pdo.start(PdoConfigPlan::new(), 0x1000, 7, 0, 100, 10)
            .unwrap();
        {
            let services = ScheduledProductionServices::new(
                Some(&mut startup),
                Some(&mut mapping),
                Some(&mut dc),
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            ));
            scheduler.refresh_selection(&services);
            assert_eq!(scheduler.active(), ScheduledProductionServiceKind::Mapping);
        }

        mapping
            .start(0x1000, 7, 0, 100, 10, &MappingTable::new())
            .unwrap();
        {
            let services = ScheduledProductionServices::new(
                Some(&mut startup),
                Some(&mut mapping),
                Some(&mut dc),
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            ));
            scheduler.refresh_selection(&services);
            assert_eq!(
                scheduler.active(),
                ScheduledProductionServiceKind::DcConfiguration
            );
        }
        assert_eq!(startup.phase(), StartupPhase::AwaitingConfiguration);
    }

    #[test]
    fn startup_barrier_releases_only_after_all_required_controllers_complete() {
        let requirements = StartupConfigurationServices::new()
            .with_pdo_configuration()
            .with_mapping();
        let mut startup = startup_at_barrier(requirements);
        let mut pdo = PdoConfigController::<0>::new();
        pdo.start(PdoConfigPlan::new(), 0x1000, 7, 0, 100, 10)
            .unwrap();
        let mut pdo_mailbox = MailboxController::new();
        let mut mapping = MappingConfigController::<0, 0>::new();
        mapping
            .start(0x1000, 7, 0, 100, 10, &MappingTable::new())
            .unwrap();
        let mailbox_config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        let mut scheduler = ScheduledProductionServiceScheduler::new();
        {
            let mut services = ScheduledProductionServices::new(
                Some(&mut startup),
                Some(&mut mapping),
                None,
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            ));
            assert_eq!(
                scheduler.release_startup_configuration(1, &mut services),
                Ok(())
            );
            scheduler.refresh_selection(&services);
            assert_eq!(scheduler.active(), ScheduledProductionServiceKind::Startup);
        }
        assert_eq!(startup.phase(), StartupPhase::TransitioningAl);
        assert!(matches!(
            startup.next_action(2),
            Ok(Some(StartupAction::Al(_)))
        ));
    }

    #[test]
    fn startup_barrier_advances_the_whole_pdo_batch_before_release() {
        let requirements = StartupConfigurationServices::new().with_pdo_configuration();
        let mut startup = startup_at_barrier(requirements);
        let mailbox_config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        let mut first = PdoConfigPlan::<1>::new();
        first
            .push(PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap())
            .unwrap();
        let mut second = PdoConfigPlan::<1>::new();
        second
            .push(PdoSdoWrite::new(0x1C13, 0, &[0]).unwrap())
            .unwrap();
        let mut plan = PdoConfigBatchPlan::<2, 1>::new();
        plan.push(PdoConfigJob::new(0x1001, first, mailbox_config))
            .unwrap();
        plan.push(PdoConfigJob::new(0x1002, second, mailbox_config))
            .unwrap();
        let mut batch = PdoConfigBatch::new();
        batch.start(plan, 41, 0, 1_000, 100).unwrap();
        complete_current_batch_job(&mut batch, 1);

        let mut scheduler = ScheduledProductionServiceScheduler::new();
        {
            let mut services = ScheduledProductionServices::<1, 0, 0, 1, 2>::new(
                Some(&mut startup),
                None,
                None,
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::batch(&mut batch));
            assert_eq!(
                scheduler.release_startup_configuration(10, &mut services),
                Ok(())
            );
            scheduler.refresh_selection(&services);
            assert_eq!(
                scheduler.active(),
                ScheduledProductionServiceKind::PdoConfiguration
            );
        }
        assert_eq!(startup.phase(), StartupPhase::AwaitingConfiguration);
        assert_eq!(batch.current_index(), 1);
        assert_eq!(batch.status().station_address, Some(0x1002));
        assert_eq!(batch.status().generation, Some(42));

        complete_current_batch_job(&mut batch, 11);
        {
            let mut services = ScheduledProductionServices::<1, 0, 0, 1, 2>::new(
                Some(&mut startup),
                None,
                None,
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::batch(&mut batch));
            assert_eq!(
                scheduler.release_startup_configuration(20, &mut services),
                Ok(())
            );
        }
        assert_eq!(batch.phase(), PdoConfigBatchPhase::Complete);
        assert_eq!(startup.phase(), StartupPhase::TransitioningAl);
    }

    #[test]
    fn faulted_required_service_keeps_the_startup_barrier_closed() {
        let requirements = StartupConfigurationServices::new().with_pdo_configuration();
        let mut startup = startup_at_barrier(requirements);
        let mut plan = PdoConfigPlan::<1>::new();
        plan.push(PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap())
            .unwrap();
        let mut pdo = PdoConfigController::<1>::new();
        pdo.start(plan, 0x1000, 7, 0, 10, 5).unwrap();
        assert_eq!(pdo.next_action(10), Err(PdoConfigError::Timeout));
        let mut pdo_mailbox = MailboxController::new();
        let mailbox_config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        let mut services =
            ScheduledProductionServices::<1, 0, 0, 1>::new(Some(&mut startup), None, None, None)
                .with_pdo_configuration(ScheduledPdoConfiguration::new(
                    &mut pdo,
                    &mut pdo_mailbox,
                    mailbox_config,
                ));
        let mut scheduler = ScheduledProductionServiceScheduler::new();

        assert_eq!(
            scheduler.release_startup_configuration(11, &mut services),
            Ok(())
        );
        scheduler.refresh_selection(&services);
        assert_eq!(
            scheduler.active(),
            ScheduledProductionServiceKind::PdoConfiguration
        );
        assert_eq!(
            scheduler.controller_fault(&services),
            Some(ScheduledProductionServiceFault::PdoConfiguration(
                PdoConfigError::Timeout
            ))
        );
        assert_eq!(startup.phase(), StartupPhase::AwaitingConfiguration);
    }
}
