use embassy_stm32::{adc::Instance, eth::PHY};
use smoltcp::{
    phy,
    phy::{Device, DeviceCapabilities},
    time::Instant,
};

use crate::eth;

impl<'d, T: Instance, P: PHY> Device for eth::Ethernet<'d, T, P> {
    type RxToken<'a> = <Self as embassy_net_driver::Driver>::RxToken<'a> where Self: 'a;
    type TxToken<'a> = <Self as embassy_net_driver::Driver>::TxToken<'a> where Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let mut cx = todo!();
        <Self as embassy_net_driver::Driver>::receive(self, &mut cx)
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        let mut cx = todo!();
        <Self as embassy_net_driver::Driver>::transmit(self, &mut cx)
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let capabilities = <Self as embassy_net_driver::Driver>::capabilities(self);

        // TODO: Complete translation
        DeviceCapabilities {
            max_transmission_unit: capabilities.max_transmission_unit,
            max_burst_size: capabilities.max_burst_size,
            ..Default::default()
        }
    }
}

impl<'a, 'd> phy::RxToken for eth::RxToken<'a, 'd> {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        <Self as embassy_net_driver::RxToken>::consume(self, f)
    }
}

impl<'a, 'd> phy::TxToken for eth::TxToken<'a, 'd> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        <Self as embassy_net_driver::TxToken>::consume(self, len, f)
    }
}
