use core::ops::{Deref, DerefMut};

use aligned::{Aligned, A8};
use volatile_register::{RO, RW};

#[cfg(not(feature = "stm32f1xx-hal"))]
const DESC_SIZE: usize = 8;

#[cfg(feature = "stm32f1xx-hal")]
const DESC_SIZE: usize = 4;

#[repr(C)]
pub struct Descriptor {
    pub(crate) desc: Aligned<A8, [u32; DESC_SIZE]>,
}

impl Clone for Descriptor {
    fn clone(&self) -> Self {
        Descriptor {
            desc: Aligned(*self.desc),
        }
    }
}

impl Default for Descriptor {
    fn default() -> Self {
        Self::new()
    }
}

impl Descriptor {
    pub const fn new() -> Self {
        Self {
            desc: Aligned([0; DESC_SIZE]),
        }
    }

    fn r(&self, n: usize) -> &RO<u32> {
        let ro = &self.desc.deref()[n] as *const _ as *const RO<u32>;
        unsafe { &*ro }
    }

    unsafe fn rw(&mut self, n: usize) -> &mut RW<u32> {
        let rw = &mut self.desc.deref_mut()[n] as *mut _ as *mut RW<u32>;
        &mut *rw
    }

    pub fn read(&self, n: usize) -> u32 {
        self.r(n).read()
    }

    pub unsafe fn write(&mut self, n: usize, value: u32) {
        self.rw(n).write(value)
    }

    pub unsafe fn modify<F>(&mut self, n: usize, f: F)
    where
        F: FnOnce(u32) -> u32,
    {
        self.rw(n).modify(f)
    }
}
