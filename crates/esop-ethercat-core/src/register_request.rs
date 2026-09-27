//! Allocation-free asynchronous fixed-station ESC register requests.
//!
//! Applications enqueue reads and writes outside the cyclic wire path. The
//! production scheduler admits at most one frozen action into the shared
//! control-request pool, while completed results remain in caller-owned slots
//! until explicitly released.

use crate::control::{
    ControlError, ControlRequestPool, MAX_CONTROL_PAYLOAD, RegisterOperation, RequestHandle,
    RequestState,
};
use crate::registers::fixed_address;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EscRegisterRequestHandle {
    slot: u8,
    generation: u16,
}

impl EscRegisterRequestHandle {
    pub const fn slot(self) -> usize {
        self.slot as usize
    }

    pub const fn generation(self) -> u16 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EscRegisterRequestState {
    Free = 0,
    Queued = 1,
    Busy = 2,
    Success = 3,
    Error = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EscRegisterRequestError {
    InvalidCapacity,
    CapacityExceeded,
    InvalidStationAddress,
    InvalidLength,
    RegisterRangeOverflow,
    InvalidTimeout,
    InvalidHandle,
    InvalidState,
    QueueCorrupt,
    ActionMismatch(EscRegisterRequestHandle),
    Timeout(EscRegisterRequestHandle),
    Control {
        request: EscRegisterRequestHandle,
        error: ControlError,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EscRegisterRequestStatus {
    pub handle: EscRegisterRequestHandle,
    pub state: EscRegisterRequestState,
    pub operation: RegisterOperation,
    pub station_address: u16,
    pub register_address: u16,
    pub length: usize,
    pub deadline_ns: u64,
    pub actual_wkc: u16,
    pub error: Option<EscRegisterRequestError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EscRegisterRequestProgress {
    ReadComplete(EscRegisterRequestHandle),
    WriteComplete(EscRegisterRequestHandle),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EscRegisterAction {
    pub handle: EscRegisterRequestHandle,
    pub datagram_index: u8,
    pub generation: u16,
    pub address: u32,
    pub operation: RegisterOperation,
    pub length: usize,
    pub deadline_ns: u64,
    payload: [u8; MAX_CONTROL_PAYLOAD],
}

impl EscRegisterAction {
    pub fn payload(&self) -> &[u8] {
        &self.payload[..self.length]
    }

    pub const fn datagram_len(&self) -> usize {
        self.length
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EscRegisterRequestSlot {
    generation: u16,
    state: EscRegisterRequestState,
    operation: RegisterOperation,
    station_address: u16,
    register_address: u16,
    length: usize,
    deadline_ns: u64,
    actual_wkc: u16,
    error: Option<EscRegisterRequestError>,
    data: [u8; MAX_CONTROL_PAYLOAD],
}

impl EscRegisterRequestSlot {
    const EMPTY: Self = Self {
        generation: 0,
        state: EscRegisterRequestState::Free,
        operation: RegisterOperation::Read,
        station_address: 0,
        register_address: 0,
        length: 0,
        deadline_ns: 0,
        actual_wkc: 0,
        error: None,
        data: [0; MAX_CONTROL_PAYLOAD],
    };
}

pub struct EscRegisterRequestController<const CAPACITY: usize> {
    datagram_index: u8,
    next_generation: u16,
    slots: [EscRegisterRequestSlot; CAPACITY],
    queue: [u8; CAPACITY],
    queue_head: usize,
    queue_len: usize,
    pending: Option<EscRegisterAction>,
}

impl<const CAPACITY: usize> EscRegisterRequestController<CAPACITY> {
    pub const fn new(datagram_index: u8) -> Self {
        Self {
            datagram_index,
            next_generation: 1,
            slots: [EscRegisterRequestSlot::EMPTY; CAPACITY],
            queue: [0; CAPACITY],
            queue_head: 0,
            queue_len: 0,
            pending: None,
        }
    }

    pub fn len(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.state != EscRegisterRequestState::Free)
            .count()
    }

    pub const fn queued_len(&self) -> usize {
        self.queue_len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub const fn has_pending(&self) -> bool {
        self.queue_len != 0
    }

    pub const fn pending(&self) -> Option<EscRegisterAction> {
        self.pending
    }

    pub fn submit_read(
        &mut self,
        station_address: u16,
        register_address: u16,
        length: usize,
        now_ns: u64,
        timeout_ns: u64,
    ) -> Result<EscRegisterRequestHandle, EscRegisterRequestError> {
        self.submit(
            station_address,
            register_address,
            RegisterOperation::Read,
            &[],
            length,
            now_ns,
            timeout_ns,
        )
    }

    pub fn submit_write(
        &mut self,
        station_address: u16,
        register_address: u16,
        data: &[u8],
        now_ns: u64,
        timeout_ns: u64,
    ) -> Result<EscRegisterRequestHandle, EscRegisterRequestError> {
        self.submit(
            station_address,
            register_address,
            RegisterOperation::Write,
            data,
            data.len(),
            now_ns,
            timeout_ns,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn submit(
        &mut self,
        station_address: u16,
        register_address: u16,
        operation: RegisterOperation,
        data: &[u8],
        length: usize,
        now_ns: u64,
        timeout_ns: u64,
    ) -> Result<EscRegisterRequestHandle, EscRegisterRequestError> {
        if CAPACITY == 0 || CAPACITY > 64 {
            return Err(EscRegisterRequestError::InvalidCapacity);
        }
        if station_address == 0 {
            return Err(EscRegisterRequestError::InvalidStationAddress);
        }
        if length == 0 || length > MAX_CONTROL_PAYLOAD || data.len() > length {
            return Err(EscRegisterRequestError::InvalidLength);
        }
        if usize::from(register_address)
            .checked_add(length)
            .is_none_or(|end| end > usize::from(u16::MAX) + 1)
        {
            return Err(EscRegisterRequestError::RegisterRangeOverflow);
        }
        if timeout_ns == 0 {
            return Err(EscRegisterRequestError::InvalidTimeout);
        }
        let deadline_ns = now_ns
            .checked_add(timeout_ns)
            .ok_or(EscRegisterRequestError::InvalidTimeout)?;
        let slot_index = self
            .slots
            .iter()
            .position(|slot| slot.state == EscRegisterRequestState::Free)
            .ok_or(EscRegisterRequestError::CapacityExceeded)?;
        if self.queue_len == CAPACITY {
            return Err(EscRegisterRequestError::CapacityExceeded);
        }

        let generation = self.next_generation;
        let handle = EscRegisterRequestHandle {
            slot: slot_index as u8,
            generation,
        };
        let mut slot = EscRegisterRequestSlot {
            generation,
            state: EscRegisterRequestState::Queued,
            operation,
            station_address,
            register_address,
            length,
            deadline_ns,
            actual_wkc: 0,
            error: None,
            data: [0; MAX_CONTROL_PAYLOAD],
        };
        slot.data[..data.len()].copy_from_slice(data);

        let tail = (self.queue_head + self.queue_len) % CAPACITY;
        self.slots[slot_index] = slot;
        self.queue[tail] = slot_index as u8;
        self.queue_len += 1;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        Ok(handle)
    }

    pub fn status(
        &self,
        handle: EscRegisterRequestHandle,
    ) -> Result<EscRegisterRequestStatus, EscRegisterRequestError> {
        let slot = self.slot(handle)?;
        Ok(EscRegisterRequestStatus {
            handle,
            state: slot.state,
            operation: slot.operation,
            station_address: slot.station_address,
            register_address: slot.register_address,
            length: slot.length,
            deadline_ns: slot.deadline_ns,
            actual_wkc: slot.actual_wkc,
            error: slot.error,
        })
    }

    pub fn data(&self, handle: EscRegisterRequestHandle) -> Result<&[u8], EscRegisterRequestError> {
        let slot = self.slot(handle)?;
        Ok(&slot.data[..slot.length])
    }

    pub fn release(
        &mut self,
        handle: EscRegisterRequestHandle,
    ) -> Result<(), EscRegisterRequestError> {
        let slot = self.slot_mut(handle)?;
        if !matches!(
            slot.state,
            EscRegisterRequestState::Success | EscRegisterRequestState::Error
        ) {
            return Err(EscRegisterRequestError::InvalidState);
        }
        *slot = EscRegisterRequestSlot::EMPTY;
        Ok(())
    }

    pub fn next_action(
        &mut self,
        now_ns: u64,
    ) -> Result<Option<EscRegisterAction>, EscRegisterRequestError> {
        if let Some(action) = self.pending {
            if now_ns >= action.deadline_ns {
                let error = EscRegisterRequestError::Timeout(action.handle);
                self.finish_error(action.handle, error)?;
                return Err(error);
            }
            return Ok(Some(action));
        }
        let Some(handle) = self.head_handle()? else {
            return Ok(None);
        };
        let slot = self.slot(handle)?;
        if slot.state != EscRegisterRequestState::Queued {
            return Err(EscRegisterRequestError::QueueCorrupt);
        }
        if now_ns >= slot.deadline_ns {
            let error = EscRegisterRequestError::Timeout(handle);
            self.finish_error(handle, error)?;
            return Err(error);
        }

        let mut payload = [0; MAX_CONTROL_PAYLOAD];
        payload[..slot.length].copy_from_slice(&slot.data[..slot.length]);
        let action = EscRegisterAction {
            handle,
            datagram_index: self.datagram_index,
            generation: handle.generation,
            address: fixed_address(slot.station_address, slot.register_address),
            operation: slot.operation,
            length: slot.length,
            deadline_ns: slot.deadline_ns,
            payload,
        };
        self.slot_mut(handle)?.state = EscRegisterRequestState::Busy;
        self.pending = Some(action);
        Ok(Some(action))
    }

    pub fn enqueue_pending<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
    ) -> Result<RequestHandle, EscRegisterRequestError> {
        let action = self.pending.ok_or(EscRegisterRequestError::InvalidState)?;
        match pool.acquire(
            action.datagram_index,
            action.generation,
            action.address,
            action.operation,
            action.payload(),
            action.deadline_ns,
        ) {
            Ok(handle) => Ok(handle),
            Err(error) => {
                let request_error = EscRegisterRequestError::Control {
                    request: action.handle,
                    error,
                };
                self.finish_error(action.handle, request_error)?;
                Err(request_error)
            }
        }
    }

    pub fn accept_completed<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
        handle: RequestHandle,
        now_ns: u64,
    ) -> Result<EscRegisterRequestProgress, EscRegisterRequestError> {
        let action = self.pending.ok_or(EscRegisterRequestError::InvalidState)?;
        let mut response = [0; MAX_CONTROL_PAYLOAD];
        let (actual_wkc, control_error, matches) = match pool.get(handle) {
            Some(request) if request.state == RequestState::Complete => {
                let matches = request.matches_action(
                    action.datagram_index,
                    action.generation,
                    action.address,
                    action.operation,
                    action.payload(),
                    action.datagram_len(),
                    action.deadline_ns,
                ) && request.length == action.length;
                if matches {
                    response[..request.length].copy_from_slice(request.payload());
                }
                (request.actual_wkc, None, matches)
            }
            Some(request) if request.state == RequestState::Failed => {
                let matches = request.matches_action(
                    action.datagram_index,
                    action.generation,
                    action.address,
                    action.operation,
                    action.payload(),
                    action.datagram_len(),
                    action.deadline_ns,
                );
                (
                    request.actual_wkc,
                    Some(request.last_error().unwrap_or(ControlError::InvalidState)),
                    matches,
                )
            }
            Some(_) => return Err(EscRegisterRequestError::InvalidState),
            None => {
                let error = EscRegisterRequestError::Control {
                    request: action.handle,
                    error: ControlError::InvalidHandle,
                };
                self.finish_error(action.handle, error)?;
                return Err(error);
            }
        };

        let release = pool.release(handle);
        self.slot_mut(action.handle)?.actual_wkc = actual_wkc;
        if !matches {
            let error = EscRegisterRequestError::ActionMismatch(action.handle);
            self.finish_error(action.handle, error)?;
            return Err(error);
        }
        if let Err(error) = release {
            let error = EscRegisterRequestError::Control {
                request: action.handle,
                error,
            };
            self.finish_error(action.handle, error)?;
            return Err(error);
        }
        if let Some(error) = control_error {
            let request_error = if error == ControlError::Timeout {
                EscRegisterRequestError::Timeout(action.handle)
            } else {
                EscRegisterRequestError::Control {
                    request: action.handle,
                    error,
                }
            };
            self.finish_error(action.handle, request_error)?;
            return Err(request_error);
        }
        if now_ns > action.deadline_ns {
            let error = EscRegisterRequestError::Timeout(action.handle);
            self.finish_error(action.handle, error)?;
            return Err(error);
        }

        let operation = action.operation;
        self.finish_success(action, &response[..action.length], actual_wkc)?;
        Ok(match operation {
            RegisterOperation::Read => EscRegisterRequestProgress::ReadComplete(action.handle),
            RegisterOperation::Write => EscRegisterRequestProgress::WriteComplete(action.handle),
            RegisterOperation::AutoIncrementRead | RegisterOperation::AutoIncrementWrite => {
                return Err(EscRegisterRequestError::QueueCorrupt);
            }
        })
    }

    fn finish_success(
        &mut self,
        action: EscRegisterAction,
        response: &[u8],
        actual_wkc: u16,
    ) -> Result<(), EscRegisterRequestError> {
        if self.pending != Some(action) || self.head_handle()? != Some(action.handle) {
            return Err(EscRegisterRequestError::QueueCorrupt);
        }
        let slot = self.slot_mut(action.handle)?;
        if slot.state != EscRegisterRequestState::Busy || response.len() != slot.length {
            return Err(EscRegisterRequestError::QueueCorrupt);
        }
        if action.operation == RegisterOperation::Read {
            slot.data[..response.len()].copy_from_slice(response);
        }
        slot.actual_wkc = actual_wkc;
        slot.state = EscRegisterRequestState::Success;
        slot.error = None;
        self.pending = None;
        self.pop_head()?;
        Ok(())
    }

    fn finish_error(
        &mut self,
        handle: EscRegisterRequestHandle,
        error: EscRegisterRequestError,
    ) -> Result<(), EscRegisterRequestError> {
        if self.head_handle()? != Some(handle) {
            return Err(EscRegisterRequestError::QueueCorrupt);
        }
        let slot = self.slot_mut(handle)?;
        if !matches!(
            slot.state,
            EscRegisterRequestState::Queued | EscRegisterRequestState::Busy
        ) {
            return Err(EscRegisterRequestError::QueueCorrupt);
        }
        slot.state = EscRegisterRequestState::Error;
        slot.error = Some(error);
        self.pending = None;
        self.pop_head()?;
        Ok(())
    }

    fn head_handle(&self) -> Result<Option<EscRegisterRequestHandle>, EscRegisterRequestError> {
        if self.queue_len == 0 {
            return Ok(None);
        }
        if CAPACITY == 0 || self.queue_head >= CAPACITY {
            return Err(EscRegisterRequestError::QueueCorrupt);
        }
        let slot_index = self.queue[self.queue_head] as usize;
        let slot = self
            .slots
            .get(slot_index)
            .ok_or(EscRegisterRequestError::QueueCorrupt)?;
        Ok(Some(EscRegisterRequestHandle {
            slot: slot_index as u8,
            generation: slot.generation,
        }))
    }

    fn pop_head(&mut self) -> Result<(), EscRegisterRequestError> {
        if self.queue_len == 0 || CAPACITY == 0 {
            return Err(EscRegisterRequestError::QueueCorrupt);
        }
        self.queue_head = (self.queue_head + 1) % CAPACITY;
        self.queue_len -= 1;
        if self.queue_len == 0 {
            self.queue_head = 0;
        }
        Ok(())
    }

    fn slot(
        &self,
        handle: EscRegisterRequestHandle,
    ) -> Result<&EscRegisterRequestSlot, EscRegisterRequestError> {
        let slot = self
            .slots
            .get(handle.slot())
            .ok_or(EscRegisterRequestError::InvalidHandle)?;
        if slot.state == EscRegisterRequestState::Free || slot.generation != handle.generation {
            return Err(EscRegisterRequestError::InvalidHandle);
        }
        Ok(slot)
    }

    fn slot_mut(
        &mut self,
        handle: EscRegisterRequestHandle,
    ) -> Result<&mut EscRegisterRequestSlot, EscRegisterRequestError> {
        let slot = self
            .slots
            .get_mut(handle.slot())
            .ok_or(EscRegisterRequestError::InvalidHandle)?;
        if slot.state == EscRegisterRequestState::Free || slot.generation != handle.generation {
            return Err(EscRegisterRequestError::InvalidHandle);
        }
        Ok(slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(
        controller: &mut EscRegisterRequestController<2>,
        pool: &mut ControlRequestPool<1>,
        response: &[u8],
    ) -> EscRegisterRequestProgress {
        let action = controller.next_action(1).unwrap().unwrap();
        let handle = controller.enqueue_pending(pool).unwrap();
        let mut frame = [0; 256];
        pool.get_mut(handle)
            .unwrap()
            .build_frame(&mut frame, [0; 6], [1; 6])
            .unwrap();
        pool.complete(handle, action.generation, action.address, response, 1)
            .unwrap();
        controller.accept_completed(pool, handle, 2).unwrap()
    }

    #[test]
    fn queues_read_and_write_in_fifo_order_and_retains_results() {
        let mut controller = EscRegisterRequestController::<2>::new(90);
        let read = controller.submit_read(0x1000, 0x0130, 2, 0, 100).unwrap();
        let write = controller
            .submit_write(0x1001, 0x0120, &[0x08, 0x00], 0, 100)
            .unwrap();

        assert_eq!(controller.queued_len(), 2);
        assert_eq!(
            complete(
                &mut controller,
                &mut ControlRequestPool::new(),
                &[0x08, 0x00]
            ),
            EscRegisterRequestProgress::ReadComplete(read)
        );
        assert_eq!(controller.data(read).unwrap(), &[0x08, 0x00]);
        assert_eq!(controller.status(read).unwrap().actual_wkc, 1);
        assert_eq!(
            controller.status(write).unwrap().state,
            EscRegisterRequestState::Queued
        );

        assert_eq!(
            complete(
                &mut controller,
                &mut ControlRequestPool::new(),
                &[0x08, 0x00]
            ),
            EscRegisterRequestProgress::WriteComplete(write)
        );
        assert_eq!(controller.data(write).unwrap(), &[0x08, 0x00]);
        assert!(!controller.has_pending());
        assert_eq!(controller.len(), 2);

        controller.release(read).unwrap();
        controller.release(write).unwrap();
        assert!(controller.is_empty());
        assert_eq!(
            controller.status(read),
            Err(EscRegisterRequestError::InvalidHandle)
        );
    }

    #[test]
    fn validates_capacity_shape_range_deadline_and_release_state() {
        let mut zero = EscRegisterRequestController::<0>::new(1);
        assert_eq!(
            zero.submit_read(1, 0, 1, 0, 1),
            Err(EscRegisterRequestError::InvalidCapacity)
        );

        let mut controller = EscRegisterRequestController::<1>::new(1);
        assert_eq!(
            controller.submit_read(0, 0, 1, 0, 1),
            Err(EscRegisterRequestError::InvalidStationAddress)
        );
        assert_eq!(
            controller.submit_read(1, 0, 0, 0, 1),
            Err(EscRegisterRequestError::InvalidLength)
        );
        assert_eq!(
            controller.submit_read(1, u16::MAX, 2, 0, 1),
            Err(EscRegisterRequestError::RegisterRangeOverflow)
        );
        assert_eq!(
            controller.submit_read(1, 0, 1, 0, 0),
            Err(EscRegisterRequestError::InvalidTimeout)
        );
        assert_eq!(
            controller.submit_read(1, 0, 1, u64::MAX, 1),
            Err(EscRegisterRequestError::InvalidTimeout)
        );

        let handle = controller.submit_read(1, 0, 1, 0, 1).unwrap();
        assert_eq!(
            controller.release(handle),
            Err(EscRegisterRequestError::InvalidState)
        );
        assert_eq!(
            controller.submit_write(1, 1, &[1], 0, 1),
            Err(EscRegisterRequestError::CapacityExceeded)
        );
    }

    #[test]
    fn timeout_and_action_mismatch_are_request_local_and_do_not_leak_pool_slots() {
        let mut controller = EscRegisterRequestController::<2>::new(42);
        let expired = controller.submit_read(1, 0, 1, 0, 5).unwrap();
        assert_eq!(
            controller.next_action(5),
            Err(EscRegisterRequestError::Timeout(expired))
        );
        assert_eq!(
            controller.status(expired).unwrap().state,
            EscRegisterRequestState::Error
        );

        let next = controller.submit_read(1, 1, 1, 5, 10).unwrap();
        let action = controller.next_action(6).unwrap().unwrap();
        let mut pool = ControlRequestPool::<1>::new();
        let pool_handle = controller.enqueue_pending(&mut pool).unwrap();
        pool.get_mut(pool_handle).unwrap().deadline_ns += 1;
        pool.get_mut(pool_handle).unwrap().state = RequestState::Complete;
        assert_eq!(
            controller.accept_completed(&mut pool, pool_handle, 7),
            Err(EscRegisterRequestError::ActionMismatch(next))
        );
        assert_eq!(pool.in_use(), 0);
        assert_eq!(controller.pending(), None);
        assert_eq!(
            controller.status(next).unwrap().state,
            EscRegisterRequestState::Error
        );
        assert_eq!(action.handle, next);
    }

    #[test]
    fn occupied_control_pool_fails_only_the_register_request() {
        let mut controller = EscRegisterRequestController::<1>::new(42);
        let request = controller.submit_read(1, 0, 1, 0, 10).unwrap();
        controller.next_action(1).unwrap().unwrap();
        let mut pool = ControlRequestPool::<1>::new();
        let occupied = pool
            .acquire(7, 1, fixed_address(2, 0), RegisterOperation::Read, &[0], 10)
            .unwrap();

        assert_eq!(
            controller.enqueue_pending(&mut pool),
            Err(EscRegisterRequestError::Control {
                request,
                error: ControlError::SlotBusy,
            })
        );
        assert_eq!(pool.in_use(), 1);
        assert_eq!(controller.pending(), None);
        assert_eq!(controller.queued_len(), 0);
        let status = controller.status(request).unwrap();
        assert_eq!(status.state, EscRegisterRequestState::Error);
        assert_eq!(
            status.error,
            Some(EscRegisterRequestError::Control {
                request,
                error: ControlError::SlotBusy,
            })
        );
        pool.release(occupied).unwrap();
    }

    #[test]
    fn rebuilt_and_completed_actions_still_obey_the_absolute_deadline() {
        let mut rebuilt = EscRegisterRequestController::<1>::new(42);
        let rebuilt_handle = rebuilt.submit_read(1, 0, 1, 0, 10).unwrap();
        rebuilt.next_action(1).unwrap().unwrap();
        let mut rebuilt_pool = ControlRequestPool::<1>::new();
        let prepared = rebuilt.enqueue_pending(&mut rebuilt_pool).unwrap();
        rebuilt_pool.release(prepared).unwrap();
        assert_eq!(
            rebuilt.next_action(10),
            Err(EscRegisterRequestError::Timeout(rebuilt_handle))
        );
        assert_eq!(
            rebuilt.status(rebuilt_handle).unwrap().state,
            EscRegisterRequestState::Error
        );

        let mut completed = EscRegisterRequestController::<1>::new(42);
        let completed_handle = completed.submit_read(1, 0, 1, 0, 10).unwrap();
        let action = completed.next_action(1).unwrap().unwrap();
        let mut completed_pool = ControlRequestPool::<1>::new();
        let in_flight = completed.enqueue_pending(&mut completed_pool).unwrap();
        let mut frame = [0; 256];
        completed_pool
            .get_mut(in_flight)
            .unwrap()
            .build_frame(&mut frame, [0; 6], [1; 6])
            .unwrap();
        completed_pool
            .complete(in_flight, action.generation, action.address, &[1], 1)
            .unwrap();
        assert_eq!(
            completed.accept_completed(&mut completed_pool, in_flight, 11),
            Err(EscRegisterRequestError::Timeout(completed_handle))
        );
        assert_eq!(completed_pool.in_use(), 0);
        assert_eq!(
            completed.status(completed_handle).unwrap().state,
            EscRegisterRequestState::Error
        );
    }

    #[test]
    fn failed_working_counter_is_retained_with_the_request_error() {
        let mut controller = EscRegisterRequestController::<1>::new(42);
        let request = controller.submit_read(1, 0, 1, 0, 10).unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        let mut pool = ControlRequestPool::<1>::new();
        let handle = controller.enqueue_pending(&mut pool).unwrap();
        let mut frame = [0; 256];
        pool.get_mut(handle)
            .unwrap()
            .build_frame(&mut frame, [0; 6], [1; 6])
            .unwrap();
        assert_eq!(
            pool.complete(handle, action.generation, action.address, &[1], 2),
            Err(ControlError::WorkingCounterMismatch)
        );
        assert_eq!(
            controller.accept_completed(&mut pool, handle, 2),
            Err(EscRegisterRequestError::Control {
                request,
                error: ControlError::WorkingCounterMismatch,
            })
        );
        let status = controller.status(request).unwrap();
        assert_eq!(status.state, EscRegisterRequestState::Error);
        assert_eq!(status.actual_wkc, 2);
    }
}
