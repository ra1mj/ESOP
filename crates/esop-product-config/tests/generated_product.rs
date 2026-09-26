use esop_product_config::{
    ActivatedProduct, Cia402AxisCommandPolicyError, DomainRegistryError, FramePlanSetError,
    MailboxConfig, OperatingMode, PdoConfigBatchPlanError, PdoConfigPlanError, PdoSdoWrite,
    ProcBuf, ProcBufHeaderError, ProductActivationError, ProductMailboxBinding,
    ProductPdoBatchError, ProductPdoPlanError, ProductSlaveKind, SlaveRecord,
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
            0x49, 0x26, 0x96, 0xaa, 0xb6, 0x8a, 0x69, 0x7d, 0xaf, 0xc5, 0xe1, 0xcf, 0xd6, 0xd0,
            0xc6, 0x0b, 0x5d, 0x17, 0x1e, 0x7e, 0x74, 0x57, 0xa3, 0xa2, 0x2c, 0x65, 0x28, 0x93,
            0x6a, 0xc0, 0x1c, 0x27,
        ]
    );
    assert_eq!(active.registry().domain_count(), 2);
    assert_eq!(active.registry().domain(0).unwrap().pdo_count, 14);
    assert_eq!(active.registry().domain(0).unwrap().expected_wkc, 4);
    assert_eq!(active.registry().domain(1).unwrap().pdo_count, 2);
    assert_eq!(active.registry().domain(1).unwrap().expected_wkc, 2);
    assert_eq!(active.schedule().hyperperiod_ticks(), 4);
    assert_eq!(active.schedule().due_mask(0), 0b11);
    assert_eq!(active.schedule().due_mask(1), 0b01);
    assert_eq!(active.frame_plans().len(), 2);
    assert_eq!(active.frame_plans()[0].datagram_count(), 2);
    assert_eq!(active.frame_plans()[1].datagram_count(), 2);
    assert_eq!(active.axis_modes(), &[OperatingMode::Csp; 2]);
    for map in active.axis_pdo_maps() {
        map.validate_for(OperatingMode::Csp).unwrap();
    }
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
        .build_pdo_startup_plan::<12>(2)
        .unwrap();

    assert_eq!(left.station_address(), 0x1001);
    assert_eq!(right.station_address(), 0x1002);
    assert_eq!(io.station_address(), 0x1003);
    assert_eq!(left.plan().writes(), right.plan().writes());
    assert_eq!(left.plan().len(), 17);
    assert_eq!(io.plan().len(), 12);

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
    assert_eq!(io_writes[6], PdoSdoWrite::new(0x1C13, 0, &[0]).unwrap());
    assert_eq!(io_writes[7], PdoSdoWrite::new(0x1A01, 0, &[0]).unwrap());
    assert_eq!(
        io_writes[8],
        PdoSdoWrite::new(0x1A01, 1, &0x1001_6000u32.to_le_bytes()).unwrap()
    );
    assert_eq!(io_writes[11], PdoSdoWrite::new(0x1C13, 0, &[1]).unwrap());
}

#[test]
fn checked_in_product_builds_one_exact_ordered_pdo_batch() {
    let left_mailbox = MailboxConfig::new(0x1000, 32, 0x1100, 32);
    let right_mailbox = MailboxConfig::new(0x1200, 48, 0x1300, 48);
    let io_mailbox = MailboxConfig::new(0x1400, 64, 0x1500, 64);
    let bindings = [
        ProductMailboxBinding::new(2, io_mailbox),
        ProductMailboxBinding::new(0, left_mailbox),
        ProductMailboxBinding::new(1, right_mailbox),
    ];
    let batch = generated::PRODUCT_CONFIG
        .build_pdo_configuration_batch::<3, 17>(&bindings)
        .unwrap();
    let jobs = batch.jobs();
    assert_eq!(jobs.len(), 3);
    assert_eq!(jobs[0].station_address(), 0x1001);
    assert_eq!(jobs[1].station_address(), 0x1002);
    assert_eq!(jobs[2].station_address(), 0x1003);
    assert_eq!(jobs[0].mailbox_config(), left_mailbox);
    assert_eq!(jobs[1].mailbox_config(), right_mailbox);
    assert_eq!(jobs[2].mailbox_config(), io_mailbox);

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
        Err(ProductActivationError::DomainEvidenceMismatch { domain_id: 0 })
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
