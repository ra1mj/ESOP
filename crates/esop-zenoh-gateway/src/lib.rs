#![cfg_attr(not(feature = "zenoh"), no_std)]

#[cfg(feature = "zenoh")]
pub mod runtime;

pub const MAX_FLEET_ID_BYTES: usize = 32;
pub const MAX_ROBOT_ID_BYTES: usize = 64;
pub const MAX_ZENOH_KEY_BYTES: usize = 5 + MAX_FLEET_ID_BYTES + 1 + MAX_ROBOT_ID_BYTES + 1 + 10;
pub const MAX_PAYLOAD_BYTES: usize = 4096;

const PREFIX: &[u8] = b"esop/";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RouteKind {
    State = 0,
    Event = 1,
    Diagnostic = 2,
    Command = 3,
    Query = 4,
}

impl RouteKind {
    const fn segment(self) -> &'static [u8] {
        match self {
            Self::State => b"state",
            Self::Event => b"event",
            Self::Diagnostic => b"diagnostic",
            Self::Command => b"cmd",
            Self::Query => b"query",
        }
    }

    pub const fn direction(self) -> RouteDirection {
        match self {
            Self::State | Self::Event | Self::Diagnostic => RouteDirection::Publish,
            Self::Command => RouteDirection::Subscribe,
            Self::Query => RouteDirection::Bidirectional,
        }
    }

    pub const fn payload(self) -> PayloadContract {
        match self {
            Self::State => PayloadContract::RobotState,
            Self::Event => PayloadContract::DiagnosticEvent,
            Self::Diagnostic => PayloadContract::RuntimeIncident,
            Self::Command => PayloadContract::MotionCommand,
            Self::Query => PayloadContract::Query,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RouteDirection {
    Publish = 0,
    Subscribe = 1,
    Bidirectional = 2,
}

impl RouteDirection {
    pub const fn allows(self, requested: Self) -> bool {
        matches!(
            (self, requested),
            (Self::Publish, Self::Publish)
                | (Self::Subscribe, Self::Subscribe)
                | (Self::Bidirectional, Self::Publish)
                | (Self::Bidirectional, Self::Subscribe)
                | (Self::Bidirectional, Self::Bidirectional)
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PayloadContract {
    RobotState = 0,
    DiagnosticEvent = 1,
    RuntimeIncident = 2,
    MotionCommand = 3,
    Query = 4,
}

impl PayloadContract {
    pub fn accepts(self, payload: PayloadContract) -> bool {
        self == payload
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteError {
    EmptyIdentifier,
    IdentifierTooLong,
    InvalidIdentifier,
    KeyBufferTooSmall,
    KeyMismatch,
    UnknownRoute,
    DirectionDenied,
    PayloadMismatch,
    EmptyPayload,
    PayloadTooLarge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeySpace {
    fleet: [u8; MAX_FLEET_ID_BYTES],
    fleet_len: u8,
    robot: [u8; MAX_ROBOT_ID_BYTES],
    robot_len: u8,
}

impl KeySpace {
    pub fn new(fleet: &[u8], robot: &[u8]) -> Result<Self, RouteError> {
        validate_identifier(fleet, MAX_FLEET_ID_BYTES)?;
        validate_identifier(robot, MAX_ROBOT_ID_BYTES)?;
        let mut fleet_storage = [0; MAX_FLEET_ID_BYTES];
        let mut robot_storage = [0; MAX_ROBOT_ID_BYTES];
        fleet_storage[..fleet.len()].copy_from_slice(fleet);
        robot_storage[..robot.len()].copy_from_slice(robot);
        Ok(Self {
            fleet: fleet_storage,
            fleet_len: fleet.len() as u8,
            robot: robot_storage,
            robot_len: robot.len() as u8,
        })
    }

    pub fn fleet(&self) -> &[u8] {
        &self.fleet[..self.fleet_len as usize]
    }

    pub fn robot(&self) -> &[u8] {
        &self.robot[..self.robot_len as usize]
    }

    pub fn write_key(&self, kind: RouteKind, buffer: &mut [u8]) -> Result<usize, RouteError> {
        let mut writer = KeyWriter::new(buffer);
        writer.push(PREFIX)?;
        writer.push(self.fleet())?;
        writer.push_byte(b'/')?;
        writer.push(self.robot())?;
        writer.push_byte(b'/')?;
        writer.push(kind.segment())?;
        Ok(writer.len)
    }

    pub fn route_for_key(
        &self,
        key: &[u8],
        requested_direction: RouteDirection,
        payload: PayloadContract,
    ) -> Result<RouteKind, RouteError> {
        for kind in [
            RouteKind::State,
            RouteKind::Event,
            RouteKind::Diagnostic,
            RouteKind::Command,
            RouteKind::Query,
        ] {
            let mut expected = [0; MAX_ZENOH_KEY_BYTES];
            let length = self.write_key(kind, &mut expected)?;
            if key == &expected[..length] {
                if !kind.direction().allows(requested_direction) {
                    return Err(RouteError::DirectionDenied);
                }
                if !kind.payload().accepts(payload) {
                    return Err(RouteError::PayloadMismatch);
                }
                return Ok(kind);
            }
        }
        Err(RouteError::UnknownRoute)
    }

    pub fn validate_payload(payload: &[u8]) -> Result<(), RouteError> {
        if payload.is_empty() {
            return Err(RouteError::EmptyPayload);
        }
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(RouteError::PayloadTooLarge);
        }
        Ok(())
    }
}

struct KeyWriter<'a> {
    buffer: &'a mut [u8],
    len: usize,
}

impl<'a> KeyWriter<'a> {
    const fn new(buffer: &'a mut [u8]) -> Self {
        Self { buffer, len: 0 }
    }

    fn push(&mut self, bytes: &[u8]) -> Result<(), RouteError> {
        let end = self
            .len
            .checked_add(bytes.len())
            .ok_or(RouteError::KeyBufferTooSmall)?;
        if end > self.buffer.len() {
            return Err(RouteError::KeyBufferTooSmall);
        }
        self.buffer[self.len..end].copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }

    fn push_byte(&mut self, byte: u8) -> Result<(), RouteError> {
        self.push(&[byte])
    }
}

fn validate_identifier(identifier: &[u8], max_len: usize) -> Result<(), RouteError> {
    if identifier.is_empty() {
        return Err(RouteError::EmptyIdentifier);
    }
    if identifier.len() > max_len {
        return Err(RouteError::IdentifierTooLong);
    }
    if identifier
        .iter()
        .any(|byte| !matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.'))
    {
        return Err(RouteError::InvalidIdentifier);
    }
    Ok(())
}
