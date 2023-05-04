use embassy_net_driver::{Driver, RxToken, TxToken};
use embassy_stm32::eth::{Instance, PHY};
use smoltcp::{
    phy,
    phy::{Device, DeviceCapabilities},
    time::Instant,
};

use crate::eth::{Ethernet, RDesRing, TDesRing};

pub struct StmDevice<'d, T: Instance, P: PHY> {
    ethernet: Ethernet<'d, T, P>,
}

impl<'d, T: Instance, P: PHY> StmDevice<'d, T, P> {
    pub fn mac_addr(&self) -> [u8; 6] {
        self.ethernet.mac_addr
    }

    pub fn rx(&self) -> &RDesRing {
        &self.ethernet.rx
    }

    pub fn tx(&self) -> &TDesRing {
        &self.ethernet.tx
    }
}

struct StmRxToken<'a, 'd>(embassy_stm32::eth::RxToken<'a, 'd>);
struct StmTxToken<'a, 'd>(embassy_stm32::eth::TxToken<'a, 'd>);

impl<'d, T: Instance, P: PHY> Device for StmDevice<'d, T, P> {
    type RxToken<'a> = StmRxToken<'a, 'd> where Self: 'a;
    type TxToken<'a> = StmTxToken<'a, 'd> where Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let mut cx = todo!();
        self.ethernet
            .receive(&mut cx)
            .map(|(rx, tx)| (StmRxToken(rx), StmTxToken(tx)))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        let mut cx = todo!();
        self.ethernet.transmit(&mut cx).map(StmTxToken)
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let embassy_caps = self.ethernet.capabilities();
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = embassy_caps.max_transmission_unit;
        caps.max_burst_size = embassy_caps.max_burst_size;
        // TODO: Complete translation
        caps
    }
}

impl<'a, 'd> phy::RxToken for StmRxToken<'a, 'd> {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        self.0.consume(f)
    }
}

impl<'a, 'd> phy::TxToken for StmTxToken<'a, 'd> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        self.0.consume(len, f)
    }
}
