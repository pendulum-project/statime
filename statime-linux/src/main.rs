use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    pin::{pin, Pin},
    sync::RwLock,
};

use clap::Parser;
use rand::{rngs::StdRng, SeedableRng};
use statime::{
    config::{ClockIdentity, InstanceConfig, SdoId, TimePropertiesDS, TimeSource},
    filters::{Filter, KalmanConfiguration, KalmanFilter},
    port::{
        is_message_buffer_compatible, InBmca, Measurement, Port, PortAction, PortActionIterator,
        TimestampContext, MAX_DATA_LEN,
    },
    time::Time,
    Clock, OverlayClock, PtpInstance, PtpInstanceState, SharedClock,
};
use statime_linux::{
    clock::{LinuxClock, PortTimestampToTime},
    initialize_logging_parse_config,
    observer::ObservableInstanceState,
    socket::{
        open_ethernet_socket, open_ipv4_event_socket, open_ipv4_general_socket,
        open_ipv6_event_socket, open_ipv6_general_socket, PtpTargetAddress,
    },
    tlvforwarder::TlvForwarder,
};
use timestamped_socket::{
    interface::interfaces,
    networkaddress::{EthernetAddress, NetworkAddress},
    socket::{InterfaceTimestampMode, Open, Socket},
};
use tokio::{
    sync::mpsc::{Receiver, Sender},
    time::Sleep,
};

trait PortClock:
    Clock<Error = <LinuxClock as Clock>::Error> + PortTimestampToTime + Send + Sync
{
    fn clone_box(&self) -> Box<dyn PortClock>;
}
impl PortClock for LinuxClock {
    fn clone_box(&self) -> Box<dyn PortClock> {
        Box::new(self.clone())
    }
}
impl PortClock for SharedClock<OverlayClock<LinuxClock>> {
    fn clone_box(&self) -> Box<dyn PortClock> {
        Box::new(self.clone())
    }
}
type BoxedClock = Box<dyn PortClock>;
type SharedOverlayClock = SharedClock<OverlayClock<LinuxClock>>;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    /// Configuration file to use
    #[clap(
        long = "config",
        short = 'c',
        default_value = "/etc/statime/statime.toml"
    )]
    config_file: Option<PathBuf>,
}

pin_project_lite::pin_project! {
    struct Timer {
        #[pin]
        timer: Sleep,
        running: bool,
    }
}

impl Timer {
    fn new() -> Self {
        Timer {
            timer: tokio::time::sleep(std::time::Duration::from_secs(0)),
            running: false,
        }
    }

    fn reset(self: Pin<&mut Self>, duration: std::time::Duration) {
        let this = self.project();
        this.timer.reset(tokio::time::Instant::now() + duration);
        *this.running = true;
    }
}

impl Future for Timer {
    type Output = ();

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.project();
        if *this.running {
            let result = this.timer.poll(cx);
            if result != std::task::Poll::Pending {
                *this.running = false;
            }
            result
        } else {
            std::task::Poll::Pending
        }
    }
}

