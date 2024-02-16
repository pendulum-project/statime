use core::iter::Fuse;

use arrayvec::ArrayVec;

use crate::{
    datastructures::common::{PortIdentity, Tlv, TlvSetIterator},
    filters::FilterUpdate,
};

#[derive(Debug, Clone)]
/// TLV that needs to be forwarded in the announce messages of other ports.
pub struct ForwardedTLV<'a> {
    pub(super) tlv: Tlv<'a>,
    pub(super) sender_identity: PortIdentity,
}

impl<'a> ForwardedTLV<'a> {
    /// Wire size of the TLV. Can be used to determine how many TLV's to keep
    pub fn size(&self) -> usize {
        self.tlv.wire_size()
    }

    /// Get an owned version of the struct.
    #[cfg(feature = "std")]
    pub fn into_owned(self) -> ForwardedTLV<'static> {
        ForwardedTLV {
            tlv: self.tlv.into_owned(),
            sender_identity: self.sender_identity,
        }
    }
}

/// Source of TLVs that need to be forwarded, provided to announce sender.
pub trait ForwardedTLVProvider {
    /// Should provide the next available TLV, unless it is larger than max_size
    fn next_if_smaller(&mut self, max_size: usize) -> Option<ForwardedTLV>;
}

/// Simple implementation when
#[derive(Debug, Copy, Clone)]
pub struct NoForwardedTLVs;

impl ForwardedTLVProvider for NoForwardedTLVs {
    fn next_if_smaller(&mut self, _max_size: usize) -> Option<ForwardedTLV> {
        None
    }
}

/// Identification of a packet that should be sent out.
///
/// The caller receives this from a [`PortAction::SendEvent`] and should return
/// it to the [`Port`](`super::Port`) with
/// [`Port::handle_send_timestamp`](`super::Port::handle_send_timestamp`) once
/// the transmit timestamp of that packet is known.
///
/// This type is non-copy and non-clone on purpose to ensures a single
/// [`handle_send_timestamp`](`super::Port::handle_send_timestamp`) per
/// [`SendEvent`](`PortAction::SendEvent`).
#[derive(Debug)]
pub struct TimestampContext {
    pub(super) inner: TimestampContextInner,
}

#[derive(Debug)]
pub(super) enum TimestampContextInner {
    Sync {
        id: u16,
    },
    DelayReq {
        id: u16,
    },
    PDelayReq {
        id: u16,
    },
    PDelayResp {
        id: u16,
        requestor_identity: PortIdentity,
    },
}

/// An action the [`Port`](`super::Port`) needs the user to perform
#[derive(Debug)]
#[must_use]
#[allow(missing_docs)] // Explaining the fields as well as the variants does not add value
pub enum PortAction<'a> {
    /// Send a time-critical packet
    ///
    /// Once the packet is sent and the transmit timestamp known the user should
    /// return the given [`TimestampContext`] using
    /// [`Port::handle_send_timestamp`](`super::Port::handle_send_timestamp`).
    ///
    /// Packets marked as link local should be sent per the instructions
    /// for sending peer to peer delay mechanism messages of the relevant
    /// transport specification of PTP.
    SendEvent {
        context: TimestampContext,
        data: &'a [u8],
        link_local: bool,
    },
    /// Send a general packet
    ///
    /// For a packet sent this way no timestamp needs to be captured.
    ///
    /// Packets marked as link local should be sent per the instructions
    /// for sending peer to peer delay mechanism messages of the relevant
    /// transport specification of PTP.
    SendGeneral { data: &'a [u8], link_local: bool },
    /// Call [`Port::handle_announce_timer`](`super::Port::handle_announce_timer`) in `duration` from now
    ResetAnnounceTimer { duration: core::time::Duration },
    /// Call [`Port::handle_sync_timer`](`super::Port::handle_sync_timer`) in
    /// `duration` from now
    ResetSyncTimer { duration: core::time::Duration },
    /// Call [`Port::handle_delay_request_timer`](`super::Port::handle_delay_request_timer`) in `duration` from now
    ResetDelayRequestTimer { duration: core::time::Duration },
    /// Call [`Port::handle_announce_receipt_timer`](`super::Port::handle_announce_receipt_timer`) in `duration` from now
    ResetAnnounceReceiptTimer { duration: core::time::Duration },
    /// Call [`Port::handle_filter_update_timer`](`super::Port::handle_filter_update_timer`) in `duration` from now
    ResetFilterUpdateTimer { duration: core::time::Duration },
    /// Forward this TLV to the announce timer call of all other ports.
    /// The receiver must ensure the TLV is yielded only once to the announce
    /// method of a port.
    ///
    /// This can be ignored when implementing a single port or slave only ptp
    /// instance.
    ForwardTLV { tlv: ForwardedTLV<'a> },
}

const MAX_ACTIONS: usize = 2;

/// An Iterator over [`PortAction`]s
///
/// These are returned by [`Port`](`super::Port`) when ever the library needs
/// the user to perform actions to the system.
///
/// **Guarantees to end user:** Any set of actions will only ever contain a
/// single event send
#[derive(Debug)]
#[must_use]
pub struct PortActionIterator<'a> {
    internal: Fuse<<ArrayVec<PortAction<'a>, MAX_ACTIONS> as IntoIterator>::IntoIter>,
    tlvs: TlvSetIterator<'a>,
    sender_identity: PortIdentity,
}

impl<'a> PortActionIterator<'a> {
    /// Get an empty Iterator
    ///
    /// This can for example be used to have a default value in chained `if`
    /// statements.
    pub fn empty() -> Self {
        Self {
            internal: ArrayVec::new().into_iter().fuse(),
            tlvs: TlvSetIterator::empty(),
            sender_identity: Default::default(),
        }
    }
    pub(super) fn from(list: ArrayVec<PortAction<'a>, MAX_ACTIONS>) -> Self {
        Self {
            internal: list.into_iter().fuse(),
            tlvs: TlvSetIterator::empty(),
            sender_identity: Default::default(),
        }
    }
    pub(super) fn from_filter(update: FilterUpdate) -> Self {
        if let Some(duration) = update.next_update {
            actions![PortAction::ResetFilterUpdateTimer { duration }]
        } else {
            actions![]
        }
    }
    pub(super) fn with_forward_tlvs(
        self,
        tlvs: TlvSetIterator<'a>,
        sender_identity: PortIdentity,
    ) -> Self {
        Self {
            internal: self.internal,
            tlvs,
            sender_identity,
        }
    }
}

impl<'a> Iterator for PortActionIterator<'a> {
    type Item = PortAction<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.internal.next().or_else(|| loop {
            let tlv = self.tlvs.next()?;
            if tlv.tlv_type.announce_propagate() {
                return Some(PortAction::ForwardTLV {
                    tlv: ForwardedTLV {
                        tlv,
                        sender_identity: self.sender_identity,
                    },
                });
            }
        })
    }
}
