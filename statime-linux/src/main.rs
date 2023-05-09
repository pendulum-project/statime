use clap::Parser;
use fern::colors::Color;
use statime::datastructures::common::{PortIdentity, TimeSource};
use statime::datastructures::datasets::{DefaultDS, DelayMechanism, PortDS, TimePropertiesDS};
use statime::datastructures::messages::SdoId;
use statime::port::Port;
use statime::{
    datastructures::common::ClockIdentity, filters::basic::BasicFilter, ptp_instance::PtpInstance,
};
use statime_linux::network::linux::{LinuxNetworkPort, Ports, TimestampingMode};
use statime_linux::{
    clock::{LinuxClock, LinuxTimer, RawLinuxClock},
    network::linux::{get_clock_id, InterfaceDescriptor, LinuxRuntime},
};

#[derive(Clone, Copy)]
struct SdoIdParser;

impl clap::builder::TypedValueParser for SdoIdParser {
    type Value = SdoId;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        use clap::error::{ContextKind, ContextValue, ErrorKind};

        let inner = clap::value_parser!(u16);
        let val = inner.parse_ref(cmd, arg, value)?;

        match SdoId::new(val) {
            None => {
                let mut err = clap::Error::new(ErrorKind::ValueValidation).with_cmd(cmd);
                if let Some(arg) = arg {
                    err.insert(
                        ContextKind::InvalidArg,
                        ContextValue::String(arg.to_string()),
                    );
                }
                err.insert(
                    ContextKind::InvalidValue,
                    ContextValue::String(val.to_string()),
                );
                Err(err)
            }
            Some(v) => Ok(v),
        }
    }
}

#[derive(clap::Args, Debug, Clone, Copy)]
struct PortDSConfig {
    /// Log value of interval expected between announce messages, see: 7.7.2.2
    /// Default init value is 1, see: A.9.4.2
    #[clap(long, default_value_t = 1)]
    log_announce_interval: i8,

    /// Time interval between Sync messages, see: 7.7.2.3
    /// Default init value is 0, see: A.9.4.2
    #[clap(long, default_value_t = 0)]
    log_sync_interval: i8,

    /// Default init value is 3, see: A.9.4.2
    #[clap(long, default_value_t = 3)]
    announce_receipt_timeout: u8,
}

#[derive(clap::Args, Debug, Clone, Copy)]
struct DefaultDSConfig {
    /// Local clock priority (part 1) used in master clock selection
    /// Default init value is 128, see: A.9.4.2
    #[clap(long, default_value_t = 255)]
    priority_1: u8,

    /// Local clock priority (part 2) used in master clock selection
    /// Default init value is 128, see: A.9.4.2
    #[clap(long, default_value_t = 255)]
    priority_2: u8,

    /// The SDO id of the desired ptp domain
    #[clap(long, default_value_t = SdoId::default(), value_parser = SdoIdParser)]
    sdo: SdoId,

    /// The domain number of the desired ptp domain
    #[clap(long, default_value_t = 0)]
    domain: u8,

    #[clap(default_value_t = false)]
    slave_only: bool,
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Set desired logging level
    #[clap(short, long, default_value_t = log::LevelFilter::Info)]
    loglevel: log::LevelFilter,

    /// Set interface on which to listen to PTP messages
    #[clap(short, long)]
    interface: InterfaceDescriptor,

    #[command(flatten)]
    default_ds_config: DefaultDSConfig,

    #[command(flatten)]
    port_ds_config: PortDSConfig,

    /// Use hardware clock
    #[clap(long, short = 'c')]
    hardware_clock: Option<String>,
}

