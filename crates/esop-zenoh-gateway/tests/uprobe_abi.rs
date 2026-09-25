#![cfg(feature = "zenoh")]

use esop_zenoh_gateway::{KeySpace, RouteKind};

unsafe extern "C" {
    fn esop_zenoh_gateway_publish_begin_v1(request_id: u64, route_kind: u32);
    fn esop_zenoh_gateway_publish_end_v1(request_id: u64, route_kind: u32, outcome: u32);
}

#[test]
fn versioned_publish_markers_keep_their_exact_c_abi_symbols() {
    let key_space = KeySpace::new(b"fleet", b"robot").unwrap();
    assert_eq!(key_space.robot(), b"robot");

    unsafe {
        esop_zenoh_gateway_publish_begin_v1(7, RouteKind::State as u32);
        esop_zenoh_gateway_publish_end_v1(7, RouteKind::State as u32, 0);
    }
}
