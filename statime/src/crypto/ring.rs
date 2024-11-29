use ring::hmac;

use super::Mac;

/// Implementation of the HMAC-SHA256-128 required by the PTP standard
pub struct HmacSha256_128 {
    key: ring::hmac::Key,
}

impl HmacSha256_128 {
    /// Create a new keyed instance of the Mac.
    pub fn new(key: [u8; 32]) -> Self {
        Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, &key),
        }
    }
}

impl Mac for HmacSha256_128 {
    fn output_size(&self) -> usize {
        16
    }

    fn verify(&self, data: &[u8], mac: &[u8]) -> bool {
        // because of truncation, regeneration is our only path to verification
        let tag = hmac::sign(&self.key, data);
        ring::constant_time::verify_slices_are_equal(mac, &tag.as_ref()[..16]).is_ok()
    }

    fn sign(&self, data: &[u8], output_buffer: &mut [u8]) -> Result<usize, super::NoSpaceError> {
        if output_buffer.len() < 16 {
            return Err(super::NoSpaceError);
        }
        let tag = hmac::sign(&self.key, data);
        output_buffer[..16].copy_from_slice(&tag.as_ref()[..16]);
        Ok(16)
    }
}
