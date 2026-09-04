use esop_ethercat_core::wire::{
    Command, DATAGRAM_HEADER_LEN, DatagramHeader, ETHERCAT_FRAME_HEADER_LEN, ETHERNET_HEADER_LEN,
    FrameBuilder, FrameView, MAX_ETHERNET_FRAME_LEN,
};
use esop_ethercat_core::{
    ControlRequestPool, ControlRxConsumer, DatagramPlan, DmaDescriptorRing, DmaOwner, DmaTxHandle,
    Domain, DomainSegment, EthercatDmaTxPort, EthercatMaster, EthercatPort, EthercatState,
    EventCode, ExpectedSlave, FrameHandle, FramePlan, LinkState, MailboxConfig, MailboxController,
    MailboxProtocol, MasterConfig, NoopDmaCache, PortError, RegisterOperation, RxExpectation,
    RxPoll, RxSlotState, SdoProgress, SdoTransfer, SlaveIdentity, StartupConfig, StartupController,
    StartupProgress,
};

const MTU: usize = MAX_ETHERNET_FRAME_LEN;

struct MockPort {
    rx: [u8; MTU],
    rx_len: usize,
    pending: bool,
    now_ns: u64,
    response_wkc: u16,
    response_payload: [u8; MTU],
    response_payload_len: usize,
    tx_len: usize,
    fail_tx: bool,
}

impl MockPort {
    fn new(response_wkc: u16) -> Self {
        Self {
            rx: [0; MTU],
            rx_len: 0,
            pending: false,
            now_ns: 0,
            response_wkc,
            response_payload: [0; MTU],
            response_payload_len: 0,
            tx_len: 0,
            fail_tx: false,
        }
    }

    fn with_response(response_wkc: u16, payload: &[u8]) -> Self {
        let mut port = Self::new(response_wkc);
        port.set_response(payload);
        port
    }

    fn with_time(response_wkc: u16, now_ns: u64) -> Self {
        let mut port = Self::new(response_wkc);
        port.now_ns = now_ns;
        port
    }

    fn set_response(&mut self, payload: &[u8]) {
        self.response_payload.fill(0);
        self.response_payload[..payload.len()].copy_from_slice(payload);
        self.response_payload_len = payload.len();
    }
}

impl EthercatPort for MockPort {
    type Error = PortError;

    fn link_state(&self) -> LinkState {
        LinkState::Up
    }

    fn now_ns(&self) -> u64 {
        self.now_ns
    }

    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        if self.fail_tx {
            return Err(PortError::HardwareFault);
        }
        self.tx_len = frame.len();
        self.rx[..frame.len()].copy_from_slice(frame);

        let view = FrameView::parse(frame).map_err(|_| PortError::HardwareFault)?;
        let datagram = view
            .datagrams()
            .next()
            .ok_or(PortError::HardwareFault)?
            .map_err(|_| PortError::HardwareFault)?;
        let wkc_offset = ETHERNET_HEADER_LEN
            + ETHERCAT_FRAME_HEADER_LEN
            + DATAGRAM_HEADER_LEN
            + datagram.payload.len();
        if self.response_payload_len != 0 {
            if self.response_payload_len != datagram.payload.len() {
                return Err(PortError::HardwareFault);
            }
            let payload_offset =
                ETHERNET_HEADER_LEN + ETHERCAT_FRAME_HEADER_LEN + DATAGRAM_HEADER_LEN;
            self.rx[payload_offset..payload_offset + datagram.payload.len()]
                .copy_from_slice(&self.response_payload[..self.response_payload_len]);
        }
        self.rx[wkc_offset..wkc_offset + 2].copy_from_slice(&self.response_wkc.to_le_bytes());
        self.rx_len = frame.len();
        self.pending = true;
        Ok(())
    }

    fn rx_poll(
        &mut self,
        destination: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, Self::Error> {
        if !self.pending {
            return Ok(RxPoll::Empty);
        }
        destination[..self.rx_len].copy_from_slice(&self.rx[..self.rx_len]);
        self.pending = false;
        Ok(RxPoll::Frame(self.rx_len))
    }
}

struct MockDmaTxPort {
    frame: [u8; MTU],
    frame_len: usize,
    handle: Option<DmaTxHandle>,
    fail: bool,
}

impl MockDmaTxPort {
    fn new() -> Self {
        Self {
            frame: [0; MTU],
            frame_len: 0,
            handle: None,
            fail: false,
        }
    }
}

