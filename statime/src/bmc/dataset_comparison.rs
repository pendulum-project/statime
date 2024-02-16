//! Implementation of chapter 9.3.4 Data set comparison algorithm

use core::cmp::Ordering;

use crate::datastructures::{
    common::{ClockIdentity, ClockQuality, PortIdentity},
    datasets::InternalDefaultDS,
    messages::AnnounceMessage,
};

/// A collection of data that is gathered from other sources (mainly announce
/// messages and the DefaultDS). When gathered from two different sources, the
/// [compare](crate::bmc::dataset_comparison::ComparisonDataset) method can be
/// used to find out which source is better according to the dataset comparison
/// algorithm.
#[derive(Eq, PartialEq, Default, Debug)]
pub(crate) struct ComparisonDataset {
    gm_priority_1: u8,
    gm_identity: ClockIdentity,
    gm_clock_quality: ClockQuality,
    gm_priority_2: u8,
    steps_removed: u16,
    identity_of_senders: ClockIdentity,
    identity_of_receiver: PortIdentity,
}

impl ComparisonDataset {
    /// Create a ComparisonDataset from the data in an announce message and the
    /// port identity of the port that received the announce message
    pub(crate) fn from_announce_message(
        message: &AnnounceMessage,
        port_receiver_identity: &PortIdentity,
    ) -> Self {
        Self {
            gm_priority_1: message.grandmaster_priority_1,
            gm_identity: message.grandmaster_identity,
            gm_clock_quality: message.grandmaster_clock_quality,
            gm_priority_2: message.grandmaster_priority_2,
            steps_removed: message.steps_removed,
            identity_of_senders: message.header.source_port_identity.clock_identity,
            identity_of_receiver: *port_receiver_identity,
        }
    }

    pub(crate) fn from_own_data(data: &InternalDefaultDS) -> Self {
        Self {
            gm_priority_1: data.priority_1,
            gm_identity: data.clock_identity,
            gm_clock_quality: data.clock_quality,
            gm_priority_2: data.priority_2,
            steps_removed: 0,
            identity_of_senders: data.clock_identity,
            identity_of_receiver: PortIdentity {
                clock_identity: data.clock_identity,
                port_number: 0,
            },
        }
    }

    /// Returns the ordering of `self` in comparison to other.
    pub(crate) fn compare(&self, other: &Self) -> DatasetOrdering {
        if self.gm_identity == other.gm_identity {
            Self::compare_same_identity(self, other)
        } else {
            Self::compare_different_identity(self, other)
        }
    }

    /// PTP grandmaster instances are different
    fn compare_different_identity(&self, other: &Self) -> DatasetOrdering {
        let self_quality = self.gm_clock_quality;
        let other_quality = other.gm_clock_quality;

        // Figure 34
        let ordering = (self.gm_priority_1.cmp(&other.gm_priority_1))
            .then_with(|| self_quality.clock_class.cmp(&other_quality.clock_class))
            // The spec assumes numerical ordering (which is the reverse of the semantic ordering)
            .then_with(|| self_quality.clock_accuracy.cmp_numeric(&other_quality.clock_accuracy))
            .then_with(|| self_quality.offset_scaled_log_variance.cmp(&other_quality.offset_scaled_log_variance))
            .then_with(|| self.gm_priority_2.cmp(&other.gm_priority_2))
            .then_with(|| self.gm_identity.cmp(&other.gm_identity));

        match ordering {
            Ordering::Equal => unreachable!("gm_identity is guaranteed to be different"),
            Ordering::Greater => DatasetOrdering::Worse,
            Ordering::Less => DatasetOrdering::Better,
        }
    }

    /// Potentially the same PTP grandmaster instance
    fn compare_same_identity(&self, other: &Self) -> DatasetOrdering {
        let steps_removed_difference = self.steps_removed as i32 - other.steps_removed as i32;

        // Figure 35
        match steps_removed_difference {
            2..=i32::MAX => DatasetOrdering::Worse,
            i32::MIN..=-2 => DatasetOrdering::Better,
            1 => match Ord::cmp(
                &self.identity_of_receiver.clock_identity,
                &self.identity_of_senders,
            ) {
                Ordering::Less => DatasetOrdering::Worse,
                Ordering::Equal => DatasetOrdering::Error1,
                Ordering::Greater => DatasetOrdering::WorseByTopology,
            },
            -1 => match Ord::cmp(
                &other.identity_of_receiver.clock_identity,
                &other.identity_of_senders,
            ) {
                Ordering::Less => DatasetOrdering::Better,
                Ordering::Equal => DatasetOrdering::Error1,
                Ordering::Greater => DatasetOrdering::BetterByTopology,
            },
            0 => {
                let senders = self.identity_of_senders.cmp(&other.identity_of_senders);
                let receivers = Ord::cmp(
                    &self.identity_of_receiver.port_number,
                    &other.identity_of_receiver.port_number,
                );

                match senders.then(receivers) {
                    Ordering::Less => DatasetOrdering::BetterByTopology,
                    Ordering::Equal => DatasetOrdering::Error2,
                    Ordering::Greater => DatasetOrdering::WorseByTopology,
                }
            }
        }
    }
}

