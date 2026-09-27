use esop_ethercat_core::wire::{Command, DatagramHeader, FrameView, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{
    Domain, DomainSegment, FMMU_IMAGE_LEN, MappingConfigController, MappingConfigItem,
    MappingConfigPhase, MappingConfigProgress, MappingTable, SlaveCopyStatus,
};
use esop_product_config::{
    ActivatedProduct, AlTransitionTimeouts, CanopenDataType, Cia402AxisCommandPolicyError,
    DomainRegistryError, ETG1020_DEFAULT_TRANSITION_TIMEOUTS_V1, EscWatchdogConfig,
    FramePlanSetError, MAX_PRODUCT_SLAVE_COPIES, MailboxConfig, MailboxConfigError,
    MailboxDirection, MailboxMappedStatusError, MailboxReceiveSyncManager, MailboxStatusBit,
    OperatingMode, PdoConfigBatchPlanError, PdoConfigPlanError, PdoSdoWrite, ProcBuf,
    ProcBufHeaderError, ProductActivationError, ProductMailboxBinding, ProductMailboxPolicyError,
    ProductMailboxStatusMappingError, ProductPdoBatchError, ProductPdoPlanError, ProductSlaveKind,
    ProductStartupError, SdoAccessPolicy, SiiConfigurationSignatureError, SiiFmmuUsage,
    SlaveCopyPlanSetError, SlaveRecord, StartupDcRequirement,
};

mod generated {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/examples/sim-dual-axis/expected/esop_product_config.rs"
    ));
}

const BOOT_ID: u64 = 0x2026_0926;

type ActiveProduct = ActivatedProduct<2, 14, 2, 4, 2, 16, 2>;

fn observed_slaves() -> [SlaveRecord; 3] {
    generated::PRODUCT_CONFIG.slaves.map(|slave| SlaveRecord {
        position: slave.position,
        station_address: slave.station_address,
        identity: slave.identity,
        online: true,
        configured: true,
        last_seen_cycle: 1,
        ..SlaveRecord::EMPTY
    })
}

fn activate(
    config: &esop_product_config::StaticProductConfig<'_, 3, 2, 2>,
    observed: &[SlaveRecord],
    procbuf: &ProcBuf<2, 16, 2, 64>,
) -> Result<ActiveProduct, ProductActivationError> {
    config.activate::<16, 64, 14, 2, 4, 2, 16>(
        generated::PRODUCT_CONFIG.metadata.config_sha256,
        observed,
        procbuf,
        BOOT_ID,
    )
}

