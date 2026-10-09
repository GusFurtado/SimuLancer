mod measures;
mod output;
mod report;

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::Instant;

use clap::{Args, Parser, Subcommand};
use outliers_sim_engine::spec::{Distribution, Node, Spec};
use outliers_sim_engine::stats::Summary;
use outliers_sim_engine::{NoTrace, parallel_replications, simulate};
use outliers_sim_model::{EventLog, Layout, Model};

/// Open-source discrete event simulation. Models are YAML files.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check a model file for errors without running it.
    Validate {
        /// Path to the model YAML file.
        model: PathBuf,
        /// Layout file to check [default: <model>.layout.yaml next to the model, if present].
        #[arg(long)]
        layout: Option<PathBuf>,
    },
    /// Run a model's experiment, print a summary and write result files.
    Run(RunArgs),
}

#[derive(Args)]
struct RunArgs {
    /// Path to the model YAML file.
    model: PathBuf,
    /// Number of replications, overriding the model's `experiment.replications`.
    #[arg(short, long)]
    replications: Option<u32>,
    /// Threads to run replications on [default: all CPU cores].
    #[arg(short, long)]
    threads: Option<usize>,
    /// Where to write result files [default: results/<model name>].
    #[arg(short, long)]
    output_dir: Option<PathBuf>,
    /// Which replication to write the event log for (when the model asks for one).
    #[arg(long, default_value_t = 1)]
    log_replication: u32,
    /// Layout file for viewers [default: <model>.layout.yaml next to the model, if present].
    #[arg(long)]
    layout: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match &cli.command {
        Command::Validate { model, layout } => {
            let Some(loaded) = load(model) else {
                return ExitCode::FAILURE;
            };
            print!("{}", describe(model, &loaded));
            match find_layout(model, layout.as_deref()) {
                None => ExitCode::SUCCESS,
                Some(path) => match load_layout(&path, &loaded) {
                    Some(_) => {
                        println!("\nlayout ok: {}", path.display());
                        ExitCode::SUCCESS
                    }
                    None => ExitCode::FAILURE,
                },
            }
        }
        Command::Run(args) => match run(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::FAILURE
            }
        },
    }
}

fn run(args: &RunArgs) -> Result<(), String> {
    let Some(model) = load(&args.model) else {
        return Err("the model has errors (see above)".into());
    };
    let layout = match find_layout(&args.model, args.layout.as_deref()) {
        None => None,
        Some(path) => Some(
            load_layout(&path, &model)
                .ok_or_else(|| "the layout has errors (see above)".to_string())?,
        ),
    };
    let mut spec = model.spec.clone();
    if let Some(n) = args.replications {
        if n == 0 {
            return Err("--replications must be at least 1".into());
        }
        spec.experiment.replications = n;
    }
    let replications = spec.experiment.replications;
    let log_events = model.outputs.event_log == EventLog::Jsonl;
    if log_events && !(1..=replications).contains(&args.log_replication) {
        return Err(format!(
            "--log-replication {} is outside 1..={replications}",
            args.log_replication
        ));
    }
    let threads = args
        .threads
        .unwrap_or_else(|| thread::available_parallelism().map_or(1, |n| n.get()))
        .max(1);
    let out_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(|| Path::new("results").join(&model.name));
    fs::create_dir_all(&out_dir).map_err(|e| format!("can't create {}: {e}", out_dir.display()))?;
    let events_path = out_dir.join("events.jsonl");

    let started = Instant::now();
    let results = parallel_replications(replications, threads, |r| {
        if log_events && r == args.log_replication {
            let file = File::create(&events_path)?;
            let mut log = output::EventLog::new(BufWriter::new(file), &model.name, &spec, r)?;
            let result = simulate(&spec, r, &mut log);
            log.finish()?;
            Ok(result)
        } else {
            Ok(simulate(&spec, r, &mut NoTrace))
        }
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

    let e = &spec.experiment;
    println!(
        "{}: {} replication{} × {} after {} warm-up, seed {} ({:.2} s on {} thread{})\n",
        model.name,
        replications,
        if replications == 1 { "" } else { "s" },
        duration(e.run_length.as_secs_f64()),
        duration(e.warmup.as_secs_f64()),
        e.seed,
        elapsed.as_secs_f64(),
        threads.min(replications as usize),
        if threads.min(replications as usize) == 1 {
            ""
        } else {
            "s"
        },
    );
    print!("{}", report::table(&measures));
    if replications < 2 {
        println!("\nwarning: confidence intervals need at least 2 replications");
    }

    let summary_path = out_dir.join("summary.json");
    let csv_path = out_dir.join("replications.csv");
    output::write_summary(&summary_path, &model.name, &spec, &measures)
        .map_err(|e| format!("can't write {}: {e}", summary_path.display()))?;
    output::write_replications_csv(&csv_path, &measures)
        .map_err(|e| format!("can't write {}: {e}", csv_path.display()))?;
    let report_path = out_dir.join("report.html");
    output::write_report(&report_path, &model, &spec, &measures)
        .map_err(|e| format!("can't write {}: {e}", report_path.display()))?;
    println!("\nwrote {}", report_path.display());
    println!("wrote {}", summary_path.display());
    println!("wrote {}", csv_path.display());
    if log_events {
        println!(
            "wrote {} (replication {})",
            events_path.display(),
            args.log_replication
        );
    }
    if let Some(layout) = &layout {
        let layout_path = out_dir.join("layout.json");
        output::write_layout(&layout_path, layout)
            .map_err(|e| format!("can't write {}: {e}", layout_path.display()))?;
        println!("wrote {}", layout_path.display());
    }
    Ok(())
}

/// The layout to use: the one given, or `<model>.layout.yaml` next to the model.
fn find_layout(model: &Path, explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    let stem = model.file_stem()?.to_str()?;
    let sibling = model.with_file_name(format!("{stem}.layout.yaml"));
    sibling.exists().then_some(sibling)
}

/// Loads and checks a layout, printing warnings and errors to stderr.
fn load_layout(path: &Path, model: &Model) -> Option<Layout> {
    let display = path.display().to_string();
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("error: can't read {display}: {err}");
            return None;
        }
    };
    match outliers_sim_model::load_layout(&source, model) {
        Ok((layout, warnings)) => {
            for w in warnings {
                eprint!("{}", w.render_as("warning", &display, &source));
            }
            Some(layout)
        }
        Err(err) => {
            eprint!("{}", err.render(&display, &source));
            None
        }
    }
}

