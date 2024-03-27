#[allow(unused)] // clippy will inaccurately mark this as unused on platforms with std
pub(crate) trait FloatPolyfill {
    #[cfg(not(feature = "std"))]
    fn abs(self) -> Self;
    #[cfg(not(feature = "std"))]
    fn signum(self) -> Self;
    #[cfg(not(feature = "std"))]
    fn sqrt(self) -> Self;
    #[cfg(not(feature = "std"))]
    fn powi(self, n: i32) -> Self;
    #[cfg(not(feature = "std"))]
    fn exp(self) -> Self;
}

impl FloatPolyfill for f64 {
    #[cfg(not(feature = "std"))]
    fn abs(self) -> Self {
        libm::fabs(self)
    }

    #[cfg(not(feature = "std"))]
    fn signum(self) -> Self {
        if self < 0.0 {
            -1.0
        } else if self > 0.0 {
            1.0
        } else {
            0.0
        }
    }

    #[cfg(not(feature = "std"))]
    fn sqrt(self) -> Self {
        libm::sqrt(self)
    }

    #[cfg(not(feature = "std"))]
    fn powi(self, n: i32) -> Self {
        libm::pow(self, n as f64)
    }

    #[cfg(not(feature = "std"))]
    fn exp(self) -> Self {
        libm::exp(self)
    }
}
