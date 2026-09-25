use esop_ethercat_linux_port::{RawPortOperation, RawPortRxOutcome, RawPortTxOutcome};

unsafe extern "C" {
    fn esop_linux_raw_port_operation_begin_v1(ifindex: u32, operation: u32);
    fn esop_linux_raw_port_operation_end_v1(ifindex: u32, operation: u32, outcome: u32);
}

#[test]
fn versioned_raw_port_markers_keep_their_exact_c_abi_symbols() {
    unsafe {
        esop_linux_raw_port_operation_begin_v1(7, RawPortOperation::Tx as u32);
        esop_linux_raw_port_operation_end_v1(
            7,
            RawPortOperation::Tx as u32,
            RawPortTxOutcome::Success as u32,
        );
        esop_linux_raw_port_operation_begin_v1(7, RawPortOperation::Rx as u32);
        esop_linux_raw_port_operation_end_v1(
            7,
            RawPortOperation::Rx as u32,
            RawPortRxOutcome::Empty as u32,
        );
    }
}
