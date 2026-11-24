mod compare;
mod decisions;
mod files;
mod markdown;
mod measures;
mod output;
mod plain;
mod projects;
mod report;
mod runner;
mod scenarios;
mod server;
mod warmup;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::Instant;

use clap::{Args, Parser, Subcommand};
use simulancer_engine::spec::{
    Arrivals, BinaryOp, Changeover, Count, Distribution, Expr, InputRule, Node, OrderQuantity,
    PerType, QueueOrder, Routing, ServerKind, Spec,
};
use simulancer_engine::{NoTrace, parallel_replications, simulate};
use simulancer_model::Model;

/// Open-source discrete event simulation. Models are YAML files.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check a model or scenario file for errors without running it.
    Validate {
        /// Path to the model or scenario YAML file.
        model: PathBuf,
        /// Layout file to check [default: <model>.layout.yaml next to the model, if present].
        #[arg(long)]
        layout: Option<PathBuf>,
    },
    /// Run a model's experiment, print a summary and write result files.
    Run(RunArgs),
    /// Compare a scenario against a base model: paired replications with
    /// common random numbers, and a confidence interval on each difference.
    Compare(CompareArgs),
    /// Serve the web application and its HTTP API.
    Serve(server::ServeArgs),
}

#[derive(Args)]
struct CompareArgs {
    /// The base model.
    base: PathBuf,
    /// The scenario to compare against the base. Its experiment settings
    /// (seed, replications, warm-up, run length) must match the base's.
    scenario: PathBuf,
    /// Number of replications for both, overriding the models' setting.
    #[arg(short, long)]
    replications: Option<u32>,
    /// Threads to run replications on [default: all CPU cores].
    #[arg(short, long)]
    threads: Option<usize>,
    /// Where to write comparison.json [default: results/<base>-vs-<scenario>].
    #[arg(short, long)]
    output_dir: Option<PathBuf>,
}

#[derive(Args)]
struct RunArgs {
    /// Path to the model or scenario YAML file.
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
    /// Layout file for the animation [default: <model>.layout.yaml next to the model, if present].
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
            match files::load_layout(model, layout.as_deref(), &loaded) {
                Ok(None) => ExitCode::SUCCESS,
                Ok(Some(l)) => {
                    l.warnings.iter().for_each(|w| eprint!("{w}"));
                    let owner = files::layout_owner(model);
                    let path = files::find_layout(&owner, layout.as_deref()).expect("just loaded");
                    println!("\nlayout ok: {}", path.display());
                    ExitCode::SUCCESS
                }
                Err(failure) => {
                    eprint!("{}", failure.render());
                    ExitCode::FAILURE
                }
            }
        }
        Command::Run(args) => report_errors(run(args)),
        Command::Compare(args) => report_errors(compare_scenarios(args)),
        Command::Serve(args) => report_errors(server::serve(args)),
    }
}

