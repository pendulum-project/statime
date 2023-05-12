#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_net::{tcp::TcpSocket, Ipv4Address, Stack, StackResources};
use embassy_stm32::{
    eth::{generic_smi::GenericSMI, Ethernet, PacketQueue},
    interrupt,
    peripherals::ETH,
    rng::Rng,
    time::mhz,
    Config,
};
use embassy_time::{Duration, Timer};
use embedded_io::asynch::Write;
use panic_probe as _;
use rand_core::RngCore;
use static_cell::StaticCell;

mod eth;

macro_rules! singleton {
    ($val:expr) => {{
        type T = impl Sized;
        static STATIC_CELL: StaticCell<T> = StaticCell::new();
        let (x,) = STATIC_CELL.init(($val,));
        x
    }};
}

type Device = Ethernet<'static, ETH, GenericSMI>;

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<Device>) -> ! {
    stack.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) -> ! {
    let mut config = Config::default();
    config.rcc.sys_ck = Some(mhz(200));
    let p = embassy_stm32::init(config);

    info!("Hello World!");

    // Generate random seed.
    let mut rng = Rng::new(p.RNG);
    let mut seed = [0; 8];
    rng.fill_bytes(&mut seed);
    let seed = u64::from_le_bytes(seed);

    let eth_int = interrupt::take!(ETH);
    let mac_addr = [0x00, 0x00, 0xde, 0xad, 0xbe, 0xef];

    let device = Ethernet::new(
        singleton!(PacketQueue::<16, 16>::new()),
        p.ETH,
        eth_int,
        p.PA1,
        p.PA2,
        p.PC1,
        p.PA7,
        p.PC4,
        p.PC5,
        p.PG13,
        p.PB13,
        p.PG11,
        GenericSMI,
        mac_addr,
        0,
    );

    let config = embassy_net::Config::Dhcp(Default::default());
    // let config = embassy_net::Config::Static(embassy_net::StaticConfig {
    //    address: Ipv4Cidr::new(Ipv4Address::new(10, 42, 0, 61), 24),
    //    dns_servers: Vec::new(),
    //    gateway: Some(Ipv4Address::new(10, 42, 0, 1)),
    //});

    // Init network stack
    let stack = &*singleton!(Stack::new(
        device,
        config,
        singleton!(StackResources::<2>::new()),
        seed
    ));

    // Launch network task
    unwrap!(spawner.spawn(net_task(&stack)));

    info!("Network task initialized");

    // Then we can use it!
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    loop {
        let mut socket = TcpSocket::new(&stack, &mut rx_buffer, &mut tx_buffer);

        socket.set_timeout(Some(embassy_net::SmolDuration::from_secs(10)));

        let remote_endpoint = (Ipv4Address::new(10, 42, 0, 1), 8000);
        info!("connecting...");
        let r = socket.connect(remote_endpoint).await;
        if let Err(e) = r {
            info!("connect error: {:?}", e);
            continue;
        }
        info!("connected!");
        let buf = [0; 1024];
        loop {
            let r = socket.write_all(&buf).await;
            if let Err(e) = r {
                info!("write error: {:?}", e);
                break;
            }
            Timer::after(Duration::from_secs(1)).await;
        }
    }
}
