<!-- ---
title: STATIME(8) statime 0.4.0 | statime
--- -->

# NAME

`statime` - The Statime PTP daemon for linux

# SYNOPSIS
`statime` [`-c` *path*] \
`statime` `-h` \
`statime` `-V`

# DESCRIPTION

...

# OPTIONS
`-c` *path*, `--config`=*path*
:   Path to the configuration file for the statime daemon. If not specified this
    defaults to `/etc/statime/statime.toml`.

`-h`, `--help`
:   Display usage instructions.

`-V`, `--version`
:   Display version information.

# SEE ALSO

[statime-metrics-exporter(8)](statime-metrics-exporter.8.md), [statime.toml(5)](statime.toml.5.md)