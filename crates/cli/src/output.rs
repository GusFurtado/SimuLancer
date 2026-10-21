//! Result files: summary.json, replications.csv, report.html and the
//! events.jsonl event log.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use outliers_sim_engine::spec::{Node, Spec, WhenBlocked};
use outliers_sim_engine::stats::Summary;
use outliers_sim_engine::{Observer, SimTime, TraceEvent};
use outliers_sim_model::{Layout, Model};
use serde::Serialize;
use serde_json::json;

use crate::compare::Comparison;
use crate::measures::Measure;

pub const CONFIDENCE: f64 = 0.95;

const REPORT_TEMPLATE: &str = include_str!("report.html");

/// The checked layout for viewers, in meters (Y up), as JSON: viewers don't
/// need a YAML parser.
pub fn write_layout(path: &Path, layout: &Layout) -> io::Result<()> {
    fs::write(path, format!("{}\n", layout_json(layout)))
}

/// The layout as viewers read it (`layout.json`).
pub fn layout_json(layout: &Layout) -> serde_json::Value {
    let objects: serde_json::Map<String, serde_json::Value> = layout
        .objects
        .iter()
        .map(|o| {
            (
                o.name.clone(),
                json!({ "position": o.position, "model": o.model }),
            )
        })
        .collect();
    let conveyors: serde_json::Map<String, serde_json::Value> = layout
        .conveyors
        .iter()
        .map(|c| (c.name.clone(), json!({ "path": c.path })))
        .collect();
    json!({
        "format": "outliers-sim-layout/0.1",
        "units": "m",
        "objects": objects,
        "conveyors": conveyors,
    })
}

/// A self-contained HTML report: the template with the results embedded as
/// JSON. No network access needed to view it.
pub fn write_report(
    path: &Path,
    model: &Model,
    spec: &Spec,
    measures: &[(Measure, Summary)],
) -> io::Result<()> {
    let node = |n: Node| match n {
        Node::Source(i) => &spec.sources[i].name,
        Node::Server(i) => &spec.servers[i].name,
        Node::Sink(i) => &spec.sinks[i].name,
    };
    let e = &spec.experiment;
    let data = json!({
        "model": model.name,
        "description": model.description,
        "version": env!("CARGO_PKG_VERSION"),
        "seed": e.seed,
        "replications": e.replications,
        "warmup": e.warmup.as_secs_f64(),
        "run_length": e.run_length.as_secs_f64(),
        "confidence": CONFIDENCE,
        "sources": spec.sources.iter().map(|s| json!({
            "name": s.name,
            "when_blocked": match s.when_blocked {
                WhenBlocked::Wait => "wait",
                WhenBlocked::Drop => "drop",
            },
        })).collect::<Vec<_>>(),
        "servers": spec.servers.iter().map(|s| json!({
            "name": s.name,
            "capacity": s.capacity,
        })).collect::<Vec<_>>(),
        "conveyors": spec.conveyors.iter().map(|c| json!({
            "name": c.name,
            "queue": c.is_queue(),
            "from": node(c.from),
            "to": node(c.to),
            "capacity": (c.capacity != u32::MAX).then_some(c.capacity),
        })).collect::<Vec<_>>(),
        "sinks": spec.sinks.iter().map(|s| &s.name).collect::<Vec<_>>(),
        "measures": measures.iter().map(|(m, s)| json!({
            "object_type": m.object_type,
            "object": m.object,
            "measure": m.name,
            "unit": m.unit.file_name(),
            "n": s.n,
            "mean": s.mean,
            "half_width": s.half_width,
            "min": s.min,
            "max": s.max,
            "values": m.values,
        })).collect::<Vec<_>>(),
    });
    // Inside <script>, "</" could close the tag early (e.g. in a description).
    let json = data.to_string().replace("</", "<\\/");
    fs::write(
        path,
        REPORT_TEMPLATE.replacen("__OUTLIERS_SIM_DATA__", &json, 1),
    )
}

#[derive(Serialize)]
struct SummaryFile<'a> {
    format: &'static str,
    model: &'a str,
    seed: u64,
    replications: u32,
    /// Seconds.
    warmup: f64,
    /// Seconds, measured after the warm-up.
    run_length: f64,
    confidence: f64,
    measures: Vec<MeasureSummary<'a>>,
}

