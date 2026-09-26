use esop_product_config::{
    ActivatedProduct, Cia402AxisCommandPolicyError, DomainRegistryError, FramePlanSetError,
    OperatingMode, ProcBuf, ProcBufHeaderError, ProductActivationError, ProductSlaveKind,
    SlaveRecord,
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