#[test]
fn checked_in_product_activates_exact_generated_evidence() {
    let procbuf =
        ProcBuf::<2, 16, 2, 64>::new(generated::PRODUCT_CONFIG.metadata.robot_id, BOOT_ID);
    let active = activate(&generated::PRODUCT_CONFIG, &observed_slaves(), &procbuf).unwrap();

    assert_eq!(
        active.metadata().config_sha256,
        [
            0x82, 0x47, 0x95, 0xdd, 0xcc, 0x1d, 0x22, 0x88, 0x0a, 0x6e, 0x68, 0x1a, 0xb4, 0x34,
            0x7c, 0xa4, 0x17, 0xed, 0xb9, 0xe9, 0x1f, 0x15, 0x19, 0x10, 0x58, 0x9b, 0x79, 0x93,
            0x19, 0x3b, 0x7e, 0x80,
        ]
    );
    assert_eq!(
        generated::PRODUCT_CONFIG.sdo_access_policy(0),
        Ok(SdoAccessPolicy::new(true))
    );
    assert_eq!(
        generated::PRODUCT_CONFIG.sdo_access_policy(1),
        Ok(SdoAccessPolicy::new(false))
    );
    assert_eq!(
        generated::PRODUCT_CONFIG.sdo_access_policy(2),
        Ok(SdoAccessPolicy::new(false))
    );
    let sdo_information = generated::PRODUCT_CONFIG.sdo_information_plan(0).unwrap();
    assert!(sdo_information.policy.enabled());
    assert_eq!(sdo_information.expectations.len(), 7);
    assert_eq!(sdo_information.expectations[0].index, 0x603f);
    assert_eq!(
        sdo_information.expectations[0].data_type,
        CanopenDataType::Unsigned16
    );
    assert_eq!(sdo_information.expectations[6].index, 0x607a);
    assert!(
        !generated::PRODUCT_CONFIG
            .sdo_information_plan(1)
            .unwrap()
            .policy
            .enabled()
    );
    assert!(
        generated::PRODUCT_CONFIG
            .sdo_information_plan(1)
            .unwrap()
            .expectations
            .is_empty()
    );

    let startup_profiles = generated::PRODUCT_CONFIG.startup_profiles().unwrap();
    assert_eq!(startup_profiles[0].expected_requesting_id, Some(0x0041));
    assert_eq!(startup_profiles[1].expected_requesting_id, Some(0x0042));
    assert_eq!(startup_profiles[2].expected_requesting_id, None);
    let drive_timeouts =
        AlTransitionTimeouts::new(3_500_000_000, 12_000_000_000, 5_500_000_000, 250_000_000);
    for profile in &startup_profiles[..2] {
        assert_eq!(profile.transition_timeouts, drive_timeouts);
        assert_eq!(profile.op_only_outputs.mask(), 1 << 2);
        assert_eq!(profile.op_only_outputs.activation_template(2), Some(0x09));
    }
    assert_eq!(
        startup_profiles[2].transition_timeouts,
        ETG1020_DEFAULT_TRANSITION_TIMEOUTS_V1
    );
    assert!(startup_profiles[2].op_only_outputs.is_empty());
    assert_eq!(
        startup_profiles[0].dc_requirement,
        StartupDcRequirement::ReferenceClock
    );
    assert_eq!(
        startup_profiles[1].dc_requirement,
        StartupDcRequirement::SystemTime
    );
    assert_eq!(
        startup_profiles[2].dc_requirement,
        StartupDcRequirement::None
    );
    assert_eq!(
        startup_profiles[0].expected_dc_mode,
        generated::PRODUCT_CONFIG.slaves[0].sii_dc_mode
    );
    assert_eq!(
        startup_profiles[1].expected_dc_mode,
        generated::PRODUCT_CONFIG.slaves[1].sii_dc_mode
    );
    assert_eq!(startup_profiles[2].expected_dc_mode, None);
    for (profile, slave) in startup_profiles
        .iter()
        .zip(generated::PRODUCT_CONFIG.slaves)
    {
        assert_eq!(slave.mailbox_send_sync_manager, 0);
        assert_eq!(slave.mailbox_send_control_byte, 0x26);
        assert_eq!(profile.expected_mailbox, Some(slave.mailbox_config));
        assert_eq!(
            profile.expected_mailbox_receive_sync_manager,
            Some(slave.mailbox_receive_sync_manager)
        );
        let expected_sii = profile.expected_sii.expect("generated SII expectation");
        assert_eq!(
            expected_sii.sync_manager_count(),
            slave.sii_sync_manager_count
        );
        assert_eq!(
            expected_sii.enabled_sync_managers(),
            slave.sii_enabled_sync_managers
        );
        assert_eq!(
            expected_sii.op_only_sync_managers(),
            slave.op_only_outputs.mask()
        );
        assert_eq!(expected_sii.fmmu_count(), slave.sii_fmmu_count);
        assert_eq!(slave.sii_fmmu_count, if slave.position < 2 { 3 } else { 2 });
        assert_eq!(slave.sii_fmmu_usages[0], SiiFmmuUsage::Outputs);
        assert_eq!(slave.sii_fmmu_usages[1], SiiFmmuUsage::Inputs);
        assert_eq!(
            slave.mailbox_config.status_bit,
            (slave.position < 2).then(|| MailboxStatusBit::sync_manager_mailbox_full(1))
        );
    }

    assert_eq!(active.registry().domain_count(), 2);
    assert_eq!(active.registry().domain(0).unwrap().pdo_count, 14);
    assert_eq!(active.registry().domain(0).unwrap().expected_wkc, 6);
    assert_eq!(active.registry().domain(1).unwrap().pdo_count, 4);
    assert_eq!(active.registry().domain(1).unwrap().expected_wkc, 2);
    assert_eq!(active.schedule().hyperperiod_ticks(), 4);
    assert_eq!(active.schedule().due_mask(0), 0b11);
    assert_eq!(active.schedule().due_mask(1), 0b01);
    assert_eq!(active.frame_plans().len(), 2);
    assert_eq!(active.frame_plans()[0].datagram_count(), 2);
    assert_eq!(active.frame_plans()[1].datagram_count(), 2);
    assert_eq!(active.slave_copy_plans().len(), 1);
    assert_eq!(active.slave_copy_plans().plans()[0].payload_len(), 4);
    assert_eq!(active.mapped_mailbox_status_count(), 2);
    assert_eq!(
        active.mapped_mailbox_status(0),
        generated::PRODUCT_CONFIG.slaves[0].mapped_mailbox_status
    );
    assert_eq!(active.mapped_mailbox_status(2), None);
    assert_eq!(active.axis_modes(), &[OperatingMode::Csp; 2]);
    for map in active.axis_pdo_maps() {
        map.validate_for(OperatingMode::Csp).unwrap();
    }
}

fn receive_motion_position(
    domain: &mut Domain<33, 1>,
    generation: u16,
    cycle: u64,
    wkc: u16,
    position: i32,
) -> bool {
    let mut payload = [0u8; 19];
    payload[5..9].copy_from_slice(&position.to_le_bytes());
    domain.begin_receive(generation).unwrap();
    domain
        .stage_datagram(
            generation,
            DatagramHeader {
                command: Command::Lrd,
                index: 1,
                address: 0x100E,
                length: payload.len() as u16,
                last: true,
            },
            &payload,
            wkc,
        )
        .unwrap();
    domain.finish_receive(generation, cycle).unwrap()
}

