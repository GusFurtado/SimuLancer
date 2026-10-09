use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use outliers_sim_engine::spec::{Distribution, Node, Spec};
use outliers_sim_model::Model;

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
    },
    /// Run a model's experiment and print a summary.
    Run {
        /// Path to the model YAML file.
        model: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match &cli.command {
        Command::Validate { model } => match load(model) {
            Some(loaded) => {
                print!("{}", describe(model, &loaded));
                ExitCode::SUCCESS
            }
            None => ExitCode::FAILURE,
        },
        Command::Run { model } => {
            if load(model).is_none() {
                return ExitCode::FAILURE;
            }
            eprintln!("`run` is not implemented yet");
            ExitCode::from(2)
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
