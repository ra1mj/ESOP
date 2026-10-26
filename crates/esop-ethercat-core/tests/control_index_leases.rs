use esop_ethercat_core::wire::{Command, DatagramHeader, FrameView, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{
    ControlError, ControlRequestPool, RegisterOperation, RequestState, RxMatch,
};

#[test]
fn frozen_namespaces_validate_before_any_request() {
    assert!(matches!(
        ControlRequestPool::<0>::new_with_datagram_indices(&[220]),
        Err(ControlError::TooManyRequests)
    ));
    assert!(matches!(
        ControlRequestPool::<65>::new_with_datagram_indices(&[220]),
        Err(ControlError::TooManyRequests)
    ));
    assert!(matches!(
        ControlRequestPool::<1>::new_with_datagram_indices(&[]),
        Err(ControlError::InvalidDatagramIndices)
    ));
    assert!(matches!(
        ControlRequestPool::<1>::new_with_datagram_indices(&[0; 65]),
        Err(ControlError::InvalidDatagramIndices)
    ));
    assert!(matches!(
        ControlRequestPool::<1>::new_with_datagram_indices(&[220, 220]),
        Err(ControlError::DuplicateDatagramIndex)
    ));
    assert!(matches!(
        ControlRequestPool::<1>::new_with_datagram_indices_excluding(
            &[220, 240],
            &[0, 1, 2, 3, 240, 241]
        ),
        Err(ControlError::ReservedDatagramIndex)
    ));
    ControlRequestPool::<1>::new_with_datagram_indices_excluding(
        &[220, 221],
        &[0, 1, 2, 3, 240, 241],
    )
    .unwrap();
}

#[test]
fn identical_logical_actions_have_distinct_retained_wire_leases() {
    let mut pool = ControlRequestPool::<3>::new_with_datagram_indices(&[220, 221]).unwrap();
    let first = pool
        .acquire(1, 9, 0x0130_1000, RegisterOperation::Read, &[0; 2], 100)
        .unwrap();
    let second = pool
        .acquire(1, 9, 0x0130_1000, RegisterOperation::Read, &[0; 2], 100)
        .unwrap();
    assert_eq!(pool.get(first).unwrap().action_datagram_index(), 1);
    assert_eq!(pool.get(second).unwrap().action_datagram_index(), 1);
    assert_eq!(pool.get(first).unwrap().datagram_index, 220);
    assert_eq!(pool.get(second).unwrap().datagram_index, 221);
    assert_eq!(
        pool.acquire(2, 9, 0, RegisterOperation::Read, &[0; 2], 100),
        Err(ControlError::DatagramIndicesExhausted)
    );
    pool.fail_transmit(first).unwrap();
    assert_eq!(
        pool.acquire(2, 9, 0, RegisterOperation::Read, &[0; 2], 100),
        Err(ControlError::DatagramIndicesExhausted)
    );
    assert_eq!(pool.in_use(), 2);
    pool.release(first).unwrap();
    let next = pool
        .acquire(240, 9, 0, RegisterOperation::Read, &[0; 2], 100)
        .unwrap();
    assert_eq!(pool.get(next).unwrap().datagram_index, 220);
    assert!(pool.get(next).unwrap().matches_action(
        240,
        9,
        0,
        RegisterOperation::Read,
        &[0; 2],
        2,
        100
    ));
}

#[test]
fn wire_completion_uses_the_lease_and_actions_keep_their_original_identity() {
    let mut pool = ControlRequestPool::<1>::new_with_datagram_indices(&[220, 221]).unwrap();
    let handle = pool
        .acquire(240, 9, 0x0130_1000, RegisterOperation::Read, &[0; 2], 100)
        .unwrap();
    let mut bytes = [0; 64];
    let length = pool
        .build_into_buffer(handle, &mut bytes, [0xff; 6], [0; 6])
        .unwrap();
    let header = FrameView::parse(&bytes[..length])
        .unwrap()
        .datagrams()
        .next()
        .unwrap()
        .unwrap()
        .header;
    assert_eq!(header.index, 220);
    let completion = RxMatch {
        slot_id: handle.index() as u16,
        generation: 9,
        working_counter: 1,
    };
    assert_eq!(
        pool.complete_match(
            completion,
            DatagramHeader {
                index: 240,
                ..header
            },
            &[8, 0]
        ),
        Err(ControlError::DatagramIndexMismatch)
    );
    assert_eq!(pool.get(handle).unwrap().state, RequestState::InFlight);
    pool.complete_match(completion, header, &[8, 0]).unwrap();
    assert!(pool.get(handle).unwrap().matches_action(
        240,
        9,
        0x0130_1000,
        RegisterOperation::Read,
        &[0; 2],
        2,
        100
    ));
    assert_eq!(pool.get(handle).unwrap().payload(), &[8, 0]);
    assert_eq!(
        pool.complete_match(completion, header, &[4, 0]),
        Err(ControlError::InvalidState)
    );
    assert_eq!(pool.get(handle).unwrap().payload(), &[8, 0]);
    pool.release(handle).unwrap();
    let handle = pool
        .acquire(1, 9, 0, RegisterOperation::Read, &[0; 2], 100)
        .unwrap();
    pool.get_mut(handle).unwrap().datagram_index = 0;
    assert_eq!(
        pool.build_into_buffer(handle, &mut bytes, [0xff; 6], [0; 6]),
        Err(ControlError::DatagramIndexMismatch)
    );
    pool.release(handle).unwrap(); // releases the immutable lease, not the tampered public field
    assert!(
        pool.acquire(1, 9, 0, RegisterOperation::Read, &[0; 2], 100)
            .is_ok()
    );
}

#[test]
fn mailbox_completion_compares_logical_identity_after_wire_translation() {
    use esop_ethercat_core::{MailboxConfig, MailboxController, MailboxPhase, MailboxProtocol};
    let mut mailbox = MailboxController::new();
    mailbox
        .start(
            MailboxConfig::new(0x1000, 32, 0x1100, 32),
            0x1001,
            9,
            0,
            MailboxProtocol::CoE,
            &[1, 2],
        )
        .unwrap();
    let mut pool = ControlRequestPool::<1>::new_with_datagram_indices(&[220, 221]).unwrap();
    for (time, payload) in [(1, &[0][..]), (3, &[0; 32][..])] {
        let action = mailbox.next_action(time).unwrap().unwrap();
        let handle = mailbox.enqueue_pending(&mut pool).unwrap();
        assert_ne!(
            pool.get(handle).unwrap().datagram_index,
            action.datagram_index
        );
        let mut frame = [0; 64];
        let length = pool
            .build_into_buffer(handle, &mut frame, [0xff; 6], [0; 6])
            .unwrap();
        let header = FrameView::parse(&frame[..length])
            .unwrap()
            .datagrams()
            .next()
            .unwrap()
            .unwrap()
            .header;
        pool.complete_match(
            RxMatch {
                slot_id: handle.index() as u16,
                generation: 9,
                working_counter: 1,
            },
            header,
            payload,
        )
        .unwrap();
        mailbox
            .accept_completed(&mut pool, handle, time + 1)
            .unwrap();
        assert_eq!(pool.in_use(), 0);
    }
    assert_ne!(mailbox.phase(), MailboxPhase::Faulted);
}

/// A simple protocol fixture for the transport test, independent of service
/// action indices. It returns EEPROM register values and six cyclic datagrams.
struct WirePort {
    now: u64,
    frames: [[u8; MAX_ETHERNET_FRAME_LEN]; 3],
    lengths: [usize; 3],
    count: usize,
    head: usize,
    eeprom_word: u16,
    control_frames: u64,
}
impl esop_ethercat_core::EthercatPort for WirePort {
    type Error = core::convert::Infallible;
    fn now_ns(&self) -> u64 {
        self.now
    }
    fn link_state(&self) -> esop_ethercat_core::LinkState {
        esop_ethercat_core::LinkState::Up
    }
    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        let slot = self.count;
        self.count += 1;
        self.frames[slot][..frame.len()].copy_from_slice(frame);
        self.lengths[slot] = frame.len();
        let mut offset = 16;
        loop {
            let header = DatagramHeader::decode(&self.frames[slot][offset..offset + 10]).unwrap();
            let start = offset + 10;
            let end = start + header.length as usize;
            if (220..=227).contains(&header.index) {
                self.control_frames += 1;
            }
            match (header.command, header.address) {
                (Command::Fpwr, 0x0504_1000) => {
                    self.eeprom_word = u16::from_le_bytes([frame[start], frame[start + 1]])
                }
                (Command::Fprd, 0x0502_1000) => self.frames[slot][start..end].fill(0),
                (Command::Fprd, 0x0508_1000) => {
                    self.frames[slot][start..end].copy_from_slice(&self.eeprom_word.to_le_bytes())
                }
                (Command::Brd, 0x092c_0000) => {
                    // Three traversed ESCs, but only one implements this register.
                    self.frames[slot][offset + 2..offset + 4].copy_from_slice(&3u16.to_le_bytes());
                    self.frames[slot][start..end].fill(0);
                }
                _ => {}
            }
            self.frames[slot][end..end + 2].copy_from_slice(&1u16.to_le_bytes());
            offset = end + 2;
            if header.last {
                break;
            }
        }
        Ok(())
    }
    fn rx_poll(
        &mut self,
        scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<esop_ethercat_core::RxPoll, Self::Error> {
        if self.head == self.count {
            self.head = 0;
            self.count = 0;
            return Ok(esop_ethercat_core::RxPoll::Empty);
        }
        let length = self.lengths[self.head];
        scratch[..length].copy_from_slice(&self.frames[self.head][..length]);
        self.head += 1;
        Ok(esop_ethercat_core::RxPoll::Frame(length))
    }
}

#[test]
fn a_thousand_sii_actions_wrap_logical_indices_without_colliding_with_domain_or_dc() {
    use esop_ethercat_core::{
        DatagramPlan, DcCyclicConfig, DcCyclicSync, DcMonitor, DcSyncWindowConfig, Domain,
        DomainSegment, EthercatMaster, FramePlanSet, MasterConfig, ScheduleDomain, ScheduleTable,
        ScheduledDomainBank, ScheduledDomainEntry, ScheduledProcessInputEntry,
        ScheduledProcessInputs, SiiBlockReader, SiiBlockRequest, SiiPhase,
    };
    let schedule = ScheduleTable::<1, 1>::build(
        1000,
        &[ScheduleDomain {
            id: 0,
            period_ticks: 1,
            phase_ticks: 0,
        }],
    )
    .unwrap();
    let mut domain = Domain::<8, 4>::new(0x1000);
    let mut plans = FramePlanSet::<1, 4>::new();
    for index in 0..4u8 {
        domain
            .add_segment(DomainSegment {
                datagram_index: index,
                input_offset: usize::from(index) * 2,
                len: 2,
                expected_wkc: 1,
            })
            .unwrap();
        plans
            .push(DatagramPlan {
                command: Command::Lrw,
                index,
                address: 0x1000 + u32::from(index) * 2,
                payload_offset: usize::from(index) * 2,
                payload_len: 2,
                expected_wkc: 1,
            })
            .unwrap();
    }
    let mut bank = ScheduledDomainBank::new(
        &schedule,
        [ScheduledDomainEntry {
            id: 0,
            domain: &mut domain,
        }],
    )
    .unwrap();
    let image = [0; 8];
    let inputs = ScheduledProcessInputs::new(
        &bank,
        &schedule,
        [ScheduledProcessInputEntry {
            id: 0,
            image: &image,
            plans: &plans,
        }],
    )
    .unwrap();
    let mut master = EthercatMaster::<3, MAX_ETHERNET_FRAME_LEN>::new_with_bus_transit_count(
        MasterConfig::new([0xff; 6], [0; 6]),
        3,
    );
    let mut dc = DcCyclicSync::new_with_bus_transit_count(
        DcCyclicConfig::new(0x1000, 240, 0),
        DcMonitor::new(50, 10, 1, 1),
        3,
    )
    .with_sync_window(DcSyncWindowConfig::new(241, 8, 1, 100, 1, 1).unwrap())
    .unwrap();
    let mut controls = ControlRequestPool::<1>::new_with_datagram_indices_excluding(
        &[220, 221, 222, 223, 224, 225, 226, 227],
        &[0, 1, 2, 3, 240, 241],
    )
    .unwrap();
    let mut reader = SiiBlockReader::<256>::new();
    reader
        .start(SiiBlockRequest {
            station_address: 0x1000,
            start_word: 0,
            word_count: 256,
            generation: 9,
            now_ns: 0,
            timeout_ns: 10_000_000,
            request_timeout_ns: 10_000,
        })
        .unwrap();
    let mut port = WirePort {
        now: 0,
        frames: [[0; MAX_ETHERNET_FRAME_LEN]; 3],
        lengths: [0; 3],
        count: 0,
        head: 0,
        eeprom_word: 0,
        control_frames: 0,
    };
    let mut actions = 0;
    let mut collided_logical_indices = 0;
    while reader.phase() != SiiPhase::Complete {
        let now = actions * 1000;
        port.now = now;
        let generation = actions as u16 + 1;
        let action = reader.next_action(now).unwrap().unwrap();
        if [0, 1, 2, 3, 240, 241].contains(&action.datagram_index) {
            collided_logical_indices += 1;
        }
        let handle = reader.enqueue_pending(&mut controls).unwrap();
        let request = controls.get(handle).unwrap();
        assert!(request.matches_action(
            action.datagram_index,
            action.generation,
            action.address,
            action.operation,
            action.payload(),
            action.datagram_len(),
            action.deadline_ns
        ));
        assert!((220..=227).contains(&request.datagram_index));
        bank.submit_due_process_inputs(
            &inputs,
            &mut master,
            &mut port,
            generation,
            now + 500,
            now + 500,
        )
        .unwrap();
        let report = bank
            .run_dc_and_control_cycle(
                &mut master,
                &mut port,
                &mut [0; MAX_ETHERNET_FRAME_LEN],
                &mut dc,
                &mut [0; 12],
                now,
                &mut controls,
                Some(handle),
                generation,
                now + 500,
                now + 500,
            )
            .unwrap();
        assert!(report.service().failure.is_none());
        assert_eq!(
            report.request_state_after_rx(),
            Some(RequestState::Complete)
        );
        assert_eq!(report.received().report.unmatched_datagrams, 0);
        assert_eq!(report.received().report.consumer_rejections, 0);
        assert!(report.received().dc_result.is_ok());
        let request = controls.get(handle).unwrap();
        reader
            .accept(
                action.token,
                request.generation,
                &request.payload()[..action.read_len as usize],
                request.actual_wkc,
                now,
            )
            .unwrap();
        controls.release(handle).unwrap();
        actions += 1;
    }
    assert_eq!(actions, 1024);
    assert!(collided_logical_indices > 10);
    assert_eq!(port.control_frames, 1024);
    assert_eq!(dc.sync_count(), 1024);
    assert_eq!(reader.words().unwrap(), &(0..256u16).collect::<Vec<_>>());
}
