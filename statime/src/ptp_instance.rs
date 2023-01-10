use futures::FutureExt;

use crate::{
    clock::{Clock, Timer},
    datastructures::common::{ClockIdentity, PortIdentity},
    filters::Filter,
    network::{NetworkPacket, NetworkRuntime},
    port::{Port, PortConfig},
    time::{Duration, Instant},
};

pub struct Config<NR: NetworkRuntime> {
    pub identity: ClockIdentity,
    pub sdo: u16,
    pub domain: u8,
    pub interface: NR::InterfaceDescriptor,
    pub port_config: PortConfig,
}

/// Object that acts as the central point of this library.
/// It is the main instance of the running protocol.
///
/// The instance doesn't run on its own, but requires the user to invoke the `handle_*` methods whenever required.
pub struct PtpInstance<NR: NetworkRuntime, C: Clock, F: Filter> {
    port: Port<NR>,
    clock: C,
    filter: F,
}

impl<NR: NetworkRuntime, C: Clock, F: Filter> PtpInstance<NR, C, F> {
    /// Create a new instance
    ///
    /// - `config`: The configuration of the ptp instance
    /// - `runtime`: The network runtime with which sockets can be opened
    /// - `clock`: The clock that will be adjusted and provides the watches
    /// - `filter`: A filter for time measurements because those are always a bit wrong and need some processing
    pub fn new(config: Config<NR>, runtime: NR, mut clock: C, filter: F) -> Self {
        PtpInstance {
            port: Port::new(
                PortIdentity {
                    clock_identity: config.identity,
                    port_number: 0,
                },
                config.sdo,
                config.domain,
                config.port_config,
                runtime,
                config.interface,
                clock.quality(),
            ),
            clock,
            filter,
        }
    }

    pub async fn run(&mut self, timer: &impl Timer) {
        let bmca_timeout = timer.after(Duration::from_secs(1)).fuse();
        futures::pin_mut!(bmca_timeout);

        loop {
            futures::select_biased!(
                _ = bmca_timeout => {
                    bmca_timeout.set(timer.after(Duration::from_secs(1)).fuse());
                    self.run_bmca();
                }
            )
        }
    }

    async fn run_bmca(&mut self) {
        // Currently we only have one port, so erbest is also automatically our ebest
        let current_time = self.clock.now();
        let erbest = self
            .port
            .take_best_port_announce_message(current_time)
            .map(|v| (v.0, v.2));
        let erbest = erbest
            .as_ref()
            .map(|(message, identity)| (message, identity));

        // Run the state decision        
        self.port.perform_state_decision(erbest, erbest);
    }

    /// Let the instance handle a received network packet.
    ///
    /// This should be called for any and all packets that were received on the opened sockets of the network runtime.
    pub fn handle_network(&mut self, packet: NetworkPacket) {
        self.port.handle_network(packet, self.bmca_watch.now());
        if let Some((data, time_properties)) = self.port.extract_measurement() {
            let (offset, freq_corr) = self.filter.absorb(data);
            self.clock
                .adjust(offset, freq_corr, time_properties)
                .expect("Unexpected error adjusting clock");
        }
    }

    /// Let the instance know what the TX or send timestamp was of a packet that was recently sent.
    ///
    /// When sending a time critical message we need to know exactly when it was sent to do all of the arithmetic.
    pub fn handle_send_timestamp(&mut self, id: usize, timestamp: Instant) {
        self.port.handle_send_timestamp(id, timestamp);
    }
}
