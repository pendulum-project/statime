# Validation

As part of our work on boundary clock and ethernet frame support, we conducted another set of measurements of statime to validate its performance is as expected.

Our results indicate that performance is still in line with previous validation reports. The impact of a boundary clock on performance appears small (200ns), though an observed sawtooth pattern makes interpretation of that result complicated.

## Test setup

Two setups were used during the experiments in this paper. In all setups, the PTP grandmaster was a Endrun Ninja, and we used a machine with an intel i210 network card as ordinary clock for measuring. The offset between the clocks of the grandmaster and the i210 were determined by having both output a pulse per second signal. All PTP traffic used raw ethernet frames as transport, and was connected through an HP Aruba 2930 JL319A with PTP support enabled on all ports.

For the direct connection test, all ports on the switch were configured to be on the same VLAN, resulting in direct time synchronization between the grandmaster and the i210 machine.

For the boundary clock test, a machine with a 4-port Broadcom NetXtreme BCM5719 Gigabit Ethernet adapter was configured as boundary clock, synchronizing time from a VLAN with the grandmaster clock and providing it to a second VLAN with the boundary clock on it.

All machines except for the grandmaster used statime commit hash c8561310 as their PTP software. The grandmaster used the stock software provided by Endrun.

## Test procedure

For each test, all machines were turned on in their respective configuration, and the entire setup was then given at least 2 hours to fully stabilize. Then the time difference between the grandmaster and the i210 were measured every second over a time period of 2 hours.

For this data we then calculated both the mean and the sample standard deviation. The mean represents a measure of asymetry in the setup, and will be ignored in the rest of this text as it can be easily corrected for. The sample standard deviation provides us with a measure of the quality of the synchronization.

## Results

![plot of offset to grand master clock for direct connection](i210-direct-offset.png?raw=true)

The above shows the offset of the clock of the i210 when synchronized directly to the grandmaster. This has a sample standard deviation of $1.4$ microseconds, though the sawtooth means it is not at all normally distributed.


![plot of offset to grand master clock for connection via boundary](i210-bc-offset.png?raw=true)

This shows the offset of the clock of the i210 when synchronized via a boundary clock. This has a sample standard deviation of $1.6$ microseconds. Note that the sawtooth here is quite a bit faster than in the direct synchronization case.

## Conclusions

The results are in line with those seen in the earlier [hardware timestamping validation results](../24-05-2022-hardware-timestamping/measurement_report.pdf), which was expected as no significant changes have occured to how we use the measurements to adjust the clock. The observed sawtooth patterns indicate that we can make significant gains by improving how we use measurements to adjust the clock, and improvements in that are currently under development.

With regards to the impact of the boundary clock, this seems to be limited with the current software, making the synchronization about 200ns worse. However, in light of the observed sawtooth patterns it is hard to determine to what degree this is from just the increased frequency of the sawtooth, and what part is actual stochastic worsening.