#[test]
fn generated_slave_copy_runs_from_verified_receive_into_the_due_target_frame() {
    let procbuf =
        ProcBuf::<2, 16, 2, 64>::new(generated::PRODUCT_CONFIG.metadata.robot_id, BOOT_ID);
    let active = activate(&generated::PRODUCT_CONFIG, &observed_slaves(), &procbuf).unwrap();
    let plan = active.slave_copy_plans().plans()[0];

    let mut source = Domain::<33, 1>::new(0x1000);
    source
        .add_segment(DomainSegment {
            datagram_index: 1,
            input_offset: 14,
            len: 19,
            expected_wkc: 4,
        })
        .unwrap();
    let mut target = Domain::<9, 1>::new(0x1100);

    assert!(receive_motion_position(&mut source, 1, 1, 4, 0x1234_5678));
    let copied = plan.apply_to_domains(&source, &mut target, 1).unwrap();
    assert_eq!(copied.status, SlaveCopyStatus::Valid);
    assert_eq!(target.output(), &[0, 0, 0x78, 0x56, 0x34, 0x12, 1, 0, 0]);

    let mut process_image = [0u8; 73];
    process_image[64..73].copy_from_slice(target.output());
    let mut frame = [0u8; MAX_ETHERNET_FRAME_LEN];
    let frame_plan = &active.frame_plans()[1].plans()[0];
    let frame_len = frame_plan
        .build(&mut frame, [0xFF; 6], [1, 2, 3, 4, 5, 6], &process_image)
        .unwrap();
    let parsed = FrameView::parse(&frame[..frame_len]).unwrap();
    let first = parsed.datagrams().next().unwrap().unwrap();
    assert_eq!(first.payload, &[0, 0, 0x78, 0x56, 0x34, 0x12, 1]);

    let stale = plan.apply_to_domains(&source, &mut target, 5).unwrap();
    assert_eq!(stale.status, SlaveCopyStatus::StaleSource);
    assert_eq!(target.output(), &[0; 9]);

    assert!(receive_motion_position(&mut source, 2, 5, 4, -123));
    assert_eq!(
        plan.apply_to_domains(&source, &mut target, 5)
            .unwrap()
            .status,
        SlaveCopyStatus::Valid
    );
    assert!(!receive_motion_position(&mut source, 3, 9, 0, 999));
    assert_eq!(
        plan.apply_to_domains(&source, &mut target, 9)
            .unwrap()
            .status,
        SlaveCopyStatus::InvalidSource
    );
    assert_eq!(target.output(), &[0; 9]);
}

#[test]
fn runtime_revalidates_generated_slave_copy_indices_and_capacity() {
    let procbuf =
        ProcBuf::<2, 16, 2, 64>::new(generated::PRODUCT_CONFIG.metadata.robot_id, BOOT_ID);
    let mut bad_index = generated::PRODUCT_CONFIG;
    let mut bad_copy = [bad_index.slave_copies[0]];
    bad_copy[0].source_pdo_index = usize::MAX;
    bad_index.slave_copies = &bad_copy;
    assert!(matches!(
        activate(&bad_index, &observed_slaves(), &procbuf),
        Err(ProductActivationError::SlaveCopyPdoIndexOutOfRange {
            copy_index: 0,
            pdo_index: usize::MAX,
        })
    ));

    let mut too_many = generated::PRODUCT_CONFIG;
    let copies = [too_many.slave_copies[0]; MAX_PRODUCT_SLAVE_COPIES + 1];
    too_many.slave_copies = &copies;
    assert!(matches!(
        activate(&too_many, &observed_slaves(), &procbuf),
        Err(ProductActivationError::SlaveCopySet {
            copy_index: MAX_PRODUCT_SLAVE_COPIES,
            error: SlaveCopyPlanSetError::CapacityExceeded,
        })
    ));
}

#[test]
fn checked_in_product_builds_ordered_watchdog_plan() {
    let plan = generated::PRODUCT_CONFIG.watchdog_plan().unwrap();
    assert_eq!(plan.entries().len(), 2);
    assert_eq!(plan.entries()[0].position, 0);
    assert_eq!(plan.entries()[0].station_address, 0x1001);
    assert_eq!(
        plan.entries()[0].config,
        EscWatchdogConfig::new(Some(2500), Some(100))
    );
    assert_eq!(plan.entries()[1].position, 1);
    assert_eq!(plan.entries()[1].station_address, 0x1002);
    assert_eq!(plan.entries()[1].config, plan.entries()[0].config);
    assert_eq!(generated::PRODUCT_CONFIG.slaves[2].watchdog, None);
}

#[test]
fn checked_in_product_rejects_tampered_fmmu_usage_profiles() {
    let mut wrong_direction = generated::PRODUCT_CONFIG;
    wrong_direction.slaves[0].sii_fmmu_usages[0] = SiiFmmuUsage::Inputs;
    assert_eq!(
        wrong_direction.startup_profiles(),
        Err(ProductStartupError::FmmuUsageMismatch {
            position: 0,
            index: 0,
            expected: SiiFmmuUsage::Outputs,
            actual: Some(SiiFmmuUsage::Inputs),
        })
    );

    let mut missing_tx = generated::PRODUCT_CONFIG;
    missing_tx.slaves[0].sii_fmmu_count = 1;
    assert_eq!(
        missing_tx.startup_profiles(),
        Err(ProductStartupError::FmmuUsageMismatch {
            position: 0,
            index: 1,
            expected: SiiFmmuUsage::Inputs,
            actual: None,
        })
    );
}

