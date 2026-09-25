#![cfg(target_os = "linux")]
#![deny(unsafe_op_in_unsafe_fn)]

use esop_ethercat_core::wire::{ETHERCAT_ETHERTYPE, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{DmaTxHandle, EthercatDmaTxPort, EthercatPort, LinkState, RxPoll};
use std::ffi::CString;
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::RawFd;
use std::time::Instant;

#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

mod sim;

pub use sim::Cia402DriveSimulator;
pub use sim::SimulatedPort;

/// Stable operation codes carried by the Linux raw-port uprobe ABI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RawPortOperation {
    Tx = 0,
    Rx = 1,
}

/// Stable TX outcomes carried by the Linux raw-port uprobe ABI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RawPortTxOutcome {
    Success = 0,
    SyscallError = 1,
    PartialWrite = 2,
}

/// Stable RX outcomes carried by the Linux raw-port uprobe ABI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RawPortRxOutcome {
    Frame = 0,
    Empty = 1,
    LinkDown = 2,
    SyscallError = 3,
}

#[cfg(test)]
static TEST_RAW_PORT_MARKER_BEGINS: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static TEST_RAW_PORT_MARKER_ENDS: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static TEST_RAW_PORT_MARKER_LAST_OUTCOME: AtomicU64 = AtomicU64::new(u64::MAX);

/// Stable v1 uprobe target immediately before a Linux raw-port syscall.
#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn esop_linux_raw_port_operation_begin_v1(ifindex: u32, operation: u32) {
    let _ = core::hint::black_box((ifindex, operation));
    #[cfg(test)]
    TEST_RAW_PORT_MARKER_BEGINS.fetch_add(1, Ordering::Relaxed);
}

/// Stable v1 uprobe target immediately after a Linux raw-port syscall.
#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn esop_linux_raw_port_operation_end_v1(ifindex: u32, operation: u32, outcome: u32) {
    let _ = core::hint::black_box((ifindex, operation, outcome));
    #[cfg(test)]
    {
        TEST_RAW_PORT_MARKER_ENDS.fetch_add(1, Ordering::Relaxed);
        TEST_RAW_PORT_MARKER_LAST_OUTCOME.store(u64::from(outcome), Ordering::Relaxed);
    }
}

struct RawPortObservation {
    ifindex: u32,
    operation: u32,
    fallback_outcome: u32,
    finished: bool,
}

impl RawPortObservation {
    fn begin(ifindex: u32, operation: RawPortOperation, fallback_outcome: u32) -> Self {
        let observation = Self {
            ifindex,
            operation: operation as u32,
            fallback_outcome,
            finished: false,
        };
        esop_linux_raw_port_operation_begin_v1(observation.ifindex, observation.operation);
        observation
    }

    fn finish(&mut self, outcome: u32) -> bool {
        if self.finished {
            return false;
        }
        self.finished = true;
        esop_linux_raw_port_operation_end_v1(self.ifindex, self.operation, outcome);
        true
    }
}

impl Drop for RawPortObservation {
    fn drop(&mut self) {
        let _ = self.finish(self.fallback_outcome);
    }
}

pub struct LinuxRawPort {
    fd: RawFd,
    interface_index: libc::c_int,
    interface_name: [libc::c_char; libc::IFNAMSIZ],
    link_state: LinkState,
    started_at: Instant,
}

