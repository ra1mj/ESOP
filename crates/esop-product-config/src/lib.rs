#![no_std]

//! Runtime attachment for product configurations emitted by `esop-cfggen`.
//!
//! The generated module is static data only. Activation rebuilds every
//! Domain, schedule, frame-plan and CiA 402 map through the owning runtime
//! crates, then returns the complete frozen result as one value.

pub use esop_ethercat_core::wire::Command;
pub use esop_ethercat_core::{
    DomainConfig, DomainDatagramSpec, DomainInfo, DomainRegistry, DomainRegistryError,
    FramePlanSet, FramePlanSetError, MailboxConfig, PdoConfigBatch, PdoConfigBatchError,
    PdoConfigBatchPhase, PdoConfigBatchPlan, PdoConfigBatchPlanError, PdoConfigBatchStatus,
    PdoConfigJob, PdoConfigPlan, PdoConfigPlanError, PdoDirection, PdoEntry, PdoEntrySpec,
    PdoRegistrationRequest, PdoSdoWrite, ScheduleTable, SlaveIdentity, SlaveRecord,
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
pub const MAX_PRODUCT_PDO_ENTRIES_PER_DOMAIN: usize = 256;
const MAX_PRODUCT_SYNC_MANAGERS: usize = 16;

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
pub struct ProductMailboxBinding {
    slave_position: u16,
    mailbox_config: MailboxConfig,
}

impl ProductMailboxBinding {
    pub const fn new(slave_position: u16, mailbox_config: MailboxConfig) -> Self {
        Self {
            slave_position,
            mailbox_config,
        }
    }

    pub const fn slave_position(&self) -> u16 {
        self.slave_position
    }

    pub const fn mailbox_config(&self) -> MailboxConfig {
        self.mailbox_config
    }
}

pub struct ProductPdoStartupPlan<const OPS: usize> {
    station_address: u16,
    plan: PdoConfigPlan<OPS>,
}

impl<const OPS: usize> ProductPdoStartupPlan<OPS> {
    pub const fn station_address(&self) -> u16 {
        self.station_address
    }

    pub const fn plan(&self) -> &PdoConfigPlan<OPS> {
        &self.plan
    }

    pub fn into_plan(self) -> PdoConfigPlan<OPS> {
        self.plan
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductPdoPlanError {
    UnknownSlave {
        position: u16,
    },
    PdoDomainMismatch {
        pdo_index: usize,
    },
    InvalidSyncManager {
        pdo_index: usize,
        sync_manager: u8,
    },
    MappingSyncManagerMismatch {
        mapping_index: u16,
        expected: u8,
        actual: u8,
    },
    MappingDirectionMismatch {
        mapping_index: u16,
        expected: PdoDirection,
        actual: PdoDirection,
    },
    PdoCapacityExceeded {
        position: u16,
    },
    Plan(PdoConfigPlanError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductPdoBatchError {
    MissingMailboxBinding {
        position: u16,
    },
    DuplicateMailboxBinding {
        position: u16,
    },
    UnknownMailboxBinding {
        position: u16,
    },
    SlavePlan {
        position: u16,
        error: ProductPdoPlanError,
    },
    Batch(PdoConfigBatchPlanError),
}

#[derive(Clone, Copy)]
struct ProductPdoMappingGroup {
    mapping_index: u16,
    sync_manager: u8,
    direction: PdoDirection,
}

impl ProductPdoMappingGroup {
    const EMPTY: Self = Self {
        mapping_index: 0,
        sync_manager: 0,
        direction: PdoDirection::Rx,
    };
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
    pub fn build_pdo_configuration_batch<const JOBS: usize, const OPS: usize>(
        &self,
        mailbox_bindings: &[ProductMailboxBinding],
    ) -> Result<PdoConfigBatchPlan<JOBS, OPS>, ProductPdoBatchError> {
        for (binding_index, binding) in mailbox_bindings.iter().enumerate() {
            if !self
                .slaves
                .iter()
                .any(|slave| slave.position == binding.slave_position)
            {
                return Err(ProductPdoBatchError::UnknownMailboxBinding {
                    position: binding.slave_position,
                });
            }
            if mailbox_bindings[..binding_index]
                .iter()
                .any(|existing| existing.slave_position == binding.slave_position)
            {
                return Err(ProductPdoBatchError::DuplicateMailboxBinding {
                    position: binding.slave_position,
                });
            }
        }

        let mut batch = PdoConfigBatchPlan::new();
        for slave in self.slaves {
            let binding = mailbox_bindings
                .iter()
                .find(|binding| binding.slave_position == slave.position)
                .ok_or(ProductPdoBatchError::MissingMailboxBinding {
                    position: slave.position,
                })?;
            let startup = self
                .build_pdo_startup_plan::<OPS>(slave.position)
                .map_err(|error| ProductPdoBatchError::SlavePlan {
                    position: slave.position,
                    error,
                })?;
            batch
                .push(PdoConfigJob::new(
                    startup.station_address(),
                    startup.into_plan(),
                    binding.mailbox_config,
                ))
                .map_err(ProductPdoBatchError::Batch)?;
        }
        batch.validate().map_err(ProductPdoBatchError::Batch)?;
        Ok(batch)
    }

    pub fn build_pdo_startup_plan<const OPS: usize>(
        &self,
        slave_position: u16,
    ) -> Result<ProductPdoStartupPlan<OPS>, ProductPdoPlanError> {
        let slave = self
            .slaves
            .iter()
            .find(|slave| slave.position == slave_position)
            .ok_or(ProductPdoPlanError::UnknownSlave {
                position: slave_position,
            })?;

        let mut groups = [ProductPdoMappingGroup::EMPTY; MAX_PRODUCT_PDO_ENTRIES_PER_DOMAIN];
        let mut group_count = 0;
        for (pdo_index, pdo) in self.pdos.iter().copied().enumerate() {
            if pdo.request.slave_position != slave_position {
                continue;
            }
            if pdo.domain_id != slave.domain_id {
                return Err(ProductPdoPlanError::PdoDomainMismatch { pdo_index });
            }
            if usize::from(pdo.sync_manager) >= MAX_PRODUCT_SYNC_MANAGERS {
                return Err(ProductPdoPlanError::InvalidSyncManager {
                    pdo_index,
                    sync_manager: pdo.sync_manager,
                });
            }
            if let Some(group) = groups[..group_count]
                .iter()
                .find(|group| group.mapping_index == pdo.assignment_index)
            {
                if group.sync_manager != pdo.sync_manager {
                    return Err(ProductPdoPlanError::MappingSyncManagerMismatch {
                        mapping_index: pdo.assignment_index,
                        expected: group.sync_manager,
                        actual: pdo.sync_manager,
                    });
                }
                if group.direction != pdo.request.direction {
                    return Err(ProductPdoPlanError::MappingDirectionMismatch {
                        mapping_index: pdo.assignment_index,
                        expected: group.direction,
                        actual: pdo.request.direction,
                    });
                }
                continue;
            }
            if group_count == groups.len() {
                return Err(ProductPdoPlanError::PdoCapacityExceeded {
                    position: slave_position,
                });
            }
            groups[group_count] = ProductPdoMappingGroup {
                mapping_index: pdo.assignment_index,
                sync_manager: pdo.sync_manager,
                direction: pdo.request.direction,
            };
            group_count += 1;
        }

        let mut plan = PdoConfigPlan::new();
        let mut processed_sync_managers = [false; MAX_PRODUCT_SYNC_MANAGERS];
        let mut entries = [PdoEntrySpec::new(0, 0, 1); MAX_PRODUCT_PDO_ENTRIES_PER_DOMAIN];
        let mut mapping_indexes = [0u16; MAX_PRODUCT_PDO_ENTRIES_PER_DOMAIN];

        for sync_manager in 0..MAX_PRODUCT_SYNC_MANAGERS {
            if groups[..group_count]
                .iter()
                .filter(|group| usize::from(group.sync_manager) == sync_manager)
                .count()
                > u8::MAX as usize
            {
                return Err(ProductPdoPlanError::Plan(
                    PdoConfigPlanError::CountOutOfBounds,
                ));
            }
        }

        for group in &groups[..group_count] {
            let sync_manager = usize::from(group.sync_manager);
            if processed_sync_managers[sync_manager] {
                continue;
            }
            processed_sync_managers[sync_manager] = true;
            let assignment_index = 0x1C10u16 + u16::from(group.sync_manager);
            plan.push(
                PdoSdoWrite::new(assignment_index, 0, &[0]).map_err(ProductPdoPlanError::Plan)?,
            )
            .map_err(ProductPdoPlanError::Plan)?;

            let mut mapping_count = 0;
            for mapping in groups[..group_count]
                .iter()
                .filter(|mapping| mapping.sync_manager == group.sync_manager)
            {
                let mut entry_count = 0;
                for pdo in self.pdos.iter().copied().filter(|pdo| {
                    pdo.request.slave_position == slave_position
                        && pdo.assignment_index == mapping.mapping_index
                }) {
                    if entry_count == entries.len() {
                        return Err(ProductPdoPlanError::PdoCapacityExceeded {
                            position: slave_position,
                        });
                    }
                    entries[entry_count] = PdoEntrySpec::new(
                        pdo.request.index,
                        pdo.request.subindex,
                        pdo.request.bit_length,
                    );
                    entry_count += 1;
                }
                plan.append_mapping(mapping.mapping_index, &entries[..entry_count])
                    .map_err(ProductPdoPlanError::Plan)?;
                mapping_indexes[mapping_count] = mapping.mapping_index;
                mapping_count += 1;
            }

            for (offset, mapping_index) in mapping_indexes[..mapping_count].iter().enumerate() {
                let subindex = u8::try_from(offset + 1)
                    .map_err(|_| ProductPdoPlanError::Plan(PdoConfigPlanError::CountOutOfBounds))?;
                plan.push(
                    PdoSdoWrite::new(assignment_index, subindex, &mapping_index.to_le_bytes())
                        .map_err(ProductPdoPlanError::Plan)?,
                )
                .map_err(ProductPdoPlanError::Plan)?;
            }
            let assignment_count = u8::try_from(mapping_count)
                .map_err(|_| ProductPdoPlanError::Plan(PdoConfigPlanError::CountOutOfBounds))?;
            plan.push(
                PdoSdoWrite::new(assignment_index, 0, &[assignment_count])
                    .map_err(ProductPdoPlanError::Plan)?,
            )
            .map_err(ProductPdoPlanError::Plan)?;
        }

        Ok(ProductPdoStartupPlan {
            station_address: slave.station_address,
            plan,
        })
    }

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

    #[test]
    fn product_pdo_plan_disables_assignments_before_mapping_and_republishes_them() {
        let startup = config().build_pdo_startup_plan::<18>(0).unwrap();
        assert_eq!(startup.station_address(), 0x1000);
        let writes = startup.plan().writes();
        assert_eq!(writes.len(), 18);

        assert_eq!(writes[0], PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap());
        assert_eq!(writes[1], PdoSdoWrite::new(0x1600, 0, &[0]).unwrap());
        assert_eq!(
            writes[2],
            PdoSdoWrite::new(0x1600, 1, &0x1000_6040u32.to_le_bytes()).unwrap()
        );
        assert_eq!(writes[5], PdoSdoWrite::new(0x1600, 0, &[3]).unwrap());
        assert_eq!(
            writes[6],
            PdoSdoWrite::new(0x1C12, 1, &0x1600u16.to_le_bytes()).unwrap()
        );
        assert_eq!(writes[7], PdoSdoWrite::new(0x1C12, 0, &[1]).unwrap());

        assert_eq!(writes[8], PdoSdoWrite::new(0x1C13, 0, &[0]).unwrap());
        assert_eq!(writes[9], PdoSdoWrite::new(0x1A00, 0, &[0]).unwrap());
        assert_eq!(
            writes[10],
            PdoSdoWrite::new(0x1A00, 1, &0x1000_6041u32.to_le_bytes()).unwrap()
        );
        assert_eq!(writes[15], PdoSdoWrite::new(0x1A00, 0, &[5]).unwrap());
        assert_eq!(
            writes[16],
            PdoSdoWrite::new(0x1C13, 1, &0x1A00u16.to_le_bytes()).unwrap()
        );
        assert_eq!(writes[17], PdoSdoWrite::new(0x1C13, 0, &[1]).unwrap());
    }

    #[test]
    fn product_pdo_plan_rejects_invalid_generated_metadata() {
        assert!(matches!(
            config().build_pdo_startup_plan::<18>(99),
            Err(ProductPdoPlanError::UnknownSlave { position: 99 })
        ));
        assert!(matches!(
            config().build_pdo_startup_plan::<17>(0),
            Err(ProductPdoPlanError::Plan(
                PdoConfigPlanError::CapacityExceeded
            ))
        ));

        let mut wrong_domain_entries = PDOS;
        wrong_domain_entries[0].domain_id = 1;
        let mut wrong_domain = config();
        wrong_domain.pdos = &wrong_domain_entries;
        assert!(matches!(
            wrong_domain.build_pdo_startup_plan::<18>(0),
            Err(ProductPdoPlanError::PdoDomainMismatch { pdo_index: 0 })
        ));

        let mut wrong_sync_entries = PDOS;
        wrong_sync_entries[0].sync_manager = 16;
        let mut wrong_sync = config();
        wrong_sync.pdos = &wrong_sync_entries;
        assert!(matches!(
            wrong_sync.build_pdo_startup_plan::<18>(0),
            Err(ProductPdoPlanError::InvalidSyncManager {
                pdo_index: 0,
                sync_manager: 16,
            })
        ));

        let mut inconsistent_sync_entries = PDOS;
        inconsistent_sync_entries[1].sync_manager = 3;
        let mut inconsistent_sync = config();
        inconsistent_sync.pdos = &inconsistent_sync_entries;
        assert!(matches!(
            inconsistent_sync.build_pdo_startup_plan::<18>(0),
            Err(ProductPdoPlanError::MappingSyncManagerMismatch {
                mapping_index: 0x1600,
                expected: 2,
                actual: 3,
            })
        ));

        let mut inconsistent_direction_entries = PDOS;
        inconsistent_direction_entries[1].request.direction = PdoDirection::Tx;
        let mut inconsistent_direction = config();
        inconsistent_direction.pdos = &inconsistent_direction_entries;
        assert!(matches!(
            inconsistent_direction.build_pdo_startup_plan::<18>(0),
            Err(ProductPdoPlanError::MappingDirectionMismatch {
                mapping_index: 0x1600,
                expected: PdoDirection::Rx,
                actual: PdoDirection::Tx,
            })
        ));
    }

    #[test]
    fn product_pdo_plan_enforces_shared_fixed_scratch_capacity() {
        let mut entries =
            [pdo(0x6040, PdoDirection::Rx, 0, 16, false); MAX_PRODUCT_PDO_ENTRIES_PER_DOMAIN + 1];
        for (index, entry) in entries.iter_mut().enumerate() {
            entry.assignment_index = 0x1600 + index as u16;
        }
        let mut product = config();
        product.pdos = &entries;

        assert!(matches!(
            product.build_pdo_startup_plan::<1024>(0),
            Err(ProductPdoPlanError::PdoCapacityExceeded { position: 0 })
        ));
    }

    #[test]
    fn product_pdo_plan_rejects_more_than_255_mappings_per_sync_manager() {
        let mut entries = [pdo(0x6040, PdoDirection::Rx, 0, 16, false); u8::MAX as usize + 1];
        for (index, entry) in entries.iter_mut().enumerate() {
            entry.assignment_index = 0x1600 + index as u16;
        }
        let mut product = config();
        product.pdos = &entries;

        assert!(matches!(
            product.build_pdo_startup_plan::<2048>(0),
            Err(ProductPdoPlanError::Plan(
                PdoConfigPlanError::CountOutOfBounds
            ))
        ));
    }
}
