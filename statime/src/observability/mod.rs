//! Serializable implementations of datastructures to be used for observability
/// A concrete implementation of the PTP Current dataset (IEEE1588-2019 section 8.2.2)
pub mod current;
/// A concrete implementation of the PTP Default dataset (IEEE1588-2019 section 8.2.1)
pub mod default;
/// A concrete implementation of the PTP Parent dataset (IEEE1588-2019 section 8.2.3)
pub mod parent;

use crate::datastructures::datasets::TimePropertiesDS;

use self::{current::CurrentDS, default::DefaultDS, parent::ParentDS};

/// Observable version of the InstanceState struct
#[derive(Debug, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ObservableInstanceState {
    /// A concrete implementation of the PTP Default dataset (IEEE1588-2019 section 8.2.1)
    pub default_ds: DefaultDS,
    /// A concrete implementation of the PTP Current dataset (IEEE1588-2019 section 8.2.2)
    pub current_ds: CurrentDS,
    /// A concrete implementation of the PTP Parent dataset (IEEE1588-2019 section 8.2.3)
    pub parent_ds: ParentDS,
    /// A concrete implementation of the PTP Time Properties dataset (IEEE1588-2019 section 8.2.4)
    pub time_properties_ds: TimePropertiesDS,
}
