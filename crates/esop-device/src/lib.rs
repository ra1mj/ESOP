#![no_std]

//! Common lifecycle state contract for EtherCAT profiles and non-EtherCAT
//! peripherals. Transport and profile-specific operations remain outside this
//! crate; this layer only owns bounded state and explicit recovery rules.

pub const MAX_DEVICE_ID: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DeviceKind {
    EthercatDrive = 0,
    EthercatIo = 1,
    EthercatSensor = 2,
    CanFd = 3,
    I2cSpi = 4,
    UartUsb = 5,
    Gpio = 6,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DeviceState {
    Empty = 0,
    Probed = 1,
    Identified = 2,
    Configuring = 3,
    Configured = 4,
    Active = 5,
    Cyclic = 6,
    Degraded = 7,
    Fault = 8,
    Recovering = 9,
    Inactive = 10,
}

impl DeviceState {
    pub const fn is_registered(self) -> bool {
        !matches!(self, Self::Empty)
    }

    pub const fn is_cyclic(self) -> bool {
        matches!(self, Self::Cyclic)
    }

    pub const fn is_faulted(self) -> bool {
        matches!(self, Self::Fault)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceError {
    InvalidId,
    CapacityExceeded,
    DuplicateId,
    NotFound,
    InvalidTransition,
    FaultLatched,
    NotRecovering,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct DeviceRecord {
    pub id: u32,
    pub kind: DeviceKind,
    pub state: DeviceState,
    pub reserved: [u8; 2],
    pub generation: u32,
    pub last_cycle: u64,
    pub fault_code: u32,
    pub transition_count: u32,
}

impl DeviceRecord {
    pub const EMPTY: Self = Self {
        id: 0,
        kind: DeviceKind::EthercatDrive,
        state: DeviceState::Empty,
        reserved: [0; 2],
        generation: 0,
        last_cycle: 0,
        fault_code: 0,
        transition_count: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceTransition {
    pub id: u32,
    pub from: DeviceState,
    pub to: DeviceState,
    pub cycle: u64,
    pub fault_code: u32,
}

/// Fixed-capacity registry and lifecycle coordinator. One realtime owner is
/// expected to call mutating methods; readers can consume copied records.
pub struct DeviceManager<const DEVICES: usize> {
    records: [DeviceRecord; DEVICES],
    count: usize,
    lifecycle_epoch: u32,
}

impl<const DEVICES: usize> DeviceManager<DEVICES> {
    pub const fn new() -> Self {
        Self {
            records: [DeviceRecord::EMPTY; DEVICES],
            count: 0,
            lifecycle_epoch: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub const fn lifecycle_epoch(&self) -> u32 {
        self.lifecycle_epoch
    }

    pub fn records(&self) -> &[DeviceRecord] {
        &self.records[..self.count]
    }

    pub fn get(&self, id: u32) -> Result<DeviceRecord, DeviceError> {
        self.records
            .iter()
            .take(self.count)
            .copied()
            .find(|record| record.id == id)
            .ok_or(DeviceError::NotFound)
    }

    pub fn register(&mut self, id: u32, kind: DeviceKind, cycle: u64) -> Result<(), DeviceError> {
        if id == 0 || id == MAX_DEVICE_ID {
            return Err(DeviceError::InvalidId);
        }
        if self.records().iter().any(|record| record.id == id) {
            return Err(DeviceError::DuplicateId);
        }
        if DEVICES == 0 || self.count == DEVICES {
            return Err(DeviceError::CapacityExceeded);
        }
        self.records[self.count] = DeviceRecord {
            id,
            kind,
            state: DeviceState::Probed,
            reserved: [0; 2],
            generation: self.lifecycle_epoch,
            last_cycle: cycle,
            fault_code: 0,
            transition_count: 0,
        };
        self.count += 1;
        Ok(())
    }

    pub fn advance(
        &mut self,
        id: u32,
        target: DeviceState,
        cycle: u64,
    ) -> Result<DeviceTransition, DeviceError> {
        let index = self.index(id)?;
        let current = self.records[index].state;
        if current == target {
            return Ok(DeviceTransition {
                id,
                from: current,
                to: target,
                cycle,
                fault_code: self.records[index].fault_code,
            });
        }
        if !allowed_transition(current, target) {
            return Err(if current == DeviceState::Fault {
                DeviceError::FaultLatched
            } else {
                DeviceError::InvalidTransition
            });
        }
        self.records[index].state = target;
        self.records[index].last_cycle = cycle;
        self.records[index].transition_count =
            self.records[index].transition_count.saturating_add(1);
        if target != DeviceState::Fault {
            self.records[index].fault_code = 0;
        }
        if target == DeviceState::Recovering {
            self.records[index].generation = self.records[index].generation.saturating_add(1);
        }
        if current == DeviceState::Inactive && target == DeviceState::Probed {
            self.records[index].generation = self.records[index].generation.saturating_add(1);
        }
        self.lifecycle_epoch = self.lifecycle_epoch.wrapping_add(1);
        Ok(DeviceTransition {
            id,
            from: current,
            to: target,
            cycle,
            fault_code: self.records[index].fault_code,
        })
    }

    pub fn fail(
        &mut self,
        id: u32,
        fault_code: u32,
        cycle: u64,
    ) -> Result<DeviceTransition, DeviceError> {
        let index = self.index(id)?;
        let current = self.records[index].state;
        if current == DeviceState::Fault {
            return Err(DeviceError::FaultLatched);
        }
        if !current.is_registered() || current == DeviceState::Inactive {
            return Err(DeviceError::InvalidTransition);
        }
        self.records[index].fault_code = fault_code;
        let transition = self.advance(id, DeviceState::Fault, cycle)?;
        Ok(DeviceTransition {
            fault_code,
            ..transition
        })
    }

    pub fn request_recovery(
        &mut self,
        id: u32,
        cycle: u64,
    ) -> Result<DeviceTransition, DeviceError> {
        self.advance(id, DeviceState::Recovering, cycle)
    }

    pub fn all_cyclic(&self) -> bool {
        !self.is_empty() && self.records().iter().all(|record| record.state.is_cyclic())
    }

    fn index(&self, id: u32) -> Result<usize, DeviceError> {
        self.records()
            .iter()
            .position(|record| record.id == id)
            .ok_or(DeviceError::NotFound)
    }
}

impl<const DEVICES: usize> Default for DeviceManager<DEVICES> {
    fn default() -> Self {
        Self::new()
    }
}

const fn allowed_transition(from: DeviceState, to: DeviceState) -> bool {
    match from {
        DeviceState::Probed => matches!(to, DeviceState::Identified | DeviceState::Fault),
        DeviceState::Identified => matches!(to, DeviceState::Configuring | DeviceState::Fault),
        DeviceState::Configuring => matches!(to, DeviceState::Configured | DeviceState::Fault),
        DeviceState::Configured => matches!(to, DeviceState::Active | DeviceState::Fault),
        DeviceState::Active => matches!(
            to,
            DeviceState::Cyclic
                | DeviceState::Degraded
                | DeviceState::Fault
                | DeviceState::Inactive
        ),
        DeviceState::Cyclic => matches!(
            to,
            DeviceState::Degraded | DeviceState::Fault | DeviceState::Inactive
        ),
        DeviceState::Degraded => matches!(to, DeviceState::Recovering | DeviceState::Fault),
        DeviceState::Fault => matches!(to, DeviceState::Recovering | DeviceState::Inactive),
        DeviceState::Recovering => matches!(to, DeviceState::Identified | DeviceState::Fault),
        DeviceState::Inactive => matches!(to, DeviceState::Probed),
        DeviceState::Empty => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_setup<const DEVICES: usize>(manager: &mut DeviceManager<DEVICES>, id: u32) {
        manager.register(id, DeviceKind::EthercatDrive, 1).unwrap();
        for state in [
            DeviceState::Identified,
            DeviceState::Configuring,
            DeviceState::Configured,
            DeviceState::Active,
            DeviceState::Cyclic,
        ] {
            manager.advance(id, state, 2).unwrap();
        }
    }

    #[test]
    fn device_lifecycle_requires_each_explicit_phase() {
        let mut manager = DeviceManager::<2>::new();
        complete_setup(&mut manager, 7);
        assert!(manager.all_cyclic());
        assert_eq!(manager.get(7).unwrap().state, DeviceState::Cyclic);
        assert_eq!(manager.get(7).unwrap().transition_count, 5);
        let generation = manager.get(7).unwrap().generation;
        manager.advance(7, DeviceState::Inactive, 3).unwrap();
        assert!(!manager.all_cyclic());
        manager.advance(7, DeviceState::Probed, 4).unwrap();
        assert_eq!(manager.get(7).unwrap().generation, generation + 1);
        assert_eq!(manager.get(7).unwrap().fault_code, 0);
    }

    #[test]
    fn fault_requires_explicit_recovery_and_reconfiguration() {
        let mut manager = DeviceManager::<1>::new();
        complete_setup(&mut manager, 7);
        let failed = manager.fail(7, 0xDEAD, 10).unwrap();
        assert_eq!(failed.from, DeviceState::Cyclic);
        assert_eq!(manager.get(7).unwrap().fault_code, 0xDEAD);
        assert_eq!(
            manager.advance(7, DeviceState::Active, 11),
            Err(DeviceError::FaultLatched)
        );
        manager.request_recovery(7, 12).unwrap();
        manager.advance(7, DeviceState::Identified, 13).unwrap();
        manager.advance(7, DeviceState::Configuring, 14).unwrap();
        manager.advance(7, DeviceState::Configured, 15).unwrap();
        manager.advance(7, DeviceState::Active, 16).unwrap();
        manager.advance(7, DeviceState::Cyclic, 17).unwrap();
        assert!(manager.all_cyclic());
    }

    #[test]
    fn capacity_duplicate_and_illegal_transition_are_rejected() {
        let mut manager = DeviceManager::<1>::new();
        assert_eq!(
            manager.register(0, DeviceKind::CanFd, 1),
            Err(DeviceError::InvalidId)
        );
        manager.register(1, DeviceKind::CanFd, 1).unwrap();
        assert_eq!(
            manager.register(1, DeviceKind::Gpio, 1),
            Err(DeviceError::DuplicateId)
        );
        assert_eq!(
            manager.register(2, DeviceKind::Gpio, 1),
            Err(DeviceError::CapacityExceeded)
        );
        assert_eq!(
            manager.advance(1, DeviceState::Cyclic, 2),
            Err(DeviceError::InvalidTransition)
        );
    }
}