#[derive(Debug, Clone)]
enum SystemClock {
    Linux(LinuxClock),
    Overlay(SharedOverlayClock),
}
impl SystemClock {
    fn clone_boxed(&self) -> BoxedClock {
        match self {
            Self::Linux(clock) => Box::new(clock.clone()),
            Self::Overlay(clock) => Box::new(clock.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum ClockSyncMode {
    #[default]
    FromSystem,
    ToSystem,
}

fn start_clock_task(
    clock: LinuxClock,
    system_clock: SystemClock,
) -> tokio::sync::watch::Sender<ClockSyncMode> {
    let (mode_sender, mode_receiver) = tokio::sync::watch::channel(ClockSyncMode::FromSystem);

    match system_clock {
        SystemClock::Linux(system_clock) => {
            tokio::spawn(clock_task(clock, system_clock, None, mode_receiver));
        }
        SystemClock::Overlay(overlay_clock) => {
            tokio::spawn(clock_task(
                clock,
                overlay_clock.clone(),
                Some(overlay_clock),
                mode_receiver,
            ));
        }
    }

    mode_sender
}

async fn clock_task<C: Clock<Error = impl core::fmt::Debug>>(
    mut clock: LinuxClock,
    mut system_clock: C,
    system_clock_overlay: Option<SharedOverlayClock>,
    mut mode_receiver: tokio::sync::watch::Receiver<ClockSyncMode>,
) {
    let mut measurement_timer = pin!(Timer::new());
    let mut update_timer = pin!(Timer::new());

    measurement_timer.as_mut().reset(std::time::Duration::ZERO);

    let mut filter = KalmanFilter::new(KalmanConfiguration::default());

    let mut current_mode = *mode_receiver.borrow_and_update();
    loop {
        tokio::select! {
            () = &mut measurement_timer => {
                let (raw_t1, t2, raw_t3) = clock.system_offset().expect("Unable to determine offset from system clock");
                let (t1, t3) = match &system_clock_overlay {
                    Some(shared) => {
                        let overlay = shared.0.lock().expect("shared clock lock is tainted");
                        (overlay.time_from_underlying(raw_t1), overlay.time_from_underlying(raw_t3))
                    },
                    None => (raw_t1, raw_t3)
                };

                log::debug!("Interclock measurement: {} {} {}", t1, t2, t3);

                let delay = (t3-t1)/2;
                let offset_a = t2 - t1;
                let offset_b = t3 - t2;

                let update = match current_mode {
                    ClockSyncMode::FromSystem => {
                        let m = Measurement {
                            event_time: t2,
                            offset: Some(offset_a - delay),
                            delay: Some(delay),
                            peer_delay: None,
                            raw_sync_offset: Some(offset_a),
                            raw_delay_offset: Some(-offset_b),
                        };
                        filter.measurement(m, &mut clock)
                    },
                    ClockSyncMode::ToSystem => {
                        let m = Measurement {
                            event_time: t1+delay,
                            offset: Some(offset_b - delay),
                            delay: Some(delay),
                            peer_delay: None,
                            raw_sync_offset: Some(offset_b),
                            raw_delay_offset: Some(-offset_a),
                        };
                        filter.measurement(m, &mut system_clock)
                    },
                };

                if let Some(timeout) = update.next_update {
                    update_timer.as_mut().reset(timeout);
                }

                measurement_timer.as_mut().reset(std::time::Duration::from_millis(250));
            }
            () = &mut update_timer => {
                let update = match current_mode {
                    ClockSyncMode::FromSystem => filter.update(&mut clock),
                    ClockSyncMode::ToSystem => filter.update(&mut system_clock),
                };
                if let Some(timeout) = update.next_update {
                    update_timer.as_mut().reset(timeout);
                }
            }
            _ = mode_receiver.changed() => {
                let new_mode = *mode_receiver.borrow_and_update();
                if new_mode != current_mode {
                    let mut new_filter = KalmanFilter::new(KalmanConfiguration::default());
                    std::mem::swap(&mut filter, &mut new_filter);
                    match current_mode {
                        ClockSyncMode::FromSystem => new_filter.demobilize(&mut clock),
                        ClockSyncMode::ToSystem => new_filter.demobilize(&mut system_clock)
                    };
                    current_mode = new_mode;
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    actual_main().await;
}

async fn actual_main() {
    let args = Args::parse();

    let config = initialize_logging_parse_config(
        &args
            .config_file
            .expect("could not determine config file path"),
    );

    let clock_identity = config.identity.unwrap_or(ClockIdentity(
        get_clock_id().expect("could not get clock identity"),
    ));

    log::info!("Clock identity: {}", hex::encode(clock_identity.0));

    let instance_config = InstanceConfig {
        clock_identity,
        priority_1: config.priority1,
        priority_2: config.priority2,
        domain_number: config.domain,
        slave_only: false,
        sdo_id: SdoId::try_from(config.sdo_id).expect("sdo-id should be between 0 and 4095"),
        path_trace: config.path_trace,
    };

    let time_properties_ds =
        TimePropertiesDS::new_arbitrary_time(false, false, TimeSource::InternalOscillator);

    let system_clock = if config.virtual_system_clock {
        SystemClock::Overlay(SharedClock::new(OverlayClock::new(LinuxClock::CLOCK_TAI)))
    } else {
        SystemClock::Linux(LinuxClock::CLOCK_TAI)
    };

    // Leak to get a static reference, the ptp instance will be around for the rest
    // of the program anyway
    let instance = Box::leak(Box::new(PtpInstance::new(
        instance_config,
        time_properties_ds,
    )));

    // The observer for the metrics exporter
    let (instance_state_sender, instance_state_receiver) =
        tokio::sync::watch::channel(ObservableInstanceState {
            default_ds: instance.default_ds(),
            current_ds: instance.current_ds(),
            parent_ds: instance.parent_ds(),
            time_properties_ds: instance.time_properties_ds(),
            path_trace_ds: instance.path_trace_ds(),
        });
    statime_linux::observer::spawn(&config, instance_state_receiver).await;

    let (bmca_notify_sender, bmca_notify_receiver) = tokio::sync::watch::channel(false);

    let mut main_task_senders = Vec::with_capacity(config.ports.len());
    let mut main_task_receivers = Vec::with_capacity(config.ports.len());

    let mut internal_sync_senders = vec![];

    let mut clock_name_map = HashMap::new();
    let mut clock_port_map = Vec::with_capacity(config.ports.len());

    let mut ports = Vec::with_capacity(config.ports.len());

    let tlv_forwarder = TlvForwarder::new();

    for port_config in config.ports {
        let interface = port_config.interface;
        let network_mode = port_config.network_mode;
        let (port_clock, timestamping) = match port_config.hardware_clock {
            Some(idx) => {
                let mut clock = LinuxClock::open_idx(idx).expect("Unable to open clock");
                if let Some(id) = clock_name_map.get(&idx) {
                    clock_port_map.push(Some(*id));
                } else {
                    clock.init().expect("Unable to initialize clock");
                    let id = internal_sync_senders.len();
                    clock_port_map.push(Some(id));
                    clock_name_map.insert(idx, id);
                    internal_sync_senders
                        .push(start_clock_task(clock.clone(), system_clock.clone()));
                }
                (
                    Box::new(clock) as BoxedClock,
                    InterfaceTimestampMode::HardwarePTPAll,
                )
            }
            None => {
                clock_port_map.push(None);
                (
                    system_clock.clone_boxed(),
                    InterfaceTimestampMode::SoftwareAll,
                )
            }
        };

        let rng = StdRng::from_entropy();
        let bind_phc = port_config.hardware_clock;
        let port = instance.add_port(
            port_config.into(),
            KalmanConfiguration::default(),
            port_clock.clone_box(),
            rng,
        );

        let (main_task_sender, port_task_receiver) = tokio::sync::mpsc::channel(1);
        let (port_task_sender, main_task_receiver) = tokio::sync::mpsc::channel(1);

        // We can't send the port yet, since that may start running on the port,
        // inhibiting write access to the instance and making it impossible to
        // create more ports.
        ports.push(port);
        main_task_senders.push(main_task_sender);
        main_task_receivers.push(main_task_receiver);

        match network_mode {
            statime_linux::config::NetworkMode::Ipv4 => {
                let event_socket = open_ipv4_event_socket(interface, timestamping, bind_phc)
                    .expect("Could not open event socket");
                let general_socket =
                    open_ipv4_general_socket(interface).expect("Could not open general socket");

                tokio::spawn(port_task(
                    port_task_receiver,
                    port_task_sender,
                    event_socket,
                    general_socket,
                    bmca_notify_receiver.clone(),
                    tlv_forwarder.duplicate(),
                    port_clock,
                ));
            }
            statime_linux::config::NetworkMode::Ipv6 => {
                let event_socket = open_ipv6_event_socket(interface, timestamping, bind_phc)
                    .expect("Could not open event socket");
                let general_socket =
                    open_ipv6_general_socket(interface).expect("Could not open general socket");

                tokio::spawn(port_task(
                    port_task_receiver,
                    port_task_sender,
                    event_socket,
                    general_socket,
                    bmca_notify_receiver.clone(),
                    tlv_forwarder.duplicate(),
                    port_clock,
                ));
            }
            statime_linux::config::NetworkMode::Ethernet => {
                let socket = open_ethernet_socket(interface, timestamping, bind_phc)
                    .expect("Could not open socket");

                tokio::spawn(ethernet_port_task(
                    port_task_receiver,
                    port_task_sender,
                    interface
                        .get_index()
                        .expect("Unable to get network interface index") as _,
                    socket,
                    bmca_notify_receiver.clone(),
                    tlv_forwarder.duplicate(),
                    port_clock,
                ));
            }
        }
    }

    // Drop the forwarder so we don't keep an unneeded subscriber.
    drop(tlv_forwarder);

    // All ports created, so we can start running them.
    for (i, port) in ports.into_iter().enumerate() {
        main_task_senders[i]
            .send(port)
            .await
            .expect("space in channel buffer");
    }

    run(
        instance,
        bmca_notify_sender,
        instance_state_sender,
        main_task_receivers,
        main_task_senders,
        internal_sync_senders,
        clock_port_map,
    )
    .await
}

async fn run(
    instance: &'static PtpInstance<KalmanFilter, RwLock<PtpInstanceState>>,
    bmca_notify_sender: tokio::sync::watch::Sender<bool>,
    instance_state_sender: tokio::sync::watch::Sender<ObservableInstanceState>,
    mut main_task_receivers: Vec<Receiver<BmcaPort>>,
    main_task_senders: Vec<Sender<BmcaPort>>,
    internal_sync_senders: Vec<tokio::sync::watch::Sender<ClockSyncMode>>,
    clock_port_map: Vec<Option<usize>>,
) -> ! {
    // run bmca over all of the ports at the same time. The ports don't perform
    // their normal actions at this time: bmca is stop-the-world!
    let mut bmca_timer = pin!(Timer::new());

    loop {
        // reset bmca timer
        bmca_timer.as_mut().reset(instance.bmca_interval());

        // wait until the next BMCA
        bmca_timer.as_mut().await;

        // notify all the ports that they need to stop what they're doing
        bmca_notify_sender
            .send(true)
            .expect("Bmca notification failed");

        let mut bmca_ports = Vec::with_capacity(main_task_receivers.len());
        let mut mut_bmca_ports = Vec::with_capacity(main_task_receivers.len());

        for receiver in main_task_receivers.iter_mut() {
            bmca_ports.push(receiver.recv().await.unwrap());
        }

        // have all ports so deassert stop
        bmca_notify_sender
            .send(false)
            .expect("Bmca notification failed");

        for mut_bmca_port in bmca_ports.iter_mut() {
            mut_bmca_ports.push(mut_bmca_port);
        }

        instance.bmca(&mut mut_bmca_ports);

        // Update instance state for observability
        // We don't care if isn't anybody on the other side
        let _ = instance_state_sender.send(ObservableInstanceState {
            default_ds: instance.default_ds(),
            current_ds: instance.current_ds(),
            parent_ds: instance.parent_ds(),
            time_properties_ds: instance.time_properties_ds(),
            path_trace_ds: instance.path_trace_ds(),
        });

        let mut clock_states = vec![ClockSyncMode::FromSystem; internal_sync_senders.len()];
        for (idx, port) in mut_bmca_ports.iter().enumerate() {
            if port.is_steering() {
                if let Some(id) = clock_port_map[idx] {
                    clock_states[id] = ClockSyncMode::ToSystem;
                }
            }
        }
        for (mode, sender) in clock_states.into_iter().zip(internal_sync_senders.iter()) {
            sender.send(mode).expect("Clock mode change failed");
        }

        drop(mut_bmca_ports);

        for (port, sender) in bmca_ports.into_iter().zip(main_task_senders.iter()) {
            sender.send(port).await.unwrap();
        }
    }
}

type BmcaPort = Port<
    'static,
    InBmca,
    Option<Vec<ClockIdentity>>,
    StdRng,
    BoxedClock,
    KalmanFilter,
    RwLock<PtpInstanceState>,
>;

// the Port task
//
// This task waits for a new port (in the bmca state) to arrive on its Receiver.
// It will then move the port into the running state, and process actions. When
// the task is notified of a BMCA, it will stop running, move the port into the
// bmca state, and send it on its Sender
async fn port_task<A: NetworkAddress + PtpTargetAddress>(
    mut port_task_receiver: Receiver<BmcaPort>,
    port_task_sender: Sender<BmcaPort>,
    mut event_socket: Socket<A, Open>,
    mut general_socket: Socket<A, Open>,
    mut bmca_notify: tokio::sync::watch::Receiver<bool>,
    mut tlv_forwarder: TlvForwarder,
    clock: BoxedClock,
) {
    let mut timers = Timers {
        port_sync_timer: pin!(Timer::new()),
        port_announce_timer: pin!(Timer::new()),
        port_announce_timeout_timer: pin!(Timer::new()),
        delay_request_timer: pin!(Timer::new()),
        filter_update_timer: pin!(Timer::new()),
    };

    loop {
        let port_in_bmca = port_task_receiver.recv().await.unwrap();

        // handle post-bmca actions
        let (mut port, actions) = port_in_bmca.end_bmca();

        let mut pending_timestamp = handle_actions(
            actions,
            &mut event_socket,
            &mut general_socket,
            &mut timers,
            &tlv_forwarder,
            &clock,
        )
        .await;

        while let Some((context, timestamp)) = pending_timestamp {
            pending_timestamp = handle_actions(
                port.handle_send_timestamp(context, timestamp),
                &mut event_socket,
                &mut general_socket,
                &mut timers,
                &tlv_forwarder,
                &clock,
            )
            .await;
        }

        let mut event_buffer = [0; MAX_DATA_LEN];
        let mut general_buffer = [0; 2048];

        loop {
            let mut actions = tokio::select! {
                result = event_socket.recv(&mut event_buffer) => match result {
                    Ok(packet) => {
                        if !is_message_buffer_compatible(&event_buffer[..packet.bytes_read]) {
                            // do not spam with missing timestamp error in mixed-version PTPv1+v2 networks
                            PortActionIterator::empty()
                        } else if let Some(timestamp) = packet.timestamp {
                            log::trace!("Recv timestamp: {:?}", packet.timestamp);
                            port.handle_event_receive(&event_buffer[..packet.bytes_read], clock.port_timestamp_to_time(timestamp))
                        } else {
                            log::error!("Missing recv timestamp");
                            PortActionIterator::empty()
                        }
                    }
                    Err(error) => panic!("Error receiving: {error:?}"),
                },
                result = general_socket.recv(&mut general_buffer) => match result {
                    Ok(packet) => port.handle_general_receive(&general_buffer[..packet.bytes_read]),
                    Err(error) => panic!("Error receiving: {error:?}"),
                },
                () = &mut timers.port_announce_timer => {
                    port.handle_announce_timer(&mut tlv_forwarder)
                },
                () = &mut timers.port_sync_timer => {
                    port.handle_sync_timer()
                },
                () = &mut timers.port_announce_timeout_timer => {
                    port.handle_announce_receipt_timer()
                },
                () = &mut timers.delay_request_timer => {
                    port.handle_delay_request_timer()
                },
                () = &mut timers.filter_update_timer => {
                    port.handle_filter_update_timer()
                },
                result = bmca_notify.wait_for(|v| *v) => match result {
                    Ok(_) => break,
                    Err(error) => panic!("Error on bmca notify: {error:?}"),
                }
            };

            loop {
                let pending_timestamp = handle_actions(
                    actions,
                    &mut event_socket,
                    &mut general_socket,
                    &mut timers,
                    &tlv_forwarder,
                    &clock,
                )
                .await;

                // there might be more actions to handle based on the current action
                actions = match pending_timestamp {
                    Some((context, timestamp)) => port.handle_send_timestamp(context, timestamp),
                    None => break,
                };
            }
        }

        let port_in_bmca = port.start_bmca();
        port_task_sender.send(port_in_bmca).await.unwrap();
    }
}

// the Port task for ethernet transport
//
// This task waits for a new port (in the bmca state) to arrive on its Receiver.
// It will then move the port into the running state, and process actions. When
// the task is notified of a BMCA, it will stop running, move the port into the
// bmca state, and send it on its Sender
async fn ethernet_port_task(
    mut port_task_receiver: Receiver<BmcaPort>,
    port_task_sender: Sender<BmcaPort>,
    interface: libc::c_int,
    mut socket: Socket<EthernetAddress, Open>,
    mut bmca_notify: tokio::sync::watch::Receiver<bool>,
    mut tlv_forwarder: TlvForwarder,
    clock: BoxedClock,
) {
    let mut timers = Timers {
        port_sync_timer: pin!(Timer::new()),
        port_announce_timer: pin!(Timer::new()),
        port_announce_timeout_timer: pin!(Timer::new()),
        delay_request_timer: pin!(Timer::new()),
        filter_update_timer: pin!(Timer::new()),
    };

    loop {
        let port_in_bmca = port_task_receiver.recv().await.unwrap();

        // Clear out old tlvs if we are not in the master state, so we don't keep em too
        // long.
        if port_in_bmca.is_master() {
            tlv_forwarder.empty()
        }

        // handle post-bmca actions
        let (mut port, actions) = port_in_bmca.end_bmca();

        let mut pending_timestamp = handle_actions_ethernet(
            actions,
            interface,
            &mut socket,
            &mut timers,
            &tlv_forwarder,
            &clock,
        )
        .await;

        while let Some((context, timestamp)) = pending_timestamp {
            pending_timestamp = handle_actions_ethernet(
                port.handle_send_timestamp(context, timestamp),
                interface,
                &mut socket,
                &mut timers,
                &tlv_forwarder,
                &clock,
            )
            .await;
        }

        let mut event_buffer = [0; MAX_DATA_LEN];

        loop {
            let mut actions = tokio::select! {
                result = socket.recv(&mut event_buffer) => match result {
                    Ok(packet) => {
                        if let Some(timestamp) = packet.timestamp {
                            log::trace!("Recv timestamp: {:?}", packet.timestamp);
                            port.handle_event_receive(&event_buffer[..packet.bytes_read], clock.port_timestamp_to_time(timestamp))
                        } else {
                            port.handle_general_receive(&event_buffer[..packet.bytes_read])
                        }
                    }
                    Err(error) => panic!("Error receiving: {error:?}"),
                },
                () = &mut timers.port_announce_timer => {
                    port.handle_announce_timer(&mut tlv_forwarder)
                },
                () = &mut timers.port_sync_timer => {
                    port.handle_sync_timer()
                },
                () = &mut timers.port_announce_timeout_timer => {
                    port.handle_announce_receipt_timer()
                },
                () = &mut timers.delay_request_timer => {
                    port.handle_delay_request_timer()
                },
                () = &mut timers.filter_update_timer => {
                    port.handle_filter_update_timer()
                },
                result = bmca_notify.wait_for(|v| *v) => match result {
                    Ok(_) => break,
                    Err(error) => panic!("Error on bmca notify: {error:?}"),
                }
            };

            loop {
                let pending_timestamp = handle_actions_ethernet(
                    actions,
                    interface,
                    &mut socket,
                    &mut timers,
                    &tlv_forwarder,
                    &clock,
                )
                .await;

                // there might be more actions to handle based on the current action
                actions = match pending_timestamp {
                    Some((context, timestamp)) => port.handle_send_timestamp(context, timestamp),
                    None => break,
                };
            }
        }

        let port_in_bmca = port.start_bmca();
        port_task_sender.send(port_in_bmca).await.unwrap();
    }
}

struct Timers<'a> {
    port_sync_timer: Pin<&'a mut Timer>,
    port_announce_timer: Pin<&'a mut Timer>,
    port_announce_timeout_timer: Pin<&'a mut Timer>,
    delay_request_timer: Pin<&'a mut Timer>,
    filter_update_timer: Pin<&'a mut Timer>,
}

async fn handle_actions<A: NetworkAddress + PtpTargetAddress>(
    actions: PortActionIterator<'_>,
    event_socket: &mut Socket<A, Open>,
    general_socket: &mut Socket<A, Open>,
    timers: &mut Timers<'_>,
    tlv_forwarder: &TlvForwarder,
    clock: &BoxedClock,
) -> Option<(TimestampContext, Time)> {
    let mut pending_timestamp = None;

    for action in actions {
        match action {
            PortAction::SendEvent {
                context,
                data,
                link_local,
            } => {
                // send timestamp of the send
                let time = event_socket
                    .send_to(
                        data,
                        if link_local {
                            A::PDELAY_EVENT
                        } else {
                            A::PRIMARY_EVENT
                        },
                    )
                    .await
                    .expect("Failed to send event message");

                // anything we send later will have a later pending (send) timestamp
                if let Some(time) = time {
                    log::trace!("Send timestamp {:?}", time);
                    pending_timestamp = Some((context, clock.port_timestamp_to_time(time)));
                } else {
                    log::error!("Missing send timestamp");
                }
            }
            PortAction::SendGeneral { data, link_local } => {
                general_socket
                    .send_to(
                        data,
                        if link_local {
                            A::PDELAY_GENERAL
                        } else {
                            A::PRIMARY_GENERAL
                        },
                    )
                    .await
                    .expect("Failed to send general message");
            }
            PortAction::ResetAnnounceTimer { duration } => {
                timers.port_announce_timer.as_mut().reset(duration);
            }
            PortAction::ResetSyncTimer { duration } => {
                timers.port_sync_timer.as_mut().reset(duration);
            }
            PortAction::ResetDelayRequestTimer { duration } => {
                timers.delay_request_timer.as_mut().reset(duration);
            }
            PortAction::ResetAnnounceReceiptTimer { duration } => {
                timers.port_announce_timeout_timer.as_mut().reset(duration);
            }
            PortAction::ResetFilterUpdateTimer { duration } => {
                timers.filter_update_timer.as_mut().reset(duration);
            }
            PortAction::ForwardTLV { tlv } => {
                tlv_forwarder.forward(tlv.into_owned());
            }
        }
    }

    pending_timestamp
}

async fn handle_actions_ethernet(
    actions: PortActionIterator<'_>,
    interface: libc::c_int,
    socket: &mut Socket<EthernetAddress, Open>,
    timers: &mut Timers<'_>,
    tlv_forwarder: &TlvForwarder,
    clock: &BoxedClock,
) -> Option<(TimestampContext, Time)> {
    let mut pending_timestamp = None;

    for action in actions {
        match action {
            PortAction::SendEvent {
                context,
                data,
                link_local,
            } => {
                // send timestamp of the send
                let time = socket
                    .send_to(
                        data,
                        EthernetAddress::new(
                            if link_local {
                                EthernetAddress::PDELAY_EVENT.protocol()
                            } else {
                                EthernetAddress::PRIMARY_EVENT.protocol()
                            },
                            if link_local {
                                EthernetAddress::PDELAY_EVENT.mac()
                            } else {
                                EthernetAddress::PRIMARY_EVENT.mac()
                            },
                            interface,
                        ),
                    )
                    .await
                    .expect("Failed to send event message");

                // anything we send later will have a later pending (send) timestamp
                if let Some(time) = time {
                    log::trace!("Send timestamp {:?}", time);
                    pending_timestamp = Some((context, clock.port_timestamp_to_time(time)));
                } else {
                    log::error!("Missing send timestamp");
                }
            }
            PortAction::SendGeneral { data, link_local } => {
                socket
                    .send_to(
                        data,
                        EthernetAddress::new(
                            if link_local {
                                EthernetAddress::PDELAY_GENERAL.protocol()
                            } else {
                                EthernetAddress::PRIMARY_GENERAL.protocol()
                            },
                            if link_local {
                                EthernetAddress::PDELAY_GENERAL.mac()
                            } else {
                                EthernetAddress::PRIMARY_GENERAL.mac()
                            },
                            interface,
                        ),
                    )
                    .await
                    .expect("Failed to send general message");
            }
            PortAction::ResetAnnounceTimer { duration } => {
                timers.port_announce_timer.as_mut().reset(duration);
            }
            PortAction::ResetSyncTimer { duration } => {
                timers.port_sync_timer.as_mut().reset(duration);
            }
            PortAction::ResetDelayRequestTimer { duration } => {
                timers.delay_request_timer.as_mut().reset(duration);
            }
            PortAction::ResetAnnounceReceiptTimer { duration } => {
                timers.port_announce_timeout_timer.as_mut().reset(duration);
            }
            PortAction::ResetFilterUpdateTimer { duration } => {
                timers.filter_update_timer.as_mut().reset(duration);
            }
            PortAction::ForwardTLV { tlv } => tlv_forwarder.forward(tlv.into_owned()),
        }
    }

    pending_timestamp
}

fn get_clock_id() -> Option<[u8; 8]> {
    let candidates = interfaces()
        .unwrap()
        .into_iter()
        .filter_map(|(_, data)| data.mac());

    for mac in candidates {
        // Ignore multicast and locally administered mac addresses
        if mac[0] & 0x3 == 0 && mac.iter().any(|x| *x != 0) {
            let f = |i| mac.get(i).copied().unwrap_or_default();
            return Some(std::array::from_fn(f));
        }
    }

    None
}
