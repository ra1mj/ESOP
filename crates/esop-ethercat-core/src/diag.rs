//! Fixed-size binary diagnostics for realtime producers and non-realtime
//! consumers. Formatting and transport are intentionally outside the core.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::coe::CoeEmergency;
use crate::ring::{SpscConsumer, SpscRing};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum EventCode {
    LinkDown = 1,
    FrameCorrupt = 2,
    RxUnmatched = 3,
    WorkingCounterMismatch = 4,
    ConsumerRejected = 5,
    RxBudgetExhausted = 6,
    RxTimeout = 7,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EventSeverity {
    Info = 1,
    Warning = 2,
    Error = 3,
    Fault = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct EventRecord {
    pub cycle: u64,
    pub timestamp_ns: u64,
    pub code: EventCode,
    pub severity: EventSeverity,
    pub index: u8,
    pub reserved: u8,
    pub value: u32,
    pub aux: u32,
}

impl EventRecord {
    pub const fn new(
        cycle: u64,
        timestamp_ns: u64,
        code: EventCode,
        severity: EventSeverity,
        index: u8,
        value: u32,
        aux: u32,
    ) -> Self {
        Self {
            cycle,
            timestamp_ns,
            code,
            severity,
            index,
            reserved: 0,
            value,
            aux,
        }
    }
}

/// Fixed diagnostic event queue. There is one producer (the realtime
/// master) and one consumer (a supervisor, recorder or eBPF bridge).
pub struct Diagnostics<const EVENTS: usize = 64> {
    events: SpscRing<EventRecord, EVENTS>,
    lost_events: AtomicUsize,
}

impl<const EVENTS: usize> Diagnostics<EVENTS> {
    pub const fn new() -> Self {
        Self {
            events: SpscRing::new(),
            lost_events: AtomicUsize::new(0),
        }
    }

    pub fn record(&self, event: EventRecord) -> bool {
        let (producer, _) = self.events.split();
        if producer.push(event).is_ok() {
            true
        } else {
            self.lost_events.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    /// Pop one event from the single consumer side. Callers must preserve the
    /// one-consumer rule when this is used from multiple non-realtime tasks.
    pub fn pop(&self) -> Option<EventRecord> {
        let (_, consumer) = self.events.split();
        consumer.pop()
    }

    pub fn pending(&self) -> usize {
        self.events.len()
    }

    pub fn lost_events(&self) -> usize {
        self.lost_events.load(Ordering::Acquire)
    }
}

impl<const EVENTS: usize> Default for Diagnostics<EVENTS> {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for APIs that need to name a diagnostic consumer explicitly.
pub type DiagnosticConsumer<'a, const EVENTS: usize = 64> = SpscConsumer<'a, EventRecord, EVENTS>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CoeEmergencyEvent {
    pub timestamp_ns: u64,
    pub station_address: u16,
    pub generation: u16,
    pub counter: u8,
    pub reserved: [u8; 3],
    pub error_code: u16,
    pub error_register: u8,
    pub manufacturer_data: [u8; 5],
}

impl CoeEmergencyEvent {
    pub const fn new(
        timestamp_ns: u64,
        station_address: u16,
        generation: u16,
        counter: u8,
        emergency: CoeEmergency,
    ) -> Self {
        Self {
            timestamp_ns,
            station_address,
            generation,
            counter,
            reserved: [0; 3],
            error_code: emergency.error_code,
            error_register: emergency.error_register,
            manufacturer_data: emergency.manufacturer_data,
        }
    }
}

/// Non-blocking destination for asynchronous CoE Emergency notifications.
pub trait EmergencySink {
    fn record(&self, event: CoeEmergencyEvent) -> bool;
}

/// Fixed-size Emergency queue. Overflow is observable through `lost_events`
/// and never blocks the mailbox state machine.
pub struct CoeEmergencyQueue<const EVENTS: usize = 64> {
    events: SpscRing<CoeEmergencyEvent, EVENTS>,
    lost_events: AtomicUsize,
}

impl<const EVENTS: usize> CoeEmergencyQueue<EVENTS> {
    pub const fn new() -> Self {
        Self {
            events: SpscRing::new(),
            lost_events: AtomicUsize::new(0),
        }
    }

    pub fn record(&self, event: CoeEmergencyEvent) -> bool {
        let (producer, _) = self.events.split();
        if producer.push(event).is_ok() {
            true
        } else {
            self.lost_events.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    pub fn pop(&self) -> Option<CoeEmergencyEvent> {
        let (_, consumer) = self.events.split();
        consumer.pop()
    }

    pub fn pending(&self) -> usize {
        self.events.len()
    }

    pub fn lost_events(&self) -> usize {
        self.lost_events.load(Ordering::Acquire)
    }
}

impl<const EVENTS: usize> EmergencySink for CoeEmergencyQueue<EVENTS> {
    fn record(&self, event: CoeEmergencyEvent) -> bool {
        CoeEmergencyQueue::record(self, event)
    }
}

impl<const EVENTS: usize> Default for CoeEmergencyQueue<EVENTS> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_event_queue_preserves_events_and_counts_overflow() {
        let diagnostics = Diagnostics::<2>::new();
        assert!(diagnostics.record(EventRecord::new(
            1,
            10,
            EventCode::WorkingCounterMismatch,
            EventSeverity::Error,
            7,
            2,
            1,
        )));
        assert!(diagnostics.record(EventRecord::new(
            1,
            11,
            EventCode::RxUnmatched,
            EventSeverity::Warning,
            8,
            0,
            0,
        )));
        assert!(!diagnostics.record(EventRecord::new(
            1,
            12,
            EventCode::FrameCorrupt,
            EventSeverity::Fault,
            0,
            0,
            0,
        )));
        assert_eq!(diagnostics.pending(), 2);
        assert_eq!(diagnostics.lost_events(), 1);
        assert_eq!(
            diagnostics.pop().unwrap().code,
            EventCode::WorkingCounterMismatch
        );
        assert_eq!(diagnostics.pop().unwrap().code, EventCode::RxUnmatched);
        assert!(diagnostics.pop().is_none());
    }

    #[test]
    fn emergency_queue_is_fixed_and_non_blocking_when_full() {
        let queue = CoeEmergencyQueue::<1>::new();
        let event = CoeEmergencyEvent::new(
            10,
            7,
            9,
            3,
            CoeEmergency {
                error_code: 0x2310,
                error_register: 0x81,
                manufacturer_data: [1, 2, 3, 4, 5],
            },
        );
        assert!(queue.record(event));
        assert!(!queue.record(event));
        assert_eq!(queue.lost_events(), 1);
        assert_eq!(queue.pop(), Some(event));
        assert!(queue.pop().is_none());
    }
}