#[test]
fn checked_in_product_rejects_tampered_mailbox_status_policy() {
    let expected = Some(MailboxStatusBit::sync_manager_mailbox_full(1));
    for actual in [
        Some(MailboxStatusBit::new(0x080c, 0x08, true)),
        Some(MailboxStatusBit::new(0x080d, 0x04, true)),
        Some(MailboxStatusBit::new(0x080d, 0x08, false)),
        None,
    ] {
        let mut tampered = generated::PRODUCT_CONFIG;
        tampered.slaves[0].mailbox_config.status_bit = actual;
        assert_eq!(
            tampered.startup_profiles(),
            Err(ProductStartupError::MailboxPolicy {
                position: 0,
                error: ProductMailboxPolicyError::StatusBitMismatch { expected, actual },
            })
        );
        assert_eq!(
            tampered.build_generated_pdo_configuration_batch::<3, 17>(),
            Err(ProductPdoBatchError::MailboxPolicy {
                position: 0,
                error: ProductMailboxPolicyError::StatusBitMismatch { expected, actual },
            })
        );
    }

    let mut wrong_index = generated::PRODUCT_CONFIG;
    wrong_index.slaves[0].mailbox_receive_sync_manager =
        MailboxReceiveSyncManager::new(4, 0x1100, 64, 0x22);
    assert_eq!(
        wrong_index.startup_profiles(),
        Err(ProductStartupError::MailboxPolicy {
            position: 0,
            error: ProductMailboxPolicyError::ReceiveSyncManagerNotDeclared { index: 4, count: 4 },
        })
    );

    let mut wrong_send_index = generated::PRODUCT_CONFIG;
    wrong_send_index.slaves[0].mailbox_send_sync_manager = 4;
    assert_eq!(
        wrong_send_index.startup_profiles(),
        Err(ProductStartupError::MailboxPolicy {
            position: 0,
            error: ProductMailboxPolicyError::SendSyncManagerNotDeclared { index: 4, count: 4 },
        })
    );

    let mut duplicate_index = generated::PRODUCT_CONFIG;
    duplicate_index.slaves[0].mailbox_send_sync_manager = 1;
    assert_eq!(
        duplicate_index.startup_profiles(),
        Err(ProductStartupError::MailboxPolicy {
            position: 0,
            error: ProductMailboxPolicyError::DuplicateMailboxSyncManager(1),
        })
    );

    let mut invalid_count = generated::PRODUCT_CONFIG;
    invalid_count.slaves[0].sii_sync_manager_count = 17;
    assert_eq!(
        invalid_count.startup_profiles(),
        Err(ProductStartupError::SiiConfiguration {
            position: 0,
            error: SiiConfigurationSignatureError::SyncManagerCountOutOfBounds,
        })
    );
    assert_eq!(
        invalid_count.build_generated_pdo_configuration_batch::<3, 17>(),
        Err(ProductPdoBatchError::MailboxPolicy {
            position: 0,
            error: ProductMailboxPolicyError::InvalidSyncManagerCount(17),
        })
    );

    assert_eq!(
        generated::PRODUCT_CONFIG.slaves[2]
            .mailbox_config
            .status_bit,
        None
    );
    assert!(generated::PRODUCT_CONFIG.startup_profiles().is_ok());
}

#[test]
fn checked_in_product_rejects_tampered_mapped_mailbox_status() {
    let procbuf =
        ProcBuf::<2, 16, 2, 64>::new(generated::PRODUCT_CONFIG.metadata.robot_id, BOOT_ID);

    let mut missing = generated::PRODUCT_CONFIG;
    missing.slaves[0].mapped_mailbox_status = None;
    assert!(matches!(
        activate(&missing, &observed_slaves(), &procbuf),
        Err(ProductActivationError::MailboxStatusMapping {
            position: 0,
            error: ProductMailboxStatusMappingError::Missing,
        })
    ));

    let mut wrong_offset = generated::PRODUCT_CONFIG;
    wrong_offset.slaves[0]
        .mapped_mailbox_status
        .as_mut()
        .unwrap()
        .domain_bit_offset = 257;
    assert!(matches!(
        activate(&wrong_offset, &observed_slaves(), &procbuf),
        Err(ProductActivationError::MailboxStatusMapping {
            position: 0,
            error: ProductMailboxStatusMappingError::BitOffsetMismatch { .. },
        })
    ));

    let mut wrong_age = generated::PRODUCT_CONFIG;
    wrong_age.slaves[0]
        .mapped_mailbox_status
        .as_mut()
        .unwrap()
        .max_age_cycles = 2;
    assert!(matches!(
        activate(&wrong_age, &observed_slaves(), &procbuf),
        Err(ProductActivationError::MailboxStatusMapping {
            position: 0,
            error: ProductMailboxStatusMappingError::MaximumAgeMismatch { .. },
        })
    ));

    let mut wrong_index = generated::PRODUCT_CONFIG;
    wrong_index.slaves[0]
        .mapped_mailbox_status
        .as_mut()
        .unwrap()
        .fmmu
        .index = 3;
    assert!(matches!(
        activate(&wrong_index, &observed_slaves(), &procbuf),
        Err(ProductActivationError::MailboxStatusMapping {
            position: 0,
            error: ProductMailboxStatusMappingError::FmmuIndexMismatch { .. },
        })
    ));

    let mut wrong_logical = generated::PRODUCT_CONFIG;
    wrong_logical.slaves[0]
        .mapped_mailbox_status
        .as_mut()
        .unwrap()
        .fmmu
        .logical_start = 0x1021;
    assert!(matches!(
        activate(&wrong_logical, &observed_slaves(), &procbuf),
        Err(ProductActivationError::MailboxStatusMapping {
            position: 0,
            error: ProductMailboxStatusMappingError::Descriptor(
                MailboxMappedStatusError::InvalidFmmu
            ),
        })
    ));

    let mut wrong_physical = generated::PRODUCT_CONFIG;
    wrong_physical.slaves[0]
        .mapped_mailbox_status
        .as_mut()
        .unwrap()
        .fmmu
        .physical_start = 0x080c;
    assert!(matches!(
        activate(&wrong_physical, &observed_slaves(), &procbuf),
        Err(ProductActivationError::MailboxStatusMapping {
            position: 0,
            error: ProductMailboxStatusMappingError::Descriptor(
                MailboxMappedStatusError::StatusBitMismatch
            ),
        })
    ));

    let mut duplicate_declaration = generated::PRODUCT_CONFIG;
    duplicate_declaration.slaves[0].sii_fmmu_count = 4;
    duplicate_declaration.slaves[0].sii_fmmu_usages[3] = SiiFmmuUsage::SyncManagerStatus;
    assert!(matches!(
        activate(&duplicate_declaration, &observed_slaves(), &procbuf),
        Err(ProductActivationError::MailboxStatusMapping {
            position: 0,
            error: ProductMailboxStatusMappingError::DeclarationCount(2),
        })
    ));

    let mut datagrams: [_; 4] =
        core::array::from_fn(|index| generated::PRODUCT_CONFIG.datagrams[index]);
    datagrams[1].spec.expected_wkc = 3;
    let mut wrong_lrd = generated::PRODUCT_CONFIG;
    wrong_lrd.datagrams = &datagrams;
    assert!(matches!(
        activate(&wrong_lrd, &observed_slaves(), &procbuf),
        Err(ProductActivationError::MailboxStatusDomainMismatch { domain_id: 0 })
    ));
}

