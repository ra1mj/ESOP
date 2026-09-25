use crate::IpcHeader;
use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerPolicy {
    robot_id: u64,
    layout_hash: u64,
    source_id: u64,
    offline_timeout_ns: u64,
    max_message_age_ns: u64,
}

impl PeerPolicy {
    pub fn new(
        robot_id: u64,
        layout_hash: u64,
        source_id: u64,
        offline_timeout_ns: u64,
        max_message_age_ns: u64,
    ) -> Result<Self, PeerError> {
        if robot_id == 0 {
            return Err(PeerError::InvalidPolicy(PolicyField::RobotId));
        }
        if layout_hash == 0 {
            return Err(PeerError::InvalidPolicy(PolicyField::LayoutHash));
        }
        if source_id == 0 {
            return Err(PeerError::InvalidPolicy(PolicyField::SourceId));
        }
        if offline_timeout_ns == 0 {
            return Err(PeerError::InvalidPolicy(PolicyField::OfflineTimeout));
        }
        if max_message_age_ns == 0 {
            return Err(PeerError::InvalidPolicy(PolicyField::MaxMessageAge));
        }
        Ok(Self {
            robot_id,
            layout_hash,
            source_id,
            offline_timeout_ns,
            max_message_age_ns,
        })
    }

    pub const fn robot_id(&self) -> u64 {
        self.robot_id
    }

    pub const fn layout_hash(&self) -> u64 {
        self.layout_hash
    }

    pub const fn source_id(&self) -> u64 {
        self.source_id
    }

    pub const fn offline_timeout_ns(&self) -> u64 {
        self.offline_timeout_ns
    }

