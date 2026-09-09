//! Optional Linux/supervision-domain adapter for a real Zenoh session.
//!
//! This module is deliberately feature-gated. The default crate remains
//! allocation-free and `no_std`; enabling `zenoh` opts into the host runtime
//! and its network transports.

use core::str;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use esop_command_gateway::{CommandIngress, ExternalMotionCommand, IngressError};
use esop_lifecycle_guard::MotionPermit;
use esop_proto::Message;
use esop_proto::v1::{DiagnosticEvent, MotionCommand, RobotState, RuntimeIncident};

use crate::{KeySpace, MAX_ZENOH_KEY_BYTES, RouteDirection, RouteError, RouteKind};

const STATE_CONNECTING: u8 = 0;
const STATE_CONNECTED: u8 = 1;
const STATE_DEGRADED: u8 = 2;
const STATE_DISCONNECTED: u8 = 3;

/// Current health of the supervision-domain Zenoh transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ConnectionState {
    Connecting = STATE_CONNECTING,
    Connected = STATE_CONNECTED,
    Degraded = STATE_DEGRADED,
    Disconnected = STATE_DISCONNECTED,
}

impl ConnectionState {
    const fn from_u8(value: u8) -> Self {
        match value {
            STATE_CONNECTED => Self::Connected,
            STATE_DEGRADED => Self::Degraded,
            STATE_DISCONNECTED => Self::Disconnected,
            _ => Self::Connecting,
        }
    }
}

/// Bounded transport health counters suitable for exporting to runtime evidence.
#[derive(Clone)]
pub struct TransportHealth {
    state: Arc<AtomicU8>,
    publish_failures: Arc<AtomicU64>,
    callback_registrations: Arc<AtomicU64>,
}

impl TransportHealth {
    fn connecting() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(STATE_CONNECTING)),
            publish_failures: Arc::new(AtomicU64::new(0)),
            callback_registrations: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Return the latest transport state without blocking.
    pub fn state(&self) -> ConnectionState {
        ConnectionState::from_u8(self.state.load(Ordering::Acquire))
    }

    /// Number of failed publish operations observed by this adapter.
    pub fn publish_failures(&self) -> u64 {
        self.publish_failures.load(Ordering::Relaxed)
    }

    /// Number of successfully registered background handlers.
    pub fn callback_registrations(&self) -> u64 {
        self.callback_registrations.load(Ordering::Relaxed)
    }

    fn set(&self, state: ConnectionState) {
        self.state.store(state as u8, Ordering::Release);
    }
}

/// Errors at the host transport boundary. Route errors remain distinguishable
/// from Zenoh failures so callers can audit policy rejection separately.
#[derive(Debug)]
pub enum RuntimeError {
    Route(RouteError),
    InvalidKeyEncoding,
    RobotMismatch,
    Zenoh(zenoh::Error),
}

/// Errors raised while converting a versioned Protobuf command at the host
/// boundary. Policy errors remain the original fixed-capacity ingress errors.
#[derive(Debug)]
pub enum CommandAdapterError {
    Payload(RouteError),
    Decode(esop_proto::DecodeError),
    RobotMismatch,
    AuthorityOutOfRange,
    IdentityMismatch,
    Policy(IngressError),
}

impl From<esop_proto::DecodeError> for CommandAdapterError {
    fn from(error: esop_proto::DecodeError) -> Self {
        Self::Decode(error)
    }
}

/// Decode a v1 command and map its policy fields into the fixed command shape.
/// This pure host-domain function is also usable by a callback that has not
/// retained a `ZenohGateway` handle.
pub fn decode_command_payload(
    key_space: KeySpace,
    payload: &[u8],
) -> Result<ExternalMotionCommand, CommandAdapterError> {
    KeySpace::validate_payload(payload).map_err(CommandAdapterError::Payload)?;
    let command = MotionCommand::decode(payload)?;
    if command.robot_id.as_bytes() != key_space.robot() {
        return Err(CommandAdapterError::RobotMismatch);
    }
    let authority =
        u8::try_from(command.authority).map_err(|_| CommandAdapterError::AuthorityOutOfRange)?;
    Ok(ExternalMotionCommand {
        boot_id: command.boot_id,
        source_id: command.source_id,
        permit_epoch: command.permit_epoch,
        sequence: command.sequence,
        deadline_ns: command.deadline_ns,
        axis_mask: command.axis_mask,
        authority,
        reserved: [0; 3],
        policy_version: command.policy_version,
    })
}

