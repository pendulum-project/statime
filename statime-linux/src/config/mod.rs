use std::{
    fs::read_to_string,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use log::warn;
use serde::{Deserialize, Deserializer};
use statime::{
    config::{ClockIdentity, DelayMechanism},
    time::{Duration, Interval},
};
use timestamped_socket::interface::InterfaceName;

use crate::tracing::LogLevel;

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub loglevel: LogLevel,
    #[serde(default = "default_sdo_id")]
    pub sdo_id: u16,
    #[serde(default = "default_domain")]
    pub domain: u8,
    #[serde(default, deserialize_with = "deserialize_clock_identity")]
    pub identity: Option<ClockIdentity>,
    #[serde(default = "default_priority1")]
    pub priority1: u8,
    #[serde(default = "default_priority2")]
    pub priority2: u8,
    #[serde(default)]
    pub path_trace: bool,
    #[serde(rename = "port")]
    pub ports: Vec<PortConfig>,
    #[serde(default)]
    pub observability: ObservabilityConfig,
    #[serde(default)]
    pub virtual_system_clock: bool,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct PortConfig {
    pub interface: InterfaceName,
    #[serde(default, deserialize_with = "deserialize_acceptable_master_list")]
    pub acceptable_master_list: Option<Vec<ClockIdentity>>,
    #[serde(default)]
    pub hardware_clock: Option<u32>,
    #[serde(default)]
    pub network_mode: NetworkMode,
    #[serde(default = "default_announce_interval")]
    pub announce_interval: i8,
    #[serde(default = "default_sync_interval")]
    pub sync_interval: i8,
    #[serde(default = "default_announce_receipt_timeout")]
    pub announce_receipt_timeout: u8,
    #[serde(default)]
    pub master_only: bool,
    #[serde(default = "default_delay_asymmetry")]
    pub delay_asymmetry: i64,
    #[serde(default)]
    pub delay_mechanism: DelayType,
    #[serde(default = "default_delay_interval")]
    pub delay_interval: i8,
}

fn deserialize_acceptable_master_list<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<ClockIdentity>>, D::Error>
where
    D: Deserializer<'de>,
{
    use hex::FromHex;
    use serde::de::Error;

    let raw: Vec<String> = Deserialize::deserialize(deserializer)?;
    let mut result = Vec::with_capacity(raw.len());

    for identity in raw {
        result.push(ClockIdentity(<[u8; 8]>::from_hex(identity).map_err(
            |e| D::Error::custom(format!("Invalid clock identifier: {}", e)),
        )?));
    }

    Ok(Some(result))
}

fn deserialize_clock_identity<'de, D>(deserializer: D) -> Result<Option<ClockIdentity>, D::Error>
where
    D: Deserializer<'de>,
{
    use hex::FromHex;
    use serde::de::Error;
    let raw: String = Deserialize::deserialize(deserializer)?;
    Ok(Some(ClockIdentity(<[u8; 8]>::from_hex(raw).map_err(
        |e| D::Error::custom(format!("Invalid clock identifier: {}", e)),
    )?)))
}

impl From<PortConfig> for statime::config::PortConfig<Option<Vec<ClockIdentity>>> {
    fn from(pc: PortConfig) -> Self {
        Self {
            acceptable_master_list: pc.acceptable_master_list,
            announce_interval: Interval::from_log_2(pc.announce_interval),
            sync_interval: Interval::from_log_2(pc.sync_interval),
            announce_receipt_timeout: pc.announce_receipt_timeout,
            master_only: pc.master_only,
            delay_asymmetry: Duration::from_nanos(pc.delay_asymmetry),
            delay_mechanism: match pc.delay_mechanism {
                DelayType::E2E => DelayMechanism::E2E {
                    interval: Interval::from_log_2(pc.delay_interval),
                },
                DelayType::P2P => DelayMechanism::P2P {
                    interval: Interval::from_log_2(pc.delay_interval),
                },
            },
        }
    }
}

#[derive(Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    #[default]
    Ipv4,
    Ipv6,
    Ethernet,
}

#[derive(Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum DelayType {
    #[default]
    E2E,
    P2P,
}

impl Config {
    /// Parse config from file
    pub fn from_file(file: &Path) -> Result<Config, ConfigError> {
        let meta = std::fs::metadata(file).map_err(ConfigError::Io)?;
        let perm = meta.permissions();

        if perm.mode() as libc::mode_t & libc::S_IWOTH != 0 {
            warn!("Unrestricted config file permissions: Others can write.");
        }

        let contents = read_to_string(file).map_err(ConfigError::Io)?;
        let config: Config = toml::de::from_str(&contents).map_err(ConfigError::Toml)?;
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

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Toml(toml::de::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => writeln!(f, "io error while reading config: {e}"),
            ConfigError::Toml(e) => writeln!(f, "config toml parsing error: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

fn default_domain() -> u8 {
    0
}

fn default_sdo_id() -> u16 {
    0x000
}

fn default_announce_interval() -> i8 {
    1
}

fn default_sync_interval() -> i8 {
    0
}

fn default_announce_receipt_timeout() -> u8 {
    3
}

fn default_priority1() -> u8 {
    128
}

fn default_priority2() -> u8 {
    128
}

fn default_delay_asymmetry() -> i64 {
    0
}

fn default_delay_interval() -> i8 {
    0
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ObservabilityConfig {
    #[serde(default)]
    pub observation_path: Option<PathBuf>,
    #[serde(default = "default_observation_permissions")]
    pub observation_permissions: u32,
    #[serde(default = "default_metrics_exporter_listen")]
    pub metrics_exporter_listen: SocketAddr,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            observation_path: Default::default(),
            observation_permissions: default_observation_permissions(),
            metrics_exporter_listen: default_metrics_exporter_listen(),
        }
    }
}

const fn default_observation_permissions() -> u32 {
    0o666
}

fn default_metrics_exporter_listen() -> SocketAddr {
    "127.0.0.1:9975".parse().unwrap()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use timestamped_socket::interface::InterfaceName;

    use crate::{config::ObservabilityConfig, tracing::LogLevel};

    // Minimal amount of config results in default values
    #[test]
    fn minimal_config() {
        const MINIMAL_CONFIG: &str = r#"
[[port]]
interface = "enp0s31f6"
"#;

        let expected_port = crate::config::PortConfig {
            interface: InterfaceName::from_str("enp0s31f6").unwrap(),
            acceptable_master_list: None,
            hardware_clock: None,
            network_mode: crate::config::NetworkMode::Ipv4,
            announce_interval: 1,
            sync_interval: 0,
            announce_receipt_timeout: 3,
            master_only: false,
            delay_asymmetry: 0,
            delay_mechanism: crate::config::DelayType::E2E,
            delay_interval: 0,
        };

        let expected = crate::config::Config {
            loglevel: LogLevel::Info,
            sdo_id: 0x000,
            domain: 0,
            identity: None,
            priority1: 128,
            priority2: 128,
            path_trace: false,
            ports: vec![expected_port],
            observability: ObservabilityConfig::default(),
            virtual_system_clock: false,
        };

        let actual = toml::from_str(MINIMAL_CONFIG).unwrap();

        assert_eq!(expected, actual);
    }
}
