use embassy_net_driver::{Driver, RxToken, TxToken};
use embassy_stm32::eth::{Instance, PHY};
use smoltcp::{
    phy,
    phy::{Device, DeviceCapabilities},
    time::Instant,
};

use crate::static_ethernet::StaticEthernet;

pub struct StmDevice<T: Instance, P: PHY> {
    pub(crate) ethernet: StaticEthernet<T, P>,
}

impl<T: Instance, P: PHY> StmDevice<T, P> {
    pub fn mac_addr(&self) -> [u8; 6] {
        self.ethernet.mac_addr
    }
}

impl<'d, T: Instance, P: PHY> Device for StmDevice<T, P> {
    type RxToken<'a> = StmRxToken<'a, 'd> where Self: 'a;
    type TxToken<'a> = StmTxToken<'a, 'd> where Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if self.ethernet.rx.available().is_some() && self.ethernet.tx.available().is_some() {
            Some((
                StmRxToken(crate::eth::RxToken {
                    rx: &mut self.ethernet.rx,
                }),
                StmTxToken(crate::eth::TxToken {
                    tx: &mut self.ethernet.tx,
                }),
            ))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        if self.ethernet.tx.available().is_some() {
            Some(StmTxToken(crate::eth::TxToken {
                tx: &mut self.ethernet.tx,
            }))
        } else {
            None
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let embassy_caps = self.ethernet.capabilities();
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = embassy_caps.max_transmission_unit;
        caps.max_burst_size = embassy_caps.max_burst_size;
        caps
    }
}

pub struct StmRxToken<'a, 'd>(crate::eth::RxToken<'a, 'd>);

impl<'a, 'd> phy::RxToken for StmRxToken<'a, 'd> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        self.0.consume(f)
    }
}

pub struct StmTxToken<'a, 'd>(crate::eth::TxToken<'a, 'd>);

impl<'a, 'd> phy::TxToken for StmTxToken<'a, 'd> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        self.0.consume(len, f)
    }
}
