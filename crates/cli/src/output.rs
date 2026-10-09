//! Result files: summary.json, replications.csv and the events.jsonl event log.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use outliers_sim_engine::spec::{Node, Spec};
use outliers_sim_engine::stats::Summary;
use outliers_sim_engine::{Observer, SimTime, TraceEvent};
use serde::Serialize;
use serde_json::json;

use crate::measures::Measure;

pub const CONFIDENCE: f64 = 0.95;

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
                measure: m.name,
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
            "sources": spec.sources.iter().map(|s| &s.name).collect::<Vec<_>>(),
            "servers": spec.servers.iter().map(|s| json!({
                "name": s.name,
                "capacity": s.capacity,
                "failures": s.failures.iter().map(|f| &f.name).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "conveyors": spec.conveyors.iter().map(|c| json!({
                "name": c.name,
                "from": node(c.from),
                "to": node(c.to),
                "length": c.length,
                "speed": c.speed,
                "spacing": c.spacing,
                "capacity": c.capacity,
            })).collect::<Vec<_>>(),
            "sinks": spec.sinks.iter().map(|s| &s.name).collect::<Vec<_>>(),
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
            TraceEvent::Created { part, source } => writeln!(
                self.out,
                r#"{{"t":{t},"type":"created","part":{part},"source":"{}"}}"#,
                s.sources[source].name
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
