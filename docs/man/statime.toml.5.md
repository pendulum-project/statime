<!-- ---
title: STATIME.TOML(5) statime 0.3.0 | statime
--- -->

# NAME

`statime.toml` - configuration file for the statime ptp-daemon

# DESCRIPTION

Configuration of ntpd-rs happens in the `statime.toml` configuration format. The
toml format is in lots of ways similar to a simple ini with several extensions
allowing a json-like syntax.

The statime configuration file consists of several sections, each of which
configures a separate part of the ptp-daemon process. Each of the sections is
described in the rest of this document. Many settings will have defaults, which
will be indicated by each configuration setting shown.

# CONFIGURATION

`identity` = *clock identity* (**unset**)
:   The unique identity of this clock.
    A clock identity is encoded as a 16-character hexadecimal string, for example
    `identity = "00FFFFFFFFFFFFFB"`.
    If unset the clock identity is derived from a MAC address.

`domain` = *u8* (**0**)
:   The PTP domain of this instance. All instances in domain are synchronized to the Grandmaster
    Clock of the domain, but are not necessarily synchronized to PTP clocks in another domain.

`sdo-id` = *u12* (**0**)
:   The "source domain identity" of this PTP instance. Together with the `domain` it identifies a domain.

`slave-only` = *bool* (**false**)
:   Forbids this instance to ever become a master on any port.

`priority1` = *priority* (**128**)
:   A tie breaker for the best master clock algorithm in the range `0..256`. `0` being the highest priority and `255` the lowest.

`priority2` = *priority* (**128**)
:   A tie breaker for the best master clock algorithm in the range `0..256`. `0` being the highest priority and `255` the lowest.

`path-trace` = *bool*
:   The instance uses the path trace option. This allows detecting clock loops when enabled on all instances in the network.

`virtual-system-clock` = *bool* (**false**)
:   Use a virtual overlay clock instead of adjusting the system clock.

## `[[port]]`

`interface` = *interface name*
:   The network interface of this PTP port. For instance `"lo"` or `"enp0s31f6"`

`announce-interval` = *interval* (**1**)
:   How often an announce message is sent by a master.
    Defined as an exponent of 2, so a value of 1 means every 2^1 = 2 seconds.

`sync-interval` = *interval* (**0**)
:   How often sync message is sent by a master.
    Defined as an exponent of 2, so a value of 0 means every 2^0 = 1 seconds.

`announce-receipt-timeout` = *number of announce intervals* (**3**)
:   Number of announce intervals to wait for announce messages from other masters before the port becomes master itself.

`delay-asymmetry` = *nanoseconds* (**0**)
:   Correct for a difference between slave-to-master and master-to-slave propagation time.
    The value is positive when the slave-to-master propagation time is longer than the master-to-slave propagation time.

`delay-mechanism` = *mechanism* (**E2E**)
:   Which delay mechanism to use on the port. Either `"E2E"` for end-to-end delay determination, or `"P2P"` for the peer
    to peer delay mechanism.

`delay-interval` = *interval* (**0**)
:   How often delay request messages are sent by a slave in end-to-end mode.
    Currently the only supported delay mechanism is end-to-end (E2E).
    Defined as an exponent of 2, so a value of 0 means every 2^0 = 1 seconds

`master-only` = *bool* (**false**)
:   The port is always a master instance, and will never become a slave instance.

`hardware-clock` = `auto` | `required` | `none` | *index* (**auto**)
:   Index of a hardware clock device, for instance `0` for `/dev/ptp0`. Set to
    auto to automatically configure the hardware clock if one is available. Set
    to required if you need a hardware clock and want the configuration to fail
    if one is not available. Set to none to disable using a hardware clock.

`acceptable-master-list` = [ *clock identity*, .. ] (**unset**)
:   List of clock identities that this port will accept as its master.
    A clock identity is encoded as a 16-character hexadecimal string, for example
    `acceptable-master-list = ["00FFFFFFFFFFFFFB"]`.
    The default is to accept all clock identities.

`minor-ptp-version` = *version number* (**1**)
:   Set a different minor ptp version. Should be either 1 or 0, intended to work around misbehaving ptp 2.0 hardware

## `[observability]`

`observation-path` = *path* (**unset**)
:   Path where the daemon will create an observation Unix domain socket. This
    socket is used by `statime-metrics-exporter` to read the current
    status of the daemon. If not set (the default) no observation socket will be
    created, and it is not possible to use `statime-metrics-exporter` to
    observe the daemon.

`observation-permissions` = *mode* (**0o666**)
:   The file system permissions with which the observation socket should be
    created. Warning: You should always write this number with the octal prefix
    `0o`, otherwise your permissions might be interpreted wrongly. The default
    should be OK for most applications.

`metrics-exporter-listen` = *socketaddr* (**127.0.0.1:9975**)
:   The listen address that is used for the statime-metrics-exporter(8).

# SEE ALSO

[statime(8)](statime.8.md), [statime-metrics-exporter(8)](statime-metrics-exporter.8.md)