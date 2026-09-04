pub const AL_STATE_MASK: u16 = 0x000F;
pub const AL_ERROR_FLAG: u16 = 0x0010;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EthercatState {
    Init = 0x01,
    PreOp = 0x02,
    Bootstrap = 0x03,
    SafeOp = 0x04,
    Op = 0x08,
    Unknown = 0x00,
}

impl EthercatState {
    pub const fn from_al_status(status: u16) -> Self {
        match status & AL_STATE_MASK {
            0x01 => Self::Init,
            0x02 => Self::PreOp,
            0x03 => Self::Bootstrap,
            0x04 => Self::SafeOp,
            0x08 => Self::Op,
            _ => Self::Unknown,
        }
    }

    pub const fn is_operational(self) -> bool {
        matches!(self, Self::Op)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlStatus {
    pub state: EthercatState,
    pub error: bool,
    pub raw: u16,
    pub code: u16,
}

impl AlStatus {
    pub const fn new(raw: u16, code: u16) -> Self {
        Self {
            state: EthercatState::from_al_status(raw),
            error: raw & AL_ERROR_FLAG != 0,
            raw,
            code,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlaveIdentity {
    pub vendor_id: u32,
    pub product_code: u32,
    pub revision: u32,
    pub serial: u32,
}

impl SlaveIdentity {
    pub const EMPTY: Self = Self {
        vendor_id: 0,
        product_code: 0,
        revision: 0,
        serial: 0,
    };

    pub const fn matches(self, expected: Self) -> bool {
        self.vendor_id == expected.vendor_id
            && self.product_code == expected.product_code
            && self.revision == expected.revision
            && (expected.serial == 0 || self.serial == expected.serial)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlaveRecord {
    pub position: u16,
    pub station_address: u16,
    pub identity: SlaveIdentity,
    pub online: bool,
    pub configured: bool,
    pub al_status: AlStatus,
    pub requested_state: EthercatState,
    pub transition_deadline_ns: u64,
    pub last_seen_cycle: u64,
}

impl SlaveRecord {
    pub const EMPTY: Self = Self {
        position: 0,
        station_address: 0,
        identity: SlaveIdentity::EMPTY,
        online: false,
        configured: false,
        al_status: AlStatus {
            state: EthercatState::Unknown,
            error: false,
            raw: 0,
            code: 0,
        },
        requested_state: EthercatState::Init,
        transition_deadline_ns: 0,
        last_seen_cycle: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlaveTableError {
    CapacityExceeded,
    DuplicatePosition,
    UnknownPosition,
    IdentityMismatch,
    InvalidTransition,
}

pub struct SlaveTable<const MAX_SLAVES: usize> {
    records: [SlaveRecord; MAX_SLAVES],
    count: usize,
}

impl<const MAX_SLAVES: usize> SlaveTable<MAX_SLAVES> {
    pub const fn new() -> Self {
        Self {
            records: [SlaveRecord::EMPTY; MAX_SLAVES],
            count: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn records(&self) -> &[SlaveRecord] {
        &self.records[..self.count]
    }

    pub fn add(
        &mut self,
        position: u16,
        station_address: u16,
        identity: SlaveIdentity,
    ) -> Result<usize, SlaveTableError> {
        if self.count >= MAX_SLAVES {
            return Err(SlaveTableError::CapacityExceeded);
        }
        if self
            .records()
            .iter()
            .any(|record| record.position == position)
        {
            return Err(SlaveTableError::DuplicatePosition);
        }
        self.records[self.count] = SlaveRecord {
            position,
            station_address,
            identity,
            online: true,
            configured: false,
            al_status: SlaveRecord::EMPTY.al_status,
            requested_state: EthercatState::Init,
            transition_deadline_ns: 0,
            last_seen_cycle: 0,
        };
        self.count += 1;
        Ok(self.count - 1)
    }

    pub fn get(&self, position: u16) -> Option<&SlaveRecord> {
        self.records()
            .iter()
            .find(|record| record.position == position)
    }

    pub fn get_mut(&mut self, position: u16) -> Option<&mut SlaveRecord> {
        self.records
            .iter_mut()
            .take(self.count)
            .find(|record| record.position == position)
    }

    pub fn verify_identity(
        &mut self,
        position: u16,
        expected: SlaveIdentity,
    ) -> Result<(), SlaveTableError> {
        let record = self
            .get_mut(position)
            .ok_or(SlaveTableError::UnknownPosition)?;
        if !record.identity.matches(expected) {
            record.configured = false;
            return Err(SlaveTableError::IdentityMismatch);
        }
        record.configured = true;
        Ok(())
    }

    pub fn request_state(
        &mut self,
        position: u16,
        target: EthercatState,
        deadline_ns: u64,
    ) -> Result<(), SlaveTableError> {
        let record = self
            .get_mut(position)
            .ok_or(SlaveTableError::UnknownPosition)?;
        if !valid_target(record.al_status.state, target) {
            return Err(SlaveTableError::InvalidTransition);
        }
        record.requested_state = target;
        record.transition_deadline_ns = deadline_ns;
        Ok(())
    }

    pub fn observe_status(
        &mut self,
        position: u16,
        status: AlStatus,
        cycle: u64,
    ) -> Result<(), SlaveTableError> {
        let record = self
            .get_mut(position)
            .ok_or(SlaveTableError::UnknownPosition)?;
        record.online = true;
        record.al_status = status;
        record.last_seen_cycle = cycle;
        Ok(())
    }

    pub fn next_request(&self, position: u16) -> Result<Option<EthercatState>, SlaveTableError> {
        let record = self.get(position).ok_or(SlaveTableError::UnknownPosition)?;
        if record.al_status.error {
            return Ok(None);
        }
        Ok(next_state(record.al_status.state, record.requested_state))
    }

    pub fn mark_offline(&mut self, position: u16) -> Result<(), SlaveTableError> {
        let record = self
            .get_mut(position)
            .ok_or(SlaveTableError::UnknownPosition)?;
        record.online = false;
        record.configured = false;
        record.requested_state = EthercatState::Init;
        Ok(())
    }
}

pub fn next_state(current: EthercatState, target: EthercatState) -> Option<EthercatState> {
    match (current, target) {
        (EthercatState::Init, EthercatState::PreOp)
        | (EthercatState::Init, EthercatState::SafeOp)
        | (EthercatState::Init, EthercatState::Op) => Some(EthercatState::PreOp),
        (EthercatState::PreOp, EthercatState::SafeOp)
        | (EthercatState::PreOp, EthercatState::Op) => Some(EthercatState::SafeOp),
        (EthercatState::SafeOp, EthercatState::Op) => Some(EthercatState::Op),
        (EthercatState::Op, EthercatState::SafeOp) => Some(EthercatState::SafeOp),
        (EthercatState::Op, EthercatState::PreOp) | (EthercatState::Op, EthercatState::Init) => {
            Some(EthercatState::SafeOp)
        }
        (EthercatState::SafeOp, EthercatState::PreOp) => Some(EthercatState::PreOp),
        (EthercatState::SafeOp, EthercatState::Init) => Some(EthercatState::PreOp),
        (EthercatState::PreOp, EthercatState::Init) => Some(EthercatState::Init),
        (EthercatState::Bootstrap, EthercatState::Init) => Some(EthercatState::Init),
        _ => None,
    }
}

fn valid_target(current: EthercatState, target: EthercatState) -> bool {
    !matches!(current, EthercatState::Unknown)
        && !matches!(target, EthercatState::Unknown)
        && (current == target || next_state(current, target).is_some())
}

impl<const MAX_SLAVES: usize> Default for SlaveTable<MAX_SLAVES> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: SlaveIdentity = SlaveIdentity {
        vendor_id: 1,
        product_code: 2,
        revision: 3,
        serial: 4,
    };

    #[test]
    fn table_tracks_identity_and_al_state_progression() {
        let mut table = SlaveTable::<2>::new();
        table.add(0, 0x1001, IDENTITY).unwrap();
        table.verify_identity(0, IDENTITY).unwrap();
        table.observe_status(0, AlStatus::new(0x01, 0), 0).unwrap();
        table.request_state(0, EthercatState::Op, 1_000).unwrap();

        table.observe_status(0, AlStatus::new(0x02, 0), 1).unwrap();
        assert_eq!(table.next_request(0), Ok(Some(EthercatState::SafeOp)));
        table.observe_status(0, AlStatus::new(0x04, 0), 2).unwrap();
        assert_eq!(table.next_request(0), Ok(Some(EthercatState::Op)));
        table.observe_status(0, AlStatus::new(0x08, 0), 3).unwrap();
        assert_eq!(table.next_request(0), Ok(None));
        assert!(table.get(0).unwrap().configured);
    }

    #[test]
    fn identity_mismatch_and_al_error_block_configuration() {
        let mut table = SlaveTable::<1>::new();
        table.add(0, 1, IDENTITY).unwrap();
        assert_eq!(
            table.verify_identity(
                0,
                SlaveIdentity {
                    serial: 99,
                    ..IDENTITY
                },
            ),
            Err(SlaveTableError::IdentityMismatch)
        );
        table
            .observe_status(0, AlStatus::new(0x12, 0x001B), 1)
            .unwrap();
        table.request_state(0, EthercatState::Op, 100).unwrap();
        assert_eq!(table.next_request(0), Ok(None));
        assert!(table.get(0).unwrap().al_status.error);
    }

    #[test]
    fn unknown_state_cannot_be_requested_or_treated_as_ready() {
        let mut table = SlaveTable::<1>::new();
        table.add(0, 1, IDENTITY).unwrap();
        assert_eq!(
            table.request_state(0, EthercatState::Unknown, 10),
            Err(SlaveTableError::InvalidTransition)
        );
        assert_eq!(table.next_request(0), Ok(None));
    }
}
