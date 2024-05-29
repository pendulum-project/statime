use core::cell::RefCell;

use critical_section::Mutex;
use statime::{PtpInstanceState, PtpInstanceStateMutex};

pub struct PtpStateMutex(Mutex<RefCell<PtpInstanceState>>);

impl PtpInstanceStateMutex for PtpStateMutex {
    fn new(state: PtpInstanceState) -> Self {
        Self(Mutex::new(RefCell::new(state)))
    }

    fn with_ref<R, F: FnOnce(&PtpInstanceState) -> R>(&self, f: F) -> R {
        critical_section::with(|cs| f(&self.0.borrow_ref(cs)))
    }

    fn with_mut<R, F: FnOnce(&mut PtpInstanceState) -> R>(&self, f: F) -> R {
        critical_section::with(|cs| f(&mut self.0.borrow_ref_mut(cs)))
    }
}