fn setup_logger(level: log::LevelFilter) -> Result<(), fern::InitError> {
    let colors = fern::colors::ColoredLevelConfig::new()
        .error(Color::Red)
        .warn(Color::Yellow)
        .info(Color::BrightGreen)
        .debug(Color::BrightBlue)
        .trace(Color::BrightBlack);

    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%H:%M:%S.%f]"),
                record.target(),
                colors.color(record.level()),
                message
            ))
        })
        .level(level)
        .chain(std::io::stdout())
        .apply()?;
    Ok(())
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    setup_logger(args.loglevel).expect("Could not setup logging");

    println!("Starting PTP");

    let raw_clock = match &args.hardware_clock {
        Some(hardware_clock) => match RawLinuxClock::get_from_file(hardware_clock) {
            Ok(clock) => clock,
            Err(_) => panic!("Could not open hardware clock {hardware_clock}"),
        },
        None => RawLinuxClock::get_realtime_clock(),
    };

    let timestamping_mode = if args.hardware_clock.is_some() {
        match args.interface.interface_name {
            Some(interface_name) => TimestampingMode::Hardware(interface_name),
            None => panic!("an interface name is required when using hardware timestamping"),
        }
    } else {
        TimestampingMode::Software
    };

    let clock_identity = ClockIdentity(get_clock_id().expect("Could not get clock identity"));

    let mut instance = build_instance(
        args.interface,
        timestamping_mode,
        LinuxClock::new(raw_clock),
        clock_identity,
        args.default_ds_config,
        args.port_ds_config,
        Ports::default(),
    );

    instance.run(&LinuxTimer).await;
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use statime_linux::network::{InterfaceName, LinuxNetworkMode};

    use super::*;

    #[tokio::test]
    async fn two_ordinary_clocks() {
        setup_logger(log::LevelFilter::Trace).unwrap();

        // let interface_name_1 = InterfaceName::from_str("enp0s31f6").unwrap();
        let interface_name_1 = InterfaceName::from_str("br-44f18fce9cc6").unwrap();
        // let interface_name_2 = InterfaceName::from_str("virbr0").unwrap();
        let interface_name_2 = InterfaceName::from_str("br-2f64ef4c8839").unwrap();

        let interface_1 = InterfaceDescriptor {
            interface_name: Some(interface_name_1),
            mode: LinuxNetworkMode::Ipv4,
        };

        let interface_2 = InterfaceDescriptor {
            interface_name: Some(interface_name_2),
            mode: LinuxNetworkMode::Ipv4,
        };

        let default_ds_config_high = DefaultDSConfig {
            priority_1: 128,
            priority_2: 128,
            sdo: SdoId::default(),
            domain: 0,
            slave_only: false,
        };

        let default_ds_config_low = DefaultDSConfig {
            priority_1: 1,
            priority_2: 1,
            sdo: SdoId::default(),
            domain: 0,
            slave_only: false,
        };

        let port_ds_config = PortDSConfig {
            log_announce_interval: 1,
            log_sync_interval: 0,
            announce_receipt_timeout: 3,
        };

        let mut ordinary1 = build_instance(
            interface_1,
            TimestampingMode::Software,
            LinuxClock::new(RawLinuxClock::get_realtime_clock()),
            ClockIdentity(42u64.to_be_bytes()),
            default_ds_config_low,
            port_ds_config,
            Ports {
                tc_port: 8007,
                ntc_port: 8008,
            },
        );

        let mut ordinary2 = build_instance(
            interface_2,
            TimestampingMode::Software,
            LinuxClock::new(RawLinuxClock::get_realtime_clock()),
            ClockIdentity(43u64.to_be_bytes()),
            default_ds_config_high,
            port_ds_config,
            Ports {
                tc_port: 8007 + 2,
                ntc_port: 8008 + 2,
            },
        );

        let handle1 = async {
            ordinary1.run(&LinuxTimer).await;
        };

        let handle2 = async {
            ordinary2.run(&LinuxTimer).await;
        };

        tokio::select! {
            err = handle1 => panic!("{err:?}"),
            // err = handle2 => panic!("{err:?}"),
        }
    }
}

fn build_instance(
    interface: InterfaceDescriptor,
    timestamping_mode: TimestampingMode,
    local_clock: LinuxClock,
    clock_identity: ClockIdentity,
    default_ds_config: DefaultDSConfig,
    port_ds_config: PortDSConfig,
    ports: Ports,
) -> PtpInstance<LinuxNetworkPort, LinuxClock, BasicFilter, 1> {
    let default_ds = DefaultDS::new_ordinary_clock(
        clock_identity,
        default_ds_config.priority_1,
        default_ds_config.priority_2,
        default_ds_config.domain,
        default_ds_config.slave_only,
        default_ds_config.sdo,
    );

    let time_properties_ds =
        TimePropertiesDS::new_arbitrary_time(false, false, TimeSource::InternalOscillator);

    let port_ds = PortDS::new(
        PortIdentity {
            clock_identity,
            port_number: 1,
        },
        1,
        port_ds_config.log_announce_interval,
        port_ds_config.announce_receipt_timeout,
        port_ds_config.log_sync_interval,
        DelayMechanism::E2E,
        1,
    );

    let mut network_runtime = LinuxRuntime::new(timestamping_mode, local_clock.clone());
    let network_port = network_runtime.open_with_options(interface, ports).unwrap();
    let port = Port::new(port_ds, network_port);

    PtpInstance::new_ordinary_clock(
        default_ds,
        time_properties_ds,
        port,
        local_clock,
        BasicFilter::new(0.25),
    )
}