impl EthercatDmaTxPort for MockDmaTxPort {
    type Error = PortError;

    fn tx_submit(&mut self, handle: DmaTxHandle, frame: &[u8]) -> Result<(), Self::Error> {
        if self.fail {
            return Err(PortError::HardwareFault);
        }
        self.frame[..frame.len()].copy_from_slice(frame);
        self.frame_len = frame.len();
        self.handle = Some(handle);
        Ok(())
    }
}

fn build_one_datagram(master: &mut EthercatMaster<2, MTU>) -> (u8, FrameHandle) {
    let handle = master.acquire_frame(42, 100_000).unwrap();
    let index = 7;
    let length = {
        let mut builder = master.frame_builder(handle).unwrap();
        builder
            .push(Command::Lrw, index, 0x1000, &[0x11, 0x22, 0x33])
            .unwrap();
        builder.finish().unwrap()
    };
    master.finish_frame(handle, length).unwrap();
    (index, handle)
}

#[test]
fn cycle_round_trip_commits_and_releases_rx_slot() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    let (index, handle) = build_one_datagram(&mut master);
    master
        .arm_rx(
            index,
            handle.index() as u16,
            RxExpectation {
                generation: 42,
                deadline_ns: 100_000,
                expected_address: 0x1000,
                expected_size: 3,
                expected_type: Command::Lrw as u8,
                expected_wkc: 1,
            },
        )
        .unwrap();

    let mut port = MockPort::new(1);
    master.submit_frame(&mut port, handle).unwrap();
    assert_eq!(port.tx_len, 64);

    let mut scratch = [0; MTU];
    let report = master.cycle_receive(&mut port, &mut scratch, 42).unwrap();
    assert_eq!(report.received_frames, 1);
    assert_eq!(report.parsed_datagrams, 1);
    assert_eq!(report.unmatched_datagrams, 0);
    assert_eq!(report.corrupt_frames, 0);
    assert_eq!(report.wkc_mismatches, 0);
    assert!(!report.budget_exhausted);
    assert_eq!(master.rx_entry(index).state, RxSlotState::Empty);
}

#[test]
fn dma_plan_builds_in_ring_and_observes_ownership_edges() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    let mut plan = FramePlan::<2>::new();
    plan.push(DatagramPlan {
        command: Command::Lrw,
        index: 21,
        address: 0x1000,
        payload_offset: 0,
        payload_len: 2,
        expected_wkc: 1,
    })
    .unwrap();
    plan.push(DatagramPlan {
        command: Command::Fprd,
        index: 22,
        address: 0x2000,
        payload_offset: 2,
        payload_len: 2,
        expected_wkc: 1,
    })
    .unwrap();

    let mut ring = DmaDescriptorRing::<1, 0, MTU>::new();
    let (handle, length) = master
        .build_and_arm_dma_frame_from_plan(&mut ring, &plan, &[0xA0, 0xA1, 0xB0, 0xB1], 7, 100_000)
        .unwrap();
    assert_eq!(ring.tx_owner(handle), Ok(DmaOwner::CpuOwned));
    let expected_frame = {
        let frame = ring.tx_buffer_mut(handle).unwrap();
        let view = FrameView::parse(&frame[..length]).unwrap();
        assert_eq!(view.datagram_count(), 2);
        assert_eq!(
            view.datagrams().next().unwrap().unwrap().payload,
            &[0xA0, 0xA1]
        );
        let mut expected = [0; MTU];
        expected[..length].copy_from_slice(&frame[..length]);
        expected
    };
    assert_eq!(master.rx_entry(21).state, RxSlotState::Armed);
    assert_eq!(master.rx_entry(21).generation, 7);
    assert_eq!(master.rx_entry(21).slot_id, handle.index() as u16);

    let mut cache = NoopDmaCache;
    let mut port = MockDmaTxPort::new();
    master
        .submit_dma_frame(&mut port, &mut ring, handle, length, &mut cache)
        .unwrap();
    assert_eq!(ring.tx_owner(handle), Ok(DmaOwner::DmaOwned));
    assert_eq!(ring.tx_length(handle), Ok(length));
    assert_eq!(ring.tx_buffer(handle).unwrap().len(), length);
    assert_eq!(port.handle, Some(handle));
    assert_eq!(&port.frame[..port.frame_len], &expected_frame[..length]);
}