fn report_errors(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn thread_count(requested: Option<usize>) -> usize {
    requested
        .unwrap_or_else(|| thread::available_parallelism().map_or(1, |n| n.get()))
        .max(1)
}

/// Loads a model or a scenario, printing any errors to stderr.
fn load(path: &Path) -> Option<Model> {
    match files::load_model(path) {
        Ok(model) => Some(model),
        Err(failure) => {
            eprint!("{}", failure.render());
            None
        }
    }
}

fn compare_scenarios(args: &CompareArgs) -> Result<(), String> {
    let base = load(&args.base).ok_or("the base model has errors (see above)")?;
    let scenario = load(&args.scenario).ok_or("the scenario has errors (see above)")?;
    let (mut base_spec, mut scenario_spec) = (base.spec.clone(), scenario.spec.clone());
    if let Some(n) = args.replications {
        if n == 0 {
            return Err("--replications must be at least 1".into());
        }
        base_spec.experiment.replications = n;
        scenario_spec.experiment.replications = n;
    }
    let e = &base_spec.experiment;
    if *e != scenario_spec.experiment {
        let describe = |x: &simulancer_engine::spec::Experiment| {
            format!(
                "seed {}, {} replications, warm-up {}, run length {}",
                x.seed,
                x.replications,
                duration(x.warmup.as_secs_f64()),
                duration(x.run_length.as_secs_f64())
            )
        };
        return Err(format!(
            "a paired comparison needs the same experiment settings in both models\n  base:     {}\n  scenario: {}",
            describe(e),
            describe(&scenario_spec.experiment)
        ));
    }
    let threads = thread_count(args.threads);
    let out_dir = args.output_dir.clone().unwrap_or_else(|| {
        Path::new("results").join(format!("{}-vs-{}", base.name, scenario.name))
    });
    fs::create_dir_all(&out_dir).map_err(|e| format!("can't create {}: {e}", out_dir.display()))?;

    let started = Instant::now();
    let n = e.replications;
    let base_results = parallel_replications(n, threads, |r| simulate(&base_spec, r, &mut NoTrace));
    let scenario_results =
        parallel_replications(n, threads, |r| simulate(&scenario_spec, r, &mut NoTrace));
    let elapsed = started.elapsed();

    let comparisons = compare::compare(
        measures::collect(&base_spec, &base.outputs, &base_results),
        measures::collect(&scenario_spec, &scenario.outputs, &scenario_results),
    );
    println!(
        "{} (base) vs {} (scenario): {} paired replication{} × {} after {} warm-up, seed {} ({:.2} s)\n",
        base.name,
        scenario.name,
        n,
        if n == 1 { "" } else { "s" },
        duration(e.run_length.as_secs_f64()),
        duration(e.warmup.as_secs_f64()),
        e.seed,
        elapsed.as_secs_f64(),
    );
    print!("{}", report::comparison_table(&comparisons));
    if let Some(gain) = compare::median_precision_gain(&comparisons) {
        println!(
            "\nCommon random numbers made the confidence intervals {gain:.1}× narrower (median) than independent runs would."
        );
    }
    if n < 2 {
        println!("\nwarning: confidence intervals need at least 2 replications");
    }

    let path = out_dir.join("comparison.json");
    output::write_comparison(&path, &base.name, &scenario.name, &base_spec, &comparisons)
        .map_err(|e| format!("can't write {}: {e}", path.display()))?;
    println!("\nwrote {}", path.display());
    Ok(())
}

fn run(args: &RunArgs) -> Result<(), String> {
    let model = load(&args.model).ok_or("the model has errors (see above)")?;
    let layout = match files::load_layout(&args.model, args.layout.as_deref(), &model) {
        Ok(layout) => layout,
        Err(failure) => {
            eprint!("{}", failure.render());
            return Err("the layout has errors (see above)".into());
        }
    };
    if let Some(l) = &layout {
        l.warnings.iter().for_each(|w| eprint!("{w}"));
    }
    let out_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(|| Path::new("results").join(&model.name));
    let outcome = runner::execute(
        &model,
        &runner::RunOptions {
            replications: args.replications,
            threads: thread_count(args.threads),
            log_replication: args.log_replication,
            layout: layout.as_ref().map(|l| &l.layout),
            progress: None,
        },
        &out_dir,
    )?;

    let e = &outcome.spec.experiment;
    let n = e.replications;
    println!(
        "{}: {} replication{} × {} after {} warm-up, seed {} ({:.2} s on {} thread{})\n",
        model.name,
        n,
        if n == 1 { "" } else { "s" },
        duration(e.run_length.as_secs_f64()),
        duration(e.warmup.as_secs_f64()),
        e.seed,
        outcome.elapsed.as_secs_f64(),
        outcome.threads,
        if outcome.threads == 1 { "" } else { "s" },
    );
    print!("{}", report::table(&outcome.measures));
    if let Some(w) = &outcome.warmup {
        let warmup = e.warmup.as_secs_f64();
        match (w.settles, w.suggested) {
            (Some(false), _) => println!(
                "\nwarning: the parts in the system don't settle within the run: it may be too short, or the system keeps filling up (see the report's warm-up check)"
            ),
            (Some(true), Some(suggested)) if suggested > warmup => println!(
                "\nwarning: the warm-up ({}) may be too short: the parts in the system settle after about {} (see the report's warm-up check)",
                duration(warmup),
                duration(suggested),
            ),
            _ => {}
        }
    }
    if n < 2 {
        println!("\nwarning: confidence intervals need at least 2 replications");
    }
    println!();
    for file in &outcome.files {
        match (
            file.file_name().and_then(|f| f.to_str()),
            outcome.logged_replication,
        ) {
            (Some("events.jsonl"), Some(r)) => {
                println!("wrote {} (replication {r})", file.display())
            }
            _ => println!("wrote {}", file.display()),
        }
    }
    Ok(())
}

fn describe(path: &Path, model: &Model) -> String {
    let spec = &model.spec;
    let mut out = format!("ok: {} ({})\n", path.display(), model.name);

    if !spec.attributes.is_empty() {
        out += "\nattributes\n";
        for a in &spec.attributes {
            let default = if a.time {
                duration(a.default)
            } else {
                a.default.to_string()
            };
            out += &format!("  {:<12} starts at {default}\n", a.name);
        }
    }

    out += "\nsources\n";
    for s in &spec.sources {
        let creates = match s.creates.as_slice() {
            [(e, _)] => spec.entities[*e].clone(),
            mix => mix
                .iter()
                .map(|(e, share)| format!("{} {:.1}%", spec.entities[*e], share * 100.0))
                .collect::<Vec<_>>()
                .join(", "),
        };
        let when = match &s.arrivals {
            Arrivals::Interarrival(d) => format!("every {}", dist(d)),
            Arrivals::Restock(r) => format!(
                "to keep {} stocked: when stock is at or below {}, orders {}, arriving after {}{}",
                spec.conveyors[r.queue].name,
                r.reorder_point,
                match r.quantity {
                    OrderQuantity::Fixed(q) => format!("{q}"),
                    OrderQuantity::UpTo(level) => format!("up to {level}"),
                },
                dist(&r.lead_time),
                r.review_every.map_or(String::new(), |t| format!(
                    ", checked every {}",
                    duration(t.as_secs_f64())
                ))
            ),
            Arrivals::Profile(periods) => {
                let clock = |t: f64| {
                    format!(
                        "{:02}:{:02}",
                        (t / 3600.0) as u32,
                        (t % 3600.0 / 60.0) as u32
                    )
                };
                let per_day: f64 = periods.iter().map(|(a, b, r)| (b - a) * r).sum();
                format!(
                    "{} a day: {}",
                    (per_day * 10.0).round() / 10.0,
                    periods
                        .iter()
                        .map(|(a, b, r)| format!(
                            "{}-{} {}/h",
                            clock(*a),
                            clock(*b),
                            (r * 3600.0 * 10.0).round() / 10.0
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        };
        let group = s
            .group
            .as_ref()
            .map_or(String::new(), |g| format!(", {} at a time", count_text(g)));
        out += &format!("  {:<12} creates {creates}, {when}{group}\n", s.name);
        if let Some(text) = routing(spec, &s.outputs, &s.routing) {
            out += &format!("  {:<12}   {text}\n", "");
        }
        if let Some(i) = s.schedule {
            out += &format!(
                "  {:<12}   arrives on schedule {}\n",
                "", spec.schedules[i].name
            );
        }
        out += &sets_text(spec, &s.sets, "on each new part");
    }
    out += "\nservers\n";
    for s in &spec.servers {
        out += &format!(
            "  {:<12} capacity {}, process {}\n",
            s.name,
            s.capacity,
            per_type(spec, &s.process_time)
        );
        if !s.needs.is_empty() {
            let needs: Vec<String> = s
                .needs
                .iter()
                .map(|&(r, units)| format!("{units} {}", spec.resources[r].name))
                .collect();
            out += &format!(
                "  {:<12}   needs {} while processing\n",
                "",
                needs.join(", ")
            );
        }
        if !s.takes.is_empty() {
            let takes: Vec<String> = s
                .takes
                .iter()
                .map(|&(r, units)| format!("{units} {}", spec.resources[r].name))
                .collect();
            out += &format!(
                "  {:<12}   each part takes {} and keeps it\n",
                "",
                takes.join(", ")
            );
        }
        if !s.gives_back.is_empty() {
            let names: Vec<&str> = s
                .gives_back
                .iter()
                .map(|&r| spec.resources[r].name.as_str())
                .collect();
            out += &format!(
                "  {:<12}   parts give back {} when done here\n",
                "",
                names.join(", ")
            );
        }
        if let Some(i) = s.schedule {
            out += &format!(
                "  {:<12}   works on schedule {}\n",
                "", spec.schedules[i].name
            );
        }
        match &s.changeover {
            None => {}
            Some(Changeover::Any(d)) => {
                out += &format!(
                    "  {:<12}   changes over between part types in {}\n",
                    "",
                    dist(d)
                );
            }
            Some(Changeover::Pairs(pairs)) => {
                for (from, row) in pairs.iter().enumerate() {
                    for (to, d) in row.iter().enumerate() {
                        if let Some(d) = d {
                            out += &format!(
                                "  {:<12}   changes over from {} to {} in {}\n",
                                "",
                                spec.entities[from],
                                spec.entities[to],
                                dist(d)
                            );
                        }
                    }
                }
            }
        }
        if let Some((e, count)) = &s.splits_into {
            out += &format!(
                "  {:<12}   splits each part into {} {}\n",
                "",
                count_text(count),
                spec.entities[*e]
            );
        }
        match &s.kind {
            ServerKind::Single => {}
            ServerKind::Batch(n) => {
                out += &format!("  {:<12}   processes batches of {n}\n", "");
            }
            ServerKind::Assembly {
                recipe,
                makes,
                clock_of,
            } => {
                let kit: Vec<String> = recipe
                    .iter()
                    .map(|&(e, n)| format!("{n} {}", spec.entities[e]))
                    .collect();
                let clock = clock_of.map_or(String::new(), |e| {
                    format!(", timed from its {}", spec.entities[e])
                });
                out += &format!(
                    "  {:<12}   assembles {} into 1 {}{clock}\n",
                    "",
                    kit.join(" + "),
                    spec.entities[*makes]
                );
            }
        }
        out += &sets_text(spec, &s.sets, "when a part is done");
        for f in &s.failures {
            out += &format!(
                "  {:<12}   fails ({}) after {}, down for {}\n",
                "",
                f.name,
                dist(&f.uptime),
                dist(&f.downtime)
            );
        }
        if s.inputs.len() > 1 {
            let names = conveyor_list(spec, &s.inputs);
            let rule = match s.input_rule {
                InputRule::Single => "single".to_string(),
                InputRule::FirstCome => "first come".to_string(),
                InputRule::Priority => "priority, in this order".to_string(),
                InputRule::RoundRobin => "round robin".to_string(),
                InputRule::Lowest(a) => format!("lowest {} first", spec.attributes[a].name),
                InputRule::Highest(a) => format!("highest {} first", spec.attributes[a].name),
            };
            out += &format!("  {:<12}   takes from {names} ({rule})\n", "");
        }
        if let Some(text) = routing(spec, &s.outputs, &s.routing) {
            out += &format!("  {:<12}   {text}\n", "");
        }
    }
    if !spec.resources.is_empty() {
        out += "\nresources\n";
        for r in &spec.resources {
            let schedule = r
                .schedule
                .map(|i| format!(", on schedule {}", spec.schedules[i].name))
                .unwrap_or_default();
            out += &format!("  {:<12} capacity {}{schedule}\n", r.name, r.capacity);
            if !r.order.is_empty() {
                let servers: Vec<&str> = r
                    .order
                    .iter()
                    .map(|&s| spec.servers[s].name.as_str())
                    .collect();
                out += &format!("  {:<12}   serves {} first\n", "", servers.join(", then "));
            }
        }
    }
    if !spec.schedules.is_empty() {
        out += "\nschedules (weekly, from Monday 00:00)\n";
        for s in &spec.schedules {
            let hours: f64 = s
                .periods
                .iter()
                .map(|(a, b)| (*b - *a).as_secs_f64() / 3600.0)
                .sum();
            out += &format!(
                "  {:<12} {hours} h a week in {} periods\n",
                s.name,
                s.periods.len()
            );
        }
    }
    let queues: Vec<_> = spec.conveyors.iter().filter(|c| c.is_queue()).collect();
    if !queues.is_empty() {
        out += "\nqueues\n";
        for c in queues {
            let route = format!("{} -> {}", node_name(spec, c.from), node_name(spec, c.to));
            let holds = if c.capacity == u32::MAX {
                "no limit".to_string()
            } else {
                format!("holds {} {}", c.capacity, model.words.many)
            };
            let target = c
                .target_wait
                .map_or(String::new(), |t| format!(", target wait {}", duration(t)));
            let order = match &c.order {
                QueueOrder::FirstIn => String::new(),
                QueueOrder::LastIn => ", last in first out".to_string(),
                QueueOrder::Lowest(a) => format!(", lowest {} first", spec.attributes[*a].name),
                QueueOrder::Highest(a) => format!(", highest {} first", spec.attributes[*a].name),
                QueueOrder::ByType(rank) => {
                    let mut types: Vec<usize> = (0..spec.entities.len()).collect();
                    types.sort_by_key(|&e| rank[e]);
                    let names: Vec<&str> =
                        types.iter().map(|&e| spec.entities[e].as_str()).collect();
                    format!(", by type: {}", names.join(", then "))
                }
            };
            out += &format!("  {:<12} {:<20} {holds}{target}{order}\n", c.name, route);
            if let Some(patience) = &c.patience {
                out += &format!(
                    "  {:<12}   gives up after waiting {}\n",
                    "",
                    per_type(spec, patience)
                );
            }
        }
    }
    let transports: Vec<_> = spec
        .conveyors
        .iter()
        .filter(|c| c.transport_by().is_some())
        .collect();
    if !transports.is_empty() {
        out += "\ntransports\n";
        for c in transports {
            let route = format!("{} -> {}", node_name(spec, c.from), node_name(spec, c.to));
            let (fleet, distance) = c.transport_by().expect("a transport");
            out += &format!(
                "  {:<12} {:<20} by {}, {distance} m\n",
                c.name, route, spec.vehicles[fleet].name
            );
        }
    }
    if !spec.lines.is_empty() {
        out += "\npaced lines\n";
        for l in &spec.lines {
            let stations: Vec<&str> = l
                .stations
                .iter()
                .map(|&s| spec.servers[s].name.as_str())
                .collect();
            out += &format!(
                "  {:<12} {} move on together every {}\n",
                l.name,
                stations.join(", "),
                duration(l.takt.as_secs_f64())
            );
        }
    }
    if !spec.vehicles.is_empty() {
        out += "\nvehicles\n";
        for f in &spec.vehicles {
            out += &format!(
                "  {:<12} {} at {} m/s, load {}, unload {}, start at {}\n",
                f.name,
                f.count,
                f.speed,
                dist(&f.load_time),
                dist(&f.unload_time),
                node_name(spec, f.home)
            );
        }
    }
    if spec.conveyors.iter().any(|c| c.is_belt()) {
        out += "\nconveyors\n";
    }
    for c in spec.conveyors.iter().filter(|c| c.is_belt()) {
        let route = format!("{} -> {}", node_name(spec, c.from), node_name(spec, c.to));
        out += &format!(
            "  {:<12} {:<20} {} m at {} m/s, holds {} {}, transit {}\n",
            c.name,
            route,
            c.length,
            c.speed,
            c.capacity,
            model.words.many,
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

/// `12`, or `10 to 14`.
fn count_text(count: &Count) -> String {
    match count {
        Count::Fixed(n) => n.to_string(),
        Count::Uniform { min, max } => format!("{min} to {max}"),
    }
}

fn conveyor_list(spec: &Spec, conveyors: &[usize]) -> String {
    conveyors
        .iter()
        .map(|&c| spec.conveyors[c].name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// How parts leave, for objects with several outgoing conveyors.
fn routing(spec: &Spec, outputs: &[usize], routing: &Routing) -> Option<String> {
    let names = conveyor_list(spec, outputs);
    match routing {
        Routing::Single => None,
        Routing::Probability(weights) => Some(format!(
            "sends to {}",
            outputs
                .iter()
                .zip(weights)
                .map(|(&c, w)| format!("{} {:.1}%", spec.conveyors[c].name, w * 100.0))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Routing::RoundRobin => Some(format!("sends to {names} (round robin)")),
        Routing::FirstAvailable => {
            Some(format!("sends to {names} (first available, in this order)"))
        }
        Routing::ShortestQueue => Some(format!("sends to {names} (shortest queue)")),
        Routing::ByType(routes) => Some(format!(
            "sends by part type: {}",
            routes
                .iter()
                .enumerate()
                .filter_map(|(e, c)| Some(format!(
                    "{} -> {}",
                    spec.entities[e],
                    spec.conveyors[(*c)?].name
                )))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Routing::ByCondition(routes) => Some(format!(
            "sends {}",
            routes
                .iter()
                .map(|(when, c)| match when {
                    Some(w) => format!("to {} when {}", spec.conveyors[*c].name, formula(spec, w)),
                    None => format!("otherwise to {}", spec.conveyors[*c].name),
                })
                .collect::<Vec<_>>()
                .join("; ")
        )),
    }
}

/// A process time, or one per part type.
fn per_type(spec: &Spec, value: &PerType<Distribution>) -> String {
    match value {
        PerType::Same(d) => dist(d),
        PerType::ByType(times) => times
            .iter()
            .enumerate()
            .filter_map(|(e, d)| Some(format!("{} {}", spec.entities[e], dist(d.as_ref()?))))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

fn node_name(spec: &Spec, node: Node) -> &str {
    match node {
        Node::Source(i) => &spec.sources[i].name,
        Node::Server(i) => &spec.servers[i].name,
        Node::Sink(i) => &spec.sinks[i].name,
    }
}

/// One line per attribute a source or server sets.
fn sets_text(spec: &Spec, sets: &[(usize, Expr)], when: &str) -> String {
    sets.iter()
        .map(|(a, f)| {
            format!(
                "  {:<12}   sets {} = {} {when}\n",
                "",
                spec.attributes[*a].name,
                formula(spec, f)
            )
        })
        .collect()
}

/// A formula as the engine understood it, fully parenthesized.
fn formula(spec: &Spec, f: &Expr) -> String {
    match f {
        Expr::Value(v) => v.to_string(),
        Expr::Seconds(s) => duration(*s),
        Expr::Attribute(a) => spec.attributes[*a].name.clone(),
        Expr::Now => "now".into(),
        Expr::Draw { distribution, time } => {
            let value = |v: f64| if *time { duration(v) } else { v.to_string() };
            dist_body(distribution, value)
        }
        Expr::IsType(e) => format!("type == {}", spec.entities[*e]),
        Expr::Neg(x) => format!("-{}", formula(spec, x)),
        Expr::Not(x) => format!("not {}", formula(spec, x)),
        Expr::Binary(op, a, b) => {
            let symbol = match op {
                BinaryOp::Add => "+",
                BinaryOp::Sub => "-",
                BinaryOp::Mul => "*",
                BinaryOp::Div => "/",
                BinaryOp::Lt => "<",
                BinaryOp::Le => "<=",
                BinaryOp::Gt => ">",
                BinaryOp::Ge => ">=",
                BinaryOp::Eq => "==",
                BinaryOp::Ne => "!=",
                BinaryOp::And => "and",
                BinaryOp::Or => "or",
            };
            let side = |x: &Expr| match x {
                Expr::Binary(..) => format!("({})", formula(spec, x)),
                _ => formula(spec, x),
            };
            format!("{} {symbol} {}", side(a), side(b))
        }
    }
}

fn dist(d: &Distribution) -> String {
    if let Distribution::Constant(v) = *d {
        return duration(v);
    }
    format!("{}, mean {}", dist_body(d, duration), duration(d.mean()))
}

/// A distribution with its parameters written by `value` (shapes are plain numbers).
fn dist_body(d: &Distribution, value: impl Fn(f64) -> String) -> String {
    let duration = value;
    match *d {
        Distribution::Constant(v) => duration(v),
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
    }
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
