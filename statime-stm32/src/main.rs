#![no_main]
#![no_std]
#![feature(type_alias_impl_trait)]

use core::task::Poll;

use defmt::unwrap;
use defmt_rtt as _;
use embassy_sync::waitqueue::WakerRegistration;
use ethernet::{NetworkResources, NetworkStack};
use futures::future::{poll_fn, FutureExt};
use ieee802_3_miim::{
    phy::{PhySpeed, LAN8742A},
    Phy,
};
use panic_probe as _;
use ptp_clock::PtpClock;
use rtic::{app, Mutex};
use rtic_monotonics::systick::{ExtU64, Systick};
use rtic_sync::{channel::Receiver, make_channel};
use smoltcp::{
    iface::{Config, Interface, SocketHandle, SocketSet},
    socket::{
        dhcpv4,
        udp::{self},
    },
    wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address, Ipv4Cidr},
};
use static_cell::StaticCell;
use statime::{BasicFilter, Duration, InstanceConfig, Interval, PortConfig, PtpInstance, SdoId};
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
    signature::Uid,
};

use crate::ptp_clock::stm_time_to_statime;

mod ethernet;
mod port;
mod ptp_clock;

defmt::timestamp!("{=u64:iso8601ms}", {
    let time = stm32_eth::ptp::EthernetPTP::get_time();
    time.seconds() as u64 * 1_000 + (time.subseconds().nanos() / 1000000) as u64
});

type StmPort<State> = statime::Port<State, Rng, &'static PtpClock, BasicFilter>;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TimerName {
    Announce,
    Sync,
    DelayRequest,
    AnnounceReceipt,
    FilterUpdate,
}

#[app(device = stm32f7xx_hal::pac, dispatchers = [CAN1_RX0])]
mod app {
    use super::*;

    #[shared]
    struct Shared {
        net: NetworkStack,
        ptp_instance: &'static statime::PtpInstance<BasicFilter>,
        ptp_port: port::Port,
        tx_waker: WakerRegistration,
    }

    #[local]
    struct Local {
        led_pin: Pin<'B', 7, Output>,
    }

    #[init(local = [net_resources: NetworkResources = NetworkResources::new()])]
    fn init(cx: init::Context) -> (Shared, Local) {
        let p = cx.device;

        // Setup clocks
        let rcc = p.RCC.constrain();
        let clocks = rcc.cfgr.sysclk(216.MHz()).hclk(216.MHz());
        let clocks = clocks.freeze();

        // Uncomment to see the statime logs at the cost of quite a bit of extra flash
        // usage log_to_defmt::setup();

        // Setup LED
        let gpioa = p.GPIOA.split();
        let gpiob = p.GPIOB.split();
        let gpioc = p.GPIOC.split();
        let gpiog = p.GPIOG.split();
        let led_pin = gpiob.pb7.into_push_pull_output();

        let systick_token = rtic_monotonics::create_systick_token!();
        Systick::start(cx.core.SYST, clocks.sysclk().to_Hz(), systick_token);

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
        } = unwrap!(stm32_eth::new_with_mii(
            ethernet, rx_ring, tx_ring, clocks, eth_pins, mdio, mdc
        )
        .ok());

        let mac_address = generate_mac_address();

        // Setup smoltcp
        let cfg = Config::new(EthernetAddress(mac_address).into());

        let mut interface = Interface::new(cfg, &mut &mut dma, smoltcp::time::Instant::ZERO);

        let dhcp_socket = smoltcp::socket::dhcpv4::Socket::new();

