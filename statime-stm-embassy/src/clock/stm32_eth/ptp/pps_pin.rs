/// Pins that can be used as PPS output
///
/// SAFETY: only pins that are capable of being a PPS output according
/// to the datasheets of respective parts may implement this trait.
pub unsafe trait PPSPin {
    /// The output type of this pin, in which it acts as a PPS output.
    type Output;

    /// Enable the PPS output.
    fn enable(self) -> Self::Output;
}

#[allow(unused_macros)]
macro_rules! impl_pps_pin {
    ($([$name:ty, $output:ty]),*) => {
        $(
            unsafe impl super::PPSPin for $name {
                type Output = $output;

                fn enable(self) -> Self::Output {
                    self.into_alternate()
                }
            }
        )*
    };
}

#[cfg(feature = "stm32f4xx-hal")]
mod impl_pps_pin {
    use crate::hal::gpio::{Alternate, Output, PushPull, PB5, PG8};

    impl_pps_pin!([PG8<Output<PushPull>>, PG8<Alternate<11>>], [PB5<Output<PushPull>>, PB5<Alternate<11>>]);
}

#[cfg(feature = "stm32f7xx-hal")]
mod impl_pps_pin {
    use crate::hal::gpio::{Alternate, Output, PushPull, PB5, PG8};

    impl_pps_pin!([PG8<Output<PushPull>>, PG8<Alternate<11>>], [PB5<Output<PushPull>>, PB5<Alternate<11>>]);
}

#[cfg(feature = "stm32f1xx-hal")]
mod impl_pps_pin {
    use crate::hal::gpio::{Alternate, Output, PushPull, PB5};

    unsafe impl super::PPSPin for PB5<Output<PushPull>> {
        type Output = PB5<Alternate<PushPull>>;

        fn enable(self) -> Self::Output {
            // Within this critical section, modifying the `CRL` register can
            // only be unsound if this critical section preempts other code
            // that is modifying the same register
            cortex_m::interrupt::free(|_| {
                // SAFETY: this is sound as long as the API of the HAL and structure of the CRL
                // struct does not change. In case the size of the `CRL` struct is changed, compilation
                // will fail as `mem::transmute` can only convert between types of the same size.
                //
                // This guards us from unsound behaviour introduced by point releases of the f1 hal
                let cr: &mut _ = &mut unsafe { core::mem::transmute(()) };
                // The speed can only be changed on output pins
                self.into_alternate_push_pull(cr)
            })
        }
    }
}
