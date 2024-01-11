use std::fs::Permissions;
use tokio::io::AsyncWriteExt;
use std::os::unix::prelude::PermissionsExt;
use std::path::Path;
use std::time::Instant;

use tokio::net::UnixStream;
use tokio::task::JoinHandle;

use crate::metrics::exporter::ObservableState;

pub async fn spawn(
    config: &super::config::ObservabilityConfig,
) -> JoinHandle<std::io::Result<()>> {
    let config = config.clone();
    tokio::spawn(async move {
        let result = observer(config).await;
        if let Err(ref e) = result {
            log::warn!("Abnormal termination of the state observer: {e}");
            log::warn!("The state observer will not be available");
        }
        result
    })
}

async fn observer(config: super::config::ObservabilityConfig) -> std::io::Result<()> {
    let start_time = Instant::now();

    let path = match config.observation_path {
        Some(path) => path,
        None => return Ok(()),
    };

    // this binary needs to run as root to be able to adjust the system clock.
    // by default, the socket inherits root permissions, but the client should not need
    // elevated permissions to read from the socket. So we explicitly set the permissions
    let permissions: std::fs::Permissions =
        PermissionsExt::from_mode(config.observation_permissions);

    let peers_listener = create_unix_socket_with_permissions(&path, permissions)?;

    loop {
        let (mut stream, _addr) = peers_listener.accept().await?;

        let observe = ObservableState { test: "test metric".to_string() };

        write_json(&mut stream, &observe).await?;
    }
}

fn other_error<T>(msg: String) -> std::io::Result<T> {
    use std::io::{Error, ErrorKind};
    Err(Error::new(ErrorKind::Other, msg))
}

pub fn create_unix_socket_with_permissions(
    path: &Path,
    permissions: Permissions,
) -> std::io::Result<tokio::net::UnixListener> {
    let listener = create_unix_socket(path)?;

    std::fs::set_permissions(path, permissions)?;

    Ok(listener)
}

fn create_unix_socket(path: &Path) -> std::io::Result<tokio::net::UnixListener> {
    // must unlink path before the bind below (otherwise we get "address already in use")
    if path.exists() {
        use std::os::unix::fs::FileTypeExt;

        let meta = std::fs::metadata(path)?;
        if !meta.file_type().is_socket() {
            return other_error(format!("path {path:?} exists but is not a socket"));
        }

        std::fs::remove_file(path)?;
    }

    // OS errors are terrible; let's try to do better
    let error = match tokio::net::UnixListener::bind(path) {
        Ok(listener) => return Ok(listener),
        Err(e) => e,
    };

    // we don create parent directories
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            let msg = format!(
                r"Could not create observe socket at {:?} because its parent directory does not exist",
                &path
            );
            return other_error(msg);
        }
    }

    // otherwise, just forward the OS error
    let msg = format!(
        "Could not create observe socket at {:?}: {:?}",
        &path, error
    );

    other_error(msg)
}

pub async fn write_json<T>(stream: &mut UnixStream, value: &T) -> std::io::Result<()>
where
    T: serde::Serialize,
{
    let bytes = serde_json::to_vec(value).unwrap();
    stream.write_all(&bytes).await
}
