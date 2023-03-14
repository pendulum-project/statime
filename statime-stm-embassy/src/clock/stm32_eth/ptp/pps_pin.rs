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

mod impl_pps_pin {
    use embassy_stm32::peripherals::{PB5, PG8};

    impl_pps_pin!([PG8, PG8], [PB5, PB5]);
}
