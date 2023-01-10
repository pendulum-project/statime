//! Implementation of the abstract network types for the linux platform

use crate::{
    clock::timespec_into_instant, network::linux_syscall::driver_enable_hardware_timestamping,
};
use nix::{
    cmsg_space,
    errno::Errno,
    ifaddrs::{getifaddrs, InterfaceAddress, InterfaceAddressIterator},
    net::if_::if_nametoindex,
    sys::socket::{
        recvmsg, sendmsg, setsockopt, socket,
        sockopt::{BindToDevice, ReuseAddr, Timestamping},
        AddressFamily, ControlMessageOwned, MsgFlags, SetSockOpt, SockFlag, SockType,
        TimestampingFlag, Timestamps,
    },
};
use statime::network::{NetworkPacket, NetworkPort, NetworkRuntime};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    os::{fd::AsRawFd, unix::prelude::RawFd},
    str::FromStr,
    sync::mpsc::Sender,
    thread::JoinHandle,
};
use tokio::net::UdpSocket;

#[derive(Clone)]
pub struct LinuxRuntime {
    hardware_timestamping: bool,
}

impl LinuxRuntime {
    pub fn new(hardware_timestamping: bool) -> Self {
        LinuxRuntime {
            hardware_timestamping,
        }
    }

    const IPV6_PRIMARY_MULTICAST: Ipv6Addr = Ipv6Addr::new(0xFF, 0x0E, 0, 0, 0, 0, 0x01, 0x81);
    const IPV6_PDELAY_MULTICAST: Ipv6Addr = Ipv6Addr::new(0xFF, 0x02, 0, 0, 0, 0, 0, 0x6B);

    const IPV4_PRIMARY_MULTICAST: Ipv4Addr = Ipv4Addr::new(224, 0, 1, 129);
    const IPV4_PDELAY_MULTICAST: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 107);
}

#[derive(Debug, Clone)]
pub struct LinuxInterfaceDescriptor {
    interface_name: Option<String>,
    mode: LinuxNetworkMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinuxNetworkMode {
    Ipv4,
    Ipv6,
}

#[derive(thiserror::Error, Debug)]
pub enum NetworkError {
    #[error("Unknown error")]
    UnknownError,
    #[error("Not allowed to bind to port {0}")]
    NoBindPermission(u16),
    #[error("Socket bind port {0} already in use")]
    AddressInUse(u16),
    #[error("Could not bind socket to a specific device")]
    BindToDeviceFailed,
    #[error("Could not iterate over interfaces")]
    CannotIterateInterfaces,
    #[error("The specified interface does not exist")]
    InterfaceDoesNotExist,
    #[error("No more packets")]
    NoMorePackets,
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

impl LinuxInterfaceDescriptor {
    fn get_index(&self) -> Option<u32> {
        if let Some(ref name) = self.interface_name {
            if_nametoindex(&name[..]).ok()
        } else {
            None
        }
    }

    fn get_address(&self) -> Result<IpAddr, NetworkError> {
        if let Some(ref name) = self.interface_name {
            let interfaces = match getifaddrs() {
                Ok(a) => a,
                Err(_) => return Err(NetworkError::CannotIterateInterfaces),
            };
            for i in interfaces {
                if name == &i.interface_name {
                    if self.mode == LinuxNetworkMode::Ipv6 {
                        if let Some(a) = i.address.map(|a| a.as_sockaddr_in6()).flatten() {
                            return Ok(a.ip().into());
                        }
                    } else if let Some(a) = i.address.map(|a| a.as_sockaddr_in()).flatten() {
                        return Ok(Ipv4Addr::from(a.ip()).into());
                    }
                }
            }
            Err(NetworkError::InterfaceDoesNotExist)
        } else if self.mode == LinuxNetworkMode::Ipv6 {
            Ok(IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED))
        } else {
            Ok(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
        }
    }
}

impl FromStr for LinuxInterfaceDescriptor {
    type Err = NetworkError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let interfaces = match getifaddrs() {
            Ok(a) => a,
            Err(_) => return Err(NetworkError::CannotIterateInterfaces),
        };

