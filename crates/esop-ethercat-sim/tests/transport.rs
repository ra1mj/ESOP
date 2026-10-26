use esop_ethercat_core::wire::{Command, FrameBuilder, FrameView, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{
    BoundedSdoTransfer, EthercatPort, MailboxConfig, MailboxController, MailboxError, MailboxPhase,
    MailboxProtocol, MailboxRetryPolicy, RegisterOperation, RxPoll, SdoAccess, SdoAccessPolicy,
    SdoProgress,
};
use esop_ethercat_sim::{Fault, VirtualBus, fixtures};

fn wire<const N: usize>(
    bus: &mut VirtualBus<N>,
    command: Command,
    address: u32,
    payload: &[u8],
) -> Option<(Vec<u8>, u16)> {
    let mut tx = [0; MAX_ETHERNET_FRAME_LEN];
    let mut builder = FrameBuilder::new(&mut tx, [0xff; 6], [2, 0, 0, 0, 0, 1]).unwrap();
    builder.push(command, 1, address, payload).unwrap();
    let len = builder.finish().unwrap();
    bus.tx_submit(&tx[..len]).unwrap();
    let mut rx = [0; MAX_ETHERNET_FRAME_LEN];
    if let RxPoll::Frame(len) = bus.rx_poll(&mut rx).unwrap() {
        let view = FrameView::parse(&rx[..len]).unwrap();
        let d = view.datagrams().next().unwrap().unwrap();
        Some((d.payload.to_vec(), d.working_counter))
    } else {
        None
    }
}
fn new_bus() -> VirtualBus<1> {
    let mut b = VirtualBus::new([fixtures::drive(1, 0)]).unwrap();
    wire(&mut b, Command::Apwr, 0x0010_0000, &[1, 0x10]).unwrap();
    b
}

fn exchange(
    bus: &mut VirtualBus<1>,
    mailbox: &mut MailboxController,
    request: &[u8],
    drop_read: bool,
) -> Result<Vec<u8>, MailboxError> {
    let mut config = MailboxConfig::new(0x1000, 64, 0x1100, 64)
        .with_retry_policy(MailboxRetryPolicy::new(2, 10));
    config.request_timeout_ns = 10_000;
    config.timeout_ns = 50_000;
    mailbox.start(
        config,
        0x1001,
        1,
        bus.now_ns(),
        MailboxProtocol::CoE,
        request,
    )?;
    let mut dropped = false;
    for _ in 0..1000 {
        bus.advance_by(100).unwrap();
        if mailbox.phase() == MailboxPhase::Complete {
            return Ok(mailbox.response().unwrap().1.to_vec());
        }
        let Some(a) = mailbox.next_action(bus.now_ns())? else {
            continue;
        };
        let command = match a.operation {
            RegisterOperation::Read => Command::Fprd,
            RegisterOperation::Write => Command::Fpwr,
            _ => panic!("mailbox must use fixed addresses"),
        };
        if drop_read
            && !dropped
            && a.address == 0x1100_1001
            && a.operation == RegisterOperation::Read
        {
            bus.inject(Fault::DropNextResponse).unwrap();
            dropped = true;
        }
        let mut payload = [0; 128];
        payload[..a.payload().len()].copy_from_slice(a.payload());
        if let Some((data, wkc)) = wire(bus, command, a.address, &payload[..a.datagram_len()]) {
            mailbox.accept(a, 1, &data[..a.read_len as usize], wkc, bus.now_ns())?;
        } else {
            bus.advance_to(a.deadline_ns).unwrap();
            mailbox.timeout(a, bus.now_ns())?;
        }
    }
    panic!("mailbox made no progress")
}

#[test]
fn production_transport_recovers_consumed_read_without_resending_sdo() {
    let mut bus = new_bus();
    let mut mailbox = MailboxController::new();
    let response = exchange(&mut bus, &mut mailbox, &[0, 0x20, 0x40, 0, 0x20, 0], true).unwrap();
    assert_eq!(response, [0, 0x30, 0x43, 0, 0x20, 0, 0, 0, 0, 0]);
    assert_eq!(bus.slave(0).unwrap().mailbox_repeats, 1);
    assert!(!bus.slave(0).unwrap().mailbox_send_full);
    assert!(!bus.slave(0).unwrap().mailbox_receive_full);
}

#[test]
fn production_transport_times_out_at_original_deadline_when_repeat_ack_is_missing() {
    let mut bus = new_bus();
    bus.inject(Fault::RepeatAck {
        slave: 0,
        enabled: false,
    })
    .unwrap();
    assert_eq!(
        exchange(
            &mut bus,
            &mut MailboxController::new(),
            &[0, 0x20, 0x40, 0, 0x20, 0],
            true
        ),
        Err(MailboxError::Timeout)
    );
}

#[test]
fn production_coe_roundtrips_eight_one_kib_objects_over_bounded_physical_mailbox() {
    let mut bus = new_bus();
    let mut mailbox = MailboxController::new();
    for object in 0..8u16 {
        let bytes: Vec<u8> = (0..1024)
            .map(|n| (n as u8).wrapping_add(object as u8))
            .collect();
        let mut download = BoundedSdoTransfer::<1024>::with_mailbox_payload_capacity(
            SdoAccessPolicy::new(false),
            58,
        )
        .unwrap();
        download
            .start_download(0x2000 + object, 0, &bytes, SdoAccess::Single)
            .unwrap();
        for _ in 0..100 {
            let request = download.request().unwrap().to_vec();
            let reply = exchange(&mut bus, &mut mailbox, &request, false).unwrap();
            if download.accept_response(&reply).unwrap() == SdoProgress::Complete {
                break;
            }
        }
        assert!(download.request().is_none());
        let mut upload = BoundedSdoTransfer::<1024>::with_mailbox_payload_capacity(
            SdoAccessPolicy::new(false),
            58,
        )
        .unwrap();
        upload
            .start_upload(0x2000 + object, 0, SdoAccess::Single)
            .unwrap();
        for _ in 0..100 {
            let request = upload.request().unwrap().to_vec();
            let reply = exchange(&mut bus, &mut mailbox, &request, false).unwrap();
            if upload.accept_response(&reply).unwrap() == SdoProgress::Complete {
                break;
            }
        }
        assert_eq!(upload.data(), bytes);
    }
}

#[test]
fn production_transport_accepts_zero_and_nonsequential_counters_and_queues_idle_emergency() {
    let mut bus = new_bus();
    let mut mailbox = MailboxController::new();
    for counter in [0, 7, 2, 6, 1] {
        bus.inject(Fault::MailboxCounter {
            slave: 0,
            next: counter,
        })
        .unwrap();
        assert_eq!(
            exchange(&mut bus, &mut mailbox, &[0, 0x20, 0x40, 0, 0x20, 0], false).unwrap()[2],
            0x43
        );
    }
    // Queue an unsolicited message ahead of the normal response. The transport
    // must consume it, and continue the outstanding SDO without blocking.
    bus.emergency(0, 0x2310, 2, [1, 2, 3, 4, 5]).unwrap();
    exchange(&mut bus, &mut mailbox, &[0, 0x20, 0x40, 0, 0x20, 0], false).unwrap();
    assert_eq!(mailbox.pop_emergency().unwrap().error_code, 0x2310);
}

#[test]
fn eight_request_bank_fairly_completes_wire_uploads_and_isolates_object_abort() {
    use esop_ethercat_core::{
        RuntimeSdoService, SdoError, SdoRequestBank, SdoRequestError, SdoRequestPhase,
    };
    let mut bus = VirtualBus::<3>::new([
        fixtures::drive(1, 0),
        fixtures::drive(2, 0),
        fixtures::drive(3, 0),
    ])
    .unwrap();
    let mut bank = SdoRequestBank::<3>::new();
    for i in 0..3 {
        wire(
            &mut bus,
            Command::Apwr,
            0x0010_0000 | 0u16.wrapping_sub(i as u16) as u32,
            &(0x1001 + i as u16).to_le_bytes(),
        )
        .unwrap();
        bank.bind_verified_slave(
            i,
            0x1001 + i as u16,
            MailboxConfig::new(0x1000, 64, 0x1100, 64),
        )
        .unwrap();
    }
    let mut handles = Vec::new();
    for i in 0..8u16 {
        let bytes: Vec<u8> = (0..1024).map(|n| (n as u8).wrapping_add(i as u8)).collect();
        bus.set_object((i % 3) as usize, 0x2000 + i, 0, &bytes)
            .unwrap();
        handles.push(
            bank.submit_upload(
                0x1001 + i % 3,
                if i == 0 { 0x7777 } else { 0x2000 + i },
                0,
                SdoAccess::Single,
                SdoAccessPolicy::new(false),
                0,
                20_000_000,
            )
            .unwrap(),
        );
    }
    assert_eq!(
        bank.submit_upload(
            0x1001,
            0x2000,
            0,
            SdoAccess::Single,
            SdoAccessPolicy::new(false),
            0,
            1_000_000
        ),
        Err(SdoRequestError::QueueFull)
    );
    let mut first_stations = Vec::new();
    for _ in 0..20000 {
        if handles.iter().all(|h| {
            matches!(
                bank.status(*h).unwrap().phase,
                SdoRequestPhase::Complete | SdoRequestPhase::Failed
            )
        }) {
            break;
        }
        bus.advance_by(100).unwrap();
        if !bank.prepare(bus.now_ns()) {
            continue;
        }
        let a = bank.current_mailbox().unwrap().pending().unwrap();
        if first_stations.len() < 3 {
            first_stations.push(a.station_address);
        }
        let command = if a.operation == RegisterOperation::Read {
            Command::Fprd
        } else {
            Command::Fpwr
        };
        let mut bytes = [0; 128];
        bytes[..a.payload().len()].copy_from_slice(a.payload());
        let (reply, wkc) = wire(&mut bus, command, a.address, &bytes[..a.datagram_len()]).unwrap();
        bank.current_mailbox_mut()
            .unwrap()
            .accept(
                a,
                a.generation,
                &reply[..a.read_len as usize],
                wkc,
                bus.now_ns(),
            )
            .unwrap();
        bank.finish_wire(bus.now_ns());
    }
    assert_eq!(first_stations, [0x1001, 0x1002, 0x1003]);
    assert_eq!(
        bank.status(handles[0]).unwrap().error,
        Some(SdoRequestError::Sdo(SdoError::Abort(0x0602_0000)))
    );
    for (i, handle) in handles.iter().enumerate().skip(1) {
        let expected: Vec<u8> = (0..1024).map(|n| (n as u8).wrapping_add(i as u8)).collect();
        assert_eq!(bank.result(*handle).unwrap(), expected);
    }
    let old = handles[0];
    bank.release(old).unwrap();
    let replacement = bank
        .submit_upload(
            0x1001,
            0x2000,
            0,
            SdoAccess::Single,
            SdoAccessPolicy::new(false),
            bus.now_ns(),
            20_000_000,
        )
        .unwrap();
    assert_eq!(replacement.slot(), old.slot());
    assert_ne!(replacement.generation(), old.generation());
    assert_eq!(bank.status(old), Err(SdoRequestError::InvalidHandle));
}