        interface.update_ip_addrs(|a| {
            unwrap!(a.push(IpCidr::new(IpAddress::v4(10, 0, 0, 2), 24)));
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
        unwrap!(time_critical_socket.bind(319));

        let rx_buffer =
            udp::PacketBuffer::new(&mut rx_meta_storage[..], &mut rx_payload_storage[..]);
        let tx_buffer =
            udp::PacketBuffer::new(&mut tx_meta_storage[..], &mut tx_payload_storage[..]);
        let mut general_socket = udp::Socket::new(rx_buffer, tx_buffer);
        unwrap!(general_socket.bind(320));

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
        ptp.set_pps_freq(4);

        // Setup statime
        static PTP_CLOCK: StaticCell<PtpClock> = StaticCell::new();
        let ptp_clock = &*PTP_CLOCK.init(PtpClock::new(ptp));

        let instance_config = InstanceConfig {
            clock_identity: statime::ClockIdentity(eui48_to_eui64(mac_address)),
            priority_1: 255,
            priority_2: 255,
            domain_number: 0,
            slave_only: false,
            sdo_id: unwrap!(SdoId::new(0)),
        };
        let time_properties_ds = statime::TimePropertiesDS::new_arbitrary_time(
            false,
            false,
            statime::TimeSource::InternalOscillator,
        );
        static PTP_INSTANCE: StaticCell<PtpInstance<BasicFilter>> = StaticCell::new();
        let ptp_instance =
            &*PTP_INSTANCE.init(PtpInstance::new(instance_config, time_properties_ds));

        let port_config = PortConfig {
            delay_mechanism: statime::DelayMechanism::E2E {
                interval: Interval::from_log_2(-2),
            },
            announce_interval: Interval::from_log_2(1),
            announce_receipt_timeout: 3,
            sync_interval: Interval::from_log_2(-6),
            master_only: false,
            delay_asymmetry: Duration::ZERO,
        };
        let filter_config = 0.1;
        let rng = p.RNG.init();

        type TimerMsg = (TimerName, core::time::Duration);
        let (timer_sender, timer_receiver) = make_channel!(TimerMsg, 4);
        type PacketIdMsg = (statime::TimestampContext, PacketId);
        let (packet_id_sender, packet_id_receiver) = make_channel!(PacketIdMsg, 16);

        let ptp_port = port::Port::new(
            timer_sender,
            packet_id_sender,
            time_critical_socket,
            general_socket,
            ptp_instance.add_port(port_config, filter_config, ptp_clock, rng),
        );

        let net = NetworkStack {
            dma,
            iface: interface,
            sockets,
        };

        unwrap!(blinky::spawn());
        time_critical_listen::spawn()
            .unwrap_or_else(|_| defmt::panic!("Failed to start time_critical_listen"));
        general_listen::spawn().unwrap_or_else(|_| defmt::panic!("Failed to start general_listen"));
        tx_timestamp_listener::spawn(packet_id_receiver)
            .unwrap_or_else(|_| defmt::panic!("Failed to start send_timestamp_grabber"));
        poll_smoltcp::spawn().unwrap_or_else(|_| defmt::panic!("Failed to start poll_smoltcp"));
        statime_timers::spawn(timer_receiver)
            .unwrap_or_else(|_| defmt::panic!("Failed to start timers"));
        instance_bmca::spawn().unwrap_or_else(|_| defmt::panic!("Failed to start instance bmca"));
        dhcp::spawn(dhcp_socket).unwrap_or_else(|_| defmt::panic!("Failed to start dhcp"));

        (
            Shared {
                net,
                ptp_instance,
                ptp_port,
                tx_waker: WakerRegistration::new(),
            },
            Local { led_pin },
        )
    }

    /// Task that runs the BMCA every required interval
    #[task(shared=[net, ptp_instance, ptp_port], priority = 1)]
    async fn instance_bmca(mut cx: instance_bmca::Context) {
        let net = &mut cx.shared.net;

        loop {
            let wait_duration = (&mut cx.shared.ptp_instance, &mut cx.shared.ptp_port).lock(
                |ptp_instance, ptp_port| {
                    ptp_port.perform_bmca(
                        |bmca_port| {
                            ptp_instance.bmca(&mut [bmca_port]);
                        },
                        net,
                    );

                    ptp_instance.bmca_interval()
                },
            );

            Systick::delay((wait_duration.as_millis() as u64).millis()).await;
        }
    }

    /// Task that runs the timers and lets the port handle the expired timers.
    /// The channel is used for resetting the timers (which comes from the port
    /// actions and get sent here).
    #[task(shared=[net, ptp_port], priority = 0)]
    async fn statime_timers(
        mut cx: statime_timers::Context,
        mut timer_resets: Receiver<'static, (TimerName, core::time::Duration), 4>,
    ) {
        let net = &mut cx.shared.net;

        let mut announce_timer_delay = core::pin::pin!(Systick::delay(24u64.hours()).fuse());
        let mut sync_timer_delay = core::pin::pin!(Systick::delay(24u64.hours()).fuse());
        let mut delay_request_timer_delay = core::pin::pin!(Systick::delay(24u64.hours()).fuse());
        let mut announce_receipt_timer_delay =
            core::pin::pin!(Systick::delay(24u64.hours()).fuse());
        let mut filter_update_timer_delay = core::pin::pin!(Systick::delay(24u64.hours()).fuse());

        loop {
            futures::select_biased! {
                _ = announce_timer_delay => {
                    cx.shared.ptp_port.lock(|port| port.handle_timer(TimerName::Announce, net));
                }
                _ = sync_timer_delay => {
                    cx.shared.ptp_port.lock(|port| port.handle_timer(TimerName::Sync, net));
                }
                _ = delay_request_timer_delay => {
                    cx.shared.ptp_port.lock(|port| port.handle_timer(TimerName::DelayRequest, net));
                }
                _ = announce_receipt_timer_delay => {
                    cx.shared.ptp_port.lock(|port| port.handle_timer(TimerName::AnnounceReceipt, net));
                }
                _ = filter_update_timer_delay => {
                    cx.shared.ptp_port.lock(|port| port.handle_timer(TimerName::FilterUpdate, net));
                }
                reset = timer_resets.recv().fuse() => {
                    let (timer, delay_time) = unwrap!(reset.ok());

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

    #[task(shared = [net, ptp_port, tx_waker], priority = 0)]
    async fn tx_timestamp_listener(
        mut cx: tx_timestamp_listener::Context,
        mut packet_id_receiver: Receiver<'static, (statime::TimestampContext, PacketId), 16>,
    ) {
        let tx_waker = &mut cx.shared.tx_waker;
        let net = &mut cx.shared.net;
        let ptp_port = &mut cx.shared.ptp_port;

        loop {
            // Wait for the next (smoltcp) packet id and its (statime) timestamp context
            let (timestamp_context, packet_id) = unwrap!(packet_id_receiver.recv().await.ok());

            // We try a limited amount of times since the queued packet might not be sent
            // first
            let mut tries = 10;

            let timestamp = core::future::poll_fn(|ctx| {
                // Register to wake up after every tx packet has been sent
                tx_waker.lock(|tx_waker| tx_waker.register(ctx.waker()));

                // Keep polling as long as we have tries left
                match net.lock(|net| net.dma.poll_tx_timestamp(&packet_id)) {
                    Poll::Ready(Ok(ts)) => Poll::Ready(ts),
                    Poll::Ready(Err(_)) | Poll::Pending => {
                        if tries > 0 {
                            tries -= 1;
                            Poll::Pending
                        } else {
                            Poll::Ready(None)
                        }
                    }
                }
            })
            .await;

            match timestamp {
                Some(timestamp) => ptp_port.lock(|port| {
                    port.handle_send_timestamp(
                        timestamp_context,
                        stm_time_to_statime(timestamp),
                        net,
                    );
                }),
                None => defmt::error!("Failed to get timestamp for packet id {}", packet_id,),
            }
        }
    }

    #[task(local = [led_pin], priority = 0)]
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
    async fn time_critical_listen(mut cx: time_critical_listen::Context) {
        let socket = cx
            .shared
            .ptp_port
            .lock(|ptp_port| ptp_port.time_critical_socket());

        listen_and_handle::<true>(&mut cx.shared.net, socket, &mut cx.shared.ptp_port).await
    }

    #[task(shared = [net, ptp_port], priority = 0)]
    async fn general_listen(mut cx: general_listen::Context) {
        let socket = cx
            .shared
            .ptp_port
            .lock(|ptp_port| ptp_port.general_socket());

        listen_and_handle::<false>(&mut cx.shared.net, socket, &mut cx.shared.ptp_port).await
    }

    async fn listen_and_handle<const IS_TIME_CRITICAL: bool>(
        net: &mut impl Mutex<T = NetworkStack>,
        socket: SocketHandle,
        port: &mut impl Mutex<T = port::Port>,
    ) {
        let mut buffer = [0u8; 1500];
        loop {
            let (len, timestamp) = match recv_slice(net, socket, &mut buffer).await {
                Ok(ok) => ok,
                Err(e) => {
                    defmt::error!("Failed to receive a packet because: {}", e);
                    continue;
                }
            };
            let data = &buffer[..len];

            port.lock(|port| {
                if IS_TIME_CRITICAL {
                    port.handle_timecritical_receive(data, stm_time_to_statime(timestamp), net);
                } else {
                    port.handle_general_receive(data, net);
                };
            });
        }
    }

    #[task(shared = [net], priority = 0)]
    async fn poll_smoltcp(mut cx: poll_smoltcp::Context) {
        loop {
            // Let smoltcp handle its things
            let delay_millis = cx
                .shared
                .net
                .lock(|net| {
                    net.poll();
                    net.poll_delay().map(|d| d.total_millis())
                })
                .unwrap_or(50);

            Systick::delay(delay_millis.millis()).await;
        }
    }

    #[task(binds = ETH, shared = [net, tx_waker], priority = 2)]
    fn eth_interrupt(mut cx: eth_interrupt::Context) {
        let reason = stm32_eth::eth_interrupt_handler();

        if reason.tx {
            cx.shared.tx_waker.lock(|tx_waker| tx_waker.wake());
        }

        cx.shared.net.lock(|net| {
            net.poll();
        });
    }

    #[task(shared = [net], priority = 0)]
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
                                let dest = unwrap!(addrs.iter_mut().next());
                                *dest = IpCidr::Ipv4(Ipv4Cidr::new(Ipv4Address::UNSPECIFIED, 0));
                            });
                            net.iface.routes_mut().remove_default_ipv4_route();
                            Poll::Pending
                        }
                        Some(dhcpv4::Event::Configured(config)) => {
                            defmt::debug!("DHCP config acquired!");

                            defmt::debug!("IP address:      {}", config.address);
                            net.iface.update_ip_addrs(|addrs| {
                                let dest = unwrap!(addrs.iter_mut().next());
                                *dest = IpCidr::Ipv4(config.address);
                            });
                            if let Some(router) = config.router {
                                defmt::debug!("Default gateway: {}", router);
                                unwrap!(net.iface.routes_mut().add_default_ipv4_route(router));
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

/// Generate a mac based on the UID of the chip.
///
/// *Note: This is not the proper way to do it.
/// You're supposed to buy a mac address or buy a phy that includes a mac and
/// use that one*
fn generate_mac_address() -> [u8; 6] {
    let mut hasher = adler::Adler32::new();

    // Form the basis of our OUI octets
    let bin_name = env!("CARGO_BIN_NAME").as_bytes();
    hasher.write_slice(bin_name);
    let oui = hasher.checksum().to_ne_bytes();

    // Form the basis of our NIC octets
    let uid: [u8; 12] =
        unsafe { core::mem::transmute_copy::<_, [u8; core::mem::size_of::<Uid>()]>(Uid::get()) };
    hasher.write_slice(&uid);
    let nic = hasher.checksum().to_ne_bytes();

    // To make it adhere to EUI-48, we set it to be a unicast locally administered
    // address
    [
        oui[0] & 0b1111_1100 | 0b0000_0010,
        oui[1],
        oui[2],
        nic[0],
        nic[1],
        nic[2],
    ]
}

fn eui48_to_eui64(address: [u8; 6]) -> [u8; 8] {
    [
        address[0] ^ 0b00000010,
        address[1],
        address[2],
        0xff,
        0xfe,
        address[3],
        address[4],
        address[5],
    ]
}
