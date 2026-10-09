//! Running a model's experiment into a results folder. Shared by `run` and
//! the server.

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use outliers_sim_engine::spec::Spec;
use outliers_sim_engine::stats::Summary;
use outliers_sim_engine::{NoTrace, parallel_replications, simulate};
use outliers_sim_model::{EventLog, Layout, Model};

use crate::measures::{self, Measure};
use crate::output;

pub struct RunOptions<'a> {
    pub replications: Option<u32>,
    pub threads: usize,
    pub log_replication: u32,
    pub layout: Option<&'a Layout>,
    /// Incremented as each replication finishes.
    pub progress: Option<&'a AtomicU32>,
}

pub struct RunOutcome {
    pub spec: Spec,
    pub measures: Vec<(Measure, Summary)>,
    pub elapsed: Duration,
    pub threads: usize,
    /// Files written, in the order they were written.
    pub files: Vec<PathBuf>,
    /// The replication the event log is for, if one was written.
    pub logged_replication: Option<u32>,
}

pub fn execute(model: &Model, options: &RunOptions, out_dir: &Path) -> Result<RunOutcome, String> {
    let mut spec = model.spec.clone();
    if let Some(n) = options.replications {
        if n == 0 {
            return Err("replications must be at least 1".into());
        }
        spec.experiment.replications = n;
    }
    let replications = spec.experiment.replications;
    let log_events = model.outputs.event_log == EventLog::Jsonl;
    if log_events && !(1..=replications).contains(&options.log_replication) {
        return Err(format!(
            "the event log replication {} is outside 1..={replications}",
            options.log_replication
        ));
    }
    fs::create_dir_all(out_dir).map_err(|e| format!("can't create {}: {e}", out_dir.display()))?;
    let events_path = out_dir.join("events.jsonl");

    let started = Instant::now();
    let results = parallel_replications(replications, options.threads, |r| {
        let result = if log_events && r == options.log_replication {
            let file = File::create(&events_path)?;
            let mut log = output::EventLog::new(BufWriter::new(file), &model.name, &spec, r)?;
            let result = simulate(&spec, r, &mut log);
            log.finish()?;
            result
        } else {
            simulate(&spec, r, &mut NoTrace)
        };
        if let Some(progress) = options.progress {
            progress.fetch_add(1, Ordering::Relaxed);
        }
        Ok(result)
    })
    .into_iter()
    .collect::<std::io::Result<Vec<_>>>()
    .map_err(|e| format!("can't write {}: {e}", events_path.display()))?;
    let elapsed = started.elapsed();

    let measures: Vec<_> = measures::collect(&spec, &model.outputs.summary, &results)
        .into_iter()
        .map(|m| {
            let summary = Summary::of(&m.values, output::CONFIDENCE);
            (m, summary)
        })
        .collect();

    let write = |name: &str, f: &dyn Fn(&Path) -> std::io::Result<()>| -> Result<PathBuf, String> {
        let path = out_dir.join(name);
        f(&path).map_err(|e| format!("can't write {}: {e}", path.display()))?;
        Ok(path)
    };
    let mut files = vec![
        write("report.html", &|p| {
            output::write_report(p, model, &spec, &measures)
        })?,
        write("summary.json", &|p| {
            output::write_summary(p, &model.name, &spec, &measures)
        })?,
        write("replications.csv", &|p| {
            output::write_replications_csv(p, &measures)
        })?,
    ];
    if log_events {
        files.push(events_path);
    }
    if let Some(layout) = options.layout {
        files.push(write("layout.json", &|p| output::write_layout(p, layout))?);
    }
    Ok(RunOutcome {
        threads: options.threads.min(replications as usize),
        spec,
        measures,
        elapsed,
        files,
        logged_replication: log_events.then_some(options.log_replication),
    })
}
