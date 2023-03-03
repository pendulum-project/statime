use core::cell::RefCell;
use core::convert::Infallible;

use crate::bmc::bmca::BestAnnounceMessage;
use crate::clock::{Clock, Timer};
use crate::datastructures::datasets::{CurrentDS, DefaultDS, ParentDS, TimePropertiesDS};
use crate::filters::Filter;
use crate::network::NetworkPort;
use crate::port::Port;

/// Object that acts as the central point of this library.
/// It is the main instance of the running protocol.
///
/// The instance doesn't run on its own, but requires the user to invoke the `handle_*` methods whenever required.
pub struct PtpInstance<P, C, F, const N: usize> {
    default_ds: DefaultDS,
    current_ds: Option<CurrentDS>,
    parent_ds: Option<ParentDS>,
    time_properties_ds: RefCell<TimePropertiesDS>,
    ports: [Port<P>; N],
    local_clock: RefCell<C>,
    filter: RefCell<F>,
    announce_messages: RefCell<[Option<BestAnnounceMessage>; N]>,
}

impl<P, C, F> PtpInstance<P, C, F, 1> {
    /// Create a new instance
    ///
    /// - `local_clock`: The clock that will be adjusted and provides the watches
    /// - `filter`: A filter for time measurements because those are always a bit wrong and need some processing
    /// - `runtime`: The network runtime with which sockets can be opened
    pub fn new_ordinary_clock(
        default_ds: DefaultDS,
        time_properties_ds: TimePropertiesDS,
        port: Port<P>,
        local_clock: C,
        filter: F,
    ) -> Self {
        PtpInstance::new_boundary_clock(default_ds, time_properties_ds, [port], local_clock, filter)
    }
}

impl<P, C, F, const N: usize> PtpInstance<P, C, F, N> {
    /// Create a new instance
    ///
    /// - `config`: The configuration of the ptp instance
    /// - `clock`: The clock that will be adjusted and provides the watches
    /// - `filter`: A filter for time measurements because those are always a bit wrong and need some processing
    pub fn new_boundary_clock(
        default_ds: DefaultDS,
        time_properties_ds: TimePropertiesDS,
        ports: [Port<P>; N],
        local_clock: C,
        filter: F,
    ) -> Self {
        for (index, port) in ports.iter().enumerate() {
            assert_eq!(port.identity().port_number - 1, index as u16);
        }
        PtpInstance {
            default_ds,
            current_ds: None,
            parent_ds: None,
            time_properties_ds: RefCell::new(time_properties_ds),
            ports,
            local_clock: RefCell::new(local_clock),
            filter: RefCell::new(filter),
            announce_messages: RefCell::new([None; N]),
        }
    }
}

impl<P: NetworkPort, C: Clock, F: Filter, const N: usize> PtpInstance<P, C, F, N> {
    pub async fn run(&mut self, timer: &impl Timer) -> [Infallible; N] {
        log::info!("Running!");

        let mut run_ports = self.ports.iter_mut().map(|port| {
            port.run_port(
                timer,
                &self.local_clock,
                &self.filter,
                &self.announce_messages,
                &self.default_ds,
                &self.time_properties_ds,
            )
        });
        let futures = [(); N].map(|_| run_ports.next().expect("not all ports were initialized"));

        embassy_futures::join::join_array(futures).await
    }
}
