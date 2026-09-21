use crate::rx_index::{RxExpectation, RxIndexError};
use crate::wire::{Command, FrameBuilder, WireError};

pub const MAX_CONTROL_PAYLOAD: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestHandle(u8);

impl RequestHandle {
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    pub const fn from_index(index: usize) -> Option<Self> {
        if index < 64 {
            Some(Self(index as u8))
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RequestState {
    Free = 0,
    Prepared = 1,
    InFlight = 2,
    Complete = 3,
    Failed = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegisterOperation {
    Read,
    Write,
    AutoIncrementRead,
    AutoIncrementWrite,
}

impl RegisterOperation {
    const fn command(self) -> Command {
        match self {
            Self::Read => Command::Fprd,
            Self::Write => Command::Fpwr,
            Self::AutoIncrementRead => Command::Aprd,
            Self::AutoIncrementWrite => Command::Apwr,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlRequest {
    pub datagram_index: u8,
    pub generation: u16,
    pub address: u32,
    pub operation: RegisterOperation,
    pub length: usize,
    pub response_length: usize,
    pub deadline_ns: u64,
    pub state: RequestState,
    pub actual_wkc: u16,
    last_error: Option<ControlError>,
    payload: [u8; MAX_CONTROL_PAYLOAD],
}

impl ControlRequest {
    const EMPTY: Self = Self {
        datagram_index: 0,
        generation: 0,
        address: 0,
        operation: RegisterOperation::Read,
        length: 0,
        response_length: 0,
        deadline_ns: 0,
        state: RequestState::Free,
        actual_wkc: 0,
        last_error: None,
        payload: [0; MAX_CONTROL_PAYLOAD],
    };

    pub fn payload(&self) -> &[u8] {
        &self.payload[..self.length]
    }

    pub fn payload_mut(&mut self) -> &mut [u8] {
        &mut self.payload[..self.length]
    }

    pub const fn last_error(&self) -> Option<ControlError> {
        self.last_error
    }

    pub const fn expectation(&self) -> RxExpectation {
        RxExpectation {
            generation: self.generation,
            deadline_ns: self.deadline_ns,
            expected_address: self.address,
            expected_size: self.response_length as u16,
            expected_type: self.operation.command() as u8,
            expected_wkc: 1,
        }
    }

    pub fn build_frame(
        &mut self,
        buffer: &mut [u8],
        destination: [u8; 6],
        source: [u8; 6],
    ) -> Result<usize, ControlError> {
        if self.state != RequestState::Prepared {
            return Err(ControlError::InvalidState);
        }
        let mut builder =
            FrameBuilder::new(buffer, destination, source).map_err(ControlError::Wire)?;
        builder
            .push(
                self.operation.command(),
                self.datagram_index,
                self.address,
                self.payload(),
            )
            .map_err(ControlError::Wire)?;
        let length = builder.finish().map_err(ControlError::Wire)?;
        self.state = RequestState::InFlight;
        Ok(length)
    }

    pub fn complete(
        &mut self,
        generation: u16,
        address: u32,
        payload: &[u8],
        working_counter: u16,
    ) -> Result<(), ControlError> {
        if self.state != RequestState::InFlight {
            if matches!(self.state, RequestState::Complete | RequestState::Failed) {
                return Err(ControlError::InvalidState);
            }
            return self.fail(ControlError::InvalidState);
        }
        if self.generation != generation {
            return self.fail(ControlError::GenerationMismatch);
        }
        if self.address != address {
            return self.fail(ControlError::AddressMismatch);
        }
        if self.response_length != payload.len() {
            return self.fail(ControlError::LengthMismatch);
        }
        self.payload[..payload.len()].copy_from_slice(payload);
        self.length = self.response_length;
        self.actual_wkc = working_counter;
        if working_counter != 1 {
            return self.fail(ControlError::WorkingCounterMismatch);
        }
        self.state = RequestState::Complete;
        self.last_error = None;
        Ok(())
    }

    fn fail<T>(&mut self, error: ControlError) -> Result<T, ControlError> {
        self.state = RequestState::Failed;
        self.last_error = Some(error);
        Err(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlError {
    TooManyRequests,
    SlotBusy,
    InvalidHandle,
    InvalidState,
    PayloadTooLarge,
    ResponseTooLarge,
    Wire(WireError),
    GenerationMismatch,
    DatagramIndexMismatch,
    TypeMismatch,
    AddressMismatch,
    LengthMismatch,
    WorkingCounterMismatch,
    Timeout,
    RxIndex(RxIndexError),
}

/// Handles that became overdue in one bounded control-request sweep. Failed
/// requests remain owned by the caller until their service FSM consumes them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlExpiry(u64);

impl ControlExpiry {
    pub const fn count(self) -> usize {
        self.0.count_ones() as usize
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, handle: RequestHandle) -> bool {
        self.0 & (1u64 << handle.index()) != 0
    }

    pub const fn handles(self) -> ControlExpiryHandles {
        ControlExpiryHandles(self.0)
    }
}

pub struct ControlExpiryHandles(u64);

impl Iterator for ControlExpiryHandles {
    type Item = RequestHandle;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0 == 0 {
            return None;
        }
        let index = self.0.trailing_zeros() as usize;
        self.0 &= !(1u64 << index);
        RequestHandle::from_index(index)
    }
}

pub struct ControlRequestPool<const REQUESTS: usize> {
    requests: [ControlRequest; REQUESTS],
    used: u64,
}

impl<const REQUESTS: usize> ControlRequestPool<REQUESTS> {
    pub const fn new() -> Self {
        Self {
            requests: [ControlRequest::EMPTY; REQUESTS],
            used: 0,
        }
    }

    pub fn acquire(
        &mut self,
        datagram_index: u8,
        generation: u16,
        address: u32,
        operation: RegisterOperation,
        payload: &[u8],
        deadline_ns: u64,
    ) -> Result<RequestHandle, ControlError> {
        self.acquire_with_response_len(
            datagram_index,
            generation,
            address,
            operation,
            payload,
            payload.len(),
            deadline_ns,
        )
    }

    // Keep the transaction fields explicit: this is the control-plane
    // boundary where address, command, wire length and deadline are audited.
    #[allow(clippy::too_many_arguments)]
    pub fn acquire_with_response_len(
        &mut self,
        datagram_index: u8,
        generation: u16,
        address: u32,
        operation: RegisterOperation,
        payload: &[u8],
        response_length: usize,
        deadline_ns: u64,
    ) -> Result<RequestHandle, ControlError> {
        if REQUESTS == 0 || REQUESTS > 64 {
            return Err(ControlError::TooManyRequests);
        }
        if payload.len() > MAX_CONTROL_PAYLOAD {
            return Err(ControlError::PayloadTooLarge);
        }
        let datagram_length = payload.len().max(response_length);
        if datagram_length > MAX_CONTROL_PAYLOAD {
            return Err(ControlError::ResponseTooLarge);
        }
        let mask = if REQUESTS == 64 {
            u64::MAX
        } else {
            (1u64 << REQUESTS) - 1
        };
        let available = (!self.used) & mask;
        if available == 0 {
            return Err(ControlError::SlotBusy);
        }
        let index = available.trailing_zeros() as usize;
        let handle = RequestHandle(index as u8);
        self.used |= 1u64 << index;
        let request = &mut self.requests[index];
        request.state = RequestState::Prepared;
        request.datagram_index = datagram_index;
        request.generation = generation;
        request.address = address;
        request.operation = operation;
        request.length = datagram_length;
        request.response_length = datagram_length;
        request.deadline_ns = deadline_ns;
        request.actual_wkc = 0;
        request.last_error = None;
        request.payload.fill(0);
        request.payload[..payload.len()].copy_from_slice(payload);
        Ok(handle)
    }

    pub fn get(&self, handle: RequestHandle) -> Option<&ControlRequest> {
        let index = handle.index();
        if index < REQUESTS && self.used & (1u64 << index) != 0 {
            Some(&self.requests[index])
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, handle: RequestHandle) -> Option<&mut ControlRequest> {
        let index = handle.index();
        if index < REQUESTS && self.used & (1u64 << index) != 0 {
            Some(&mut self.requests[index])
        } else {
            None
        }
    }

    pub fn expectation(&self, handle: RequestHandle) -> Result<RxExpectation, ControlError> {
        self.get(handle)
            .map(ControlRequest::expectation)
            .ok_or(ControlError::InvalidHandle)
    }

    pub fn complete(
        &mut self,
        handle: RequestHandle,
        generation: u16,
        address: u32,
        payload: &[u8],
        working_counter: u16,
    ) -> Result<(), ControlError> {
        self.get_mut(handle)
            .ok_or(ControlError::InvalidHandle)?
            .complete(generation, address, payload, working_counter)
    }

    pub fn complete_match(
        &mut self,
        completion: crate::rx_index::RxMatch,
        header: crate::wire::DatagramHeader,
        payload: &[u8],
    ) -> Result<(), ControlError> {
        let handle = RequestHandle::from_index(completion.slot_id as usize)
            .ok_or(ControlError::InvalidHandle)?;
        let request = self.get(handle).ok_or(ControlError::InvalidHandle)?;
        if request.datagram_index != header.index {
            return Err(ControlError::DatagramIndexMismatch);
        }
        if request.operation.command() != header.command {
            return Err(ControlError::TypeMismatch);
        }
        self.complete(
            handle,
            completion.generation,
            header.address,
            payload,
            completion.working_counter,
        )
    }

    pub fn release(&mut self, handle: RequestHandle) -> Result<(), ControlError> {
        let index = handle.index();
        if index >= REQUESTS || self.used & (1u64 << index) == 0 {
            return Err(ControlError::InvalidHandle);
        }
        self.used &= !(1u64 << index);
        self.requests[index] = ControlRequest::EMPTY;
        Ok(())
    }

    pub const fn in_use(&self) -> usize {
        self.used.count_ones() as usize
    }

    /// Finalize only missing in-flight responses after the RX owner has
    /// finished polling. A response received at the deadline is still valid;
    /// expiry uses the same strict boundary as `RxIndexTable::expire_armed`.
    /// Call the matching service FSM for each returned handle, then release it.
    pub fn expire_in_flight(&mut self, now_ns: u64) -> ControlExpiry {
        let mut expired = 0u64;
        let mut remaining = self.used;
        while remaining != 0 {
            let index = remaining.trailing_zeros() as usize;
            let bit = 1u64 << index;
            remaining &= !bit;
            let request = &mut self.requests[index];
            if request.state == RequestState::InFlight && now_ns > request.deadline_ns {
                request.state = RequestState::Failed;
                request.last_error = Some(ControlError::Timeout);
                expired |= bit;
            }
        }
        ControlExpiry(expired)
    }

    pub fn build_into_buffer(
        &mut self,
        handle: RequestHandle,
        frame_buffer: &mut [u8],
        destination: [u8; 6],
        source: [u8; 6],
    ) -> Result<usize, ControlError> {
        let request = self.get_mut(handle).ok_or(ControlError::InvalidHandle)?;
        request.build_frame(frame_buffer, destination, source)
    }
}

pub struct ControlRxConsumer<'a, const REQUESTS: usize> {
    pool: &'a mut ControlRequestPool<REQUESTS>,
    rejected: usize,
}

impl<'a, const REQUESTS: usize> ControlRxConsumer<'a, REQUESTS> {
    pub fn new(pool: &'a mut ControlRequestPool<REQUESTS>) -> Self {
        Self { pool, rejected: 0 }
    }

    pub const fn rejected(&self) -> usize {
        self.rejected
    }

    pub fn pool(&mut self) -> &mut ControlRequestPool<REQUESTS> {
        self.pool
    }
}

impl<const REQUESTS: usize> crate::engine::RxDatagramConsumer for ControlRxConsumer<'_, REQUESTS> {
    fn accept(
        &mut self,
        _: u64,
        _: u64,
        completion: crate::rx_index::RxMatch,
        header: crate::wire::DatagramHeader,
        payload: &[u8],
    ) -> bool {
        let Some(handle) = RequestHandle::from_index(completion.slot_id as usize) else {
            return false;
        };
        if !self.pool.get(handle).is_some_and(|request| {
            request.state == RequestState::InFlight && request.datagram_index == header.index
        }) {
            return false;
        }
        if self
            .pool
            .complete_match(completion, header, payload)
            .is_ok()
        {
            true
        } else {
            self.rejected += 1;
            false
        }
    }
}

impl<const REQUESTS: usize> Default for ControlRequestPool<REQUESTS> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::RxDatagramConsumer;
    use crate::rx_index::RxMatch;
    use crate::wire::{DatagramHeader, FrameView, MAX_ETHERNET_FRAME_LEN};

    #[test]
    fn fixed_request_pool_builds_and_completes_register_read() {
        let mut pool = ControlRequestPool::<2>::new();
        let handle = pool
            .acquire(4, 9, 0x1000, RegisterOperation::Read, &[0; 4], 10_000)
            .unwrap();
        let expectation = pool.expectation(handle).unwrap();
        assert_eq!(expectation.expected_address, 0x1000);
        let mut frame = [0; MAX_ETHERNET_FRAME_LEN];
        let length = pool
            .get_mut(handle)
            .unwrap()
            .build_frame(&mut frame, [0xFF; 6], [1, 2, 3, 4, 5, 6])
            .unwrap();
        let view = FrameView::parse(&frame[..length]).unwrap();
        let datagram = view.datagrams().next().unwrap().unwrap();
        assert_eq!(datagram.header.command, Command::Fprd);
        pool.complete(handle, 9, 0x1000, &[1, 2, 3, 4], 1).unwrap();
        assert_eq!(pool.get(handle).unwrap().state, RequestState::Complete);
        assert_eq!(pool.get(handle).unwrap().payload(), &[1, 2, 3, 4]);
        pool.release(handle).unwrap();
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn read_request_zero_fills_tx_data_and_keeps_full_rx_payload() {
        let mut pool = ControlRequestPool::<1>::new();
        let handle = pool
            .acquire_with_response_len(4, 9, 0x1000, RegisterOperation::Read, &[], 4, 10_000)
            .unwrap();
        let request = pool.get(handle).unwrap();
        assert_eq!(request.length, 4);
        assert_eq!(request.response_length, 4);
        assert_eq!(request.payload(), &[0, 0, 0, 0]);

        let mut frame = [0; MAX_ETHERNET_FRAME_LEN];
        let length = pool
            .get_mut(handle)
            .unwrap()
            .build_frame(&mut frame, [0xFF; 6], [1, 2, 3, 4, 5, 6])
            .unwrap();
        let view = FrameView::parse(&frame[..length]).unwrap();
        let datagram = view.datagrams().next().unwrap().unwrap();
        assert_eq!(datagram.payload, &[0, 0, 0, 0]);

        pool.complete(handle, 9, 0x1000, &[1, 2, 3, 4], 1).unwrap();
        assert_eq!(pool.get(handle).unwrap().payload(), &[1, 2, 3, 4]);
    }

    #[test]
    fn request_pool_rejects_bad_completion_without_publishing_data() {
        let mut pool = ControlRequestPool::<1>::new();
        let handle = pool
            .acquire(1, 1, 0x20, RegisterOperation::Write, &[7, 8], 10)
            .unwrap();
        let mut frame = [0; MAX_ETHERNET_FRAME_LEN];
        pool.get_mut(handle)
            .unwrap()
            .build_frame(&mut frame, [0; 6], [0; 6])
            .unwrap();
        assert_eq!(
            pool.complete(handle, 1, 0x20, &[9, 9], 0),
            Err(ControlError::WorkingCounterMismatch)
        );
        assert_eq!(pool.get(handle).unwrap().state, RequestState::Failed);
        assert_eq!(
            pool.get(handle).unwrap().last_error(),
            Some(ControlError::WorkingCounterMismatch)
        );
    }

    #[test]
    fn unrelated_datagram_cannot_complete_a_control_request_with_the_same_slot_id() {
        let mut pool = ControlRequestPool::<1>::new();
        let handle = pool
            .acquire(15, 2, 0x5000, RegisterOperation::Read, &[0; 4], 250_000)
            .unwrap();
        let mut frame = [0; MAX_ETHERNET_FRAME_LEN];
        pool.get_mut(handle)
            .unwrap()
            .build_frame(&mut frame, [0xFF; 6], [1, 2, 3, 4, 5, 6])
            .unwrap();
        let completion = RxMatch {
            slot_id: handle.index() as u16,
            generation: 2,
            working_counter: 1,
        };
        let other_index = DatagramHeader::new(Command::Fprd, 12, 0x5000, 4);
        let other_command = DatagramHeader::new(Command::Fpwr, 15, 0x5000, 4);
        assert_eq!(
            pool.complete_match(completion, other_index, &[1, 2, 3, 4]),
            Err(ControlError::DatagramIndexMismatch)
        );
        assert_eq!(
            pool.complete_match(completion, other_command, &[1, 2, 3, 4]),
            Err(ControlError::TypeMismatch)
        );
        let mut consumer = ControlRxConsumer::new(&mut pool);
        assert!(!consumer.accept(1, 200_000, completion, other_index, &[1, 2, 3, 4]));
        assert_eq!(consumer.rejected(), 0);
        assert!(!consumer.accept(1, 200_000, completion, other_command, &[1, 2, 3, 4]));
        assert_eq!(consumer.rejected(), 1);
        assert_eq!(pool.get(handle).unwrap().state, RequestState::InFlight);
        assert_eq!(pool.get(handle).unwrap().payload(), &[0; 4]);
        let expected = DatagramHeader::new(Command::Fprd, 15, 0x5000, 4);
        assert!(
            pool.complete_match(completion, expected, &[1, 2, 3, 4])
                .is_ok()
        );
        assert_eq!(pool.get(handle).unwrap().state, RequestState::Complete);
        let mut consumer = ControlRxConsumer::new(&mut pool);
        assert!(!consumer.accept(2, 210_000, completion, expected, &[9, 9, 9, 9]));
        assert_eq!(consumer.rejected(), 0);
        assert_eq!(pool.get(handle).unwrap().state, RequestState::Complete);
        assert_eq!(pool.get(handle).unwrap().payload(), &[1, 2, 3, 4]);
    }

    #[test]
    fn expiry_is_bounded_once_and_preserves_terminal_requests_and_payloads() {
        let mut pool = ControlRequestPool::<64>::new();
        let prepared = pool
            .acquire(1, 7, 0x1000, RegisterOperation::Read, &[0; 2], 10)
            .unwrap();
        let overdue = pool
            .acquire(2, 7, 0x1001, RegisterOperation::Read, &[0; 2], 10)
            .unwrap();
        let on_boundary = pool
            .acquire(3, 7, 0x1002, RegisterOperation::Read, &[0; 2], 11)
            .unwrap();
        let completed = pool
            .acquire(4, 7, 0x1003, RegisterOperation::Read, &[0; 2], 10)
            .unwrap();
        let mut frame = [0; MAX_ETHERNET_FRAME_LEN];
        for handle in [overdue, on_boundary, completed] {
            pool.build_into_buffer(handle, &mut frame, [0; 6], [0; 6])
                .unwrap();
        }
        pool.complete(completed, 7, 0x1003, &[8, 9], 1).unwrap();

        assert!(pool.expire_in_flight(10).is_empty());
        let expired = pool.expire_in_flight(11);
        assert_eq!(expired.count(), 1);
        assert_eq!(expired.handles().collect::<std::vec::Vec<_>>(), [overdue]);
        assert!(expired.contains(overdue));
        assert!(!expired.contains(on_boundary));
        assert_eq!(pool.get(prepared).unwrap().state, RequestState::Prepared);
        assert_eq!(pool.get(overdue).unwrap().state, RequestState::Failed);
        assert_eq!(
            pool.get(overdue).unwrap().last_error(),
            Some(ControlError::Timeout)
        );
        assert_eq!(pool.get(overdue).unwrap().payload(), &[0, 0]);
        assert_eq!(pool.get(completed).unwrap().payload(), &[8, 9]);
        assert!(pool.expire_in_flight(11).is_empty());
        assert_eq!(
            pool.expire_in_flight(12).handles().next(),
            Some(on_boundary)
        );
        assert!(pool.expire_in_flight(20).is_empty());
        assert_eq!(pool.in_use(), 4);

        let completion = RxMatch {
            slot_id: overdue.index() as u16,
            generation: 7,
            working_counter: 1,
        };
        let header = DatagramHeader::new(Command::Fprd, 2, 0x1001, 2);
        let mut consumer = ControlRxConsumer::new(&mut pool);
        assert!(!consumer.accept(2, 20, completion, header, &[1, 2]));
        assert_eq!(consumer.rejected(), 0);
        assert_eq!(
            pool.complete_match(completion, header, &[1, 2]),
            Err(ControlError::InvalidState)
        );
        assert_eq!(
            pool.get(overdue).unwrap().last_error(),
            Some(ControlError::Timeout)
        );
        assert_eq!(pool.get(overdue).unwrap().payload(), &[0, 0]);
        assert_eq!(
            pool.complete(completed, 7, 0x1003, &[1, 2], 1),
            Err(ControlError::InvalidState)
        );
        assert_eq!(pool.get(completed).unwrap().payload(), &[8, 9]);
    }

    #[test]
    fn expiry_reports_last_pool_slot_without_allocating() {
        let mut pool = ControlRequestPool::<64>::new();
        let mut last = None;
        for index in 0..64 {
            last = Some(
                pool.acquire(index, 1, 0x1000, RegisterOperation::Read, &[], 1)
                    .unwrap(),
            );
        }
        let last = last.unwrap();
        pool.build_into_buffer(last, &mut [0; MAX_ETHERNET_FRAME_LEN], [0; 6], [0; 6])
            .unwrap();
        assert_eq!(last.index(), 63);
        assert_eq!(pool.expire_in_flight(2).handles().next(), Some(last));
    }
}
