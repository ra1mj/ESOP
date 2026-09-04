//! Activation-time multi-rate schedule generation.
//!
//! The runtime only indexes a precomputed due-mask. Period validation,
//! hyperperiod calculation and phase expansion happen before activation.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduleDomain {
    pub id: u8,
    pub period_ticks: u32,
    pub phase_ticks: u32,
}

impl ScheduleDomain {
    pub const EMPTY: Self = Self {
        id: 0,
        period_ticks: 0,
        phase_ticks: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduleSlot {
    pub due_mask: u64,
}

impl ScheduleSlot {
    pub const EMPTY: Self = Self { due_mask: 0 };

    pub const fn is_due(self, domain_id: u8) -> bool {
        domain_id < 64 && self.due_mask & (1u64 << domain_id) != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleError {
    InvalidBaseTick,
    CapacityExceeded,
    TooManyDomains,
    DuplicateDomainId,
    InvalidPeriod,
    InvalidPhase,
    HyperperiodOverflow,
    HyperperiodTooLarge,
}

/// A fixed-size, activation-time generated multi-rate schedule.
pub struct ScheduleTable<const DOMAINS: usize, const SLOTS: usize> {
    base_tick_ns: u64,
    hyperperiod_ticks: u32,
    domains: [ScheduleDomain; DOMAINS],
    domain_count: usize,
    slots: [ScheduleSlot; SLOTS],
}

impl<const DOMAINS: usize, const SLOTS: usize> ScheduleTable<DOMAINS, SLOTS> {
    pub fn build(base_tick_ns: u64, domains: &[ScheduleDomain]) -> Result<Self, ScheduleError> {
        if base_tick_ns == 0 {
            return Err(ScheduleError::InvalidBaseTick);
        }
        if domains.len() > DOMAINS {
            return Err(ScheduleError::CapacityExceeded);
        }
        if domains.len() > 64 {
            return Err(ScheduleError::TooManyDomains);
        }
        if SLOTS == 0 {
            return Err(ScheduleError::HyperperiodTooLarge);
        }

        let mut hyperperiod = 1u32;
        let mut copied = [ScheduleDomain::EMPTY; DOMAINS];
        for (index, domain) in domains.iter().copied().enumerate() {
            if domain.period_ticks == 0 {
                return Err(ScheduleError::InvalidPeriod);
            }
            if domain.phase_ticks >= domain.period_ticks {
                return Err(ScheduleError::InvalidPhase);
            }
            if copied[..index]
                .iter()
                .any(|existing| existing.id == domain.id)
            {
                return Err(ScheduleError::DuplicateDomainId);
            }
            if domain.id >= 64 {
                return Err(ScheduleError::TooManyDomains);
            }
            hyperperiod =
                lcm(hyperperiod, domain.period_ticks).ok_or(ScheduleError::HyperperiodOverflow)?;
            copied[index] = domain;
        }
        if hyperperiod as usize > SLOTS {
            return Err(ScheduleError::HyperperiodTooLarge);
        }

        let mut table = Self {
            base_tick_ns,
            hyperperiod_ticks: hyperperiod,
            domains: copied,
            domain_count: domains.len(),
            slots: [ScheduleSlot::EMPTY; SLOTS],
        };
        for slot in 0..hyperperiod as usize {
            let mut due_mask = 0u64;
            for domain in table.domains[..table.domain_count].iter() {
                if (slot as u32 + domain.period_ticks - domain.phase_ticks) % domain.period_ticks
                    == 0
                {
                    due_mask |= 1u64 << domain.id;
                }
            }
            table.slots[slot] = ScheduleSlot { due_mask };
        }
        Ok(table)
    }

    pub const fn base_tick_ns(&self) -> u64 {
        self.base_tick_ns
    }

    pub const fn hyperperiod_ticks(&self) -> u32 {
        self.hyperperiod_ticks
    }

    pub const fn domain_count(&self) -> usize {
        self.domain_count
    }

    pub fn domains(&self) -> &[ScheduleDomain] {
        &self.domains[..self.domain_count]
    }

    pub fn slot(&self, tick: u32) -> ScheduleSlot {
        self.slots[(tick % self.hyperperiod_ticks) as usize]
    }

    pub fn due_mask(&self, tick: u32) -> u64 {
        self.slot(tick).due_mask
    }
}

fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn lcm(left: u32, right: u32) -> Option<u32> {
    left.checked_div(gcd(left, right))?.checked_mul(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_expands_periods_and_phases_into_due_masks() {
        let table = ScheduleTable::<3, 16>::build(
            250_000,
            &[
                ScheduleDomain {
                    id: 0,
                    period_ticks: 1,
                    phase_ticks: 0,
                },
                ScheduleDomain {
                    id: 1,
                    period_ticks: 4,
                    phase_ticks: 0,
                },
                ScheduleDomain {
                    id: 2,
                    period_ticks: 4,
                    phase_ticks: 2,
                },
            ],
        )
        .unwrap();

        assert_eq!(table.base_tick_ns(), 250_000);
        assert_eq!(table.hyperperiod_ticks(), 4);
        assert_eq!(table.due_mask(0), 0b011);
        assert_eq!(table.due_mask(1), 0b001);
        assert_eq!(table.due_mask(2), 0b101);
        assert_eq!(table.due_mask(4), table.due_mask(0));
        assert!(table.slot(6).is_due(2));
    }

    #[test]
    fn schedule_rejects_invalid_activation_inputs() {
        assert!(matches!(
            ScheduleTable::<1, 4>::build(
                0,
                &[ScheduleDomain {
                    id: 0,
                    period_ticks: 1,
                    phase_ticks: 0,
                }],
            ),
            Err(ScheduleError::InvalidBaseTick)
        ));
        assert!(matches!(
            ScheduleTable::<2, 4>::build(
                1,
                &[ScheduleDomain {
                    id: 1,
                    period_ticks: 2,
                    phase_ticks: 2,
                }],
            ),
            Err(ScheduleError::InvalidPhase)
        ));
        assert!(matches!(
            ScheduleTable::<2, 4>::build(
                1,
                &[
                    ScheduleDomain {
                        id: 1,
                        period_ticks: 2,
                        phase_ticks: 0,
                    },
                    ScheduleDomain {
                        id: 1,
                        period_ticks: 4,
                        phase_ticks: 0,
                    },
                ],
            ),
            Err(ScheduleError::DuplicateDomainId)
        ));
    }

    #[test]
    fn schedule_rejects_hyperperiod_larger_than_static_table() {
        assert!(matches!(
            ScheduleTable::<2, 8>::build(
                1,
                &[
                    ScheduleDomain {
                        id: 0,
                        period_ticks: 3,
                        phase_ticks: 0,
                    },
                    ScheduleDomain {
                        id: 1,
                        period_ticks: 4,
                        phase_ticks: 0,
                    },
                ],
            ),
            Err(ScheduleError::HyperperiodTooLarge)
        ));
    }
}
