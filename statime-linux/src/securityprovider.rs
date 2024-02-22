use std::{collections::HashMap, sync::Arc};

use rand::Rng;
use statime::crypto::{
    SecurityAssociation, SecurityAssociationProvider, SecurityPolicy, SenderIdentificaton,
};

pub struct NTSProvider {
    associations: Arc<HashMap<u8, Arc<std::sync::Mutex<NTSAssociationInner>>>>,
}

struct NTSAssociationInner {
    grace_period: std::time::Duration,
    transition_period: std::time::Duration,
    keys: HashMap<u32, NTSKey>,
    current_key: u32,
    next_key: Option<u32>,
    sequence_ids: HashMap<(SenderIdentificaton, u32), u16>,
}

pub struct NTSAssociation<'a>(std::sync::MutexGuard<'a, NTSAssociationInner>);

struct NTSKey {
    key: statime::crypto::HmacSha256_128,
    valid_till: std::time::Instant,
}

impl SecurityAssociationProvider for NTSProvider {
    type Association<'a> = NTSAssociation<'a>;

    fn lookup(&self, spp: u8) -> Option<Self::Association<'_>> {
        self.associations
            .get(&spp)
            .map(|a| NTSAssociation(a.lock().unwrap()))
    }
}

impl<'a> SecurityAssociation for NTSAssociation<'a> {
    fn policy_data(&self) -> statime::crypto::SecurityPolicy {
        SecurityPolicy {
            ignore_correction: false,
        }
    }

    fn mac(&self, key_id: u32) -> Option<&dyn statime::crypto::Mac> {
        self.0.keys.get(&key_id).and_then(|key| {
            if std::time::Instant::now() < key.valid_till + self.0.grace_period {
                Some(&key.key as &dyn statime::crypto::Mac)
            } else {
                None
            }
        })
    }

    fn register_sequence_id(
        &mut self,
        key_id: u32,
        sender: statime::crypto::SenderIdentificaton,
        sequence_id: u16,
    ) -> bool {
        match self.0.sequence_ids.entry((sender, key_id)) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if (*entry.get() - sequence_id) as i16 > 0 {
                    *entry.get_mut() = sequence_id;
                    true
                } else {
                    false
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(sequence_id);
                true
            }
        }
    }

    fn signing_mac(&self) -> (u32, &dyn statime::crypto::Mac) {
        (
            self.0.current_key,
            self.0
                .keys
                .get(&self.0.current_key)
                .map(|k| &k.key)
                .unwrap(),
        )
    }
}

impl NTSAssociationInner {
    fn clean_sequence_id(&mut self) {
        self.sequence_ids
            .retain(|(_, key_id), _| self.keys.contains_key(key_id))
    }

    fn clean_keys(&mut self) {
        let expired = std::time::Instant::now() - self.grace_period;
        self.keys
            .retain(|_, NTSKey { valid_till, .. }| *valid_till > expired);
    }
}

#[allow(unused)]
// await_holding_lock is somewhat buggy (see https://github.com/rust-lang/rust-clippy/issues/9683)
#[allow(clippy::await_holding_lock)]
async fn assocation_manager(association: Arc<std::sync::Mutex<NTSAssociationInner>>) {
    let mut this = association.lock().unwrap();

    loop {
        // wait for new parameters
        if this.next_key.is_none() {
            let transition_start = this.keys[&this.current_key].valid_till - this.transition_period;
            let random_offset = this
                .transition_period
                .mul_f32(rand::thread_rng().gen_range(0.0..0.75));
            drop(this);
            tokio::time::sleep_until((transition_start + random_offset).into()).await;
            this = association.lock().unwrap();
            // TODO: Fetch new parameters
        }

        // switchover
        let switchover_time = this.keys[&this.current_key].valid_till;

        drop(this);
        tokio::time::sleep_until(switchover_time.into()).await;
        this = association.lock().unwrap();
        let old_key = this.current_key;
        this.current_key = this.next_key.take().unwrap();

        // wait out grace period
        let grace_end = this.keys[&old_key].valid_till + this.grace_period;
        drop(this);
        tokio::time::sleep_until(grace_end.into()).await;
        this = association.lock().unwrap();
        this.clean_keys();
        this.clean_sequence_id();
    }
}
