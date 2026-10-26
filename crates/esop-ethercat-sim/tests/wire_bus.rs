use esop_ethercat_core::wire::MAX_ETHERNET_FRAME_LEN;
use esop_ethercat_core::{EthercatPort, RxPoll};
use esop_ethercat_sim::{Fault, SimError, VirtualBus, fixtures};

// This encoder is deliberately independent of the production codec. Numeric
// opcodes and on-wire byte positions are literal protocol facts.
fn frame(command: u8, adp: u16, ado: u16, data: &[u8]) -> Vec<u8> {
    let mut f = vec![0; 28 + data.len()];
    f[..6].fill(0xff);
    f[6..12].copy_from_slice(&[2, 0, 0, 0, 0, 1]);
    f[12..14].copy_from_slice(&[0x88, 0xa4]);
    f[14..16].copy_from_slice(&(0x1000 | ((12 + data.len()) as u16)).to_le_bytes());
    f[16] = command;
    f[17] = 0xa5;
    f[18..20].copy_from_slice(&adp.to_le_bytes());
    f[20..22].copy_from_slice(&ado.to_le_bytes());
    f[22..24].copy_from_slice(&(data.len() as u16).to_le_bytes());
    f[26..26 + data.len()].copy_from_slice(data);
    f
}
fn exchange<const N: usize, const Q: usize>(
    bus: &mut VirtualBus<N, Q>,
    command: u8,
    adp: u16,
    ado: u16,
    data: &[u8],
) -> Vec<u8> {
    let f = frame(command, adp, ado, data);
    bus.tx_submit(&f).unwrap();
    let mut b = [0; MAX_ETHERNET_FRAME_LEN];
    let RxPoll::Frame(len) = bus.rx_poll(&mut b).unwrap() else {
        panic!("response missing")
    };
    b[..len].to_vec()
}
fn data(f: &[u8]) -> &[u8] {
    &f[26..f.len() - 2]
}
fn wkc(f: &[u8]) -> u16 {
    u16::from_le_bytes(f[f.len() - 2..].try_into().unwrap())
}
fn station<const N: usize, const Q: usize>(
    bus: &mut VirtualBus<N, Q>,
    position: usize,
    address: u16,
) {
    assert_eq!(
        wkc(&exchange(
            bus,
            2,
            0u16.wrapping_sub(position as u16),
            0x10,
            &address.to_le_bytes()
        )),
        1
    );
}
fn configure_drive(bus: &mut VirtualBus<1>) {
    station(bus, 0, 0x1001);
    for (offset, bytes) in [
        (0x0810, &[0, 0x14, 7, 0, 0x24, 0, 1, 0][..]),
        (0x0818, &[0, 0x15, 15, 0, 0x20, 0, 1, 0][..]),
    ] {
        assert_eq!(wkc(&exchange(bus, 5, 0x1001, offset, bytes)), 1);
    }
    let output = [0, 0x10, 0, 0, 7, 0, 0, 7, 0, 0x14, 0, 2, 1, 0, 0, 0];
    let input = [7, 0x10, 0, 0, 15, 0, 0, 7, 0, 0x15, 0, 1, 1, 0, 0, 0];
    exchange(bus, 5, 0x1001, 0x600, &output);
    exchange(bus, 5, 0x1001, 0x610, &input);
    exchange(bus, 5, 0x1001, 0x120, &[2, 0]);
    exchange(bus, 5, 0x1001, 0x120, &[4, 0]);
    exchange(bus, 5, 0x1001, 0x120, &[8, 0]);
    assert_eq!(bus.slave(0).unwrap().al_state, 8);
}

#[test]
fn golden_broadcast_response_counts_real_escs_and_advances_adp() {
    let mut bus = VirtualBus::<3>::new(fixtures::dual_axis_io()).unwrap();
    let raw = [
        255, 255, 255, 255, 255, 255, 2, 0, 0, 0, 0, 1, 0x88, 0xa4, 14, 0x10, 7, 0xa5, 0, 0, 0, 0,
        2, 0, 0, 0, 0, 0, 0, 0,
    ];
    bus.tx_submit(&raw).unwrap();
    let mut b = [0; MAX_ETHERNET_FRAME_LEN];
    assert_eq!(bus.rx_poll(&mut b), Ok(RxPoll::Frame(30)));
    assert_eq!(
        &b[16..30],
        &[7, 0xa5, 3, 0, 0, 0, 2, 0, 0, 0, 0x11, 1, 3, 0]
    );
    assert_eq!(bus.slave(0).unwrap().station_address, 0);
}

