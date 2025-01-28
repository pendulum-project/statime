use crate::{datastructures::{WireFormat, WireFormatError}, time::Time};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PartialOrd, Ord)]
pub struct WireTimestampV1 {
    /// The seconds field of the timestamp.
    /// May wrap around (Y38K problem)
    pub seconds: u32,
    /// The nanoseconds field of the timestamp.
    /// Must be less than 10^9
    pub nanos: u32,
}
impl WireFormat for WireTimestampV1 {
    fn serialize(&self, buffer: &mut [u8]) -> Result<(), WireFormatError> {
        buffer[0..4].copy_from_slice(&self.seconds.to_be_bytes());
        buffer[4..8].copy_from_slice(&self.nanos.to_be_bytes());
        Ok(())
    }
    fn deserialize(buffer: &[u8]) -> Result<Self, WireFormatError> {
        Ok(Self {
            seconds: u32::from_be_bytes(buffer[0..4].try_into().unwrap()),
            nanos: u32::from_be_bytes(buffer[4..8].try_into().unwrap()),
        })
    }
}
impl From<Time> for WireTimestampV1 {
    fn from(instant: Time) -> Self {
        WireTimestampV1 {
            seconds: instant.secs() as u32, // TODO: will wrap-around in 2038
            // TODO we need to account for this overflow in other parts of the code
            nanos: instant.subsec_nanos(),
        }
    }
}
/* impl From<WireTimestampV1> for WireTimestamp {
    fn from(v1: WireTimestampV1) -> Self {
        Self { seconds: v1.seconds as u64, nanos: v1.nanos }
    }
}
impl From<WireTimestamp> for WireTimestampV1 {
    fn from(v2: WireTimestamp) -> Self {
        Self { seconds: v2.seconds as u32, nanos: v2.nanos }
    }
}
//hopefully won't be needed, TODO remove */