impl LinuxRawPort {
    pub fn open(interface: &str) -> io::Result<Self> {
        let interface_name = CString::new(interface).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "interface contains NUL byte")
        })?;
        let interface_index = unsafe { libc::if_nametoindex(interface_name.as_ptr()) };
        if interface_index == 0 {
            return Err(io::Error::last_os_error());
        }

        let protocol = libc::htons(ETHERCAT_ETHERTYPE);
        let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW, protocol as i32) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        if let Err(error) =
            set_nonblocking(fd).and_then(|_| bind_to_interface(fd, interface_index, protocol))
        {
            unsafe {
                libc::close(fd);
            }
            return Err(error);
        }

        let mut interface_name = [0; libc::IFNAMSIZ];
        // `CString` includes the trailing NUL and IFNAMSIZ is the kernel's
        // fixed interface-name storage size.
        let source = interface.as_bytes();
        if source.len() >= libc::IFNAMSIZ {
            unsafe {
                libc::close(fd);
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "interface name is too long",
            ));
        }
        for (target, byte) in interface_name.iter_mut().zip(source.iter().copied()) {
            *target = byte as libc::c_char;
        }

        let mut port = Self {
            fd,
            interface_index: interface_index as libc::c_int,
            interface_name,
            link_state: LinkState::Down,
            started_at: Instant::now(),
        };
        port.refresh_link_state()?;
        Ok(port)
    }

    pub fn interface_index(&self) -> libc::c_int {
        self.interface_index
    }

    pub fn raw_fd(&self) -> RawFd {
        self.fd
    }

    /// Refresh the cached interface state outside the hard realtime cycle.
    /// The cycle path reads the cache and never performs an ioctl.
    pub fn refresh_link_state(&mut self) -> io::Result<LinkState> {
        let mut request = unsafe { MaybeUninit::<libc::ifreq>::zeroed().assume_init() };
        request.ifr_name = self.interface_name;
        let result =
            unsafe { libc::ioctl(self.fd, libc::SIOCGIFFLAGS as libc::c_ulong, &mut request) };
        if result < 0 {
            self.link_state = LinkState::Down;
            return Err(io::Error::last_os_error());
        }

        let flags = unsafe { request.ifr_ifru.ifru_flags };
        self.link_state = if flags & 0x0001 != 0 && flags & 0x0040 != 0 {
            LinkState::Up
        } else {
            LinkState::Down
        };
        Ok(self.link_state)
    }
}

impl EthercatPort for LinuxRawPort {
    type Error = io::Error;

    fn link_state(&self) -> LinkState {
        self.link_state
    }

    fn now_ns(&self) -> u64 {
        self.started_at.elapsed().as_nanos().min(u64::MAX as u128) as u64
    }

    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        if frame.is_empty() || frame.len() > MAX_ETHERNET_FRAME_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "EtherCAT frame length is outside the port limit",
            ));
        }
        let mut observation = RawPortObservation::begin(
            self.interface_index as u32,
            RawPortOperation::Tx,
            RawPortTxOutcome::SyscallError as u32,
        );
        let written = unsafe { libc::send(self.fd, frame.as_ptr().cast(), frame.len(), 0) };
        if written < 0 {
            let error = io::Error::last_os_error();
            let _ = observation.finish(RawPortTxOutcome::SyscallError as u32);
            if matches!(
                error.raw_os_error(),
                Some(code) if code == libc::ENETDOWN || code == libc::ENETUNREACH
            ) {
                self.link_state = LinkState::Down;
            }
            return Err(error);
        }
        if written as usize != frame.len() {
            let _ = observation.finish(RawPortTxOutcome::PartialWrite as u32);
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "raw socket accepted only a partial frame",
            ));
        }
        let _ = observation.finish(RawPortTxOutcome::Success as u32);
        Ok(())
    }

    fn rx_poll(
        &mut self,
        destination: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, Self::Error> {
        let mut observation = RawPortObservation::begin(
            self.interface_index as u32,
            RawPortOperation::Rx,
            RawPortRxOutcome::SyscallError as u32,
        );
        let length = unsafe {
            libc::recv(
                self.fd,
                destination.as_mut_ptr().cast(),
                destination.len(),
                libc::MSG_DONTWAIT,
            )
        };
        if length >= 0 {
            let _ = observation.finish(RawPortRxOutcome::Frame as u32);
            return Ok(RxPoll::Frame(length as usize));
        }

        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EAGAIN) => {
                let _ = observation.finish(RawPortRxOutcome::Empty as u32);
                Ok(RxPoll::Empty)
            }
            Some(libc::ENETDOWN) | Some(libc::ENETUNREACH) => {
                let _ = observation.finish(RawPortRxOutcome::LinkDown as u32);
                self.link_state = LinkState::Down;
                Ok(RxPoll::LinkDown)
            }
            _ => {
                let _ = observation.finish(RawPortRxOutcome::SyscallError as u32);
                Err(error)
            }
        }
    }
}

