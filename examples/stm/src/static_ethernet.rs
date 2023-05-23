use core::{
    cell::RefCell,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use embassy_stm32::eth::{Instance, PHY};
use embassy_sync::blocking_mutex::ThreadModeMutex;

use crate::eth::Ethernet;

static mut ETHERNET: Option<ThreadModeMutex<RefCell<Ethernet<'static, T, P>>>> = None;

pub struct StaticEthernet<T: Instance, P: PHY>(PhantomData<(T, P)>);

impl<T: Instance, P: PHY> Ethernet<'_, T, P> {
    /// Store the device globally, making its lifetime static.
    pub fn persist(self) -> StaticEthernet<T, P> {
        unsafe {
            if ETHERNET.is_some() {
                unreachable!();
            }
            ETHERNET = Some(ThreadModeMutex::new(RefCell::new(ethernet)));
        }
        StaticEthernet(PhantomData::default())
    }
}

impl<T: Instance, P: PHY> Deref for StaticEthernet<T, P> {
    type Target = Ethernet<'static, T, P>;

    fn deref(&self) -> &Self::Target {
        unsafe { ETHERNET.as_ref().unwrap().borrow().borrow().deref() }
    }
}

impl<T: Instance, P: PHY> DerefMut for StaticEthernet<T, P> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { ETHERNET.as_ref().unwrap().borrow().borrow_mut().deref_mut() }
    }
}
