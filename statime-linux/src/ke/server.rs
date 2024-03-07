use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use log::{debug, info, warn, LevelFilter};
use rustls::ServerConfig;
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
    sync::RwLock,
    time::Instant,
};
use tokio_rustls::{server::TlsStream, TlsAcceptor};

use super::{
    common::{load_certs, load_certs_from_files, load_private_key, Key},
    record::*,
    tls_utils::OnlyAllowedClients,
};
use crate::setup_logger;

struct KeySetStore {
    current: Key,
    next: Option<Key>,
}

impl KeySetStore {
    /// This function generates the next key if there is no next key, otherwise
    /// it moves the next key to the current key.
    fn rotate(&mut self) {
        let next_key = self.next.take();

        if let Some(next_key) = next_key {
            self.current = next_key;
            // when switched to the current key the valid since time resets
            self.current.valid_since = Instant::now();
        } else {
            self.next
                .replace(Key::generate(self.current.id + 1, rand::thread_rng()));
        }
    }

    /// Generate a new keyset that stores the current key.
    fn new() -> KeySetStore {
        KeySetStore {
            current: Key::generate(0, rand::thread_rng()),
            next: None,
        }
    }

    fn spawn_rotate_process(
        store: Arc<RwLock<Self>>,
        lifetime: Duration,
        update_period: Duration,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                // if update period is as great as lifetime we immediately rotate to have the
                // next key available
                if update_period < lifetime {
                    tokio::time::sleep(lifetime - update_period).await;
                }

                // first rotate generates the next key
                store.write().await.rotate();

                // wait for the update period, but never more than the lifetime
                tokio::time::sleep(update_period.min(lifetime)).await;

                // second rotate moves the next key to current (deleting the previous current
                // key)
                store.write().await.rotate();
            }
        })
    }
}

struct KeConfig {
    validity_period: u32,
    update_period: u32,
    grace_period: u32,
    listen_addr: SocketAddr,
    cert_chain_path: PathBuf,
    private_key_path: PathBuf,
    allowed_clients: Vec<PathBuf>,
}

async fn prep_server_config(
    cert_chain_path: impl AsRef<Path>,
    private_key_path: impl AsRef<Path>,
    allowed_clients: impl Iterator<Item = impl AsRef<Path>>,
) -> Result<ServerConfig, Box<dyn std::error::Error>> {
    let cert_chain = load_certs(cert_chain_path).await?;
    let key_der = load_private_key(private_key_path).await?;
    let allowed_clients = load_certs_from_files(allowed_clients).await?;

    // setup tls server
    let mut config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(Arc::new(OnlyAllowedClients::new(
            rustls::crypto::ring::default_provider(),
            allowed_clients,
        )))
        .with_single_cert(cert_chain, key_der)?;
    config.alpn_protocols.clear();
    config.alpn_protocols.push(b"ntske/1".to_vec());

    Ok(config)
}

pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_logger(LevelFilter::Debug).expect("could not setup logging");

    let ke_config = {
        let listen_addr = "0.0.0.0:4460";
        let cert_chain_path = "statime-linux/testkeys/test.chain.pem";
        let private_key_path = "statime-linux/testkeys/test.key";
        let allowed_clients = vec!["statime-linux/testkeys/test.chain.pem".into()];

        let listen_addr = listen_addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::from(io::ErrorKind::AddrNotAvailable))?;

        Arc::new(KeConfig {
            validity_period: 3600,
            update_period: 300,
            grace_period: 5,
            listen_addr,
            cert_chain_path: cert_chain_path.into(),
            private_key_path: private_key_path.into(),
            allowed_clients,
        })
    };

    // setup the tls server
    let config = prep_server_config(
        &ke_config.cert_chain_path,
        &ke_config.private_key_path,
        ke_config.allowed_clients.iter(),
    )
    .await?;
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind(ke_config.listen_addr).await?;

    info!("Statime-KE bound on {:?}", ke_config.listen_addr);

    // create the keyset store and let it automatically update itself
    let store = Arc::new(RwLock::new(KeySetStore::new()));
    KeySetStore::spawn_rotate_process(
        store.clone(),
        Duration::from_secs(ke_config.validity_period as u64),
        Duration::from_secs(ke_config.update_period as u64),
    );

    // handle new connections on the TCP socket and process them with the TLS
    // acceptor
    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let store = store.clone();
        let ke_config = ke_config.clone();

        debug!("Received connection from {}", peer_addr);

        // TODO: cancel the future after a timeout occurs
        let fut = async move {
            let stream = acceptor.accept(stream).await?;
            handle_connection(stream, store, ke_config).await?;

            Ok(()) as Result<(), Box<dyn std::error::Error>>
        };

        tokio::spawn(async move {
            if let Err(err) = fut.await {
                warn!("Error during connection processing: {:?}", err);
            }
        });
    }
}

async fn handle_connection(
    mut stream: TlsStream<TcpStream>,
    store: Arc<RwLock<KeySetStore>>,
    ke_config: Arc<KeConfig>,
) -> Result<(), Box<dyn std::error::Error>> {
    debug!("Attempting to read NTS-KE records from connection");

    // we expect the to receive messages to be smaller than data_buf
    let mut data_buf = vec![0; 4096];
    let mut bytes_received = 0;

    let mut records = None;
    while records.is_none() {
        bytes_received += stream.read(&mut data_buf[bytes_received..]).await?;
        let mut data = &data_buf[0..bytes_received];
        records = Record::read_until_eom(&mut data)?;
        if bytes_received == data_buf.len() && records.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "NTS message too large to handle",
            )
            .into());
        }
    }

    // records must always be filled at this point
    let Some(records) = records else {
        unreachable!()
    };

    let keyset = store.read().await;
    let resp = respond(records, &keyset, ke_config).await?;
    resp.write(stream).await?;
    Ok(())
}

async fn respond<'a>(
    records: Vec<Record<'_>>,
    keyset: &'a KeySetStore,
    ke_config: Arc<KeConfig>,
) -> Result<PtpKeyResponseMessage<'a>, Box<dyn std::error::Error>> {
    // TODO: probably send back an error message to the client instead of just
    // erroring the connection
    let request: PtpKeyRequestMessage = records.try_into()?;

    // TODO: we ignore the assocation mode entirely right now

    if !request
        .next_protocol
        .iter()
        .any(|np| *np == NextProtocol::Ptpv2_1)
    {
        // TODO: send back error instead of just erroring the connection
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Received NTS request without PTP next protocol",
        )
        .into());
    }

    let time_since = keyset.current.valid_since.elapsed().as_secs() as u32;
    let lifetime = if time_since > ke_config.validity_period {
        0
    } else {
        ke_config.validity_period - time_since
    };

    Ok(PtpKeyResponseMessage {
        next_protocol: NextProtocols::ptpv2_1(),
        current_parameters: ParameterSet {
            security_assocation: SecurityAssocation::from_key_data(
                keyset.current.id,
                keyset.current.as_bytes(),
            ),
            validity_period: ValidityPeriod {
                lifetime,
                update_period: ke_config.update_period,
                grace_period: ke_config.grace_period,
            },
        },
        next_parameters: keyset.next.as_ref().map(|next| ParameterSet {
            security_assocation: SecurityAssocation::from_key_data(next.id, next.as_bytes()),
            validity_period: ValidityPeriod {
                lifetime: ke_config.validity_period,
                update_period: ke_config.update_period,
                grace_period: ke_config.grace_period,
            },
        }),
    })
}
