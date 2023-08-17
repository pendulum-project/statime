use std::{os::unix::fs::PermissionsExt, path::Path};
use log::warn;
use serde::Deserialize;
use thiserror::Error;
use tokio::{fs::read_to_string, io};

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Config {
    pub loglevel: String,
    pub mode: PtpMode,
    pub sdo_id: String,
    pub domain: u8,
    pub priority1: u8,
    pub priority2: u8,
    pub hardware_clock: Option<String>,
    pub ports: Vec<PortConfig>,
}

#[derive(Deserialize, Debug)]
pub enum PtpMode {
    Ordinary,
    Boundary,
    Transparant,
}

#[derive(Deserialize, Debug)]
pub struct PortConfig {
    pub interface: String,
    pub announce_interval: i8,
    pub sync_interval: i8,
    pub announce_receipt_timeout: u8,
    pub master_only: bool,
    pub slave_only: bool,
}

impl Config {

    /// Parse config from file
    pub async fn from_file(file: impl AsRef<Path>) -> Result<Config, ConfigError> {
        let meta = std::fs::metadata(&file).unwrap();
        let perm = meta.permissions();

        if perm.mode() as libc::mode_t & libc::S_IWOTH != 0 {
            warn!("Unrestricted config file permissions: Others can write.");
        }

        let contents = read_to_string(file).await?;
        Ok( toml::de::from_str(&contents).unwrap()  )
    }

    /// Check that the config is reasonable
    pub fn check(&self) -> bool {
        let mut ok = true;

        if self.ports.len() < 1 {
            warn!("No ports configured.");
            ok = false;
        }

        if self.ports.len() > 16 {
            warn!("Too many ports are configured.");
            ok = false;
        }

        ok
    }
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("io error while reading config: {0}")]
    Io(#[from] io::Error),
    #[error("config toml parsing error: {0}")]
    Toml(#[from] toml::de::Error),
}
