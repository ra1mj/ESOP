use esop_ethercat_core::wire::{Command, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{
    DatagramPlan, DmaDescriptorRing, DmaOwner, EthercatMaster, FramePlan, MasterConfig,
    NoopDmaCache, RxSlotState,
};
use esop_ethercat_linux_port::SimulatedPort;

#[test]
fn public_simulator_runs_a_preplanned_multi_datagram_cycle() {
    let mut master = EthercatMaster::<2, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut plan = FramePlan::<2>::new();
    plan.push(DatagramPlan {
        command: Command::Lrw,
        index: 10,
        address: 0x1000,
        payload_offset: 0,
        payload_len: 2,
        expected_wkc: 1,
    })
    .unwrap();
    plan.push(DatagramPlan {
        command: Command::Fprd,
        index: 11,
        address: 0x2000,
        payload_offset: 2,
        payload_len: 2,
        expected_wkc: 1,
    })
    .unwrap();

    let frame = master.acquire_frame(9, 100_000).unwrap();
    master
        .build_and_arm_frame_from_plan(frame, &plan, &[0x10, 0x11, 0x20, 0x21])
        .unwrap();
    assert_eq!(master.rx_entry(10).state, RxSlotState::Armed);
    assert_eq!(master.rx_entry(11).state, RxSlotState::Armed);

    let mut port = SimulatedPort::new(1);
    master.submit_frame(&mut port, frame).unwrap();
    let mut scratch = [0u8; MAX_ETHERNET_FRAME_LEN];
    let report = master.cycle_receive(&mut port, &mut scratch, 9).unwrap();

    assert_eq!(report.received_frames, 1);
    assert_eq!(report.parsed_datagrams, 2);
    assert_eq!(report.wkc_mismatches, 0);
    assert_eq!(master.rx_entry(10).state, RxSlotState::Empty);
    assert_eq!(master.rx_entry(11).state, RxSlotState::Empty);
    assert_eq!(port.tx_frames(), 1);
    assert_eq!(port.rx_frames(), 1);
}

#[test]
fn public_simulator_runs_the_dma_tx_hot_path() {
    let mut master = EthercatMaster::<2, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut plan = FramePlan::<1>::new();
    plan.push(DatagramPlan {
        command: Command::Lrw,
        index: 30,
        address: 0x3000,
        payload_offset: 0,
        payload_len: 3,
        expected_wkc: 1,
    })
    .unwrap();
    let mut ring = DmaDescriptorRing::<1, 0, MAX_ETHERNET_FRAME_LEN>::new();
    let (handle, length) = master
        .build_and_arm_dma_frame_from_plan(&mut ring, &plan, &[0x31, 0x32, 0x33], 9, 100_000)
        .unwrap();
    let mut port = SimulatedPort::new(1);
    let mut cache = NoopDmaCache;
    master
        .submit_dma_frame(&mut port, &mut ring, handle, length, &mut cache)
        .unwrap();
    assert_eq!(ring.tx_owner(handle), Ok(DmaOwner::DmaOwned));

    let mut scratch = [0u8; MAX_ETHERNET_FRAME_LEN];
    let report = master.cycle_receive(&mut port, &mut scratch, 9).unwrap();
    assert_eq!(report.received_frames, 1);
    assert_eq!(report.parsed_datagrams, 1);
    assert_eq!(master.rx_entry(30).state, RxSlotState::Empty);

    ring.tx_complete(handle, &mut cache).unwrap();
    ring.tx_reclaim(handle).unwrap();
    assert_eq!(ring.tx_owner(handle), Ok(DmaOwner::Free));
}