#[test]
fn dma_plan_failure_releases_cpu_descriptor_and_rx_expectations() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    let mut plan = FramePlan::<1>::new();
    plan.push(DatagramPlan {
        command: Command::Lwr,
        index: 31,
        address: 0x3000,
        payload_offset: 2,
        payload_len: 2,
        expected_wkc: 1,
    })
    .unwrap();
    let mut ring = DmaDescriptorRing::<1, 0, MTU>::new();

    assert!(matches!(
        master.build_and_arm_dma_frame_from_plan(&mut ring, &plan, &[0x01], 9, 100_000),
        Err(esop_ethercat_core::CycleError::Plan(
            esop_ethercat_core::PlanError::ProcessImageOutOfBounds
        ))
    ));
    assert_eq!(ring.tx_acquire().unwrap().index(), 0);
    assert_eq!(master.rx_entry(31).state, RxSlotState::Empty);
}

#[test]
fn dma_port_failure_aborts_descriptor_and_owned_expectations() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    let mut plan = FramePlan::<1>::new();
    plan.push(DatagramPlan {
        command: Command::Lrw,
        index: 41,
        address: 0x4000,
        payload_offset: 0,
        payload_len: 1,
        expected_wkc: 1,
    })
    .unwrap();
    let mut ring = DmaDescriptorRing::<1, 0, MTU>::new();
    let (handle, length) = master
        .build_and_arm_dma_frame_from_plan(&mut ring, &plan, &[0x5A], 11, 100_000)
        .unwrap();
    let mut port = MockDmaTxPort::new();
    port.fail = true;
    let mut cache = NoopDmaCache;

    assert!(matches!(
        master.submit_dma_frame(&mut port, &mut ring, handle, length, &mut cache),
        Err(esop_ethercat_core::CycleError::Port(
            PortError::HardwareFault
        ))
    ));
    assert_eq!(ring.tx_owner(handle), Ok(DmaOwner::Free));
    assert_eq!(master.rx_entry(41).state, RxSlotState::Empty);
}

#[test]
fn dma_receive_cycle_parses_completed_descriptor_buffer_in_place() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    master
        .arm_rx(
            51,
            0,
            RxExpectation {
                generation: 13,
                deadline_ns: 100_000,
                expected_address: 0x5000,
                expected_size: 3,
                expected_type: Command::Lrw as u8,
                expected_wkc: 0,
            },
        )
        .unwrap();

    let mut ring = DmaDescriptorRing::<0, 1, MTU>::new();
    let rx_handle = ring.rx_acquire().unwrap();
    let length = {
        let buffer = ring.rx_buffer_mut(rx_handle).unwrap();
        let mut builder = FrameBuilder::new(buffer, [0xFF; 6], [1, 2, 3, 4, 5, 6]).unwrap();
        builder
            .push(Command::Lrw, 51, 0x5000, &[0x61, 0x62, 0x63])
            .unwrap();
        builder.finish().unwrap()
    };
    let mut cache = NoopDmaCache;
    ring.rx_submit(rx_handle, &mut cache).unwrap();
    ring.rx_complete(rx_handle, length).unwrap();
    let completed = ring.rx_poll(&mut cache).unwrap().unwrap();

    let mut receive = master.begin_dma_receive_cycle(0, 13);
    assert!(receive.can_consume(1));
    {
        let frame = ring.rx_buffer(completed).unwrap();
        receive.consume_frame(frame, 10, &mut ());
    }
    let report = receive.finish(10);

    assert_eq!(report.received_frames, 1);
    assert_eq!(report.parsed_datagrams, 1);
    assert_eq!(report.wkc_mismatches, 0);
    assert_eq!(report.corrupt_frames, 0);
    assert_eq!(master.rx_entry(51).state, RxSlotState::Empty);
    ring.rx_rearm(completed, &mut cache).unwrap();
    assert_eq!(ring.rx_owner(completed), Ok(DmaOwner::DmaOwned));
}

#[test]
fn tx_failure_releases_frame_and_owned_rx_expectations() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    let (index, handle) = build_one_datagram(&mut master);
    master
        .arm_rx(
            index,
            handle.index() as u16,
            RxExpectation {
                generation: 42,
                deadline_ns: 100_000,
                expected_address: 0x1000,
                expected_size: 3,
                expected_type: Command::Lrw as u8,
                expected_wkc: 1,
            },
        )
        .unwrap();

    let mut port = MockPort::new(1);
    port.fail_tx = true;
    assert!(matches!(
        master.submit_frame(&mut port, handle),
        Err(esop_ethercat_core::CycleError::Port(
            PortError::HardwareFault
        ))
    ));
    assert_eq!(master.rx_entry(index).state, RxSlotState::Empty);
    assert_eq!(
        master.acquire_frame(43, 100_000).unwrap().index(),
        handle.index()
    );
}

