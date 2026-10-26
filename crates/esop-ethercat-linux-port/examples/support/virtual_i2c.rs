//! One fixed virtual I2C transaction, polled by the caller after EtherCAT P0.
//! Freshness uses caller-owned monotonic time and never borrows DC quality.
use esop_device::{DeviceError, DeviceKind, DeviceManager, DeviceRecord, DeviceState};

pub const I2C_NACK_FAULT: u32 = 0x4932_0001;
pub const I2C_TIMEOUT_FAULT: u32 = 0x4932_0002;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum I2cOutcome {
    Success(u16),
    Nack,
    NeverCompletes,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum I2cError {
    Device(DeviceError),
    InvalidConfiguration,
    InvalidDeadline,
    Busy,
    ClockRegression,
}
impl From<DeviceError> for I2cError {
    fn from(value: DeviceError) -> Self {
        Self::Device(value)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum I2cPoll {
    Idle,
    Pending { generation: u32 },
    Complete { generation: u32, value: u16 },
    Nack { generation: u32 },
    Timeout { generation: u32 },
    FaultLatched,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Transaction {
    generation: u32,
    ready_at_ns: u64,
    deadline_ns: u64,
    outcome: I2cOutcome,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct I2cSnapshot {
    pub device: DeviceRecord,
    pub address: u8,
    pub pending: bool,
    pub value: Option<u16>,
    pub sampled_at_ns: Option<u64>,
    pub fresh: bool,
    pub nacks: u64,
    pub timeouts: u64,
}

pub struct VirtualI2c {
    devices: DeviceManager<1>,
    id: u32,
    address: u8,
    max_age_ns: u64,
    pending: Option<Transaction>,
    generation: u32,
    last_time_ns: Option<u64>,
    sampled_at_ns: Option<u64>,
    value: Option<u16>,
    nacks: u64,
    timeouts: u64,
}
impl VirtualI2c {
    pub fn new(id: u32, address: u8, max_age_ns: u64) -> Result<Self, I2cError> {
        if address == 0 || address > 0x7f || max_age_ns == 0 {
            return Err(I2cError::InvalidConfiguration);
        }
        let mut devices = DeviceManager::new();
        devices.register(id, DeviceKind::I2cSpi, 0)?;
        Ok(Self {
            devices,
            id,
            address,
            max_age_ns,
            pending: None,
            generation: 0,
            last_time_ns: None,
            sampled_at_ns: None,
            value: None,
            nacks: 0,
            timeouts: 0,
        })
    }
    /// All shared lifecycle phases are explicit; construction alone cannot
    /// qualify a device for cyclic service.
    pub fn initialize(&mut self, cycle: u64) -> Result<(), I2cError> {
        if self.devices.get(self.id)?.state != DeviceState::Probed {
            return Err(I2cError::Device(DeviceError::InvalidTransition));
        }
        self.configure_from_identified(cycle)
    }
    fn configure_from_identified(&mut self, cycle: u64) -> Result<(), I2cError> {
        for state in [
            DeviceState::Identified,
            DeviceState::Configuring,
            DeviceState::Configured,
            DeviceState::Active,
            DeviceState::Cyclic,
        ] {
            self.devices.advance(self.id, state, cycle)?;
        }
        Ok(())
    }
    pub fn submit(
        &mut self,
        now_ns: u64,
        timeout_ns: u64,
        complete_after_ns: u64,
        outcome: I2cOutcome,
    ) -> Result<u32, I2cError> {
        if self.devices.get(self.id)?.state == DeviceState::Fault {
            return Err(I2cError::Device(DeviceError::FaultLatched));
        }
        if self.devices.get(self.id)?.state != DeviceState::Cyclic {
            return Err(I2cError::Device(DeviceError::InvalidTransition));
        }
        if self.pending.is_some() {
            return Err(I2cError::Busy);
        }
        if timeout_ns == 0 {
            return Err(I2cError::InvalidDeadline);
        }
        let deadline_ns = now_ns
            .checked_add(timeout_ns)
            .ok_or(I2cError::InvalidDeadline)?;
        let ready_at_ns = now_ns
            .checked_add(complete_after_ns)
            .ok_or(I2cError::InvalidDeadline)?;
        self.observe_time(now_ns)?;
        self.generation = self.generation.wrapping_add(1).max(1);
        self.pending = Some(Transaction {
            generation: self.generation,
            ready_at_ns,
            deadline_ns,
            outcome,
        });
        Ok(self.generation)
    }
    fn observe_time(&mut self, now_ns: u64) -> Result<(), I2cError> {
        if self.last_time_ns.is_some_and(|previous| now_ns < previous) {
            return Err(I2cError::ClockRegression);
        }
        self.last_time_ns = Some(now_ns);
        Ok(())
    }
    pub fn poll(&mut self, now_ns: u64, cycle: u64) -> Result<I2cPoll, I2cError> {
        self.observe_time(now_ns)?;
        if self.devices.get(self.id)?.state == DeviceState::Fault {
            return Ok(I2cPoll::FaultLatched);
        }
        let Some(transaction) = self.pending else {
            return Ok(I2cPoll::Idle);
        };
        if now_ns >= transaction.deadline_ns {
            self.pending = None;
            self.value = None;
            self.sampled_at_ns = None;
            self.timeouts = self.timeouts.saturating_add(1);
            self.devices.fail(self.id, I2C_TIMEOUT_FAULT, cycle)?;
            return Ok(I2cPoll::Timeout {
                generation: transaction.generation,
            });
        }
        if now_ns < transaction.ready_at_ns || transaction.outcome == I2cOutcome::NeverCompletes {
            return Ok(I2cPoll::Pending {
                generation: transaction.generation,
            });
        }
        self.pending = None;
        match transaction.outcome {
            I2cOutcome::Success(value) => {
                self.value = Some(value);
                self.sampled_at_ns = Some(now_ns);
                Ok(I2cPoll::Complete {
                    generation: transaction.generation,
                    value,
                })
            }
            I2cOutcome::Nack => {
                self.value = None;
                self.sampled_at_ns = None;
                self.nacks = self.nacks.saturating_add(1);
                self.devices.fail(self.id, I2C_NACK_FAULT, cycle)?;
                Ok(I2cPoll::Nack {
                    generation: transaction.generation,
                })
            }
            I2cOutcome::NeverCompletes => Ok(I2cPoll::Pending {
                generation: transaction.generation,
            }),
        }
    }
    pub fn request_recovery(&mut self, cycle: u64) -> Result<(), I2cError> {
        self.devices.request_recovery(self.id, cycle)?;
        self.pending = None;
        self.value = None;
        self.sampled_at_ns = None;
        Ok(())
    }
    pub fn reconfigure(&mut self, cycle: u64) -> Result<(), I2cError> {
        if self.devices.get(self.id)?.state != DeviceState::Recovering {
            return Err(I2cError::Device(DeviceError::NotRecovering));
        }
        self.configure_from_identified(cycle)
    }
    pub fn snapshot(&self, now_ns: u64) -> Result<I2cSnapshot, I2cError> {
        let device = self.devices.get(self.id)?;
        let fresh = device.state == DeviceState::Cyclic
            && self
                .sampled_at_ns
                .is_some_and(|time| now_ns >= time && now_ns - time <= self.max_age_ns);
        Ok(I2cSnapshot {
            device,
            address: self.address,
            pending: self.pending.is_some(),
            value: self.value,
            sampled_at_ns: self.sampled_at_ns,
            fresh,
            nacks: self.nacks,
            timeouts: self.timeouts,
        })
    }
    /// Optional sample work. Call only after Domain/DC/lifecycle P0 is done.
    /// Faults remain latched on every subsequent invocation until the caller
    /// explicitly requests recovery and reconfiguration.
    pub fn service_cycle(&mut self, now_ns: u64, cycle: u64) -> Result<I2cPoll, I2cError> {
        if self.devices.get(self.id)?.state == DeviceState::Fault {
            return Ok(I2cPoll::FaultLatched);
        }
        if self.pending.is_none() {
            let generation =
                self.submit(now_ns, 10_000_000, 1, I2cOutcome::Success(cycle as u16))?;
            return Ok(I2cPoll::Pending { generation });
        }
        self.poll(now_ns, cycle)
    }
}
