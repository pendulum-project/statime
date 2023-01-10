//! Definitions and implementations of the abstract network types

use crate::time::Instant;

#[cfg(test)]
pub mod test;

/// Abstraction for the network
///
/// With it the network ports can be opened
pub trait NetworkRuntime: Clone {
    /// A descriptor type for the interface to be used.
    /// Can be useful to select between e.g. ethernet and wifi if both are present on the machine
    /// or to select between IPv4 and IPv6.
    type InterfaceDescriptor: Clone;
    type PortType: NetworkPort;
    type Error: std::error::Error + std::fmt::Display;

    /// Open a port on the given network interface.
    /// 
    /// This port has a time-critical and non-time-critical component.
    ///
    /// For example, when using IPv4, there must be a connection to the multicast address 244.0.1.129.
    /// It needs two sockets. For the time-critical component port 319 must be used. For the other one port 320 is to be used.
    async fn open(
        &mut self,
        interface: Self::InterfaceDescriptor,
    ) -> Result<Self::PortType, Self::Error>;
}

/// The representation of a network packet
#[derive(Debug, Clone)]
pub struct NetworkPacket {
    /// The received data of a network port
    pub data: Vec<u8>,
    /// The timestamp at which the packet was received. This is preferrably a timestamp
    /// that has been reported by the network hardware.
    ///
    /// The timestamp must be Some when the packet comes from a time-critical port.
    /// The timestamp will be ignored when it comes from a non-time-critical port, so it may as well be None.
    pub timestamp: Option<Instant>,
}

/// Abstraction for a port or socket
///
/// This object only has to be able to send a message because if a message is received, it must be
/// reported to the instance using the [PtpInstance::handle_network](crate::ptp_instance::PtpInstance::handle_network) function.
pub trait NetworkPort {
    type Error: std::error::Error + std::fmt::Display;

    /// Send the given non-time-critical data.
    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error>;

    /// Send the given time-critical data.
    ///
    /// If this is on a time-critical port, then the function must return an id and the TX timestamp must be
    /// reported to the instance using the [PtpInstance::handle_send_timestamp](crate::ptp_instance::PtpInstance::handle_send_timestamp) function using the same id that was returned.
    async fn send_time_critical(&mut self, data: &[u8]) -> Result<Instant, Self::Error>;

    /// Wait until a message is received
    async fn recv(&mut self) -> Result<NetworkPacket, Self::Error>;
}