#[test]
fn wkc_failure_is_reported_and_index_can_be_rearmed() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    let (index, handle) = build_one_datagram(&mut master);
    master
        .arm_rx(
            index,
            handle.index() as u16,
            RxExpectation {
                generation: 42,
                deadline_ns: 100_000,
                expected_address: 0x1000,
                expected_size: 3,
                expected_type: Command::Lrw as u8,
                expected_wkc: 1,
            },
        )
        .unwrap();

    let mut port = MockPort::new(0);
    master.submit_frame(&mut port, handle).unwrap();
    let mut scratch = [0; MTU];
    let report = master.cycle_receive(&mut port, &mut scratch, 42).unwrap();

    assert_eq!(report.wkc_mismatches, 1);
    assert_eq!(master.rx_entry(index).state, RxSlotState::Empty);
    let event = master.diagnostics().pop().unwrap();
    assert_eq!(event.code, EventCode::WorkingCounterMismatch);
    assert_eq!(event.index, index);
    assert_eq!(event.value, 1);
    assert_eq!(event.aux, 0);
    master
        .arm_rx(
            index,
            handle.index() as u16,
            RxExpectation {
                generation: 43,
                deadline_ns: 100_000,
                expected_address: 0x1000,
                expected_size: 3,
                expected_type: Command::Lrw as u8,
                expected_wkc: 1,
            },
        )
        .unwrap();
}

#[test]
fn missing_response_expires_rx_index_and_records_timeout() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    master
        .arm_rx(
            17,
            0,
            RxExpectation {
                generation: 7,
                deadline_ns: 10,
                expected_address: 0x1000,
                expected_size: 3,
                expected_type: Command::Lrw as u8,
                expected_wkc: 1,
            },
        )
        .unwrap();

    let mut port = MockPort::with_time(1, 11);
    let mut scratch = [0; MTU];
    let report = master.cycle_receive(&mut port, &mut scratch, 7).unwrap();

    assert_eq!(report.timed_out_datagrams, 1);
    assert_eq!(master.rx_entry(17).state, RxSlotState::Empty);
    let event = master.diagnostics().pop().unwrap();
    assert_eq!(event.code, EventCode::RxTimeout);
    assert_eq!(event.index, 17);
}

#[test]
fn datagram_header_can_be_decoded_from_the_response() {
    let mut bytes = [0u8; MTU];
    let mut builder = FrameBuilder::new(&mut bytes, [0xFF; 6], [1, 2, 3, 4, 5, 6]).unwrap();
    builder.push(Command::Lrd, 3, 0x2000, &[9, 8]).unwrap();
    let length = builder.finish().unwrap();
    let frame = FrameView::parse(&bytes[..length]).unwrap();
    let datagram = frame.datagrams().next().unwrap().unwrap();
    assert_eq!(
        datagram.header,
        DatagramHeader {
            command: Command::Lrd,
            index: 3,
            address: 0x2000,
            length: 2,
            last: true,
        }
    );
}

#[test]
fn master_builds_a_frame_from_a_precomputed_plan() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    let handle = master.acquire_frame(7, 100_000).unwrap();
    let mut plan = FramePlan::<1>::new();
    plan.push(DatagramPlan {
        command: esop_ethercat_core::wire::Command::Lwr,
        index: 4,
        address: 0x3000,
        payload_offset: 1,
        payload_len: 2,
        expected_wkc: 1,
    })
    .unwrap();

    let length = master
        .build_frame_from_plan(handle, &plan, &[0x00, 0x55, 0x66])
        .unwrap();
    let slot = master.frame_slot_mut(handle).unwrap();
    let view = esop_ethercat_core::wire::FrameView::parse(&slot.bytes[..length]).unwrap();
    let datagram = view.datagrams().next().unwrap().unwrap();
    assert_eq!(datagram.payload, &[0x55, 0x66]);
}

