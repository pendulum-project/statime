#![no_std]
#![no_main]

use core::mem::MaybeUninit;

use defmt::unwrap;
use embassy_executor::Spawner;
use embassy_net::{Config as NetConfig, StackResources};
use embassy_stm32::{
    bind_interrupts,
    eth::{Ethernet, GenericPhy, InterruptHandler, PacketQueue, PtpClockConfig, Sma},
    interrupt::{InterruptExt as _, Priority},
    pac::{self, Interrupt},
    peripherals::{ETH, ETH_SMA},
};
use static_cell::StaticCell;
use statime::filters::{FixedWanderKalmanConfig, FixedWanderKalmanFilter};
use statime_embassy_net::{Config as PtpConfig, EmbassyClock, PtpStorage, Runner as PtpRunner};

use {defmt_rtt as _, panic_probe as _};

defmt::timestamp!("{=u64:us}", embassy_time::Instant::now().as_micros());

#[defmt::panic_handler]
fn defmt_panic() -> ! {
    panic_probe::hard_fault()
}

const ETH_TX_PACKETS: usize = 4;
const ETH_RX_PACKETS: usize = 4;
const STACK_SOCKETS: usize = 4;

type Device = Ethernet<'static, ETH, GenericPhy<Sma<'static, ETH_SMA>>>;

bind_interrupts!(struct Irqs {
    ETH => InterruptHandler<ETH>;
});

#[unsafe(link_section = ".sram3.eth")]
static mut PACKETS: MaybeUninit<PacketQueue<ETH_TX_PACKETS, ETH_RX_PACKETS>> =
    MaybeUninit::uninit();
static PTP_STORAGE: StaticCell<PtpStorage> = StaticCell::new();
static STACK_RESOURCES: StaticCell<StackResources<STACK_SOCKETS>> = StaticCell::new();

mod board {
    use embassy_stm32::{
        Config,
        rcc::{
            AHBPrescaler, APBPrescaler, HSIPrescaler, Hse, HseMode, Pll, PllDiv, PllMul, PllPreDiv,
            PllSource, Sysclk, VoltageScale,
        },
        time::Hertz,
    };

    pub const MAC_ADDRESS: [u8; 6] = [0x02, 0x50, 0x54, 0x50, 0x00, 0x01];
    pub const SEED: u64 = 0x5054_5020_4847_4331;

    pub fn stm32_config() -> Config {
        let mut config = Config::default();
        config.rcc.hse = Some(Hse {
            freq: Hertz(8_000_000),
            mode: HseMode::Oscillator,
        });
        config.rcc.hsi = Some(HSIPrescaler::Div1);
        config.rcc.csi = false;
        config.rcc.pll1 = Some(Pll {
            source: PllSource::Hse,
            prediv: PllPreDiv::Div4,
            mul: PllMul::Mul400,
            fracn: None,
            divp: Some(PllDiv::Div2),
            divq: None,
            divr: None,
        });
        config.rcc.sys = Sysclk::Pll1P;
        config.rcc.d1c_pre = AHBPrescaler::Div1;
        config.rcc.ahb_pre = AHBPrescaler::Div2;
        config.rcc.apb1_pre = APBPrescaler::Div2;
        config.rcc.apb2_pre = APBPrescaler::Div2;
        config.rcc.apb3_pre = APBPrescaler::Div2;
        config.rcc.apb4_pre = APBPrescaler::Div2;
        config.rcc.voltage_scale = VoltageScale::Scale1;
        config
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) -> ! {
    let p = embassy_stm32::init(board::stm32_config());

    // Ethernet DMA buffers are placed in SRAM3 by this example linker
    // script, so enable that RAM before initializing the packet queue.
    pac::RCC.ahb2enr().modify(|w| w.set_sram3en(true));

    // ETH wakes the network runner; TIM12 drives embassy-time deadlines.
    Interrupt::ETH.set_priority(Priority::P6);
    Interrupt::TIM8_BRK_TIM12.set_priority(Priority::P7);

    #[cfg(feature = "stabilizer")]
    {
        use embassy_stm32::gpio::{Level, Output, Speed};
        const SYSCLK_HZ: u32 = 400_000_000;

        let mut phy_reset = Output::new(p.PE3, Level::Low, Speed::Low);
        phy_reset.set_low();
        cortex_m::asm::delay(SYSCLK_HZ / 4);
        phy_reset.set_high();
        cortex_m::asm::delay(SYSCLK_HZ / 4);
        core::mem::forget(phy_reset);
    }

    let queue = unsafe {
        let packets = core::ptr::addr_of_mut!(PACKETS);
        PacketQueue::init(&mut *packets);
        (*packets).assume_init_mut()
    };
    let phy = GenericPhy::new(Sma::new(p.ETH_SMA, p.PA2, p.PC1), 0);
    let mut device = Ethernet::new_with_phy(
        queue,
        p.ETH,
        Irqs,
        p.PA1,
        p.PA7,
        p.PC4,
        p.PC5,
        p.PB12,
        p.PG14,
        p.PB11,
        board::MAC_ADDRESS,
        phy,
    );
    let ptp_clock = EmbassyClock::new(device.start_ptp(PtpClockConfig::default()));
    let (stack, runner) = embassy_net::new(
        device,
        NetConfig::dhcpv4(Default::default()),
        STACK_RESOURCES.init(StackResources::new()),
        board::SEED,
    );
    let ptp_runner = PtpRunner::<_, FixedWanderKalmanFilter>::new(
        stack,
        ptp_clock,
        PTP_STORAGE.init(PtpStorage::new()),
        PtpConfig::new(board::MAC_ADDRESS, board::SEED),
        FixedWanderKalmanConfig::default(),
    );

    spawner.spawn(unwrap!(net_task(runner)));
    spawner.spawn(unwrap!(ptp_task(ptp_runner)));
    match core::future::pending::<core::convert::Infallible>().await {}
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, Device>) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn ptp_task(mut runner: PtpRunner<'static, EmbassyClock<embassy_stm32::eth::PtpClock<ETH>>>) {
    unwrap!(runner.run().await);
}