#[test]
fn generated_mailbox_status_fmmu_passes_mapping_write_and_readback() {
    let binding = generated::PRODUCT_CONFIG.slaves[0]
        .mapped_mailbox_status
        .unwrap();
    let mut table = MappingTable::<0, 1>::new();
    table.add_fmmu(binding.fmmu).unwrap();

    let mut controller = MappingConfigController::<0, 1>::new();
    controller.start(0x1001, 7, 0, 1_000, 100, &table).unwrap();
    let write = controller.next_action(1).unwrap().unwrap();
    assert_eq!(write.item, MappingConfigItem::Fmmu(2));
    assert_eq!(write.payload().len(), FMMU_IMAGE_LEN);
    let mut expected = [0; FMMU_IMAGE_LEN];
    binding.fmmu.encode(&mut expected).unwrap();
    assert_eq!(write.payload(), &expected);
    assert_eq!(
        controller.accept(write, 7, &[], 1, 2),
        Ok(MappingConfigProgress::Advanced)
    );

    let read = controller.next_action(3).unwrap().unwrap();
    assert_eq!(read.item, MappingConfigItem::Fmmu(2));
    assert_eq!(
        controller.accept(read, 7, &expected, 1, 4),
        Ok(MappingConfigProgress::Complete)
    );
    assert_eq!(controller.phase(), MappingConfigPhase::Complete);
}

#[test]
fn checked_in_product_builds_exact_per_slave_pdo_startup_plans() {
    let left = generated::PRODUCT_CONFIG
        .build_pdo_startup_plan::<17>(0)
        .unwrap();
    let right = generated::PRODUCT_CONFIG
        .build_pdo_startup_plan::<17>(1)
        .unwrap();
    let io = generated::PRODUCT_CONFIG
        .build_pdo_startup_plan::<14>(2)
        .unwrap();

    assert_eq!(left.station_address(), 0x1001);
    assert_eq!(right.station_address(), 0x1002);
    assert_eq!(io.station_address(), 0x1003);
    assert_eq!(left.plan().writes(), right.plan().writes());
    assert_eq!(left.plan().len(), 17);
    assert_eq!(io.plan().len(), 14);

    let left_writes = left.plan().writes();
    assert_eq!(left_writes[0], PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap());
    assert_eq!(left_writes[1], PdoSdoWrite::new(0x1600, 0, &[0]).unwrap());
    assert_eq!(
        left_writes[2],
        PdoSdoWrite::new(0x1600, 1, &0x1000_6040u32.to_le_bytes()).unwrap()
    );
    assert_eq!(left_writes[5], PdoSdoWrite::new(0x1600, 0, &[3]).unwrap());
    assert_eq!(
        left_writes[6],
        PdoSdoWrite::new(0x1C12, 1, &[0, 0x16]).unwrap()
    );
    assert_eq!(left_writes[7], PdoSdoWrite::new(0x1C12, 0, &[1]).unwrap());
    assert_eq!(left_writes[8], PdoSdoWrite::new(0x1C13, 0, &[0]).unwrap());
    assert_eq!(left_writes[9], PdoSdoWrite::new(0x1A00, 0, &[0]).unwrap());
    assert_eq!(left_writes[14], PdoSdoWrite::new(0x1A00, 0, &[4]).unwrap());
    assert_eq!(left_writes[16], PdoSdoWrite::new(0x1C13, 0, &[1]).unwrap());

    let io_writes = io.plan().writes();
    assert_eq!(io_writes[0], PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap());
    assert_eq!(io_writes[1], PdoSdoWrite::new(0x1601, 0, &[0]).unwrap());
    assert_eq!(
        io_writes[2],
        PdoSdoWrite::new(0x1601, 1, &0x1001_7000u32.to_le_bytes()).unwrap()
    );
    assert_eq!(
        io_writes[4],
        PdoSdoWrite::new(0x1601, 3, &0x0801_7011u32.to_le_bytes()).unwrap()
    );
    assert_eq!(io_writes[8], PdoSdoWrite::new(0x1C13, 0, &[0]).unwrap());
    assert_eq!(io_writes[9], PdoSdoWrite::new(0x1A01, 0, &[0]).unwrap());
    assert_eq!(
        io_writes[10],
        PdoSdoWrite::new(0x1A01, 1, &0x1001_6000u32.to_le_bytes()).unwrap()
    );
    assert_eq!(io_writes[13], PdoSdoWrite::new(0x1C13, 0, &[1]).unwrap());
}

