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
    use ieee802_3_miim::{
        phy::{PhySpeed, LAN8742A},
        Phy,
    };
    use rtic_monotonics::{
        systick::{fugit::RateExtU32, ExtU32, Systick},
        Monotonic,
    };
    use smoltcp::{
        iface::{Config, Interface, SocketHandle, SocketSet},
        socket::udp::{self},
        wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address},
    };
    use stm32_eth::{dma::EthernetDMA, mac, EthPins, Parts, PartsIn};
    use stm32f7xx_hal::{
        gpio::{Output, Pin, Speed},
        prelude::*,
    };

    use crate::ethernet::NetworkResources;

    pub struct NetworkStack {
        dma: EthernetDMA<'static, 'static>,
        iface: Interface,
        sockets: SocketSet<'static>,
    }

    impl NetworkStack {
        pub fn poll(&mut self) {
            self.iface
                .poll(now(), &mut &mut self.dma, &mut self.sockets);
        }

        pub fn poll_delay(&mut self) -> Option<smoltcp::time::Duration> {
            self.iface.poll_delay(now(), &self.sockets)
        }
    }

    const CLIENT_ADDR: [u8; 6] = [0x80, 0x00, 0xde, 0xad, 0xbe, 0xef];

    fn now() -> smoltcp::time::Instant {
        let now_micros = Systick::now().ticks() * 1000;
        smoltcp::time::Instant::from_micros(now_micros as i64)
    }

    #[shared]
    struct Shared {
        net: NetworkStack,
        socket: SocketHandle,
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

        let net = NetworkStack {
            dma,
            iface: interface,
            sockets,
        };

        unwrap!(blinky::spawn());
        unwrap!(udp_ping::spawn());
        unwrap!(poll_smoltcp::spawn());

        (
            Shared {
                net,
                socket: udp_socket,
            },
            Local { led_pin },
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

    #[task(shared = [socket, net], priority = 1)]
    async fn udp_ping(cx: udp_ping::Context) {
        let mut net_sock = (cx.shared.net, cx.shared.socket);

        loop {
            Systick::delay(10000.millis()).await;

            let mut meta: udp::UdpMetadata = smoltcp::wire::IpEndpoint {
                addr: IpAddress::Ipv4(Ipv4Address([10, 0, 0, 1])),
                port: 1337,
            }
            .into();

            net_sock.lock(|net, s| {
                let packet_id = net.dma.next_packet_id();
                meta.meta = packet_id.into();

                defmt::println!("to: {}", meta);

                let result = net
                    .sockets
                    .get_mut::<udp::Socket>(*s)
                    .send_slice(&[0x42; 42], meta);

                match result {
                    Ok(_) => (),
                    Err(e) => defmt::error!("Could not sent UDP packet because: {}", e),
                }

                net.poll();
            });
            defmt::trace!("sent udp");
        }
    }

    #[task(shared = [net], priority = 1)]
    async fn poll_smoltcp(mut cx: poll_smoltcp::Context) {
        loop {
            defmt::trace!("poll");
            let delay_millis = cx.shared.net.lock(|net| {
                net.poll();
                net.poll_delay().map(|d| d.total_millis()).unwrap_or(100)
            });

            Systick::delay(u32::try_from(delay_millis).unwrap_or(100).millis()).await;
        }
    }

    #[task(binds = ETH, shared = [net], priority = 2)]
    fn eth_interrupt(mut cx: eth_interrupt::Context) {
        stm32_eth::eth_interrupt_handler();

        cx.shared.net.lock(|net| {
            net.poll();
        })
    }
}
