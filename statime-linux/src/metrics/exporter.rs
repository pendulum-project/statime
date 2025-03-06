use std::path::{Path, PathBuf};

use clap::Parser;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UnixStream},
};

use super::format::format_response;
use crate::{initialize_logging_parse_config, observer::ObservableInstanceState};

#[derive(Debug, Serialize, Deserialize)]
pub struct ObservableState {
    pub program: ProgramData,
    pub instance: ObservableInstanceState,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProgramData {
    pub version: String,
    pub build_commit: String,
    pub build_commit_date: String,
    pub uptime_seconds: f64,
}

impl ProgramData {
    pub fn with_uptime(uptime_seconds: f64) -> ProgramData {
        ProgramData {
            uptime_seconds,
            ..Default::default()
        }
    }
}

impl Default for ProgramData {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            build_commit: env!("STATIME_GIT_REV").to_owned(),
            build_commit_date: env!("STATIME_GIT_DATE").to_owned(),
            uptime_seconds: 0.0,
        }
    }
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub(crate) struct Args {
    /// Configuration file to use
    #[clap(
        long = "config",
        short = 'c',
        default_value = "/etc/statime/statime.toml"
    )]
    config: PathBuf,
}

pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Args::parse();

    let config = initialize_logging_parse_config(&options.config);

    let observation_socket_path = match config.observability.observation_path {
        Some(path) => path,
        None => {
            eprintln!(
                "An observation socket path must be configured using the observation-path option \
                 in the [observability] section of the configuration"
            );
            std::process::exit(1);
        }
    };

    println!(
        "starting statime-metrics-exporter on {}",
        &config.observability.metrics_exporter_listen
    );

    let listener = TcpListener::bind(&config.observability.metrics_exporter_listen).await?;
    let mut buf = String::with_capacity(4 * 1024);

    loop {
        let (mut tcp_stream, _) = listener.accept().await?;

        // Wait until a request was sent, dropping the bytes read when this scope ends
        // to ensure we don't accidentally use them afterwards
        {
            // Receive all data until the header was fully received, or until max buf size
            let mut buf = [0u8; 2048];
            let mut bytes_read = 0;
            loop {
                bytes_read += tcp_stream.read(&mut buf[bytes_read..]).await?;

                // The headers end with two CRLFs in a row
                if buf[0..bytes_read].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }

                // Headers should easily fit within the buffer
                // If we have not found the end yet, we are not going to
                if bytes_read >= buf.len() {
                    tracing::warn!("Metrics connection request too long");
                    continue;
                }
            }

            // We only respond to GET requests
            if !buf[0..bytes_read].starts_with(b"GET ") {
                tracing::warn!("Metrics connection wasn't get");
                continue;
            }
        }

        buf.clear();
        match handler(&mut buf, &observation_socket_path).await {
            Ok(()) => {
                tcp_stream.write_all(buf.as_bytes()).await?;
            }
            Err(e) => {
                log::warn!("error: {e}");
                const ERROR_REPONSE: &str = concat!(
                    "HTTP/1.1 500 Internal Server Error\r\n",
                    "content-type: text/plain\r\n",
                    "content-length: 0\r\n\r\n",
                );

                tcp_stream.write_all(ERROR_REPONSE.as_bytes()).await?;
            }
        }
    }
}

pub async fn read_json<'a, T>(
    stream: &mut UnixStream,
    buffer: &'a mut Vec<u8>,
) -> std::io::Result<T>
where
    T: serde::Deserialize<'a>,
{
    buffer.clear();

    let n = stream.read_buf(buffer).await?;
    buffer.truncate(n);
    serde_json::from_slice(buffer)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
}

async fn handler(buf: &mut String, observation_socket_path: &Path) -> std::io::Result<()> {
    let mut stream = tokio::net::UnixStream::connect(observation_socket_path).await?;
    let mut msg = Vec::with_capacity(16 * 1024);
    let observable_state: ObservableState = read_json(&mut stream, &mut msg).await?;

    format_response(buf, &observable_state)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "formatting error"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::Parser;

    use crate::metrics::exporter::Args;

    const BINARY: &str = "/usr/bin/statime-metrics-exporter";

    #[test]
    fn cli_config() {
        let config_str = "/foo/bar/statime.toml";
        let config = Path::new(config_str);
        let arguments = &[BINARY, "-c", config_str];

        let options = Args::try_parse_from(arguments).unwrap();
        assert_eq!(options.config.as_path(), config);
    }
}
