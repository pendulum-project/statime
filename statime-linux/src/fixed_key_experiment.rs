use std::{collections::HashMap, sync::{Arc, Mutex}};

use statime::crypto::{HmacSha256_128, SecurityAssociation, SecurityAssociationProvider, SecurityPolicy, SenderIdentificaton};

pub struct FixedKeyAssociation {
    mac: HmacSha256_128,
    sequence_data: Arc<Mutex<HashMap<(u32, SenderIdentificaton),u16>>>,
}

impl SecurityAssociation for FixedKeyAssociation {
    fn policy_data(&self) -> statime::crypto::SecurityPolicy {
        SecurityPolicy {
            ignore_correction: true,
        }
    }

    fn mac(&self, key_id: u32) -> Option<&dyn statime::crypto::Mac> {
        if key_id == 0 {
            Some(&self.mac)
        } else {
            None
        }
    }

    fn signing_mac(&self) -> (u32, &dyn statime::crypto::Mac) {
        (0, &self.mac)
    }

    fn register_sequence_id(
        &mut self,
        key_id: u32,
        sender: statime::crypto::SenderIdentificaton,
        sequence_id: u16,
    ) -> bool {
        let mut sequence_data = self.sequence_data.lock().unwrap();
        match sequence_data.entry((key_id, sender)) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let delta = sequence_id.wrapping_sub(*entry.get()) as i16;
                if delta <= 0 || delta > 256 {
                    false
                } else {
                    *entry.get_mut() = sequence_id;
                    true
                }
            },
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(sequence_id);
                true
            },
        }
    }
}

#[derive(Default, Debug, Clone)]
pub struct FixedKeyProvider {
    sequence_data: Arc<Mutex<HashMap<(u32, SenderIdentificaton),u16>>>,
}

impl SecurityAssociationProvider for FixedKeyProvider {
    type Association = FixedKeyAssociation;

    fn lookup(&self, spp: u8) -> Option<Self::Association> {
        if spp == 0 {
            Some(FixedKeyAssociation {
                mac: HmacSha256_128::new([0;32]),
                sequence_data: self.sequence_data.clone(),
            })
        } else {
            None
        }
    }
}
