use crate::control::{ControlError, ControlRequestPool, RequestHandle};
use crate::diag::{Diagnostics, EventCode, EventRecord, EventSeverity};
use crate::dma::{DmaCacheOps, DmaDescriptorRing, DmaRingError, DmaTxHandle};
use crate::frame_pool::{FrameHandle, FramePool, FramePoolError};
use crate::plan::{FramePlan, PlanError};
use crate::port::{EthercatDmaTxPort, EthercatPort, LinkState, RxPoll};
use crate::rx_index::{
    RxExpectation, RxIndexError, RxIndexTable, RxMatch, RxResponse, RxSlotState,
};
use crate::wire::{DatagramHeader, FrameBuilder, FrameView, MAX_ETHERNET_FRAME_LEN, WireError};

pub trait RxDatagramConsumer {
    /// Returns false when a verified RX datagram cannot be consumed by the
    /// caller's staging model. The master records this separately from wire
    /// corruption because the frame itself was valid. `cycle` and
    /// `received_at_ns` are captured by the master at the RX boundary so
    /// consumers do not substitute wall-clock or logging timestamps.
    fn accept(
        &mut self,
        cycle: u64,
        received_at_ns: u64,
        completion: RxMatch,
        header: DatagramHeader,
        payload: &[u8],
    ) -> bool;
}

impl RxDatagramConsumer for () {
    fn accept(&mut self, _: u64, _: u64, _: RxMatch, _: DatagramHeader, _: &[u8]) -> bool {
        true
    }
}

/// Route verified datagrams to the first consumer that accepts them.
///
/// Consumers must own disjoint datagram-index sets. This is intentionally a
/// static pair instead of a dynamic handler list so cyclic RX dispatch stays
/// allocation-free and has a fixed upper bound.
pub struct RxConsumerMux<FIRST, SECOND> {
    first: FIRST,
    second: SECOND,
}

impl<FIRST, SECOND> RxConsumerMux<FIRST, SECOND> {
    pub const fn new(first: FIRST, second: SECOND) -> Self {
        Self { first, second }
    }

    pub fn first(&self) -> &FIRST {
        &self.first
    }

    pub fn first_mut(&mut self) -> &mut FIRST {
        &mut self.first
    }

    pub fn second(&self) -> &SECOND {
        &self.second
    }

    pub fn second_mut(&mut self) -> &mut SECOND {
        &mut self.second
    }

    pub fn into_inner(self) -> (FIRST, SECOND) {
        (self.first, self.second)
    }
}

