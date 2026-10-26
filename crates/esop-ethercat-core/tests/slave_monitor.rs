use esop_ethercat_core::{
    ControlRequestPool, EthercatState, RegisterOperation, SlaveMonitorActionKind,
    SlaveMonitorConfig, SlaveMonitorController, SlaveMonitorError, SlaveMonitorEventKind,
    SlaveMonitorProgress, SlaveMonitoringService,
};

fn config(position: u16) -> SlaveMonitorConfig {
    SlaveMonitorConfig {
        position,
        station_address: 0x1000 + position,
        required_al_state: EthercatState::Op,
        required_dl_mask: 0x0010,
    }
}

fn complete<const N: usize, const E: usize>(
    monitor: &mut SlaveMonitorController<N, E>,
    now: u64,
    payload: &[u8],
    wkc: u16,
) -> SlaveMonitorProgress {
    let action = monitor.next_action(now).unwrap();
    let mut pool = ControlRequestPool::<1>::new();
    let handle = pool
        .acquire_with_response_len(
            action.datagram_index,
            action.generation,
            action.address,
            RegisterOperation::Read,
            &[],
            action.length,
            action.deadline_ns,
        )
        .unwrap();
    let mut frame = [0; 64];
    pool.build_into_buffer(handle, &mut frame, [0xff; 6], [0; 6])
        .unwrap();
    let result = pool.complete(handle, action.generation, action.address, payload, wkc);
    if wkc == 1 && payload.len() == action.length {
        result.unwrap();
    } else {
        assert!(result.is_err());
    }
    let progress = monitor
        .accept_completed(pool.get(handle).unwrap(), now + 1)
        .unwrap();
    pool.release(handle).unwrap();
    progress
}

#[test]
fn publishes_only_complete_same_sample_al_dl_and_ages_the_earlier_read() {
    let mut monitor = SlaveMonitorController::<1, 4>::new(42, &[config(0)], 10, 30, 20, 0).unwrap();
    assert!(!monitor.quality(0).is_good());
    assert_eq!(monitor.quality(0).fresh_until_ns, None);
    assert_eq!(
        complete(&mut monitor, 0, &[8, 0, 0, 0, 0, 0], 1),
        SlaveMonitorProgress::AlStaged(0)
    );
    assert!(!monitor.snapshots()[0].known);
    assert!(!monitor.is_due(10));
    assert!(monitor.is_due(11));
    let dl = monitor.next_action(11).unwrap();
    assert_eq!(dl.kind, SlaveMonitorActionKind::DataLinkStatus);
    assert_eq!(dl.sample_sequence, 1);
    assert_eq!(
        complete(&mut monitor, 11, &[0x10, 0], 1),
        SlaveMonitorProgress::SamplePublished(0)
    );
    let sample = monitor.snapshots()[0];
    assert_eq!(sample.observed_at_ns, 1);
    assert_eq!(sample.completed_at_ns, 12);
    assert_ne!(sample.al_generation, sample.dl_generation);
    let held = monitor.quality(12);
    assert_eq!(held.sampled_at_ns, 12);
    assert_eq!(held.fresh_until_ns, Some(31));
    assert!(!held.is_good_at(11));
    assert!(held.is_good_at(31));
    assert!(!held.is_good_at(32));
    assert!(monitor.quality(31).is_good());
    assert!(!monitor.quality(32).is_good());
    assert!(!monitor.quality(0).is_good());
    assert_eq!(
        monitor.pop_event().unwrap().kind,
        SlaveMonitorEventKind::SamplePublished
    );
    assert_eq!(
        complete(&mut monitor, 22, &[0x18, 0, 0, 0, 0x11, 0], 1),
        SlaveMonitorProgress::AlStaged(0)
    );
    // A new bad AL blocks immediately, but the published pair is unchanged.
    assert_eq!(monitor.snapshots()[0], sample);
    assert!(!monitor.quality(23).al_good);
    assert_eq!(
        monitor.pop_event().unwrap().kind,
        SlaveMonitorEventKind::AlStateFault(esop_ethercat_core::AlStatus::new(0x18, 0x11))
    );
}

#[test]
fn quality_carries_the_earliest_expiring_station_bound() {
    let mut monitor =
        SlaveMonitorController::<2, 4>::new(42, &[config(0), config(1)], 1, 10, 20, 0).unwrap();
    complete(&mut monitor, 0, &[8, 0, 0, 0, 0, 0], 1);
    complete(&mut monitor, 2, &[0x10, 0], 1);
    complete(&mut monitor, 4, &[8, 0, 0, 0, 0, 0], 1);
    complete(&mut monitor, 6, &[0x10, 0], 1);
    let quality = monitor.quality(7);
    assert_eq!(quality.sampled_at_ns, 7);
    assert_eq!(quality.fresh_until_ns, Some(11));
    assert!(quality.is_good_at(11));
    assert!(!quality.is_good_at(12));
    assert!(!quality.is_good_at(6));
}