impl From<RouteError> for RuntimeError {
    fn from(error: RouteError) -> Self {
        Self::Route(error)
    }
}

/// Real Zenoh session adapter for the supervision domain.
pub struct ZenohGateway {
    session: zenoh::Session,
    key_space: KeySpace,
    health: TransportHealth,
}

impl ZenohGateway {
    /// Open a real Zenoh session using the supplied router/peer configuration.
    pub async fn open(key_space: KeySpace, config: zenoh::Config) -> Result<Self, zenoh::Error> {
        let health = TransportHealth::connecting();
        let session = zenoh::open(config).await?;
        health.set(ConnectionState::Connected);
        Ok(Self {
            session,
            key_space,
            health,
        })
    }

    pub fn health(&self) -> TransportHealth {
        self.health.clone()
    }

    pub fn key_space(&self) -> KeySpace {
        self.key_space
    }

    /// Decode a v1 command and map only its fixed policy fields into the
    /// RT-facing command shape. Motion targets remain outside this policy
    /// boundary and are handled by the profile/ProcBuf path after admission.
    pub fn decode_command(
        &self,
        payload: &[u8],
    ) -> Result<ExternalMotionCommand, CommandAdapterError> {
        decode_command_payload(self.key_space, payload)
    }

    /// Decode and submit a command to the existing fixed-capacity ingress
    /// policy. This method does not publish or touch the EtherCAT cycle.
    pub fn admit_command(
        &self,
        ingress: &mut CommandIngress,
        payload: &[u8],
        now_ns: u64,
    ) -> Result<MotionPermit, CommandAdapterError> {
        let command = self.decode_command(payload)?;
        ingress
            .admit(command, now_ns)
            .map_err(CommandAdapterError::Policy)
    }

    /// Admit a command only after a trusted host identity has been resolved.
    ///
    /// The transport/authentication service owns the mapping from its
    /// authenticated principal to the fixed `source_id`; this comparison
    /// prevents a payload from claiming a different authorized source.
    pub fn admit_authenticated_command(
        &self,
        ingress: &mut CommandIngress,
        payload: &[u8],
        authenticated_source_id: u64,
        now_ns: u64,
    ) -> Result<MotionPermit, CommandAdapterError> {
        let command = self.decode_command(payload)?;
        let command = validate_authenticated_source(command, authenticated_source_id)?;
        ingress
            .admit(command, now_ns)
            .map_err(CommandAdapterError::Policy)
    }

    /// Publish a contract-checked payload on a state, event, diagnostic, or
    /// query route. The fixed key is held only across the awaited operation.
    pub async fn publish(&self, kind: RouteKind, payload: &[u8]) -> Result<(), RuntimeError> {
        let mut key = [0; MAX_ZENOH_KEY_BYTES];
        let length = self.key_space.write_key(kind, &mut key)?;
        self.key_space
            .route_for_key(&key[..length], RouteDirection::Publish, kind.payload())?;
        KeySpace::validate_payload(payload)?;
        let key = str::from_utf8(&key[..length]).map_err(|_| RuntimeError::InvalidKeyEncoding)?;

        match self.session.put(key, payload).await {
            Ok(()) => {
                self.health.set(ConnectionState::Connected);
                Ok(())
            }
            Err(error) => {
                self.health.publish_failures.fetch_add(1, Ordering::Relaxed);
                self.health.set(if self.session.is_closed() {
                    ConnectionState::Disconnected
                } else {
                    ConnectionState::Degraded
                });
                Err(RuntimeError::Zenoh(error))
            }
        }
    }

