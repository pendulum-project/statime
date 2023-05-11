use fern::colors::Color;
use statime::datastructures::common::{PortIdentity, TimeSource};
use statime::datastructures::datasets::{DefaultDS, DelayMechanism, PortDS, TimePropertiesDS};
use statime::datastructures::messages::SdoId;
use statime::port::Port;
use statime::{
    datastructures::common::ClockIdentity, filters::basic::BasicFilter, ptp_instance::PtpInstance,
};
use statime_linux::{
    clock::{LinuxClock, LinuxTimer, RawLinuxClock},
    network::linux::{get_clock_id, LinuxRuntime},
};

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
    setup_logger(log::LevelFilter::Trace).expect("Could not setup logging");

    let local_clock = LinuxClock::new(RawLinuxClock::get_realtime_clock());
    let mut network_runtime = LinuxRuntime::new(false, &local_clock);
    let clock_identity = ClockIdentity(get_clock_id().expect("Could not get clock identity"));

    let default_ds =
        DefaultDS::new_boundary_clock(clock_identity, 2, 128, 128, 0, SdoId::default());

    let time_properties_ds =
        TimePropertiesDS::new_arbitrary_time(false, false, TimeSource::InternalOscillator);

    let port_1_ds = PortDS::new(
        PortIdentity {
            clock_identity,
            port_number: 1,
        },
        1,
        1,
        3,
        0,
        DelayMechanism::E2E,
        1,
    );
    let port_1 = Port::new(port_1_ds, &mut network_runtime, "enp1s0f0".parse().unwrap()).await;

    let port_2_ds = PortDS::new(
        PortIdentity {
            clock_identity,
            port_number: 2,
        },
        1,
        1,
        3,
        0,
        DelayMechanism::E2E,
        1,
    );
    let port_2 = Port::new(port_2_ds, &mut network_runtime, "enp1s0f1".parse().unwrap()).await;

    let mut instance = PtpInstance::new_boundary_clock(
        default_ds,
        time_properties_ds,
        [port_1, port_2],
        local_clock,
        BasicFilter::new(0.25),
    );

    instance.run(&LinuxTimer).await;
}
