# STM32H743 example

Complete `statime-embassy-net` application for an STM32H743ZI with an RMII PHY
at address 0, an 8 MHz HSE oscillator, and DHCPv4. The pin mapping is in
[`src/main.rs`](src/main.rs). [`memory.x`](memory.x) places Ethernet descriptors
in SRAM3 and ordinary data, including Xarxa's shared packet pool, in
DMA-accessible AXI SRAM. DTCM cannot hold Ethernet packet payloads. This
example leaves the data cache disabled.

Packet-pool and UDP-socket capacities are configured in
[`.cargo/config.toml`](.cargo/config.toml), including the buffer budget for
descriptors, queued receives, and spare capacity. Increase that budget when
adding other network services.

The sibling Embassy dependency is described in the [crate README](../../README.md).

From this directory:

```console
cargo build --release
cargo run --release
```

The optional `stabilizer` feature resets its PHY through PE3 before Ethernet
initialization.