#[test]
fn planned_frame_is_armed_and_consumed_by_domain() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    let frame = master.acquire_frame(23, 100_000).unwrap();
    let mut plan = FramePlan::<1>::new();
    plan.push(DatagramPlan {
        command: Command::Lrw,
        index: 12,
        address: 0x1000,
        payload_offset: 0,
        payload_len: 3,
        expected_wkc: 1,
    })
    .unwrap();

    let length = master
        .build_and_arm_frame_from_plan(frame, &plan, &[0x31, 0x32, 0x33])
        .unwrap();
    assert_eq!(length, 64);
    assert_eq!(master.rx_entry(12).state, RxSlotState::Armed);

    let mut domain = Domain::<3, 1>::new(0x1000);
    domain
        .add_segment(DomainSegment {
            datagram_index: 12,
            input_offset: 0,
            len: 3,
            expected_wkc: 1,
        })
        .unwrap();
    domain.begin_receive(23).unwrap();

    let mut port = MockPort::new(1);
    master.submit_frame(&mut port, frame).unwrap();
    let mut scratch = [0; MTU];
    let report = master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 23, &mut domain)
        .unwrap();

    assert_eq!(report.parsed_datagrams, 1);
    assert!(domain.finish_receive(23, report.cycle).unwrap());
    assert_eq!(domain.input(), &[0x31, 0x32, 0x33]);
}

#[test]
fn verified_response_is_staged_then_committed_to_domain() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    let (index, handle) = build_one_datagram(&mut master);
    master
        .arm_rx(
            index,
            handle.index() as u16,
            RxExpectation {
                generation: 42,
                deadline_ns: 100_000,
                expected_address: 0x1000,
                expected_size: 3,
                expected_type: Command::Lrw as u8,
                expected_wkc: 1,
            },
        )
        .unwrap();
    let mut domain = Domain::<3, 1>::new(0x1000);
    domain
        .add_segment(DomainSegment {
            datagram_index: index,
            input_offset: 0,
            len: 3,
            expected_wkc: 1,
        })
        .unwrap();
    domain.begin_receive(42).unwrap();

    let mut port = MockPort::new(1);
    master.submit_frame(&mut port, handle).unwrap();
    let mut scratch = [0; MTU];
    let report = master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 42, &mut domain)
        .unwrap();

    assert_eq!(report.consumer_rejections, 0);
    assert!(domain.finish_receive(42, report.cycle).unwrap());
    assert_eq!(domain.input(), &[0x11, 0x22, 0x33]);
    assert!(domain.quality().valid);
}

#[test]
fn control_request_is_built_armed_and_completed_by_the_master() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    let mut requests = ControlRequestPool::<2>::new();
    let request = requests
        .acquire(19, 51, 0x1000_0130, RegisterOperation::Read, &[], 100_000)
        .unwrap();
    let frame = master.acquire_frame(51, 100_000).unwrap();
    let length = master
        .build_control_request(&mut requests, request, frame)
        .unwrap();
    assert_eq!(length, 64);
    assert_eq!(master.rx_entry(19).slot_id, request.index() as u16);

    let mut port = MockPort::new(1);
    master.submit_frame(&mut port, frame).unwrap();
    let mut scratch = [0; MTU];
    let mut consumer = ControlRxConsumer::new(&mut requests);
    let report = master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 51, &mut consumer)
        .unwrap();

    assert_eq!(report.parsed_datagrams, 1);
    assert_eq!(consumer.rejected(), 0);
    assert_eq!(
        requests.get(request).unwrap().state,
        esop_ethercat_core::RequestState::Complete
    );
}

#[test]
fn startup_control_request_round_trips_through_master_and_rx_consumer() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    let expected = [ExpectedSlave {
        position: 0,
        station_address: 0x1000,
        identity: SlaveIdentity::EMPTY,
    }];
    let mut startup = StartupController::<2>::new(0x1000);
    startup
        .start(12, 0, StartupConfig::new(EthercatState::PreOp), &expected)
        .unwrap();
    let action = startup.next_action(1).unwrap().unwrap();

    let mut requests = ControlRequestPool::<1>::new();
    let request = startup.enqueue_pending(&mut requests).unwrap();
    let frame = master
        .acquire_frame(action.generation(), action.deadline_ns())
        .unwrap();
    master
        .build_control_request(&mut requests, request, frame)
        .unwrap();

    let mut port = MockPort::with_response(1, &[0x88, 0x02]);
    master.submit_frame(&mut port, frame).unwrap();
    let mut scratch = [0; MTU];
    let report = {
        let mut consumer = ControlRxConsumer::new(&mut requests);
        master
            .cycle_receive_with_consumer(
                &mut port,
                &mut scratch,
                action.generation(),
                &mut consumer,
            )
            .unwrap()
    };
    assert_eq!(report.parsed_datagrams, 1);
    assert_eq!(report.consumer_rejections, 0);

    assert_eq!(
        startup.accept_completed(&mut requests, request, 2),
        Ok(StartupProgress::Advanced)
    );
    assert_eq!(requests.in_use(), 0);
}

