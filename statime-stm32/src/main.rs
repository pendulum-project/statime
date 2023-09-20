#![no_main]
#![no_std]
#![feature(type_alias_impl_trait)]

use core::task::Poll;

use defmt::unwrap;
use defmt_rtt as _;
use futures::{
    future::{poll_fn, FutureExt},
    select_biased,
};
use ieee802_3_miim::{
    phy::{PhySpeed, LAN8742A},
    Phy,
};
// global logger
use panic_probe as _; // panic handler
use rtic::{app, Mutex};
use rtic_monotonics::systick::{ExtU64, Systick};
use rtic_sync::{
    channel::{Receiver, Sender},
    make_channel,
};
use smoltcp::{
    iface::{Config, Interface, SocketHandle, SocketSet},
    socket::udp::{self, UdpMetadata},
    wire::{EthernetAddress, IpAddress, IpCidr, IpEndpoint, Ipv4Address},
};
use static_cell::StaticCell;
use statime::{
    BasicFilter, Duration, InBmca, InstanceConfig, Interval, PortAction, PortActionIterator,
    PortConfig, PtpInstance, Running, SdoId,
};
use stm32_eth::{
    dma::{PacketId, PacketIdNotFound},
    mac,
    ptp::Timestamp,
    EthPins, Parts, PartsIn,
};
use stm32f7xx_hal::{
    gpio::{Output, Pin, Speed},
    prelude::*,
    rng::{Rng, RngExt},
};

use crate::{
    ethernet::{NetworkResources, NetworkStack, CLIENT_ADDR},
    ptp_clock::PtpClock,
};

defmt::timestamp!("{=u64:us}", {
    let time = stm32_eth::ptp::EthernetPTP::get_time();
    time.seconds() as u64 * 1_000_000 + (time.subseconds().nanos() / 1000) as u64
});

type StmPort<State> = statime::Port<State, Rng, &'static PtpClock, BasicFilter>;

