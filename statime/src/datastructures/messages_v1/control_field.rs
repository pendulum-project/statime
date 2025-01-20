use super::EnumConversionError;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlField {
    Sync,
    DelayReq,
    FollowUp,
    DelayResp,
    /* Management, */ // TOOD
}
impl ControlField {
    pub fn to_primitive(self) -> u8 {
        match self {
            ControlField::Sync => 0x00,
            ControlField::DelayReq => 0x01,
            ControlField::FollowUp => 0x02,
            ControlField::DelayResp => 0x03,
            /* ControlField::Management => 0x04, */
        }
    }
}
impl TryFrom<u8> for ControlField {
    type Error = EnumConversionError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        use ControlField::*;
        match value {
            0x00 => Ok(Sync),
            0x01 => Ok(DelayReq),
            0x02 => Ok(FollowUp),
            0x03 => Ok(DelayResp),
            /* 0x04 => Ok(Management), */
            _ => Err(EnumConversionError),
        }
    }
}