    /// Encode and publish a state snapshot on the fixed state route.
    ///
    /// The robot ID is checked against the namespace before serialization so
    /// a valid protobuf cannot be published under the wrong robot key.
    pub async fn publish_state(&self, state: &RobotState) -> Result<(), RuntimeError> {
        validate_robot_id(self.key_space, &state.robot_id)?;
        self.publish(RouteKind::State, &state.encode_to_vec()).await
    }

    /// Encode and publish a diagnostic event on the fixed event route.
    pub async fn publish_event(&self, event: &DiagnosticEvent) -> Result<(), RuntimeError> {
        self.publish(RouteKind::Event, &event.encode_to_vec()).await
    }

    /// Encode and publish a correlated runtime incident on the diagnostic route.
    pub async fn publish_incident(&self, incident: &RuntimeIncident) -> Result<(), RuntimeError> {
        self.publish(RouteKind::Diagnostic, &incident.encode_to_vec())
            .await
    }

    /// Register a background command subscriber. The callback should enqueue a
    /// bounded host-domain command for policy validation and must not touch the
    /// EtherCAT cycle directly.
    pub async fn subscribe_commands<F>(&self, callback: F) -> Result<(), RuntimeError>
    where
        F: Fn(zenoh::sample::Sample) + Send + Sync + 'static,
    {
        self.register_subscriber(RouteKind::Command, callback).await
    }

    /// Register a background queryable for the robot's query route.
    pub async fn serve_queries<F>(&self, callback: F) -> Result<(), RuntimeError>
    where
        F: Fn(zenoh::query::Query) + Send + Sync + 'static,
    {
        let (key, length) = self.key(RouteKind::Query, RouteDirection::Subscribe)?;
        let key = str::from_utf8(&key[..length]).map_err(|_| RuntimeError::InvalidKeyEncoding)?;
        self.session
            .declare_queryable(key)
            .callback(callback)
            .background()
            .await
            .map_err(|error| self.record_registration_error(error))?;
        self.health
            .callback_registrations
            .fetch_add(1, Ordering::Relaxed);
        self.health.set(ConnectionState::Connected);
        Ok(())
    }

    /// Close the session and make the disconnection visible to the supervisor.
    pub async fn close(&self) -> Result<(), RuntimeError> {
        self.session.close().await.map_err(RuntimeError::Zenoh)?;
        self.health.set(ConnectionState::Disconnected);
        Ok(())
    }

    async fn register_subscriber<F>(&self, kind: RouteKind, callback: F) -> Result<(), RuntimeError>
    where
        F: Fn(zenoh::sample::Sample) + Send + Sync + 'static,
    {
        let (key, length) = self.key(kind, RouteDirection::Subscribe)?;
        let key = str::from_utf8(&key[..length]).map_err(|_| RuntimeError::InvalidKeyEncoding)?;
        self.session
            .declare_subscriber(key)
            .callback(callback)
            .background()
            .await
            .map_err(|error| self.record_registration_error(error))?;
        self.health
            .callback_registrations
            .fetch_add(1, Ordering::Relaxed);
        self.health.set(ConnectionState::Connected);
        Ok(())
    }

    fn key(
        &self,
        kind: RouteKind,
        direction: RouteDirection,
    ) -> Result<([u8; MAX_ZENOH_KEY_BYTES], usize), RuntimeError> {
        let mut key = [0; MAX_ZENOH_KEY_BYTES];
        let length = self.key_space.write_key(kind, &mut key)?;
        self.key_space
            .route_for_key(&key[..length], direction, kind.payload())?;
        Ok((key, length))
    }

    fn record_registration_error(&self, error: zenoh::Error) -> RuntimeError {
        self.health.set(if self.session.is_closed() {
            ConnectionState::Disconnected
        } else {
            ConnectionState::Degraded
        });
        RuntimeError::Zenoh(error)
    }
}

fn validate_robot_id(key_space: KeySpace, robot_id: &str) -> Result<(), RuntimeError> {
    if robot_id.as_bytes() == key_space.robot() {
        Ok(())
    } else {
        Err(RuntimeError::RobotMismatch)
    }
}