/// The ordering result of the dataset comparison algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetOrdering {
    /// The [ComparisonDataset] is better than the one being compared against
    Better,
    /// The [ComparisonDataset] is of equal quality as the one being compared
    /// against, but is preferred because of the network topology
    BetterByTopology,
    /// The [ComparisonDataset] is equal in quality and topology
    Error1,
    /// The [ComparisonDataset] is probably based on the same set of data
    Error2,
    /// The [ComparisonDataset] is of equal quality as the one being compared
    /// against, but is not preferred because of the network topology
    WorseByTopology,
    /// The [ComparisonDataset] is worse than the one being compared against
    Worse,
}

impl DatasetOrdering {
    pub const fn as_ordering(self) -> Ordering {
        // We get errors if two announce messages are (functionally) the same
        // in that case either option is a valid choice
        match self {
            DatasetOrdering::Better | DatasetOrdering::BetterByTopology => Ordering::Greater,
            DatasetOrdering::Error1 | DatasetOrdering::Error2 => Ordering::Equal,
            DatasetOrdering::WorseByTopology | DatasetOrdering::Worse => Ordering::Less,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datastructures::common::ClockAccuracy;

    const IDENTITY_A: ClockIdentity = ClockIdentity([1, 1, 1, 1, 1, 1, 1, 1]);
    const IDENTITY_B: ClockIdentity = ClockIdentity([2, 2, 2, 2, 2, 2, 2, 2]);
    const IDENTITY_C: ClockIdentity = ClockIdentity([3, 3, 3, 3, 3, 3, 3, 3]);

    fn get_default_test_pair() -> (ComparisonDataset, ComparisonDataset) {
        Default::default()
    }

    #[test]
    fn figure_34() {
        // Start with two identical datasets
        let (mut a, mut b) = get_default_test_pair();

        // Now we work bottom up to test everything
        // Every time we we change which one is better or worse so we know that it's not
        // still the previous result coming through

        a.gm_identity = IDENTITY_A;
        b.gm_identity = IDENTITY_B;

        assert_eq!(a.compare(&b), DatasetOrdering::Better);
        assert_eq!(b.compare(&a), DatasetOrdering::Worse);

        a.gm_priority_2 = 1;
        b.gm_priority_2 = 0;

        assert_eq!(a.compare(&b), DatasetOrdering::Worse);
        assert_eq!(b.compare(&a), DatasetOrdering::Better);

        a.gm_clock_quality.offset_scaled_log_variance = 0;
        b.gm_clock_quality.offset_scaled_log_variance = 1;

        assert_eq!(a.compare(&b), DatasetOrdering::Better);
        assert_eq!(b.compare(&a), DatasetOrdering::Worse);

        a.gm_clock_quality.clock_accuracy = ClockAccuracy::US1;
        b.gm_clock_quality.clock_accuracy = ClockAccuracy::NS1;

        assert_eq!(a.compare(&b), DatasetOrdering::Worse);
        assert_eq!(b.compare(&a), DatasetOrdering::Better);

        a.gm_clock_quality.clock_class = 0;
        b.gm_clock_quality.clock_class = 1;

        assert_eq!(a.compare(&b), DatasetOrdering::Better);
        assert_eq!(b.compare(&a), DatasetOrdering::Worse);

        a.gm_priority_1 = 1;
        b.gm_priority_1 = 0;

        assert_eq!(a.compare(&b), DatasetOrdering::Worse);
        assert_eq!(b.compare(&a), DatasetOrdering::Better);
    }

    #[test]
    fn figure_35() {
        let (mut a, mut b) = get_default_test_pair();

        assert_eq!(a.compare(&b), DatasetOrdering::Error2);
        assert_eq!(b.compare(&a), DatasetOrdering::Error2);

        a.identity_of_receiver.port_number = 1;
        b.identity_of_receiver.port_number = 0;

        assert_eq!(a.compare(&b), DatasetOrdering::WorseByTopology);
        assert_eq!(b.compare(&a), DatasetOrdering::BetterByTopology);

        a.identity_of_senders = IDENTITY_A;
        b.identity_of_senders = IDENTITY_B;

        assert_eq!(a.compare(&b), DatasetOrdering::BetterByTopology);
        assert_eq!(b.compare(&a), DatasetOrdering::WorseByTopology);

        a.steps_removed = 0;
        a.identity_of_receiver.clock_identity = IDENTITY_A;
        b.steps_removed = 1;
        b.identity_of_receiver.clock_identity = IDENTITY_B;

        assert_eq!(a.compare(&b), DatasetOrdering::Error1);
        assert_eq!(b.compare(&a), DatasetOrdering::Error1);

        a.identity_of_receiver.clock_identity = IDENTITY_B;
        b.identity_of_receiver.clock_identity = IDENTITY_C;

        assert_eq!(a.compare(&b), DatasetOrdering::BetterByTopology);
        assert_eq!(b.compare(&a), DatasetOrdering::WorseByTopology);

        // the inverse of the identity_of_senders
        a.identity_of_receiver.clock_identity = IDENTITY_B;
        b.identity_of_receiver.clock_identity = IDENTITY_A;

        assert_eq!(a.compare(&b), DatasetOrdering::Better);
        assert_eq!(b.compare(&a), DatasetOrdering::Worse);

        a.steps_removed = 0;
        b.steps_removed = 2;

        assert_eq!(a.compare(&b), DatasetOrdering::Better);
        assert_eq!(b.compare(&a), DatasetOrdering::Worse);
    }
}
