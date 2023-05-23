#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![feature(async_fn_in_trait)]
#![allow(incomplete_features)]

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_net::{tcp::TcpSocket, Ipv4Address, Ipv4Cidr, Stack, StackResources};
use embassy_stm32::{
    eth::generic_smi::GenericSMI, interrupt, peripherals::ETH, rng::Rng, time::mhz, Config,
};
use embassy_time::{Duration, Timer};
use embedded_io::asynch::Write;
use eth::{Ethernet, PTPClock, PacketQueue};
use fixed::types::{I96F32, U96F32};
use heapless::Vec;
use panic_probe as _;
use rand_core::RngCore;
use static_cell::StaticCell;
use statime::clock::Clock;

mod device;
mod eth;
mod runtime;
mod static_ethernet;

macro_rules! singleton {
    ($val:expr) => {{
        type T = impl Sized;
        static STATIC_CELL: StaticCell<T> = StaticCell::new();
        let (x,) = STATIC_CELL.init(($val,));
        x
    }};
}

type Device = Ethernet<'static, ETH, GenericSMI>;

pub struct StmClock<'d> {
    ptp: PTPClock<'d>,
    multiplier: f64,
}

impl<'d> StmClock<'d> {
    fn new(mut ptp: PTPClock<'d>) -> Self {
        ptp.set_freq(0.0);
        StmClock {
            ptp,
            multiplier: 1.0,
        }
    }
}

impl<'d> Clock for StmClock<'d> {
    type Error = core::convert::Infallible;

    fn now(&self) -> statime::time::Instant {
        let (sec, subsec) = self.ptp.time();
        let inter = 1_000_000_000_u128 * ((sec as u128) << 32) + (subsec as u128);
        statime::time::Instant::from_fixed_nanos(U96F32::from_bits(inter))
    }

    fn quality(&self) -> statime::datastructures::common::ClockQuality {
        statime::datastructures::common::ClockQuality {
            clock_class: 248,
            clock_accuracy: statime::datastructures::common::ClockAccuracy::NS25,
            offset_scaled_log_variance: 0xffff,
        }
    }

    fn adjust(
        &mut self,
        time_offset: statime::time::Duration,
        frequency_multiplier: f64,
        _time_properties_ds: &statime::datastructures::datasets::TimePropertiesDS,
    ) -> Result<(), Self::Error> {
        let offset: I96F32 = time_offset.nanos() * I96F32::from_num(1_000_000_000);
        let is_negative = offset < 0;
        let offset_abs: I96F32 = offset.abs();
        let offset_bits = offset_abs.to_bits() as u128;
        self.ptp.jump_time(
            is_negative,
            (offset_bits >> 32) as _,
            ((offset_bits >> 31) & 0x8ffffff) as _,
        );

        self.multiplier *= frequency_multiplier;
        self.ptp.set_freq((self.multiplier - 1.0) as _);

        Ok(())
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<Device>) -> ! {
    stack.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) -> ! {
    let mut config = Config::default();
    config.rcc.sys_ck = Some(mhz(216));
    let p = embassy_stm32::init(config);

    info!("Hello World!");

    // Generate random seed.
    let mut rng = Rng::new(p.RNG);
    let mut seed = [0; 8];
    rng.fill_bytes(&mut seed);
    let seed = u64::from_le_bytes(seed);

    let eth_int = interrupt::take!(ETH);
    let mac_addr = [0x00, 0x00, 0xde, 0xad, 0xbe, 0xef];

    let (device, ptp) = Ethernet::new_with_ptp(
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
        p.PB5,
        GenericSMI,
        mac_addr,
        0,
    );

    // let config = embassy_net::Config::Dhcp(Default::default());
    let config = embassy_net::Config::Static(embassy_net::StaticConfig {
        address: Ipv4Cidr::new(Ipv4Address::new(10, 42, 0, 61), 24),
        dns_servers: Vec::new(),
        gateway: Some(Ipv4Address::new(10, 42, 0, 1)),
    });

    // Init network stack
    let stack = &*singleton!(Stack::new(
        device,
        config,
        singleton!(StackResources::<2>::new()),
        seed
    ));

    // Launch network task
    unwrap!(spawner.spawn(net_task(&stack)));

    let mut clock = StmClock::new(ptp);

    clock.adjust(
        statime::time::Duration::ZERO,
        0.9872,
        &statime::datastructures::datasets::TimePropertiesDS::new_arbitrary_time(
            false,
            false,
            statime::datastructures::common::TimeSource::Other,
        ),
    );
    clock.adjust(
        statime::time::Duration::ZERO,
        1.0004,
        &statime::datastructures::datasets::TimePropertiesDS::new_arbitrary_time(
            false,
            false,
            statime::datastructures::common::TimeSource::Other,
        ),
    );

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
