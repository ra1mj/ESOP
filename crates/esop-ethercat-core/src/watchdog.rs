//! Fixed-capacity ESC watchdog configuration with exact readback.

use crate::control::{
    ControlError, ControlRequestPool, MAX_CONTROL_PAYLOAD, RegisterOperation, RequestHandle,
    RequestState,
};
use crate::registers::fixed_address;

pub const ESC_WATCHDOG_DIVIDER: u16 = 0x0400;
pub const ESC_PROCESS_DATA_WATCHDOG_TIME: u16 = 0x0420;
pub const ESC_WATCHDOG_REGISTER_LEN: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EscWatchdogConfigError {
    Empty,
    ZeroDivider,
    ZeroProcessDataIntervals,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EscWatchdogConfig {
    pub divider: Option<u16>,
    pub process_data_intervals: Option<u16>,
}

impl EscWatchdogConfig {
    pub const fn new(divider: Option<u16>, process_data_intervals: Option<u16>) -> Self {
        Self {
            divider,
            process_data_intervals,
        }
    }

    pub const fn validate(self) -> Result<(), EscWatchdogConfigError> {
        if self.divider.is_none() && self.process_data_intervals.is_none() {
            return Err(EscWatchdogConfigError::Empty);
        }
        if matches!(self.divider, Some(0)) {
            return Err(EscWatchdogConfigError::ZeroDivider);
        }
        if matches!(self.process_data_intervals, Some(0)) {
            return Err(EscWatchdogConfigError::ZeroProcessDataIntervals);
        }
        Ok(())
    }

    const fn first_field(self) -> WatchdogField {
        if self.divider.is_some() {
            WatchdogField::Divider
        } else {
            WatchdogField::ProcessDataIntervals
        }
    }

    const fn value(self, field: WatchdogField) -> Option<u16> {
        match field {
            WatchdogField::Divider => self.divider,
            WatchdogField::ProcessDataIntervals => self.process_data_intervals,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WatchdogPlanEntry {
    pub position: u16,
    pub station_address: u16,
    pub config: EscWatchdogConfig,
}

impl WatchdogPlanEntry {
    const EMPTY: Self = Self {
        position: 0,
        station_address: 0,
        config: EscWatchdogConfig::new(None, None),
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchdogPlanError {
    CapacityExceeded,
    InvalidConfig {
        position: u16,
        error: EscWatchdogConfigError,
    },
    DuplicatePosition(u16),
    DuplicateStationAddress(u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WatchdogPlan<const MAX_SLAVES: usize> {
    entries: [WatchdogPlanEntry; MAX_SLAVES],
    len: usize,
}

impl<const MAX_SLAVES: usize> WatchdogPlan<MAX_SLAVES> {
    pub const fn new() -> Self {
        Self {
            entries: [WatchdogPlanEntry::EMPTY; MAX_SLAVES],
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entries(&self) -> &[WatchdogPlanEntry] {
        &self.entries[..self.len]
    }

    pub fn push(&mut self, entry: WatchdogPlanEntry) -> Result<(), WatchdogPlanError> {
        entry
            .config
            .validate()
            .map_err(|error| WatchdogPlanError::InvalidConfig {
                position: entry.position,
                error,
            })?;
        if self.len == MAX_SLAVES {
            return Err(WatchdogPlanError::CapacityExceeded);
        }
        if self
            .entries()
            .iter()
            .any(|current| current.position == entry.position)
        {
            return Err(WatchdogPlanError::DuplicatePosition(entry.position));
        }
        if self
            .entries()
            .iter()
            .any(|current| current.station_address == entry.station_address)
        {
            return Err(WatchdogPlanError::DuplicateStationAddress(
                entry.station_address,
            ));
        }
        self.entries[self.len] = entry;
        self.len += 1;
        Ok(())
    }
}

impl<const MAX_SLAVES: usize> Default for WatchdogPlan<MAX_SLAVES> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WatchdogControllerConfig {
    pub timeout_ns: u64,
    pub request_timeout_ns: u64,
}

impl WatchdogControllerConfig {
    pub const fn new() -> Self {
        Self {
            timeout_ns: 1_000_000_000,
            request_timeout_ns: 1_000_000,
        }
    }

    pub const fn validate(self) -> Result<(), WatchdogError> {
        if self.timeout_ns == 0 || self.request_timeout_ns == 0 {
            return Err(WatchdogError::InvalidConfiguration);
        }
        Ok(())
    }
}

impl Default for WatchdogControllerConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchdogField {
    Divider,
    ProcessDataIntervals,
}

impl WatchdogField {
    const fn register(self) -> u16 {
        match self {
            Self::Divider => ESC_WATCHDOG_DIVIDER,
            Self::ProcessDataIntervals => ESC_PROCESS_DATA_WATCHDOG_TIME,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchdogPhase {
    Idle,
    Writing,
    Verifying,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WatchdogAction {
    pub token: u8,
    pub datagram_index: u8,
    pub generation: u16,
    pub position: u16,
    pub station_address: u16,
    pub field: WatchdogField,
    pub operation: RegisterOperation,
    pub address: u32,
    pub read_len: u16,
    pub write_payload: [u8; ESC_WATCHDOG_REGISTER_LEN],
    pub write_len: u8,
    pub deadline_ns: u64,
    pub expected_wkc: u16,
}

impl WatchdogAction {
    pub fn payload(&self) -> &[u8] {
        &self.write_payload[..self.write_len as usize]
    }

    pub const fn datagram_len(&self) -> usize {
        let read_len = self.read_len as usize;
        let write_len = self.write_len as usize;
        if read_len > write_len {
            read_len
        } else {
            write_len
        }
    }

    pub const fn response_len(&self) -> usize {
        self.read_len as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchdogProgress {
    Advanced,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchdogError {
    Busy,
    NotStarted,
    NoPendingAction,
    InvalidConfiguration,
    DeadlineOverflow,
    ActionMismatch,
    GenerationMismatch,
    PayloadLengthMismatch,
    UnexpectedWorkingCounter,
    ReadbackMismatch {
        position: u16,
        field: WatchdogField,
        expected: u16,
        actual: u16,
    },
    Timeout,
    Control(ControlError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WatchdogProgrammedSlave {
    pub position: u16,
    pub station_address: u16,
    pub config: EscWatchdogConfig,
}

impl WatchdogProgrammedSlave {
    const EMPTY: Self = Self {
        position: 0,
        station_address: 0,
        config: EscWatchdogConfig::new(None, None),
    };
}

pub struct WatchdogController<const MAX_SLAVES: usize> {
    phase: WatchdogPhase,
    config: WatchdogControllerConfig,
    generation: u16,
    configuration_deadline_ns: u64,
    plan: [WatchdogPlanEntry; MAX_SLAVES],
    plan_len: usize,
    current_index: usize,
    current_field: WatchdogField,
    staged: [WatchdogProgrammedSlave; MAX_SLAVES],
    published_len: usize,
    completed_action_count: usize,
    pending: Option<WatchdogAction>,
    next_token: u8,
    next_datagram_index: u8,
    last_error: Option<WatchdogError>,
}

impl<const MAX_SLAVES: usize> WatchdogController<MAX_SLAVES> {
    pub const fn new() -> Self {
        Self {
            phase: WatchdogPhase::Idle,
            config: WatchdogControllerConfig::new(),
            generation: 0,
            configuration_deadline_ns: 0,
            plan: [WatchdogPlanEntry::EMPTY; MAX_SLAVES],
            plan_len: 0,
            current_index: 0,
            current_field: WatchdogField::Divider,
            staged: [WatchdogProgrammedSlave::EMPTY; MAX_SLAVES],
            published_len: 0,
            completed_action_count: 0,
            pending: None,
            next_token: 1,
            next_datagram_index: 1,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> WatchdogPhase {
        self.phase
    }

    pub const fn pending(&self) -> Option<WatchdogAction> {
        self.pending
    }

    pub const fn last_error(&self) -> Option<WatchdogError> {
        self.last_error
    }

    pub const fn planned_count(&self) -> usize {
        self.plan_len
    }

    pub const fn completed_action_count(&self) -> usize {
        self.completed_action_count
    }

    pub fn programmed_slaves(&self) -> &[WatchdogProgrammedSlave] {
        &self.staged[..self.published_len]
    }

    pub fn programmed_slave(&self, position: u16) -> Option<&WatchdogProgrammedSlave> {
        self.programmed_slaves()
            .iter()
            .find(|slave| slave.position == position)
    }

    pub fn start(
        &mut self,
        config: WatchdogControllerConfig,
        plan: &WatchdogPlan<MAX_SLAVES>,
        generation: u16,
        now_ns: u64,
    ) -> Result<(), WatchdogError> {
        if !matches!(
            self.phase,
            WatchdogPhase::Idle | WatchdogPhase::Complete | WatchdogPhase::Faulted
        ) {
            return Err(WatchdogError::Busy);
        }
        if let Err(error) = config.validate() {
            return self.start_failed(error);
        }
        let configuration_deadline_ns = match now_ns.checked_add(config.timeout_ns) {
            Some(deadline) => deadline,
            None => return self.start_failed(WatchdogError::DeadlineOverflow),
        };

        self.reset_for_start(config, generation, configuration_deadline_ns);
        if plan.is_empty() {
            self.phase = WatchdogPhase::Complete;
            return Ok(());
        }
        self.plan[..plan.len()].copy_from_slice(plan.entries());
        self.plan_len = plan.len();
        self.current_field = self.plan[0].config.first_field();
        self.phase = WatchdogPhase::Writing;
        Ok(())
    }

    pub fn next_action(&mut self, now_ns: u64) -> Result<Option<WatchdogAction>, WatchdogError> {
        if self.phase == WatchdogPhase::Idle {
            return Err(WatchdogError::NotStarted);
        }
        if matches!(self.phase, WatchdogPhase::Complete | WatchdogPhase::Faulted) {
            return Ok(None);
        }
        if let Some(action) = self.pending {
            return Ok(Some(action));
        }
        if now_ns >= self.configuration_deadline_ns {
            return self.fail(WatchdogError::Timeout);
        }
        let deadline_ns = match now_ns.checked_add(self.config.request_timeout_ns) {
            Some(deadline) => deadline.min(self.configuration_deadline_ns),
            None => return self.fail(WatchdogError::DeadlineOverflow),
        };
        let entry = self.plan[self.current_index];
        let value = match entry.config.value(self.current_field) {
            Some(value) => value,
            None => return self.fail(WatchdogError::InvalidConfiguration),
        };
        let mut write_payload = [0; ESC_WATCHDOG_REGISTER_LEN];
        let (operation, read_len, write_len) = match self.phase {
            WatchdogPhase::Writing => {
                write_payload.copy_from_slice(&value.to_le_bytes());
                (RegisterOperation::Write, 0, ESC_WATCHDOG_REGISTER_LEN)
            }
            WatchdogPhase::Verifying => (RegisterOperation::Read, ESC_WATCHDOG_REGISTER_LEN, 0),
            WatchdogPhase::Idle | WatchdogPhase::Complete | WatchdogPhase::Faulted => {
                return Ok(None);
            }
        };
        let action = WatchdogAction {
            token: self.next_token,
            datagram_index: self.next_datagram_index,
            generation: self.generation,
            position: entry.position,
            station_address: entry.station_address,
            field: self.current_field,
            operation,
            address: fixed_address(entry.station_address, self.current_field.register()),
            read_len: read_len as u16,
            write_payload,
            write_len: write_len as u8,
            deadline_ns,
            expected_wkc: 1,
        };
        self.next_token = self.next_token.wrapping_add(1).max(1);
        self.next_datagram_index = self.next_datagram_index.wrapping_add(1).max(1);
        self.pending = Some(action);
        Ok(Some(action))
    }

    pub fn enqueue_pending<const REQUESTS: usize>(
        &self,
        pool: &mut ControlRequestPool<REQUESTS>,
    ) -> Result<RequestHandle, ControlError> {
        let action = self.pending.ok_or(ControlError::InvalidState)?;
        pool.acquire_with_response_len(
            action.datagram_index,
            action.generation,
            action.address,
            action.operation,
            action.payload(),
            action.datagram_len(),
            action.deadline_ns,
        )
    }

    pub fn accept(
        &mut self,
        action: WatchdogAction,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<WatchdogProgress, WatchdogError> {
        if self.pending != Some(action) {
            return self.fail(WatchdogError::ActionMismatch);
        }
        if action.generation != generation {
            return self.fail(WatchdogError::GenerationMismatch);
        }
        if now_ns > action.deadline_ns {
            return self.fail(WatchdogError::Timeout);
        }
        if working_counter != action.expected_wkc {
            return self.fail(WatchdogError::UnexpectedWorkingCounter);
        }
        if payload.len() != action.response_len() {
            return self.fail(WatchdogError::PayloadLengthMismatch);
        }

        self.completed_action_count += 1;
        let progress = match self.phase {
            WatchdogPhase::Writing => {
                self.phase = WatchdogPhase::Verifying;
                WatchdogProgress::Advanced
            }
            WatchdogPhase::Verifying => {
                let actual = u16::from_le_bytes(match payload.try_into() {
                    Ok(bytes) => bytes,
                    Err(_) => return self.fail(WatchdogError::PayloadLengthMismatch),
                });
                let expected = match self.plan[self.current_index]
                    .config
                    .value(self.current_field)
                {
                    Some(value) => value,
                    None => return self.fail(WatchdogError::InvalidConfiguration),
                };
                if actual != expected {
                    return self.fail(WatchdogError::ReadbackMismatch {
                        position: action.position,
                        field: action.field,
                        expected,
                        actual,
                    });
                }
                self.advance_after_readback()
            }
            WatchdogPhase::Idle | WatchdogPhase::Complete | WatchdogPhase::Faulted => {
                return self.fail(WatchdogError::NoPendingAction);
            }
        };
        self.pending = None;
        Ok(progress)
    }

    pub fn accept_completed<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
        handle: RequestHandle,
        now_ns: u64,
    ) -> Result<WatchdogProgress, WatchdogError> {
        let action = match self.pending {
            Some(action) => action,
            None => return self.fail(WatchdogError::NoPendingAction),
        };
        let (generation, actual_wkc, response) = match pool.get(handle) {
            Some(request) if request.state == RequestState::Complete => {
                if !request.matches_action(
                    action.datagram_index,
                    action.generation,
                    action.address,
                    action.operation,
                    action.payload(),
                    action.datagram_len(),
                    action.deadline_ns,
                ) {
                    let _ = pool.release(handle);
                    return self.fail(WatchdogError::ActionMismatch);
                }
                let mut response = [0; MAX_CONTROL_PAYLOAD];
                response[..request.length].copy_from_slice(request.payload());
                (request.generation, request.actual_wkc, response)
            }
            Some(request) if request.state == RequestState::Failed => {
                if !request.matches_action(
                    action.datagram_index,
                    action.generation,
                    action.address,
                    action.operation,
                    action.payload(),
                    action.datagram_len(),
                    action.deadline_ns,
                ) {
                    let _ = pool.release(handle);
                    return self.fail(WatchdogError::ActionMismatch);
                }
                let error = request.last_error().unwrap_or(ControlError::InvalidState);
                if let Err(release_error) = pool.release(handle) {
                    return self.fail(WatchdogError::Control(release_error));
                }
                if error == ControlError::Timeout {
                    return self.timeout(action, now_ns);
                }
                return self.fail(WatchdogError::Control(error));
            }
            Some(_) => return Err(WatchdogError::Control(ControlError::InvalidState)),
            None => return self.fail(WatchdogError::Control(ControlError::InvalidHandle)),
        };
        let progress = self.accept(
            action,
            generation,
            &response[..action.response_len()],
            actual_wkc,
            now_ns,
        );
        let release = pool.release(handle);
        match (progress, release) {
            (Ok(progress), Ok(())) => Ok(progress),
            (Ok(_), Err(error)) => self.fail(WatchdogError::Control(error)),
            (Err(error), _) => Err(error),
        }
    }

    pub fn timeout(
        &mut self,
        action: WatchdogAction,
        now_ns: u64,
    ) -> Result<WatchdogProgress, WatchdogError> {
        if self.pending != Some(action) {
            return self.fail(WatchdogError::ActionMismatch);
        }
        if now_ns < action.deadline_ns {
            return Err(WatchdogError::Timeout);
        }
        self.fail(WatchdogError::Timeout)
    }

    fn advance_after_readback(&mut self) -> WatchdogProgress {
        let entry = self.plan[self.current_index];
        if self.current_field == WatchdogField::Divider
            && entry.config.process_data_intervals.is_some()
        {
            self.current_field = WatchdogField::ProcessDataIntervals;
            self.phase = WatchdogPhase::Writing;
            return WatchdogProgress::Advanced;
        }

        self.staged[self.current_index] = WatchdogProgrammedSlave {
            position: entry.position,
            station_address: entry.station_address,
            config: entry.config,
        };
        self.current_index += 1;
        if self.current_index == self.plan_len {
            self.phase = WatchdogPhase::Complete;
            self.published_len = self.plan_len;
            WatchdogProgress::Complete
        } else {
            self.current_field = self.plan[self.current_index].config.first_field();
            self.phase = WatchdogPhase::Writing;
            WatchdogProgress::Advanced
        }
    }

    fn reset_for_start(
        &mut self,
        config: WatchdogControllerConfig,
        generation: u16,
        configuration_deadline_ns: u64,
    ) {
        self.config = config;
        self.generation = generation;
        self.configuration_deadline_ns = configuration_deadline_ns;
        self.plan = [WatchdogPlanEntry::EMPTY; MAX_SLAVES];
        self.plan_len = 0;
        self.current_index = 0;
        self.current_field = WatchdogField::Divider;
        self.staged = [WatchdogProgrammedSlave::EMPTY; MAX_SLAVES];
        self.published_len = 0;
        self.completed_action_count = 0;
        self.pending = None;
        self.next_token = 1;
        self.next_datagram_index = 1;
        self.last_error = None;
    }

    fn start_failed<T>(&mut self, error: WatchdogError) -> Result<T, WatchdogError> {
        self.phase = WatchdogPhase::Faulted;
        self.plan = [WatchdogPlanEntry::EMPTY; MAX_SLAVES];
        self.plan_len = 0;
        self.current_index = 0;
        self.staged = [WatchdogProgrammedSlave::EMPTY; MAX_SLAVES];
        self.published_len = 0;
        self.completed_action_count = 0;
        self.pending = None;
        self.last_error = Some(error);
        Err(error)
    }

    fn fail<T>(&mut self, error: WatchdogError) -> Result<T, WatchdogError> {
        self.phase = WatchdogPhase::Faulted;
        self.pending = None;
        self.published_len = 0;
        if self.last_error.is_none() {
            self.last_error = Some(error);
        }
        Err(error)
    }
}

impl<const MAX_SLAVES: usize> Default for WatchdogController<MAX_SLAVES> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTROLLER_CONFIG: WatchdogControllerConfig = WatchdogControllerConfig {
        timeout_ns: 100,
        request_timeout_ns: 10,
    };

    fn entry(
        position: u16,
        station_address: u16,
        divider: Option<u16>,
        intervals: Option<u16>,
    ) -> WatchdogPlanEntry {
        WatchdogPlanEntry {
            position,
            station_address,
            config: EscWatchdogConfig::new(divider, intervals),
        }
    }

    fn accept_action<const MAX_SLAVES: usize>(
        controller: &mut WatchdogController<MAX_SLAVES>,
        action: WatchdogAction,
        response: &[u8],
        now_ns: u64,
    ) -> Result<WatchdogProgress, WatchdogError> {
        controller.accept(action, action.generation, response, 1, now_ns)
    }

    #[test]
    fn config_and_plan_reject_ambiguous_or_duplicate_values() {
        assert_eq!(
            EscWatchdogConfig::new(None, None).validate(),
            Err(EscWatchdogConfigError::Empty)
        );
        assert_eq!(
            EscWatchdogConfig::new(Some(0), None).validate(),
            Err(EscWatchdogConfigError::ZeroDivider)
        );
        assert_eq!(
            EscWatchdogConfig::new(None, Some(0)).validate(),
            Err(EscWatchdogConfigError::ZeroProcessDataIntervals)
        );

        let mut plan = WatchdogPlan::<1>::new();
        plan.push(entry(0, 0x1001, Some(2500), None)).unwrap();
        assert_eq!(
            plan.push(entry(0, 0x1002, Some(1), None)),
            Err(WatchdogPlanError::CapacityExceeded)
        );

        let mut duplicates = WatchdogPlan::<3>::new();
        duplicates.push(entry(0, 0x1001, Some(2500), None)).unwrap();
        assert_eq!(
            duplicates.push(entry(0, 0x1002, None, Some(10))),
            Err(WatchdogPlanError::DuplicatePosition(0))
        );
        assert_eq!(
            duplicates.push(entry(1, 0x1001, None, Some(10))),
            Err(WatchdogPlanError::DuplicateStationAddress(0x1001))
        );
    }

    #[test]
    fn empty_plan_completes_without_actions() {
        let mut controller = WatchdogController::<2>::new();
        controller
            .start(CONTROLLER_CONFIG, &WatchdogPlan::new(), 7, 1)
            .unwrap();
        assert_eq!(controller.phase(), WatchdogPhase::Complete);
        assert_eq!(controller.next_action(2), Ok(None));
        assert!(controller.programmed_slaves().is_empty());
    }

    #[test]
    fn controller_programs_optional_fields_in_product_order_and_publishes_atomically() {
        let mut plan = WatchdogPlan::<2>::new();
        plan.push(entry(0, 0x1001, Some(2500), Some(100))).unwrap();
        plan.push(entry(2, 0x1003, None, Some(40))).unwrap();
        let mut controller = WatchdogController::<2>::new();
        controller.start(CONTROLLER_CONFIG, &plan, 7, 1).unwrap();

        let write_divider = controller.next_action(2).unwrap().unwrap();
        assert_eq!(write_divider.position, 0);
        assert_eq!(write_divider.field, WatchdogField::Divider);
        assert_eq!(write_divider.operation, RegisterOperation::Write);
        assert_eq!(write_divider.address, fixed_address(0x1001, 0x0400));
        assert_eq!(write_divider.payload(), &2500u16.to_le_bytes());
        assert_eq!(
            accept_action(&mut controller, write_divider, &[], 3),
            Ok(WatchdogProgress::Advanced)
        );

        let read_divider = controller.next_action(4).unwrap().unwrap();
        assert_eq!(read_divider.operation, RegisterOperation::Read);
        assert_eq!(read_divider.response_len(), 2);
        assert_eq!(
            accept_action(&mut controller, read_divider, &2500u16.to_le_bytes(), 5),
            Ok(WatchdogProgress::Advanced)
        );
        assert!(controller.programmed_slaves().is_empty());

        let write_intervals = controller.next_action(6).unwrap().unwrap();
        assert_eq!(write_intervals.field, WatchdogField::ProcessDataIntervals);
        assert_eq!(write_intervals.address, fixed_address(0x1001, 0x0420));
        assert_eq!(write_intervals.payload(), &100u16.to_le_bytes());
        accept_action(&mut controller, write_intervals, &[], 7).unwrap();
        let read_intervals = controller.next_action(8).unwrap().unwrap();
        accept_action(&mut controller, read_intervals, &100u16.to_le_bytes(), 9).unwrap();
        assert!(controller.programmed_slaves().is_empty());

        let write_second = controller.next_action(10).unwrap().unwrap();
        assert_eq!(write_second.position, 2);
        assert_eq!(write_second.field, WatchdogField::ProcessDataIntervals);
        accept_action(&mut controller, write_second, &[], 11).unwrap();
        let read_second = controller.next_action(12).unwrap().unwrap();
        assert_eq!(
            accept_action(&mut controller, read_second, &40u16.to_le_bytes(), 13),
            Ok(WatchdogProgress::Complete)
        );
        assert_eq!(controller.phase(), WatchdogPhase::Complete);
        assert_eq!(controller.completed_action_count(), 6);
        assert_eq!(controller.programmed_slaves().len(), 2);
        assert_eq!(
            controller.programmed_slaves()[0].config,
            plan.entries()[0].config
        );
        assert_eq!(controller.programmed_slaves()[1].position, 2);
    }

    #[test]
    fn control_pool_completion_preserves_exact_action_ownership() {
        let mut plan = WatchdogPlan::<1>::new();
        plan.push(entry(0, 0x1001, Some(2500), None)).unwrap();
        let mut controller = WatchdogController::<1>::new();
        controller.start(CONTROLLER_CONFIG, &plan, 9, 1).unwrap();
        let mut pool = ControlRequestPool::<1>::new();

        for now_ns in [2, 4] {
            let action = controller.next_action(now_ns).unwrap().unwrap();
            let handle = controller.enqueue_pending(&mut pool).unwrap();
            let mut frame = [0; 128];
            pool.get_mut(handle)
                .unwrap()
                .build_frame(&mut frame, [0; 6], [1; 6])
                .unwrap();
            let response = if action.operation == RegisterOperation::Read {
                2500u16.to_le_bytes()
            } else {
                action.write_payload
            };
            pool.get_mut(handle)
                .unwrap()
                .complete(action.generation, action.address, &response, 1)
                .unwrap();
            controller
                .accept_completed(&mut pool, handle, now_ns + 1)
                .unwrap();
            assert!(pool.get(handle).is_none());
        }
        assert_eq!(controller.phase(), WatchdogPhase::Complete);
    }

    #[test]
    fn malformed_readbacks_and_substituted_actions_fail_closed() {
        let mut plan = WatchdogPlan::<1>::new();
        plan.push(entry(0, 0x1001, Some(2500), None)).unwrap();

        let mut action_mismatch = WatchdogController::<1>::new();
        action_mismatch
            .start(CONTROLLER_CONFIG, &plan, 7, 1)
            .unwrap();
        let action = action_mismatch.next_action(2).unwrap().unwrap();
        let mut substituted = action;
        substituted.token += 1;
        assert_eq!(
            action_mismatch.accept(substituted, 7, &[], 1, 3),
            Err(WatchdogError::ActionMismatch)
        );
        assert!(action_mismatch.programmed_slaves().is_empty());

        let cases = [
            (8, &[][..], 1, WatchdogError::GenerationMismatch),
            (7, &[0][..], 1, WatchdogError::PayloadLengthMismatch),
            (
                7,
                &2500u16.to_le_bytes()[..],
                0,
                WatchdogError::UnexpectedWorkingCounter,
            ),
        ];
        for (generation, payload, wkc, expected) in cases {
            let mut controller = WatchdogController::<1>::new();
            controller.start(CONTROLLER_CONFIG, &plan, 7, 1).unwrap();
            let write = controller.next_action(2).unwrap().unwrap();
            accept_action(&mut controller, write, &[], 3).unwrap();
            let read = controller.next_action(4).unwrap().unwrap();
            assert_eq!(
                controller.accept(read, generation, payload, wkc, 5),
                Err(expected)
            );
            assert_eq!(controller.phase(), WatchdogPhase::Faulted);
            assert!(controller.programmed_slaves().is_empty());
        }

        let mut mismatch = WatchdogController::<1>::new();
        mismatch.start(CONTROLLER_CONFIG, &plan, 7, 1).unwrap();
        let write = mismatch.next_action(2).unwrap().unwrap();
        accept_action(&mut mismatch, write, &[], 3).unwrap();
        let read = mismatch.next_action(4).unwrap().unwrap();
        assert_eq!(
            mismatch.accept(read, 7, &2499u16.to_le_bytes(), 1, 5),
            Err(WatchdogError::ReadbackMismatch {
                position: 0,
                field: WatchdogField::Divider,
                expected: 2500,
                actual: 2499,
            })
        );
    }

    #[test]
    fn deadline_failures_latch_and_explicit_restart_clears_them() {
        let mut plan = WatchdogPlan::<1>::new();
        plan.push(entry(0, 0x1001, Some(2500), None)).unwrap();
        let mut controller = WatchdogController::<1>::new();
        assert_eq!(
            controller.start(CONTROLLER_CONFIG, &plan, 1, u64::MAX - 50),
            Err(WatchdogError::DeadlineOverflow)
        );
        assert_eq!(controller.phase(), WatchdogPhase::Faulted);

        controller.start(CONTROLLER_CONFIG, &plan, 2, 1).unwrap();
        let action = controller.next_action(2).unwrap().unwrap();
        assert_eq!(
            controller.timeout(action, action.deadline_ns),
            Err(WatchdogError::Timeout)
        );
        assert_eq!(controller.last_error(), Some(WatchdogError::Timeout));

        controller.start(CONTROLLER_CONFIG, &plan, 3, 20).unwrap();
        assert_eq!(controller.phase(), WatchdogPhase::Writing);
        assert_eq!(controller.last_error(), None);
        assert_eq!(controller.completed_action_count(), 0);
    }
}
