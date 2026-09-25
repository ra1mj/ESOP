use crate::{FrameError, IpcFrame, MAX_DATAGRAM_BYTES};
use core::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum EndpointError {
    LocalPathExists,
    WouldBlock,
    PeerUnavailable,
    UnexpectedSource,
    OversizedDatagram { maximum: usize, actual: usize },
    ShortWrite { expected: usize, actual: usize },
    Codec(FrameError),
    Io(io::Error),
}

impl fmt::Display for EndpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LocalPathExists => formatter.write_str("local Unix socket path already exists"),
            Self::WouldBlock => formatter.write_str("Unix datagram operation would block"),
            Self::PeerUnavailable => formatter.write_str("Unix datagram peer is unavailable"),
            Self::UnexpectedSource => {
                formatter.write_str("Unix datagram came from an unexpected source")
            }
            Self::OversizedDatagram { maximum, actual } => write!(
                formatter,
                "Unix datagram exceeds maximum size {maximum}: received {actual} bytes"
            ),
            Self::ShortWrite { expected, actual } => write!(
                formatter,
                "Unix datagram short write: expected {expected}, wrote {actual} bytes"
            ),
            Self::Codec(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for EndpointError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<FrameError> for EndpointError {
    fn from(value: FrameError) -> Self {
        Self::Codec(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SocketIdentity {
    device: u64,
    inode: u64,
}

#[derive(Debug)]
pub struct UnixDatagramEndpoint {
    socket: UnixDatagram,
    local_path: PathBuf,
    peer_path: PathBuf,
    local_identity: SocketIdentity,
}

impl UnixDatagramEndpoint {
    pub fn bind(
        local_path: impl AsRef<Path>,
        peer_path: impl AsRef<Path>,
    ) -> Result<Self, EndpointError> {
        let local_path = local_path.as_ref();
        match fs::symlink_metadata(local_path) {
            Ok(_) => return Err(EndpointError::LocalPathExists),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(EndpointError::Io(error)),
        }

        let socket = match UnixDatagram::bind(local_path) {
            Ok(socket) => socket,
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                return Err(EndpointError::LocalPathExists);
            }
            Err(error) => return Err(EndpointError::Io(error)),
        };
        let metadata = match fs::symlink_metadata(local_path) {
            Ok(metadata) => metadata,
            Err(error) => return Err(EndpointError::Io(error)),
        };
        if !metadata.file_type().is_socket() {
            return Err(EndpointError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "local Unix socket path was replaced during bind",
            )));
        }
        let local_identity = SocketIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        if let Err(error) = socket.set_nonblocking(true) {
            remove_owned_path(local_path, local_identity);
            return Err(EndpointError::Io(error));
        }
        Ok(Self {
            socket,
            local_path: local_path.to_path_buf(),
            peer_path: peer_path.as_ref().to_path_buf(),
            local_identity,
        })
    }

    pub fn local_path(&self) -> &Path {
        &self.local_path
    }

    pub fn peer_path(&self) -> &Path {
        &self.peer_path
    }

    pub fn send(&self, frame: &IpcFrame) -> Result<(), EndpointError> {
        let mut datagram = [0; MAX_DATAGRAM_BYTES];
        let len = frame.encode_into(&mut datagram)?;
        let written = self
            .socket
            .send_to(&datagram[..len], &self.peer_path)
            .map_err(map_send_error)?;
        if written != len {
            return Err(EndpointError::ShortWrite {
                expected: len,
                actual: written,
            });
        }
        Ok(())
    }

    pub fn receive(&self) -> Result<IpcFrame, EndpointError> {
        let mut datagram = [0; MAX_DATAGRAM_BYTES + 1];
        let (len, source) = self.socket.recv_from(&mut datagram).map_err(map_io_error)?;
        if len > MAX_DATAGRAM_BYTES {
            return Err(EndpointError::OversizedDatagram {
                maximum: MAX_DATAGRAM_BYTES,
                actual: len,
            });
        }
        if source.as_pathname() != Some(self.peer_path.as_path()) {
            return Err(EndpointError::UnexpectedSource);
        }
        IpcFrame::decode(&datagram[..len]).map_err(EndpointError::Codec)
    }
}

impl Drop for UnixDatagramEndpoint {
    fn drop(&mut self) {
        remove_owned_path(&self.local_path, self.local_identity);
    }
}

fn remove_owned_path(path: &Path, expected: SocketIdentity) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    let current = SocketIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    if metadata.file_type().is_socket() && current == expected {
        let _ = fs::remove_file(path);
    }
}

fn map_send_error(error: io::Error) -> EndpointError {
    match error.kind() {
        io::ErrorKind::WouldBlock => EndpointError::WouldBlock,
        io::ErrorKind::NotFound
        | io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset => EndpointError::PeerUnavailable,
        _ => EndpointError::Io(error),
    }
}

fn map_io_error(error: io::Error) -> EndpointError {
    if error.kind() == io::ErrorKind::WouldBlock {
        EndpointError::WouldBlock
    } else {
        EndpointError::Io(error)
    }
}