#[test]
fn auto_increment_station_assignment_fixed_read_and_readwrite_are_stateful() {
    let mut bus = VirtualBus::<3>::new(fixtures::dual_axis_io()).unwrap();
    for i in 0..3 {
        station(&mut bus, i, 0x1001 + i as u16);
    }
    let r = exchange(&mut bus, 1, 0xffff, 0x10, &[0, 0]);
    assert_eq!(data(&r), [2, 0x10]);
    assert_eq!(&r[18..20], &2u16.to_le_bytes());
    let r = exchange(&mut bus, 6, 0x1002, 0x10, &[0x23, 1]);
    assert_eq!(wkc(&r), 3);
    assert_eq!(data(&r), [2, 0x10]);
    assert_eq!(bus.slave(1).unwrap().station_address, 0x123);
    assert_eq!(wkc(&exchange(&mut bus, 4, 0x9999, 0x10, &[0, 0])), 0);
    assert_eq!(wkc(&exchange(&mut bus, 1, 0xfffd, 0, &[0, 0])), 0);
}

#[test]
fn all_physical_opcodes_and_read_multiple_write_use_bus_state() {
    let mut bus = VirtualBus::<3>::new(fixtures::dual_axis_io()).unwrap();
    for i in 0..3 {
        station(&mut bus, i, 0x10 + i as u16);
    }
    assert_eq!(wkc(&exchange(&mut bus, 0, 0, 0, &[])), 0);
    assert_eq!(wkc(&exchange(&mut bus, 3, 0, 0x10, &[0x10, 0])), 3);
    assert_eq!(wkc(&exchange(&mut bus, 4, 0x10, 0, &[0; 2])), 1);
    assert_eq!(wkc(&exchange(&mut bus, 5, 0x10, 0x12, &[0x55, 0])), 1);
    assert_eq!(wkc(&exchange(&mut bus, 8, 0, 0x12, &[0x66, 0])), 3);
    let r = exchange(&mut bus, 9, 0, 0x12, &[0, 0]);
    assert_eq!(wkc(&r), 9);
    assert_eq!(data(&r), [0x66, 0]);
    let r = exchange(&mut bus, 13, 0xffff, 0x10, &[0, 0]);
    assert_eq!(data(&r), [0x11, 0]);
    assert_eq!(bus.slave(2).unwrap().station_address, 0x11);
    assert_eq!(wkc(&r), 3);
    let r = exchange(&mut bus, 14, 0x10, 0x12, &[0, 0]);
    assert_eq!(data(&r), [0, 0]);
    assert_eq!(wkc(&r), 3);
}

