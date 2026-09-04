//! Deterministic EtherCAT wire simulator for host tests and HIL scaffolding.
//!
//! The simulator models the port boundary only. It echoes submitted frames,
//! rewrites every datagram working counter, and exposes explicit controls for
//! link loss, dropped responses, injected frames, and virtual time. It does
//! not pretend to be a complete ESC or PDO device model.

use esop_ethercat_core::wire::{
    DatagramHeader, ETHERCAT_FRAME_HEADER_LEN, ETHERNET_HEADER_LEN, FrameView,
    MAX_ETHERNET_FRAME_LEN, WORKING_COUNTER_LEN,
};
use esop_ethercat_core::{
    DmaTxHandle, EthercatDmaTxPort, EthercatPort, LinkState, PortError, RxPoll,
};

#[derive(Debug)]
pub struct SimulatedPort {
    pending: [u8; MAX_ETHERNET_FRAME_LEN],
    pending_len: usize,
    pending_ready: bool,
    link_state: LinkState,
    now_ns: u64,
    response_wkc: u16,
    drop_next_response: bool,
    fail_next_tx: bool,
    tx_frames: usize,
    rx_frames: usize,
}

impl SimulatedPort {
    pub const fn new(response_wkc: u16) -> Self {
        Self {
            pending: [0; MAX_ETHERNET_FRAME_LEN],
            pending_len: 0,
            pending_ready: false,
            link_state: LinkState::Up,
            now_ns: 0,
            response_wkc,
            drop_next_response: false,
            fail_next_tx: false,
            tx_frames: 0,
            rx_frames: 0,
        }
    }

    pub const fn response_wkc(&self) -> u16 {
        self.response_wkc
    }

    pub fn set_response_wkc(&mut self, response_wkc: u16) {
        self.response_wkc = response_wkc;
    }

    pub const fn tx_frames(&self) -> usize {
        self.tx_frames
    }

    pub const fn rx_frames(&self) -> usize {
        self.rx_frames
    }

    pub const fn pending(&self) -> bool {
        self.pending_ready
    }

    pub const fn now_ns_value(&self) -> u64 {
        self.now_ns
    }

    pub fn set_now_ns(&mut self, now_ns: u64) {
        self.now_ns = now_ns;
    }

    pub fn advance_ns(&mut self, delta_ns: u64) {
        self.now_ns = self.now_ns.saturating_add(delta_ns);
    }

    pub fn set_link_state(&mut self, link_state: LinkState) {
        self.link_state = link_state;
    }

    /// The next valid TX is accepted but produces no RX response.
    pub fn drop_next_response(&mut self) {
        self.drop_next_response = true;
    }

    /// The next TX returns a deterministic hardware error.
    pub fn fail_next_tx(&mut self) {
        self.fail_next_tx = true;
    }

    /// Inject a raw frame as the next RX result without parsing or rewriting.
    pub fn inject_rx_frame(&mut self, frame: &[u8]) -> Result<(), PortError> {
        if frame.is_empty() || frame.len() > MAX_ETHERNET_FRAME_LEN {
            return Err(PortError::BufferTooSmall);
        }
        self.pending[..frame.len()].copy_from_slice(frame);
        self.pending_len = frame.len();
        self.pending_ready = true;
        Ok(())
    }

    fn build_response(&mut self, frame: &[u8]) -> Result<(), PortError> {
        if frame.is_empty() || frame.len() > MAX_ETHERNET_FRAME_LEN {
            return Err(PortError::BufferTooSmall);
        }
        self.pending[..frame.len()].copy_from_slice(frame);
        let datagram_count = FrameView::parse(&self.pending[..frame.len()])
            .map_err(|_| PortError::HardwareFault)?
            .datagram_count();

        let mut offset = ETHERNET_HEADER_LEN + ETHERCAT_FRAME_HEADER_LEN;
        for _ in 0..datagram_count {
            let header_end = offset
                .checked_add(esop_ethercat_core::wire::DATAGRAM_HEADER_LEN)
                .ok_or(PortError::HardwareFault)?;
            let header = DatagramHeader::decode(&self.pending[offset..header_end])
                .map_err(|_| PortError::HardwareFault)?;
            let payload_end = header_end
                .checked_add(header.length as usize)
                .ok_or(PortError::HardwareFault)?;
            let wkc_end = payload_end
                .checked_add(WORKING_COUNTER_LEN)
                .ok_or(PortError::HardwareFault)?;
            if wkc_end > frame.len() {
                return Err(PortError::HardwareFault);
            }
            self.pending[payload_end..wkc_end].copy_from_slice(&self.response_wkc.to_le_bytes());
            offset = wkc_end;
        }

        self.pending_len = frame.len();
        self.pending_ready = !self.drop_next_response;
        self.drop_next_response = false;
        Ok(())
    }
}

