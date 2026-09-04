use esop_ethercat_core::wire::{Command, FrameView, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{
    Domain, DomainConfig, DomainDatagramSpec, DomainRegistry, DomainRegistryError, DomainSegment,
    FramePlan, FramePlanSet, PdoDirection, PdoRegistrationRequest, SII_CATEGORY_END,
    SII_CATEGORY_RX_PDO, SII_CATEGORY_SYNC_MANAGER, SII_CATEGORY_TX_PDO, SiiConfigurationCandidate,
    SiiSegmentDatagramSpec,
};

fn append_category(bytes: &mut Vec<u8>, kind: u16, data: &[u8]) {
    bytes.extend_from_slice(&kind.to_le_bytes());
    bytes.extend_from_slice(&((data.len() / 2) as u16).to_le_bytes());
    bytes.extend_from_slice(data);
}

fn one_entry_pdo(index: u16, sync_manager: u8, object_index: u16) -> [u8; 16] {
    one_entry_pdo_with_bits(index, sync_manager, object_index, 16)
}

fn one_entry_pdo_with_bits(
    index: u16,
    sync_manager: u8,
    object_index: u16,
    bit_length: u8,
) -> [u8; 16] {
    let mut pdo = [0u8; 16];
    pdo[0..2].copy_from_slice(&index.to_le_bytes());
    pdo[2] = 1;
    pdo[3] = sync_manager;
    pdo[8..10].copy_from_slice(&object_index.to_le_bytes());
    pdo[12] = bit_length;
    pdo
}

#[test]
fn public_registry_flow_feeds_domain_and_frame_plan() {
    let mut registry = DomainRegistry::<2, 4, 2>::new();
    registry
        .register_domain(DomainConfig::new(0, 0x1000, 2, 6, 1, 0))
        .unwrap();
    let command = registry
        .register_pdo(
            0,
            PdoRegistrationRequest::new(3, 0x6040, 0, PdoDirection::Rx, 16, false),
        )
        .unwrap();
    assert_eq!(registry.pdo(command).unwrap().entry.bit_offset, 0);

    registry
        .register_datagram(
            0,
            DomainDatagramSpec::output(Command::Lrw, 11, 0x1000, 0, 2, 1),
        )
        .unwrap();
    registry
        .register_datagram(
            0,
            DomainDatagramSpec::input(Command::Lrd, 12, 0x1002, 2, 2, 1),
        )
        .unwrap();

    let mut segments = [DomainSegment::EMPTY; 2];
    assert_eq!(registry.copy_domain_segments(0, &mut segments), Ok(1));
    let mut domain = Domain::<6, 2>::new(0x1000);
    domain.add_segment(segments[0]).unwrap();
    domain.begin_receive(5).unwrap();

    let mut plan = FramePlan::<2>::new();
    registry.append_frame_plan(0, &mut plan).unwrap();
    let mut frame = [0u8; MAX_ETHERNET_FRAME_LEN];
    let image = [0, 0, 0xA0, 0xA1, 0xB0, 0xB1];
    let length = plan
        .build(&mut frame, [0xFF; 6], [1, 2, 3, 4, 5, 6], &image)
        .unwrap();
    let view = FrameView::parse(&frame[..length]).unwrap();
    let datagrams: [_; 2] = [
        view.datagrams().next().unwrap().unwrap(),
        view.datagrams().nth(1).unwrap().unwrap(),
    ];
    assert_eq!(datagrams[0].payload, &[0xA0, 0xA1]);
    assert_eq!(datagrams[1].payload, &[0xB0, 0xB1]);

    domain
        .stage_datagram(5, datagrams[1].header, datagrams[1].payload, 1)
        .unwrap();
    assert!(domain.finish_receive(5, 1).unwrap());
    assert_eq!(domain.input(), &[0, 0, 0xB0, 0xB1, 0, 0]);
}

#[test]
fn public_sii_candidate_registers_rx_and_tx_in_one_domain_transaction() {
    let mut bytes = Vec::new();
    append_category(
        &mut bytes,
        SII_CATEGORY_SYNC_MANAGER,
        &[
            0x00, 0x10, 0x02, 0x00, 0x26, 0x64, 0x01, 0x00, 0x00, 0x11, 0x02, 0x00, 0x26, 0x64,
            0x01, 0x00,
        ],
    );
    append_category(
        &mut bytes,
        SII_CATEGORY_RX_PDO,
        &one_entry_pdo(0x1600, 0, 0x6040),
    );
    append_category(
        &mut bytes,
        SII_CATEGORY_TX_PDO,
        &one_entry_pdo(0x1A00, 1, 0x6041),
    );
    bytes.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());

    let mut candidate = SiiConfigurationCandidate::<2, 2, 1, 1>::new();
    assert_eq!(candidate.apply_bytes(&bytes), Ok(3));
    candidate.allocate_fmmus(0x3000).unwrap();

    let mut registry = DomainRegistry::<2, 4, 2>::new();
    let registration = registry
        .register_sii_candidate(DomainConfig::new(4, 0x3000, 8, 4, 1, 0), 7, &candidate)
        .unwrap();
    assert_eq!(registration.domain.config.id, 4);
    assert_eq!(registration.process_image_len, 4);
    assert_eq!(registration.rx_pdo_count, 1);
    assert_eq!(registration.tx_pdo_count, 1);
    assert_eq!(registration.segment_count, 2);
    assert_eq!(registration.mapping.fmmu_count, 2);

    let pdos = registry.pdos(4).unwrap();
    assert_eq!(pdos[0].entry.index, 0x6040);
    assert_eq!(pdos[0].entry.bit_offset, 0);
    assert_eq!(pdos[1].entry.index, 0x6041);
    assert_eq!(pdos[1].entry.bit_offset, 16);
    assert_eq!(pdos[0].slave_position, 7);
    assert_eq!(pdos[1].slave_position, 7);
}