/// Loads a model, printing any errors to stderr.
fn load(path: &Path) -> Option<Model> {
    let display = path.display().to_string();
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("error: can't read {display}: {err}");
            return None;
        }
    };
    match outliers_sim_model::load_str(&source) {
        Ok(model) => Some(model),
        Err(err) => {
            eprint!("{}", err.render(&display, &source));
            None
        }
    }
}

fn describe(path: &Path, model: &Model) -> String {
    let spec = &model.spec;
    let mut out = format!("ok: {} ({})\n", path.display(), model.name);

    out += "\nsources\n";
    for s in &spec.sources {
        out += &format!(
            "  {:<12} creates {}, every {}\n",
            s.name,
            spec.entities[s.entity],
            dist(&s.interarrival)
        );
    }
    out += "\nservers\n";
    for s in &spec.servers {
        out += &format!(
            "  {:<12} capacity {}, process {}\n",
            s.name,
            s.capacity,
            dist(&s.process_time)
        );
        for f in &s.failures {
            out += &format!(
                "  {:<12}   fails ({}) after {}, down for {}\n",
                "",
                f.name,
                dist(&f.uptime),
                dist(&f.downtime)
            );
        }
    }
    out += "\nconveyors\n";
    for c in &spec.conveyors {
        let route = format!("{} -> {}", node_name(spec, c.from), node_name(spec, c.to));
        out += &format!(
            "  {:<12} {:<20} {} m at {} m/s, holds {} parts, transit {}\n",
            c.name,
            route,
            c.length,
            c.speed,
            c.capacity,
            duration(c.transit_time())
        );
    }
    let e = &spec.experiment;
    out += &format!(
        "\nexperiment: {} replications, warm-up {}, run length {}, seed {}\n",
        e.replications,
        duration(e.warmup.as_secs_f64()),
        duration(e.run_length.as_secs_f64()),
        e.seed
    );
    out
}

fn node_name(spec: &Spec, node: Node) -> &str {
    match node {
        Node::Source(i) => &spec.sources[i].name,
        Node::Server(i) => &spec.servers[i].name,
        Node::Sink(i) => &spec.sinks[i].name,
    }
}

fn dist(d: &Distribution) -> String {
    let body = match *d {
        Distribution::Constant(v) => return duration(v),
        Distribution::Exponential { mean } => format!("exponential({})", duration(mean)),
        Distribution::Uniform { min, max } => {
            format!("uniform({}, {})", duration(min), duration(max))
        }
        Distribution::Triangular { min, mode, max } => format!(
            "triangular({}, {}, {})",
            duration(min),
            duration(mode),
            duration(max)
        ),
        Distribution::Lognormal { mean, sd } => {
            format!("lognormal(mean {}, sd {})", duration(mean), duration(sd))
        }
        Distribution::Weibull { shape, scale } => {
            format!("weibull(shape {shape}, scale {})", duration(scale))
        }
        Distribution::Gamma { shape, scale } => {
            format!("gamma(shape {shape}, scale {})", duration(scale))
        }
    };
    format!("{body}, mean {}", duration(d.mean()))
}

/// Seconds in the largest unit that keeps the number readable.
fn duration(seconds: f64) -> String {
    let (value, unit) = if seconds >= 86400.0 * 2.0 {
        (seconds / 86400.0, "d")
    } else if seconds >= 3600.0 {
        (seconds / 3600.0, "h")
    } else if seconds >= 60.0 {
        (seconds / 60.0, "min")
    } else {
        (seconds, "s")
    };
    let rounded = (value * 100.0).round() / 100.0;
    format!("{rounded} {unit}")
}