impl<FIRST: RxDatagramConsumer, SECOND: RxDatagramConsumer> RxDatagramConsumer
    for RxConsumerMux<FIRST, SECOND>
{
    fn accept(
        &mut self,
        cycle: u64,
        received_at_ns: u64,
        completion: RxMatch,
        header: DatagramHeader,
        payload: &[u8],
    ) -> bool {
        self.first
            .accept(cycle, received_at_ns, completion, header, payload)
            || self
                .second
                .accept(cycle, received_at_ns, completion, header, payload)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MasterConfig {
    pub destination_mac: [u8; 6],
    pub source_mac: [u8; 6],
    pub rx_budget_frames: usize,
    pub rx_budget_bytes: usize,
    pub rx_budget_ns: u64,
}

impl MasterConfig {
    pub const fn new(destination_mac: [u8; 6], source_mac: [u8; 6]) -> Self {
        Self {
            destination_mac,
            source_mac,
            rx_budget_frames: 8,
            rx_budget_bytes: 8 * MAX_ETHERNET_FRAME_LEN,
            rx_budget_ns: 50_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CycleReport {
    pub cycle: u64,
    pub received_frames: usize,
    pub received_bytes: usize,
    pub parsed_datagrams: usize,
    pub unmatched_datagrams: usize,
    pub corrupt_frames: usize,
    pub wkc_mismatches: usize,
    pub timed_out_datagrams: usize,
    pub consumer_rejections: usize,
    pub budget_exhausted: bool,
    pub link_down: bool,
}

impl CycleReport {
    const fn new(cycle: u64) -> Self {
        Self {
            cycle,
            received_frames: 0,
            received_bytes: 0,
            parsed_datagrams: 0,
            unmatched_datagrams: 0,
            corrupt_frames: 0,
            wkc_mismatches: 0,
            timed_out_datagrams: 0,
            consumer_rejections: 0,
            budget_exhausted: false,
            link_down: false,
        }
    }
}

/// Borrowed receive session for a platform DMA RX ring.
///
/// The platform polls and completes descriptors itself, then passes each
/// completed frame to [`Self::consume_frame`]. The frame is parsed directly
/// from the descriptor buffer and is not copied into a core-owned scratch
/// array. `finish` closes the bounded cycle and resets completed RX indices.
pub struct DmaReceiveCycle<'master, const SLOTS: usize, const MTU: usize> {
    master: &'master mut EthercatMaster<SLOTS, MTU>,
    report: CycleReport,
    generation: u16,
    deadline_ns: u64,
}

impl<'master, const SLOTS: usize, const MTU: usize> DmaReceiveCycle<'master, SLOTS, MTU> {
    pub const fn cycle(&self) -> u64 {
        self.report.cycle
    }

    pub const fn deadline_ns(&self) -> u64 {
        self.deadline_ns
    }

    pub const fn report(&self) -> CycleReport {
        self.report
    }

    /// Return whether another DMA RX descriptor may be consumed under the
    /// configured frame, byte, and time budgets.
    pub fn can_consume(&self, now_ns: u64) -> bool {
        self.report.received_frames < self.master.config.rx_budget_frames
            && self.report.received_bytes < self.master.config.rx_budget_bytes
            && now_ns < self.deadline_ns
    }

    /// Parse and validate one descriptor-backed frame in place.
    pub fn consume_frame<C: RxDatagramConsumer>(
        &mut self,
        frame: &[u8],
        received_at_ns: u64,
        consumer: &mut C,
    ) {
        if !self.can_consume(received_at_ns) {
            self.report.budget_exhausted = true;
            return;
        }
        if frame.is_empty() || frame.len() > MAX_ETHERNET_FRAME_LEN {
            self.master.record_corrupt_frame(
                self.report.cycle,
                received_at_ns,
                frame.len(),
                MAX_ETHERNET_FRAME_LEN,
            );
            self.report.corrupt_frames += 1;
            return;
        }
        self.report.received_frames += 1;
        self.report.received_bytes = self.report.received_bytes.saturating_add(frame.len());
        self.master.process_received_frame(
            frame,
            self.generation,
            self.report.cycle,
            received_at_ns,
            &mut self.report,
            consumer,
        );
    }

    /// Close the receive cycle, emit budget/timeout diagnostics, and release
    /// completed/rejected RX index entries for the next generation.
    pub fn finish(mut self, now_ns: u64) -> CycleReport {
        self.master.expire_rx_entries(now_ns, &mut self.report);
        self.master
            .finish_receive_report(&mut self.report, now_ns, self.deadline_ns);
        self.master.rx_index.reset_complete();
        self.report
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CycleError<E> {
    Port(E),
    Control(ControlError),
    Wire(WireError),
    FramePool(FramePoolError),
    Plan(PlanError),
    RxIndex(RxIndexError),
    Dma(DmaRingError),
    InvalidFrameLength,
}

pub struct EthercatMaster<const SLOTS: usize, const MTU: usize> {
    config: MasterConfig,
    cycle: u64,
    frames: FramePool<SLOTS, MTU>,
    rx_index: RxIndexTable,
    diagnostics: Diagnostics,
}

impl<const SLOTS: usize, const MTU: usize> EthercatMaster<SLOTS, MTU> {
    pub const fn new(config: MasterConfig) -> Self {
        Self {
            config,
            cycle: 0,
            frames: FramePool::new(),
            rx_index: RxIndexTable::new(),
            diagnostics: Diagnostics::new(),
        }
    }

    pub const fn config(&self) -> MasterConfig {
        self.config
    }

    pub fn acquire_frame(
        &mut self,
        generation: u16,
        deadline_ns: u64,
    ) -> Result<FrameHandle, FramePoolError> {
        let sequence = self.cycle.wrapping_add(1);
        self.frames.acquire(sequence, generation, deadline_ns)
    }

    pub fn frame_slot_mut(
        &mut self,
        handle: FrameHandle,
    ) -> Option<&mut crate::frame_pool::FrameSlot<MTU>> {
        self.frames.slot_mut(handle)
    }

    pub fn frame_builder(
        &mut self,
        handle: FrameHandle,
    ) -> Result<FrameBuilder<'_>, CycleError<core::convert::Infallible>> {
        let slot = self
            .frames
            .slot_mut(handle)
            .ok_or(CycleError::FramePool(FramePoolError::InvalidHandle))?;
        FrameBuilder::new(
            &mut slot.bytes,
            self.config.destination_mac,
            self.config.source_mac,
        )
        .map_err(CycleError::Wire)
    }

    pub fn finish_frame(
        &mut self,
        handle: FrameHandle,
        length: usize,
    ) -> Result<(), CycleError<core::convert::Infallible>> {
        let slot = self
            .frames
            .slot_mut(handle)
            .ok_or(CycleError::FramePool(FramePoolError::InvalidHandle))?;
        if length > MTU {
            return Err(CycleError::InvalidFrameLength);
        }
        slot.len = length;
        Ok(())
    }

    pub fn build_frame_from_plan<const DATAGRAMS: usize>(
        &mut self,
        handle: FrameHandle,
        plan: &FramePlan<DATAGRAMS>,
        process_image: &[u8],
    ) -> Result<usize, CycleError<core::convert::Infallible>> {
        let slot = self
            .frames
            .slot_mut(handle)
            .ok_or(CycleError::FramePool(FramePoolError::InvalidHandle))?;
        let length = plan
            .build(
                &mut slot.bytes,
                self.config.destination_mac,
                self.config.source_mac,
                process_image,
            )
            .map_err(CycleError::Plan)?;
        slot.len = length;
        Ok(length)
    }

    /// Build a precomputed cyclic frame and arm every planned datagram for
    /// this frame generation. Keeping frame construction and RX registration
    /// together prevents a valid TX plan from being submitted without a
    /// matching completion expectation.
    pub fn build_and_arm_frame_from_plan<const DATAGRAMS: usize>(
        &mut self,
        handle: FrameHandle,
        plan: &FramePlan<DATAGRAMS>,
        process_image: &[u8],
    ) -> Result<usize, CycleError<core::convert::Infallible>> {
        for datagram in plan.datagrams() {
            if self.rx_index.entry(datagram.index).state == RxSlotState::Armed {
                return Err(CycleError::RxIndex(RxIndexError::AlreadyArmed));
            }
        }

        let length = self.build_frame_from_plan(handle, plan, process_image)?;
        let (generation, deadline_ns) = self
            .frames
            .slot(handle)
            .map(|slot| (slot.generation, slot.deadline_ns))
            .ok_or(CycleError::FramePool(FramePoolError::InvalidHandle))?;
        for datagram in plan.datagrams() {
            self.rx_index
                .arm(
                    datagram.index,
                    handle.index() as u16,
                    RxExpectation {
                        generation,
                        deadline_ns,
                        expected_address: datagram.address,
                        expected_size: datagram.payload_len as u16,
                        expected_type: datagram.command as u8,
                        expected_wkc: datagram.expected_wkc,
                    },
                )
                .map_err(CycleError::RxIndex)?;
        }
        Ok(length)
    }

    /// Build a cyclic frame directly into a DMA TX descriptor buffer and arm
    /// all planned RX datagrams against that descriptor generation.
    ///
    /// This is the zero-copy hot path for a platform MAC adapter: the master
    /// never stages the frame in `FramePool`, so the adapter can publish the
    /// returned handle to hardware after calling `DmaDescriptorRing::tx_submit`.
    /// If construction or RX registration fails, the CPU-owned descriptor is
    /// canceled and all expectations owned by its index are cleared.
    pub fn build_and_arm_dma_frame_from_plan<
        const TX: usize,
        const RX: usize,
        const DATAGRAMS: usize,
    >(
        &mut self,
        ring: &mut DmaDescriptorRing<TX, RX, MTU>,
        plan: &FramePlan<DATAGRAMS>,
        process_image: &[u8],
        generation: u16,
        deadline_ns: u64,
    ) -> Result<(DmaTxHandle, usize), CycleError<core::convert::Infallible>> {
        for datagram in plan.datagrams() {
            if self.rx_index.entry(datagram.index).state == RxSlotState::Armed {
                return Err(CycleError::RxIndex(RxIndexError::AlreadyArmed));
            }
        }

        let handle = ring.tx_acquire().map_err(CycleError::Dma)?;
        let length = {
            let buffer = match ring.tx_buffer_mut(handle) {
                Ok(buffer) => buffer,
                Err(error) => {
                    let _ = ring.tx_cancel(handle);
                    return Err(CycleError::Dma(error));
                }
            };
            match plan.build(
                buffer,
                self.config.destination_mac,
                self.config.source_mac,
                process_image,
            ) {
                Ok(length) => length,
                Err(error) => {
                    let _ = ring.tx_cancel(handle);
                    return Err(CycleError::Plan(error));
                }
            }
        };

        for datagram in plan.datagrams() {
            if let Err(error) = self.rx_index.arm(
                datagram.index,
                handle.index() as u16,
                RxExpectation {
                    generation,
                    deadline_ns,
                    expected_address: datagram.address,
                    expected_size: datagram.payload_len as u16,
                    expected_type: datagram.command as u8,
                    expected_wkc: datagram.expected_wkc,
                },
            ) {
                self.rx_index.cancel_slot(handle.index() as u16);
                let _ = ring.tx_cancel(handle);
                return Err(CycleError::RxIndex(error));
            }
        }
        Ok((handle, length))
    }

    /// Clear RX expectations owned by a DMA frame after the platform rejects
    /// it before hardware ownership is published. The descriptor itself must
    /// be released through `DmaDescriptorRing::tx_cancel`.
    pub fn cancel_dma_frame(&mut self, handle: DmaTxHandle) -> usize {
        self.rx_index.cancel_slot(handle.index() as u16)
    }

    /// Start a bounded receive session backed by a platform-owned DMA RX
    /// ring. The caller remains responsible for polling/completing/rearming
    /// descriptors and passes each completed buffer to `consume_frame`.
    pub fn begin_dma_receive_cycle(
        &mut self,
        start_ns: u64,
        generation: u16,
    ) -> DmaReceiveCycle<'_, SLOTS, MTU> {
        self.cycle = self.cycle.wrapping_add(1);
        let mut report = CycleReport::new(self.cycle);
        self.expire_rx_entries(start_ns, &mut report);
        let deadline_ns = start_ns.saturating_add(self.config.rx_budget_ns);
        DmaReceiveCycle {
            master: self,
            report,
            generation,
            deadline_ns,
        }
    }

    /// Publish a built DMA frame to the platform TX path. The platform owns
    /// completion after this method succeeds; it must call the ring's
    /// completion/reclaim operations after hardware reports the send done.
    pub fn submit_dma_frame<
        P: EthercatDmaTxPort,
        C: DmaCacheOps,
        const TX: usize,
        const RX: usize,
    >(
        &mut self,
        port: &mut P,
        ring: &mut DmaDescriptorRing<TX, RX, MTU>,
        handle: DmaTxHandle,
        length: usize,
        cache: &mut C,
    ) -> Result<(), CycleError<P::Error>> {
        ring.tx_submit(handle, length, cache)
            .map_err(CycleError::Dma)?;
        let submit_result = {
            let frame = ring.tx_buffer(handle).map_err(CycleError::Dma)?;
            port.tx_submit(handle, frame)
        };
        match submit_result {
            Ok(()) => Ok(()),
            Err(error) => {
                self.cancel_dma_frame(handle);
                let _ = ring.tx_abort(handle);
                Err(CycleError::Port(error))
            }
        }
    }

    pub fn build_control_request<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
        request: RequestHandle,
        frame: FrameHandle,
    ) -> Result<usize, CycleError<core::convert::Infallible>> {
        let datagram_index = pool
            .get(request)
            .ok_or(CycleError::Control(ControlError::InvalidHandle))?
            .datagram_index;
        if self.rx_index.entry(datagram_index).state == RxSlotState::Armed {
            return Err(CycleError::RxIndex(RxIndexError::AlreadyArmed));
        }

        let destination = self.config.destination_mac;
        let source = self.config.source_mac;
        let (length, datagram_index, expectation) = {
            let slot = self
                .frames
                .slot_mut(frame)
                .ok_or(CycleError::FramePool(FramePoolError::InvalidHandle))?;
            let control_request = pool
                .get_mut(request)
                .ok_or(CycleError::Control(ControlError::InvalidHandle))?;
            let length = control_request
                .build_frame(&mut slot.bytes, destination, source)
                .map_err(CycleError::Control)?;
            (
                length,
                control_request.datagram_index,
                control_request.expectation(),
            )
        };
        self.frames
            .slot_mut(frame)
            .ok_or(CycleError::FramePool(FramePoolError::InvalidHandle))?
            .len = length;
        self.rx_index
            .arm(datagram_index, request.index() as u16, expectation)
            .map_err(CycleError::RxIndex)?;
        Ok(length)
    }

    pub fn submit_frame<P: EthercatPort>(
        &mut self,
        port: &mut P,
        handle: FrameHandle,
    ) -> Result<(), CycleError<P::Error>> {
        let length = self
            .frames
            .slot(handle)
            .ok_or(CycleError::FramePool(FramePoolError::InvalidHandle))?
            .len;
        if length == 0 || length > MTU {
            return Err(CycleError::InvalidFrameLength);
        }
        let tx_result = {
            let slot = self
                .frames
                .slot(handle)
                .ok_or(CycleError::FramePool(FramePoolError::InvalidHandle))?;
            port.tx_submit(&slot.bytes[..length])
        };
        match tx_result {
            Ok(()) => self.frames.release(handle).map_err(CycleError::FramePool),
            Err(error) => {
                self.rx_index.cancel_slot(handle.index() as u16);
                let _ = self.frames.release(handle);
                Err(CycleError::Port(error))
            }
        }
    }

    pub fn arm_rx(
        &mut self,
        index: u8,
        slot_id: u16,
        expectation: RxExpectation,
    ) -> Result<(), RxIndexError> {
        self.rx_index.arm(index, slot_id, expectation)
    }

    pub fn rx_entry(&self, index: u8) -> crate::rx_index::RxIndexEntry {
        self.rx_index.entry(index)
    }

    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    pub fn cycle_receive<P: EthercatPort>(
        &mut self,
        port: &mut P,
        scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN],
        generation: u16,
    ) -> Result<CycleReport, CycleError<P::Error>> {
        let mut noop = ();
        self.cycle_receive_with_consumer(port, scratch, generation, &mut noop)
    }

    pub fn cycle_receive_with_consumer<P: EthercatPort, C: RxDatagramConsumer>(
        &mut self,
        port: &mut P,
        scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN],
        generation: u16,
        consumer: &mut C,
    ) -> Result<CycleReport, CycleError<P::Error>> {
        self.cycle = self.cycle.wrapping_add(1);
        let mut report = CycleReport::new(self.cycle);
        let start_ns = port.now_ns();
        let deadline_ns = start_ns.saturating_add(self.config.rx_budget_ns);
        self.expire_rx_entries(start_ns, &mut report);

        if port.link_state() == LinkState::Down {
            report.link_down = true;
            self.diagnostics.record(EventRecord::new(
                report.cycle,
                start_ns,
                EventCode::LinkDown,
                EventSeverity::Fault,
                0,
                0,
                0,
            ));
            return Ok(report);
        }

        while report.received_frames < self.config.rx_budget_frames
            && report.received_bytes < self.config.rx_budget_bytes
            && port.now_ns() < deadline_ns
        {
            let poll = port.rx_poll(scratch).map_err(CycleError::Port)?;
            let length = match poll {
                RxPoll::Empty => break,
                RxPoll::LinkDown => {
                    report.link_down = true;
                    self.diagnostics.record(EventRecord::new(
                        report.cycle,
                        port.now_ns(),
                        EventCode::LinkDown,
                        EventSeverity::Fault,
                        0,
                        0,
                        0,
                    ));
                    break;
                }
                RxPoll::Frame(length) => length,
            };
            if length > scratch.len() || length == 0 {
                report.corrupt_frames += 1;
                self.diagnostics.record(EventRecord::new(
                    report.cycle,
                    port.now_ns(),
                    EventCode::FrameCorrupt,
                    EventSeverity::Error,
                    0,
                    length as u32,
                    scratch.len() as u32,
                ));
                continue;
            }
            report.received_frames += 1;
            report.received_bytes = report.received_bytes.saturating_add(length);
            let received_at_ns = port.now_ns();
            self.process_received_frame(
                &scratch[..length],
                generation,
                report.cycle,
                received_at_ns,
                &mut report,
                consumer,
            );
        }

        self.expire_rx_entries(port.now_ns(), &mut report);

        self.finish_receive_report(&mut report, port.now_ns(), deadline_ns);
        self.rx_index.reset_complete();
        Ok(report)
    }

    fn process_received_frame<C: RxDatagramConsumer>(
        &mut self,
        bytes: &[u8],
        generation: u16,
        cycle: u64,
        received_at_ns: u64,
        report: &mut CycleReport,
        consumer: &mut C,
    ) {
        let frame = match FrameView::parse(bytes) {
            Ok(frame) => frame,
            Err(_) => {
                self.record_corrupt_frame(cycle, received_at_ns, bytes.len(), 0);
                report.corrupt_frames += 1;
                return;
            }
        };
        for datagram in frame.datagrams() {
            let datagram = match datagram {
                Ok(datagram) => datagram,
                Err(_) => {
                    report.corrupt_frames += 1;
                    self.diagnostics.record(EventRecord::new(
                        cycle,
                        received_at_ns,
                        EventCode::FrameCorrupt,
                        EventSeverity::Error,
                        0,
                        0,
                        0,
                    ));
                    break;
                }
            };
            report.parsed_datagrams += 1;
            let entry = self.rx_index.entry(datagram.header.index);
            if entry.state != RxSlotState::Armed {
                report.unmatched_datagrams += 1;
                self.diagnostics.record(EventRecord::new(
                    cycle,
                    received_at_ns,
                    EventCode::RxUnmatched,
                    EventSeverity::Warning,
                    datagram.header.index,
                    entry.state as u32,
                    0,
                ));
                continue;
            }
            match self.rx_index.validate_and_complete(
                datagram.header.index,
                RxResponse {
                    generation,
                    address: datagram.header.address,
                    payload_size: datagram.payload.len() as u16,
                    command: datagram.header.command as u8,
                    working_counter: datagram.working_counter,
                    received_at_ns,
                },
            ) {
                Ok(completion) => {
                    if !consumer.accept(
                        cycle,
                        received_at_ns,
                        completion,
                        datagram.header,
                        datagram.payload,
                    ) {
                        report.consumer_rejections += 1;
                        self.diagnostics.record(EventRecord::new(
                            cycle,
                            received_at_ns,
                            EventCode::ConsumerRejected,
                            EventSeverity::Error,
                            datagram.header.index,
                            datagram.working_counter as u32,
                            0,
                        ));
                    }
                }
                Err(RxIndexError::WorkingCounterMismatch) => {
                    report.wkc_mismatches += 1;
                    self.diagnostics.record(EventRecord::new(
                        cycle,
                        received_at_ns,
                        EventCode::WorkingCounterMismatch,
                        EventSeverity::Error,
                        datagram.header.index,
                        entry.expected_wkc as u32,
                        datagram.working_counter as u32,
                    ));
                }
                Err(RxIndexError::DeadlineExceeded) => {
                    report.timed_out_datagrams += 1;
                    self.diagnostics.record(EventRecord::new(
                        cycle,
                        received_at_ns,
                        EventCode::RxTimeout,
                        EventSeverity::Error,
                        datagram.header.index,
                        0,
                        0,
                    ));
                }
                Err(RxIndexError::UnknownIndex | RxIndexError::NotArmed) => {
                    report.unmatched_datagrams += 1;
                    self.diagnostics.record(EventRecord::new(
                        cycle,
                        received_at_ns,
                        EventCode::RxUnmatched,
                        EventSeverity::Warning,
                        datagram.header.index,
                        entry.state as u32,
                        0,
                    ));
                }
                Err(_) => {
                    report.corrupt_frames += 1;
                    self.diagnostics.record(EventRecord::new(
                        cycle,
                        received_at_ns,
                        EventCode::FrameCorrupt,
                        EventSeverity::Error,
                        datagram.header.index,
                        0,
                        0,
                    ));
                }
            }
        }
    }

    fn record_corrupt_frame(&mut self, cycle: u64, timestamp_ns: u64, length: usize, limit: usize) {
        self.diagnostics.record(EventRecord::new(
            cycle,
            timestamp_ns,
            EventCode::FrameCorrupt,
            EventSeverity::Error,
            0,
            length as u32,
            limit as u32,
        ));
    }

    fn finish_receive_report(&mut self, report: &mut CycleReport, now_ns: u64, deadline_ns: u64) {
        if report.received_frames >= self.config.rx_budget_frames
            || report.received_bytes >= self.config.rx_budget_bytes
            || now_ns >= deadline_ns
        {
            report.budget_exhausted = true;
            self.diagnostics.record(EventRecord::new(
                report.cycle,
                now_ns,
                EventCode::RxBudgetExhausted,
                EventSeverity::Warning,
                0,
                report.received_frames as u32,
                report.received_bytes as u32,
            ));
        }
    }

    fn expire_rx_entries(&mut self, now_ns: u64, report: &mut CycleReport) {
        for index in self.rx_index.expire_armed(now_ns).indices() {
            report.timed_out_datagrams += 1;
            self.diagnostics.record(EventRecord::new(
                report.cycle,
                now_ns,
                EventCode::RxTimeout,
                EventSeverity::Error,
                index,
                0,
                0,
            ));
        }
    }
}
