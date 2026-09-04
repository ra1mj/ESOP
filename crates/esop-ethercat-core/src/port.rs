use crate::dma::DmaTxHandle;
use crate::wire::MAX_ETHERNET_FRAME_LEN;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkState {
    Down,
    Up,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RxPoll {
    Empty,
    Frame(usize),
    LinkDown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortError {
    BufferTooSmall,
    TxUnavailable,
    RxUnavailable,
    LinkDown,
    HardwareFault,
}

/// Minimal raw-frame boundary. Implementations may wrap DMA descriptors,
/// AF_PACKET for host HIL, or a vendor MAC without exposing it to the core.
pub trait EthercatPort {
    type Error;

    fn link_state(&self) -> LinkState;
    fn now_ns(&self) -> u64;

    /// Submit one fully-built Ethernet frame. The implementation must consume
    /// the bytes before returning; a DMA port may copy or provide a later
    /// descriptor-backed implementation behind the same lifecycle.
    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), Self::Error>;

    /// Poll one received frame into caller-provided storage. Returning
    /// `Empty` must be non-blocking.
    fn rx_poll(
        &mut self,
        destination: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, Self::Error>;
}

/// TX-only port boundary for the DMA hot path.
///
/// The core cleans the frame, publishes DMA ownership, and then calls this
/// method with the descriptor handle and borrowed frame bytes. An error must
/// mean that the port did not hand the descriptor to hardware; once hardware
/// owns it, the port must report completion through its platform-specific
/// completion path instead of returning an error from this method.
pub trait EthercatDmaTxPort {
    type Error;

    fn tx_submit(&mut self, handle: DmaTxHandle, frame: &[u8]) -> Result<(), Self::Error>;
}