pub enum PtpPort {
    None,
    Running(StmPort<Running<'static>>),
    InBmca(StmPort<InBmca<'static>>),
}

impl PtpPort {
    pub fn as_bmca_mode(&mut self) -> &mut StmPort<InBmca<'static>> {
        let this = core::mem::replace(self, PtpPort::None);

        *self = match this {
            PtpPort::Running(port) => PtpPort::InBmca(port.start_bmca()),
            val => val,
        };

        match self {
            PtpPort::InBmca(port) => port,
            _ => defmt::unreachable!(),
        }
    }

    pub fn to_running(&mut self) -> PortActionIterator<'static> {
        let this = core::mem::replace(self, PtpPort::None);

        let (this, actions) = match this {
            PtpPort::InBmca(port) => {
                let (port, actions) = port.end_bmca();
                (PtpPort::Running(port), actions)
            }
            _ => defmt::panic!("Port in bad state"),
        };

        *self = this;

        actions
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TimerName {
    Announce,
    Sync,
    DelayRequest,
    AnnounceReceipt,
    FilterUpdate,
}

pub mod ethernet;
pub mod ptp_clock;

#[app(device = stm32f7xx_hal::pac, dispatchers = [CAN1_RX0, CAN1_RX1, CAN1_TX])]
mod app {
    use smoltcp::{socket::dhcpv4, wire::Ipv4Cidr};

    use super::*;
    use crate::ptp_clock::stm_time_to_statime;

    #[shared]
    struct Shared {
        net: NetworkStack,
        ptp_instance: &'static statime::PtpInstance<&'static PtpClock, BasicFilter>,
        ptp_port: PtpPort,
    }

    #[local]
    struct Local {
        led_pin: Pin<'B', 7, Output>,
        please_poll_s: Sender<'static, (), 1>,
        tx_done_s: Sender<'static, (), 1>,
    }

    #[init(local = [net_resources: NetworkResources = NetworkResources::new()])]
    fn init(cx: init::Context) -> (Shared, Local) {
        let p = cx.device;

        // Setup clocks
        let rcc = p.RCC.constrain();
        let clocks = rcc.cfgr.sysclk(96.MHz()).hclk(96.MHz());
        let clocks = clocks.freeze();

        log_to_defmt::setup();

        // Setup LED
        let gpioa = p.GPIOA.split();
        let gpiob = p.GPIOB.split();
        let gpioc = p.GPIOC.split();
        let gpiog = p.GPIOG.split();
        let led_pin = gpiob.pb7.into_push_pull_output();

        let systick_token = rtic_monotonics::create_systick_token!();
        Systick::start(cx.core.SYST, 96_000_000, systick_token);

        // Setup Ethernet
        let ethernet = PartsIn {
            dma: p.ETHERNET_DMA,
            mac: p.ETHERNET_MAC,
            mmc: p.ETHERNET_MMC,
            ptp: p.ETHERNET_PTP,
        };

        let ref_clk = gpioa.pa1.into_floating_input();
        let crs = gpioa.pa7.into_floating_input();
        let tx_d1 = gpiob.pb13.into_floating_input();
        let rx_d0 = gpioc.pc4.into_floating_input();
        let rx_d1 = gpioc.pc5.into_floating_input();

        let (tx_en, tx_d0) = {
            (
                gpiog.pg11.into_floating_input(),
                gpiog.pg13.into_floating_input(),
            )
        };

        let (mdio, mdc) = (
            gpioa.pa2.into_alternate().set_speed(Speed::VeryHigh),
            gpioc.pc1.into_alternate().set_speed(Speed::VeryHigh),
        );

        let pps = gpiob.pb5.into_push_pull_output();

        let eth_pins = EthPins {
            ref_clk,
            crs,
            tx_en,
            tx_d0,
            tx_d1,
            rx_d0,
            rx_d1,
        };

        let NetworkResources {
            rx_ring,
            tx_ring,
            rx_meta_storage,
            rx_payload_storage,
            tx_meta_storage,
            tx_payload_storage,
            tc_rx_meta_storage,
            tc_rx_payload_storage,
            tc_tx_meta_storage,
            tc_tx_payload_storage,
            sockets,
        } = cx.local.net_resources;

        let Parts {
            mut dma,
            mac,
            mut ptp,
        } = stm32_eth::new_with_mii(ethernet, rx_ring, tx_ring, clocks, eth_pins, mdio, mdc)
            .unwrap();

        // Setup smoltcp
        let cfg = Config::new(EthernetAddress(CLIENT_ADDR).into());

        let mut interface = Interface::new(cfg, &mut &mut dma, smoltcp::time::Instant::ZERO);

        let dhcp_socket = smoltcp::socket::dhcpv4::Socket::new();

        interface.update_ip_addrs(|a| {
            a.push(IpCidr::new(IpAddress::v4(10, 0, 0, 2), 24)).unwrap();
        });

        unwrap!(interface.join_multicast_group(
            &mut &mut dma,
            Ipv4Address::new(224, 0, 1, 129),
            smoltcp::time::Instant::ZERO
        ));
        unwrap!(interface.join_multicast_group(
            &mut &mut dma,
            Ipv4Address::new(224, 0, 0, 107),
            smoltcp::time::Instant::ZERO
        ));

        defmt::info!("Set IPs to: {}", interface.ip_addrs());

        // Setup socket
        let tc_rx_buffer =
            udp::PacketBuffer::new(&mut tc_rx_meta_storage[..], &mut tc_rx_payload_storage[..]);
        let tc_tx_buffer =
            udp::PacketBuffer::new(&mut tc_tx_meta_storage[..], &mut tc_tx_payload_storage[..]);
        let mut time_critical_socket = udp::Socket::new(tc_rx_buffer, tc_tx_buffer);
        time_critical_socket.bind(319).unwrap();

        let rx_buffer =
            udp::PacketBuffer::new(&mut rx_meta_storage[..], &mut rx_payload_storage[..]);
        let tx_buffer =
            udp::PacketBuffer::new(&mut tx_meta_storage[..], &mut tx_payload_storage[..]);
        let mut general_socket = udp::Socket::new(rx_buffer, tx_buffer);
        general_socket.bind(320).unwrap();

        // Register socket
        let mut sockets = SocketSet::new(&mut sockets[..]);
        let time_critical_socket = sockets.add(time_critical_socket);
        let general_socket = sockets.add(general_socket);
        let dhcp_socket = sockets.add(dhcp_socket);

        defmt::info!("Enabling interrupts");
        dma.enable_interrupt();

        // Setup PHY
        let mut phy = LAN8742A::new(mac, 0);

        phy.phy_init();

        defmt::info!("Waiting for link up.");

        while !phy.phy_link_up() {}

        defmt::info!("Link up.");

        if let Some(speed) = phy.link_speed().map(|s| match s {
            PhySpeed::HalfDuplexBase10T => mac::Speed::HalfDuplexBase10T,
            PhySpeed::FullDuplexBase10T => mac::Speed::FullDuplexBase10T,
            PhySpeed::HalfDuplexBase100Tx => mac::Speed::HalfDuplexBase100Tx,
            PhySpeed::FullDuplexBase100Tx => mac::Speed::FullDuplexBase100Tx,
        }) {
            phy.get_miim().set_speed(speed);
            defmt::info!("Detected link speed: {}", speed);
        } else {
            defmt::warn!("Failed to detect link speed.");
        }

        // Setup PPS
        ptp.enable_pps(pps);
        ptp.set_pps_freq(0);
        // todo handle addend

        // Setup statime
        static PTP_CLOCK: StaticCell<PtpClock> = StaticCell::new();
        let ptp_clock = &*PTP_CLOCK.init(PtpClock::new(ptp));

        let instance_config = InstanceConfig {
            clock_identity: statime::ClockIdentity(*b"TODOFIXM"),
            priority_1: 255,
            priority_2: 255,
            domain_number: 0,
            slave_only: false,
            sdo_id: SdoId::new(0).unwrap(),
        };
        let time_properties_ds = statime::TimePropertiesDS::new_arbitrary_time(
            false,
            false,
            statime::TimeSource::InternalOscillator,
        );
        static PTP_INSTANCE: StaticCell<PtpInstance<&PtpClock, BasicFilter>> = StaticCell::new();
        let ptp_instance = &*PTP_INSTANCE.init(PtpInstance::new(
            instance_config,
            time_properties_ds,
            ptp_clock,
        ));

        let port_config = PortConfig {
            delay_mechanism: statime::DelayMechanism::E2E {
                interval: Interval::ONE_SECOND,
            },
            announce_interval: Interval::ONE_SECOND,
            announce_receipt_timeout: 42,
            sync_interval: Interval::ONE_SECOND,
            master_only: false,
            delay_asymmetry: Duration::ZERO,
        };
        let filter_config = 1.0;
        let rng = p.RNG.init();
        let ptp_port =
            PtpPort::InBmca(ptp_instance.add_port(port_config, filter_config, ptp_clock, rng));

        type Empty = ();
        let (please_poll_s, please_poll_r) = make_channel!(Empty, 1);
        let (tx_done_s, tx_done_r) = make_channel!(Empty, 1);
        type TimerMsg = (TimerName, core::time::Duration);
        let (timer_sender, timer_receiver) = make_channel!(TimerMsg, 4);
        type PacketIdMsg = (statime::TimestampContext, PacketId);
        let (packet_id_sender, packet_id_receiver) = make_channel!(PacketIdMsg, 16);

        let net = NetworkStack {
            dma,
            iface: interface,
            sockets,
        };

        unwrap!(blinky::spawn());
        time_critical_listen::spawn(
            time_critical_socket,
            timer_sender.clone(),
            packet_id_sender.clone(),
            time_critical_socket,
            general_socket,
        )
        .unwrap_or_else(|_| defmt::panic!("Failed to start time_critical_listen"));
        general_listen::spawn(
            general_socket,
            timer_sender.clone(),
            packet_id_sender.clone(),
            time_critical_socket,
            general_socket,
        )
        .unwrap_or_else(|_| defmt::panic!("Failed to start general_listen"));
        send_timestamp_grabber::spawn(
            timer_sender.clone(),
            packet_id_receiver,
            tx_done_r,
            packet_id_sender.clone(),
            time_critical_socket,
            general_socket,
        )
        .unwrap_or_else(|_| defmt::panic!("Failed to start send_timestamp_grabber"));
        poll_smoltcp::spawn(please_poll_r)
            .unwrap_or_else(|_| defmt::panic!("Failed to start poll_smoltcp"));
        statime_timers::spawn(
            timer_receiver,
            timer_sender.clone(),
            packet_id_sender.clone(),
            time_critical_socket,
            general_socket,
        )
        .unwrap_or_else(|_| defmt::panic!("Failed to start timers"));
        instance_bmca::spawn(
            timer_sender.clone(),
            packet_id_sender.clone(),
            time_critical_socket,
            general_socket,
        )
        .unwrap_or_else(|_| defmt::panic!("Failed to start instance bmca"));
        dhcp::spawn(dhcp_socket).unwrap_or_else(|_| defmt::panic!("Failed to start dhcp"));

        (
            Shared {
                net,
                ptp_instance,
                ptp_port,
            },
            Local {
                led_pin,
                please_poll_s,
                tx_done_s,
            },
        )
    }

    #[task(shared=[net, ptp_instance, ptp_port], priority = 1)]
    async fn instance_bmca(
        mut cx: instance_bmca::Context,
        mut timer_resets_sender: Sender<'static, (TimerName, core::time::Duration), 4>,
        mut packet_id_sender: Sender<'static, (statime::TimestampContext, PacketId), 16>,
        time_critical_socket: SocketHandle,
        general_socket: SocketHandle,
    ) {
        let net = &mut cx.shared.net;

        loop {
            let wait_duration = (&mut cx.shared.ptp_instance, &mut cx.shared.ptp_port).lock(
                |ptp_instance, ptp_port| {
                    ptp_instance.bmca(&mut [ptp_port.as_bmca_mode()]);
                    handle_port_actions(
                        ptp_port.to_running(),
                        &mut timer_resets_sender,
                        &mut packet_id_sender,
                        net,
                        time_critical_socket,
                        general_socket,
                    );
                    ptp_instance.bmca_interval()
                },
            );

            Systick::delay((wait_duration.as_millis() as u64).millis()).await;
        }
    }

    #[task(shared=[net, ptp_port], priority = 1)]
    async fn statime_timers(
        mut cx: statime_timers::Context,
        mut timer_resets: Receiver<'static, (TimerName, core::time::Duration), 4>,
        mut timer_resets_sender: Sender<'static, (TimerName, core::time::Duration), 4>,
        mut packet_id_sender: Sender<'static, (statime::TimestampContext, PacketId), 16>,
        time_critical_socket: SocketHandle,
        general_socket: SocketHandle,
    ) {
        let net = &mut cx.shared.net;
        let port = &mut cx.shared.ptp_port;
        let packet_id_sender = &mut packet_id_sender;

        let mut announce_timer_delay = core::pin::pin!(Systick::delay(24u64.hours()).fuse());
        let mut sync_timer_delay = core::pin::pin!(Systick::delay(24u64.hours()).fuse());
        let mut delay_request_timer_delay = core::pin::pin!(Systick::delay(24u64.hours()).fuse());
        let mut announce_receipt_timer_delay =
            core::pin::pin!(Systick::delay(24u64.hours()).fuse());
        let mut filter_update_timer_delay = core::pin::pin!(Systick::delay(24u64.hours()).fuse());

        loop {
            futures::select_biased! {
                _ = announce_timer_delay => {
                    with_port(port, |port| handle_port_actions(port.handle_announce_timer(), &mut timer_resets_sender, packet_id_sender, net, time_critical_socket, general_socket));
                }
                _ = sync_timer_delay => {
                    with_port(port, |port| handle_port_actions(port.handle_sync_timer(), &mut timer_resets_sender, packet_id_sender, net, time_critical_socket, general_socket));
                }
                _ = delay_request_timer_delay => {
                    with_port(port, |port| handle_port_actions(port.handle_delay_request_timer(), &mut timer_resets_sender, packet_id_sender, net, time_critical_socket, general_socket));
                }
                _ = announce_receipt_timer_delay => {
                    with_port(port, |port| handle_port_actions(port.handle_announce_receipt_timer(), &mut timer_resets_sender, packet_id_sender, net, time_critical_socket, general_socket));
                }
                _ = filter_update_timer_delay => {
                    with_port(port, |port| handle_port_actions(port.handle_filter_update_timer(), &mut timer_resets_sender, packet_id_sender, net, time_critical_socket, general_socket));
                }
                reset = timer_resets.recv().fuse() => {
                    let (timer, delay_time) = reset.unwrap();

                    let delay = match timer {
                        TimerName::Announce => &mut announce_timer_delay,
                        TimerName::Sync => &mut sync_timer_delay,
                        TimerName::DelayRequest => &mut delay_request_timer_delay,
                        TimerName::AnnounceReceipt => &mut announce_receipt_timer_delay,
                        TimerName::FilterUpdate => &mut filter_update_timer_delay,
                    };

                    delay.set(Systick::delay((delay_time.as_millis() as u64).millis()).fuse());
                }
            }
        }
    }

    #[task(shared = [net, ptp_port], priority = 1)]
    async fn send_timestamp_grabber(
        mut cx: send_timestamp_grabber::Context,
        mut timer_resets_sender: Sender<'static, (TimerName, core::time::Duration), 4>,
        mut packet_id_receiver: Receiver<'static, (statime::TimestampContext, PacketId), 16>,
        mut tx_done_r: Receiver<'static, (), 1>,
        mut packet_id_sender: Sender<'static, (statime::TimestampContext, PacketId), 16>,
        time_critical_socket: SocketHandle,
        general_socket: SocketHandle,
    ) {
        let net = &mut cx.shared.net;
        let ptp_port = &mut cx.shared.ptp_port;

        loop {
            let (ts_cx, pid) = packet_id_receiver.recv().await.unwrap();
            match get_timestamp(net, &mut tx_done_r, &pid).await {
                Ok(timestamp) => with_port(ptp_port, |p| {
                    let timestamp = stm_time_to_statime(timestamp);
                    let actions = p.handle_send_timestamp(ts_cx, timestamp);
                    handle_port_actions(
                        actions,
                        &mut timer_resets_sender,
                        &mut packet_id_sender,
                        net,
                        time_critical_socket,
                        general_socket,
                    );
                }),
                Err(e) => defmt::error!(
                    "Failed to get timestamp for packet id {}, because {}",
                    pid,
                    e
                ),
            }
        }
    }

    #[task(local = [led_pin], priority = 1)]
    async fn blinky(cx: blinky::Context) {
        let led = cx.local.led_pin;
        loop {
            Systick::delay(500u64.millis()).await;
            led.set_high();
            Systick::delay(500u64.millis()).await;
            led.set_low();
        }
    }

    #[task(shared = [net, ptp_port], priority = 1)]
    async fn time_critical_listen(
        mut cx: time_critical_listen::Context,
        socket: SocketHandle,
        mut timer_sender: Sender<'static, (TimerName, core::time::Duration), 4>,
        mut packet_id_sender: Sender<'static, (statime::TimestampContext, PacketId), 16>,
        time_critical_socket: SocketHandle,
        general_socket: SocketHandle,
    ) {
        listen_and_handle::<true>(
            &mut cx.shared.net,
            socket,
            &mut cx.shared.ptp_port,
            &mut timer_sender,
            &mut packet_id_sender,
            time_critical_socket,
            general_socket,
        )
        .await
    }

    #[task(shared = [net, ptp_port], priority = 1)]
    async fn general_listen(
        mut cx: general_listen::Context,
        recv_socket: SocketHandle,
        mut timer_sender: Sender<'static, (TimerName, core::time::Duration), 4>,
        mut packet_id_sender: Sender<'static, (statime::TimestampContext, PacketId), 16>,
        time_critical_socket: SocketHandle,
        general_socket: SocketHandle,
    ) {
        listen_and_handle::<false>(
            &mut cx.shared.net,
            recv_socket,
            &mut cx.shared.ptp_port,
            &mut timer_sender,
            &mut packet_id_sender,
            time_critical_socket,
            general_socket,
        )
        .await
    }

    async fn listen_and_handle<const IS_TIME_CRITICAL: bool>(
        net: &mut impl Mutex<T = NetworkStack>,
        recv_socket: SocketHandle,
        port: &mut impl Mutex<T = PtpPort>,
        timer_sender: &mut Sender<'static, (TimerName, core::time::Duration), 4>,
        packet_id_sender: &mut Sender<'static, (statime::TimestampContext, PacketId), 16>,
        time_critical_socket: SocketHandle,
        general_socket: SocketHandle,
    ) {
        let mut buffer = [0u8; 1500];
        loop {
            let (len, timestamp) = match recv_slice(net, recv_socket, &mut buffer).await {
                Ok(ok) => ok,
                Err(e) => {
                    defmt::error!("Failed to receive a packet because: {}", e);
                    continue;
                }
            };
            let data = &buffer[..len];

            with_port(port, |p| {
                let port_actions = if IS_TIME_CRITICAL {
                    let timestamp = stm_time_to_statime(timestamp);
                    p.handle_timecritical_receive(data, timestamp)
                } else {
                    p.handle_general_receive(data)
                };

                handle_port_actions(
                    port_actions,
                    timer_sender,
                    packet_id_sender,
                    net,
                    time_critical_socket,
                    general_socket,
                );
            });
        }
    }

    #[task(shared = [net], priority = 1)]
    async fn poll_smoltcp(
        mut cx: poll_smoltcp::Context,
        mut please_poll: Receiver<'static, (), 1>,
    ) {
        loop {
            // Let smoltcp handle its things
            let delay_millis = cx.shared.net.lock(|net| {
                net.poll();
                net.poll_delay().map(|d| d.total_millis())
            });

            // And wait until it wants to get polled again, we want to send something, or we
            // received something
            if let Some(delay_millis) = delay_millis {
                let delay = delay_millis.millis();
                select_biased! {
                    _ = Systick::delay(delay).fuse() => (),
                    _ = please_poll.recv().fuse() => (),
                };
            } else {
                let _ = please_poll.recv().await;
            }
        }
    }

    #[task(binds = ETH, local = [please_poll_s, tx_done_s], priority = 2)]
    fn eth_interrupt(cx: eth_interrupt::Context) {
        let reason = stm32_eth::eth_interrupt_handler();

        if reason.rx {
            let _ = cx.local.please_poll_s.try_send(());
        }

        if reason.tx {
            let _ = cx.local.tx_done_s.try_send(());
        }
    }

    #[task(shared = [net], priority = 1)]
    async fn dhcp(mut cx: dhcp::Context, dhcp_handle: SocketHandle) {
        loop {
            core::future::poll_fn(|ctx| {
                cx.shared.net.lock(|net| {
                    let dhcp_socket = net.sockets.get_mut::<dhcpv4::Socket>(dhcp_handle);
                    dhcp_socket.register_waker(ctx.waker());

                    match dhcp_socket.poll() {
                        Some(dhcpv4::Event::Deconfigured) => {
                            defmt::warn!("DHCP got deconfigured");
                            net.iface.update_ip_addrs(|addrs| {
                                let dest = addrs.iter_mut().next().unwrap();
                                *dest = IpCidr::Ipv4(Ipv4Cidr::new(Ipv4Address::UNSPECIFIED, 0));
                            });
                            net.iface.routes_mut().remove_default_ipv4_route();
                            Poll::Pending
                        }
                        Some(dhcpv4::Event::Configured(config)) => {
                            defmt::debug!("DHCP config acquired!");

                            defmt::debug!("IP address:      {}", config.address);
                            net.iface.update_ip_addrs(|addrs| {
                                let dest = addrs.iter_mut().next().unwrap();
                                *dest = IpCidr::Ipv4(config.address);
                            });
                            if let Some(router) = config.router {
                                defmt::debug!("Default gateway: {}", router);
                                net.iface
                                    .routes_mut()
                                    .add_default_ipv4_route(router)
                                    .unwrap();
                            } else {
                                defmt::debug!("Default gateway: None");
                                net.iface.routes_mut().remove_default_ipv4_route();
                            }

                            for (i, s) in config.dns_servers.iter().enumerate() {
                                defmt::debug!("DNS server {}:    {}", i, s);
                            }
                            Poll::Ready(())
                        }
                        None => Poll::Pending,
                    }
                })
            })
            .await;
        }
    }
}

#[derive(Debug, Clone, Copy, defmt::Format)]
enum TimestampError {
    PacketIdNotFound(PacketIdNotFound),
    NoTimestampRecorded,
}

impl From<PacketIdNotFound> for TimestampError {
    fn from(value: PacketIdNotFound) -> Self {
        Self::PacketIdNotFound(value)
    }
}

#[derive(Debug, Clone, Copy, defmt::Format)]
enum RecvError {
    Exhausted,
    PacketIdNotFound(PacketIdNotFound),
    NoTimestampRecorded,
}

impl From<PacketIdNotFound> for RecvError {
    fn from(value: PacketIdNotFound) -> Self {
        Self::PacketIdNotFound(value)
    }
}

impl From<udp::RecvError> for RecvError {
    fn from(value: udp::RecvError) -> Self {
        match value {
            udp::RecvError::Exhausted => Self::Exhausted,
        }
    }
}

async fn recv_slice(
    net: &mut impl Mutex<T = ethernet::NetworkStack>,
    socket: SocketHandle,
    buffer: &mut [u8],
) -> Result<(usize, Timestamp), RecvError> {
    poll_fn(|cx| {
        let result = net.lock(|net| {
            // Get next packet (if any)
            let socket: &mut udp::Socket = net.sockets.get_mut(socket);
            socket.register_recv_waker(cx.waker());
            let (len, meta) = socket.recv_slice(buffer)?;

            // Get the timestamp
            let packet_id = PacketId::from(meta.meta);
            let timestamp = match net.dma.rx_timestamp(&packet_id) {
                Ok(Some(ts)) => ts,
                Ok(None) => return Err(RecvError::NoTimestampRecorded),
                Err(e) => return Err(e.into()),
            };

            // Return the buffer length and timestamp
            Ok((len, timestamp))
        });

        match result {
            Ok(r) => Poll::Ready(Ok(r)),
            Err(RecvError::Exhausted) => Poll::Pending,
            e @ Err(_) => Poll::Ready(e),
        }
    })
    .await
}

fn send(
    net: &mut impl Mutex<T = ethernet::NetworkStack>,
    socket: SocketHandle,
    to: &smoltcp::wire::IpEndpoint,
    data: &[u8],
) -> Result<PacketId, udp::SendError> {
    net.lock(|net| {
        // Get an Id to track our packet
        let packet_id = net.dma.next_packet_id();

        // Combine the receiver with the packet id
        let mut meta: UdpMetadata = (*to).into();
        meta.meta = packet_id.clone().into();

        // Actually send the packet
        net.sockets
            .get_mut::<udp::Socket>(socket)
            .send_slice(data, meta)?;

        // Send out the packet, this makes sure the MAC actually sees the packet and
        // knows about the packet_id
        net.poll();

        Ok(packet_id)
    })
}

async fn get_timestamp(
    net: &mut impl Mutex<T = NetworkStack>,
    tx_done_r: &mut Receiver<'static, (), 1>,
    packet_id: &PacketId,
) -> Result<Timestamp, TimestampError> {
    let mut tries = 0;

    let timestamp = loop {
        let poll = net.lock(|net| net.dma.poll_tx_timestamp(packet_id));

        async fn tx_done(tx_done_r: &mut Receiver<'static, (), 1>) -> Result<(), TimestampError> {
            if tx_done_r.recv().await.is_err() {
                return Err(TimestampError::NoTimestampRecorded);
            }
            Ok(())
        }

        match poll {
            Poll::Ready(Ok(ts)) => break ts,
            Poll::Ready(Err(e @ PacketIdNotFound)) => {
                // Smoltcp sometimes does not send the package immediately (eg because of ARP)
                // so we retry a few tiems TODO maybe add a way to ask smoltcp
                // for if a package with a given Id still is in the send buffer
                if tries < 5 {
                    defmt::warn!("Package with Id {} not yet send... waiting...", packet_id);
                    tries += 1;
                    tx_done(tx_done_r).await?;
                } else {
                    return Err(e.into());
                }
            }
            Poll::Pending => tx_done(tx_done_r).await?,
        };
    };

    let timestamp = match timestamp {
        Some(ts) => ts,
        None => return Err(TimestampError::NoTimestampRecorded),
    };

    Ok(timestamp)
}

fn with_port<F, R>(port: &mut impl Mutex<T = PtpPort>, f: F) -> R
where
    F: FnOnce(&mut StmPort<Running>) -> R,
{
    port.lock(|port| {
        let running_port = match port {
            PtpPort::None => panic!("Port was left in None state..."),
            PtpPort::Running(r) => r,
            PtpPort::InBmca(_) => panic!("Port was left in InBmca state..."),
        };

        f(running_port)
    })
}

fn handle_port_actions(
    actions: statime::PortActionIterator<'_>,
    timer_sender: &mut Sender<'static, (TimerName, core::time::Duration), 4>,
    packet_id_sender: &mut Sender<'static, (statime::TimestampContext, PacketId), 16>,
    net: &mut impl Mutex<T = ethernet::NetworkStack>,
    time_critical_socket: SocketHandle,
    general_socket: SocketHandle,
) {
    for action in actions {
        match action {
            PortAction::SendTimeCritical { context, data } => {
                const TO: IpEndpoint = IpEndpoint {
                    addr: IpAddress::v4(224, 0, 1, 129),
                    port: 319,
                };
                match send(net, time_critical_socket, &TO, data) {
                    Ok(pid) => packet_id_sender.try_send((context, pid)).unwrap(),
                    Err(e) => defmt::error!("Failed to send time critical packet, because: {}", e),
                }
            }
            PortAction::SendGeneral { data } => {
                const TO: IpEndpoint = IpEndpoint {
                    addr: IpAddress::v4(224, 0, 1, 129),
                    port: 320,
                };
                match send(net, general_socket, &TO, data) {
                    Ok(_) => (),
                    Err(e) => defmt::error!("Failed to send general packet, because: {}", e),
                }
            }
            PortAction::ResetAnnounceTimer { duration } => {
                timer_sender
                    .try_send((TimerName::Announce, duration))
                    .unwrap();
            }
            PortAction::ResetSyncTimer { duration } => {
                timer_sender.try_send((TimerName::Sync, duration)).unwrap();
            }
            PortAction::ResetDelayRequestTimer { duration } => {
                timer_sender
                    .try_send((TimerName::DelayRequest, duration))
                    .unwrap();
            }
            PortAction::ResetAnnounceReceiptTimer { duration } => {
                timer_sender
                    .try_send((TimerName::AnnounceReceipt, duration))
                    .unwrap();
            }
            PortAction::ResetFilterUpdateTimer { duration } => {
                timer_sender
                    .try_send((TimerName::FilterUpdate, duration))
                    .unwrap();
            }
        }
    }
}
