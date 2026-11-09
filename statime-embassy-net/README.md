# statime-embassy-net

PTP ordinary-clock runner for `statime` on timestamp-capable `embassy-net`
Ethernet drivers.

Construct `Runner::new(iface, clock, config, filter_config)` for an IPv4
Ethernet interface and run it alongside the Embassy network runner. It must
be the stack's only TX timestamp requester and consumer. PTP sockets remain
bound to the selected interface and its clock. Reserve two UDP sockets for
PTP ports 319 and 320.

The clock must control the hardware time domain used for packet timestamps.
`EmbassyClock` adapts an initialized `embassy_ptp_driver::Clock` to Statime's
clock interface. Clock configuration and network interface setup belong to
the application.

The runner supports single-port UDP/IPv4 with E2E delay measurement and is
slave-only by default. See `Config` and `Runner::run` for configuration and
lifecycle behavior.

Enable `defmt` for diagnostics or `monitor` with `Runner::with_monitor` for
protocol/filter activity indicators. These indicators do not establish clock
accuracy or lock.

See the [`examples/stm32h743`](examples/stm32h743) package for a complete
STM32H743 Embassy application.

The Cargo patches pin an upstream Embassy main revision that includes the TX
timestamp queue. Embassy pins the matching Xarxa revision. For STM32, enable
`embassy-stm32/ptp`; this crate enables the required Embassy network timestamp
features.
