use std::fmt::Write;

use statime::{config::TimePropertiesDS, observability::default::DefaultDS};

use super::exporter::ObservableState;

macro_rules! format_bool {
    ($value:expr) => {
        match $value {
            true => 0,
            false => 1,
        }
    };
}

pub fn format_response(buf: &mut String, state: &ObservableState) -> std::fmt::Result {
    let mut content = String::with_capacity(4 * 1024);
    format_state(&mut content, state)?;

    // headers
    buf.push_str("HTTP/1.1 200 OK\r\n");
    buf.push_str("content-type: text/plain\r\n");
    buf.write_fmt(format_args!("content-length: {}\r\n\r\n", content.len()))?;

    // actual content
    buf.write_str(&content)?;

    Ok(())
}

fn format_default_ds(
    w: &mut impl std::fmt::Write,
    default_ds: &DefaultDS,
    labels: Vec<(&'static str, String)>,
) -> std::fmt::Result {
    format_metric(
        w,
        "number_ports",
        "The amount of ports assigned",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: default_ds.number_ports,
        }],
    )?;

    format_metric(
        w,
        "quality_class",
        "The PTP clock class",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: default_ds.clock_quality.clock_class,
        }],
    )?;

    format_metric(
        w,
        "quality_accuracy",
        "The quality of the clock",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: default_ds.clock_quality.clock_accuracy.to_primitive(),
        }],
    )?;

    format_metric(
        w,
        "quality_offset_scaled_log_variance",
        "2-log of the variance (in seconds^2) of the clock when not synchronized",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: default_ds.clock_quality.offset_scaled_log_variance,
        }],
    )?;

    Ok(())
}

pub fn format_time_properties_ds(
    w: &mut impl std::fmt::Write,
    time_properties_ds: &TimePropertiesDS,
    labels: Vec<(&'static str, String)>,
) -> std::fmt::Result {
    if let Some(current_utc_offset) = time_properties_ds.current_utc_offset {
        format_metric(
            w,
            "current_utc_offset",
            "Current offset from UTC in seconds",
            MetricType::Gauge,
            Some(Unit::Seconds),
            vec![Measurement {
                labels: labels.clone(),
                value: current_utc_offset,
            }],
        )?;
    }

    format_metric(
        w,
        "upcoming_leap",
        "The amount of seconds the last minute of this will be",
        MetricType::Gauge,
        Some(Unit::Seconds),
        vec![Measurement {
            labels: labels.clone(),
            value: match time_properties_ds.leap_indicator() {
                statime::config::LeapIndicator::NoLeap => 60,
                statime::config::LeapIndicator::Leap61 => 61,
                statime::config::LeapIndicator::Leap59 => 59,
            },
        }],
    )?;

    format_metric(
        w,
        "time_traceable",
        "Wheter the timescale is tracable to a primary reference",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: format_bool!(time_properties_ds.time_traceable),
        }],
    )?;

    Ok(())
}

pub fn format_state(w: &mut impl std::fmt::Write, state: &ObservableState) -> std::fmt::Result {
    format_metric(
        w,
        "uptime",
        "The time that statime has been running",
        MetricType::Gauge,
        Some(Unit::Seconds),
        vec![Measurement {
            labels: vec![
                ("version", state.program.version.clone()),
                ("build_commit", state.program.build_commit.clone()),
                ("build_commit_date", state.program.build_commit_date.clone()),
            ],
            value: state.program.uptime_seconds,
        }],
    )?;

    let clock_identity = vec![(
        "clock_identity",
        format!("{}", &state.instance.default_ds.clock_identity),
    )];

    format_default_ds(w, &state.instance.default_ds, clock_identity.clone())?;
    format_time_properties_ds(
        w,
        &state.instance.time_properties_ds,
        clock_identity.clone(),
    )?;

    w.write_str("# EOF\n")?;
    Ok(())
}

fn format_metric<T: std::fmt::Display>(
    w: &mut impl std::fmt::Write,
    name: &str,
    help: &str,
    metric_type: MetricType,
    unit: Option<Unit>,
    measurements: Vec<Measurement<T>>,
) -> std::fmt::Result {
    let name = if let Some(unit) = unit {
        format!("statime_{}_{}", name, unit.as_str())
    } else {
        format!("statime_{}", name)
    };

    // write help text
    writeln!(w, "# HELP {name} {help}.")?;

    // write type
    writeln!(w, "# TYPE {name} {}", metric_type.as_str())?;

    // write unit
    if let Some(unit) = unit {
        writeln!(w, "# UNIT {name} {}", unit.as_str())?;
    }

    // write all the measurements
    for measurement in measurements {
        w.write_str(&name)?;
        if !measurement.labels.is_empty() {
            w.write_str("{")?;

            for (offset, (label, value)) in measurement.labels.iter().enumerate() {
                let value = value
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n");
                write!(w, "{label}=\"{value}\"")?;
                if offset < measurement.labels.len() - 1 {
                    w.write_str(",")?;
                }
            }
            w.write_str("}")?;
        }
        w.write_str(" ")?;
        write!(w, "{}", measurement.value)?;
        w.write_str("\n")?;
    }

    Ok(())
}

struct Measurement<T> {
    labels: Vec<(&'static str, String)>,
    value: T,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Unit {
    Seconds,
}

impl Unit {
    fn as_str(&self) -> &str {
        "seconds"
    }
}

#[allow(dead_code)]
enum MetricType {
    Gauge,
    Counter,
}

impl MetricType {
    fn as_str(&self) -> &str {
        match self {
            MetricType::Gauge => "gauge",
            MetricType::Counter => "counter",
        }
    }
}
