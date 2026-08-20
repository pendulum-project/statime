# statime-embassy-net

PTP ordinary-clock runner for `statime` on timestamp-capable `embassy-net`
Ethernet drivers.

This crate connects:

- `statime` for the PTP protocol and servo,
- `embassy-net` for UDP multicast transport,
- a `statime::Clock` implementation controlling the same hardware clock used
  for packet timestamps.

The network driver must provide packet timestamps through `embassy-net` packet
metadata and asynchronous transmit timestamp polling. `EmbassyClock` adapts
any `embassy_ptp_driver::Clock` to Statime's clock interface. The example
uses Embassy STM32; applications using it must select their concrete
`embassy-stm32` chip feature.

The runner is currently a single-port UDP/IPv4 ordinary clock using E2E delay
measurement. It is slave-only by default. To keep static packet storage small,
it accepts PTP datagrams up to 256 bytes; larger, TLV-heavy messages are
discarded as truncated.

Runner startup reports multicast-membership and socket-binding errors. Once
started, transient link or IP-configuration loss keeps the clock in holdover;
the existing sockets and protocol state resume when connectivity returns.

The default servo is Statime's `FixedWanderKalmanFilter`, intended for embedded
systems whose oscillator wander is characterized or conservatively bounded.

The default feature set has no logging backend. Enable `defmt` for diagnostics
and `monitor` to expose lock-free tracking and holdover state. Enabling
`monitor` does not change `Runner::new`; attach a monitor with
`Runner::with_monitor` where needed.

See the [`examples/stm32h743`](examples/stm32h743) package for a complete
STM32H743 Embassy application. Its target, linker, probe, and board dependencies
are kept outside the reusable library package.