#[derive(Serialize)]
struct MeasureSummary<'a> {
    object_type: &'a str,
    object: &'a str,
    measure: &'a str,
    unit: &'a str,
    /// Replications where the measure is defined. NaN values are written as null.
    n: usize,
    mean: f64,
    half_width: f64,
    sd: f64,
    min: f64,
    max: f64,
}

pub fn write_summary(
    path: &Path,
    model_name: &str,
    spec: &Spec,
    measures: &[(Measure, Summary)],
) -> io::Result<()> {
    let e = &spec.experiment;
    let file = SummaryFile {
        format: "outliers-sim-summary/0.1",
        model: model_name,
        seed: e.seed,
        replications: e.replications,
        warmup: e.warmup.as_secs_f64(),
        run_length: e.run_length.as_secs_f64(),
        confidence: CONFIDENCE,
        measures: measures
            .iter()
            .map(|(m, s)| MeasureSummary {
                object_type: m.object_type,
                object: &m.object,
                measure: &m.name,
                unit: m.unit.file_name(),
                n: s.n,
                mean: s.mean,
                half_width: s.half_width,
                sd: s.sd,
                min: s.min,
                max: s.max,
            })
            .collect(),
    };
    let mut text = serde_json::to_string_pretty(&file).map_err(io::Error::other)?;
    text.push('\n');
    fs::write(path, text)
}

/// Base vs scenario for every shared measure, in standard units.
pub fn write_comparison(
    path: &Path,
    base_name: &str,
    scenario_name: &str,
    spec: &Spec,
    comparisons: &[Comparison],
) -> io::Result<()> {
    let e = &spec.experiment;
    let stats =
        |s: &Summary| json!({ "n": s.n, "mean": s.mean, "half_width": s.half_width, "sd": s.sd });
    let data = json!({
        "format": "outliers-sim-comparison/0.1",
        "base": base_name,
        "scenario": scenario_name,
        "seed": e.seed,
        "replications": e.replications,
        "warmup": e.warmup.as_secs_f64(),
        "run_length": e.run_length.as_secs_f64(),
        "confidence": CONFIDENCE,
        "common_random_numbers": true,
        "measures": comparisons.iter().map(|c| json!({
            "object_type": c.base.object_type,
            "object": c.base.object,
            "measure": c.base.name,
            "unit": c.base.unit.file_name(),
            "base": stats(&c.base_summary),
            "scenario": stats(&c.scenario_summary),
            "difference": stats(&c.difference),
            "unpaired_half_width": c.unpaired_half_width,
            "verdict": c.verdict.describe(),
        })).collect::<Vec<_>>(),
    });
    let mut text = serde_json::to_string_pretty(&data).map_err(io::Error::other)?;
    text.push('\n');
    fs::write(path, text)
}

/// One row per replication and measure ("long" format), for spreadsheets,
/// pandas or R.
pub fn write_replications_csv(path: &Path, measures: &[(Measure, Summary)]) -> io::Result<()> {
    let mut out = String::from("replication,object_type,object,measure,unit,value\n");
    for (m, _) in measures {
        for (i, v) in m.values.iter().enumerate() {
            let value = if v.is_finite() {
                v.to_string()
            } else {
                String::new()
            };
            out += &format!(
                "{},{},{},{},{},{value}\n",
                i + 1,
                m.object_type,
                m.object,
                m.name,
                m.unit.file_name()
            );
        }
    }
    fs::write(path, out)
}

/// Writes the trace of one replication as JSON Lines: a header describing
/// the model, then one event per line. Times are seconds with microsecond
/// precision, written exactly. Objects are referred to by name.
pub struct EventLog<'a, W: Write> {
    out: W,
    spec: &'a Spec,
    error: Option<io::Error>,
}

