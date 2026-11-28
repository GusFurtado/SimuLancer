//! Running a model's experiment into a results folder. Shared by `run` and
//! the server.

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use simulancer_engine::spec::Spec;
use simulancer_engine::stats::Summary;
use simulancer_engine::{parallel_replications, simulate};
use simulancer_model::{EventLog, Layout, Model};

use crate::expect::{self, Outcome};
use crate::measures::{self, Measure};
use crate::observed;
use crate::output;
use crate::precision::{self, KeyResult, PrecisionCheck};
use crate::warmup::{self, Both, PartsInSystem, WarmupCheck};

pub struct RunOptions<'a> {
    pub replications: Option<u32>,
    pub threads: usize,
    pub log_replication: u32,
    pub layout: Option<&'a Layout>,
    /// Incremented as each replication finishes.
    pub progress: Option<&'a AtomicU32>,
    /// The results to check for precision (a project's); none means the sinks'.
    pub key_results: &'a [KeyResult],
}

pub struct RunOutcome {
    pub spec: Spec,
    pub measures: Vec<(Measure, Summary)>,
    /// Parts in the system over time and where MSER would cut, to check the warm-up.
    pub warmup: Option<WarmupCheck>,
    /// Whether the key results' intervals are narrow enough.
    pub precision: Option<PrecisionCheck>,
    /// The model's expected results, and whether each was met.
    pub expectations: Vec<Outcome>,
    /// The real system's observed values against the model's.
    pub observations: Vec<observed::Outcome>,
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
    // Expected results that can't be read stop the run before it starts.
    let expected = expect::resolve(model).map_err(|problems| problems.join("\n"))?;
    let observed = observed::resolve(model).map_err(|problems| problems.join("\n"))?;
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
        let mut parts = PartsInSystem::new(&spec);
        let result = if log_events && r == options.log_replication {
            let file = File::create(&events_path)?;
            let mut log = output::EventLog::new(BufWriter::new(file), &model.name, &spec, r)?;
            let result = simulate(&spec, r, &mut Both(&mut log, &mut parts));
            log.finish()?;
            result
        } else {
            simulate(&spec, r, &mut parts)
        };
        if let Some(progress) = options.progress {
            progress.fetch_add(1, Ordering::Relaxed);
        }
        Ok((result, parts.finish()))
    })
    .into_iter()
    .collect::<std::io::Result<Vec<_>>>()
    .map_err(|e| format!("can't write {}: {e}", events_path.display()))?;
    let (results, series): (Vec<_>, Vec<_>) = results.into_iter().unzip();
    let warmup = warmup::check(&spec, &series);
    let elapsed = started.elapsed();

    let measures: Vec<_> = measures::collect(&spec, &model.outputs, &results)
        .into_iter()
        .map(|m| {
            let summary = Summary::of(&m.values, output::CONFIDENCE);
            (m, summary)
        })
        .collect();
    let precision = precision::check(&spec, &measures, options.key_results);
    let expectations = expect::check(&expected, &measures);
    let observations = observed::compare(&observed, &measures);

    let write = |name: &str, f: &dyn Fn(&Path) -> std::io::Result<()>| -> Result<PathBuf, String> {
        let path = out_dir.join(name);
        f(&path).map_err(|e| format!("can't write {}: {e}", path.display()))?;
        Ok(path)
    };
    let checks = output::Checks {
        warmup: warmup.as_ref(),
        precision: precision.as_ref(),
        expectations: &expectations,
        observations: &observations,
    };
    let mut files = vec![
        write("report.html", &|p| {
            output::write_report(p, model, &spec, &measures, &checks)
        })?,
        write("summary.json", &|p| {
            output::write_summary(p, &model.name, &spec, &measures, &checks)
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
        warmup,
        precision,
        expectations,
        observations,
        elapsed,
        files,
        logged_replication: log_events.then_some(options.log_replication),
    })
}