#[test]
fn one_eight_and_thirty_two_device_counts_are_actual_wire_counts() {
    fn count<const N: usize>() {
        let mut bus = VirtualBus::<N>::new([fixtures::drive(1, 0); N]).unwrap();
        let r = exchange(&mut bus, 7, 0, 0, &[0; 2]);
        assert_eq!(wkc(&r), N as u16);
        assert_eq!(&r[18..20], &(N as u16).to_le_bytes());
    }
    count::<1>();
    count::<8>();
    // The 32 ESC fixture exceeds small platform test-thread stacks during
    // construction; runtime uses fixed caller-owned storage thereafter.
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(count::<32>)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn invalid_second_datagram_does_not_partially_change_station_address() {
    let mut bus = VirtualBus::<1>::new([fixtures::drive(1, 0)]).unwrap();
    let mut f = frame(2, 0, 0x10, &[0x99, 0]);
    f[23] |= 0x80;
    let second = frame(4, 0x99, 0, &[0; 2]);
    f.extend_from_slice(&second[16..]);
    f[14..16].copy_from_slice(&0x101cu16.to_le_bytes());
    f[30] = 0xff;
    assert_eq!(bus.tx_submit(&f), Err(SimError::InvalidFrame));
    assert_eq!(bus.slave(0).unwrap().station_address, 0);
    f[30] = 4;
    f[37] &= !0x80;
    assert_eq!(bus.tx_submit(&f), Ok(()));
    assert_eq!(bus.slave(0).unwrap().station_address, 0x99);
}

#[test]
fn eeprom_identity_serial_and_category_data_are_device_owned() {
    let mut bus = VirtualBus::<1>::new([fixtures::drive(0x12345678, 0x41)]).unwrap();
    station(&mut bus, 0, 0x1001);
    exchange(&mut bus, 5, 0x1001, 0x504, &[8, 0, 0, 0]);
    exchange(&mut bus, 5, 0x1001, 0x502, &[0, 1]);
    assert_eq!(
        data(&exchange(&mut bus, 4, 0x1001, 0x508, &[0; 8])),
        [0, 0xe5, 0, 0, 0x20, 0x40, 0, 0]
    );
    exchange(&mut bus, 5, 0x1001, 0x504, &[0xe, 0, 0, 0]);
    exchange(&mut bus, 5, 0x1001, 0x502, &[0, 1]);
    assert_eq!(
        data(&exchange(&mut bus, 4, 0x1001, 0x508, &[0; 4])),
        [0x78, 0x56, 0x34, 0x12]
    );
    exchange(&mut bus, 5, 0x1001, 0x120, &[0x21, 0]);
    assert_eq!(
        data(&exchange(&mut bus, 4, 0x1001, 0x130, &[0; 6])),
        [0x21, 0, 0, 0, 0x41, 0]
    );
}

#[test]
fn logical_data_requires_mapping_and_al_then_drive_handshake_uses_real_pdo_bytes() {
    let mut bus = VirtualBus::<1>::new([fixtures::drive(1, 0)]).unwrap();
    assert_eq!(
        wkc(&exchange(&mut bus, 11, 0x1000, 0, &[6, 0, 8, 0, 0, 0, 0])),
        0
    );
    configure_drive(&mut bus);
    exchange(&mut bus, 11, 0x1000, 0, &[15, 0, 8, 0, 0, 0, 0]);
    bus.advance_by(1_000_000).unwrap();
    assert_eq!(
        bus.slave(0).unwrap().statusword,
        0x40,
        "Enable Operation must not bypass the PDS handshake"
    );
    for (cw, status) in [(6, 0x21), (7, 0x23), (15, 0x27)] {
        assert_eq!(
            wkc(&exchange(&mut bus, 11, 0x1000, 0, &[cw, 0, 8, 0, 0, 0, 0])),
            1
        );
        bus.advance_by(1_000_000).unwrap();
        assert_eq!(bus.slave(0).unwrap().statusword, status);
    }
    exchange(&mut bus, 11, 0x1000, 0, &[15, 0, 8, 0x34, 0x12, 0, 0]);
    bus.advance_by(1_000_000).unwrap();
    let r = exchange(&mut bus, 10, 0x1007, 0, &[0; 15]);
    assert_eq!(wkc(&r), 1);
    assert_eq!(&data(&r)[..9], &[0x27, 0, 8, 0, 0, 0x34, 0x12, 0, 0]);
    let r = exchange(&mut bus, 12, 0x1000, 0, &[0; 22]);
    assert_eq!(wkc(&r), 3);
    bus.inject(Fault::Drive {
        slave: 0,
        code: 0x2310,
    })
    .unwrap();
    bus.advance_by(1_000_000).unwrap();
    assert_eq!(bus.slave(0).unwrap().statusword, 8);
    bus.inject(Fault::Al {
        slave: 0,
        code: 0x1b,
    })
    .unwrap();
    assert_eq!(bus.slave(0).unwrap().al_error, 0x1b);
}

#[test]
fn multiple_read_fmmus_on_one_esc_contribute_one_working_counter() {
    let mut bus = VirtualBus::<1>::new([fixtures::drive(1, 0)]).unwrap();
    configure_drive(&mut bus);
    // PDO input [0x1007,0x1016) and a disjoint mailbox status bit 0x1016
    // both access the same physical ESC in one LRD.
    let status = [0x16, 0x10, 0, 0, 1, 0, 0, 0, 0x0d, 8, 3, 1, 1, 0, 0, 0];
    exchange(&mut bus, 5, 0x1001, 0x620, &status);
    bus.emergency(0, 0x1234, 1, [0; 5]).unwrap();
    let r = exchange(&mut bus, 10, 0x1007, 0, &[0; 16]);
    assert_eq!(wkc(&r), 1);
    assert_eq!(data(&r)[15] & 1, 1);
}

#[test]
fn physical_sync_manager_activation_masks_sii_only_reserved_bits() {
    let mut bus = VirtualBus::<1>::new([fixtures::drive(1, 0)]).unwrap();
    station(&mut bus, 0, 0x1001);
    exchange(&mut bus, 5, 0x1001, 0x816, &[9]);
    assert_eq!(data(&exchange(&mut bus, 4, 0x1001, 0x816, &[0])), [1]);
    exchange(&mut bus, 5, 0x1001, 0x816, &[8]);
    assert_eq!(data(&exchange(&mut bus, 4, 0x1001, 0x816, &[0])), [0]);
    exchange(&mut bus, 5, 0x1001, 0x816, &[0xff]);
    assert_eq!(data(&exchange(&mut bus, 4, 0x1001, 0x816, &[0])), [0xc3]);
}

#[test]
fn dc_time_follows_virtual_time_offsets_and_reference_multiple_write() {
    let mut bus = VirtualBus::<3>::new(fixtures::dual_axis_io()).unwrap();
    for i in 0..3 {
        station(&mut bus, i, 0x1001 + i as u16);
    }
    bus.advance_to(1_234_567).unwrap();
    let r = exchange(&mut bus, 4, 0x1001, 0x910, &[0; 8]);
    assert_eq!(data(&r), 1_234_567u64.to_le_bytes());
    exchange(&mut bus, 5, 0x1001, 0x920, &100i64.to_le_bytes());
    bus.inject(Fault::DcOffset {
        slave: 1,
        nanoseconds: 700,
    })
    .unwrap();
    let r = exchange(&mut bus, 14, 0x1001, 0x910, &[0; 8]);
    assert_eq!(wkc(&r), 2);
    assert_eq!(data(&r), 1_234_667u64.to_le_bytes());
    assert_eq!(
        data(&exchange(&mut bus, 4, 0x1002, 0x910, &[0; 8])),
        1_234_667u64.to_le_bytes()
    );
    assert_eq!(wkc(&exchange(&mut bus, 4, 0x1003, 0x910, &[0; 8])), 0);
}

#[test]
fn delay_duplicate_corrupt_and_drop_are_bounded_and_preserve_consumed_wire_effects() {
    let mut bus = VirtualBus::<1, 2>::new([fixtures::drive(1, 0)]).unwrap();
    let f = frame(2, 0, 0x10, &[1, 0]);
    bus.inject(Fault::DropNextResponse).unwrap();
    bus.tx_submit(&f).unwrap();
    assert_eq!(bus.slave(0).unwrap().station_address, 1);
    let mut b = [0; MAX_ETHERNET_FRAME_LEN];
    assert_eq!(bus.rx_poll(&mut b), Ok(RxPoll::Empty));
    bus.inject(Fault::DelayNextResponse(20)).unwrap();
    bus.tx_submit(&f).unwrap();
    bus.advance_by(19).unwrap();
    assert_eq!(bus.rx_poll(&mut b), Ok(RxPoll::Empty));
    bus.advance_by(1).unwrap();
    assert_eq!(bus.rx_poll(&mut b), Ok(RxPoll::Frame(f.len())));
    bus.inject(Fault::DuplicateNextResponse).unwrap();
    bus.tx_submit(&f).unwrap();
    assert_eq!(bus.tx_submit(&f), Err(SimError::QueueFull));
    assert_eq!(bus.rx_poll(&mut b), Ok(RxPoll::Frame(f.len())));
    assert_eq!(bus.rx_poll(&mut b), Ok(RxPoll::Frame(f.len())));
    bus.inject(Fault::CorruptNextResponse {
        offset: 17,
        xor: 0xff,
    })
    .unwrap();
    bus.tx_submit(&f).unwrap();
    bus.rx_poll(&mut b).unwrap();
    assert_eq!(b[17], 0x5a);
    assert_eq!(bus.advance_to(0), Err(SimError::TimeRegression));
    bus.inject(Fault::SlaveLink {
        slave: 0,
        up: false,
    })
    .unwrap();
    assert_eq!(wkc(&exchange(&mut bus, 7, 0, 0, &[0; 2])), 0);
}

fn sdo(bus: &mut VirtualBus<1>, payload: &[u8]) -> Vec<u8> {
    let mut mb = [0; 64];
    mb[..2].copy_from_slice(&(payload.len() as u16).to_le_bytes());
    mb[5] = 0x13;
    mb[6..6 + payload.len()].copy_from_slice(payload);
    assert_eq!(wkc(&exchange(bus, 5, 0x1001, 0x1000, &mb)), 1);
    assert!(bus.slave(0).unwrap().mailbox_send_full);
    bus.advance_by(100).unwrap();
    let r = exchange(bus, 4, 0x1001, 0x1100, &[0; 64]);
    assert_eq!(wkc(&r), 1);
    let m = data(&r);
    let n = u16::from_le_bytes([m[0], m[1]]) as usize;
    m[6..6 + n].to_vec()
}

#[test]
fn mailbox_ownership_normal_upload_download_segments_aborts_and_repeat_are_real() {
    let mut bus = VirtualBus::<1>::new([fixtures::drive(1, 0)]).unwrap();
    station(&mut bus, 0, 0x1001);
    let value: Vec<u8> = (0..130).map(|v| v as u8).collect();
    bus.set_object(0, 0x2001, 0, &value).unwrap();
    let r = sdo(&mut bus, &[0, 0x20, 0x40, 1, 0x20, 0]);
    assert_eq!(r[2], 0x41);
    assert_eq!(u32::from_le_bytes(r[6..10].try_into().unwrap()), 130);
    assert_eq!(&r[10..], &value[..48]);
    let r = sdo(&mut bus, &[0, 0x20, 0x60]);
    assert_eq!(r[2], 0);
    assert_eq!(&r[3..], &value[48..103]);
    let r = sdo(&mut bus, &[0, 0x20, 0x70]);
    assert_eq!(r[2], 0x11);
    assert_eq!(&r[3..], &value[103..]);
    // Unknown object abort retains exact object correlation.
    assert_eq!(
        sdo(&mut bus, &[0, 0x20, 0x40, 0xff, 0x7f, 0]),
        [0, 0x30, 0x80, 0xff, 0x7f, 0, 0, 0, 2, 6]
    );
    // Last seven-byte segment carries explicit unused padding.
    assert_eq!(
        sdo(&mut bus, &[0, 0x20, 0x21, 2, 0x20, 0, 9, 0, 0, 0])[2],
        0x60
    );
    assert_eq!(sdo(&mut bus, &[0, 0x20, 0, 1, 2, 3, 4, 5, 6, 7])[2], 0x20);
    assert_eq!(
        sdo(&mut bus, &[0, 0x20, 0x1b, 8, 9, 0, 0, 0, 0, 0])[2],
        0x30
    );
    assert_eq!(
        &sdo(&mut bus, &[0, 0x20, 0x40, 2, 0x20, 0])[10..],
        &[1, 2, 3, 4, 5, 6, 7, 8, 9]
    );
    // A consumed mailbox read is not restored by polling. Toggle Repeat Request
    // in Activation+6 and observe ACK in PDI+7 before receiving identical data.
    let previous = data(&exchange(&mut bus, 4, 0x1001, 0x80d, &[0]))[0];
    assert_eq!(previous & 8, 0);
    exchange(&mut bus, 5, 0x1001, 0x80e, &[3]);
    assert_eq!(data(&exchange(&mut bus, 4, 0x1001, 0x80f, &[0]))[0] & 2, 2);
    assert_eq!(wkc(&exchange(&mut bus, 4, 0x1001, 0x1100, &[0; 64])), 1);
    assert_eq!(bus.slave(0).unwrap().mailbox_repeats, 1);
}

#[test]
fn unsolicited_emergency_zero_counter_and_missing_repeat_ack_are_explicit() {
    let mut bus = VirtualBus::<1>::new([fixtures::drive(1, 0)]).unwrap();
    station(&mut bus, 0, 0x1001);
    bus.inject(Fault::MailboxCounter { slave: 0, next: 0 })
        .unwrap();
    bus.emergency(0, 0x1234, 2, [1, 2, 3, 4, 5]).unwrap();
    assert_eq!(bus.emergency(0, 0, 0, [0; 5]), Err(SimError::MailboxFull));
    let r = exchange(&mut bus, 4, 0x1001, 0x1100, &[0; 64]);
    assert_eq!(data(&r)[5], 3);
    assert_eq!(&data(&r)[6..16], &[0, 0x10, 0x34, 0x12, 2, 1, 2, 3, 4, 5]);
    bus.inject(Fault::RepeatAck {
        slave: 0,
        enabled: false,
    })
    .unwrap();
    exchange(&mut bus, 5, 0x1001, 0x80e, &[3]);
    assert_eq!(data(&exchange(&mut bus, 4, 0x1001, 0x80f, &[0]))[0] & 2, 0);
    assert!(!bus.slave(0).unwrap().mailbox_receive_full);
}

#[test]
fn application_uses_standard_coe_mapping_descriptors_after_real_remapping() {
    let mut bus = VirtualBus::<1>::new([fixtures::drive(1, 0)]).unwrap();
    station(&mut bus, 0, 0x1001);
    exchange(&mut bus, 5, 0x1001, 0x120, &[2, 0]);
    // Clear mapping count, then remap mode before Controlword. Descriptor bytes
    // are literal CANopen uint32: bit length, subindex, index low, index high.
    assert_eq!(
        sdo(&mut bus, &[0, 0x20, 0x2f, 0, 0x16, 0, 0, 0, 0, 0])[2],
        0x60
    );
    for (sub, descriptor) in [
        (1, [8, 0, 0x60, 0x60]),
        (2, [16, 0, 0x40, 0x60]),
        (3, [32, 0, 0x7a, 0x60]),
    ] {
        let mut payload = [0, 0x20, 0x23, 0, 0x16, sub, 0, 0, 0, 0];
        payload[6..].copy_from_slice(&descriptor);
        assert_eq!(sdo(&mut bus, &payload)[2], 0x60);
    }
    assert_eq!(
        sdo(&mut bus, &[0, 0x20, 0x2f, 0, 0x16, 0, 3, 0, 0, 0])[2],
        0x60
    );
    // Remap feedback independently: position, Statusword, mode, error.
    assert_eq!(
        sdo(&mut bus, &[0, 0x20, 0x2f, 0, 0x1a, 0, 0, 0, 0, 0])[2],
        0x60
    );
    for (sub, descriptor) in [
        (1, [32, 0, 0x64, 0x60]),
        (2, [16, 0, 0x41, 0x60]),
        (3, [8, 0, 0x61, 0x60]),
        (4, [16, 0, 0x3f, 0x60]),
    ] {
        let mut payload = [0, 0x20, 0x23, 0, 0x1a, sub, 0, 0, 0, 0];
        payload[6..].copy_from_slice(&descriptor);
        assert_eq!(sdo(&mut bus, &payload)[2], 0x60);
    }
    assert_eq!(
        sdo(&mut bus, &[0, 0x20, 0x2f, 0, 0x1a, 0, 4, 0, 0, 0])[2],
        0x60
    );
    configure_drive(&mut bus);
    for (cw, status) in [(6, 0x21), (7, 0x23), (15, 0x27)] {
        exchange(&mut bus, 11, 0x1000, 0, &[8, cw, 0, 0x34, 0x12, 0, 0]);
        bus.advance_by(1_000_000).unwrap();
        assert_eq!(bus.slave(0).unwrap().statusword, status);
    }
    assert_eq!(
        &data(&exchange(&mut bus, 10, 0x1007, 0, &[0; 15]))[..9],
        &[0x34, 0x12, 0, 0, 0x27, 0, 8, 0, 0]
    );
}
