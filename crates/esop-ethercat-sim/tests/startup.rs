//! The unchanged generated product must discover independently modelled ESCs.
//! Controller acceptance uses only the parsed wire response, never expected
//! WKC values or synthetic controller completions.
use esop_ethercat_core::wire::{Command, FrameBuilder, FrameView, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{
    EthercatPort, EthercatState, RegisterOperation, RxPoll, StartupConfig,
    StartupConfigurationServices, StartupController, StartupPhase,
};
use esop_ethercat_sim::{VirtualBus, fixtures};

mod generated {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/examples/sim-dual-axis/expected/esop_product_config.rs"
    ));
}

#[test]
fn generated_product_reaches_preop_configuration_barrier_through_wire() {
    let mut bus = VirtualBus::<3>::new(fixtures::dual_axis_io()).unwrap();
    let mut startup = StartupController::<3>::new(0x1001);
    generated::PRODUCT_CONFIG
        .start_startup(
            &mut startup,
            1,
            0,
            StartupConfig::new(EthercatState::Op)
                .with_configuration_services(StartupConfigurationServices::new().with_mapping()),
        )
        .unwrap();
    let mut wire_actions = 0;
    for _ in 0..20000 {
        if startup.phase() == StartupPhase::AwaitingConfiguration {
            break;
        }
        bus.advance_by(10_000).unwrap();
        let Some(action) = startup
            .next_action(bus.now_ns())
            .unwrap_or_else(|e| panic!("next {e:?}, action {wire_actions}"))
        else {
            continue;
        };
        let command = match action.operation() {
            RegisterOperation::Read => Command::Fprd,
            RegisterOperation::Write => Command::Fpwr,
            RegisterOperation::AutoIncrementRead => Command::Aprd,
            RegisterOperation::AutoIncrementWrite => Command::Apwr,
            RegisterOperation::BroadcastRead => Command::Brd,
        };
        let mut tx = [0; MAX_ETHERNET_FRAME_LEN];
        let mut payload = [0; 256];
        payload[..action.payload().len()].copy_from_slice(action.payload());
        let mut builder = FrameBuilder::new(&mut tx, [0xff; 6], [2, 0, 0, 0, 0, 1]).unwrap();
        builder
            .push(
                command,
                action.datagram_index(),
                action.address(),
                &payload[..action.datagram_len()],
            )
            .unwrap();
        let len = builder.finish().unwrap();
        bus.tx_submit(&tx[..len]).unwrap();
        let mut rx = [0; MAX_ETHERNET_FRAME_LEN];
        let RxPoll::Frame(len) = bus.rx_poll(&mut rx).unwrap() else {
            panic!("missing wire response");
        };
        let view = FrameView::parse(&rx[..len]).unwrap();
        let d = view.datagrams().next().unwrap().unwrap();
        assert!(action.address_policy().accepts(
            command as u8,
            action.address(),
            d.header.address,
            d.working_counter
        ));
        startup
            .accept(
                action,
                1,
                &d.payload[..action.response_len()],
                d.working_counter,
                bus.now_ns(),
            )
            .unwrap_or_else(|e| {
                panic!(
                    "accept {e:?}, action {wire_actions} {action:?}, bytes {:?}",
                    d.payload
                )
            });
        wire_actions += 1;
    }
    assert_eq!(startup.phase(), StartupPhase::AwaitingConfiguration);
    assert!(
        wire_actions > 1000,
        "must read actual EEPROM and register banks"
    );
    assert_eq!(startup.records().len(), 3);
    for (i, record) in startup.records().iter().enumerate() {
        assert!(record.online);
        assert_eq!(record.station_address, 0x1001 + i as u16);
        assert_eq!(bus.slave(i).unwrap().al_state, 2);
        assert_eq!(
            startup.verified_mailbox(i as u16),
            Some(generated::PRODUCT_CONFIG.slaves[i].mailbox_config)
        );
        assert!(startup.verified_sii(i as u16).is_some());
    }
    assert_eq!(startup.verified_requesting_id(0), Some(0x41));
    assert_eq!(startup.verified_requesting_id(1), Some(0x42));
    assert!(startup.verified_dc_mode(0).is_some());
}

#[test]
fn one_eight_and_thirty_two_slaves_scan_through_the_shared_rx_authority() {
    fn scan<const N: usize>() {
        use esop_ethercat_core::{
            ControlRequestPool, ControlRxConsumer, EthercatMaster, MasterConfig, ScanController,
            ScanPhase,
        };
        let mut bus = VirtualBus::<N>::new([fixtures::drive(1, 0); N]).unwrap();
        let mut controller = ScanController::<N>::new(0x1001);
        controller.start(1, 0, 1_000_000_000, 1_000_000).unwrap();
        let mut controls = ControlRequestPool::<1>::new_with_datagram_indices(&[220]).unwrap();
        let mut master = EthercatMaster::<1, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
            [255; 6],
            [2, 0, 0, 0, 0, 1],
        ));
        let mut bytes = [0; MAX_ETHERNET_FRAME_LEN];
        for _ in 0..2000 {
            if controller.phase() == ScanPhase::Complete {
                break;
            }
            bus.advance_by(10_000).unwrap();
            let now = bus.now_ns();
            let Some(action) = controller.next_action(now).unwrap() else {
                continue;
            };
            let request = controller.enqueue_pending(&mut controls).unwrap();
            let frame = master.acquire_frame(1, action.deadline_ns).unwrap();
            master
                .build_control_request(&mut controls, request, frame)
                .unwrap();
            master.submit_frame(&mut bus, frame).unwrap();
            let report = master
                .cycle_receive_with_consumer(
                    &mut bus,
                    &mut bytes,
                    1,
                    &mut ControlRxConsumer::new(&mut controls),
                )
                .unwrap();
            assert_eq!(report.wkc_mismatches, 0);
            assert_eq!(report.unmatched_datagrams, 0);
            assert_eq!(report.consumer_rejections, 0);
            let completed = controls.get(request).unwrap();
            assert!(completed.matches_action(
                action.datagram_index,
                action.generation,
                action.address,
                action.operation,
                action.payload(),
                action.datagram_len(),
                action.deadline_ns
            ));
            controller
                .accept(
                    action.token,
                    completed.generation,
                    &completed.payload()[..action.read_len as usize],
                    completed.actual_wkc,
                    now,
                )
                .unwrap();
            controls.release(request).unwrap();
        }
        assert_eq!(controller.phase(), ScanPhase::Complete);
        assert_eq!(controller.records().len(), N);
        for (position, record) in controller.records().iter().enumerate() {
            assert_eq!(record.position, position as u16);
            assert_eq!(record.station_address, 0x1001 + position as u16);
            assert!(record.online);
        }
    }
    scan::<1>();
    scan::<8>();
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(scan::<32>)
        .unwrap()
        .join()
        .unwrap();
}
