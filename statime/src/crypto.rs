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

/// Policy data for a ptp security association (see IEEE1588-2019 section 16.14)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityPolicy {
    /// Whether or not the correction field is signed
    pub ignore_correction: bool,
}

/// Data for a ptp security association (see IEEE1588-2019 section 16.14)
pub trait SecurityAssociation {
    /// Security policy for the assocation
    fn policy_data(&self) -> SecurityPolicy;

    /// Lookup a specific key for the association
    fn mac(&self, key_id: u32) -> Option<&dyn Mac>;

    /// Get key that should be used for signing
    fn signing_mac(&self) -> (u32, &dyn Mac);
}

/// Interface to the database of security associations
pub trait SecurityAssociationProvider {
    /// Type used for the security assocations
    type Association: SecurityAssociation;

    /// Lookup a specific security association
    fn lookup(&self, spp: u8) -> Option<Self::Association>;
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

    fn signing_mac(&self) -> (u32, &dyn Mac) {
        unreachable!()
    }
}

/// Empty security association provider
pub struct NoSecurityProvider;

impl SecurityAssociationProvider for NoSecurityProvider {
    type Association = NoSecurityAssocation;

    fn lookup(&self, _spp: u8) -> Option<Self::Association> {
        None
    }
}
