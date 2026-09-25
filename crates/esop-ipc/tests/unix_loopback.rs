#![cfg(unix)]

use esop_ipc::{
    EndpointError, IpcFrame, IpcHeader, MAX_DATAGRAM_BYTES, MessageKind, PeerMonitor,
    PeerObservation, PeerPolicy, UnixDatagramEndpoint,
};
use std::fs;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let suffix = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("esop-ipc-{}-{suffix}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn socket(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn header(kind: MessageKind, boot_id: u64, sequence: u64, time_ns: u64) -> IpcHeader {
    IpcHeader::new(kind, 1, 13, 11, boot_id, 17, sequence, time_ns, 0x20)
}

fn frame(kind: MessageKind, boot_id: u64, sequence: u64, time_ns: u64) -> IpcFrame {
    let payload = match kind {
        MessageKind::Heartbeat => &[][..],
        MessageKind::Command => b"command",
        MessageKind::State => b"state",
        MessageKind::Event => b"event",
        MessageKind::Diagnostic => b"diagnostic",
    };
    IpcFrame::new(header(kind, boot_id, sequence, time_ns), payload).unwrap()
}

fn endpoints(
    directory: &TestDirectory,
) -> (UnixDatagramEndpoint, UnixDatagramEndpoint, PathBuf, PathBuf) {
    let left_path = directory.socket("left.sock");
    let right_path = directory.socket("right.sock");
    let left = UnixDatagramEndpoint::bind(&left_path, &right_path).unwrap();
    let right = UnixDatagramEndpoint::bind(&right_path, &left_path).unwrap();
    (left, right, left_path, right_path)
}

#[test]
fn bidirectional_command_state_and_heartbeat_are_nonblocking() {
    let directory = TestDirectory::new();
    let (left, right, _, _) = endpoints(&directory);

    assert!(matches!(left.receive(), Err(EndpointError::WouldBlock)));

    let command = frame(MessageKind::Command, 19, 1, 100);
    left.send(&command).unwrap();
    assert_eq!(right.receive().unwrap(), command);
    right.send(&command).unwrap();
    assert_eq!(left.receive().unwrap(), command);

    let state = frame(MessageKind::State, 23, 1, 110);
    right.send(&state).unwrap();
    assert_eq!(left.receive().unwrap(), state);
    left.send(&state).unwrap();
    assert_eq!(right.receive().unwrap(), state);

    let heartbeat = frame(MessageKind::Heartbeat, 19, 2, 120);
    left.send(&heartbeat).unwrap();
    assert_eq!(right.receive().unwrap(), heartbeat);
    right.send(&heartbeat).unwrap();
    assert_eq!(left.receive().unwrap(), heartbeat);
}

#[test]
fn absent_peer_unexpected_source_and_oversized_datagram_are_distinct() {
    let directory = TestDirectory::new();
    let local_path = directory.socket("local.sock");
    let peer_path = directory.socket("peer.sock");
    let attacker_path = directory.socket("attacker.sock");
    let endpoint = UnixDatagramEndpoint::bind(&local_path, &peer_path).unwrap();

    assert!(matches!(
        endpoint.send(&frame(MessageKind::State, 19, 1, 100)),
        Err(EndpointError::PeerUnavailable)
    ));

    let attacker = UnixDatagram::bind(&attacker_path).unwrap();
    attacker.send_to(b"unexpected", &local_path).unwrap();
    assert!(matches!(
        endpoint.receive(),
        Err(EndpointError::UnexpectedSource)
    ));

    drop(attacker);
    fs::remove_file(&attacker_path).unwrap();
    let peer = UnixDatagram::bind(&peer_path).unwrap();
    peer.send_to(&vec![0; MAX_DATAGRAM_BYTES + 1], &local_path)
        .unwrap();
    assert!(matches!(
        endpoint.receive(),
        Err(EndpointError::OversizedDatagram {
            maximum: MAX_DATAGRAM_BYTES,
            actual
        }) if actual == MAX_DATAGRAM_BYTES + 1
    ));
}

#[test]
fn same_path_peer_restart_is_detected_by_boot_id() {
    let directory = TestDirectory::new();
    let (left, right, _, right_path) = endpoints(&directory);
    let mut monitor = PeerMonitor::new(PeerPolicy::new(11, 13, 17, 100, 50).unwrap());

    right
        .send(&frame(MessageKind::Heartbeat, 19, 8, 100))
        .unwrap();
    let first = left.receive().unwrap();
    assert_eq!(
        monitor.observe(first.header(), 100),
        Ok(PeerObservation::FirstContact)
    );

    drop(right);
    assert!(!right_path.exists());
    let replacement = UnixDatagramEndpoint::bind(&right_path, left.local_path()).unwrap();
    replacement
        .send(&frame(MessageKind::Heartbeat, 23, 1, 110))
        .unwrap();
    let restarted = left.receive().unwrap();
    assert_eq!(
        monitor.observe(restarted.header(), 110),
        Ok(PeerObservation::Restarted {
            previous_boot_id: 19,
            new_boot_id: 23,
        })
    );
}

#[test]
fn same_boot_peer_return_after_timeout_is_reconnected() {
    let directory = TestDirectory::new();
    let (left, right, _, _) = endpoints(&directory);
    let mut monitor = PeerMonitor::new(PeerPolicy::new(11, 13, 17, 100, 50).unwrap());

    right
        .send(&frame(MessageKind::Heartbeat, 19, 1, 100))
        .unwrap();
    let first = left.receive().unwrap();
    monitor.observe(first.header(), 100).unwrap();
    assert_eq!(monitor.status(201), Ok(esop_ipc::PeerStatus::Offline));

    right
        .send(&frame(MessageKind::Heartbeat, 19, 2, 210))
        .unwrap();
    let returned = left.receive().unwrap();
    assert_eq!(
        monitor.observe(returned.header(), 210),
        Ok(PeerObservation::Reconnected)
    );
}

#[test]
fn bind_refuses_existing_paths_and_drop_preserves_replacements() {
    let directory = TestDirectory::new();
    let local_path = directory.socket("local.sock");
    let peer_path = directory.socket("peer.sock");
    fs::write(&local_path, b"do not remove").unwrap();
    assert!(matches!(
        UnixDatagramEndpoint::bind(&local_path, &peer_path),
        Err(EndpointError::LocalPathExists)
    ));
    assert_eq!(fs::read(&local_path).unwrap(), b"do not remove");

    fs::remove_file(&local_path).unwrap();
    let endpoint = UnixDatagramEndpoint::bind(&local_path, &peer_path).unwrap();
    fs::remove_file(&local_path).unwrap();
    fs::write(&local_path, b"replacement").unwrap();
    drop(endpoint);
    assert_eq!(fs::read(&local_path).unwrap(), b"replacement");
}

#[test]
fn drop_removes_only_the_owned_socket_path() {
    let directory = TestDirectory::new();
    let local_path = directory.socket("local.sock");
    let peer_path = directory.socket("peer.sock");
    let endpoint = UnixDatagramEndpoint::bind(&local_path, &peer_path).unwrap();
    assert!(Path::new(&local_path).exists());
    drop(endpoint);
    assert!(!Path::new(&local_path).exists());
}
