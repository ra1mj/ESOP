#![forbid(unsafe_code)]

mod frame;
#[cfg(feature = "payloads")]
pub mod payloads;
mod peer;
#[cfg(unix)]
mod unix;

pub use frame::{
    FrameError, HEADER_BYTES, IPC_MAGIC, IPC_VERSION, IpcFrame, IpcHeader, MAX_DATAGRAM_BYTES,
    MAX_PAYLOAD_BYTES, MessageKind,
};
pub use peer::{PeerError, PeerMonitor, PeerObservation, PeerPolicy, PeerStatus};
#[cfg(unix)]
pub use unix::{EndpointError, UnixDatagramEndpoint};
