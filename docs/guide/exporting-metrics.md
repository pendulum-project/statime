# Exporting metrics

Statime supports exporting key operational metrics to an external [prometheus](https://prometheus.io/) instance.

## Installed from package

If statime was installed from the packages distributed by us, the first step is to enable the observation socket in the configuration. For this, add the following section to the configuration:
```toml
[observability]
observation-path = "/var/run/statime/observe"
```

After restarting statime, the metrics exporter can then be enabled with
```sh
sudo systemctl enable --now statime-metrics-exporter
```

After enabling the metrics exporter, a prometheus metrics dataset will be served on `127.0.0.1:9975/metrics`

The dataset will look something like:
```
# HELP statime_uptime_seconds The time that statime has been running.
# TYPE statime_uptime_seconds gauge
# UNIT statime_uptime_seconds seconds
statime_uptime_seconds{version="0.4.0",build_commit="1fb674074ece5c492327ddd9982bc38dc0fa766e",build_commit_date="2025-03-13"} 648.041376773
# HELP statime_number_ports The amount of ports assigned.
# TYPE statime_number_ports gauge
statime_number_ports{clock_identity="9c:6b:00:05:17:21:00:00"} 1
# HELP statime_quality_class The PTP clock class.
# TYPE statime_quality_class gauge
statime_quality_class{clock_identity="9c:6b:00:05:17:21:00:00"} 248
# HELP statime_quality_accuracy The quality of the clock.
# TYPE statime_quality_accuracy gauge
statime_quality_accuracy{clock_identity="9c:6b:00:05:17:21:00:00"} 254
# HELP statime_quality_offset_scaled_log_variance 2-log of the variance (in seconds^2) of the clock when not synchronized.
# TYPE statime_quality_offset_scaled_log_variance gauge
statime_quality_offset_scaled_log_variance{clock_identity="9c:6b:00:05:17:21:00:00"} 26880
# HELP statime_priority_1 priority 1 used in the BMCA.
# TYPE statime_priority_1 gauge
statime_priority_1{clock_identity="9c:6b:00:05:17:21:00:00"} 128
# HELP statime_priority_2 priority 2 used in the BMCA.
# TYPE statime_priority_2 gauge
statime_priority_2{clock_identity="9c:6b:00:05:17:21:00:00"} 128
# HELP statime_steps_removed The number of paths traversed between this instance and the Grandmaster PTP instance.
# TYPE statime_steps_removed gauge
statime_steps_removed{clock_identity="9c:6b:00:05:17:21:00:00"} 1
# HELP statime_offset_from_master_nanoseconds Time difference between a Master PTP Instance as calculated by the Slave instance.
# TYPE statime_offset_from_master_nanoseconds gauge
# UNIT statime_offset_from_master_nanoseconds nanoseconds
statime_offset_from_master_nanoseconds{clock_identity="9c:6b:00:05:17:21:00:00"} -0.000000005820766091346741
# HELP statime_mean_delay_nanoseconds Packet delay between a Master PTP Instance as calculated by the Slave instance.
# TYPE statime_mean_delay_nanoseconds gauge
# UNIT statime_mean_delay_nanoseconds nanoseconds
statime_mean_delay_nanoseconds{clock_identity="9c:6b:00:05:17:21:00:00"} 0.00000012153759598731995
# HELP statime_grandmaster_clock_quality_class The PTP clock class.
# TYPE statime_grandmaster_clock_quality_class gauge
statime_grandmaster_clock_quality_class{clock_identity="9c:6b:00:05:17:21:00:00",parent_clock_identity="00:0e:fe:ff:fe:03:00:51",parent_port_number="1"} 6
# HELP statime_grandmaster_clock_quality_accuracy The quality of the clock.
# TYPE statime_grandmaster_clock_quality_accuracy gauge
statime_grandmaster_clock_quality_accuracy{clock_identity="9c:6b:00:05:17:21:00:00",parent_clock_identity="00:0e:fe:ff:fe:03:00:51",parent_port_number="1"} 32
# HELP statime_grandmaster_clock_quality_offset_scaled_log_variance 2-log of the variance (in seconds^2) of the grandmaster clock when not synchronized.
# TYPE statime_grandmaster_clock_quality_offset_scaled_log_variance gauge
statime_grandmaster_clock_quality_offset_scaled_log_variance{clock_identity="9c:6b:00:05:17:21:00:00",parent_clock_identity="00:0e:fe:ff:fe:03:00:51",parent_port_number="1"} 29038
# HELP statime_grandmaster_priority_1 priority 1 of the parent's grandmaster.
# TYPE statime_grandmaster_priority_1 gauge
statime_grandmaster_priority_1{clock_identity="9c:6b:00:05:17:21:00:00",parent_clock_identity="00:0e:fe:ff:fe:03:00:51",parent_port_number="1"} 0
# HELP statime_grandmaster_priority_2 priority 2 of the parent's grandmaster.
# TYPE statime_grandmaster_priority_2 gauge
statime_grandmaster_priority_2{clock_identity="9c:6b:00:05:17:21:00:00",parent_clock_identity="00:0e:fe:ff:fe:03:00:51",parent_port_number="1"} 0
# HELP statime_current_utc_offset_seconds Current offset from UTC in seconds.
# TYPE statime_current_utc_offset_seconds gauge
# UNIT statime_current_utc_offset_seconds seconds
statime_current_utc_offset_seconds{clock_identity="9c:6b:00:05:17:21:00:00"} 37
# HELP statime_upcoming_leap_seconds The amount of seconds the last minute of this will be.
# TYPE statime_upcoming_leap_seconds gauge
# UNIT statime_upcoming_leap_seconds seconds
statime_upcoming_leap_seconds{clock_identity="9c:6b:00:05:17:21:00:00"} 60
# HELP statime_time_traceable Whether the timescale is traceable to a primary reference.
# TYPE statime_time_traceable gauge
statime_time_traceable{clock_identity="9c:6b:00:05:17:21:00:00"} 0
# HELP statime_frequency_traceable Whether the frequency determining the timescale is traceable to a primary reference.
# TYPE statime_frequency_traceable gauge
statime_frequency_traceable{clock_identity="9c:6b:00:05:17:21:00:00"} 0
# HELP statime_ptp_timescale Whether the timescale of the Grandmaster PTP Instance is PTP.
# TYPE statime_ptp_timescale gauge
statime_ptp_timescale{clock_identity="9c:6b:00:05:17:21:00:00"} 0
# HELP statime_time_source The source of time used by the Grandmaster PTP instance.
# TYPE statime_time_source gauge
statime_time_source{clock_identity="9c:6b:00:05:17:21:00:00"} 32
# HELP statime_path_trace_enable 1 if path trace options is enabled, 0 otherwise.
# TYPE statime_path_trace_enable gauge
statime_path_trace_enable{clock_identity="9c:6b:00:05:17:21:00:00"} 1
# HELP statime_path_trace_list list of clocks from grandmaster to local clock.
# TYPE statime_path_trace_list gauge
statime_path_trace_list{clock_identity="9c:6b:00:05:17:21:00:00",node="self"} 0
# HELP statime_port_state The current state of the port.
# TYPE statime_port_state gauge
statime_port_state{clock_identity="9c:6b:00:05:17:21:00:00",port="1"} 9
# HELP statime_mean_link_delay_nanoseconds The current mean link delay of the port.
# TYPE statime_mean_link_delay_nanoseconds gauge
# UNIT statime_mean_link_delay_nanoseconds nanoseconds
# EOF
```

## Installed through cargo or from source

When installed through cargo or from source, two things need to be done:
- Enabling the observability socket in the statime configuration.
- Configuring the system to run the statime-metrics-exporter as a service

The observability socket can be enabled by adding the following to the configuration:
```toml
[observability]
observation-path = "/var/run/statime/observe"
```

Next, configure your system to run the statime-metrics-exporter binay as a service. For systemd based systems, an example is provided below.
```ini
[Unit]
Description=Statime metrics exporter
Documentation=https://github.com/pendulum-project/statime
After=statime.service
Requires=statime.service
Conflicts=

[Service]
Type=simple
Restart=always
ExecStart=/usr/bin/statime-metrics-exporter
Environment="RUST_LOG=info"
RuntimeDirectory=statime-observe
User=statime-observe
Group=statime-observe

[Install]
WantedBy=multi-user.target
```