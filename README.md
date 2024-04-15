# Statime

[![codecov](https://codecov.io/gh/pendulum-project/statime/branch/main/graph/badge.svg?token=QCO6NKS64J)](https://codecov.io/gh/pendulum-project/statime)
[![book](https://shields.io/badge/manual-main-blue)](https://docs.statime.pendulum-project.org/)
[![book](https://shields.io/badge/docs.rs-statime-green)](https://docs.statime.pendulum-project.org/api/statime/)

Statime is a library providing an implementation of PTP version 2.1 (IEEE1588-2019). It provides all the building blocks to setup PTP ordinary and boundary clocks.

It is designed to be able to work with many different underlying platforms, including embedded targets. This does mean that it cannot use the standard library and platform specific libraries to interact with the system clock and to access the network. That needs to be provided by the user of the library.

On modern Linux kernels, the `statime-linux` crate provides ready to use implementations of these interfaces. For other platforms the user will need to implement these themselves.

The `statime-stm32` crate gives an example of how to use statime on an embedded target.

<p align="center">
<img width="216px" alt="Statime - PTP in Rust" src="https://tweedegolf.nl/images/statime.jpg" />
</p>

The `statime-linux` crate provides a binary for Linux implementing an ordinary or boundary clock. It will need sufficient permissions to change the system clock to use. The easiest way to start it is through sudo:
```
sudo ./target/debug/statime -c path/to/statime.toml
```

## Structure

The `statime` library has been built in a way to try and be platform-agnostic. To do that, the network and clock have been abstracted. The `statime-linux` library provides implementations of these abstractions for linux-based platforms. For other platforms, this needs to be provided by the user. For more details, see [the documentation](https://docs.statime.pendulum-project.org/api/statime/)

## Rust version

Statime requires a nigthly version of cargo/rust. The easiest way to obtain these is through [rustup](https://rustup.rs)

## Running with elevated privileges

Because of the use of ports 319 and 320 in the PTP protocol, the code here needs to be run as root. It is best to build the code as a non-root user with
```
cargo +nightly build
```
and then run it as root with
```
sudo ./target/debug/statime -i <ETHERNET INTERFACE NAME>
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
where `<INTERFACE>` is the network interface you want ptpd to use. Here `-n` disables clock adjustment by ptpd, and `-M` ensures that it runs in master mode only.

# Roadmap

- Q2 2024: Power sector profile + experimental NTS4PTP
- Q3 2024: Stable release Statime (pending funding)
- Q1-Q4 2024: Adoption work & maintenance work

# Support our work

The development of Statime is kindly supported by the NGI Assure Fund of the [NLnet Foundation](https://nlnet.nl).

<img style="margin: 1rem 5% 1rem 5%;" src="https://nlnet.nl/logo/banner.svg" alt="Logo NLnet"  width="150px" />
<img style="margin: 1rem 5% 1rem 5%;" src="https://nlnet.nl/image/logos/NGIAssure_tag.svg" alt="Logo NGI Assure" width="150px" />

[SIDN Fonds](https://www.sidnfonds.nl/excerpt) is supporting us with a grant to develop clock devices running Statime and ntpd-rs, in collaboration with SIDN Labs' [TimeNL](https://www.sidnlabs.nl/en/news-and-blogs/an-open-infrastructure-for-sub-millisecond-internet-time).

In August of 2023, Sovereign Tech Fund invested in Pendulum (Statime and ntpd-rs). Read more on [their website](https://sovereigntechfund.de/en/projects/pendulum/).

<img style="margin: 1rem 5% 1rem 5%;" src="https://tweedegolf.nl/images/logo-stf-blank.png" alt="Logo STF" width="250px" />

We continuously seek the involvement of interested parties and funding for future work. See [Project Pendulum](https://github.com/pendulum-project) or reach out to pendulum@tweedegolf.com.
