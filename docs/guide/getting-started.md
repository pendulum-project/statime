# Getting started

## Installing statime

We recommend installing statime through our precompiled binary packages, available on the [release page](https://github.com/pendulum-project/statime). These can be installed through the standard methods for your linux installation, and will provide a binary, as well as a skeleton configuration and systemd unit for statime.

## Configuring interfaces

Before we start statime, we first need to tell it which network interfaces we want it to use. For each network interface, add a section like
```toml
[[port]]
interface = "<interface_name>"
network-mode = "ipv4" # or "ipv6" or "ethernet"
```
to the configuration file in `/etc/statime/statime.toml`. This will tell statime to create a PTP port for that network interface.

### Hardware timestamping
If the network interface supports hardware timestamping for a specific PTP hardware clock, this can be enabled by specifying the hardware clock of the network interface:
```toml
hardware-clock = <hardware clock number>
```
Statime will then enable hardware timestamping, and automatically synchronize the hardware clock and the system clock as needed. To determine which hardware clock belongs to an interface, `ethtool` can be used:
```
> ethtool -T <ifname>
Time stamping parameters for <ifname>:
Capabilities:
	hardware-transmit
	software-transmit
	hardware-receive
	software-receive
	software-system-clock
	hardware-raw-clock
PTP Hardware Clock: 0
Hardware Transmit Timestamp Modes:
	off
	on
Hardware Receive Filter Modes:
	none
	all
```
Here, the number after `PTP Hardware Clock` indicates which hardware clock device should be used. The 0 in this case means `/dev/ptp0`.

## Starting the daemon

We can now start the statime daemon through systemd with
```
> systemctl start statime
```

If everything is configured well, and there is a PTP time source available in your network, the status of the daemon should look something like:
```
> systemctl status statime
● statime.service - Statime linux
     Loaded: loaded (/lib/systemd/system/statime.service; disabled; vendor preset: disabled)
     Active: active (running) since Fri 2023-11-24 09:49:28 CET; 19s ago
       Docs: https://github.com/pendulum-project/statime
   Main PID: 13032 (statime)
      Tasks: 17 (limit: 38206)
     Memory: 2.2M
        CPU: 13ms
     CGroup: /system.slice/statime.service
             └─13032 /usr/bin/statime

nov 24 09:49:43 magnesium statime[13032]: [08:49:43.6224660.873413086][statime::filters::basic][INFO] Offset to master: 5.52605e4ns, corrected with phase change -1.3815125e4ns and freq change 1.7684712628529555e0ppm
nov 24 09:49:44 magnesium statime[13032]: [08:49:44.6225960.254669189][statime::port::state::slave][INFO] Measurement: Measurement { event_time: Time { inner: 1700815821622293016 }, offset: Some(Duration { inner: 39944.5 }), delay: None, raw_sync_offset: Some(Duration { inner: 111057 }), raw_delay_offset: None }
nov 24 09:49:44 magnesium statime[13032]: [08:49:44.6226181.983947754][statime::filters::basic][INFO] Offset to master: 3.99445e4ns, corrected with phase change -9.986125e3ns and freq change 3.751591897138696e-2ppm
nov 24 09:49:44 magnesium statime[13032]: [08:49:44.7314345.836639404][statime::port::state::slave][INFO] Measurement: Measurement { event_time: Time { inner: 1700815821730825550 }, offset: None, delay: Some(Duration { inner: 42616.5 }), raw_sync_offset: None, raw_delay_offset: Some(Duration { inner: 25824 }) }
nov 24 09:49:45 magnesium statime[13032]: [08:49:45.6227660.179138184][statime::port::state::slave][INFO] Measurement: Measurement { event_time: Time { inner: 1700815822622463004 }, offset: Some(Duration { inner: 60108.5 }), delay: None, raw_sync_offset: Some(Duration { inner: 102725 }), raw_delay_offset: None }
nov 24 09:49:45 magnesium statime[13032]: [08:49:45.6227915.287017822][statime::filters::basic][INFO] Offset to master: 6.01085e4ns, corrected with phase change -1.5027125e4ns and freq change -7.536402116092855e-1ppm
nov 24 09:49:46 magnesium statime[13032]: [08:49:46.1561985.0158691406][statime::port::state::slave][INFO] Measurement: Measurement { event_time: Time { inner: 1700815823155437065 }, offset: None, delay: Some(Duration { inner: 47689.5 }), raw_sync_offset: None, raw_delay_offset: Some(Duration { inner: 7346 }) }
nov 24 09:49:46 magnesium statime[13032]: [08:49:46.2030603.8856506348][statime::port::state::slave][INFO] Measurement: Measurement { event_time: Time { inner: 1700815823202129510 }, offset: None, delay: Some(Duration { inner: 47014.5 }), raw_sync_offset: None, raw_delay_offset: Some(Duration { inner: 8696 }) }
nov 24 09:49:46 magnesium statime[13032]: [08:49:46.6230144.500732422][statime::port::state::slave][INFO] Measurement: Measurement { event_time: Time { inner: 1700815823622633571 }, offset: Some(Duration { inner: 48557.5 }), delay: None, raw_sync_offset: Some(Duration { inner: 95572 }), raw_delay_offset: None }
nov 24 09:49:46 magnesium statime[13032]: [08:49:46.6230444.90814209][statime::filters::basic][INFO] Offset to master: 4.85575e4ns, corrected with phase change -1.2139375e4ns and freq change -8.688730125938628e-2ppm
```

## Further steps

Further information on configuration options for statime can be found in the [man page](../man/statime.8.md)
