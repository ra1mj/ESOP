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
use esop_proto::v1::{DiagnosticEvent, MotionCommand, RobotState, RuntimeIncident};
use esop_proto::{Message, SchemaCompatibilityError, validate_schema_version};
use zenoh::qos::{CongestionControl, Priority};

use crate::{KeySpace, MAX_ZENOH_KEY_BYTES, RouteDirection, RouteError, RouteKind};

const STATE_CONNECTING: u8 = 0;
const STATE_CONNECTED: u8 = 1;
const STATE_DEGRADED: u8 = 2;
const STATE_DISCONNECTED: u8 = 3;
const STATE_CLOSED: u8 = 4;

/// Current health of the supervision-domain Zenoh transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ConnectionState {
    Connecting = STATE_CONNECTING,
    Connected = STATE_CONNECTED,
    Degraded = STATE_DEGRADED,
    Disconnected = STATE_DISCONNECTED,
}

/// Host-domain QoS policy for an ESOP publication route.
///
/// All routes use non-blocking congestion handling: an unavailable transport
/// queue must not make a supervision task wait indefinitely. These settings
/// affect only the Zenoh host adapter and are not EtherCAT or safety guarantees.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublishQos {
    pub priority: Priority,
    pub congestion_control: CongestionControl,
    pub express: bool,
}

/// Explicit security requirements for a supervision-domain Zenoh session.
///
/// The development [`ZenohGateway::open`] entry point remains permissive so
/// loopback/HIL configurations can use plain TCP. Production code should use
/// [`ZenohGateway::open_secure`] with [`TransportSecurityPolicy::production`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransportSecurityPolicy {
    pub require_tls: bool,
    pub require_mtls: bool,
    pub require_authentication: bool,
}

impl TransportSecurityPolicy {
    pub const fn production() -> Self {
        Self {
            require_tls: true,
            require_mtls: true,
            require_authentication: true,
        }
    }

