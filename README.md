# Rust implementation of PTP

This repository is work in progress for a rust implementation of PTP version 2.1 (IEEE 1588-2019). It is far from complete and not yet usable in any capacity.

The current state of the project is such that the main binary, when compiled, measures and outputs the time difference to any ptp master clock happening to be sending in the network it listens to.

## Structure

The library has been built in a way to try and be platform-agnostic. To do that, the network and clock have been abstracted.

Many things are event-based where the user needs to call a function on the ptp instance object to let it handle e.g. an incoming network packet.

## Rust version

For compiling this software we advise using the latest version of cargo/rustc as available through rustup. At time of writing this is `1.58.1`.

## Running with elevated privileges

Because of the use of ports 319 and 320 in the PTP protocol, the code here needs to be run as root. It is best to build the code as a non-root user with
```
cargo build
```
and then run it as root with
```
sudo ./target/debug/linux -i <ETHERNET INTERFACE NAME>
```

## PTPd setup for testing

PTPd can be used as a ptp master clock for testing. Because of the port usage required by the PTP standard, this master clock must be on a different machine than that used to run the code in this repository. On Ubuntu, it can be installed with
```bash
apt install ptpd
```
You probably wont want to run this continuously as a service, so disable it with
```bash
service ptpd disable
```
Then, to start ptpd, as root run
```bash
ptpd -V -n -M -i <INTERFACE>
```
where `<INTERFACE>` is the netwerk interface you want ptpd to use. Here `-n` disables clock adjustment by ptpd, and `-M` ensures that it runs in master mode only.

# Support our work

The first phase of our project is kindly support by the [NLnet Foundation](https://nlnet.nl/project/PTP-Rust/).

We seek involvement and/or sponsoring of interested parties, see the announcement [here](https://twitter.com/tweedegolfbv/status/1504439532971827208).
