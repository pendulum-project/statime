use std::fmt::Write;

use statime::{
    config::TimePropertiesDS,
    observability::{current::CurrentDS, default::DefaultDS, parent::ParentDS},
};

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

    format_metric(
        w,
        "priority_1",
        "priority 1 used in the BMCA",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: default_ds.priority_1,
        }],
    )?;

    format_metric(
        w,
        "priority_2",
        "priority 2 used in the BMCA",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: default_ds.priority_2,
        }],
    )?;

    Ok(())
}

pub fn format_current_ds(
    w: &mut impl std::fmt::Write,
    current_ds: &CurrentDS,
    labels: Vec<(&'static str, String)>,
) -> std::fmt::Result {
    format_metric(
        w,
        "steps_removed",
        "The number of paths traversed between this instance and the Grandmaster PTP instance",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: current_ds.steps_removed,
        }],
    )?;

    format_metric(
        w,
        "offset_from_master",
        "Time difference between a Master PTP Instance as calculated by the Slave instance",
        MetricType::Gauge,
        Some(Unit::Nanoseconds),
        vec![Measurement {
            labels: labels.clone(),
            value: current_ds.offset_from_master,
        }],
    )?;

    Ok(())
}

pub fn format_parent_ds(
    w: &mut impl std::fmt::Write,
    parent_ds: &ParentDS,
    labels: Vec<(&'static str, String)>,
) -> std::fmt::Result {
    format_metric(
        w,
        "grandmaster_clock_quality_class",
        "The PTP clock class",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: parent_ds.grandmaster_clock_quality.clock_class,
        }],
    )?;

    format_metric(
        w,
        "grandmaster_clock_quality_accuracy",
        "The quality of the clock",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: parent_ds
                .grandmaster_clock_quality
                .clock_accuracy
                .to_primitive(),
        }],
    )?;

    format_metric(
        w,
        "grandmaster_clock_quality_offset_scaled_log_variance",
        "2-log of the variance (in seconds^2) of the grandmaster clock when not synchronized",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: parent_ds
                .grandmaster_clock_quality
                .offset_scaled_log_variance,
        }],
    )?;

    format_metric(
        w,
        "grandmaster_priority_1",
        "priority 1 of the parent's grandmaster",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: parent_ds.grandmaster_priority_1,
        }],
    )?;

    format_metric(
        w,
        "grandmaster_priority_2",
        "priority 2 of the parent's grandmaster",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: parent_ds.grandmaster_priority_2,
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

    format_metric(
        w,
        "frequency_traceable",
        "Wheter the frequence determining the timescale is tracable to a primary reference",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: format_bool!(time_properties_ds.frequency_traceable),
        }],
    )?;

    format_metric(
        w,
        "ptp_timescale",
        "Wheter the timescale of the Grandmaster PTP Instance is PTP",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: format_bool!(time_properties_ds.ptp_timescale),
        }],
    )?;

    format_metric(
        w,
        "time_source",
        "Wheter the timescale of the Grandmaster PTP Instance is PTP",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: time_properties_ds.time_source.to_primitive(),
        }],
    )?;

    Ok(())
}

fn format_path_trace_ds(
    w: &mut impl Write,
    path_trace_ds: &statime::observability::PathTraceDS,
    labels: Vec<(&'static str, String)>,
) -> std::fmt::Result {
    format_metric(
        w,
        "path_trace_enable",
        "true if path trace options is enabled",
        MetricType::Gauge,
        None,
        vec![Measurement {
            labels: labels.clone(),
            value: path_trace_ds.enable,
        }],
    )?;

    let mut measurements: Vec<_> = path_trace_ds
        .list
        .iter()
        .enumerate()
        .map(|(steps_removed, clock_identity)| {
            let mut labels = labels.clone();
            labels.push(("node", clock_identity.to_string()));
            Measurement {
                labels,
                value: steps_removed,
            }
        })
        .collect();

    let mut last_labels = labels.clone();
    last_labels.push(("node", "self".into()));
    measurements.push(Measurement {
        labels: last_labels,
        value: path_trace_ds.list.len(),
    });

    format_metric(
        w,
        "path_trace_list",
        "list of clocks from grandmaster to local clock",
        MetricType::Gauge,
        None,
        measurements,
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

    let labels = vec![(
        "clock_identity",
        format!("{}", &state.instance.default_ds.clock_identity),
    )];

    format_default_ds(w, &state.instance.default_ds, labels.clone())?;
    format_current_ds(w, &state.instance.current_ds, labels.clone())?;
    format_parent_ds(w, &state.instance.parent_ds, labels.clone())?;
    format_time_properties_ds(w, &state.instance.time_properties_ds, labels.clone())?;
    format_path_trace_ds(w, &state.instance.path_trace_ds, labels.clone())?;

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
    Nanoseconds,
}

impl Unit {
    fn as_str(&self) -> &str {
        match self {
            Unit::Seconds => "seconds",
            Unit::Nanoseconds => "nanoseconds",
        }
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