#[test]
fn failed_read_invalidates_station_then_advances_without_latching_wire_selection() {
    let mut monitor =
        SlaveMonitorController::<2, 4>::new(42, &[config(0), config(1)], 1, 100, 20, 0).unwrap();
    assert!(matches!(
        complete(&mut monitor, 0, &[8, 0, 0, 0, 0, 0], 0),
        SlaveMonitorProgress::StationFailed(0, SlaveMonitorError::Control(_))
    ));
    assert!(!monitor.snapshots()[0].known);
    assert_eq!(monitor.fault_count(), 1);
    let action = monitor.next_action(2).unwrap();
    assert_eq!(action.station_address, 0x1001);
    assert_eq!(action.kind, SlaveMonitorActionKind::AlStatus);
    complete(&mut monitor, 2, &[8, 0, 0, 0, 0, 0], 1);
    complete(&mut monitor, 4, &[0x10, 0], 1);
    assert!(monitor.snapshots()[1].known);
    assert!(!monitor.quality(6).known);
    assert_eq!(monitor.quality(6).fresh_until_ns, None);
    assert_eq!(monitor.next_action(6).unwrap().position, 0);
}

#[test]
fn full_events_never_block_samples_and_bad_dl_is_visible() {
    let mut monitor = SlaveMonitorController::<1, 1>::new(42, &[config(0)], 1, 100, 20, 0).unwrap();
    for time in [0, 4, 8] {
        complete(&mut monitor, time, &[8, 0, 0, 0, 0, 0], 1);
        complete(&mut monitor, time + 2, &[0, 0], 1);
    }
    assert!(monitor.snapshots()[0].known);
    assert_eq!(monitor.snapshots()[0].sample_sequence, 3);
    assert!(!monitor.quality(12).data_link_good);
    assert_eq!(monitor.lost_events(), 2);
    let event = monitor.pop_event().unwrap();
    assert_eq!(event.kind, SlaveMonitorEventKind::DataLinkFault(0));
    let action = event.action.unwrap();
    assert_eq!(action.station_address, 0x1000);
    assert_eq!(action.deadline_ns, 22);
    assert_eq!(action.kind, SlaveMonitorActionKind::DataLinkStatus);
    assert_eq!(monitor.pop_event(), None);
}

#[test]
fn malformed_or_substituted_read_cannot_publish_a_pair() {
    let mut monitor = SlaveMonitorController::<1, 0>::new(42, &[config(0)], 1, 100, 20, 0).unwrap();
    assert!(matches!(
        complete(&mut monitor, 0, &[8], 1),
        SlaveMonitorProgress::StationFailed(0, _)
    ));
    assert_eq!(monitor.lost_events(), 1);
    let action = monitor.next_action(2).unwrap();
    let mut pool = ControlRequestPool::<1>::new();
    let handle = pool
        .acquire_with_response_len(
            action.datagram_index,
            action.generation,
            action.address,
            RegisterOperation::Read,
            &[],
            action.length,
            action.deadline_ns + 1,
        )
        .unwrap();
    let mut frame = [0; 64];
    pool.build_into_buffer(handle, &mut frame, [0xff; 6], [0; 6])
        .unwrap();
    pool.complete(
        handle,
        action.generation,
        action.address,
        &[8, 0, 0, 0, 0, 0],
        1,
    )
    .unwrap();
    assert_eq!(
        monitor.accept_completed(pool.get(handle).unwrap(), 3),
        Ok(SlaveMonitorProgress::StationFailed(
            0,
            SlaveMonitorError::ActionMismatch
        ))
    );
    assert!(!monitor.snapshots()[0].known);
}

#[test]
fn configuration_is_frozen_and_validated_before_any_action() {
    assert!(matches!(
        SlaveMonitorController::<1>::new(42, &[], 1, 100, 20, 0),
        Err(SlaveMonitorError::InvalidCapacity)
    ));
    assert!(matches!(
        SlaveMonitorController::<2>::new(42, &[config(0), config(0)], 1, 100, 20, 0),
        Err(SlaveMonitorError::DuplicateStation)
    ));
    assert!(matches!(
        SlaveMonitorController::<1>::new(42, &[config(0)], 0, 100, 20, 0),
        Err(SlaveMonitorError::InvalidTiming)
    ));
}

