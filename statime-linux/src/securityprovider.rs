use std::{collections::HashMap, error::Error, path::PathBuf, sync::Arc};

use rand::Rng;
use rustls::{ClientConfig, RootCertStore};
use statime::crypto::{
    HmacSha256_128, SecurityAssociation, SecurityAssociationProvider, SecurityPolicy,
    SenderIdentificaton,
};

use crate::ke::{
    client::fetch_data,
    common::{load_certs, load_private_key},
};

#[derive(Clone)]
pub struct NTSProvider {
    associations: Arc<HashMap<u8, Arc<std::sync::Mutex<NTSAssociationInner>>>>,
}

impl NTSProvider {
    pub fn empty() -> Self {
        NTSProvider {
            associations: Arc::new(HashMap::new()),
        }
    }

    pub async fn new(
        server_name: String,
        client_key: PathBuf,
        client_cert: PathBuf,
        server_root: PathBuf,
    ) -> Result<(Self, u8), Box<dyn Error>> {
        let client_key = load_private_key(client_key).await?;
        let client_chain = load_certs(client_cert).await?;
        let mut root_store = RootCertStore::empty();
        for cert in load_certs(server_root).await?.into_iter() {
            root_store.add(cert)?;
        }

        let config = Arc::new(
            ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_client_auth_cert(client_chain, client_key)?,
        );

        let initial_data = fetch_data(&server_name, config.clone()).await?;
        let now = std::time::Instant::now();

        let mut keys = HashMap::new();
        keys.insert(
            initial_data.current_parameters.security_assocation.key_id,
            NTSKey {
                key: statime::crypto::HmacSha256_128::new(
                    initial_data
                        .current_parameters
                        .security_assocation
                        .key
                        .as_ref()
                        .try_into()?,
                ),
                valid_till: now
                    + std::time::Duration::from_secs(
                        initial_data.current_parameters.validity_period.lifetime as _,
                    ),
            },
        );
        let next_key = if let Some(next_parameters) = initial_data.next_parameters {
            keys.insert(
                next_parameters.security_assocation.key_id,
                NTSKey {
                    key: statime::crypto::HmacSha256_128::new(
                        next_parameters
                            .security_assocation
                            .key
                            .as_ref()
                            .try_into()?,
                    ),
                    valid_till: now
                        + std::time::Duration::from_secs(
                            next_parameters.validity_period.lifetime as _,
                        ),
                },
            );
            Some(next_parameters.security_assocation.key_id)
        } else {
            None
        };

        let association = Arc::new(std::sync::Mutex::new(NTSAssociationInner {
            grace_period: std::time::Duration::from_secs(
                initial_data.current_parameters.validity_period.grace_period as _,
            ),
            transition_period: std::time::Duration::from_secs(
                initial_data
                    .current_parameters
                    .validity_period
                    .update_period as _,
            ),
            keys,
            current_key: initial_data.current_parameters.security_assocation.key_id,
            next_key,
            sequence_ids: Default::default(),
        }));

        let spp = initial_data.current_parameters.security_assocation.spp;

        tokio::spawn(assocation_manager(association.clone(), server_name, config));

        let mut associations = HashMap::new();
        associations.insert(spp, association);

        Ok((
            NTSProvider {
                associations: Arc::new(associations),
            },
            spp,
        ))
    }
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
async fn assocation_manager(
    association: Arc<std::sync::Mutex<NTSAssociationInner>>,
    server_name: String,
    config: Arc<ClientConfig>,
) {
    loop {
        let deadline = {
            let mut this = association.lock().unwrap();
            this.clean_keys();
            this.clean_sequence_id();

            // wait for new parameters
            if this.next_key.is_none() {
                let transition_start =
                    this.keys[&this.current_key].valid_till - this.transition_period;
                let random_offset = this
                    .transition_period
                    .mul_f32(rand::thread_rng().gen_range(0.0..0.75));
                Some(transition_start + random_offset)
            } else {
                None
            }
        };
        let await_data = if let Some(deadline) = deadline {
            tokio::time::sleep_until(deadline.into()).await;
            let params = fetch_data(&server_name, config.clone()).await.unwrap();
            let now = std::time::Instant::now();
            Some((params, now))
        } else {
            None
        };

        let deadline = {
            let mut this = association.lock().unwrap();
            if let Some((params, now)) = await_data {
                let next_params = params.next_parameters.unwrap();
                this.keys.insert(
                    next_params.security_assocation.key_id,
                    NTSKey {
                        key: HmacSha256_128::new(
                            next_params
                                .security_assocation
                                .key
                                .as_ref()
                                .try_into()
                                .unwrap(),
                        ),
                        valid_till: now
                            + std::time::Duration::from_secs(
                                next_params.validity_period.lifetime as _,
                            ),
                    },
                );
                this.next_key = Some(next_params.security_assocation.key_id);
            }

            this.keys[&this.current_key].valid_till
        };

        tokio::time::sleep_until(deadline.into()).await;
        let deadline = {
            let mut this = association.lock().unwrap();
            let old_key = this.current_key;
            this.current_key = this.next_key.take().unwrap();

            // wait out grace period
            this.keys[&old_key].valid_till + this.grace_period
        };
        tokio::time::sleep_until(deadline.into()).await;
    }
}