        match std::net::IpAddr::from_str(s) {
            Ok(addr) => {
                if addr.is_unspecified() {
                    return Ok(LinuxInterfaceDescriptor {
                        interface_name: None,
                        mode: if addr.is_ipv4() {
                            LinuxNetworkMode::Ipv4
                        } else {
                            LinuxNetworkMode::Ipv6
                        },
                    });
                }

                let sock_addr = std::net::SocketAddr::new(addr, 0);
                for ifaddr in interfaces {
                    if if_has_address(&ifaddr, sock_addr.ip()) {
                        return Ok(LinuxInterfaceDescriptor {
                            interface_name: Some(ifaddr.interface_name),
                            mode: LinuxNetworkMode::Ipv4,
                        });
                    }
                }

                Err(NetworkError::InterfaceDoesNotExist)
            }
            Err(_) => {
                if if_name_exists(interfaces, s) {
                    Ok(LinuxInterfaceDescriptor {
                        interface_name: Some(s.to_owned()),
                        mode: LinuxNetworkMode::Ipv4,
                    })
                } else {
                    Err(NetworkError::InterfaceDoesNotExist)
                }
            }
        }
    }
}

fn if_has_address(ifaddr: &InterfaceAddress, address: IpAddr) -> bool {
    match (
        address,
        ifaddr.address.map(|a| a.as_sockaddr_in()).flatten(),
        ifaddr.address.map(|a| a.as_sockaddr_in6()).flatten(),
    ) {
        (_, None, None) => false,

        (IpAddr::V4(_), None, _) => false,
        (IpAddr::V4(addr1), Some(addr2), _) => addr1.octets() == addr2.ip().to_be_bytes(),

        (IpAddr::V6(_), _, None) => false,
        (IpAddr::V6(addr1), _, Some(addr2)) => addr1.octets() == addr2.ip().octets(),
    }
}

fn if_name_exists(interfaces: InterfaceAddressIterator, name: &str) -> bool {
    for i in interfaces {
        if i.interface_name == name {
            return true;
        }
    }

    false
}

/// Request for multicast socket operations
///
/// This is a wrapper type around `ip_mreqn`.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IpMembershipRequest(libc::ip_mreqn);

impl IpMembershipRequest {
    /// Instantiate a new `IpMembershipRequest`
    ///
    ///
    pub fn new(group: Ipv4Addr, interface_idx: Option<u32>) -> Self {
        IpMembershipRequest(libc::ip_mreqn {
            imr_multiaddr: libc::in_addr {
                s_addr: group.into(),
            },
            imr_address: libc::in_addr { s_addr: 0 },
            imr_ifindex: interface_idx.unwrap_or(0) as i32,
        })
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct IpAddMembership;

impl SetSockOpt for IpAddMembership {
    type Val = IpMembershipRequest;

    fn set(&self, fd: RawFd, val: &Self::Val) -> nix::Result<()> {
        let ptr = val as *const Self::Val as *const libc::c_void;
        let ptr_len = std::mem::size_of::<Self::Val>() as libc::socklen_t;
        let res = unsafe {
            libc::setsockopt(fd, libc::IPPROTO_IP, libc::IP_ADD_MEMBERSHIP, ptr, ptr_len)
        };
        Errno::result(res).map(drop)
    }
}

/// Request for ipv6 multicast socket operations
///
/// This is a wrapper type around `ipv6_mreq`.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Ipv6MembershipRequest(libc::ipv6_mreq);

impl Ipv6MembershipRequest {
    /// Instantiate a new `Ipv6MembershipRequest`
    pub const fn new(group: Ipv6Addr, interface_idx: Option<u32>) -> Self {
        Ipv6MembershipRequest(libc::ipv6_mreq {
            ipv6mr_multiaddr: libc::in6_addr {
                s6_addr: group.octets(),
            },
            ipv6mr_interface: match interface_idx {
                Some(v) => v,
                _ => 0,
            },
        })
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct Ipv6AddMembership;

impl SetSockOpt for Ipv6AddMembership {
    type Val = Ipv6MembershipRequest;

    fn set(&self, fd: RawFd, val: &Self::Val) -> nix::Result<()> {
        let ptr = val as *const Self::Val as *const libc::c_void;
        let ptr_len = std::mem::size_of::<Self::Val>() as libc::socklen_t;
        let res = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_IPV6,
                libc::IPV6_ADD_MEMBERSHIP,
                ptr,
                ptr_len,
            )
        };
        Errno::result(res).map(drop)
    }
}

impl NetworkRuntime for LinuxRuntime {
    type InterfaceDescriptor = LinuxInterfaceDescriptor;
    type NetworkPort = LinuxNetworkPort;
    type Error = NetworkError;

    async fn open(
        &mut self,
        interface: Self::InterfaceDescriptor,
    ) -> Result<Self::NetworkPort, NetworkError> {
        let bind_ip = if interface.mode == LinuxNetworkMode::Ipv6 {
            IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)
        } else {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        };

        let tc_socket = tokio::net::UdpSocket::bind(SocketAddr::new(bind_ip, 319)).await?;
        // We want to allow multiple listening sockets, as we bind to a specific interface later
        setsockopt(tc_socket.as_raw_fd(), ReuseAddr, &true)
            .map_err(|_| NetworkError::UnknownError)?;
        let ntc_socket = tokio::net::UdpSocket::bind(SocketAddr::new(bind_ip, 320)).await?;
        // We want to allow multiple listening sockets, as we bind to a specific interface later
        setsockopt(ntc_socket.as_raw_fd(), ReuseAddr, &true)
            .map_err(|_| NetworkError::UnknownError)?;

        // Bind device to specified interface
        tc_socket.bind_device(interface.interface_name.map(|string| string.as_bytes()));
        ntc_socket.bind_device(interface.interface_name.map(|string| string.as_bytes()));

        // TODO: multicast ttl limit for ipv4/multicast hops limit for ipv6

        match interface.get_address()? {
            IpAddr::V4(ip) => {
                tc_socket.join_multicast_v4(Self::IPV4_PRIMARY_MULTICAST, ip)?;
                ntc_socket.join_multicast_v4(Self::IPV4_PRIMARY_MULTICAST, ip)?;
                tc_socket.join_multicast_v4(Self::IPV4_PDELAY_MULTICAST, ip)?;
                ntc_socket.join_multicast_v4(Self::IPV4_PDELAY_MULTICAST, ip)?;
            }
            IpAddr::V6(ip) => {
                tc_socket.join_multicast_v6(
                    &Self::IPV6_PRIMARY_MULTICAST,
                    interface.get_index().unwrap_or(0),
                )?;
                ntc_socket.join_multicast_v6(
                    &Self::IPV6_PRIMARY_MULTICAST,
                    interface.get_index().unwrap_or(0),
                )?;
                tc_socket.join_multicast_v6(
                    &Self::IPV6_PDELAY_MULTICAST,
                    interface.get_index().unwrap_or(0),
                )?;
                ntc_socket.join_multicast_v6(
                    &Self::IPV6_PDELAY_MULTICAST,
                    interface.get_index().unwrap_or(0),
                )?;
            }
        }

        // Setup timestamping if needed
        if time_critical {
            if self.hardware_timestamping {
                driver_enable_hardware_timestamping(
                    socket,
                    &interface
                        .interface_name
                        .ok_or(NetworkError::InterfaceDoesNotExist)?,
                );
                setsockopt(
                    socket,
                    Timestamping,
                    &(TimestampingFlag::SOF_TIMESTAMPING_RAW_HARDWARE
                        | TimestampingFlag::SOF_TIMESTAMPING_RX_HARDWARE
                        | TimestampingFlag::SOF_TIMESTAMPING_TX_HARDWARE),
                )
                .map_err(|_| NetworkError::UnknownError)?;
            } else {
                setsockopt(
                    socket,
                    Timestamping,
                    &(TimestampingFlag::SOF_TIMESTAMPING_SOFTWARE
                        | TimestampingFlag::SOF_TIMESTAMPING_RX_SOFTWARE
                        | TimestampingFlag::SOF_TIMESTAMPING_TX_SOFTWARE),
                )
                .map_err(|_| NetworkError::UnknownError)?;
            }
        }

        // TODO: replace recv thread with select
        let tx = self.tx.clone();
        let hardware_timestamping = self.hardware_timestamping;
        let recv_thread = std::thread::Builder::new()
            .name(format!("ptp {}", port))
            .spawn(move || LinuxNetworkPort::recv_thread(socket, tx, hardware_timestamping))
            .unwrap();

        Ok(LinuxNetworkPort {
            tc_socket,
            ntc_socket,
        })
    }
}

pub struct LinuxNetworkPort {
    tc_socket: UdpSocket,
    ntc_socket: UdpSocket,
}

impl NetworkPort for LinuxNetworkPort {
    fn send(&mut self, data: &[u8]) -> Option<usize> {
        let io_vec = [IoVec::from_slice(data)];
        sendmsg(
            self.socket,
            &io_vec,
            &[],
            MsgFlags::empty(),
            Some(&self.addr),
        )
        .unwrap();

        // TODO: Implement better method for send timestamps
        Some(u16::from_be_bytes(data[30..32].try_into().unwrap()) as usize)
    }
}

impl LinuxNetworkPort {
    fn recv_thread(socket: i32, tx: Sender<NetworkPacket>, hardware_timestamping: bool) {
        let mut read_buf = [0u8; 2048];
        let io_vec = [IoVec::from_mut_slice(&mut read_buf)];
        let mut cmsg = cmsg_space!(Timestamps);
        let flags = MsgFlags::empty();
        loop {
            let recv = recvmsg(socket, &io_vec, Some(&mut cmsg), flags).unwrap();
            let mut ts = None;
            for c in recv.cmsgs() {
                if let ControlMessageOwned::ScmTimestampsns(timestamps) = c {
                    let spec = if hardware_timestamping {
                        timestamps.hw_raw
                    } else {
                        timestamps.system
                    };
                    ts = Some(timespec_into_instant(spec));
                }
            }
            tx.send(NetworkPacket {
                data: io_vec[0].as_slice()[0..recv.bytes].to_vec(),
                timestamp: ts,
            })
            .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

pub fn get_clock_id() -> Option<[u8; 8]> {
    let candidates = getifaddrs().unwrap();
    for candidate in candidates {
        if let Some(SockAddr::Link(mac)) = candidate.address {
            // Ignore multicast and locally administered mac addresses
            if mac.addr()[0] & 0x3 == 0 && mac.addr().iter().any(|x| *x != 0) {
                let mut result: [u8; 8] = [0; 8];
                for (i, v) in mac.addr().iter().enumerate() {
                    result[i] = *v;
                }
                return Some(result);
            }
        }
    }
    None
}
