//! The measures a run reports, one value per replication, in standard units.

use outliers_sim_engine::ReplicationResult;
use outliers_sim_engine::spec::{Arrivals, Spec, WhenBlocked};
use outliers_sim_model::{Outputs, SummaryItem, ThroughputPer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Seconds,
    /// 0 to 1; shown as a percentage.
    Fraction,
    Parts,
    PartsPerHour,
    PartsPerDay,
    PartsPerWeek,
    /// Per hour of a schedule's working time (`outputs.throughput_per`).
    PartsPerWorkingHour,
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
            Unit::PartsPerDay => "parts/day",
            Unit::PartsPerWeek => "parts/week",
            Unit::PartsPerWorkingHour => "parts/working h",
            Unit::Count => "count",
        }
    }
}

impl serde::Serialize for Unit {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.file_name())
    }
}

#[derive(Debug, Clone)]
pub struct Measure {
    pub object_type: &'static str,
    pub object: String,
    pub name: String,
    pub unit: Unit,
    /// One value per replication; NaN where undefined.
    pub values: Vec<f64>,
}

/// Seconds in a week, for "overtime per week".
const WEEK: f64 = 7.0 * 24.0 * 3600.0;

/// Extracts the measures selected in the model's `outputs.summary`, grouped
/// by object in the order sources, servers, resources, queues, conveyors,
/// sinks. Throughputs are per `outputs.throughput_per`.
pub fn collect(spec: &Spec, outputs: &Outputs, results: &[ReplicationResult]) -> Vec<Measure> {
    let wants = |item| outputs.summary.contains(&item);
    // Parts counted over the measured period, as a rate.
    let e = &spec.experiment;
    let (per_unit, hours): (Unit, f64) = match outputs.throughput_per {
        ThroughputPer::Hour => (Unit::PartsPerHour, 1.0),
        ThroughputPer::Day => (Unit::PartsPerDay, 24.0),
        ThroughputPer::Week => (Unit::PartsPerWeek, 168.0),
        ThroughputPer::WorkingHour(schedule) => {
            let on = spec.schedules[schedule]
                .on_time(e.warmup, e.warmup + e.run_length)
                .as_secs_f64();
            // Per working hour: the measured period's length over its working hours.
            (
                Unit::PartsPerWorkingHour,
                e.run_length.as_secs_f64() / on.max(f64::MIN_POSITIVE),
            )
        }
    };
    let rate = move |count: u64, r: &ReplicationResult| count as f64 / r.measured * 3600.0 * hours;
    let types_at = spec.types_at();
    let mut out = Vec::new();
    let mut add =
        |object_type, object: &str, name: &str, unit, f: &dyn Fn(&ReplicationResult) -> f64| {
            out.push(Measure {
                object_type,
                object: object.to_string(),
                name: name.to_string(),
                unit,
                values: results.iter().map(f).collect(),
            });
        };

    if wants(SummaryItem::SourceBlockedTime) {
        for (i, s) in spec.sources.iter().enumerate() {
            if matches!(s.arrivals, Arrivals::Restock(_)) {
                add("source", &s.name, "orders placed", Unit::Count, &|r| {
                    r.sources[i].orders as f64
                });
                add("source", &s.name, "on order", Unit::Parts, &|r| {
                    r.sources[i].on_order
                });
            }
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
            if let Some(schedule) = s.schedule {
                add("server", &s.name, "off shift", Unit::Fraction, &|r| {
                    r.servers[i].off_shift
                });
                if spec.schedules[schedule].finish_work {
                    add(
                        "server",
                        &s.name,
                        "overtime per week",
                        Unit::Seconds,
                        &|r| r.servers[i].overtime * WEEK,
                    );
                }
            }
            if !s.needs.is_empty() {
                add(
                    "server",
                    &s.name,
                    "waiting for resources",
                    Unit::Fraction,
                    &|r| r.servers[i].waiting_for_resources,
                );
            }
            if s.changeover.is_some() {
                add("server", &s.name, "changeover", Unit::Fraction, &|r| {
                    r.servers[i].changeover
                });
                add("server", &s.name, "changeovers", Unit::Count, &|r| {
                    r.servers[i].changeovers as f64
                });
            }
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
    if wants(SummaryItem::ServerStates) {
        for (i, res) in spec.resources.iter().enumerate() {
            add("resource", &res.name, "busy", Unit::Fraction, &|r| {
                r.resources[i].busy
            });
            if let Some(schedule) = res.schedule {
                add("resource", &res.name, "off shift", Unit::Fraction, &|r| {
                    r.resources[i].off_shift
                });
                if spec.schedules[schedule].finish_work {
                    add(
                        "resource",
                        &res.name,
                        "overtime per week",
                        Unit::Seconds,
                        &|r| r.resources[i].overtime * WEEK,
                    );
                }
            }
        }
    }
    if wants(SummaryItem::ConveyorOccupancy) {
        for (i, c) in spec
            .conveyors
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_queue())
        {
            add("queue", &c.name, "mean parts", Unit::Parts, &|r| {
                r.conveyors[i].mean_parts
            });
            add("queue", &c.name, "max parts", Unit::Parts, &|r| {
                r.conveyors[i].max_parts as f64
            });
            add("queue", &c.name, "waiting time", Unit::Seconds, &|r| {
                r.conveyors[i].time.mean()
            });
            add(
                "queue",
                &c.name,
                "waiting time (90th percentile)",
                Unit::Seconds,
                &|r| {
                    r.conveyors[i]
                        .times
                        .as_ref()
                        .map_or(f64::NAN, |h| h.quantile(0.9))
                },
            );
            add("queue", &c.name, "longest wait", Unit::Seconds, &|r| {
                let t = &r.conveyors[i].time;
                if t.count == 0 { f64::NAN } else { t.max }
            });
            if c.target_wait.is_some() {
                add(
                    "queue",
                    &c.name,
                    "within target wait",
                    Unit::Fraction,
                    &|r| {
                        let c = &r.conveyors[i];
                        c.within_target as f64 / c.time.count as f64
                    },
                );
            }
            // With several part types waiting here, the wait of each.
            let waiting: Vec<usize> = (0..spec.entities.len())
                .filter(|&e| types_at.conveyors[i][e])
                .collect();
            if waiting.len() > 1 {
                for &e in &waiting {
                    add(
                        "queue",
                        &c.name,
                        &format!("waiting time of {}", spec.entities[e]),
                        Unit::Seconds,
                        &|r| r.conveyors[i].time_by_type[e].mean(),
                    );
                }
            }
            if c.capacity != u32::MAX {
                add("queue", &c.name, "full", Unit::Fraction, &|r| {
                    r.conveyors[i].full
                });
            }
            // A stock: how often it ran out.
            let stocked = c.starts_with.is_some()
                || spec
                    .sources
                    .iter()
                    .any(|s| matches!(&s.arrivals, Arrivals::Restock(r) if r.queue == i));
            if stocked {
                add("queue", &c.name, "empty", Unit::Fraction, &|r| {
                    r.conveyors[i].empty
                });
            }
            add("queue", &c.name, "throughput", per_unit, &|r| {
                rate(r.conveyors[i].entered, r)
            });
        }
        for (i, c) in spec
            .conveyors
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.is_queue())
        {
            add("conveyor", &c.name, "mean parts", Unit::Parts, &|r| {
                r.conveyors[i].mean_parts
            });
            add("conveyor", &c.name, "max parts", Unit::Parts, &|r| {
                r.conveyors[i].max_parts as f64
            });
            add("conveyor", &c.name, "full", Unit::Fraction, &|r| {
                r.conveyors[i].full
            });
            add("conveyor", &c.name, "throughput", per_unit, &|r| {
                rate(r.conveyors[i].entered, r)
            });
        }
    }
    for (i, s) in spec.sinks.iter().enumerate() {
        if wants(SummaryItem::Throughput) {
            add("sink", &s.name, "throughput", per_unit, &|r| {
                rate(r.sinks[i].exited, r)
            });
        }
        if wants(SummaryItem::TimeInSystem) {
            add("sink", &s.name, "time in system", Unit::Seconds, &|r| {
                r.sinks[i].time_in_system.mean()
            });
        }
        // With several part types arriving here, the same per type.
        let arriving: Vec<usize> = (0..spec.entities.len())
            .filter(|&e| types_at.sinks[i][e])
            .collect();
        if arriving.len() > 1 {
            for &e in &arriving {
                let entity = &spec.entities[e];
                if wants(SummaryItem::Throughput) {
                    add(
                        "sink",
                        &s.name,
                        &format!("throughput of {entity}"),
                        per_unit,
                        &|r| rate(r.sinks[i].by_type[e].exited, r),
                    );
                }
                if wants(SummaryItem::TimeInSystem) {
                    add(
                        "sink",
                        &s.name,
                        &format!("time in system of {entity}"),
                        Unit::Seconds,
                        &|r| r.sinks[i].by_type[e].time_in_system.mean(),
                    );
                }
            }
        }
    }
    out
}
