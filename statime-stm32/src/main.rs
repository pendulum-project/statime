#![no_main]
#![no_std]
#![feature(type_alias_impl_trait)]

use defmt_rtt as _; // global logger
use panic_probe as _; // panic handler
use rtic::app;

pub mod ethernet;
pub mod ptp_clock;

#[app(device = stm32f7xx_hal::pac, dispatchers = [CAN1_RX0, CAN1_RX1, CAN1_TX])]
mod app {

    use defmt::unwrap;
    use futures::{future::FutureExt, select_biased};
    use ieee802_3_miim::{
        phy::{PhySpeed, LAN8742A},
        Phy,
    };
    use rtic_monotonics::systick::{fugit::RateExtU32, ExtU32, Systick};
    use rtic_sync::{
        channel::{Receiver, Sender},
        make_channel,
    };
    use smoltcp::{
        iface::{Config, Interface, SocketHandle, SocketSet},
        socket::udp::{self},
        wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address},
    };
    use stm32_eth::{mac, EthPins, Parts, PartsIn};
    use stm32f7xx_hal::{
        gpio::{Output, Pin, Speed},
        prelude::*,
    };

    use crate::ethernet::{NetworkResources, NetworkStack, CLIENT_ADDR};

    #[shared]
    struct Shared {
        net: NetworkStack,
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

        defmt::info!("Set IPs to: {}", interface.ip_addrs());

        // Setup socket
        let rx_buffer =
            udp::PacketBuffer::new(&mut rx_meta_storage[..], &mut rx_payload_storage[..]);
        let tx_buffer =
            udp::PacketBuffer::new(&mut tx_meta_storage[..], &mut tx_payload_storage[..]);
        let mut udp_socket = udp::Socket::new(rx_buffer, tx_buffer);
        udp_socket.bind(1337).unwrap();

        // Register socket
        let mut sockets = SocketSet::new(&mut sockets[..]);
        let udp_socket = sockets.add(udp_socket);

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

        type Empty = ();
        let (please_poll_s, please_poll_r) = make_channel!(Empty, 1);
        let (tx_done_s, tx_done_r) = make_channel!(Empty, 1);

        let net = NetworkStack {
            dma,
            iface: interface,
            sockets,
        };

        unwrap!(blinky::spawn());
        udp_ping::spawn(udp_socket, please_poll_s.clone(), tx_done_r).unwrap();
        poll_smoltcp::spawn(please_poll_r).unwrap();

        (
            Shared { net },
            Local {
                led_pin,
                please_poll_s,
                tx_done_s,
            },
        )
    }

    #[task(local = [led_pin], priority = 1)]
    async fn blinky(cx: blinky::Context) {
        let led = cx.local.led_pin;
        loop {
            Systick::delay(500.millis()).await;
            led.set_high();
            Systick::delay(500.millis()).await;
            led.set_low();
        }
    }

    #[task(shared = [net], priority = 1)]
    async fn udp_ping(
        mut cx: udp_ping::Context,
        socket: SocketHandle,
        mut please_poll: Sender<'static, (), 1>,
        mut tx_done_r: Receiver<'static, (), 1>,
    ) {
        let mut meta: udp::UdpMetadata = smoltcp::wire::IpEndpoint {
            addr: IpAddress::Ipv4(Ipv4Address([10, 0, 0, 1])),
            port: 1337,
        }
        .into();

        loop {
            Systick::delay(1000.millis()).await;

            let packet_id = cx.shared.net.lock(|net| {
                let packet_id = net.dma.next_packet_id();
                meta.meta = packet_id.clone().into();

                defmt::println!("to: {}", meta);

                let result = net
                    .sockets
                    .get_mut::<udp::Socket>(socket)
                    .send_slice(&[0x42; 42], meta);

                match result {
                    Ok(()) => (),
                    Err(e) => defmt::error!("Could not sent UDP packet because: {}", e),
                }

                // Send out the packet, this makes sure the MAC actually dets the packet and
                // knows about the packet_id
                net.poll();

                packet_id
            });

            defmt::trace!("sent udp");

            let timestamp = loop {
                let poll = cx
                    .shared
                    .net
                    .lock(|net| net.dma.poll_tx_timestamp(&packet_id));

                let _ = match poll {
                    core::task::Poll::Ready(result) => break result,
                    core::task::Poll::Pending => tx_done_r.recv().await,
                };
            };

            defmt::info!("Timestamp for {}: {}", packet_id, timestamp);
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
                let delay = u32::try_from(delay_millis).unwrap_or(1_000_000).millis();
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
