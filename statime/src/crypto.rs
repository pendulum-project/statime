//! Provides primitives for cryptographic validation of PTP messages

/// Error representing a Mac running out of space for its output
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct NoSpaceError;

/// Message authentication code provider
pub trait Mac {
    /// Size of the output
    fn output_size(&self) -> usize;

    /// Verify that the given mac represents a signature for the given data
    fn verify(&self, data: &[u8], mac: &[u8]) -> bool;

    /// Sign the given data
    fn sign(&self, data: &[u8], output_buffer: &mut [u8]) -> Result<usize, NoSpaceError>;
}

#[cfg(feature = "ring")]
mod ring;
#[cfg(feature = "ring")]
pub use ring::*;

use crate::datastructures::{common::PortIdentity, messages::MessageType};

/// Policy data for a ptp security association (see IEEE1588-2019 section 16.14)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityPolicy {
    /// Whether or not the correction field is signed
    pub ignore_correction: bool,
}

/// Identification of a sender for sequence id generation
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SenderIdentificaton {
    pub(crate) message_type: MessageType,
    pub(crate) source_port_id: PortIdentity,
}

/// Data for a ptp security association (see IEEE1588-2019 section 16.14)
pub trait SecurityAssociation {
    /// Security policy for the assocation
    fn policy_data(&self) -> SecurityPolicy;

    /// Lookup a specific key for the association
    fn mac(&self, key_id: u32) -> Option<&dyn Mac>;

    /// Register an observed message sequence id, returning whether it
    /// is acceptable for further processing.
    ///
    /// Per the specification, once a sequence_id has been seen for a
    /// combination of sender and key_id, only higher sequence_id's must be
    /// accepted, allowing for rollover of the sequence id. Typically, this
    /// should check whether the signed difference between the last sequence
    /// id and the provided sequence id is larger than 0 and smaller than
    /// some configured limit. Note that it should only actually register
    /// id's if they are acceptable, as otherwise an attacker can still
    /// avoid the checks by first sending an even older sequence id than
    /// what he wants the instance to accept.
    fn register_sequence_id(
        &mut self,
        key_id: u32,
        sender: SenderIdentificaton,
        sequence_id: u16,
    ) -> bool;

    /// Get key that should be used for signing
    fn signing_mac(&self) -> (u32, &dyn Mac);
}

/// Interface to the database of security associations
pub trait SecurityAssociationProvider {
    /// Type used for the security assocations
    type Association<'a>: SecurityAssociation
    where
        Self: 'a;

    /// Lookup a specific security association
    fn lookup(&self, spp: u8) -> Option<Self::Association<'_>>;
}

/// Association type for the empty security association provider
pub enum NoSecurityAssocation {}

impl SecurityAssociation for NoSecurityAssocation {
    fn policy_data(&self) -> SecurityPolicy {
        unreachable!()
    }

    fn mac(&self, _key_id: u32) -> Option<&dyn Mac> {
        unreachable!()
    }

    fn register_sequence_id(
        &mut self,
        _key_id: u32,
        _sender: SenderIdentificaton,
        _sequence_id: u16,
    ) -> bool {
        unreachable!()
    }

    fn signing_mac(&self) -> (u32, &dyn Mac) {
        unreachable!()
    }
}

/// Empty security association provider
pub struct NoSecurityProvider;

impl SecurityAssociationProvider for NoSecurityProvider {
    type Association<'a> = NoSecurityAssocation;

    fn lookup(&self, _spp: u8) -> Option<Self::Association<'_>> {
        None
    }
}
