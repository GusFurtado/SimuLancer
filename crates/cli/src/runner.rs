//! Running a model's experiment into a results folder. Shared by `run` and
//! the server.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use simulancer_engine::spec::Spec;
use simulancer_engine::stats::Summary;
use simulancer_engine::{Observer, SimTime, TraceEvent, parallel_replications, simulate};
use simulancer_model::{Layout, Model};

use crate::columnar;
use crate::measures::{self, Measure};
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
    let event_log = model.outputs.event_log;
    let log_events = event_log.any();
    if log_events && !(1..=replications).contains(&options.log_replication) {
        return Err(format!(
            "the event log replication {} is outside 1..={replications}",
            options.log_replication
        ));
    }
    fs::create_dir_all(out_dir).map_err(|e| format!("can't create {}: {e}", out_dir.display()))?;
    let events_path = out_dir.join("events.jsonl");
    let events_parquet = out_dir.join("events.parquet");

    let started = Instant::now();
    let results = parallel_replications(replications, options.threads, |r| {
        let mut parts = PartsInSystem::new(&spec);
        let result = if log_events && r == options.log_replication {
            let header = output::event_log_header(&model.name, &spec, r);
            let mut logs = EventLogs {
                jsonl: if event_log.jsonl {
                    let file = File::create(&events_path)?;
                    Some(output::EventLog::new(BufWriter::new(file), &header, &spec)?)
                } else {
                    None
                },
                parquet: if event_log.parquet {
                    let file = File::create(&events_parquet)?;
                    Some(columnar::ParquetEventLog::new(
                        BufWriter::new(file),
                        &header,
                        &spec,
                    )?)
                } else {
                    None
                },
            };
            let result = simulate(&spec, r, &mut Both(&mut logs, &mut parts));
            logs.finish()?;
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
    .map_err(|e| format!("can't write the event log in {}: {e}", out_dir.display()))?;
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

    let write = |name: &str, f: &dyn Fn(&Path) -> std::io::Result<()>| -> Result<PathBuf, String> {
        let path = out_dir.join(name);
        f(&path).map_err(|e| format!("can't write {}: {e}", path.display()))?;
        Ok(path)
    };
    let mut files = vec![
        write("report.html", &|p| {
            output::write_report(
                p,
                model,
                &spec,
                &measures,
                warmup.as_ref(),
                precision.as_ref(),
            )
        })?,
        write("summary.json", &|p| {
            output::write_summary(
                p,
                &model.name,
                &spec,
                &measures,
                warmup.as_ref(),
                precision.as_ref(),
            )
        })?,
        write("replications.csv", &|p| {
            output::write_replications_csv(p, &measures)
        })?,
        write("replications.parquet", &|p| {
            columnar::write_replications(BufWriter::new(File::create(p)?), &model.name, &measures)
        })?,
    ];
    if log_events && event_log.jsonl {
        files.push(events_path);
    }
    if log_events && event_log.parquet {
        files.push(events_parquet);
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
        elapsed,
        files,
        logged_replication: log_events.then_some(options.log_replication),
    })
}

/// The event logs a model asks for, fed the same events.
struct EventLogs<'a, J: Write, P: Write + Send> {
    jsonl: Option<output::EventLog<'a, J>>,
    parquet: Option<columnar::ParquetEventLog<P>>,
}

impl<J: Write, P: Write + Send> EventLogs<'_, J, P> {
    fn finish(self) -> std::io::Result<()> {
        if let Some(log) = self.jsonl {
            log.finish()?;
        }
        if let Some(log) = self.parquet {
            log.finish()?;
        }
        Ok(())
    }
}

impl<J: Write, P: Write + Send> Observer for EventLogs<'_, J, P> {
    #[inline]
    fn record(&mut self, time: SimTime, event: TraceEvent) {
        if let Some(log) = &mut self.jsonl {
            log.record(time, event);
        }
        if let Some(log) = &mut self.parquet {
            log.record(time, event);
        }
    }
}
