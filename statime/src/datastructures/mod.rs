//! General datastructures as defined by the ptp spec

use core::fmt::Debug;

use self::messages::EnumConversionError;

pub mod common;
pub mod datasets;
pub mod messages;

#[derive(Clone, Debug)]
pub(crate) enum WireFormatError {
    EnumConversionError,
    BufferTooShort,
    CapacityError,
    Invalid,
}

impl core::fmt::Display for WireFormatError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WireFormatError::EnumConversionError => f.write_str("enum conversion failed"),
            WireFormatError::BufferTooShort => f.write_str("a buffer is too short"),
            WireFormatError::CapacityError => f.write_str("a container has insufficient capacity"),
            WireFormatError::Invalid => f.write_str("an invariant was violated"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for WireFormatError {}

impl From<arrayvec::CapacityError> for WireFormatError {
    fn from(_: arrayvec::CapacityError) -> Self {
        WireFormatError::CapacityError
    }
}

impl From<EnumConversionError> for WireFormatError {
    fn from(_: EnumConversionError) -> Self {
        Self::EnumConversionError
    }
}

trait WireFormat: Debug + Clone + Eq {
    /// Serializes the object into the PTP wire format.
    ///
    /// Returns the used buffer size that contains the message or an error.
    fn serialize(&self, buffer: &mut [u8]) -> Result<(), WireFormatError>;

    /// Deserializes the object from the PTP wire format.
    ///
    /// Returns the object and the size in the buffer that it takes up or an
    /// error.
    fn deserialize(buffer: &[u8]) -> Result<Self, WireFormatError>;
}
