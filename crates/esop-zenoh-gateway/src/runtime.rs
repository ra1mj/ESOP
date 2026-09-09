//! Optional Linux/supervision-domain adapter for a real Zenoh session.
//!
//! This module is deliberately feature-gated. The default crate remains
//! allocation-free and `no_std`; enabling `zenoh` opts into the host runtime
//! and its network transports.

use core::str;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

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
    Zenoh(zenoh::Error),
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