#[test]
fn checked_in_product_builds_ordered_dc_sync_plan() {
    let plan = generated::PRODUCT_CONFIG.dc_sync_plan().unwrap();
    assert_eq!(plan.reference_position(), Some(0));
    assert_eq!(plan.entries().len(), 2);
    assert_eq!(plan.entries()[0].position, 0);
    assert_eq!(plan.entries()[0].station_address, 0x1001);
    assert_eq!(plan.entries()[0].timing.cycle_time0_ns, 1_000_000);
    assert_eq!(plan.entries()[0].timing.cycle_time1_ns, 0);
    assert_eq!(plan.entries()[0].timing.assign_activate, 0x0300);
    assert_eq!(plan.entries()[1].position, 1);
    assert_eq!(plan.entries()[1].station_address, 0x1002);
    assert_eq!(plan.entries()[1].timing, plan.entries()[0].timing);
}

#[test]
fn checked_in_product_builds_one_exact_ordered_pdo_batch() {
    let batch = generated::PRODUCT_CONFIG
        .build_generated_pdo_configuration_batch::<3, 17>()
        .unwrap();
    let jobs = batch.jobs();
    assert_eq!(jobs.len(), 3);
    assert_eq!(jobs[0].station_address(), 0x1001);
    assert_eq!(jobs[1].station_address(), 0x1002);
    assert_eq!(jobs[2].station_address(), 0x1003);
    assert_eq!(
        jobs[0].mailbox_config(),
        MailboxConfig::new(0x1000, 64, 0x1100, 64)
            .with_status_bit(MailboxStatusBit::sync_manager_mailbox_full(1))
    );
    assert_eq!(jobs[1].mailbox_config(), jobs[0].mailbox_config());
    assert_eq!(
        jobs[2].mailbox_config(),
        MailboxConfig::new(0x1200, 32, 0x1300, 32)
    );

    let left = generated::PRODUCT_CONFIG
        .build_pdo_startup_plan::<17>(0)
        .unwrap();
    let right = generated::PRODUCT_CONFIG
        .build_pdo_startup_plan::<17>(1)
        .unwrap();
    let io = generated::PRODUCT_CONFIG
        .build_pdo_startup_plan::<17>(2)
        .unwrap();
    assert_eq!(jobs[0].plan().writes(), left.plan().writes());
    assert_eq!(jobs[1].plan().writes(), right.plan().writes());
    assert_eq!(jobs[2].plan().writes(), io.plan().writes());

    let override_mailbox = MailboxConfig::new(0x2000, 48, 0x2100, 48);
    let bindings = [
        ProductMailboxBinding::new(2, override_mailbox),
        ProductMailboxBinding::new(0, override_mailbox),
        ProductMailboxBinding::new(1, override_mailbox),
    ];
    let overridden = generated::PRODUCT_CONFIG
        .build_pdo_configuration_batch::<3, 17>(&bindings)
        .unwrap();
    assert!(
        overridden
            .jobs()
            .iter()
            .all(|job| job.mailbox_config() == override_mailbox)
    );
}

#[test]
fn product_pdo_batch_rejects_binding_and_capacity_mismatches_transactionally() {
    let mailbox = MailboxConfig::new(0x1000, 32, 0x1100, 32);
    let complete = [
        ProductMailboxBinding::new(0, mailbox),
        ProductMailboxBinding::new(1, mailbox),
        ProductMailboxBinding::new(2, mailbox),
    ];
    assert_eq!(
        generated::PRODUCT_CONFIG.build_pdo_configuration_batch::<3, 17>(&complete[..2]),
        Err(ProductPdoBatchError::MissingMailboxBinding { position: 2 })
    );
    assert_eq!(
        generated::PRODUCT_CONFIG.build_pdo_configuration_batch::<3, 17>(&[
            ProductMailboxBinding::new(0, mailbox),
            ProductMailboxBinding::new(0, mailbox),
            ProductMailboxBinding::new(2, mailbox),
        ]),
        Err(ProductPdoBatchError::DuplicateMailboxBinding { position: 0 })
    );
    assert_eq!(
        generated::PRODUCT_CONFIG.build_pdo_configuration_batch::<3, 17>(&[
            ProductMailboxBinding::new(0, mailbox),
            ProductMailboxBinding::new(1, mailbox),
            ProductMailboxBinding::new(99, mailbox),
        ]),
        Err(ProductPdoBatchError::UnknownMailboxBinding { position: 99 })
    );
    assert_eq!(
        generated::PRODUCT_CONFIG.build_pdo_configuration_batch::<2, 17>(&complete),
        Err(ProductPdoBatchError::Batch(
            PdoConfigBatchPlanError::CapacityExceeded
        ))
    );
    assert_eq!(
        generated::PRODUCT_CONFIG.build_pdo_configuration_batch::<3, 16>(&complete),
        Err(ProductPdoBatchError::SlavePlan {
            position: 0,
            error: ProductPdoPlanError::Plan(PdoConfigPlanError::CapacityExceeded),
        })
    );

    let mut duplicate_station = generated::PRODUCT_CONFIG;
    duplicate_station.slaves[1].station_address = duplicate_station.slaves[0].station_address;
    assert_eq!(
        duplicate_station.build_pdo_configuration_batch::<3, 17>(&complete),
        Err(ProductPdoBatchError::Batch(
            PdoConfigBatchPlanError::DuplicateStationAddress {
                station_address: 0x1001,
            }
        ))
    );

    let mut invalid_generated_mailbox = generated::PRODUCT_CONFIG;
    invalid_generated_mailbox.slaves[0].mailbox_config = MailboxConfig::new(0, 32, 0x1100, 32);
    assert_eq!(
        invalid_generated_mailbox.build_generated_pdo_configuration_batch::<3, 17>(),
        Err(ProductPdoBatchError::InvalidMailboxConfig {
            position: 0,
            error: MailboxConfigError::AddressZero(MailboxDirection::Send),
        })
    );
}

