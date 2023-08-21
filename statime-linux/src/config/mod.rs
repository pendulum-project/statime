use std::{os::unix::fs::PermissionsExt, path::PathBuf, fs::read_to_string};
use log::warn;
use serde::Deserialize;
use statime::{Interval, Duration, DelayMechanism};
use thiserror::Error;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Config {
    pub loglevel: String,
    pub sdo_id: u16,
    pub domain: u8,
    pub priority1: u8,
    pub priority2: u8,
    pub hardware_clock: Option<String>,
    pub ports: Vec<PortConfig>,
}

#[derive(Deserialize, Debug)]
pub struct PortConfig {
    pub interface: String,
    pub announce_interval: i8,
    pub sync_interval: i8,
    pub announce_receipt_timeout: u8,
    pub master_only: bool,
    pub delay_asymetry: i64,
    pub delay_mechanism: i8,
}

impl From<PortConfig> for statime::PortConfig {
    fn from(pc: PortConfig) -> Self {
        Self {
            announce_interval: Interval::from_log_2(pc.announce_interval),
            sync_interval: Interval::from_log_2(pc.sync_interval),
            announce_receipt_timeout: pc.announce_receipt_timeout,
            master_only: pc.master_only,
            delay_asymmetry: Duration::from_nanos(pc.delay_asymetry),
            delay_mechanism: DelayMechanism::E2E { interval: Interval::from_log_2(pc.delay_mechanism) },
        }
    }
}

#[derive(Deserialize, Debug)]
pub enum PtpMode {
    Ordinary,
    Boundary,
    Transparant,
}

impl Config {

    /// Parse config from file
    pub fn from_file(file: PathBuf) -> Result<Config, ConfigError> {
        let meta = std::fs::metadata(&file).unwrap();
        let perm = meta.permissions();

        if perm.mode() as libc::mode_t & libc::S_IWOTH != 0 {
            warn!("Unrestricted config file permissions: Others can write.");
        }

        let contents = read_to_string(file)?;
        let config: Config = toml::de::from_str(&contents)?;
        config.warn_when_unreasonable();
        Ok(config)
    }

    /// Warns about unreasonable config values
    pub fn warn_when_unreasonable(&self) {
        if self.ports.is_empty() {
            warn!("No ports configured.");
        }

        if self.ports.len() > 16 {
            warn!("Too many ports are configured.");
        }
    }
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("io error while reading config: {0}")]
    Io(#[from] std::io::Error),
    #[error("config toml parsing error: {0}")]
    Toml(#[from] toml::de::Error),
}
