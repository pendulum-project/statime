use clap::{AppSettings, Parser};
use statime::{
    datastructures::common::ClockIdentity,
    filters::basic::BasicFilter,
    ptp_instance::{Config, PtpInstance},
};
use statime_linux::{
    clock::{LinuxClock, LinuxTimer, RawLinuxClock},
    network::linux::{get_clock_id, LinuxInterfaceDescriptor, LinuxRuntime},
};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None, setting = AppSettings::DeriveDisplayOrder)]
struct Args {
    /// Set desired logging level
    #[clap(short, long, default_value_t = log::LevelFilter::Info)]
    loglevel: log::LevelFilter,

    /// Set interface on which to listen to PTP messages
    #[clap(short, long)]
    interface: LinuxInterfaceDescriptor,

    /// The SDO id of the desired ptp domain
    #[clap(long, default_value_t = 0)]
    sdo: u16,

    /// The domain number of the desired ptp domain
    #[clap(long, default_value_t = 0)]
    domain: u8,

    /// Local clock priority (part 1) used in master clock selection
    #[clap(long, default_value_t = 255)]
    priority_1: u8,

    /// Locqal clock priority (part 2) used in master clock selection
    #[clap(long, default_value_t = 255)]
    priority_2: u8,

    /// Log value of interval expected between announce messages
    #[clap(long, default_value_t = 1)]
    log_announce_interval: i8,

    /// Use hardware clock
    #[clap(long, short)]
    hardware_clock: Option<String>,
}

fn setup_logger(level: log::LevelFilter) -> Result<(), fern::InitError> {
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
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

    let clock = if let Some(hardware_clock) = &args.hardware_clock {
        LinuxClock::new(
            RawLinuxClock::get_from_file(hardware_clock).expect("Could not open hardware clock"),
        )
    } else {
        LinuxClock::new(RawLinuxClock::get_realtime_clock())
    };
    let network_runtime = LinuxRuntime::new(args.hardware_clock.is_some(), &clock);
    let clock_id = ClockIdentity(get_clock_id().expect("Could not get clock identity"));

    let config = Config {
        identity: clock_id,
        sdo: args.sdo,
        domain: args.domain,
        interface: args.interface,
        port_config: statime::port::PortConfig {
            log_announce_interval: args.log_announce_interval,
            priority_1: args.priority_1,
            priority_2: args.priority_2,
        },
    };

    let mut instance =
        PtpInstance::new(config, network_runtime, clock, BasicFilter::new(0.25)).await;
    instance.run(&LinuxTimer).await;
}
