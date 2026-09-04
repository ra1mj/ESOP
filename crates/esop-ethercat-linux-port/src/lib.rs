#![cfg(target_os = "linux")]
#![deny(unsafe_op_in_unsafe_fn)]

use esop_ethercat_core::wire::{ETHERCAT_ETHERTYPE, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{DmaTxHandle, EthercatDmaTxPort, EthercatPort, LinkState, RxPoll};
use std::ffi::CString;
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::RawFd;
use std::time::Instant;

mod sim;

pub use sim::SimulatedPort;

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
        let written = unsafe { libc::send(self.fd, frame.as_ptr().cast(), frame.len(), 0) };
        if written < 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.raw_os_error(),
                Some(code) if code == libc::ENETDOWN || code == libc::ENETUNREACH
            ) {
                self.link_state = LinkState::Down;
            }
            return Err(error);
        }
        if written as usize != frame.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "raw socket accepted only a partial frame",
            ));
        }
        Ok(())
    }

    fn rx_poll(
        &mut self,
        destination: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, Self::Error> {
        let length = unsafe {
            libc::recv(
                self.fd,
                destination.as_mut_ptr().cast(),
                destination.len(),
                libc::MSG_DONTWAIT,
            )
        };
        if length >= 0 {
            return Ok(RxPoll::Frame(length as usize));
        }

        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EAGAIN) => Ok(RxPoll::Empty),
            Some(libc::ENETDOWN) | Some(libc::ENETUNREACH) => {
                self.link_state = LinkState::Down;
                Ok(RxPoll::LinkDown)
            }
            _ => Err(error),
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
}
