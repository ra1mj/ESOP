use esop_ethercat_core::wire::{Command, DatagramHeader, FrameView, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{
    Domain, DomainConfig, DomainDatagramSpec, DomainRegistry, DomainSegment, FramePlan,
    PdoDirection, PdoEntryHandle, PdoRegistrationRequest, SlaveCopyError, SlaveCopyPlan,
    SlaveCopyStatus,
};

type Registry = DomainRegistry<2, 4, 2>;

fn mapping() -> (Registry, PdoEntryHandle, PdoEntryHandle, PdoEntryHandle) {
    let mut registry = Registry::new();
    registry
        .register_domain(DomainConfig::new(1, 0x1000, 0, 2, 1, 0))
        .unwrap();
    registry
        .register_domain(DomainConfig::new(2, 0x2000, 2, 3, 1, 0))
        .unwrap();
    let source = registry
        .register_pdo(
            1,
            PdoRegistrationRequest::new(0, 0x6000, 1, PdoDirection::Tx, 16, false),
        )
        .unwrap();
    let target = registry
        .register_pdo(
            2,
            PdoRegistrationRequest::new(1, 0x7000, 1, PdoDirection::Rx, 16, false),
        )
        .unwrap();
    let quality = registry
        .register_pdo(
            2,
            PdoRegistrationRequest::new(1, 0x7000, 2, PdoDirection::Rx, 8, false),
        )
        .unwrap();
    registry
        .register_datagram(
            1,
            DomainDatagramSpec::input(Command::Lrd, 11, 0x1000, 0, 2, 1),
        )
        .unwrap();
    registry
        .register_datagram(
            2,
            DomainDatagramSpec::output(Command::Lwr, 12, 0x2000, 0, 3, 1),
        )
        .unwrap();
    (registry, source, target, quality)
}

fn source_domain() -> Domain<2, 1> {
    let mut source = Domain::new(0x1000);
    source
        .add_segment(DomainSegment {
            datagram_index: 11,
            input_offset: 0,
            len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    source
}

fn receive(
    domain: &mut Domain<2, 1>,
    generation: u16,
    cycle: u64,
    wkc: u16,
    data: [u8; 2],
) -> bool {
    domain.begin_receive(generation).unwrap();
    domain
        .stage_datagram(
            generation,
            DatagramHeader {
                command: Command::Lrd,
                index: 11,
                address: 0x1000,
                length: 2,
                last: true,
            },
            &data,
            wkc,
        )
        .unwrap();
    domain.finish_receive(generation, cycle).unwrap()
}

#[test]
fn copy_to_next_target_frame_carries_quality_and_degrades_on_stale_or_bad_wkc() {
    let (mut registry, source_handle, target_handle, quality_handle) = mapping();
    assert_eq!(
        SlaveCopyPlan::build(
            &registry,
            source_handle,
            target_handle,
            quality_handle,
            0xCC
        ),
        Err(SlaveCopyError::RegistryNotActive),
    );
    registry.activate::<4>(250_000).unwrap();
    let plan = SlaveCopyPlan::build(
        &registry,
        source_handle,
        target_handle,
        quality_handle,
        0xCC,
    )
    .unwrap();
    assert_eq!((plan.source_domain_id(), plan.target_domain_id()), (1, 2));
    let mut source = source_domain();
    let mut target = Domain::<3, 1>::new(0x2000);
    assert_eq!(
        plan.apply_to_domains(&source, &mut target, 1)
            .unwrap()
            .status,
        SlaveCopyStatus::InvalidSource
    );
    assert_eq!(target.output(), &[0xCC, 0xCC, 0]);
    assert!(receive(&mut source, 1, 1, 1, [0xA5, 0x5A]));

    let copied = plan.apply_to_domains(&source, &mut target, 2).unwrap();
    assert_eq!(
        (copied.status, copied.source_age_cycles),
        (SlaveCopyStatus::Valid, 1)
    );
    assert_eq!(target.output(), &[0xA5, 0x5A, 1]);
    let mut frame_plan = FramePlan::<1>::new();
    registry.append_frame_plan(2, &mut frame_plan).unwrap();
    let mut frame = [0; MAX_ETHERNET_FRAME_LEN];
    let frame_len = frame_plan
        .build(
            &mut frame,
            [0xFF; 6],
            [1, 2, 3, 4, 5, 6],
            &[
                0,
                0,
                target.output()[0],
                target.output()[1],
                target.output()[2],
            ],
        )
        .unwrap();
    let received = FrameView::parse(&frame[..frame_len]).unwrap();
    assert_eq!(
        received.datagrams().next().unwrap().unwrap().payload,
        &[0xA5, 0x5A, 1]
    );

    let stale = plan.apply_to_domains(&source, &mut target, 3).unwrap();
    assert_eq!(
        (stale.status, stale.source_age_cycles),
        (SlaveCopyStatus::StaleSource, 2)
    );
    assert_eq!(target.output(), &[0xCC, 0xCC, 0]);
    assert!(!receive(&mut source, 4, 4, 0, [0xDE, 0xAD]));
    assert_eq!(source.input(), &[0xA5, 0x5A]);
    let bad = plan.apply_to_domains(&source, &mut target, 4).unwrap();
    assert_eq!(bad.status, SlaveCopyStatus::InvalidSource);
    assert_eq!(target.output(), &[0xCC, 0xCC, 0]);
    assert!(receive(&mut source, 5, 5, 1, [0x12, 0x34]));
    assert_eq!(
        plan.apply_to_domains(&source, &mut target, 5)
            .unwrap()
            .status,
        SlaveCopyStatus::Valid
    );
    assert_eq!(target.output(), &[0x12, 0x34, 1]);
}

#[test]
fn runtime_rejects_wrong_domain_image_and_target_phase_without_partial_output() {
    let (mut registry, source_handle, target_handle, quality_handle) = mapping();
    registry.activate::<4>(250_000).unwrap();
    let plan =
        SlaveCopyPlan::build(&registry, source_handle, target_handle, quality_handle, 0).unwrap();
    let mut source = source_domain();
    assert!(receive(&mut source, 1, 1, 1, [7, 8]));
    let mut wrong = Domain::<3, 1>::new(0x3000);
    *wrong.output_mut() = [9; 3];
    assert_eq!(
        plan.apply_to_domains(&source, &mut wrong, 1),
        Err(SlaveCopyError::DomainMismatch)
    );
    assert_eq!(wrong.output(), &[9; 3]);
    let mut short = Domain::<2, 1>::new(0x2000);
    *short.output_mut() = [9; 2];
    assert_eq!(
        plan.apply_to_domains(&source, &mut short, 1),
        Err(SlaveCopyError::ImageBounds)
    );
    assert_eq!(short.output(), &[9; 2]);
    let mut target = Domain::<3, 1>::new(0x2000);
    *target.output_mut() = [9; 3];
    assert_eq!(
        plan.apply_to_domains(&source, &mut target, 0),
        Err(SlaveCopyError::TargetNotDue)
    );
    assert_eq!(target.output(), &[9; 3]);
}

#[test]
fn same_domain_can_copy_between_distinct_slaves_without_aliasing() {
    let mut registry = DomainRegistry::<1, 3, 2>::new();
    registry
        .register_domain(DomainConfig::new(5, 0x3000, 0, 5, 2, 0))
        .unwrap();
    let src = registry
        .register_pdo(
            5,
            PdoRegistrationRequest::new(0, 0x6000, 1, PdoDirection::Tx, 16, false),
        )
        .unwrap();
    let dst = registry
        .register_pdo(
            5,
            PdoRegistrationRequest::new(1, 0x7000, 1, PdoDirection::Rx, 16, false),
        )
        .unwrap();
    let flag = registry
        .register_pdo(
            5,
            PdoRegistrationRequest::new(1, 0x7000, 2, PdoDirection::Rx, 8, false),
        )
        .unwrap();
    registry
        .register_datagram(
            5,
            DomainDatagramSpec::input(Command::Lrd, 11, 0x3000, 0, 2, 1),
        )
        .unwrap();
    registry
        .register_datagram(
            5,
            DomainDatagramSpec::output(Command::Lwr, 12, 0x3002, 2, 3, 1),
        )
        .unwrap();
    registry.activate::<2>(250_000).unwrap();
    let plan = SlaveCopyPlan::build(&registry, src, dst, flag, 0xFF).unwrap();
    let mut domain = Domain::<5, 1>::new(0x3000);
    domain
        .add_segment(DomainSegment {
            datagram_index: 11,
            input_offset: 0,
            len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    domain.begin_receive(1).unwrap();
    domain
        .stage_datagram(
            1,
            DatagramHeader {
                command: Command::Lrd,
                index: 11,
                address: 0x3000,
                length: 2,
                last: true,
            },
            &[7, 8],
            1,
        )
        .unwrap();
    assert!(domain.finish_receive(1, 1).unwrap());
    assert_eq!(
        plan.apply_within_domain(&mut domain, 1).unwrap().status,
        SlaveCopyStatus::Valid
    );
    assert_eq!(domain.output(), &[0, 0, 7, 8, 1]);
    assert_eq!(
        plan.apply_within_domain(&mut domain, 2),
        Err(SlaveCopyError::TargetNotDue)
    );
    assert_eq!(domain.output(), &[0, 0, 7, 8, 1]);
    assert_eq!(
        plan.apply_within_domain(&mut domain, 3).unwrap().status,
        SlaveCopyStatus::StaleSource
    );
    assert_eq!(domain.output(), &[0, 0, 0xFF, 0xFF, 0]);
    // A successful receive on an off-phase tick is not scheduled evidence.
    domain.begin_receive(2).unwrap();
    domain
        .stage_datagram(
            2,
            DatagramHeader {
                command: Command::Lrd,
                index: 11,
                address: 0x3000,
                length: 2,
                last: true,
            },
            &[9, 10],
            1,
        )
        .unwrap();
    assert!(domain.finish_receive(2, 2).unwrap());
    assert_eq!(
        plan.apply_within_domain(&mut domain, 3).unwrap().status,
        SlaveCopyStatus::InvalidSource
    );
    assert_eq!(domain.output(), &[0, 0, 0xFF, 0xFF, 0]);
}

#[test]
fn static_plan_rejects_mismapped_fields_and_unroutable_outputs() {
    let (registry, source, target, quality) = mapping();
    let mut wrong_direction = registry;
    wrong_direction.activate::<4>(250_000).unwrap();
    assert_eq!(
        SlaveCopyPlan::build(&wrong_direction, source, target, source, 0),
        Err(SlaveCopyError::WrongDirection)
    );
    assert_eq!(
        SlaveCopyPlan::build(&wrong_direction, target, quality, quality, 0),
        Err(SlaveCopyError::SameSlave)
    );

    let mut unreadable = Registry::new();
    unreadable
        .register_domain(DomainConfig::new(1, 0x1000, 0, 2, 1, 0))
        .unwrap();
    unreadable
        .register_domain(DomainConfig::new(2, 0x2000, 2, 3, 1, 0))
        .unwrap();
    let source = unreadable
        .register_pdo(
            1,
            PdoRegistrationRequest::new(0, 0x6000, 1, PdoDirection::Tx, 16, false),
        )
        .unwrap();
    let target = unreadable
        .register_pdo(
            2,
            PdoRegistrationRequest::new(1, 0x7000, 1, PdoDirection::Rx, 16, false),
        )
        .unwrap();
    let quality = unreadable
        .register_pdo(
            2,
            PdoRegistrationRequest::new(1, 0x7000, 2, PdoDirection::Rx, 8, false),
        )
        .unwrap();
    unreadable
        .register_datagram(
            1,
            DomainDatagramSpec::input(Command::Lrd, 11, 0x1000, 0, 2, 1),
        )
        .unwrap();
    // A read-only datagram cannot send target RxPDOs even when it covers them.
    unreadable
        .register_datagram(
            2,
            DomainDatagramSpec::input(Command::Lrd, 12, 0x2000, 0, 3, 1),
        )
        .unwrap();
    unreadable.activate::<4>(250_000).unwrap();
    assert_eq!(
        SlaveCopyPlan::build(&unreadable, source, target, quality, 0),
        Err(SlaveCopyError::MissingDatagram)
    );
}
