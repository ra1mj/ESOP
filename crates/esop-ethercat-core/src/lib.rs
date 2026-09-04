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
mod pdo;
mod pdo_config;
mod plan;
mod port;
mod registers;
mod ring;
mod rx_index;
mod scan;
mod schedule;
mod sii;
mod sii_config;
mod sii_discovery;
mod slave;
mod startup;
pub mod wire;

pub use al::{AlAction, AlError, AlPhase, AlProgress, AlTransitionController, AlTransitionRequest};
pub use arena::{Arena, ArenaError};
pub use coe::{
    COE_EMERGENCY_LEN, COE_HEADER_LEN, CoeEmergency, CoeHeader, CoeService, MAX_SDO_DATA,
    MAX_SDO_SEGMENT_BYTES, SDO_DATA_OFFSET, SdoDirection, SdoError, SdoPhase, SdoProgress,
    SdoResponse, SdoTransfer,
};
pub use control::{
    ControlError, ControlRequest, ControlRequestPool, ControlRxConsumer, MAX_CONTROL_PAYLOAD,
    RegisterOperation, RequestHandle, RequestState,
};
pub use dc::{
    DC_SYNC_DELAY_NS, DcAction, DcActionKind, DcConfig, DcController, DcCyclicConfig,
    DcCyclicError, DcCyclicSync, DcError, DcLockState, DcMonitor, DcPhase, DcProgress, DcSample,
    DcSyncMode,
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
    MAILBOX_HEADER_LEN, MAX_MAILBOX_BYTES, MailboxAction, MailboxConfig, MailboxController,
    MailboxError, MailboxFrame, MailboxHeader, MailboxPhase, MailboxProgress, MailboxProtocol,
    MailboxRetryPolicy, MailboxStatusBit,
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
pub use pdo::{PdoDirection, PdoEntry, PdoError, PdoLayout};
pub use pdo_config::{
    MAX_PDO_SDO_DATA, PdoConfigAction, PdoConfigController, PdoConfigError, PdoConfigPhase,
    PdoConfigPlan, PdoConfigPlanError, PdoConfigProgress, PdoEntrySpec, PdoSdoWrite,
};
pub use plan::{DatagramPlan, FramePlan, FramePlanSet, FramePlanSetError, PlanError};
pub use port::{EthercatDmaTxPort, EthercatPort, LinkState, PortError, RxPoll};
pub use registers::{
    AL_STATUS_WITH_CODE_LEN, BASIC_ESC_INFO_LEN, ESC_AL_CONTROL, ESC_AL_STATUS, ESC_AL_STATUS_CODE,
    ESC_BUILD, ESC_DC_CUC, ESC_DC_CYCLE0, ESC_DC_CYCLE1, ESC_DC_START0, ESC_DC_SYNC_ACTIVATION,
    ESC_DC_SYSTEM_DELAY, ESC_DC_SYSTEM_DIFF, ESC_DC_SYSTEM_OFFSET, ESC_DC_SYSTEM_TIME,
    ESC_DC_TIME0, ESC_DC_TIME1, ESC_DC_TIME2, ESC_DC_TIME3, ESC_DL_STATUS, ESC_EEPROM_ADDRESS,
    ESC_EEPROM_CONTROL, ESC_EEPROM_DATA, ESC_FMMU_COUNT, ESC_PORT_DESCRIPTOR, ESC_RAM_SIZE,
    ESC_REVISION, ESC_STATION_ADDRESS, ESC_SYNC_MANAGER_COUNT, ESC_TYPE, auto_increment_address,
    fixed_address, register_from_address, station_from_address,
};
pub use ring::{RingError, SpscConsumer, SpscProducer, SpscRing};
pub use rx_index::{
    RxExpectation, RxExpiry, RxExpiryIndices, RxIndexEntry, RxIndexError, RxIndexTable, RxMatch,
    RxResponse, RxSlotState,
};
pub use scan::{ScanAction, ScanController, ScanError, ScanPhase, ScanProgress, ScanRecord};
pub use schedule::{ScheduleDomain, ScheduleError, ScheduleSlot, ScheduleTable};
pub use sii::{
    EEPROM_BUSY, EEPROM_ERROR_MASK, EEPROM_READ_COMMAND, SII_CATEGORY_DC, SII_CATEGORY_END,
    SII_CATEGORY_FMMU, SII_CATEGORY_GENERAL, SII_CATEGORY_RX_PDO, SII_CATEGORY_STRINGS,
    SII_CATEGORY_SYNC_MANAGER, SII_CATEGORY_TX_PDO, SII_PRODUCT_CODE_WORD, SII_REVISION_WORD,
    SII_SERIAL_WORD, SII_VENDOR_ID_WORD, SiiAction, SiiBlockError, SiiBlockReader, SiiBlockRequest,
    SiiCategory, SiiCategoryError, SiiCategoryReader, SiiError, SiiIdentityReader, SiiPdoCategory,
    SiiPdoEntry, SiiPhase, SiiProgress, SiiSyncManager, SiiSyncManagerCategory,
};
pub use sii_config::{
    SiiConfigurationCandidate, SiiConfigurationError, SiiConfigurationProgress,
    SiiDomainProjection, SiiProcessDataSegment,
};
pub use sii_discovery::{
    SiiDiscoveryController, SiiDiscoveryError, SiiDiscoveryPhase, SiiDiscoveryRequest,
};
pub use slave::{
    AlStatus, EthercatState, SlaveIdentity, SlaveRecord, SlaveTable, SlaveTableError, next_state,
};
pub use startup::{
    ExpectedSlave, StartupAction, StartupConfig, StartupController, StartupError, StartupPhase,
    StartupProgress,
};