impl EthercatDmaTxPort for LinuxRawPort {
    type Error = io::Error;

    fn tx_submit(&mut self, _: DmaTxHandle, frame: &[u8]) -> Result<(), Self::Error> {
        EthercatPort::tx_submit(self, frame)
    }
}

impl Drop for LinuxRawPort {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn bind_to_interface(
    fd: RawFd,
    interface_index: libc::c_uint,
    protocol: libc::c_ushort,
) -> io::Result<()> {
    let mut address = MaybeUninit::<libc::sockaddr_ll>::zeroed();
    let address_ptr = address.as_mut_ptr();
    unsafe {
        (*address_ptr).sll_family = libc::AF_PACKET as libc::c_ushort;
        (*address_ptr).sll_protocol = protocol;
        (*address_ptr).sll_ifindex = interface_index as libc::c_int;
    }
    let address = unsafe { address.assume_init() };
    let result = unsafe {
        libc::bind(
            fd,
            (&address as *const libc::sockaddr_ll).cast(),
            std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
        )
    };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_uses_ethercat_protocol_and_fixed_mtu() {
        assert_eq!(ETHERCAT_ETHERTYPE, 0x88A4);
        assert_eq!(MAX_ETHERNET_FRAME_LEN, 1518);
    }

    #[test]
    fn raw_port_marker_codes_are_append_only_and_bounded() {
        assert_eq!(RawPortOperation::Tx as u32, 0);
        assert_eq!(RawPortOperation::Rx as u32, 1);
        assert_eq!(RawPortTxOutcome::Success as u32, 0);
        assert_eq!(RawPortTxOutcome::SyscallError as u32, 1);
        assert_eq!(RawPortTxOutcome::PartialWrite as u32, 2);
        assert_eq!(RawPortRxOutcome::Frame as u32, 0);
        assert_eq!(RawPortRxOutcome::Empty as u32, 1);
        assert_eq!(RawPortRxOutcome::LinkDown as u32, 2);
        assert_eq!(RawPortRxOutcome::SyscallError as u32, 3);
    }

    #[test]
    fn raw_port_observation_finishes_once_and_drop_closes_errors() {
        let begins = TEST_RAW_PORT_MARKER_BEGINS.load(Ordering::Relaxed);
        let ends = TEST_RAW_PORT_MARKER_ENDS.load(Ordering::Relaxed);
        {
            let mut observation = RawPortObservation::begin(
                7,
                RawPortOperation::Tx,
                RawPortTxOutcome::SyscallError as u32,
            );
            assert!(observation.finish(RawPortTxOutcome::Success as u32));
            assert!(!observation.finish(RawPortTxOutcome::PartialWrite as u32));
        }
        assert_eq!(
            TEST_RAW_PORT_MARKER_BEGINS.load(Ordering::Relaxed),
            begins + 1
        );
        assert_eq!(TEST_RAW_PORT_MARKER_ENDS.load(Ordering::Relaxed), ends + 1);
        assert_eq!(
            TEST_RAW_PORT_MARKER_LAST_OUTCOME.load(Ordering::Relaxed),
            u64::from(RawPortTxOutcome::Success as u32)
        );

        {
            let _observation = RawPortObservation::begin(
                7,
                RawPortOperation::Rx,
                RawPortRxOutcome::SyscallError as u32,
            );
        }
        assert_eq!(
            TEST_RAW_PORT_MARKER_LAST_OUTCOME.load(Ordering::Relaxed),
            u64::from(RawPortRxOutcome::SyscallError as u32)
        );
    }
}