#[test]
fn coe_sdo_round_trips_through_mailbox_control_and_master() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    let mut pool = ControlRequestPool::<2>::new();
    let mut sdo = SdoTransfer::new();
    sdo.start_download(0x6040, 0, &[0x06, 0x00], false).unwrap();

    let mut mailbox = MailboxController::new();
    mailbox
        .start(
            MailboxConfig::new(0x1000, 32, 0x1100, 32),
            1,
            7,
            0,
            MailboxProtocol::CoE,
            sdo.request().unwrap(),
        )
        .unwrap();

    let _send = mailbox.next_action(1).unwrap().unwrap();
    let send_request = mailbox.enqueue_pending(&mut pool).unwrap();
    let send_frame = master.acquire_frame(7, 100_000).unwrap();
    master
        .build_control_request(&mut pool, send_request, send_frame)
        .unwrap();
    let mut port = MockPort::with_response(1, &[0; 16]);
    master.submit_frame(&mut port, send_frame).unwrap();
    let mut scratch = [0; MTU];
    let mut consumer = ControlRxConsumer::new(&mut pool);
    master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 7, &mut consumer)
        .unwrap();
    assert_eq!(
        mailbox.accept_completed(&mut pool, send_request, 2),
        Ok(esop_ethercat_core::MailboxProgress::Advanced)
    );
    assert_eq!(mailbox.phase(), esop_ethercat_core::MailboxPhase::Polling);

    let mut response = [0; 32];
    esop_ethercat_core::MailboxHeader {
        length: 6,
        address: 0,
        priority: 0,
        protocol: MailboxProtocol::CoE,
        counter: 1,
    }
    .encode(&mut response)
    .unwrap();
    response[6..12].copy_from_slice(&[0, 0x30, 0x60, 0x40, 0x60, 0]);
    port.set_response(&response);

    let _poll = mailbox.next_action(3).unwrap().unwrap();
    let poll_request = mailbox.enqueue_pending(&mut pool).unwrap();
    let poll_frame = master.acquire_frame(7, 100_000).unwrap();
    master
        .build_control_request(&mut pool, poll_request, poll_frame)
        .unwrap();
    master.submit_frame(&mut port, poll_frame).unwrap();
    let mut consumer = ControlRxConsumer::new(&mut pool);
    master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 7, &mut consumer)
        .unwrap();
    assert_eq!(
        mailbox.accept_completed(&mut pool, poll_request, 4),
        Ok(esop_ethercat_core::MailboxProgress::Complete)
    );
    assert_eq!(
        sdo.accept_response(mailbox.response().unwrap().1),
        Ok(SdoProgress::Complete)
    );
    assert_eq!(sdo.phase(), esop_ethercat_core::SdoPhase::Complete);
}

#[test]
fn control_request_index_conflict_does_not_mutate_request_or_frame() {
    let config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    let mut master = EthercatMaster::<2, MTU>::new(config);
    let mut requests = ControlRequestPool::<1>::new();
    let request = requests
        .acquire(21, 61, 0x1000_0130, RegisterOperation::Read, &[], 100_000)
        .unwrap();
    let frame = master.acquire_frame(61, 100_000).unwrap();
    master
        .arm_rx(
            21,
            99,
            RxExpectation {
                generation: 61,
                deadline_ns: 100_000,
                expected_address: 0x1000_0130,
                expected_size: 0,
                expected_type: Command::Fprd as u8,
                expected_wkc: 1,
            },
        )
        .unwrap();

    assert_eq!(
        master.build_control_request(&mut requests, request, frame),
        Err(esop_ethercat_core::CycleError::RxIndex(
            esop_ethercat_core::RxIndexError::AlreadyArmed
        ))
    );
    assert_eq!(
        requests.get(request).unwrap().state,
        esop_ethercat_core::RequestState::Prepared
    );
    assert_eq!(master.frame_slot_mut(frame).unwrap().len, 0);
}
