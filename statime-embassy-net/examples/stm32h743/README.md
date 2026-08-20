# STM32H743 example

Complete `statime-embassy-net` application for an STM32H743ZI with an RMII PHY
at address 0, an 8 MHz HSE oscillator, and DHCPv4. The pin mapping is in
[`src/main.rs`](src/main.rs); Ethernet DMA storage is placed in SRAM3 by
[`memory.x`](memory.x).

From this directory:

```console
cargo build --release
cargo run --release
```

The optional `stabilizer` feature resets its PHY through PE3 before Ethernet
initialization.