impl Default for SimulatedPort {
    fn default() -> Self {
        Self::new(1)
    }
}

impl EthercatPort for SimulatedPort {
    type Error = PortError;

    fn link_state(&self) -> LinkState {
        self.link_state
    }

    fn now_ns(&self) -> u64 {
        self.now_ns
    }

    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        if self.link_state == LinkState::Down {
            return Err(PortError::LinkDown);
        }
        if self.fail_next_tx {
            self.fail_next_tx = false;
            return Err(PortError::HardwareFault);
        }
        self.build_response(frame)?;
        self.tx_frames = self.tx_frames.saturating_add(1);
        Ok(())
    }

    fn rx_poll(
        &mut self,
        destination: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, Self::Error> {
        if self.link_state == LinkState::Down {
            return Ok(RxPoll::LinkDown);
        }
        if !self.pending_ready {
            return Ok(RxPoll::Empty);
        }
        destination[..self.pending_len].copy_from_slice(&self.pending[..self.pending_len]);
        self.pending_ready = false;
        self.rx_frames = self.rx_frames.saturating_add(1);
        Ok(RxPoll::Frame(self.pending_len))
    }
}

impl EthercatDmaTxPort for SimulatedPort {
    type Error = PortError;

    fn tx_submit(&mut self, _: DmaTxHandle, frame: &[u8]) -> Result<(), Self::Error> {
        EthercatPort::tx_submit(self, frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use esop_ethercat_core::wire::{Command, FrameBuilder};

    #[test]
    fn echoes_multiple_datagrams_and_rewrites_working_counters() {
        let mut tx = [0u8; MAX_ETHERNET_FRAME_LEN];
        let mut builder = FrameBuilder::new(&mut tx, [0xFF; 6], [1, 2, 3, 4, 5, 6]).unwrap();
        builder.push(Command::Lrw, 1, 0x1000, &[1, 2]).unwrap();
        builder.push(Command::Fprd, 2, 0x2000, &[3, 4, 5]).unwrap();
        let length = builder.finish().unwrap();

        let mut port = SimulatedPort::new(7);
        EthercatPort::tx_submit(&mut port, &tx[..length]).unwrap();
        let mut rx = [0u8; MAX_ETHERNET_FRAME_LEN];
        assert_eq!(port.rx_poll(&mut rx), Ok(RxPoll::Frame(length)));
        let view = FrameView::parse(&rx[..length]).unwrap();
        let datagrams: Vec<_> = view.datagrams().map(Result::unwrap).collect();
        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[0].payload, &[1, 2]);
        assert_eq!(datagrams[0].working_counter, 7);
        assert_eq!(datagrams[1].working_counter, 7);
        assert_eq!(port.tx_frames(), 1);
        assert_eq!(port.rx_frames(), 1);
    }

    #[test]
    fn drop_and_link_controls_are_deterministic() {
        let mut tx = [0u8; MAX_ETHERNET_FRAME_LEN];
        let mut builder = FrameBuilder::new(&mut tx, [0xFF; 6], [1, 2, 3, 4, 5, 6]).unwrap();
        builder.push(Command::Lrd, 1, 0, &[]).unwrap();
        let length = builder.finish().unwrap();

        let mut port = SimulatedPort::default();
        port.drop_next_response();
        EthercatPort::tx_submit(&mut port, &tx[..length]).unwrap();
        let mut rx = [0u8; MAX_ETHERNET_FRAME_LEN];
        assert_eq!(port.rx_poll(&mut rx), Ok(RxPoll::Empty));

        port.set_link_state(LinkState::Down);
        assert_eq!(
            EthercatPort::tx_submit(&mut port, &tx[..length]),
            Err(PortError::LinkDown)
        );
        assert_eq!(port.rx_poll(&mut rx), Ok(RxPoll::LinkDown));
        port.set_now_ns(10);
        port.advance_ns(5);
        assert_eq!(port.now_ns_value(), 15);
    }
}
