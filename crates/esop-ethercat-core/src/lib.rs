#![no_std]

#[cfg(test)]
extern crate std;

mod al;
mod arena;
mod coe;
mod control;
mod dc;
mod diag;
mod dma;
mod domain;
mod domain_registry;
mod engine;
mod frame_pool;
mod mailbox;
mod mapping;
mod mapping_config;
mod op_only;
mod pdo;
mod pdo_config;
mod plan;
mod port;
mod production_service;
mod registers;
mod ring;
mod rx_index;
mod scan;
mod schedule;
mod scheduled_domains;
mod sii;
mod sii_config;
mod sii_discovery;
mod sii_stream;
mod slave;
mod slave_copy;
mod startup;
pub mod wire;

pub use al::{
    AlAction, AlError, AlErrorAcknowledgePolicy, AlErrorAcknowledgeStatus, AlFaultRecord, AlPhase,
    AlProgress, AlTransitionController, AlTransitionRequest, AlTransitionTimeouts,
    ETG1020_DEFAULT_TRANSITION_TIMEOUTS_V1,
};
pub use arena::{Arena, ArenaError};
pub use coe::{
    COE_EMERGENCY_LEN, COE_HEADER_LEN, CoeEmergency, CoeHeader, CoeService, MAX_SDO_DATA,
    MAX_SDO_SEGMENT_BYTES, SDO_DATA_OFFSET, SdoDirection, SdoError, SdoPhase, SdoProgress,
    SdoResponse, SdoTransfer,
};
pub use control::{
    ControlError, ControlExpiry, ControlExpiryHandles, ControlRequest, ControlRequestPool,
    ControlRxConsumer, MAX_CONTROL_PAYLOAD, RegisterOperation, RequestHandle, RequestState,
};
pub use dc::{
    DC_SYNC_DELAY_NS, DcAction, DcActionKind, DcClockAction, DcClockActionKind, DcClockConfig,
    DcClockController, DcClockError, DcClockPhase, DcClockProgrammedSlave, DcClockProgress,
    DcConfig, DcController, DcCyclicConfig, DcCyclicError, DcCyclicSync, DcError, DcLockState,
    DcMonitor, DcPhase, DcProgress, DcSample, DcSyncAction, DcSyncActionKind, DcSyncConfig,
    DcSyncController, DcSyncError, DcSyncMode, DcSyncPhase, DcSyncPlan, DcSyncPlanEntry,
    DcSyncPlanError, DcSyncProgrammedSlave, DcSyncProgress, DcSyncTiming, DcSyncTimingError,
    DcTopology, DcTopologyError, DcTopologyPort, DcTopologySlave,
};
pub use diag::{
    CoeEmergencyEvent, CoeEmergencyQueue, DiagnosticConsumer, Diagnostics, EmergencySink,
    EventCode, EventRecord, EventSeverity,
};
pub use dma::{
    DMA_ALIGNMENT, DmaCacheOps, DmaDescriptorRing, DmaOwner, DmaRingError, DmaRxHandle,
    DmaTxHandle, NoopDmaCache,
};
pub use domain::{Domain, DomainError, DomainQuality, DomainSegment};
pub use domain_registry::{
    DomainConfig, DomainDatagram, DomainDatagramSpec, DomainInfo, DomainRegistry,
    DomainRegistryError, DomainRegistryPhase, PdoEntryHandle, PdoRegistrationRequest,
    RegisteredPdo, SiiDomainRegistration, SiiSegmentDatagramSpec,
};
pub use engine::{
    CycleError, CycleReport, DmaReceiveCycle, EthercatMaster, MasterConfig, RxConsumerMux,
    RxDatagramConsumer,
};
pub use frame_pool::{FrameHandle, FramePool, FramePoolError, FrameSlot};
pub use mailbox::{
    MAILBOX_HEADER_LEN, MAX_MAILBOX_BYTES, MailboxAction, MailboxConfig, MailboxConfigError,
    MailboxController, MailboxDirection, MailboxError, MailboxFrame, MailboxHeader, MailboxPhase,
    MailboxProgress, MailboxProtocol, MailboxRetryPolicy, MailboxStatusBit,
};
pub use mapping::{
    ESC_FMMU_BASE, ESC_FMMU_STRIDE, ESC_SYNC_MANAGER_BASE, ESC_SYNC_MANAGER_STRIDE, FMMU_IMAGE_LEN,
    FmmuConfig, MappingError, MappingSummary, MappingTable, SYNC_MANAGER_IMAGE_LEN,
    SyncManagerConfig,
};
pub use mapping_config::{
    MappingConfigAction, MappingConfigController, MappingConfigError, MappingConfigItem,
    MappingConfigPhase, MappingConfigProgress,
};
pub use op_only::{
    MAX_ESC_SYNC_MANAGERS, OpOnlyProfileError, OpOnlySyncManagerAction,
    OpOnlySyncManagerController, OpOnlySyncManagerError, OpOnlySyncManagerPhase,
    OpOnlySyncManagerProfile, OpOnlySyncManagerProgress, SYNC_MANAGER_ACTIVATION_OFFSET,
    SYNC_MANAGER_ENABLE_FLAG, SYNC_MANAGER_OP_ONLY_FLAG,
};
pub use pdo::{PdoDirection, PdoEntry, PdoError, PdoLayout};
pub use pdo_config::{
    MAX_PDO_SDO_DATA, PdoConfigAction, PdoConfigBatch, PdoConfigBatchError, PdoConfigBatchPhase,
    PdoConfigBatchPlan, PdoConfigBatchPlanError, PdoConfigBatchStatus, PdoConfigController,
    PdoConfigError, PdoConfigJob, PdoConfigPhase, PdoConfigPlan, PdoConfigPlanError,
    PdoConfigProgress, PdoConfigStep, PdoEntrySpec, PdoSdoWrite,
};
pub use plan::{DatagramPlan, FramePlan, FramePlanSet, FramePlanSetError, PlanError};
pub use port::{EthercatDmaTxPort, EthercatPort, LinkState, PortError, RxPoll};
pub use production_service::{
    ScheduledPdoConfiguration, ScheduledPdoConfigurationProgress,
    ScheduledProductionServiceCycleError, ScheduledProductionServiceCycleReport,
    ScheduledProductionServiceFault, ScheduledProductionServiceKind,
    ScheduledProductionServiceProgress, ScheduledProductionServiceRecovery,
    ScheduledProductionServiceScheduler, ScheduledProductionServices,
};
pub use registers::{
    AL_STATUS_WITH_CODE_LEN, BASIC_ESC_INFO_LEN, ESC_AL_CONTROL, ESC_AL_STATUS, ESC_AL_STATUS_CODE,
    ESC_BUILD, ESC_CONFIGURATION, ESC_DC_CUC, ESC_DC_CYCLE0, ESC_DC_CYCLE1,
    ESC_DC_RECEIVE_TIME_LEN, ESC_DC_START0, ESC_DC_SYNC_ACTIVATION, ESC_DC_SYSTEM_DELAY,
    ESC_DC_SYSTEM_DIFF, ESC_DC_SYSTEM_OFFSET, ESC_DC_SYSTEM_TIME, ESC_DC_TIME0, ESC_DC_TIME1,
    ESC_DC_TIME2, ESC_DC_TIME3, ESC_DEVICE_EMULATION, ESC_DL_STATUS, ESC_DL_STATUS_LEN,
    ESC_EEPROM_ADDRESS, ESC_EEPROM_CONTROL, ESC_EEPROM_DATA, ESC_FEATURE_DC_64_BIT,
    ESC_FEATURE_DC_SUPPORTED, ESC_FEATURE_FMMU_BIT_OPERATION, ESC_FEATURES_SUPPORTED,
    ESC_FMMU_COUNT, ESC_PORT_COUNT, ESC_PORT_DESCRIPTOR, ESC_RAM_SIZE, ESC_REVISION,
    ESC_STATION_ADDRESS, ESC_SYNC_MANAGER_COUNT, ESC_TYPE, auto_increment_address, fixed_address,
    register_from_address, station_from_address,
};
pub use ring::{RingError, SpscConsumer, SpscProducer, SpscRing};
pub use rx_index::{
    RxExpectation, RxExpiry, RxExpiryIndices, RxIndexEntry, RxIndexError, RxIndexTable, RxMatch,
    RxResponse, RxSlotState, RxWorkingCounterPolicy,
};
pub use scan::{
    EscDcRange, ScanAction, ScanController, ScanDcCapabilities, ScanError, ScanPhase, ScanPortLink,
    ScanProgress, ScanRecord,
};
pub use schedule::{ScheduleDomain, ScheduleError, ScheduleSlot, ScheduleTable};
pub use scheduled_domains::{
    ScheduledControlCycleError, ScheduledControlCycleReport, ScheduledDomainBank,
    ScheduledDomainEntry, ScheduledDomainError, ScheduledDomainRx, ScheduledMailboxCycleError,
    ScheduledMailboxCycleReport, ScheduledMailboxReceiveReport, ScheduledMailboxTxError,
    ScheduledMailboxTxReport, ScheduledProcessFrameError, ScheduledProcessInputEntry,
    ScheduledProcessInputPlanError, ScheduledProcessInputs, ScheduledProcessTxError,
    ScheduledProcessTxFailure, ScheduledProcessTxReport, ScheduledReceiveError,
    ScheduledReceiveReport, ScheduledServiceFrameError, ScheduledServiceTxError,
    ScheduledServiceTxFailure, ScheduledServiceTxReport,
};
pub use sii::{
    EEPROM_BUSY, EEPROM_ERROR_MASK, EEPROM_READ_COMMAND, SII_CATEGORY_DC, SII_CATEGORY_END,
    SII_CATEGORY_FMMU, SII_CATEGORY_GENERAL, SII_CATEGORY_RX_PDO, SII_CATEGORY_STRINGS,
    SII_CATEGORY_SYNC_MANAGER, SII_CATEGORY_TX_PDO, SII_MAILBOX_PROTOCOL_AOE,
    SII_MAILBOX_PROTOCOL_COE, SII_MAILBOX_PROTOCOL_EOE, SII_MAILBOX_PROTOCOL_FOE,
    SII_MAILBOX_PROTOCOL_SOE, SII_MAILBOX_PROTOCOL_VOE, SII_MAILBOX_PROTOCOLS_WORD,
    SII_PRODUCT_CODE_WORD, SII_REVISION_WORD, SII_SERIAL_WORD, SII_STANDARD_MAILBOX_WORD_COUNT,
    SII_STANDARD_RECEIVE_MAILBOX_OFFSET_WORD, SII_STANDARD_RECEIVE_MAILBOX_SIZE_WORD,
    SII_STANDARD_SEND_MAILBOX_OFFSET_WORD, SII_STANDARD_SEND_MAILBOX_SIZE_WORD, SII_VENDOR_ID_WORD,
    SiiAction, SiiBlockError, SiiBlockReader, SiiBlockRequest, SiiCategory, SiiCategoryError,
    SiiCategoryReader, SiiDcCategory, SiiDcMode, SiiDcModeDescriptor, SiiDcModeExpectation,
    SiiError, SiiIdentityReader, SiiMailboxError, SiiMailboxProtocols, SiiPdoCategory, SiiPdoEntry,
    SiiPhase, SiiProgress, SiiStandardMailbox, SiiStringsCategory, SiiSyncManager,
    SiiSyncManagerCategory, find_sii_dc_mode,
};
pub use sii_config::{
    SII_CONFIGURATION_SIGNATURE_SCHEMA, SiiConfigurationCandidate, SiiConfigurationError,
    SiiConfigurationProgress, SiiConfigurationSignature, SiiConfigurationSignatureBuilder,
    SiiConfigurationSignatureError, SiiDomainProjection, SiiProcessDataSegment,
};
pub use sii_discovery::{
    SiiDiscoveryController, SiiDiscoveryError, SiiDiscoveryPhase, SiiDiscoveryRequest,
    SiiStreamDiscoveryController, SiiStreamDiscoveryRequest,
};
pub use sii_stream::{
    SII_CATEGORY_START_WORD, SiiCategoryStreamError, SiiCategoryStreamPhase,
    SiiCategoryStreamProgress, SiiCategoryStreamReader, SiiCategoryStreamRequest,
};
pub use slave::{
    AlStatus, EthercatState, SlaveIdentity, SlaveRecord, SlaveTable, SlaveTableError, next_state,
};
pub use slave_copy::{SlaveCopyError, SlaveCopyOutcome, SlaveCopyPlan, SlaveCopyStatus};
pub use startup::{
    ExpectedSlave, STARTUP_SII_IMAGE_BYTE_CAPACITY, STARTUP_SII_IMAGE_WORD_CAPACITY,
    STARTUP_SII_PDO_ENTRY_CAPACITY, StartupAction, StartupAlFault, StartupConfig,
    StartupConfigurationServices, StartupController, StartupDcRequirement, StartupError,
    StartupPhase, StartupProgress, StartupReferenceClock, StartupSlaveProfile,
};