fn validate_authenticated_source(
    command: ExternalMotionCommand,
    authenticated_source_id: u64,
) -> Result<ExternalMotionCommand, CommandAdapterError> {
    if command.source_id == authenticated_source_id {
        Ok(command)
    } else {
        Err(CommandAdapterError::IdentityMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use esop_command_gateway::IngressPolicy;

    #[test]
    fn health_is_bounded_and_transitions_are_explicit() {
        let health = TransportHealth::connecting();
        assert_eq!(health.state(), ConnectionState::Connecting);
        health.set(ConnectionState::Connected);
        assert_eq!(health.state(), ConnectionState::Connected);
        health.publish_failures.fetch_add(1, Ordering::Relaxed);
        assert_eq!(health.publish_failures(), 1);
        assert_eq!(health.callback_registrations(), 0);
    }

    fn encoded_command(robot_id: &str, authority: u32) -> Vec<u8> {
        MotionCommand {
            robot_id: robot_id.to_owned(),
            boot_id: 7,
            source_id: 42,
            permit_epoch: 1,
            sequence: 1,
            deadline_ns: 100,
            axis_mask: 0x03,
            authority,
            policy_version: 9,
            ..MotionCommand::default()
        }
        .encode_to_vec()
    }

    #[test]
    fn command_payload_enters_fixed_ingress_and_becomes_audited_permit() {
        let key_space = KeySpace::new(b"fleet_a", b"robot_01").unwrap();
        let mut ingress = CommandIngress::new(
            7,
            IngressPolicy {
                authorized_sources: [42, 0, 0, 0],
                authorized_source_count: 1,
                minimum_authority: 2,
                reserved: [0; 2],
                permit_policy_version: 9,
                allowed_axis_mask: 0x03,
                max_ttl_ns: 100,
                rate_window_ns: 1_000,
                max_commands_per_window: 2,
                reserved_tail: [0; 6],
            },
        );
        let permit = decode_command_payload(key_space, encoded_command("robot_01", 2).as_slice())
            .and_then(|command| {
                ingress
                    .admit(command, 1)
                    .map_err(CommandAdapterError::Policy)
            })
            .unwrap();
        assert_eq!(permit.source_id, 42);
        assert_eq!(permit.sequence, 1);
        assert_eq!(ingress.audit_count(), 1);
    }

    #[test]
    fn command_payload_rejects_robot_mismatch_and_authority_truncation() {
        let key_space = KeySpace::new(b"fleet_a", b"robot_01").unwrap();
        assert!(matches!(
            decode_command_payload(key_space, encoded_command("robot_02", 2).as_slice()),
            Err(CommandAdapterError::RobotMismatch)
        ));
        assert!(matches!(
            decode_command_payload(key_space, encoded_command("robot_01", 256).as_slice()),
            Err(CommandAdapterError::AuthorityOutOfRange)
        ));
    }

    #[test]
    fn authenticated_source_must_match_the_command_identity() {
        let key_space = KeySpace::new(b"fleet_a", b"robot_01").unwrap();
        let ingress = CommandIngress::new(
            7,
            IngressPolicy {
                authorized_sources: [42, 0, 0, 0],
                authorized_source_count: 1,
                minimum_authority: 2,
                reserved: [0; 2],
                permit_policy_version: 9,
                allowed_axis_mask: 0x03,
                max_ttl_ns: 100,
                rate_window_ns: 1_000,
                max_commands_per_window: 2,
                reserved_tail: [0; 6],
            },
        );
        let command = encoded_command("robot_01", 2);
        assert!(matches!(
            decode_command_payload(key_space, &command)
                .and_then(|decoded| validate_authenticated_source(decoded, 43)),
            Err(CommandAdapterError::IdentityMismatch)
        ));
        assert_eq!(ingress.audit_count(), 0);
    }

    #[test]
    fn typed_state_validation_rejects_a_cross_robot_snapshot() {
        let key_space = KeySpace::new(b"fleet_a", b"robot_01").unwrap();
        assert!(matches!(
            validate_robot_id(key_space, "robot_02"),
            Err(RuntimeError::RobotMismatch)
        ));
        assert!(validate_robot_id(key_space, "robot_01").is_ok());
    }
}
