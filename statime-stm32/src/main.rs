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
    BasicFilter, Duration, InBmca, InstanceConfig, Interval, PortAction, PortConfig, PtpInstance,
    Running, SdoId,
};
use stm32_eth::{
    dma::{PacketId, PacketIdNotFound},
    mac, EthPins, Parts, PartsIn,
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

type StmPort<State> = statime::Port<State, Rng, &'static PtpClock, BasicFilter>;

pub enum PtpPort {
    Running(StmPort<Running<'static>>),
    InBmca(StmPort<InBmca<'static>>),
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
            priority_1: 0,
            priority_2: 0,
            domain_number: 0,
            slave_only: false,
            sdo_id: SdoId::new(42).unwrap(),
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

        let net = NetworkStack {
            dma,
            iface: interface,
            sockets,
        };

        unwrap!(blinky::spawn());
        time_critical_listen::spawn(time_critical_socket, timer_sender.clone())
            .unwrap_or_else(|_| defmt::panic!("Failed to start time_critical_listen"));
        general_listen::spawn(general_socket, timer_sender.clone())
            .unwrap_or_else(|_| defmt::panic!("Failed to start general_listen"));
        poll_smoltcp::spawn(please_poll_r)
            .unwrap_or_else(|_| defmt::panic!("Failed to start poll_smoltcp"));
        statime_timers::spawn(timer_receiver, timer_sender.clone()).unwrap_or_else(|_| defmt::panic!("Failed to start timers"));

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

    #[task(shared=[ptp_port],priority = 1)]
    async fn statime_timers(
        mut cx: statime_timers::Context,
        mut timer_resets: Receiver<'static, (TimerName, core::time::Duration), 4>,
        mut timer_resets_sender: Sender<'static, (TimerName, core::time::Duration), 4>,
    ) {
        let port = &mut cx.shared.ptp_port;

        let mut announce_timer_delay = core::pin::pin!(Systick::delay(24u64.hours()).fuse());
        let mut sync_timer_delay = core::pin::pin!(Systick::delay(24u64.hours()).fuse());
        let mut delay_request_timer_delay = core::pin::pin!(Systick::delay(24u64.hours()).fuse());
        let mut announce_receipt_timer_delay =
            core::pin::pin!(Systick::delay(24u64.hours()).fuse());
        let mut filter_update_timer_delay = core::pin::pin!(Systick::delay(24u64.hours()).fuse());

        loop {
            futures::select_biased! {
                _ = announce_timer_delay => {
                    with_port(port, |port| handle_port_actions(port.handle_announce_timer(), &mut timer_resets_sender));
                }
                _ = sync_timer_delay => {
                    with_port(port, |port| handle_port_actions(port.handle_sync_timer(), &mut timer_resets_sender));
                }
                _ = delay_request_timer_delay => {
                    with_port(port, |port| handle_port_actions(port.handle_delay_request_timer(), &mut timer_resets_sender));
                }
                _ = announce_receipt_timer_delay => {
                    with_port(port, |port| handle_port_actions(port.handle_announce_receipt_timer(), &mut timer_resets_sender));
                }
                _ = filter_update_timer_delay => {
                    with_port(port, |port| handle_port_actions(port.handle_filter_update_timer(), &mut timer_resets_sender));
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
    ) {
        listen_and_handle::<true>(
            &mut cx.shared.net,
            socket,
            &mut cx.shared.ptp_port,
            &mut timer_sender,
        )
        .await
    }

    #[task(shared = [net, ptp_port], priority = 1)]
    async fn general_listen(
        mut cx: general_listen::Context,
        socket: SocketHandle,
        mut timer_sender: Sender<'static, (TimerName, core::time::Duration), 4>,
    ) {
        listen_and_handle::<false>(
            &mut cx.shared.net,
            socket,
            &mut cx.shared.ptp_port,
            &mut timer_sender,
        )
        .await
    }

    async fn listen_and_handle<const IS_TIME_CRITICAL: bool>(
        net: &mut impl Mutex<T = NetworkStack>,
        socket: SocketHandle,
        port: &mut impl Mutex<T = PtpPort>,
        timer_sender: &mut Sender<'static, (TimerName, core::time::Duration), 4>,
    ) {
        loop {
            let result = recv_with(net, socket, |_from, data, ts| {
                let timestamp = stm_time_to_statime(ts);
                with_port(port, |p| {
                    let port_actions = if IS_TIME_CRITICAL {
                        p.handle_timecritical_receive(data, timestamp)
                    } else {
                        p.handle_general_receive(data)
                    };

                    handle_port_actions(port_actions, timer_sender);
                })
            })
            .await;

            match result {
                Ok(_) => (),
                Err(e) => defmt::error!("Failed to receive time critical: {}", e),
            }
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
                let delay = u64::try_from(delay_millis).unwrap_or(1_000_000).millis();
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
}

#[derive(Debug, Clone, Copy, defmt::Format)]
enum SendError {
    PacketIdNotFound(PacketIdNotFound),
    Unaddressable,
    BufferFull,
    NoTimestampRecorded,
}

impl From<PacketIdNotFound> for SendError {
    fn from(value: PacketIdNotFound) -> Self {
        Self::PacketIdNotFound(value)
    }
}

impl From<udp::SendError> for SendError {
    fn from(value: udp::SendError) -> Self {
        match value {
            udp::SendError::Unaddressable => Self::Unaddressable,
            udp::SendError::BufferFull => Self::BufferFull,
        }
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

async fn recv_with<R>(
    net: &mut impl Mutex<T = ethernet::NetworkStack>,
    socket: SocketHandle,
    handler: impl FnOnce(IpEndpoint, &[u8], stm32_eth::ptp::Timestamp) -> R,
) -> Result<R, RecvError> {
    let mut handler = Some(handler);

    poll_fn(|cx| {
        let result = net.lock(|net| -> Result<R, RecvError> {
            // Get next packet (if any)
            let socket: &mut udp::Socket = net.sockets.get_mut(socket);
            socket.register_recv_waker(cx.waker());
            let (data, meta) = socket.recv()?;
            let from = meta.endpoint;

            // Get the timestamp
            let packet_id = PacketId::from(meta.meta);
            let timestamp = match net.dma.rx_timestamp(&packet_id) {
                Ok(Some(ts)) => ts,
                Ok(None) => return Err(RecvError::NoTimestampRecorded),
                Err(e) => return Err(e.into()),
            };

            // Unpack the handler
            let handler = handler.take().unwrap_or_else(|| {
                defmt::panic!("Polled recv_with after it already returned Ready!")
            });

            // Let the handler handle the rest
            Ok(handler(from, data, timestamp))
        });

        match result {
            Ok(r) => Poll::Ready(Ok(r)),
            Err(RecvError::Exhausted) => Poll::Pending,
            e @ Err(_) => Poll::Ready(e),
        }
    })
    .await
}

async fn send_with_timestamp(
    net: &mut impl Mutex<T = ethernet::NetworkStack>,
    socket: SocketHandle,
    to: &smoltcp::wire::IpEndpoint,
    data: &[u8],
    tx_done_r: &mut Receiver<'static, (), 1>,
) -> Result<stm32_eth::ptp::Timestamp, SendError> {
    let packet_id = net.lock(|net| -> Result<_, SendError> {
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
    })?;

    let mut tries = 0;

    let timestamp = loop {
        let poll = net.lock(|net| net.dma.poll_tx_timestamp(&packet_id));

        async fn tx_done(tx_done_r: &mut Receiver<'static, (), 1>) -> Result<(), SendError> {
            if tx_done_r.recv().await.is_err() {
                return Err(SendError::NoTimestampRecorded);
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
        None => return Err(SendError::NoTimestampRecorded),
    };

    Ok(timestamp)
}

fn with_port<F, R>(port: &mut impl Mutex<T = PtpPort>, f: F) -> R
where
    F: FnOnce(&mut StmPort<Running>) -> R,
{
    port.lock(|port| {
        let running_port = match port {
            PtpPort::Running(r) => r,
            PtpPort::InBmca(_) => panic!("Port was left in InBmca state..."),
        };

        f(running_port)
    })
}

fn handle_port_actions(
    actions: statime::PortActionIterator<'_>,
    timer_sender: &mut Sender<'static, (TimerName, core::time::Duration), 4>,
) {
    for action in actions {
        match action {
            PortAction::SendTimeCritical { context, data } => {
                todo!()
            }
            PortAction::SendGeneral { data } => {
                todo!()
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
