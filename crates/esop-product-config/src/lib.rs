#![no_std]

//! Runtime attachment for product configurations emitted by `esop-cfggen`.
//!
//! The generated module is static data only. Activation rebuilds every
//! Domain, schedule, frame-plan and CiA 402 map through the owning runtime
//! crates, then returns the complete frozen result as one value.

pub use esop_ethercat_core::wire::Command;
pub use esop_ethercat_core::{
    DomainConfig, DomainDatagramSpec, DomainInfo, DomainRegistry, DomainRegistryError,
    FramePlanSet, FramePlanSetError, PdoDirection, PdoEntry, PdoRegistrationRequest, ScheduleTable,
    SlaveIdentity, SlaveRecord,
};
pub use esop_lifecycle_guard::procbuf::{Cia402AxisCommandPolicy, Cia402AxisCommandPolicyError};
pub use esop_procbuf::{
    HeaderError as ProcBufHeaderError, ProcBuf, ProcBufDimensions, ProcBufLayoutDescriptor,
    ProcBufLayoutError, describe_layout,
};
pub use esop_profile_cia402::{Cia402PdoError, Cia402PdoMap, OperatingMode};

pub const PRODUCT_RUNTIME_SCHEMA: &str = "esop.product-runtime.v1";
pub const CONFIG_SHA256_BYTES: usize = 32;
pub const MAX_PRODUCT_AXIS_PDOS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProductMetadata {
    pub schema_version: &'static str,
    pub product_name: &'static str,
    pub config_sha256: [u8; CONFIG_SHA256_BYTES],
    pub robot_id: u64,
    pub policy_version: u32,
    pub base_period_ns: u64,
    pub deadline_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductSlaveKind {
    Cia402Drive,
    EthercatIo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProductSlaveConfig {
    pub name: &'static str,
    pub position: u16,
    pub station_address: u16,
    pub domain_id: u8,
    pub kind: ProductSlaveKind,
    pub identity: SlaveIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProductDomainConfig {
    pub name: &'static str,
    pub config: DomainConfig,
    pub expected_pdo_count: usize,
    pub expected_datagram_count: usize,
    pub expected_wkc: u16,
    pub input_expected_wkc: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProductPdoConfig {
    pub domain_id: u8,
    pub assignment_index: u16,
    pub sync_manager: u8,
    pub bit_offset: usize,
    pub request: PdoRegistrationRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProductDatagramConfig {
    pub domain_id: u8,
    pub spec: DomainDatagramSpec,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProductAxisConfig {
    pub name: &'static str,
    pub index: usize,
    pub slave_position: u16,
    pub mode: OperatingMode,
    pub policy: Cia402AxisCommandPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticProductConfig<'a, const SLAVES: usize, const DOMAINS: usize, const AXES: usize> {
    pub metadata: ProductMetadata,
    pub procbuf_layout: ProcBufLayoutDescriptor,
    pub slaves: [ProductSlaveConfig; SLAVES],
    pub domains: [ProductDomainConfig; DOMAINS],
    pub pdos: &'a [ProductPdoConfig],
    pub datagrams: &'a [ProductDatagramConfig],
    pub axes: [ProductAxisConfig; AXES],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductActivationError {
    SchemaMismatch,
    ConfigHashMismatch,
    EmptyProductName,
    InvalidRobotId,
    InvalidPolicyVersion,
    InvalidCycleTiming,
    ProcBufDimensionsMismatch,
    ProcBufLayout(ProcBufLayoutError),
    ProcBufLayoutMismatch,
    ProcBufHeader(ProcBufHeaderError),
    SlaveCountMismatch {
        expected: usize,
        actual: usize,
    },
    DuplicateConfiguredSlavePosition {
        position: u16,
    },
    DuplicateObservedSlavePosition {
        position: u16,
    },
    MissingSlave {
        position: u16,
    },
    SlaveOffline {
        position: u16,
    },
    SlaveNotConfigured {
        position: u16,
    },
    SlaveStationAddressMismatch {
        position: u16,
    },
    SlaveIdentityMismatch {
        position: u16,
    },
    Registry(DomainRegistryError),
    PdoUnknownSlave {
        pdo_index: usize,
    },
    PdoDomainMismatch {
        pdo_index: usize,
    },
    DomainEvidenceMismatch {
        domain_id: u8,
    },
    AxisIndexOutOfRange {
        axis: usize,
        index: usize,
    },
    DuplicateAxisIndex {
        index: usize,
    },
    AxisUnknownSlave {
        axis: usize,
    },
    AxisRequiresDrive {
        axis: usize,
    },
    DuplicateAxisDrive {
        axis: usize,
        slave_position: u16,
    },
    AxisPolicy {
        axis: usize,
        error: Cia402AxisCommandPolicyError,
    },
    AxisPdoCapacity {
        axis: usize,
    },
    AxisPdo {
        axis: usize,
        error: Cia402PdoError,
    },
}

pub struct ActivatedProduct<
    const DOMAINS: usize,
    const PDOS: usize,
    const DATAGRAMS: usize,
    const SLOTS: usize,
    const FRAMES: usize,
    const PLAN: usize,
    const AXES: usize,
> {
    metadata: ProductMetadata,
    registry: DomainRegistry<DOMAINS, PDOS, DATAGRAMS>,
    schedule: ScheduleTable<DOMAINS, SLOTS>,
    frame_plans: [FramePlanSet<FRAMES, PLAN>; DOMAINS],
    axis_modes: [OperatingMode; AXES],
    axis_policies: [Cia402AxisCommandPolicy; AXES],
    axis_pdo_maps: [Cia402PdoMap; AXES],
}

impl<
    const DOMAINS: usize,
    const PDOS: usize,
    const DATAGRAMS: usize,
    const SLOTS: usize,
    const FRAMES: usize,
    const PLAN: usize,
    const AXES: usize,
> ActivatedProduct<DOMAINS, PDOS, DATAGRAMS, SLOTS, FRAMES, PLAN, AXES>
{
    pub const fn metadata(&self) -> ProductMetadata {
        self.metadata
    }

    pub const fn registry(&self) -> &DomainRegistry<DOMAINS, PDOS, DATAGRAMS> {
        &self.registry
    }

    pub const fn schedule(&self) -> &ScheduleTable<DOMAINS, SLOTS> {
        &self.schedule
    }

    pub const fn frame_plans(&self) -> &[FramePlanSet<FRAMES, PLAN>; DOMAINS] {
        &self.frame_plans
    }

    pub const fn axis_modes(&self) -> &[OperatingMode; AXES] {
        &self.axis_modes
    }

    pub const fn axis_policies(&self) -> &[Cia402AxisCommandPolicy; AXES] {
        &self.axis_policies
    }

    pub const fn axis_pdo_maps(&self) -> &[Cia402PdoMap; AXES] {
        &self.axis_pdo_maps
    }
}

const EMPTY_AXIS_POLICY: Cia402AxisCommandPolicy = Cia402AxisCommandPolicy {
    position_units_per_radian: 0.0,
    velocity_units_per_radian_per_second: 0.0,
    torque_units_per_newton_metre: 0.0,
    position_offset: 0,
    min_position_radians: 0.0,
    max_position_radians: 0.0,
    max_velocity_radians_per_second: 0.0,
    max_torque_newton_metres: 0.0,
    max_position_step_radians: 0.0,
};

type ActivatedAxes<const AXES: usize> = (
    [OperatingMode; AXES],
    [Cia402AxisCommandPolicy; AXES],
    [Cia402PdoMap; AXES],
);

impl<'a, const SLAVES: usize, const DOMAINS: usize, const AXES: usize>
    StaticProductConfig<'a, SLAVES, DOMAINS, AXES>
{
    #[allow(clippy::too_many_arguments)]
    pub fn activate<
        const IO: usize,
        const EVENTS: usize,
        const PDOS: usize,
        const DATAGRAMS: usize,
        const SLOTS: usize,
        const FRAMES: usize,
        const PLAN: usize,
    >(
        &self,
        expected_config_sha256: [u8; CONFIG_SHA256_BYTES],
        observed_slaves: &[SlaveRecord],
        procbuf: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
        boot_id: u64,
    ) -> Result<
        ActivatedProduct<DOMAINS, PDOS, DATAGRAMS, SLOTS, FRAMES, PLAN, AXES>,
        ProductActivationError,
    > {
        self.validate_identity(expected_config_sha256)?;
        self.validate_procbuf::<IO, EVENTS>(procbuf, boot_id)?;
        self.validate_topology(observed_slaves)?;

        let mut registry = DomainRegistry::<DOMAINS, PDOS, DATAGRAMS>::new();
        for domain in self.domains {
            registry
                .register_domain(domain.config)
                .map_err(ProductActivationError::Registry)?;
        }
        for (pdo_index, pdo) in self.pdos.iter().copied().enumerate() {
            self.validate_pdo_owner(pdo_index, pdo)?;
            registry
                .register_pdo_at(pdo.domain_id, pdo.bit_offset, pdo.request)
                .map_err(ProductActivationError::Registry)?;
        }
        for datagram in self.datagrams.iter().copied() {
            registry
                .register_datagram(datagram.domain_id, datagram.spec)
                .map_err(ProductActivationError::Registry)?;
        }
        self.validate_domain_evidence(&registry)?;

        let mut frame_plans = [FramePlanSet::new(); DOMAINS];
        let schedule = registry
            .activate_with_frame_plans::<SLOTS, FRAMES, PLAN>(
                self.metadata.base_period_ns,
                &mut frame_plans,
            )
            .map_err(ProductActivationError::Registry)?;
        let (axis_modes, axis_policies, axis_pdo_maps) = self.validate_axes(&registry)?;

        Ok(ActivatedProduct {
            metadata: self.metadata,
            registry,
            schedule,
            frame_plans,
            axis_modes,
            axis_policies,
            axis_pdo_maps,
        })
    }

    fn validate_identity(
        &self,
        expected_config_sha256: [u8; CONFIG_SHA256_BYTES],
    ) -> Result<(), ProductActivationError> {
        if self.metadata.schema_version != PRODUCT_RUNTIME_SCHEMA {
            return Err(ProductActivationError::SchemaMismatch);
        }
        if self.metadata.config_sha256 != expected_config_sha256 {
            return Err(ProductActivationError::ConfigHashMismatch);
        }
        if self.metadata.product_name.is_empty() {
            return Err(ProductActivationError::EmptyProductName);
        }
        if self.metadata.robot_id == 0 {
            return Err(ProductActivationError::InvalidRobotId);
        }
        if self.metadata.policy_version == 0 {
            return Err(ProductActivationError::InvalidPolicyVersion);
        }
        if self.metadata.base_period_ns == 0
            || self.metadata.deadline_ns == 0
            || self.metadata.deadline_ns > self.metadata.base_period_ns
        {
            return Err(ProductActivationError::InvalidCycleTiming);
        }
        Ok(())
    }

    fn validate_procbuf<const IO: usize, const EVENTS: usize>(
        &self,
        procbuf: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
        boot_id: u64,
    ) -> Result<(), ProductActivationError> {
        let dimensions = self.procbuf_layout.dimensions;
        if usize::from(dimensions.axes) != AXES
            || usize::from(dimensions.io_channels) != IO
            || usize::from(dimensions.domains) != DOMAINS
            || usize::from(dimensions.event_capacity) != EVENTS
        {
            return Err(ProductActivationError::ProcBufDimensionsMismatch);
        }
        let actual = describe_layout(dimensions).map_err(ProductActivationError::ProcBufLayout)?;
        if actual != self.procbuf_layout {
            return Err(ProductActivationError::ProcBufLayoutMismatch);
        }
        procbuf
            .validate_header(self.metadata.robot_id, boot_id)
            .map_err(ProductActivationError::ProcBufHeader)
    }

    fn validate_topology(
        &self,
        observed_slaves: &[SlaveRecord],
    ) -> Result<(), ProductActivationError> {
        if observed_slaves.len() != SLAVES {
            return Err(ProductActivationError::SlaveCountMismatch {
                expected: SLAVES,
                actual: observed_slaves.len(),
            });
        }
        for (index, configured) in self.slaves.iter().enumerate() {
            if self.slaves[..index]
                .iter()
                .any(|previous| previous.position == configured.position)
            {
                return Err(ProductActivationError::DuplicateConfiguredSlavePosition {
                    position: configured.position,
                });
            }
        }
        for (index, observed) in observed_slaves.iter().enumerate() {
            if observed_slaves[..index]
                .iter()
                .any(|previous| previous.position == observed.position)
            {
                return Err(ProductActivationError::DuplicateObservedSlavePosition {
                    position: observed.position,
                });
            }
        }
        for configured in self.slaves {
            let observed = observed_slaves
                .iter()
                .find(|candidate| candidate.position == configured.position)
                .ok_or(ProductActivationError::MissingSlave {
                    position: configured.position,
                })?;
            if !observed.online {
                return Err(ProductActivationError::SlaveOffline {
                    position: configured.position,
                });
            }
            if !observed.configured {
                return Err(ProductActivationError::SlaveNotConfigured {
                    position: configured.position,
                });
            }
            if observed.station_address != configured.station_address {
                return Err(ProductActivationError::SlaveStationAddressMismatch {
                    position: configured.position,
                });
            }
            if !observed.identity.matches(configured.identity) {
                return Err(ProductActivationError::SlaveIdentityMismatch {
                    position: configured.position,
                });
            }
        }
        Ok(())
    }

    fn validate_pdo_owner(
        &self,
        pdo_index: usize,
        pdo: ProductPdoConfig,
    ) -> Result<(), ProductActivationError> {
        let slave = self
            .slaves
            .iter()
            .find(|slave| slave.position == pdo.request.slave_position)
            .ok_or(ProductActivationError::PdoUnknownSlave { pdo_index })?;
        if slave.domain_id != pdo.domain_id {
            return Err(ProductActivationError::PdoDomainMismatch { pdo_index });
        }
        Ok(())
    }

    fn validate_domain_evidence<const PDOS: usize, const DATAGRAMS: usize>(
        &self,
        registry: &DomainRegistry<DOMAINS, PDOS, DATAGRAMS>,
    ) -> Result<(), ProductActivationError> {
        for expected in self.domains {
            let actual = registry
                .domain(expected.config.id)
                .map_err(ProductActivationError::Registry)?;
            if actual.config != expected.config
                || actual.pdo_count != expected.expected_pdo_count
                || actual.datagram_count != expected.expected_datagram_count
                || actual.expected_wkc != expected.expected_wkc
                || actual.input_expected_wkc != expected.input_expected_wkc
            {
                return Err(ProductActivationError::DomainEvidenceMismatch {
                    domain_id: expected.config.id,
                });
            }
        }
        Ok(())
    }

    fn validate_axes<const PDOS: usize, const DATAGRAMS: usize>(
        &self,
        registry: &DomainRegistry<DOMAINS, PDOS, DATAGRAMS>,
    ) -> Result<ActivatedAxes<AXES>, ProductActivationError> {
        let mut modes = [OperatingMode::Unknown; AXES];
        let mut policies = [EMPTY_AXIS_POLICY; AXES];
        let mut maps = [Cia402PdoMap::new(); AXES];
        let mut used_indices = [false; AXES];

        for (axis_position, axis) in self.axes.iter().copied().enumerate() {
            if axis.index >= AXES {
                return Err(ProductActivationError::AxisIndexOutOfRange {
                    axis: axis_position,
                    index: axis.index,
                });
            }
            if used_indices[axis.index] {
                return Err(ProductActivationError::DuplicateAxisIndex { index: axis.index });
            }
            used_indices[axis.index] = true;

            let slave = self
                .slaves
                .iter()
                .find(|slave| slave.position == axis.slave_position)
                .ok_or(ProductActivationError::AxisUnknownSlave {
                    axis: axis_position,
                })?;
            if slave.kind != ProductSlaveKind::Cia402Drive {
                return Err(ProductActivationError::AxisRequiresDrive {
                    axis: axis_position,
                });
            }
            if self.axes[..axis_position]
                .iter()
                .any(|previous| previous.slave_position == axis.slave_position)
            {
                return Err(ProductActivationError::DuplicateAxisDrive {
                    axis: axis_position,
                    slave_position: axis.slave_position,
                });
            }
            axis.policy.validate_for_product().map_err(|error| {
                ProductActivationError::AxisPolicy {
                    axis: axis_position,
                    error,
                }
            })?;

            let mut entries = [PdoEntry::EMPTY; MAX_PRODUCT_AXIS_PDOS];
            let mut count = 0;
            for domain in self.domains {
                for registered in registry
                    .pdos(domain.config.id)
                    .map_err(ProductActivationError::Registry)?
                {
                    if registered.slave_position != axis.slave_position {
                        continue;
                    }
                    if count == entries.len() {
                        return Err(ProductActivationError::AxisPdoCapacity {
                            axis: axis_position,
                        });
                    }
                    entries[count] = registered.entry;
                    count += 1;
                }
            }
            let map = Cia402PdoMap::from_pdo_entries(&entries[..count])
                .and_then(|map| {
                    map.validate_for(axis.mode)?;
                    Ok(map)
                })
                .map_err(|error| ProductActivationError::AxisPdo {
                    axis: axis_position,
                    error,
                })?;
            modes[axis.index] = axis.mode;
            policies[axis.index] = axis.policy;
            maps[axis.index] = map;
        }
        Ok((modes, policies, maps))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use esop_ethercat_core::{AlStatus, EthercatState};

    const POLICY: Cia402AxisCommandPolicy = Cia402AxisCommandPolicy {
        position_units_per_radian: 1_000.0,
        velocity_units_per_radian_per_second: 100.0,
        torque_units_per_newton_metre: 10.0,
        position_offset: 0,
        min_position_radians: -10.0,
        max_position_radians: 10.0,
        max_velocity_radians_per_second: 5.0,
        max_torque_newton_metres: 2.0,
        max_position_step_radians: 0.1,
    };

    const IDENTITY: SlaveIdentity = SlaveIdentity {
        vendor_id: 1,
        product_code: 2,
        revision: 3,
        serial: 0,
    };

    static PDOS: [ProductPdoConfig; 8] = [
        pdo(0x6040, PdoDirection::Rx, 0, 16, false),
        pdo(0x6060, PdoDirection::Rx, 16, 8, true),
        pdo(0x607A, PdoDirection::Rx, 24, 32, true),
        pdo(0x6041, PdoDirection::Tx, 56, 16, false),
        pdo(0x6061, PdoDirection::Tx, 72, 8, true),
        pdo(0x603F, PdoDirection::Tx, 80, 16, false),
        pdo(0x6064, PdoDirection::Tx, 96, 32, true),
        pdo(0x606C, PdoDirection::Tx, 128, 32, true),
    ];

    static DATAGRAMS: [ProductDatagramConfig; 2] = [
        ProductDatagramConfig {
            domain_id: 0,
            spec: DomainDatagramSpec::output(Command::Lwr, 0, 0x1000, 0, 7, 1),
        },
        ProductDatagramConfig {
            domain_id: 0,
            spec: DomainDatagramSpec::input(Command::Lrd, 1, 0x1007, 7, 13, 1),
        },
    ];

    const fn pdo(
        index: u16,
        direction: PdoDirection,
        bit_offset: usize,
        bit_length: u8,
        signed: bool,
    ) -> ProductPdoConfig {
        ProductPdoConfig {
            domain_id: 0,
            assignment_index: if matches!(direction, PdoDirection::Rx) {
                0x1600
            } else {
                0x1A00
            },
            sync_manager: if matches!(direction, PdoDirection::Rx) {
                2
            } else {
                3
            },
            bit_offset,
            request: PdoRegistrationRequest::new(0, index, 0, direction, bit_length, signed),
        }
    }

    fn config() -> StaticProductConfig<'static, 1, 1, 1> {
        StaticProductConfig {
            metadata: ProductMetadata {
                schema_version: PRODUCT_RUNTIME_SCHEMA,
                product_name: "unit-test",
                config_sha256: [0xA5; CONFIG_SHA256_BYTES],
                robot_id: 7,
                policy_version: 1,
                base_period_ns: 1_000_000,
                deadline_ns: 800_000,
            },
            procbuf_layout: describe_layout(ProcBufDimensions {
                axes: 1,
                io_channels: 0,
                domains: 1,
                event_capacity: 8,
            })
            .unwrap(),
            slaves: [ProductSlaveConfig {
                name: "drive",
                position: 0,
                station_address: 0x1000,
                domain_id: 0,
                kind: ProductSlaveKind::Cia402Drive,
                identity: IDENTITY,
            }],
            domains: [ProductDomainConfig {
                name: "motion",
                config: DomainConfig::new(0, 0x1000, 0, 20, 1, 0),
                expected_pdo_count: 8,
                expected_datagram_count: 2,
                expected_wkc: 2,
                input_expected_wkc: 1,
            }],
            pdos: &PDOS,
            datagrams: &DATAGRAMS,
            axes: [ProductAxisConfig {
                name: "axis",
                index: 0,
                slave_position: 0,
                mode: OperatingMode::Csp,
                policy: POLICY,
            }],
        }
    }

    fn observed() -> SlaveRecord {
        SlaveRecord {
            position: 0,
            station_address: 0x1000,
            identity: SlaveIdentity {
                serial: 99,
                ..IDENTITY
            },
            online: true,
            configured: true,
            al_status: AlStatus::new(EthercatState::Op as u16, 0),
            requested_state: EthercatState::Op,
            transition_deadline_ns: 0,
            last_seen_cycle: 1,
        }
    }

    type Active = ActivatedProduct<1, 8, 2, 4, 2, 2, 1>;

    fn activate(
        config: &StaticProductConfig<'_, 1, 1, 1>,
        observed: &[SlaveRecord],
        procbuf: &ProcBuf<1, 0, 1, 8>,
    ) -> Result<Active, ProductActivationError> {
        config.activate::<0, 8, 8, 2, 4, 2, 2>([0xA5; 32], observed, procbuf, 11)
    }

    #[test]
    fn valid_product_activates_all_runtime_evidence() {
        let procbuf = ProcBuf::<1, 0, 1, 8>::new(7, 11);
        let active = activate(&config(), &[observed()], &procbuf).unwrap();

        assert!(active.registry().is_active());
        assert_eq!(active.registry().domain_count(), 1);
        assert_eq!(active.schedule().hyperperiod_ticks(), 1);
        assert_eq!(active.frame_plans()[0].datagram_count(), 2);
        assert_eq!(active.axis_modes(), &[OperatingMode::Csp]);
        assert_eq!(active.axis_policies(), &[POLICY]);
        assert!(
            active.axis_pdo_maps()[0]
                .validate_for(OperatingMode::Csp)
                .is_ok()
        );
    }

    #[test]
    fn identity_procbuf_and_topology_mismatches_fail_before_activation() {
        let procbuf = ProcBuf::<1, 0, 1, 8>::new(7, 11);
        let product = config();
        assert!(matches!(
            product.activate::<0, 8, 8, 2, 4, 2, 2>([0; 32], &[observed()], &procbuf, 11),
            Err(ProductActivationError::ConfigHashMismatch)
        ));
        assert!(activate(&product, &[observed()], &procbuf).is_ok());
        assert!(matches!(
            product.activate::<0, 8, 8, 2, 4, 2, 2>([0xA5; 32], &[observed()], &procbuf, 12),
            Err(ProductActivationError::ProcBufHeader(
                ProcBufHeaderError::BootIdMismatch
            ))
        ));

        let mut wrong = observed();
        wrong.identity.product_code = 99;
        assert!(matches!(
            activate(&product, &[wrong], &procbuf),
            Err(ProductActivationError::SlaveIdentityMismatch { position: 0 })
        ));
        assert!(matches!(
            activate(&product, &[], &procbuf),
            Err(ProductActivationError::SlaveCountMismatch {
                expected: 1,
                actual: 0
            })
        ));
    }

    #[test]
    fn generated_domain_and_axis_contracts_fail_closed() {
        let procbuf = ProcBuf::<1, 0, 1, 8>::new(7, 11);

        let mut wrong_domain = config();
        wrong_domain.domains[0].expected_wkc = 3;
        assert!(matches!(
            activate(&wrong_domain, &[observed()], &procbuf),
            Err(ProductActivationError::DomainEvidenceMismatch { domain_id: 0 })
        ));

        let mut wrong_axis = config();
        wrong_axis.axes[0].mode = OperatingMode::Cst;
        assert!(matches!(
            activate(&wrong_axis, &[observed()], &procbuf),
            Err(ProductActivationError::AxisPdo { axis: 0, .. })
        ));

        let mut wrong_policy = config();
        wrong_policy.axes[0].policy.max_velocity_radians_per_second = 0.0;
        assert!(matches!(
            activate(&wrong_policy, &[observed()], &procbuf),
            Err(ProductActivationError::AxisPolicy { axis: 0, .. })
        ));
    }
}