impl<'a, W: Write> EventLog<'a, W> {
    pub fn new(mut out: W, model_name: &str, spec: &'a Spec, replication: u32) -> io::Result<Self> {
        let node = |n: Node| match n {
            Node::Source(i) => &spec.sources[i].name,
            Node::Server(i) => &spec.servers[i].name,
            Node::Sink(i) => &spec.sinks[i].name,
        };
        let e = &spec.experiment;
        let header = json!({
            "format": "outliers-sim-events/0.1",
            "model": model_name,
            "replication": replication,
            "seed": e.seed,
            "warmup": e.warmup.as_secs_f64(),
            "end": (e.warmup + e.run_length).as_secs_f64(),
            "entities": spec.entities,
            "sources": spec.sources.iter().map(|s| &s.name).collect::<Vec<_>>(),
            "servers": spec.servers.iter().map(|s| json!({
                "name": s.name,
                "capacity": s.capacity,
                "failures": s.failures.iter().map(|f| &f.name).collect::<Vec<_>>(),
                "schedule": s.schedule.map(|i| &spec.schedules[i].name),
            })).collect::<Vec<_>>(),
            "conveyors": spec.conveyors.iter().map(|c| json!({
                "name": c.name,
                "queue": c.is_queue(),
                "from": node(c.from),
                "to": node(c.to),
                "length": c.length,
                "speed": c.speed,
                "spacing": c.spacing,
                "capacity": c.capacity,
            })).collect::<Vec<_>>(),
            "sinks": spec.sinks.iter().map(|s| &s.name).collect::<Vec<_>>(),
            "resources": spec.resources.iter().map(|r| json!({ "name": r.name, "capacity": r.capacity })).collect::<Vec<_>>(),
        });
        writeln!(out, "{header}")?;
        Ok(EventLog {
            out,
            spec,
            error: None,
        })
    }

    /// Flushes and reports the first write error, if any.
    pub fn finish(mut self) -> io::Result<()> {
        if let Some(err) = self.error.take() {
            return Err(err);
        }
        self.out.flush()
    }

    fn write(&mut self, time: SimTime, event: TraceEvent) -> io::Result<()> {
        let s = self.spec;
        let micros = time.as_micros();
        let t = format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000);
        // Names are validated identifiers, so they need no JSON escaping.
        match event {
            TraceEvent::Created {
                part,
                source,
                entity,
            } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"created","part":{part},"source":"{}","entity":"{}"}}"#,
                s.sources[source].name, s.entities[entity]
            ),
            TraceEvent::Lost { part, source } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"lost","part":{part},"source":"{}"}}"#,
                s.sources[source].name
            ),
            TraceEvent::EnteredConveyor { part, conveyor } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"entered_conveyor","part":{part},"conveyor":"{}"}}"#,
                s.conveyors[conveyor].name
            ),
            TraceEvent::ReachedEnd { part, conveyor } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"reached_end","part":{part},"conveyor":"{}"}}"#,
                s.conveyors[conveyor].name
            ),
            TraceEvent::StartedProcess { part, server } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"started_process","part":{part},"server":"{}"}}"#,
                s.servers[server].name
            ),
            TraceEvent::FinishedProcess { part, server } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"finished_process","part":{part},"server":"{}"}}"#,
                s.servers[server].name
            ),
            TraceEvent::Failed { server, failure } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"failed","server":"{}","failure":"{}"}}"#,
                s.servers[server].name, s.servers[server].failures[failure].name
            ),
            TraceEvent::Repaired { server, failure } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"repaired","server":"{}","failure":"{}"}}"#,
                s.servers[server].name, s.servers[server].failures[failure].name
            ),
            TraceEvent::Collected { part, server } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"collected","part":{part},"server":"{}"}}"#,
                s.servers[server].name
            ),
            TraceEvent::Consumed { part, server } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"consumed","part":{part},"server":"{}"}}"#,
                s.servers[server].name
            ),
            TraceEvent::Assembled {
                part,
                server,
                entity,
            } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"assembled","part":{part},"server":"{}","entity":"{}"}}"#,
                s.servers[server].name, s.entities[entity]
            ),
            TraceEvent::ShiftEnded { server } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"shift_ended","server":"{}"}}"#,
                s.servers[server].name
            ),
            TraceEvent::ShiftStarted { server } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"shift_started","server":"{}"}}"#,
                s.servers[server].name
            ),
            TraceEvent::Exited { part, sink } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"exited","part":{part},"sink":"{}"}}"#,
                s.sinks[sink].name
            ),
        }
    }
}

impl<W: Write> Observer for EventLog<'_, W> {
    fn record(&mut self, time: SimTime, event: TraceEvent) {
        if self.error.is_none()
            && let Err(err) = self.write(time, event)
        {
            self.error = Some(err);
        }
    }
}
