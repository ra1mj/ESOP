use crate::wire::{
    Command, FrameBuilder, MAX_ETHERNET_FRAME_LEN, MIN_ETHERNET_FRAME_LEN, WireError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DatagramPlan {
    pub command: Command,
    pub index: u8,
    pub address: u32,
    pub payload_offset: usize,
    pub payload_len: usize,
    pub expected_wkc: u16,
}

impl DatagramPlan {
    pub const EMPTY: Self = Self {
        command: Command::Lrw,
        index: 0,
        address: 0,
        payload_offset: 0,
        payload_len: 0,
        expected_wkc: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanError {
    CapacityExceeded,
    DuplicateIndex,
    DatagramLengthOutOfBounds,
    FrameTooLarge,
    BufferTooSmall,
    ProcessImageOutOfBounds,
    Wire(WireError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FramePlan<const DATAGRAMS: usize> {
    datagrams: [DatagramPlan; DATAGRAMS],
    count: usize,
    payload_len: usize,
}

impl<const DATAGRAMS: usize> FramePlan<DATAGRAMS> {
    pub const fn new() -> Self {
        Self {
            datagrams: [DatagramPlan::EMPTY; DATAGRAMS],
            count: 0,
            payload_len: 0,
        }
    }

    pub fn push(&mut self, datagram: DatagramPlan) -> Result<(), PlanError> {
        if self.count >= DATAGRAMS {
            return Err(PlanError::CapacityExceeded);
        }
        if self
            .datagrams()
            .iter()
            .any(|existing| existing.index == datagram.index)
        {
            return Err(PlanError::DuplicateIndex);
        }
        if datagram.payload_len > 0x07FF {
            return Err(PlanError::DatagramLengthOutOfBounds);
        }

        let next_payload_len = self
            .payload_len
            .saturating_add(10)
            .saturating_add(datagram.payload_len)
            .saturating_add(2);
        if next_payload_len > 0x07FF {
            return Err(PlanError::FrameTooLarge);
        }
        let frame_len = 16usize.saturating_add(next_payload_len);
        if frame_len > MAX_ETHERNET_FRAME_LEN {
            return Err(PlanError::FrameTooLarge);
        }

        self.datagrams[self.count] = datagram;
        self.count += 1;
        self.payload_len = next_payload_len;
        Ok(())
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub const fn payload_len(&self) -> usize {
        self.payload_len
    }

    pub fn datagrams(&self) -> &[DatagramPlan] {
        &self.datagrams[..self.count]
    }

    pub fn build(
        &self,
        buffer: &mut [u8],
        destination: [u8; 6],
        source: [u8; 6],
        process_image: &[u8],
    ) -> Result<usize, PlanError> {
        if self.is_empty() {
            return Err(PlanError::Wire(WireError::DatagramHeaderTruncated));
        }
        if buffer.len() < MIN_ETHERNET_FRAME_LEN {
            return Err(PlanError::BufferTooSmall);
        }

        let mut builder =
            FrameBuilder::new(buffer, destination, source).map_err(PlanError::Wire)?;
        for datagram in self.datagrams() {
            let end = datagram
                .payload_offset
                .checked_add(datagram.payload_len)
                .ok_or(PlanError::ProcessImageOutOfBounds)?;
            if end > process_image.len() {
                return Err(PlanError::ProcessImageOutOfBounds);
            }
            builder
                .push(
                    datagram.command,
                    datagram.index,
                    datagram.address,
                    &process_image[datagram.payload_offset..end],
                )
                .map_err(PlanError::Wire)?;
        }
        builder.finish().map_err(PlanError::Wire)
    }
}

impl<const DATAGRAMS: usize> Default for FramePlan<DATAGRAMS> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramePlanSetError {
    CapacityExceeded,
    DuplicateIndex,
    Plan(PlanError),
}

/// Fixed-capacity collection of frames produced from one deterministic
/// datagram sequence.
///
/// Datagrams are appended in order. A new frame is opened only when the
/// current frame cannot accept the next datagram because of its datagram
/// capacity or encoded frame size. The collection is committed atomically so
/// a failed append leaves every already-published frame unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FramePlanSet<const FRAMES: usize, const DATAGRAMS: usize> {
    plans: [FramePlan<DATAGRAMS>; FRAMES],
    count: usize,
}

impl<const FRAMES: usize, const DATAGRAMS: usize> FramePlanSet<FRAMES, DATAGRAMS> {
    pub const fn new() -> Self {
        Self {
            plans: [FramePlan::new(); FRAMES],
            count: 0,
        }
    }

    pub const fn frame_count(&self) -> usize {
        self.count
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn plans(&self) -> &[FramePlan<DATAGRAMS>] {
        &self.plans[..self.count]
    }

    pub fn plan(&self, index: usize) -> Option<&FramePlan<DATAGRAMS>> {
        self.plans.get(index).filter(|_| index < self.count)
    }

    pub fn datagram_count(&self) -> usize {
        self.plans().iter().map(FramePlan::len).sum()
    }

    pub fn push(&mut self, datagram: DatagramPlan) -> Result<(), FramePlanSetError> {
        self.append_datagram(datagram)
    }

    pub fn append_datagram(&mut self, datagram: DatagramPlan) -> Result<(), FramePlanSetError> {
        self.append_datagrams(core::slice::from_ref(&datagram))
            .map(|_| ())
    }

    pub fn append_datagrams(
        &mut self,
        datagrams: &[DatagramPlan],
    ) -> Result<usize, FramePlanSetError> {
        let mut next = *self;
        let mut appended = 0;

        for datagram in datagrams {
            if next.plans().iter().any(|plan| {
                plan.datagrams()
                    .iter()
                    .any(|item| item.index == datagram.index)
            }) {
                return Err(FramePlanSetError::DuplicateIndex);
            }

            if next.count == 0 {
                if FRAMES == 0 {
                    return Err(FramePlanSetError::CapacityExceeded);
                }
                next.count = 1;
            }

            let current = next.count - 1;
            match next.plans[current].push(*datagram) {
                Ok(()) => {}
                Err(PlanError::CapacityExceeded | PlanError::FrameTooLarge) => {
                    if next.count >= FRAMES {
                        return Err(FramePlanSetError::CapacityExceeded);
                    }
                    let mut frame = FramePlan::new();
                    frame.push(*datagram).map_err(FramePlanSetError::Plan)?;
                    next.plans[next.count] = frame;
                    next.count += 1;
                }
                Err(PlanError::DuplicateIndex) => {
                    return Err(FramePlanSetError::DuplicateIndex);
                }
                Err(error) => return Err(FramePlanSetError::Plan(error)),
            }
            appended += 1;
        }

        *self = next;
        Ok(appended)
    }
}

impl<const FRAMES: usize, const DATAGRAMS: usize> Default for FramePlanSet<FRAMES, DATAGRAMS> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::FrameView;

    #[test]
    fn plan_builds_fixed_datagrams_from_process_image() {
        let mut plan = FramePlan::<2>::new();
        plan.push(DatagramPlan {
            command: Command::Lrw,
            index: 1,
            address: 0x1000,
            payload_offset: 2,
            payload_len: 3,
            expected_wkc: 1,
        })
        .unwrap();
        plan.push(DatagramPlan {
            command: Command::Fprd,
            index: 2,
            address: 0x2000,
            payload_offset: 0,
            payload_len: 2,
            expected_wkc: 1,
        })
        .unwrap();

        let mut frame = [0u8; MAX_ETHERNET_FRAME_LEN];
        let process_image = [0xA0, 0xA1, 0xA2, 0xA3, 0xA4];
        let length = plan
            .build(&mut frame, [0xFF; 6], [1, 2, 3, 4, 5, 6], &process_image)
            .unwrap();
        let view = FrameView::parse(&frame[..length]).unwrap();
        let datagrams: [_; 2] = [
            view.datagrams().next().unwrap().unwrap(),
            view.datagrams().nth(1).unwrap().unwrap(),
        ];
        assert_eq!(datagrams[0].payload, &[0xA2, 0xA3, 0xA4]);
        assert_eq!(datagrams[1].payload, &[0xA0, 0xA1]);
    }

    #[test]
    fn plan_rejects_process_image_overrun() {
        let mut plan = FramePlan::<1>::new();
        plan.push(DatagramPlan {
            command: Command::Lwr,
            index: 1,
            address: 0,
            payload_offset: 4,
            payload_len: 2,
            expected_wkc: 0,
        })
        .unwrap();
        let mut frame = [0u8; MAX_ETHERNET_FRAME_LEN];
        assert_eq!(
            plan.build(&mut frame, [0; 6], [0; 6], &[0; 5]),
            Err(PlanError::ProcessImageOutOfBounds)
        );
    }

    #[test]
    fn plan_rejects_duplicate_indices() {
        let mut plan = FramePlan::<2>::new();
        let datagram = DatagramPlan {
            command: Command::Lrw,
            index: 5,
            address: 0,
            payload_offset: 0,
            payload_len: 1,
            expected_wkc: 1,
        };
        plan.push(datagram).unwrap();
        assert_eq!(plan.push(datagram), Err(PlanError::DuplicateIndex));
    }

    fn datagram(index: u8, payload_len: usize) -> DatagramPlan {
        DatagramPlan {
            command: Command::Lrw,
            index,
            address: 0,
            payload_offset: 0,
            payload_len,
            expected_wkc: 1,
        }
    }

    #[test]
    fn plan_set_splits_when_frame_datagram_capacity_is_reached() {
        let mut plans = FramePlanSet::<2, 1>::new();
        plans
            .append_datagrams(&[datagram(1, 1), datagram(2, 1)])
            .unwrap();

        assert_eq!(plans.frame_count(), 2);
        assert_eq!(plans.datagram_count(), 2);
        assert_eq!(plans.plan(0).unwrap().datagrams()[0].index, 1);
        assert_eq!(plans.plan(1).unwrap().datagrams()[0].index, 2);
    }

    #[test]
    fn plan_set_splits_when_encoded_frame_exceeds_mtu() {
        let mut plans = FramePlanSet::<2, 4>::new();
        plans
            .append_datagrams(&[datagram(1, 750), datagram(2, 750)])
            .unwrap();

        assert_eq!(plans.frame_count(), 2);
        assert_eq!(plans.plan(0).unwrap().len(), 1);
        assert_eq!(plans.plan(1).unwrap().len(), 1);
    }

    #[test]
    fn plan_set_failure_does_not_publish_partial_frames() {
        let mut plans = FramePlanSet::<1, 1>::new();
        let original = plans;
        assert_eq!(
            plans.append_datagrams(&[datagram(1, 1), datagram(2, 1)]),
            Err(FramePlanSetError::CapacityExceeded)
        );
        assert_eq!(plans, original);
    }

    #[test]
    fn plan_set_rejects_duplicate_indices_across_frames() {
        let mut plans = FramePlanSet::<2, 1>::new();
        plans.push(datagram(1, 1)).unwrap();
        plans.push(datagram(2, 1)).unwrap();
        let original = plans;

        assert_eq!(
            plans.push(datagram(1, 1)),
            Err(FramePlanSetError::DuplicateIndex)
        );
        assert_eq!(plans, original);
    }
}