    pub const fn max_message_age_ns(&self) -> u64 {
        self.max_message_age_ns
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyField {
    RobotId,
    LayoutHash,
    SourceId,
    OfflineTimeout,
    MaxMessageAge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerObservation {
    FirstContact,
    Continued,
    Restarted {
        previous_boot_id: u64,
        new_boot_id: u64,
    },
    Reconnected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerStatus {
    NeverSeen,
    Online,
    Offline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerError {
    InvalidPolicy(PolicyField),
    RobotMismatch { expected: u64, actual: u64 },
    LayoutMismatch { expected: u64, actual: u64 },
    SourceMismatch { expected: u64, actual: u64 },
    LocalTimeRegressed { previous: u64, actual: u64 },
    FutureTimestamp { remote: u64, received_at: u64 },
    MessageStale { age_ns: u64, maximum_ns: u64 },
    SequenceReplayed { previous: u64, actual: u64 },
    RemoteTimeRegressed { previous: u64, actual: u64 },
}

impl fmt::Display for PeerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid ESOP IPC peer observation: {self:?}")
    }
}

impl std::error::Error for PeerError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PeerState {
    boot_id: u64,
    sequence: u64,
    remote_time_ns: u64,
    received_at_ns: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerMonitor {
    policy: PeerPolicy,
    last: Option<PeerState>,
}

impl PeerMonitor {
    pub const fn new(policy: PeerPolicy) -> Self {
        Self { policy, last: None }
    }

    pub const fn policy(&self) -> &PeerPolicy {
        &self.policy
    }

    pub fn status(&self, now_ns: u64) -> Result<PeerStatus, PeerError> {
        let Some(last) = self.last else {
            return Ok(PeerStatus::NeverSeen);
        };
        if now_ns < last.received_at_ns {
            return Err(PeerError::LocalTimeRegressed {
                previous: last.received_at_ns,
                actual: now_ns,
            });
        }
        if now_ns - last.received_at_ns > self.policy.offline_timeout_ns {
            Ok(PeerStatus::Offline)
        } else {
            Ok(PeerStatus::Online)
        }
    }

    pub fn observe(
        &mut self,
        header: &IpcHeader,
        received_at_ns: u64,
    ) -> Result<PeerObservation, PeerError> {
        self.validate_identity(header)?;
        if header.monotonic_time_ns > received_at_ns {
            return Err(PeerError::FutureTimestamp {
                remote: header.monotonic_time_ns,
                received_at: received_at_ns,
            });
        }
        let age_ns = received_at_ns - header.monotonic_time_ns;
        if age_ns > self.policy.max_message_age_ns {
            return Err(PeerError::MessageStale {
                age_ns,
                maximum_ns: self.policy.max_message_age_ns,
            });
        }

        let observation = match self.last {
            None => PeerObservation::FirstContact,
            Some(last) => {
                if received_at_ns < last.received_at_ns {
                    return Err(PeerError::LocalTimeRegressed {
                        previous: last.received_at_ns,
                        actual: received_at_ns,
                    });
                }
                if header.boot_id != last.boot_id {
                    PeerObservation::Restarted {
                        previous_boot_id: last.boot_id,
                        new_boot_id: header.boot_id,
                    }
                } else {
                    if header.sequence <= last.sequence {
                        return Err(PeerError::SequenceReplayed {
                            previous: last.sequence,
                            actual: header.sequence,
                        });
                    }
                    if header.monotonic_time_ns < last.remote_time_ns {
                        return Err(PeerError::RemoteTimeRegressed {
                            previous: last.remote_time_ns,
                            actual: header.monotonic_time_ns,
                        });
                    }
                    if received_at_ns - last.received_at_ns > self.policy.offline_timeout_ns {
                        PeerObservation::Reconnected
                    } else {
                        PeerObservation::Continued
                    }
                }
            }
        };

        self.last = Some(PeerState {
            boot_id: header.boot_id,
            sequence: header.sequence,
            remote_time_ns: header.monotonic_time_ns,
            received_at_ns,
        });
        Ok(observation)
    }

    fn validate_identity(&self, header: &IpcHeader) -> Result<(), PeerError> {
        if header.robot_id != self.policy.robot_id {
            return Err(PeerError::RobotMismatch {
                expected: self.policy.robot_id,
                actual: header.robot_id,
            });
        }
        if header.layout_hash != self.policy.layout_hash {
            return Err(PeerError::LayoutMismatch {
                expected: self.policy.layout_hash,
                actual: header.layout_hash,
            });
        }
        if header.source_id != self.policy.source_id {
            return Err(PeerError::SourceMismatch {
                expected: self.policy.source_id,
                actual: header.source_id,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MessageKind;

    fn policy() -> PeerPolicy {
        PeerPolicy::new(11, 13, 17, 100, 50).unwrap()
    }

    fn header(boot_id: u64, sequence: u64, monotonic_time_ns: u64) -> IpcHeader {
        IpcHeader::new(
            MessageKind::Heartbeat,
            1,
            13,
            11,
            boot_id,
            17,
            sequence,
            monotonic_time_ns,
            0,
        )
    }

    #[test]
    fn tracks_first_continued_offline_and_reconnected() {
        let mut monitor = PeerMonitor::new(policy());
        assert_eq!(monitor.status(0), Ok(PeerStatus::NeverSeen));
        assert_eq!(
            monitor.observe(&header(19, 1, 100), 100),
            Ok(PeerObservation::FirstContact)
        );
        assert_eq!(monitor.status(200), Ok(PeerStatus::Online));
        assert_eq!(monitor.status(201), Ok(PeerStatus::Offline));
        assert_eq!(
            monitor.observe(&header(19, 2, 210), 210),
            Ok(PeerObservation::Reconnected)
        );
        assert_eq!(
            monitor.observe(&header(19, 3, 220), 220),
            Ok(PeerObservation::Continued)
        );
    }

    #[test]
    fn boot_change_resets_sequence_and_time_floors() {
        let mut monitor = PeerMonitor::new(policy());
        monitor.observe(&header(19, 20, 100), 100).unwrap();
        assert_eq!(
            monitor.observe(&header(23, 1, 90), 110),
            Ok(PeerObservation::Restarted {
                previous_boot_id: 19,
                new_boot_id: 23,
            })
        );
    }

    #[test]
    fn rejected_observations_do_not_mutate_state() {
        let mut monitor = PeerMonitor::new(policy());
        monitor.observe(&header(19, 3, 100), 100).unwrap();

        assert_eq!(
            monitor.observe(&header(19, 3, 101), 101),
            Err(PeerError::SequenceReplayed {
                previous: 3,
                actual: 3,
            })
        );
        assert_eq!(
            monitor.observe(&header(19, 4, 99), 102),
            Err(PeerError::RemoteTimeRegressed {
                previous: 100,
                actual: 99,
            })
        );
        assert_eq!(
            monitor.observe(&header(19, 4, 102), 102),
            Ok(PeerObservation::Continued)
        );
    }

    #[test]
    fn rejects_identity_time_and_age_mismatches() {
        let mut monitor = PeerMonitor::new(policy());
        let mut wrong_robot = header(19, 1, 100);
        wrong_robot.robot_id = 99;
        assert!(matches!(
            monitor.observe(&wrong_robot, 100),
            Err(PeerError::RobotMismatch { .. })
        ));
        assert!(matches!(
            monitor.observe(&header(19, 1, 101), 100),
            Err(PeerError::FutureTimestamp { .. })
        ));
        assert_eq!(
            monitor.observe(&header(19, 1, 100), 151),
            Err(PeerError::MessageStale {
                age_ns: 51,
                maximum_ns: 50,
            })
        );
        assert_eq!(monitor.status(0), Ok(PeerStatus::NeverSeen));
    }

    #[test]
    fn rejects_all_identity_mismatches_and_backwards_local_time() {
        let mut monitor = PeerMonitor::new(policy());
        let mut wrong_layout = header(19, 1, 100);
        wrong_layout.layout_hash = 99;
        assert!(matches!(
            monitor.observe(&wrong_layout, 100),
            Err(PeerError::LayoutMismatch { .. })
        ));

        let mut wrong_source = header(19, 1, 100);
        wrong_source.source_id = 99;
        assert!(matches!(
            monitor.observe(&wrong_source, 100),
            Err(PeerError::SourceMismatch { .. })
        ));

        monitor.observe(&header(19, 1, 100), 100).unwrap();
        assert_eq!(
            monitor.status(99),
            Err(PeerError::LocalTimeRegressed {
                previous: 100,
                actual: 99,
            })
        );
        assert_eq!(
            monitor.observe(&header(19, 2, 99), 99),
            Err(PeerError::LocalTimeRegressed {
                previous: 100,
                actual: 99,
            })
        );
    }

    #[test]
    fn policy_rejects_zero_contract_fields() {
        assert_eq!(
            PeerPolicy::new(0, 1, 1, 1, 1),
            Err(PeerError::InvalidPolicy(PolicyField::RobotId))
        );
        assert_eq!(
            PeerPolicy::new(1, 1, 1, 0, 1),
            Err(PeerError::InvalidPolicy(PolicyField::OfflineTimeout))
        );
    }
}