    pub const fn development() -> Self {
        Self {
            require_tls: false,
            require_mtls: false,
            require_authentication: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecurityConfigError {
    MissingTransportProtocols,
    InsecureTransportProtocol,
    MissingConnectEndpoint,
    InsecureConnectEndpoint,
    MissingTlsConfiguration,
    MissingRootCertificate,
    NameVerificationDisabled,
    MtlsDisabled,
    MissingClientCertificate,
    MissingClientPrivateKey,
    MissingAuthentication,
    InvalidConfigValue,
}

impl PublishQos {
    pub const fn for_route(kind: RouteKind) -> Self {
        match kind {
            RouteKind::Event | RouteKind::Diagnostic => Self {
                priority: Priority::InteractiveHigh,
                congestion_control: CongestionControl::Drop,
                express: true,
            },
            RouteKind::State | RouteKind::Command | RouteKind::Query => Self {
                priority: Priority::Data,
                congestion_control: CongestionControl::Drop,
                express: false,
            },
        }
    }
}

impl ConnectionState {
    const fn from_u8(value: u8) -> Self {
        match value {
            STATE_CONNECTED => Self::Connected,
            STATE_DEGRADED => Self::Degraded,
            STATE_DISCONNECTED | STATE_CLOSED => Self::Disconnected,
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

    /// Return the last observed transport state without blocking. The host
    /// supervisor must call `ZenohGateway::refresh_health` while idle.
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
        // A refresh racing with close must not resurrect a terminal session.
        let _ = self
            .state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current != STATE_CLOSED).then_some(state as u8)
            });
    }

    fn close(&self) {
        self.state.store(STATE_CLOSED, Ordering::Release);
    }

    fn degrade(&self) {
        let _ = self.state.compare_exchange(
            STATE_CONNECTED,
            STATE_DEGRADED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

/// Errors at the host transport boundary. Route errors remain distinguishable
/// from Zenoh failures so callers can audit policy rejection separately.
#[derive(Debug)]
pub enum RuntimeError {
    Route(RouteError),
    InvalidKeyEncoding,
    RobotMismatch,
    Schema(SchemaCompatibilityError),
    Security(SecurityConfigError),
    Zenoh(zenoh::Error),
}

/// Errors raised while converting a versioned Protobuf command at the host
/// boundary. Policy errors remain the original fixed-capacity ingress errors.
#[derive(Debug)]
pub enum CommandAdapterError {
    Payload(RouteError),
    Decode(esop_proto::DecodeError),
    RobotMismatch,
    Schema(SchemaCompatibilityError),
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
    validate_schema_version(command.schema_version).map_err(CommandAdapterError::Schema)?;
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
        let gateway = Self {
            session,
            key_space,
            health,
        };
        gateway.refresh_health().await;
        Ok(gateway)
    }

    /// Open a session only after the supplied security requirements have been
    /// validated. This check is a host-domain admission boundary; it is not a
    /// replacement for certificate authority operations or remote ACL policy.
    pub async fn open_secure(
        key_space: KeySpace,
        config: zenoh::Config,
        policy: TransportSecurityPolicy,
    ) -> Result<Self, RuntimeError> {
        validate_security_config(&config, policy).map_err(RuntimeError::Security)?;
        Self::open(key_space, config)
            .await
            .map_err(RuntimeError::Zenoh)
    }

    pub fn health(&self) -> TransportHealth {
        self.health.clone()
    }

    /// Observe current upstream router/peer connectivity using local Zenoh
    /// session information. Call periodically from the host supervisor, never
    /// from the EtherCAT cycle. This is not an application-level heartbeat or
    /// a delivery acknowledgement, and it does not grant a motion permit.
    pub async fn refresh_health(&self) -> ConnectionState {
        if self.session.is_closed() {
            self.health.close();
        } else {
            let info = self.session.info();
            let own_id = info.zid().await;
            let connected = info.routers_zid().await.any(|id| id != own_id)
                || info.peers_zid().await.any(|id| id != own_id);
            if self.session.is_closed() {
                self.health.close();
            } else {
                self.health.set(if connected {
                    ConnectionState::Connected
                } else {
                    ConnectionState::Disconnected
                });
            }
        }
        self.health.state()
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

        let qos = PublishQos::for_route(kind);
        match self
            .session
            .put(key, payload)
            .priority(qos.priority)
            .congestion_control(qos.congestion_control)
            .express(qos.express)
            .await
        {
            Ok(()) => {
                self.refresh_health().await;
                Ok(())
            }
            Err(error) => {
                self.health.publish_failures.fetch_add(1, Ordering::Relaxed);
                Err(self.record_transport_error(error).await)
            }
        }
    }

    /// Encode and publish a state snapshot on the fixed state route.
    ///
    /// The robot ID is checked against the namespace before serialization so
    /// a valid protobuf cannot be published under the wrong robot key.
    pub async fn publish_state(&self, state: &RobotState) -> Result<(), RuntimeError> {
        validate_robot_id(self.key_space, &state.robot_id)?;
        validate_schema_version(state.schema_version).map_err(RuntimeError::Schema)?;
        self.publish(RouteKind::State, &state.encode_to_vec()).await
    }

    /// Encode and publish a diagnostic event on the fixed event route.
    pub async fn publish_event(&self, event: &DiagnosticEvent) -> Result<(), RuntimeError> {
        validate_schema_version(event.schema_version).map_err(RuntimeError::Schema)?;
        self.publish(RouteKind::Event, &event.encode_to_vec()).await
    }

    /// Encode and publish a correlated runtime incident on the diagnostic route.
    pub async fn publish_incident(&self, incident: &RuntimeIncident) -> Result<(), RuntimeError> {
        validate_schema_version(incident.schema_version).map_err(RuntimeError::Schema)?;
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
        if let Err(error) = self
            .session
            .declare_queryable(key)
            .callback(callback)
            .background()
            .await
        {
            return Err(self.record_transport_error(error).await);
        }
        self.health
            .callback_registrations
            .fetch_add(1, Ordering::Relaxed);
        self.refresh_health().await;
        Ok(())
    }

    /// Close the session and make the disconnection visible to the supervisor.
    pub async fn close(&self) -> Result<(), RuntimeError> {
        self.session.close().await.map_err(RuntimeError::Zenoh)?;
        self.health.close();
        Ok(())
    }

    async fn register_subscriber<F>(&self, kind: RouteKind, callback: F) -> Result<(), RuntimeError>
    where
        F: Fn(zenoh::sample::Sample) + Send + Sync + 'static,
    {
        let (key, length) = self.key(kind, RouteDirection::Subscribe)?;
        let key = str::from_utf8(&key[..length]).map_err(|_| RuntimeError::InvalidKeyEncoding)?;
        if let Err(error) = self
            .session
            .declare_subscriber(key)
            .callback(callback)
            .background()
            .await
        {
            return Err(self.record_transport_error(error).await);
        }
        self.health
            .callback_registrations
            .fetch_add(1, Ordering::Relaxed);
        self.refresh_health().await;
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

    async fn record_transport_error(&self, error: zenoh::Error) -> RuntimeError {
        self.refresh_health().await;
        self.health.degrade();
        RuntimeError::Zenoh(error)
    }
}

/// Validate the security-relevant portions of a Zenoh configuration without
/// opening a session or contacting the network. Secrets are never returned or
/// included in the error value.
pub fn validate_security_config(
    config: &zenoh::Config,
    policy: TransportSecurityPolicy,
) -> Result<(), SecurityConfigError> {
    if !policy.require_tls && !policy.require_mtls && !policy.require_authentication {
        return Ok(());
    }

    let protocols = config_value(config, "transport/link/protocols")?;
    let protocols = protocols
        .as_array()
        .ok_or(SecurityConfigError::MissingTransportProtocols)?;
    if protocols.is_empty() {
        return Err(SecurityConfigError::MissingTransportProtocols);
    }
    if protocols.iter().any(|protocol| protocol.as_str().is_none()) {
        return Err(SecurityConfigError::InvalidConfigValue);
    }
    if policy.require_tls
        && protocols
            .iter()
            .any(|protocol| protocol.as_str() != Some("tls"))
    {
        return Err(SecurityConfigError::InsecureTransportProtocol);
    }

    let endpoints = config_value(config, "connect/endpoints")?;
    let mut endpoint_count = 0;
    let mut endpoint_is_insecure = false;
    collect_endpoint_protocols(&endpoints, &mut endpoint_count, &mut endpoint_is_insecure);
    if endpoint_count == 0 {
        return Err(SecurityConfigError::MissingConnectEndpoint);
    }
    if policy.require_tls && endpoint_is_insecure {
        return Err(SecurityConfigError::InsecureConnectEndpoint);
    }

    if policy.require_tls || policy.require_mtls {
        let tls = config_value(config, "transport/link/tls")?;
        let tls = tls
            .as_object()
            .ok_or(SecurityConfigError::MissingTlsConfiguration)?;
        if !has_nonempty_string(tls, "root_ca_certificate") {
            return Err(SecurityConfigError::MissingRootCertificate);
        }
        if tls.get("verify_name_on_connect").and_then(Value::as_bool) != Some(true) {
            return Err(SecurityConfigError::NameVerificationDisabled);
        }
        if policy.require_mtls {
            if tls.get("enable_mtls").and_then(Value::as_bool) != Some(true) {
                return Err(SecurityConfigError::MtlsDisabled);
            }
            if !has_nonempty_string(tls, "connect_certificate") {
                return Err(SecurityConfigError::MissingClientCertificate);
            }
            if !has_nonempty_string(tls, "connect_private_key") {
                return Err(SecurityConfigError::MissingClientPrivateKey);
            }
        }
    }

    if policy.require_authentication {
        let auth = config_value(config, "transport/auth")?;
        let auth = auth
            .as_object()
            .ok_or(SecurityConfigError::MissingAuthentication)?;
        let pubkey = auth.get("pubkey").and_then(Value::as_object);
        let pubkey_configured = pubkey.is_some_and(|value| {
            (has_nonempty_string(value, "public_key_file")
                || has_nonempty_string(value, "public_key_pem"))
                && (has_nonempty_string(value, "private_key_file")
                    || has_nonempty_string(value, "private_key_pem"))
        });
        let usrpwd = auth.get("usrpwd").and_then(Value::as_object);
        let usrpwd_configured = usrpwd.is_some_and(|value| {
            has_nonempty_string(value, "user") && has_nonempty_string(value, "password")
        });
        if !pubkey_configured && !usrpwd_configured {
            return Err(SecurityConfigError::MissingAuthentication);
        }
    }

    Ok(())
}

use serde_json::Value;

fn config_value(config: &zenoh::Config, path: &str) -> Result<Value, SecurityConfigError> {
    let json = config
        .get_json(path)
        .map_err(|_| SecurityConfigError::InvalidConfigValue)?;
    serde_json::from_str(&json).map_err(|_| SecurityConfigError::InvalidConfigValue)
}

fn has_nonempty_string(object: &serde_json::Map<String, Value>, key: &str) -> bool {
    object
        .get(key)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn collect_endpoint_protocols(value: &Value, count: &mut usize, insecure: &mut bool) {
    match value {
        Value::String(endpoint) => {
            *count += 1;
            if !endpoint.starts_with("tls/") {
                *insecure = true;
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_endpoint_protocols(value, count, insecure);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_endpoint_protocols(value, count, insecure);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

impl Drop for ZenohGateway {
    fn drop(&mut self) {
        self.health.close();
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
    use esop_proto::CURRENT_SCHEMA_VERSION;

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

    #[test]
    fn publication_qos_keeps_state_loss_tolerant_and_diagnostics_prompt() {
        let state = PublishQos::for_route(RouteKind::State);
        assert_eq!(state.priority, Priority::Data);
        assert_eq!(state.congestion_control, CongestionControl::Drop);
        assert!(!state.express);

        let event = PublishQos::for_route(RouteKind::Event);
        assert_eq!(event.priority, Priority::InteractiveHigh);
        assert_eq!(event.congestion_control, CongestionControl::Drop);
        assert!(event.express);
        assert_eq!(event, PublishQos::for_route(RouteKind::Diagnostic));
    }

    fn production_config() -> zenoh::Config {
        let mut config = zenoh::Config::default();
        config
            .insert_json5("connect/endpoints", r#"["tls/router.example:7447"]"#)
            .unwrap();
        config
            .insert_json5("transport/link/protocols", r#"["tls"]"#)
            .unwrap();
        config
            .insert_json5(
                "transport/link/tls",
                r#"{
                    root_ca_certificate: "ca.pem",
                    enable_mtls: true,
                    connect_certificate: "client.pem",
                    connect_private_key: "client.key",
                    verify_name_on_connect: true,
                }"#,
            )
            .unwrap();
        config
            .insert_json5(
                "transport/auth",
                r#"{
                    pubkey: {
                        public_key_file: "client.pub",
                        private_key_file: "client.key",
                        known_keys_file: "known.keys",
                    },
                }"#,
            )
            .unwrap();
        config
    }

    #[test]
    fn production_security_policy_accepts_tls_mtls_and_pubkey_config() {
        assert_eq!(
            validate_security_config(&production_config(), TransportSecurityPolicy::production()),
            Ok(())
        );
    }

    #[test]
    fn production_security_policy_rejects_default_and_plaintext_config() {
        assert_eq!(
            validate_security_config(
                &zenoh::Config::default(),
                TransportSecurityPolicy::production()
            ),
            Err(SecurityConfigError::MissingTransportProtocols)
        );

        let mut config = production_config();
        config
            .insert_json5("transport/link/protocols", r#"["tcp"]"#)
            .unwrap();
        assert_eq!(
            validate_security_config(&config, TransportSecurityPolicy::production()),
            Err(SecurityConfigError::InsecureTransportProtocol)
        );
    }

    #[test]
    fn production_security_policy_rejects_weak_tls_settings() {
        let mut config = production_config();
        config
            .insert_json5("transport/link/tls/verify_name_on_connect", "false")
            .unwrap();
        assert_eq!(
            validate_security_config(&config, TransportSecurityPolicy::production()),
            Err(SecurityConfigError::NameVerificationDisabled)
        );

        let mut config = production_config();
        config
            .insert_json5("transport/link/tls/connect_private_key", "null")
            .unwrap();
        assert_eq!(
            validate_security_config(&config, TransportSecurityPolicy::production()),
            Err(SecurityConfigError::MissingClientPrivateKey)
        );
    }

    #[test]
    fn terminal_health_cannot_be_overwritten_by_a_late_refresh_or_error() {
        let health = TransportHealth::connecting();
        health.set(ConnectionState::Connected);
        health.degrade();
        assert_eq!(health.state(), ConnectionState::Degraded);
        health.set(ConnectionState::Disconnected);
        health.degrade();
        assert_eq!(health.state(), ConnectionState::Disconnected);
        health.set(ConnectionState::Connected);
        health.close();
        health.set(ConnectionState::Connected);
        health.degrade();
        assert_eq!(health.state(), ConnectionState::Disconnected);
    }

    fn encoded_command(robot_id: &str, authority: u32) -> Vec<u8> {
        MotionCommand {
            robot_id: robot_id.to_owned(),
            boot_id: 7,
            schema_version: CURRENT_SCHEMA_VERSION,
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
    fn command_payload_rejects_missing_or_unknown_schema_versions() {
        let key_space = KeySpace::new(b"fleet_a", b"robot_01").unwrap();
        let missing = MotionCommand {
            robot_id: "robot_01".to_owned(),
            ..MotionCommand::default()
        }
        .encode_to_vec();
        assert!(matches!(
            decode_command_payload(key_space, &missing),
            Err(CommandAdapterError::Schema(
                SchemaCompatibilityError::Missing
            ))
        ));

        let unknown = MotionCommand {
            robot_id: "robot_01".to_owned(),
            schema_version: 2,
            ..MotionCommand::default()
        }
        .encode_to_vec();
        assert!(matches!(
            decode_command_payload(key_space, &unknown),
            Err(CommandAdapterError::Schema(
                SchemaCompatibilityError::Unsupported(2)
            ))
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
