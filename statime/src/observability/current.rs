use crate::{datastructures::datasets::InternalCurrentDS, filters::FilterEstimate, time::Duration};

/// A concrete implementation of the PTP Current dataset (IEEE1588-2019 section
/// 8.2.2)
///
/// Note that the `meanDelay` field from IEEE1588-2019 section 8.2.2.4 is
/// missing since this field can be constructed from the portDS.
#[derive(Debug, Default, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CurrentDS {
    /// See *IEEE1588-2019 section 8.2.2.2*.
    pub steps_removed: u16,
    /// See *IEEE1588-2019 section 8.2.2.3*.
    pub offset_from_master: Duration,
    /// See *IEEE1588-2019 section 8.2.2.3*.
    pub mean_delay: Duration,
}

impl CurrentDS {
    pub(crate) fn from_state(
        current_ds: &InternalCurrentDS,
        port_contribution: Option<FilterEstimate>,
    ) -> Self {
        match port_contribution {
            Some(port_contribution) => Self {
                steps_removed: current_ds.steps_removed,
                offset_from_master: port_contribution.offset_from_master,
                mean_delay: port_contribution.mean_delay,
            },
            None => Self {
                steps_removed: current_ds.steps_removed,
                offset_from_master: Duration::ZERO,
                mean_delay: Duration::ZERO,
            },
        }
    }
}