#[test]
fn production_cycle_keeps_domain_dc_first_and_reports_stale_monitor_on_idle_ticks() {
    use esop_ethercat_core::wire::{Command, DatagramHeader, MAX_ETHERNET_FRAME_LEN};
    use esop_ethercat_core::{
        DatagramPlan, DcCyclicConfig, DcCyclicSync, DcMonitor, Domain, DomainSegment,
        EthercatMaster, EthercatPort, FramePlanSet, LinkState, MasterConfig, RxPoll,
        ScheduleDomain, ScheduleTable, ScheduledDomainBank, ScheduledDomainEntry,
        ScheduledProcessInputEntry, ScheduledProcessInputs, ScheduledProductionServiceKind,
        ScheduledProductionServiceScheduler, ScheduledProductionServices,
    };
    struct Port {
        now: u64,
        frames: [[u8; MAX_ETHERNET_FRAME_LEN]; 3],
        lengths: [usize; 3],
        count: usize,
        head: usize,
        commands: Vec<u8>,
    }
    impl EthercatPort for Port {
        type Error = core::convert::Infallible;
        fn now_ns(&self) -> u64 {
            self.now
        }
        fn link_state(&self) -> LinkState {
            LinkState::Up
        }
        fn tx_submit(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
            let slot = self.count;
            self.frames[slot][..frame.len()].copy_from_slice(frame);
            self.lengths[slot] = frame.len();
            self.count += 1;
            let mut offset = 16;
            loop {
                let header =
                    DatagramHeader::decode(&self.frames[slot][offset..offset + 10]).unwrap();
                self.commands.push(header.command as u8);
                let data = offset + 10;
                let end = data + header.length as usize;
                if header.command == Command::Fprd {
                    self.frames[slot][data..end].fill(0);
                    match header.address {
                        0x0130_1000 => self.frames[slot][data] = 8,
                        0x0110_1000 => self.frames[slot][data] = 0x10,
                        _ => panic!("unexpected monitor address"),
                    }
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
        ) -> Result<RxPoll, Self::Error> {
            if self.head == self.count {
                self.head = 0;
                self.count = 0;
                return Ok(RxPoll::Empty);
            }
            let length = self.lengths[self.head];
            scratch[..length].copy_from_slice(&self.frames[self.head][..length]);
            self.head += 1;
            Ok(RxPoll::Frame(length))
        }
    }
    let schedule = ScheduleTable::<1, 1>::build(
        10,
        &[ScheduleDomain {
            id: 0,
            period_ticks: 1,
            phase_ticks: 0,
        }],
    )
    .unwrap();
    let mut domain = Domain::<2, 1>::new(0x1000);
    domain
        .add_segment(DomainSegment {
            datagram_index: 12,
            input_offset: 0,
            len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let mut bank = ScheduledDomainBank::new(
        &schedule,
        [ScheduledDomainEntry {
            id: 0,
            domain: &mut domain,
        }],
    )
    .unwrap();
    let mut plans = FramePlanSet::<1, 1>::new();
    plans
        .push(DatagramPlan {
            command: Command::Lrw,
            index: 12,
            address: 0x1000,
            payload_offset: 0,
            payload_len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let image = [0; 2];
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
    let mut master =
        EthercatMaster::<3, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new([0xff; 6], [0; 6]));
    let mut dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x1000, 13, 0),
        DcMonitor::new(50, 10, 1, 1),
    );
    // The next sample is deliberately not due while the accepted sample ages.
    // Staleness must remain visible on real Idle cycles, independently of the
    // optional-service round-robin cursor or the presence of other services.
    let mut monitor = SlaveMonitorController::<1>::new(42, &[config(0)], 15, 25, 100, 0).unwrap();
    let mut scheduler = ScheduledProductionServiceScheduler::new();
    let mut controls = ControlRequestPool::<1>::new();
    let mut port = Port {
        now: 0,
        frames: [[0; MAX_ETHERNET_FRAME_LEN]; 3],
        lengths: [0; 3],
        count: 0,
        head: 0,
        commands: Vec::new(),
    };
    for tick in 0..4u16 {
        port.now = u64::from(tick) * 10;
        let start = port.commands.len();
        bank.submit_due_process_inputs(
            &inputs,
            &mut master,
            &mut port,
            tick + 1,
            u64::from(tick) * 10 + 5,
            u64::from(tick) * 10 + 5,
        )
        .unwrap();
        let report = scheduler
            .run_cycle(
                &mut bank,
                &mut master,
                &mut port,
                &mut [0; MAX_ETHERNET_FRAME_LEN],
                &mut dc,
                &mut [0; 8],
                u64::from(tick) * 10,
                &mut controls,
                &mut ScheduledProductionServices::<0, 0, 0>::new(None, None, None, None)
                    .with_slave_monitor(&mut monitor),
                tick + 1,
                u64::from(tick) * 10 + 5,
                u64::from(tick) * 10 + 5,
            )
            .unwrap();
        assert_eq!(
            &port.commands[start..start + 2],
            &[Command::Lrw as u8, Command::Frmw as u8]
        );
        let quality = report.slave_monitor_quality().unwrap();
        if tick == 2 {
            assert!(quality.is_good());
        }
        if tick == 3 {
            assert_eq!(report.selected(), ScheduledProductionServiceKind::Idle);
            assert!(quality.known);
            assert!(!quality.fresh);
            assert!(!quality.is_good());
        }
    }
}
