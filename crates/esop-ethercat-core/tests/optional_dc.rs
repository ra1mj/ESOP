use esop_ethercat_core::wire::MAX_ETHERNET_FRAME_LEN;
use esop_ethercat_core::*;

struct Port {
    frame: [u8; MAX_ETHERNET_FRAME_LEN],
    pending: usize,
    sent: usize,
}

impl EthercatPort for Port {
    type Error = ();
    fn link_state(&self) -> LinkState {
        LinkState::Up
    }
    fn now_ns(&self) -> u64 {
        1
    }
    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), ()> {
        // Only the explicit FPRD is allowed; no synthetic DC frame precedes it.
        assert_eq!(frame[16], 0x04);
        self.sent += 1;
        self.frame[..frame.len()].copy_from_slice(frame);
        let body = usize::from(u16::from_le_bytes([frame[14], frame[15]]) & 0x07ff);
        self.frame[26..28].copy_from_slice(&[0x34, 0x12]);
        self.frame[16 + body - 2..16 + body].copy_from_slice(&[1, 0]);
        self.pending = frame.len();
        Ok(())
    }
    fn rx_poll(&mut self, dst: &mut [u8; MAX_ETHERNET_FRAME_LEN]) -> Result<RxPoll, ()> {
        let len = core::mem::take(&mut self.pending);
        if len == 0 {
            return Ok(RxPoll::Empty);
        }
        dst[..len].copy_from_slice(&self.frame[..len]);
        Ok(RxPoll::Frame(len))
    }
}

#[test]
fn zero_working_counter_does_not_implicitly_disable_dc() {
    let mut config = DcCyclicConfig::new(0x1001, 7, 0);
    config.expected_wkc = 0;
    let mut dc = DcCyclicSync::new(config, DcMonitor::new(10, 10, 1, 1));
    assert!(dc.is_enabled());
    assert_eq!(
        dc.prepare(1, 1, &mut [0; 8]),
        Err(DcCyclicError::InvalidConfiguration)
    );
    let disabled = DcCyclicSync::disabled();
    assert!((0..=255).all(|index| !disabled.uses_datagram_index(index)));
    assert!(!disabled.is_enabled());
    assert_eq!(disabled.config().expected_wkc, 0);
}

#[test]
fn disabled_dc_sends_nothing_but_preserves_control_and_domain_validation() {
    let schedule = ScheduleTable::<1, 1>::build(
        1_000_000,
        &[ScheduleDomain {
            id: 0,
            period_ticks: 1,
            phase_ticks: 0,
        }],
    )
    .unwrap();
    let mut domain = Domain::<1, 1>::new(0x1000);
    domain
        .add_segment(DomainSegment {
            datagram_index: 0,
            input_offset: 0,
            len: 1,
            expected_wkc: 1,
        })
        .unwrap();
    let mut bank = ScheduledDomainBank::new(
        &schedule,
        [ScheduledDomainEntry {
            id: 0,
            domain: &mut domain,
        }],
    )
    .unwrap();
    let mut master =
        EthercatMaster::<2, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new([0xff; 6], [2; 6]));
    let mut controls = ControlRequestPool::<1>::new();
    let mut port = Port {
        frame: [0; MAX_ETHERNET_FRAME_LEN],
        pending: 0,
        sent: 0,
    };
    let mut dc = DcCyclicSync::disabled();
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let idle = bank
        .run_dc_and_control_cycle(
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut [],
            1,
            &mut controls,
            None,
            1,
            100,
            100,
        )
        .unwrap();
    assert!(!idle.service().dc_requested && !idle.service().dc_sent);
    assert!(bank.confirms_control_cycle(&idle));
    assert!(idle.received().dc_result.is_ok());
    assert!(!idle.received().qualities[0].valid);
    assert_eq!(port.sent, 0);
    let request = controls
        .acquire_with_response_len(5, 2, 0x0120_1001, RegisterOperation::Read, &[], 2, 100)
        .unwrap();
    let cycle = bank
        .run_dc_and_control_cycle(
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut [],
            1,
            &mut controls,
            Some(request),
            2,
            100,
            100,
        )
        .unwrap();
    assert!(bank.confirms_control_cycle(&cycle));
    assert!(
        !cycle.service().dc_requested && !cycle.service().dc_sent && cycle.service().control_sent
    );
    assert_eq!(port.sent, 1);
    assert_eq!(controls.get(request).unwrap().state, RequestState::Complete);
    assert_eq!(controls.get(request).unwrap().payload(), &[0x34, 0x12]);
    assert_eq!(dc.monitor().state(), DcLockState::Unknown);
    assert_eq!(dc.monitor().sample_count(), 0);
    assert_eq!(dc.sync_count(), 0);
    assert!(!dc.is_locked());
}
