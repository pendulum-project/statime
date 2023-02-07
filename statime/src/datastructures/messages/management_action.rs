/// See: 15.4.1.6
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagementAction {
    Reserved,
    GET,
    SET,
    RESPONSE,
    COMMAND,
    ACKNOWLEDGE,
}

impl ManagementAction {
    pub fn to_primitive(&self) -> u8 {
        match self {
            Self::GET => 0x0,
            Self::SET => 0x1,
            Self::RESPONSE => 0x2,
            Self::COMMAND => 0x3,
            Self::ACKNOWLEDGE => 0x4,
            Self::Reserved => 0x5,
        }
    }

    pub fn from_primitive(value: u8) -> Self {
        match value {
            0x0 => Self::GET,
            0x1 => Self::SET,
            0x2 => Self::RESPONSE,
            0x3 => Self::COMMAND,
            0x4 => Self::ACKNOWLEDGE,
            0x5..=u8::MAX => Self::Reserved,
        }
    }
}
