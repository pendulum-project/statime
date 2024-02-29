use crate::time::Duration;

#[derive(Default, Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct InternalCurrentDS {
    pub(crate) steps_removed: u16,
    pub(crate) offset_from_master: Duration,
}
