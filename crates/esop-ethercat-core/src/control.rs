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
    AddressMismatch,
    LengthMismatch,
    WorkingCounterMismatch,
    RxIndex(RxIndexError),
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
    use crate::wire::{FrameView, MAX_ETHERNET_FRAME_LEN};

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
}
