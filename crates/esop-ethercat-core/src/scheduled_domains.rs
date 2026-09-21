//! Fixed-capacity receive ownership for a frozen multi-rate Domain schedule.

use crate::dc::{DcCyclicError, DcCyclicSync};
use crate::domain::{Domain, DomainError, DomainQuality, DomainSegment};
use crate::engine::{CycleError, CycleReport, EthercatMaster, RxConsumerMux, RxDatagramConsumer};
use crate::plan::{FramePlan, FramePlanSet};
use crate::port::{EthercatPort, LinkState};
use crate::rx_index::RxMatch;
use crate::schedule::ScheduleTable;
use crate::wire::{DatagramHeader, MAX_ETHERNET_FRAME_LEN};
use core::any::Any;

mod private {
    pub trait Sealed {}
}

/// The receive operations required from a configured Domain. Implemented by
/// `Domain` for any fixed image and segment capacity. External implementations
/// cannot bypass the activation-time segment and receive-state checks.
pub trait ScheduledDomainRx: private::Sealed + RxDatagramConsumer {
    fn as_any(&self) -> &dyn Any;
    fn logical_address(&self) -> u32;
    fn segments(&self) -> &[DomainSegment];
    fn receive_generation(&self) -> Option<u16>;
    fn begin_receive(&mut self, generation: u16) -> Result<(), DomainError>;
    fn finish_receive(&mut self, generation: u16, cycle: u64) -> Result<bool, DomainError>;
    fn quality(&self) -> DomainQuality;
}

impl<const BYTES: usize, const SEGMENTS: usize> private::Sealed for Domain<BYTES, SEGMENTS> {}

impl<const BYTES: usize, const SEGMENTS: usize> ScheduledDomainRx for Domain<BYTES, SEGMENTS> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn logical_address(&self) -> u32 {
        Domain::logical_address(self)
    }

    fn segments(&self) -> &[DomainSegment] {
        Domain::segments(self)
    }

    fn receive_generation(&self) -> Option<u16> {
        Domain::receive_generation(self)
    }

    fn begin_receive(&mut self, generation: u16) -> Result<(), DomainError> {
        Domain::begin_receive(self, generation)
    }

    fn finish_receive(&mut self, generation: u16, cycle: u64) -> Result<bool, DomainError> {
        Domain::finish_receive(self, generation, cycle)
    }

    fn quality(&self) -> DomainQuality {
        Domain::quality(self)
    }
}