#[test]
fn public_sii_registration_rejects_mismatched_image_without_partial_domain() {
    let mut bytes = Vec::new();
    append_category(
        &mut bytes,
        SII_CATEGORY_SYNC_MANAGER,
        &[0x00, 0x10, 0x02, 0x00, 0x26, 0x64, 0x01, 0x00],
    );
    append_category(
        &mut bytes,
        SII_CATEGORY_RX_PDO,
        &one_entry_pdo(0x1600, 0, 0x6040),
    );
    bytes.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());

    let mut candidate = SiiConfigurationCandidate::<1, 1, 1, 1>::new();
    candidate.apply_bytes(&bytes).unwrap();
    candidate.allocate_fmmus(0x4000).unwrap();

    let mut registry = DomainRegistry::<1, 1, 1>::new();
    assert_eq!(
        registry.register_sii_candidate(DomainConfig::new(0, 0x4000, 0, 1, 1, 0), 1, &candidate,),
        Err(DomainRegistryError::SiiProcessImageTooSmall)
    );
    assert_eq!(registry.domain_count(), 0);
    assert_eq!(registry.pdos(0), Err(DomainRegistryError::UnknownDomain));
}

#[test]
fn public_sii_candidate_generates_byte_aligned_segment_datagrams() {
    let mut bytes = Vec::new();
    append_category(
        &mut bytes,
        SII_CATEGORY_SYNC_MANAGER,
        &[
            0x00, 0x10, 0x02, 0x00, 0x26, 0x64, 0x01, 0x00, 0x00, 0x11, 0x02, 0x00, 0x26, 0x64,
            0x01, 0x00,
        ],
    );
    append_category(
        &mut bytes,
        SII_CATEGORY_RX_PDO,
        &one_entry_pdo(0x1600, 0, 0x6040),
    );
    append_category(
        &mut bytes,
        SII_CATEGORY_TX_PDO,
        &one_entry_pdo(0x1A00, 1, 0x6041),
    );
    bytes.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());

    let mut candidate = SiiConfigurationCandidate::<2, 2, 1, 1>::new();
    candidate.apply_bytes(&bytes).unwrap();
    candidate.allocate_fmmus(0x3000).unwrap();

    let mut registry = DomainRegistry::<1, 4, 2>::new();
    let registration = registry
        .register_sii_candidate_with_datagrams(
            DomainConfig::new(2, 0x3000, 8, 4, 1, 0),
            7,
            &candidate,
            &[
                SiiSegmentDatagramSpec::new(21, 1),
                SiiSegmentDatagramSpec::new(22, 1),
            ],
        )
        .unwrap();

    assert_eq!(registration.domain.datagram_count, 2);
    let datagrams = registry.datagrams(2).unwrap();
    assert_eq!(datagrams[0].plan.command, Command::Lwr);
    assert!(!datagrams[0].input);
    assert_eq!(datagrams[0].plan.index, 21);
    assert_eq!(datagrams[0].plan.address, 0x3000);
    assert_eq!(datagrams[0].plan.payload_offset, 8);
    assert_eq!(datagrams[0].plan.payload_len, 2);
    assert_eq!(datagrams[1].plan.command, Command::Lrd);
    assert!(datagrams[1].input);
    assert_eq!(datagrams[1].plan.index, 22);
    assert_eq!(datagrams[1].plan.address, 0x3002);
    assert_eq!(datagrams[1].plan.payload_offset, 10);
    assert_eq!(datagrams[1].plan.payload_len, 2);

    let mut segments = [DomainSegment::EMPTY; 1];
    assert_eq!(registry.copy_domain_segments(2, &mut segments), Ok(1));
    assert_eq!(segments[0].datagram_index, 22);
    assert_eq!(segments[0].input_offset, 2);
    assert_eq!(segments[0].len, 2);
}