#[test]
fn hash_schema_and_procbuf_contracts_fail_closed() {
    let procbuf =
        ProcBuf::<2, 16, 2, 64>::new(generated::PRODUCT_CONFIG.metadata.robot_id, BOOT_ID);
    assert!(matches!(
        generated::PRODUCT_CONFIG.activate::<16, 64, 14, 2, 4, 2, 16>(
            [0; 32],
            &observed_slaves(),
            &procbuf,
            BOOT_ID,
        ),
        Err(ProductActivationError::ConfigHashMismatch)
    ));

    let mut config = generated::PRODUCT_CONFIG;
    config.metadata.schema_version = "esop.product-runtime.v0";
    assert!(matches!(
        activate(&config, &observed_slaves(), &procbuf),
        Err(ProductActivationError::SchemaMismatch)
    ));

    let mut config = generated::PRODUCT_CONFIG;
    config.metadata.product_name = "";
    assert!(matches!(
        activate(&config, &observed_slaves(), &procbuf),
        Err(ProductActivationError::EmptyProductName)
    ));

    let mut config = generated::PRODUCT_CONFIG;
    config.metadata.robot_id = 0;
    assert!(matches!(
        activate(&config, &observed_slaves(), &procbuf),
        Err(ProductActivationError::InvalidRobotId)
    ));

    let mut config = generated::PRODUCT_CONFIG;
    config.metadata.policy_version = 0;
    assert!(matches!(
        activate(&config, &observed_slaves(), &procbuf),
        Err(ProductActivationError::InvalidPolicyVersion)
    ));

    let mut config = generated::PRODUCT_CONFIG;
    config.metadata.deadline_ns = config.metadata.base_period_ns + 1;
    assert!(matches!(
        activate(&config, &observed_slaves(), &procbuf),
        Err(ProductActivationError::InvalidCycleTiming)
    ));

    let mut config = generated::PRODUCT_CONFIG;
    config.procbuf_layout.layout_hash ^= 1;
    assert!(matches!(
        activate(&config, &observed_slaves(), &procbuf),
        Err(ProductActivationError::ProcBufLayoutMismatch)
    ));

    let wrong_robot = ProcBuf::<2, 16, 2, 64>::new(99, BOOT_ID);
    assert!(matches!(
        activate(&generated::PRODUCT_CONFIG, &observed_slaves(), &wrong_robot),
        Err(ProductActivationError::ProcBufHeader(
            ProcBufHeaderError::RobotIdMismatch
        ))
    ));
    assert!(matches!(
        generated::PRODUCT_CONFIG.activate::<16, 64, 14, 2, 4, 2, 16>(
            generated::PRODUCT_CONFIG.metadata.config_sha256,
            &observed_slaves(),
            &procbuf,
            BOOT_ID + 1,
        ),
        Err(ProductActivationError::ProcBufHeader(
            ProcBufHeaderError::BootIdMismatch
        ))
    ));

    let wrong_capacity =
        ProcBuf::<2, 15, 2, 64>::new(generated::PRODUCT_CONFIG.metadata.robot_id, BOOT_ID);
    assert!(matches!(
        generated::PRODUCT_CONFIG.activate::<15, 64, 14, 2, 4, 2, 16>(
            generated::PRODUCT_CONFIG.metadata.config_sha256,
            &observed_slaves(),
            &wrong_capacity,
            BOOT_ID,
        ),
        Err(ProductActivationError::ProcBufDimensionsMismatch)
    ));
}

#[test]
fn exact_topology_is_required() {
    let procbuf =
        ProcBuf::<2, 16, 2, 64>::new(generated::PRODUCT_CONFIG.metadata.robot_id, BOOT_ID);
    let observed = observed_slaves();
    assert!(matches!(
        activate(&generated::PRODUCT_CONFIG, &observed[..2], &procbuf),
        Err(ProductActivationError::SlaveCountMismatch { .. })
    ));

    let mut extra = [SlaveRecord::EMPTY; 4];
    extra[..3].copy_from_slice(&observed);
    extra[3] = SlaveRecord {
        position: 3,
        online: true,
        ..SlaveRecord::EMPTY
    };
    assert!(matches!(
        activate(&generated::PRODUCT_CONFIG, &extra, &procbuf),
        Err(ProductActivationError::SlaveCountMismatch { .. })
    ));

    let mut offline = observed;
    offline[0].online = false;
    assert!(matches!(
        activate(&generated::PRODUCT_CONFIG, &offline, &procbuf),
        Err(ProductActivationError::SlaveOffline { position: 0 })
    ));

    let mut unconfigured = observed;
    unconfigured[0].configured = false;
    assert!(matches!(
        activate(&generated::PRODUCT_CONFIG, &unconfigured, &procbuf),
        Err(ProductActivationError::SlaveNotConfigured { position: 0 })
    ));

    let mut wrong_station = observed;
    wrong_station[1].station_address ^= 1;
    assert!(matches!(
        activate(&generated::PRODUCT_CONFIG, &wrong_station, &procbuf),
        Err(ProductActivationError::SlaveStationAddressMismatch { position: 1 })
    ));

    let mut wrong_identity = observed;
    wrong_identity[2].identity.product_code ^= 1;
    assert!(matches!(
        activate(&generated::PRODUCT_CONFIG, &wrong_identity, &procbuf),
        Err(ProductActivationError::SlaveIdentityMismatch { position: 2 })
    ));

    let mut duplicate = observed;
    duplicate[2].position = 1;
    assert!(matches!(
        activate(&generated::PRODUCT_CONFIG, &duplicate, &procbuf),
        Err(ProductActivationError::DuplicateObservedSlavePosition { position: 1 })
    ));

    let mut duplicate_config = generated::PRODUCT_CONFIG;
    duplicate_config.slaves[2].position = 1;
    assert!(matches!(
        activate(&duplicate_config, &observed_slaves(), &procbuf),
        Err(ProductActivationError::DuplicateConfiguredSlavePosition { position: 1 })
    ));
}