pub struct ScheduledDomainEntry<'a> {
    pub id: u8,
    pub domain: &'a mut dyn ScheduledDomainRx,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledDomainError {
    InvalidSchedule,
    EmptyDomain(usize),
    DuplicateDatagramIndex(u8),
    CycleOrder,
    CycleInProgress,
    DomainBusy(usize),
    Domain(usize, DomainError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledReceiveError {
    DcIndexConflict(u8),
    DcGenerationMismatch,
    Domain(ScheduledDomainError),
}

/// Even an RX port error returns a cycle report tied to the real master
/// cycle. The conservative budget miss blocks motion while retaining the
/// precise transport error for diagnostics; Domain and DC pending state have
/// already been finalized so they cannot leak into the next generation.
#[derive(Debug)]
pub struct ScheduledReceiveReport<E, const DOMAINS: usize> {
    pub report: CycleReport,
    pub qualities: [DomainQuality; DOMAINS],
    pub dc_result: Result<(), DcCyclicError>,
    pub transport_error: Option<CycleError<E>>,
}

/// Owns the receive borrow for all configured Domains. A single master RX
/// session dispatches only verified datagrams to Domains due on this tick.
/// Use `receive_with_dc` when DC is enabled so a port error cannot skip
/// Domain or DC receive finalization. For other receive arrangements the
/// caller must pair `begin_due` and `finish_due`, including on RX errors.
/// The caller still owns verified TX plans, control RX, and the final cycle
/// deadline. Pass the resulting qualities to the lifecycle schedule
/// projection in the same order.
pub struct ScheduledDomainBank<'a, const DOMAINS: usize, const SLOTS: usize> {
    schedule: &'a ScheduleTable<DOMAINS, SLOTS>,
    domains: [ScheduledDomainEntry<'a>; DOMAINS],
    index_owner: [u8; 256],
    active: Option<(u64, u16, u64)>,
    last_cycle: u64,
}

impl<'a, const DOMAINS: usize, const SLOTS: usize> ScheduledDomainBank<'a, DOMAINS, SLOTS> {
    /// One bounded master RX step for every due Domain and the same-generation
    /// DC response. Requires `dc.prepare` and arming its frozen FRMW plan
    /// before this call. No response, link loss, or port error may skip either
    /// receive finalizer. The caller still owns TX and lifecycle publication.
    pub fn receive_with_dc<P: EthercatPort, const FRAMES: usize, const MTU: usize>(
        &mut self,
        master: &mut EthercatMaster<FRAMES, MTU>,
        port: &mut P,
        scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN],
        generation: u16,
        dc: &mut DcCyclicSync,
    ) -> Result<ScheduledReceiveReport<P::Error, DOMAINS>, ScheduledReceiveError> {
        let dc_index = dc.datagram_plan().index;
        if self.index_owner[dc_index as usize] != 0 {
            return Err(ScheduledReceiveError::DcIndexConflict(dc_index));
        }
        if dc.pending_generation() != Some(generation) {
            return Err(ScheduledReceiveError::DcGenerationMismatch);
        }
        let cycle = master.cycle_number().wrapping_add(1);
        self.begin_due(cycle, generation)
            .map_err(ScheduledReceiveError::Domain)?;
        let rx = {
            let mut consumers = RxConsumerMux::new(&mut *self, &mut *dc);
            master.cycle_receive_with_consumer(port, scratch, generation, &mut consumers)
        };
        let (report, transport_error) = match rx {
            Ok(report) => (report, None),
            Err(error) => {
                let mut report = CycleReport::new(master.cycle_number());
                report.budget_exhausted = true;
                report.link_down = port.link_state() == LinkState::Down;
                (report, Some(error))
            }
        };
        let dc_result = dc.finish_receive(report.cycle, generation);
        let qualities = self
            .finish_due(report.cycle, generation)
            .map_err(ScheduledReceiveError::Domain)?;
        Ok(ScheduledReceiveReport {
            report,
            qualities,
            dc_result,
            transport_error,
        })
    }

    pub fn uses_schedule(&self, schedule: &ScheduleTable<DOMAINS, SLOTS>) -> bool {
        core::ptr::eq(self.schedule, schedule)
    }

    /// Validate the ID order and exclusive datagram-index ownership once at
    /// activation, not on the cyclic RX path.
    pub fn new(
        schedule: &'a ScheduleTable<DOMAINS, SLOTS>,
        domains: [ScheduledDomainEntry<'a>; DOMAINS],
    ) -> Result<Self, ScheduledDomainError> {
        if DOMAINS == 0
            || DOMAINS != schedule.domain_count()
            || domains
                .iter()
                .zip(schedule.domains())
                .any(|(domain, configured)| domain.id != configured.id)
        {
            return Err(ScheduledDomainError::InvalidSchedule);
        }
        let mut index_owner = [0u8; 256];
        for (slot, entry) in domains.iter().enumerate() {
            if entry.domain.segments().is_empty() {
                return Err(ScheduledDomainError::EmptyDomain(slot));
            }
            if entry.domain.receive_generation().is_some() {
                return Err(ScheduledDomainError::DomainBusy(slot));
            }
            for segment in entry.domain.segments() {
                let owner = &mut index_owner[segment.datagram_index as usize];
                if *owner != 0 {
                    return Err(ScheduledDomainError::DuplicateDatagramIndex(
                        segment.datagram_index,
                    ));
                }
                *owner = slot as u8 + 1;
            }
        }
        Ok(Self {
            schedule,
            domains,
            index_owner,
            active: None,
            last_cycle: 0,
        })
    }

    /// Read the actual committed image of a bound Domain after `finish_due`.
    /// The caller must supply its configured image and segment capacities;
    /// mismatched IDs or shapes cannot be cast into a motion Domain.
    pub fn domain<const BYTES: usize, const SEGMENTS: usize>(
        &self,
        id: u8,
    ) -> Option<&Domain<BYTES, SEGMENTS>> {
        if self.active.is_some() {
            return None;
        }
        self.domains
            .iter()
            .find(|entry| entry.id == id)?
            .domain
            .as_any()
            .downcast_ref()
    }

    /// Bind every TX datagram to exactly one actual RX segment of this Domain.
    /// A split frame plan may rearrange frame boundaries but not addresses,
    /// indices, offsets, lengths, or WKC ownership.
    pub fn matches_frame_plans<const FRAMES: usize, const DATAGRAMS: usize>(
        &self,
        id: u8,
        plans: &FramePlanSet<FRAMES, DATAGRAMS>,
    ) -> bool {
        let Some(entry) = self.domains.iter().find(|entry| entry.id == id) else {
            return false;
        };
        let segments = entry.domain.segments();
        plans.datagram_count() == segments.len()
            && segments.iter().all(|segment| {
                plans
                    .plans()
                    .iter()
                    .flat_map(|plan| plan.datagrams())
                    .any(|datagram| {
                        datagram.index == segment.datagram_index
                            && datagram.payload_offset == segment.input_offset
                            && datagram.payload_len == segment.len
                            && datagram.expected_wkc == segment.expected_wkc
                            && u32::try_from(segment.input_offset).ok().and_then(|offset| {
                                entry.domain.logical_address().checked_add(offset)
                            }) == Some(datagram.address)
                    })
            })
    }

    pub fn matches_frame_plan<const DATAGRAMS: usize>(
        &self,
        id: u8,
        plan: &FramePlan<DATAGRAMS>,
    ) -> bool {
        let Some(entry) = self.domains.iter().find(|entry| entry.id == id) else {
            return false;
        };
        let segments = entry.domain.segments();
        plan.len() == segments.len()
            && segments.iter().all(|segment| {
                plan.datagrams().iter().any(|datagram| {
                    datagram.index == segment.datagram_index
                        && datagram.payload_offset == segment.input_offset
                        && datagram.payload_len == segment.len
                        && datagram.expected_wkc == segment.expected_wkc
                        && u32::try_from(segment.input_offset)
                            .ok()
                            .and_then(|offset| entry.domain.logical_address().checked_add(offset))
                            == Some(datagram.address)
                })
            })
    }

    pub fn begin_due(&mut self, cycle: u64, generation: u16) -> Result<(), ScheduledDomainError> {
        if self.active.is_some() {
            return Err(ScheduledDomainError::CycleInProgress);
        }
        if cycle == 0 || cycle != self.last_cycle.saturating_add(1) {
            return Err(ScheduledDomainError::CycleOrder);
        }
        let tick = (cycle - 1) % u64::from(self.schedule.hyperperiod_ticks());
        let due_mask = self.schedule.due_mask(tick as u32);
        for (slot, entry) in self.domains.iter().enumerate() {
            if due_mask & (1u64 << entry.id) != 0 && entry.domain.receive_generation().is_some() {
                return Err(ScheduledDomainError::DomainBusy(slot));
            }
        }
        for (slot, entry) in self.domains.iter_mut().enumerate() {
            if due_mask & (1u64 << entry.id) != 0 {
                entry
                    .domain
                    .begin_receive(generation)
                    .map_err(|error| ScheduledDomainError::Domain(slot, error))?;
            }
        }
        self.active = Some((cycle, generation, due_mask));
        Ok(())
    }

    /// Even a missing due response is finished, invalidating that Domain's
    /// quality rather than carrying its previous committed image as current.
    pub fn finish_due(
        &mut self,
        cycle: u64,
        generation: u16,
    ) -> Result<[DomainQuality; DOMAINS], ScheduledDomainError> {
        let Some((active_cycle, active_generation, due_mask)) = self.active else {
            return Err(ScheduledDomainError::CycleOrder);
        };
        if cycle != active_cycle || generation != active_generation {
            return Err(ScheduledDomainError::CycleOrder);
        }
        self.active = None;
        for (slot, entry) in self.domains.iter_mut().enumerate() {
            if due_mask & (1u64 << entry.id) != 0 {
                entry
                    .domain
                    .finish_receive(generation, cycle)
                    .map_err(|error| ScheduledDomainError::Domain(slot, error))?;
            }
        }
        self.last_cycle = cycle;
        Ok(core::array::from_fn(|slot| {
            self.domains[slot].domain.quality()
        }))
    }
}

impl<const DOMAINS: usize, const SLOTS: usize> RxDatagramConsumer
    for ScheduledDomainBank<'_, DOMAINS, SLOTS>
{
    fn accept(
        &mut self,
        cycle: u64,
        received_at_ns: u64,
        completion: RxMatch,
        header: DatagramHeader,
        payload: &[u8],
    ) -> bool {
        let Some((active_cycle, generation, due_mask)) = self.active else {
            return false;
        };
        if cycle != active_cycle || completion.generation != generation {
            return false;
        }
        let owner = self.index_owner[header.index as usize];
        if owner == 0 {
            return false;
        }
        let entry = &mut self.domains[(owner - 1) as usize];
        due_mask & (1u64 << entry.id) != 0
            && entry
                .domain
                .accept(cycle, received_at_ns, completion, header, payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{DatagramPlan, FramePlanSet};
    use crate::schedule::ScheduleDomain;
    use crate::wire::Command;

    fn domain<const BYTES: usize>(index: u8) -> Domain<BYTES, 1> {
        let mut domain = Domain::new(0x1000 + u32::from(index));
        domain
            .add_segment(DomainSegment {
                datagram_index: index,
                input_offset: 0,
                len: BYTES,
                expected_wkc: 1,
            })
            .unwrap();
        domain
    }

    #[test]
    fn schedule_binding_rejects_wrong_order_duplicate_indices_and_empty_domains() {
        let schedule = ScheduleTable::<2, 2>::build(
            100_000,
            &[
                ScheduleDomain {
                    id: 9,
                    period_ticks: 1,
                    phase_ticks: 0,
                },
                ScheduleDomain {
                    id: 10,
                    period_ticks: 2,
                    phase_ticks: 0,
                },
            ],
        )
        .unwrap();
        let mut first = domain::<2>(12);
        let mut second = domain::<1>(12);
        assert!(matches!(
            ScheduledDomainBank::new(
                &schedule,
                [
                    ScheduledDomainEntry {
                        id: 10,
                        domain: &mut first,
                    },
                    ScheduledDomainEntry {
                        id: 9,
                        domain: &mut second,
                    },
                ],
            ),
            Err(ScheduledDomainError::InvalidSchedule)
        ));
        assert!(matches!(
            ScheduledDomainBank::new(
                &schedule,
                [
                    ScheduledDomainEntry {
                        id: 9,
                        domain: &mut first,
                    },
                    ScheduledDomainEntry {
                        id: 10,
                        domain: &mut second,
                    },
                ],
            ),
            Err(ScheduledDomainError::DuplicateDatagramIndex(12))
        ));
        let mut empty = Domain::<2, 1>::new(0x2000);
        assert!(matches!(
            ScheduledDomainBank::new(
                &schedule,
                [
                    ScheduledDomainEntry {
                        id: 9,
                        domain: &mut first,
                    },
                    ScheduledDomainEntry {
                        id: 10,
                        domain: &mut empty,
                    },
                ],
            ),
            Err(ScheduledDomainError::EmptyDomain(1))
        ));
    }

    #[test]
    fn non_due_and_wrong_generation_datagrams_cannot_change_domain_quality() {
        let schedule = ScheduleTable::<2, 2>::build(
            100_000,
            &[
                ScheduleDomain {
                    id: 9,
                    period_ticks: 1,
                    phase_ticks: 0,
                },
                ScheduleDomain {
                    id: 10,
                    period_ticks: 2,
                    phase_ticks: 0,
                },
            ],
        )
        .unwrap();
        let mut first = domain::<2>(12);
        let mut second = domain::<1>(13);
        let mut bank = ScheduledDomainBank::new(
            &schedule,
            [
                ScheduledDomainEntry {
                    id: 9,
                    domain: &mut first,
                },
                ScheduledDomainEntry {
                    id: 10,
                    domain: &mut second,
                },
            ],
        )
        .unwrap();
        let completion = |generation| RxMatch {
            slot_id: 0,
            generation,
            working_counter: 1,
        };
        let header = |index| DatagramHeader {
            command: Command::Lrw,
            index,
            address: 0x1000,
            length: 1,
            last: true,
        };
        assert_eq!(bank.begin_due(2, 1), Err(ScheduledDomainError::CycleOrder));
        bank.begin_due(1, 1).unwrap();
        assert_eq!(
            bank.begin_due(1, 1),
            Err(ScheduledDomainError::CycleInProgress)
        );
        assert!(!bank.accept(1, 100, completion(2), header(13), &[9]));
        assert!(bank.accept(1, 100, completion(1), header(13), &[8]));
        assert!(bank.accept(1, 100, completion(1), header(12), &[1, 2]));
        assert_eq!(bank.finish_due(1, 2), Err(ScheduledDomainError::CycleOrder));
        let qualities = bank.finish_due(1, 1).unwrap();
        assert!(qualities.iter().all(|quality| quality.valid));
        bank.begin_due(2, 2).unwrap();
        assert!(!bank.accept(2, 200, completion(2), header(13), &[3]));
        assert!(bank.accept(2, 200, completion(2), header(12), &[4, 5]));
        let qualities = bank.finish_due(2, 2).unwrap();
        assert_eq!(qualities[1].last_valid_cycle, 1);
        assert_eq!(qualities[1].actual_wkc, 1);
        bank.begin_due(3, 3).unwrap();
        assert!(bank.accept(3, 300, completion(3), header(12), &[6, 7]));
        let qualities = bank.finish_due(3, 3).unwrap();
        assert!(!qualities[1].valid);
        assert!(!qualities[1].complete);
        assert_eq!(qualities[1].last_valid_cycle, 1);
    }

    #[test]
    fn split_output_plans_must_match_every_bound_domain_segment() {
        let schedule = ScheduleTable::<2, 2>::build(
            100_000,
            &[
                ScheduleDomain {
                    id: 9,
                    period_ticks: 1,
                    phase_ticks: 0,
                },
                ScheduleDomain {
                    id: 10,
                    period_ticks: 2,
                    phase_ticks: 0,
                },
            ],
        )
        .unwrap();
        let mut motion = domain::<2>(12);
        let mut auxiliary = Domain::<2, 2>::new(0x2000);
        for (index, offset) in [(13, 0), (14, 1)] {
            auxiliary
                .add_segment(DomainSegment {
                    datagram_index: index,
                    input_offset: offset,
                    len: 1,
                    expected_wkc: 1,
                })
                .unwrap();
        }
        let bank = ScheduledDomainBank::new(
            &schedule,
            [
                ScheduledDomainEntry {
                    id: 9,
                    domain: &mut motion,
                },
                ScheduledDomainEntry {
                    id: 10,
                    domain: &mut auxiliary,
                },
            ],
        )
        .unwrap();
        let mut plans = FramePlanSet::<2, 1>::new();
        for (index, offset) in [(13, 0), (14, 1)] {
            plans
                .push(DatagramPlan {
                    command: Command::Lrw,
                    index,
                    address: 0x2000 + offset as u32,
                    payload_offset: offset,
                    payload_len: 1,
                    expected_wkc: 1,
                })
                .unwrap();
        }
        assert_eq!(plans.frame_count(), 2);
        assert!(bank.matches_frame_plans(10, &plans));
        assert!(!bank.matches_frame_plans(9, &plans));
        let mut wrong = FramePlanSet::<2, 1>::new();
        wrong
            .push(DatagramPlan {
                address: 0x2002,
                ..plans.plan(0).unwrap().datagrams()[0]
            })
            .unwrap();
        wrong.push(plans.plan(1).unwrap().datagrams()[0]).unwrap();
        assert!(!bank.matches_frame_plans(10, &wrong));
    }
}
