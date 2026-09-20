use esop_ethercat_core::wire::{Command, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{
    DatagramPlan, Domain, DomainConfig, DomainDatagramSpec, DomainRegistry, DomainSegment,
    EthercatMaster, FramePlan, MasterConfig, PdoDirection, PdoRegistrationRequest, SlaveCopyPlan,
    SlaveCopyStatus,
};
use esop_ethercat_linux_port::SimulatedPort;

#[test]
fn master_receives_source_then_sends_qualified_target_and_safe_fallback() {
    let mut registry = DomainRegistry::<2, 2, 1>::new();
    registry
        .register_domain(DomainConfig::new(3, 0x1000, 0, 2, 1, 0))
        .unwrap();
    registry
        .register_domain(DomainConfig::new(7, 0x2000, 2, 3, 1, 0))
        .unwrap();
    let src = registry
        .register_pdo(
            3,
            PdoRegistrationRequest::new(0, 0x6000, 1, PdoDirection::Tx, 16, false),
        )
        .unwrap();
    let dst = registry
        .register_pdo(
            7,
            PdoRegistrationRequest::new(1, 0x7000, 1, PdoDirection::Rx, 16, false),
        )
        .unwrap();
    let quality = registry
        .register_pdo(
            7,
            PdoRegistrationRequest::new(1, 0x7000, 2, PdoDirection::Rx, 8, false),
        )
        .unwrap();
    registry
        .register_datagram(
            3,
            DomainDatagramSpec::input(Command::Lrd, 11, 0x1000, 0, 2, 1),
        )
        .unwrap();
    registry
        .register_datagram(
            7,
            DomainDatagramSpec::output(Command::Lwr, 12, 0x2000, 0, 3, 1),
        )
        .unwrap();
    registry.activate::<2>(250_000).unwrap();
    let copy = SlaveCopyPlan::build(&registry, src, dst, quality, 0).unwrap();
    let mut source_plan = FramePlan::<1>::new();
    let mut target_plan = FramePlan::<1>::new();
    source_plan
        .push(DatagramPlan {
            command: Command::Lrd,
            index: 11,
            address: 0x1000,
            payload_offset: 0,
            payload_len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    target_plan
        .push(DatagramPlan {
            command: Command::Lwr,
            index: 12,
            address: 0x2000,
            payload_offset: 2,
            payload_len: 3,
            expected_wkc: 1,
        })
        .unwrap();
    let mut source = Domain::<2, 1>::new(0x1000);
    source
        .add_segment(DomainSegment {
            datagram_index: 11,
            input_offset: 0,
            len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let mut target = Domain::<3, 1>::new(0x2000);
    let mut master = EthercatMaster::<2, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut port = SimulatedPort::new(1);
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut image = [0; 5];
    image[..2].copy_from_slice(&[0x34, 0x12]);

    source.begin_receive(1).unwrap();
    let tx = master.acquire_frame(1, 100_000).unwrap();
    master
        .build_and_arm_frame_from_plan(tx, &source_plan, &image)
        .unwrap();
    master.submit_frame(&mut port, tx).unwrap();
    let received = master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 1, &mut source)
        .unwrap();
    assert_eq!(received.cycle, 1);
    assert!(source.finish_receive(1, received.cycle).unwrap());
    assert_eq!(
        copy.apply_to_domains(&source, &mut target, 2)
            .unwrap()
            .status,
        SlaveCopyStatus::Valid
    );
    assert_eq!(target.output(), &[0x34, 0x12, 1]);

    image[2..].copy_from_slice(target.output());
    let tx = master.acquire_frame(2, 100_000).unwrap();
    master
        .build_and_arm_frame_from_plan(tx, &target_plan, &image)
        .unwrap();
    master.submit_frame(&mut port, tx).unwrap();
    let response = master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 2, &mut ())
        .unwrap();
    assert_eq!((response.cycle, response.wkc_mismatches), (2, 0));

    port.set_response_wkc(0);
    source.begin_receive(3).unwrap();
    let tx = master.acquire_frame(3, 100_000).unwrap();
    master
        .build_and_arm_frame_from_plan(tx, &source_plan, &image)
        .unwrap();
    master.submit_frame(&mut port, tx).unwrap();
    let bad = master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 3, &mut source)
        .unwrap();
    assert_eq!(bad.wkc_mismatches, 1);
    assert!(!source.finish_receive(3, bad.cycle).unwrap());
    assert_eq!(source.input(), &[0x34, 0x12]);
    assert_eq!(
        copy.apply_to_domains(&source, &mut target, 4)
            .unwrap()
            .status,
        SlaveCopyStatus::InvalidSource
    );
    assert_eq!(target.output(), &[0, 0, 0]);
    image[2..].copy_from_slice(target.output());
    let tx = master.acquire_frame(4, 100_000).unwrap();
    master
        .build_and_arm_frame_from_plan(tx, &target_plan, &image)
        .unwrap();
    master.submit_frame(&mut port, tx).unwrap();
    let response = master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 4, &mut ())
        .unwrap();
    assert_eq!(response.cycle, 4);
}
