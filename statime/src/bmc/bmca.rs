//! Implementation of the best master clock algorithm [Bmca]

use core::cmp::Ordering;

use super::{
    dataset_comparison::{ComparisonDataset, DatasetOrdering},
    foreign_master::ForeignMasterList,
};
use crate::{
    datastructures::{
        common::{PortIdentity, TimeInterval, WireTimestamp},
        datasets::DefaultDS,
        messages::AnnounceMessage,
    },
    port::state::PortState,
};

/// Object implementing the Best Master Clock Algorithm
///
/// Usage:
///
/// - Every port has its own instance.
/// - When a port receives an announce message, it has to register it with the
///   [Bmca::register_announce_message] method
/// - When it is time to run the algorithm, the ptp runtime has to take all the
///   best announce messages using [Bmca::take_best_port_announce_message]
/// - Of the resulting set, the best global one needs to be determined. This can
///   be done using [Bmca::find_best_announce_message]
/// - Then to get the recommended state for each port,
///   [Bmca::calculate_recommended_state] needs to be called
pub struct Bmca {
    foreign_master_list: ForeignMasterList,
    own_port_identity: PortIdentity,
}

impl Bmca {
    pub fn new(own_port_announce_interval: TimeInterval, own_port_identity: PortIdentity) -> Self {
        Self {
            foreign_master_list: ForeignMasterList::new(
                own_port_announce_interval,
                own_port_identity,
            ),
            own_port_identity,
        }
    }

    /// Register a received announce message to the BMC algorithm
    pub fn register_announce_message(
        &mut self,
        announce_message: &AnnounceMessage,
        current_time: WireTimestamp,
    ) {
        // Ignore messages comming from the same port
        if announce_message.header().source_port_identity() != self.own_port_identity {
            self.foreign_master_list
                .register_announce_message(announce_message, current_time);
        }
    }

    /// Takes the Erbest from this port
    pub fn take_best_port_announce_message(
        &mut self,
        current_time: WireTimestamp,
    ) -> Option<BestAnnounceMessage> {
        // Find the announce message we want to use from each foreign master that has
        // qualified messages
        let announce_messages = self
            .foreign_master_list
            .take_qualified_announce_messages(current_time);

        // The best of the foreign master messages is our erbest
        let erbest =
            Self::find_best_announce_message(announce_messages.map(|(message, timestamp)| {
                BestAnnounceMessage {
                    message,
                    timestamp,
                    identity: self.own_port_identity,
                }
            }));

        if let Some(best) = &erbest {
            // All messages that were considered have been removed from the
            // foreignmasterlist. However, the one that has been selected as the
            // Erbest must not be removed, so let's just reregister it.
            self.register_announce_message(&best.message, best.timestamp);
        }

        erbest
    }

    /// Finds the best announce message in the given iterator.
    /// The port identity in the tuple is the identity of the port that received
    /// the announce message.
    pub fn find_best_announce_message(
        announce_messages: impl IntoIterator<Item = BestAnnounceMessage>,
    ) -> Option<BestAnnounceMessage> {
        announce_messages
            .into_iter()
            .max_by(BestAnnounceMessage::compare)
    }

