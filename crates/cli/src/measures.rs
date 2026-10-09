//! The measures a run reports, one value per replication, in standard units.

use outliers_sim_engine::ReplicationResult;
use outliers_sim_engine::spec::{Spec, WhenBlocked};
use outliers_sim_model::SummaryItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Seconds,
    /// 0 to 1; shown as a percentage.
    Fraction,
    Parts,
    PartsPerHour,
    Count,
}

impl Unit {
    /// How the unit is written in result files.
    pub fn file_name(self) -> &'static str {
        match self {
            Unit::Seconds => "s",
            Unit::Fraction => "fraction",
            Unit::Parts => "parts",
            Unit::PartsPerHour => "parts/h",
            Unit::Count => "count",
        }
    }
}

pub struct Measure {
    pub object_type: &'static str,
    pub object: String,
    pub name: &'static str,
    pub unit: Unit,
    /// One value per replication; NaN where undefined.
    pub values: Vec<f64>,
}

/// Extracts the measures selected in the model's `outputs.summary`, grouped
/// by object in the order sources, servers, conveyors, sinks.
pub fn collect(spec: &Spec, items: &[SummaryItem], results: &[ReplicationResult]) -> Vec<Measure> {
    let wants = |item| items.contains(&item);
    let mut out = Vec::new();
    let mut add = |object_type, object: &str, name, unit, f: &dyn Fn(&ReplicationResult) -> f64| {
        out.push(Measure {
            object_type,
            object: object.to_string(),
            name,
            unit,
            values: results.iter().map(f).collect(),
        });
    };

    if wants(SummaryItem::SourceBlockedTime) {
        for (i, s) in spec.sources.iter().enumerate() {
            match s.when_blocked {
                WhenBlocked::Wait => add("source", &s.name, "blocked", Unit::Fraction, &|r| {
                    r.sources[i].blocked
                }),
                WhenBlocked::Drop => add("source", &s.name, "lost", Unit::Fraction, &|r| {
                    r.sources[i].lost as f64 / r.sources[i].created as f64
                }),
            }
        }
    }
    if wants(SummaryItem::ServerStates) {
        for (i, s) in spec.servers.iter().enumerate() {
            add("server", &s.name, "idle", Unit::Fraction, &|r| {
                r.servers[i].idle
            });
            add("server", &s.name, "busy", Unit::Fraction, &|r| {
                r.servers[i].busy
            });
            add("server", &s.name, "blocked", Unit::Fraction, &|r| {
                r.servers[i].blocked
            });
            if !s.failures.is_empty() {
                add("server", &s.name, "failed", Unit::Fraction, &|r| {
                    r.servers[i].failed
                });
                add("server", &s.name, "failures", Unit::Count, &|r| {
                    r.servers[i].failures as f64
                });
            }
        }
    }
    if wants(SummaryItem::ConveyorOccupancy) {
        for (i, c) in spec.conveyors.iter().enumerate() {
            add("conveyor", &c.name, "mean parts", Unit::Parts, &|r| {
                r.conveyors[i].mean_parts
            });
            add("conveyor", &c.name, "max parts", Unit::Parts, &|r| {
                r.conveyors[i].max_parts as f64
            });
            add("conveyor", &c.name, "full", Unit::Fraction, &|r| {
                r.conveyors[i].full
            });
        }
    }
    for (i, s) in spec.sinks.iter().enumerate() {
        if wants(SummaryItem::Throughput) {
            add("sink", &s.name, "throughput", Unit::PartsPerHour, &|r| {
                r.sinks[i].exited as f64 / r.measured * 3600.0
            });
        }
        if wants(SummaryItem::TimeInSystem) {
            add("sink", &s.name, "time in system", Unit::Seconds, &|r| {
                r.sinks[i].time_in_system.mean()
            });
        }
    }
    out
}