#[test]
fn runtime_capacities_and_generated_domain_evidence_are_enforced() {
    let procbuf =
        ProcBuf::<2, 16, 2, 64>::new(generated::PRODUCT_CONFIG.metadata.robot_id, BOOT_ID);
    assert!(matches!(
        generated::PRODUCT_CONFIG.activate::<16, 64, 13, 2, 4, 2, 16>(
            generated::PRODUCT_CONFIG.metadata.config_sha256,
            &observed_slaves(),
            &procbuf,
            BOOT_ID,
        ),
        Err(ProductActivationError::Registry(
            DomainRegistryError::PdoCapacityExceeded
        ))
    ));
    assert!(matches!(
        generated::PRODUCT_CONFIG.activate::<16, 64, 14, 1, 4, 2, 16>(
            generated::PRODUCT_CONFIG.metadata.config_sha256,
            &observed_slaves(),
            &procbuf,
            BOOT_ID,
        ),
        Err(ProductActivationError::Registry(
            DomainRegistryError::DatagramCapacityExceeded
        ))
    ));
    assert!(matches!(
        generated::PRODUCT_CONFIG.activate::<16, 64, 14, 2, 3, 2, 16>(
            generated::PRODUCT_CONFIG.metadata.config_sha256,
            &observed_slaves(),
            &procbuf,
            BOOT_ID,
        ),
        Err(ProductActivationError::Registry(
            DomainRegistryError::Schedule(_)
        ))
    ));
    assert!(matches!(
        generated::PRODUCT_CONFIG.activate::<16, 64, 14, 2, 4, 1, 1>(
            generated::PRODUCT_CONFIG.metadata.config_sha256,
            &observed_slaves(),
            &procbuf,
            BOOT_ID,
        ),
        Err(ProductActivationError::Registry(
            DomainRegistryError::FramePlans(FramePlanSetError::CapacityExceeded)
        ))
    ));

    let mut config = generated::PRODUCT_CONFIG;
    config.domains[0].expected_wkc += 1;
    assert!(matches!(
        activate(&config, &observed_slaves(), &procbuf),
        Err(ProductActivationError::MailboxStatusDomainMismatch { domain_id: 0 })
    ));
}

#[test]
fn generated_axis_contracts_are_revalidated() {
    let procbuf =
        ProcBuf::<2, 16, 2, 64>::new(generated::PRODUCT_CONFIG.metadata.robot_id, BOOT_ID);

    let mut duplicate_index = generated::PRODUCT_CONFIG;
    duplicate_index.axes[1].index = 0;
    assert!(matches!(
        activate(&duplicate_index, &observed_slaves(), &procbuf),
        Err(ProductActivationError::DuplicateAxisIndex { index: 0 })
    ));

    let mut non_drive = generated::PRODUCT_CONFIG;
    non_drive.axes[0].slave_position = 2;
    assert!(matches!(
        activate(&non_drive, &observed_slaves(), &procbuf),
        Err(ProductActivationError::AxisRequiresDrive { axis: 0 })
    ));

    let mut duplicate_drive = generated::PRODUCT_CONFIG;
    duplicate_drive.axes[1].slave_position = 0;
    assert!(matches!(
        activate(&duplicate_drive, &observed_slaves(), &procbuf),
        Err(ProductActivationError::DuplicateAxisDrive { axis: 1, .. })
    ));

    let mut invalid_policy = generated::PRODUCT_CONFIG;
    invalid_policy.axes[0]
        .policy
        .max_velocity_radians_per_second = 0.0;
    assert!(matches!(
        activate(&invalid_policy, &observed_slaves(), &procbuf),
        Err(ProductActivationError::AxisPolicy {
            axis: 0,
            error: Cia402AxisCommandPolicyError::NonPositiveProductLimit
        })
    ));

    let mut invalid_map = generated::PRODUCT_CONFIG;
    invalid_map.axes[0].mode = OperatingMode::Cst;
    assert!(matches!(
        activate(&invalid_map, &observed_slaves(), &procbuf),
        Err(ProductActivationError::AxisPdo { axis: 0, .. })
    ));

    let mut invalid_kind = generated::PRODUCT_CONFIG;
    invalid_kind.slaves[0].kind = ProductSlaveKind::EthercatIo;
    assert!(matches!(
        activate(&invalid_kind, &observed_slaves(), &procbuf),
        Err(ProductActivationError::AxisRequiresDrive { axis: 0 })
    ));
}