    /// Calculates the recommended port state. This has to be run for every
    /// port. The PTP spec calls this the State Decision Algorithm.
    ///
    /// - `own_data`: Called 'D0' by the PTP spec. The DefaultDS data of our own
    ///   ptp instance.
    /// - `best_global_announce_message`: Called 'Ebest' by the PTP spec. This
    ///   is the best announce message and the
    /// identity of the port that received it of all of the best port announce
    /// messages.
    /// - `best_port_announce_message`: Called 'Erbest' by the PTP spec. This is
    ///   the best announce message and the
    /// identity of the port that received it of the port we are calculating the
    /// recommended state for.
    /// - `port_state`: The current state of the port we are doing the
    ///   calculation for.
    ///
    /// If None is returned, then the port should remain in the same state as it
    /// is now.
    pub fn calculate_recommended_state(
        own_data: &DefaultDS,
        best_global_announce_message: Option<BestAnnounceMessage>,
        best_port_announce_message: Option<BestAnnounceMessage>,
        port_state: &PortState,
    ) -> Option<RecommendedState> {
        let d0 = ComparisonDataset::from_own_data(own_data);
        let ebest = best_global_announce_message
            .map(|best| ComparisonDataset::from_announce_message(&best.message, &best.identity));
        let erbest = best_port_announce_message
            .map(|best| ComparisonDataset::from_announce_message(&best.message, &best.identity));

        if best_global_announce_message.is_none() && matches!(port_state, PortState::Listening) {
            return None;
        }

        if (1..=127).contains(&own_data.clock_quality.clock_class) {
            return match erbest {
                None => Some(RecommendedState::M1(*own_data)),
                Some(erbest) => {
                    if d0.compare(&erbest).is_better() {
                        Some(RecommendedState::M1(*own_data))
                    } else {
                        Some(RecommendedState::P1(
                            best_port_announce_message.unwrap().message,
                        ))
                    }
                }
            };
        }

        match &ebest {
            None => return Some(RecommendedState::M2(*own_data)),
            Some(ebest) => {
                if d0.compare(ebest).is_better() {
                    return Some(RecommendedState::M2(*own_data));
                }
            }
        }

        // If ebest was empty, then we would have returned in the previous step
        let best_global_announce_message = best_global_announce_message.unwrap();
        let ebest = ebest.unwrap();

        match erbest {
            None => Some(RecommendedState::M3(best_global_announce_message.message)),
            Some(erbest) => {
                let best_port_announce_message = best_port_announce_message.unwrap();

                if best_global_announce_message.timestamp == best_port_announce_message.timestamp {
                    Some(RecommendedState::S1(best_global_announce_message.message))
                } else if matches!(ebest.compare(&erbest), DatasetOrdering::BetterByTopology) {
                    Some(RecommendedState::P2(best_port_announce_message.message))
                } else {
                    Some(RecommendedState::M3(best_global_announce_message.message))
                }
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct BestAnnounceMessage {
    message: AnnounceMessage,
    timestamp: WireTimestamp,
    identity: PortIdentity,
}

impl BestAnnounceMessage {
    fn compare(&self, other: &Self) -> Ordering {
        // use the timestamp as a tie-break if needed (prefer newer messages)
        let tie_break = self.timestamp.cmp(&other.timestamp);
        self.compare_dataset(other).as_ordering().then(tie_break)
    }

    fn compare_dataset(&self, other: &Self) -> DatasetOrdering {
        let data1 = ComparisonDataset::from_announce_message(&self.message, &self.identity);
        let data2 = ComparisonDataset::from_announce_message(&other.message, &other.identity);

        data1.compare(&data2)
    }
}

#[derive(Debug)]
pub enum RecommendedState {
    M1(DefaultDS),
    M2(DefaultDS),
    M3(AnnounceMessage),
    P1(AnnounceMessage),
    P2(AnnounceMessage),
    S1(AnnounceMessage),
}

#[cfg(test)]

mod tests {
    use super::*;
    use crate::{
        datastructures::messages::{Header, PtpVersion},
        ClockIdentity,
    };

    fn default_best_announce_message() -> BestAnnounceMessage {
        let header = Header {
            sdo_id: Default::default(),
            version: PtpVersion::new(2, 1).unwrap(),
            domain_number: Default::default(),
            alternate_master_flag: false,
            two_step_flag: false,
            unicast_flag: false,
            ptp_profile_specific_1: false,
            ptp_profile_specific_2: false,
            leap61: false,
            leap59: false,
            current_utc_offset_valid: false,
            ptp_timescale: false,
            time_tracable: false,
            frequency_tracable: false,
            synchronization_uncertain: false,
            correction_field: Default::default(),
            source_port_identity: Default::default(),
            sequence_id: Default::default(),
            log_message_interval: Default::default(),
        };

        let message = AnnounceMessage {
            header,
            origin_timestamp: Default::default(),
            current_utc_offset: Default::default(),
            grandmaster_priority_1: Default::default(),
            grandmaster_clock_quality: Default::default(),
            grandmaster_priority_2: Default::default(),
            grandmaster_identity: Default::default(),
            steps_removed: Default::default(),
            time_source: Default::default(),
        };

        let timestamp = WireTimestamp {
            seconds: 0,
            nanos: 0,
        };

        let identity = PortIdentity {
            clock_identity: ClockIdentity([0; 8]),
            port_number: 1,
        };

        BestAnnounceMessage {
            message,
            timestamp,
            identity,
        }
    }

    #[test]
    fn best_announce_message_compare_equal() {
        let message1 = default_best_announce_message();
        let message2 = default_best_announce_message();

        let ordering = message1.compare_dataset(&message2).as_ordering();
        assert_eq!(ordering, Ordering::Equal);
    }

    #[test]
    fn best_announce_message_compare() {
        let mut message1 = default_best_announce_message();
        let mut message2 = default_best_announce_message();

        // identities are different
        message1.message.grandmaster_identity = ClockIdentity([0; 8]);
        message2.message.grandmaster_identity = ClockIdentity([1; 8]);

        // higher priority is worse in this ordering
        message1.message.grandmaster_priority_1 = 0;
        message2.message.grandmaster_priority_1 = 1;

        // hence we expect message1 to be better than message2
        assert_eq!(message1.compare_dataset(&message2), DatasetOrdering::Better);
        assert_eq!(message2.compare_dataset(&message1), DatasetOrdering::Worse);

        assert_eq!(message1.compare(&message2), Ordering::Greater);
        assert_eq!(message2.compare(&message1), Ordering::Less);
    }

    #[test]
    fn best_announce_message_max_tie_break() {
        let mut message1 = default_best_announce_message();
        let mut message2 = default_best_announce_message();

        message1.timestamp = WireTimestamp {
            seconds: 1_000_000,
            nanos: 1001,
        };
        message2.timestamp = WireTimestamp {
            seconds: 1_000_001,
            nanos: 1000,
        };

        // the newest message should be preferred
        assert!(message2.timestamp > message1.timestamp);

        let ordering = message1.compare_dataset(&message2).as_ordering();
        assert_eq!(ordering, Ordering::Equal);

        // so message1 is lower in the ordering than message2
        assert_eq!(message1.compare(&message2), Ordering::Less)
    }
}