#[test]
fn public_sii_segment_datagram_binding_rejects_bit_packed_segments_atomically() {
    let mut bytes = Vec::new();
    append_category(
        &mut bytes,
        SII_CATEGORY_SYNC_MANAGER,
        &[0x00, 0x10, 0x01, 0x00, 0x26, 0x64, 0x01, 0x00],
    );
    append_category(
        &mut bytes,
        SII_CATEGORY_RX_PDO,
        &one_entry_pdo_with_bits(0x1600, 0, 0x6040, 3),
    );
    bytes.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());

    let mut candidate = SiiConfigurationCandidate::<1, 1, 1, 1>::new();
    candidate.apply_bytes(&bytes).unwrap();
    candidate.allocate_fmmus(0x4000).unwrap();

    let mut registry = DomainRegistry::<1, 1, 1>::new();
    assert_eq!(
        registry.register_sii_candidate_with_datagrams(
            DomainConfig::new(3, 0x4000, 0, 1, 1, 0),
            1,
            &candidate,
            &[SiiSegmentDatagramSpec::new(31, 1)],
        ),
        Err(DomainRegistryError::SiiSegmentNotByteAddressable)
    );
    assert_eq!(registry.domain_count(), 0);
}

#[test]
fn public_activation_publishes_split_frame_plans_atomically() {
    let mut registry = DomainRegistry::<1, 1, 2>::new();
    registry
        .register_domain(DomainConfig::new(0, 0x5000, 0, 1500, 1, 0))
        .unwrap();
    registry
        .register_datagram(
            0,
            DomainDatagramSpec::output(Command::Lwr, 41, 0x5000, 0, 750, 1),
        )
        .unwrap();
    registry
        .register_datagram(
            0,
            DomainDatagramSpec::input(Command::Lrd, 42, 0x52EE, 750, 750, 1),
        )
        .unwrap();

    let mut plans = [FramePlanSet::<2, 2>::new()];
    registry
        .activate_with_frame_plans::<2, 2, 2>(1_000, &mut plans)
        .unwrap();
    assert!(registry.is_active());
    assert_eq!(plans[0].frame_count(), 2);
    assert_eq!(plans[0].datagram_count(), 2);
    assert_eq!(plans[0].plan(0).unwrap().datagrams()[0].index, 41);
    assert_eq!(plans[0].plan(1).unwrap().datagrams()[0].index, 42);
}

#[test]
fn public_activation_frame_capacity_failure_leaves_registry_and_plans_unchanged() {
    let mut registry = DomainRegistry::<1, 1, 2>::new();
    registry
        .register_domain(DomainConfig::new(0, 0x6000, 0, 2, 1, 0))
        .unwrap();
    registry
        .register_datagram(
            0,
            DomainDatagramSpec::output(Command::Lwr, 51, 0x6000, 0, 1, 1),
        )
        .unwrap();
    registry
        .register_datagram(
            0,
            DomainDatagramSpec::input(Command::Lrd, 52, 0x6001, 1, 1, 1),
        )
        .unwrap();

    let mut plans = [FramePlanSet::<1, 1>::new()];
    let original_plans = plans;
    assert!(matches!(
        registry.activate_with_frame_plans::<2, 1, 1>(1_000, &mut plans),
        Err(DomainRegistryError::FramePlans(
            esop_ethercat_core::FramePlanSetError::CapacityExceeded,
        ))
    ));
    assert!(!registry.is_active());
    assert_eq!(plans, original_plans);
}
