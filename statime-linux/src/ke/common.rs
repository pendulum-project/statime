use std::{io, path::Path};

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::time::Instant;

pub struct Key {
    pub id: u32,
    data: [u8; 32],
    pub valid_since: Instant,
}

impl Key {
    pub fn generate(id: u32, mut rng: impl rand::Rng) -> Key {
        let mut data = [0; 32];
        rng.fill_bytes(&mut data);

        Key {
            id,
            data,
            valid_since: Instant::now(),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }
}

pub async fn load_certs(path: impl AsRef<Path>) -> io::Result<Vec<CertificateDer<'static>>> {
    let cert_chain_data = tokio::fs::read(path).await?;
    rustls_pemfile::certs(&mut &cert_chain_data[..]).collect()
}

pub async fn load_private_key(path: impl AsRef<Path>) -> io::Result<PrivateKeyDer<'static>> {
    let private_key_data = tokio::fs::read(path).await?;
    rustls_pemfile::private_key(&mut &private_key_data[..])?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "No private key data found"))
}

pub async fn load_certs_from_files(it: impl Iterator<Item = impl AsRef<Path>>) -> io::Result<Vec<CertificateDer<'static>>> {
    let mut certs = vec![];
    for p in it {
        certs.push(
            load_certs(p)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "No certificate found in file"))?
        );
    }
    Ok(certs)
}

