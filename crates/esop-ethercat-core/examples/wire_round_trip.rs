//! Host-side linked example of the allocation-free wire layer.
//! This does not open a NIC or measure a complete master/embedded image.

use esop_ethercat_core::wire::{Command, FrameBuilder, FrameView, MAX_ETHERNET_FRAME_LEN};

fn main() {
    let mut buffer = [0; MAX_ETHERNET_FRAME_LEN];
    let payload = std::hint::black_box([0x12, 0x34, 0x56, 0x78]);
    let mut builder = FrameBuilder::new(&mut buffer, [0xff; 6], [0x02, 0, 0, 0, 0, 1])
        .expect("fixed buffer fits Ethernet headers");
    builder
        .push(Command::Lrw, 1, 0x1000, &payload)
        .expect("payload fits");
    let length = builder.finish().expect("one valid datagram");
    let frame = FrameView::parse(std::hint::black_box(&buffer[..length])).expect("valid frame");
    let datagram = frame.datagrams().next().unwrap().unwrap();
    assert_eq!(datagram.header.command, Command::Lrw);
    assert_eq!(datagram.payload, payload);
    assert_eq!(
        datagram.working_counter, 0,
        "request has not visited slaves"
    );
    println!(
        "encoded and parsed {length} bytes; fixed frame buffer: {MAX_ETHERNET_FRAME_LEN} bytes"
    );
}
