use crate::{config::{LeapIndicator, TimePropertiesDS}, datastructures::{common::{GrandmasterPropertiesV1, WireTimestampV1}, WireFormat, WireFormatError}};

use super::Header;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SyncOrDelayReqMessage {
    pub(crate) header: Header,
    pub(crate) origin_timestamp: WireTimestampV1,
    pub(crate) epoch_number: u16,
    pub(crate) current_utc_offset: i16,
    pub(crate) grandmaster: GrandmasterPropertiesV1,
    pub(crate) sync_interval: i8,
    pub(crate) local_clock_variance: i16,
    pub(crate) local_steps_removed: u16,
    pub(crate) local_clock_stratum: u8,
    pub(crate) local_clock_identifier: [u8; 4],
    pub(crate) parent_communication_technology: u8,
    pub(crate) parent_uuid: [u8; 6],
    pub(crate) parent_port_field: u16,
    pub(crate) estimated_master_variance: i16,
    pub(crate) estimated_master_drift: i32,
    pub(crate) utc_reasonable: bool
}
impl SyncOrDelayReqMessage {
    pub(crate) fn content_size(&self) -> usize {
        84
    }
    pub(crate) fn serialize_content(&self, buffer: &mut [u8]) -> Result<(), WireFormatError> {
        buffer[0..84].fill(0);
        self.origin_timestamp.serialize(&mut buffer[0..8])?;
        buffer[8..10].copy_from_slice(&self.epoch_number.to_be_bytes());
        buffer[10..12].copy_from_slice(&self.current_utc_offset.to_be_bytes());
        self.grandmaster.serialize(&mut buffer[12..40])?;
        buffer[43] = self.sync_interval as u8;
        buffer[46..48].copy_from_slice(&self.local_clock_variance.to_be_bytes());
        buffer[50..52].copy_from_slice(&self.local_steps_removed.to_be_bytes());
        buffer[55] = self.local_clock_stratum;
        buffer[56..60].copy_from_slice(&self.local_clock_identifier);
        buffer[61] = self.parent_communication_technology;
        buffer[62..68].copy_from_slice(&self.parent_uuid);
        buffer[70..72].copy_from_slice(&self.parent_port_field.to_be_bytes());
        buffer[74..76].copy_from_slice(&self.estimated_master_variance.to_be_bytes());
        buffer[76..80].copy_from_slice(&self.estimated_master_drift.to_be_bytes());
        buffer[83] = self.utc_reasonable as u8;
        Ok(())
    }
    pub(crate) fn deserialize_content(header: Header, buffer: &[u8]) -> Result<Self, WireFormatError> {
        match buffer.get(0..84) {
            None => {
                log::error!("BufferTooShort SyncOrDelayReqMessage");
                Err(WireFormatError::BufferTooShort)
            },
            Some(buf) => Ok(Self {
                header,
                origin_timestamp: WireTimestampV1::deserialize(&buf[0..8])?,
                epoch_number: u16::from_be_bytes([buf[8], buf[9]]),
                current_utc_offset: i16::from_be_bytes([buf[10], buf[11]]),
                grandmaster: GrandmasterPropertiesV1::deserialize(&buf[12..40])?,
                sync_interval: buf[43] as i8,
                local_clock_variance: i16::from_be_bytes([buf[46], buf[47]]),
                local_steps_removed: u16::from_be_bytes([buf[50], buf[51]]),
                local_clock_stratum: buf[55],
                local_clock_identifier: buf[56..60].try_into().unwrap(),
                parent_communication_technology: buf[61],
                parent_uuid: buf[62..68].try_into().unwrap(),
                parent_port_field: u16::from_be_bytes([buf[70], buf[71]]),
                estimated_master_variance: i16::from_be_bytes([buf[74], buf[75]]),
                estimated_master_drift: i32::from_be_bytes(buf[76..80].try_into().unwrap()),
                utc_reasonable: buf[83] > 0
            }),
        }
    }

    pub(crate) fn time_properties(&self) -> TimePropertiesDS {
        let leap_indicator = if self.header.leap59 {
            LeapIndicator::Leap59
        } else if self.header.leap61 {
            LeapIndicator::Leap61
        } else {
            LeapIndicator::NoLeap
        };

        let current_utc_offset = self
            .utc_reasonable
            .then_some(self.current_utc_offset);

        TimePropertiesDS {
            current_utc_offset,
            leap_indicator,
            time_traceable: false, // TODO
            frequency_traceable: false, // TODO
            ptp_timescale: [*b"ATOM", *b"GPS\0", *b"NTP\0", *b"HAND"].contains(&self.grandmaster.clock_identifier), // TODO: really???
            time_source: crate::config::TimeSource::InternalOscillator, // TODO
        }
    }
}
pub(crate) type SyncMessage = SyncOrDelayReqMessage;
pub(crate) type DelayReqMessage = SyncOrDelayReqMessage;
