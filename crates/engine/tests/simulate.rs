//! Engine behavior on small models with known answers. Models are written
//! in YAML (through the model crate) because that's how people write them.

use outliers_sim_engine::{NoTrace, ReplicationResult, SimTime, Spec, TraceEvent, simulate};

const CONVEYOR_LINE: &str = include_str!("../../../examples/conveyor-line.yaml");

/// Deterministic line: a part every 60 s, 30 s of processing, 2 s on each
/// conveyor. Every number is exact, so results can be checked exactly.
const DETERMINISTIC: &str = r#"
format: outliers-sim/0.1
name: deterministic
entities:
  part: {}
sources:
  arrivals: { creates: part, interarrival: "60 s" }
servers:
  machine: { process_time: "30 s" }
conveyors:
  c_in:  { from: arrivals, to: machine, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
  c_out: { from: machine, to: done, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
sinks:
  done: {}
experiment:
  replications: 1
  run_length: "10 h"
  seed: 1
"#;

fn spec(yaml: &str) -> Spec {
    match outliers_sim_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

fn with(yaml: &str, replacements: &[(&str, &str)]) -> Spec {
    let mut yaml = yaml.to_string();
    for (from, to) in replacements {
        assert!(yaml.contains(from), "test bug: `{from}` not in model");
        yaml = yaml.replacen(from, to, 1);
    }
    spec(&yaml)
}

fn run(spec: &Spec) -> ReplicationResult {
    simulate(spec, 1, &mut NoTrace)
}

fn assert_conserved(r: &ReplicationResult) {
    let t = &r.totals;
    assert_eq!(
        t.created,
        t.exited + t.lost + t.in_system,
        "parts created must exit, be lost or still be in the system: {t:?}"
    );
}

fn assert_near(actual: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: got {actual}, expected {expected} ± {tolerance}"
    );
}

#[test]
fn deterministic_line_exact_results() {
    let r = run(&spec(DETERMINISTIC));
    // Arrivals at 0, 60, ..., 35940: 600 parts, each exits 34 s after creation.
    assert_eq!(r.sources[0].created, 600);
    assert_eq!(r.sinks[0].exited, 600);
    let tis = &r.sinks[0].time_in_system;
    assert_eq!((tis.min, tis.max), (34.0, 34.0));
    assert_eq!(r.servers[0].busy, 0.5);
    assert_eq!(r.servers[0].blocked, 0.0);
    assert_eq!(r.servers[0].failed, 0.0);
    assert_eq!(r.sources[0].blocked, 0.0);
    assert_eq!(r.conveyors[0].max_parts, 1);
    assert_conserved(&r);
}

#[test]
fn deterministic_trace_of_first_part() {
    let mut trace = Vec::new();
    simulate(&spec(DETERMINISTIC), 1, &mut trace);
    let first: Vec<(f64, TraceEvent)> = trace
        .iter()
        .filter(|(_, e)| match e {
            TraceEvent::Created { part, .. }
            | TraceEvent::EnteredConveyor { part, .. }
            | TraceEvent::ReachedEnd { part, .. }
            | TraceEvent::StartedProcess { part, .. }
            | TraceEvent::FinishedProcess { part, .. }
            | TraceEvent::Exited { part, .. } => *part == 0,
            _ => false,
        })
        .map(|(t, e)| (t.as_secs_f64(), *e))
        .collect();
    use TraceEvent::*;
    assert_eq!(
        first,
        [
            (0.0, Created { part: 0, source: 0 }),
            (
                0.0,
                EnteredConveyor {
                    part: 0,
                    conveyor: 0
                }
            ),
            (
                2.0,
                ReachedEnd {
                    part: 0,
                    conveyor: 0
                }
            ),
            (2.0, StartedProcess { part: 0, server: 0 }),
            (32.0, FinishedProcess { part: 0, server: 0 }),
            (
                32.0,
                EnteredConveyor {
                    part: 0,
                    conveyor: 1
                }
            ),
            (
                34.0,
                ReachedEnd {
                    part: 0,
                    conveyor: 1
                }
            ),
            (34.0, Exited { part: 0, sink: 0 }),
        ]
    );
}

#[test]
fn warmup_excludes_early_statistics() {
    let s = with(
        DETERMINISTIC,
        &[(
            "run_length: \"10 h\"",
            "warmup: \"1 h\"\n  run_length: \"1 h\"",
        )],
    );
    let r = run(&s);
    // Exits at 60k + 34 s inside [3600, 7200): k = 60..=119.
    assert_eq!(r.sinks[0].exited, 60);
    assert_eq!(r.servers[0].busy, 0.5);
    assert_eq!(r.measured, 3600.0);
}

#[test]
fn slow_server_fills_its_input_and_stops_the_source() {
    let s = with(
        DETERMINISTIC,
        &[("process_time: \"30 s\"", "process_time: \"90 s\"")],
    );
    let r = run(&s);
    // Busy from t = 2 s to the end, one part every 90 s.
    assert_near(r.servers[0].busy, 1.0, 1e-3, "server busy");
    assert_eq!(r.servers[0].processed, (36000 - 2) / 90);
    assert_eq!(r.conveyors[0].max_parts, 4); // 2 m / 0.5 m
    assert!(
        r.conveyors[0].full > 0.9,
        "input conveyor full {}",
        r.conveyors[0].full
    );
    // The source can only release a part as fast as the server takes one.
    assert!(
        r.sources[0].blocked > 0.3,
        "source blocked {}",
        r.sources[0].blocked
    );
    assert_conserved(&r);
}

#[test]
fn drop_loses_parts_instead_of_waiting() {
    let s = with(
        DETERMINISTIC,
        &[
            ("process_time: \"30 s\"", "process_time: \"90 s\""),
            (
                "interarrival: \"60 s\" }",
                "interarrival: \"60 s\", when_blocked: drop }",
            ),
        ],
    );
    let r = run(&s);
    assert_eq!(r.sources[0].blocked, 0.0);
    assert_eq!(r.sources[0].created, 600);
    // 600 arrive, about 400 get processed, a handful are in the system.
    assert!(r.sources[0].lost > 190, "lost {}", r.sources[0].lost);
    assert_conserved(&r);
}

const TWO_SERVERS: &str = r#"
format: outliers-sim/0.1
name: two-servers
entities:
  part: {}
sources:
  arrivals: { creates: part, interarrival: "5 s" }
servers:
  fast: { process_time: "10 s" }
  slow: { process_time: "1000 s" }
conveyors:
  c1: { from: arrivals, to: fast, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
  c2: { from: fast, to: slow, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
  c3: { from: slow, to: done, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
sinks:
  done: {}
experiment:
  replications: 1
  run_length: "10 h"
  seed: 1
"#;

#[test]
fn slow_downstream_server_blocks_the_upstream_one() {
    let r = run(&spec(TWO_SERVERS));
    let fast = &r.servers[0];
    assert!(fast.blocked > 0.95, "fast server blocked {}", fast.blocked);
    assert!(
        r.conveyors[1].full > 0.95,
        "c2 full {}",
        r.conveyors[1].full
    );
    assert_near(r.servers[1].busy, 1.0, 1e-3, "slow server busy");
    // Upstream of a blocked server, everything backs up too.
    assert!(
        r.conveyors[0].full > 0.95,
        "c1 full {}",
        r.conveyors[0].full
    );
    assert!(r.sources[0].blocked > 0.95);
    let shares = fast.idle + fast.busy + fast.blocked + fast.failed;
    assert_near(shares, 1.0, 1e-9, "state shares");
    assert_conserved(&r);
}

#[test]
fn busy_time_failures_take_the_expected_share() {
    // Always busy, so busy time = time up: up 100 min, down 10 min.
    let s = with(
        DETERMINISTIC,
        &[
            (
                "interarrival: \"60 s\" }",
                "interarrival: \"1 s\", when_blocked: drop }",
            ),
            (
                "machine: { process_time: \"30 s\" }",
                "machine:\n    process_time: \"10 s\"\n    failures:\n      - { name: jam, uptime: \"100 min\", downtime: \"10 min\" }",
            ),
            ("run_length: \"10 h\"", "run_length: \"1100 h\""),
        ],
    );
    let r = run(&s);
    let m = &r.servers[0];
    assert_near(m.failed, 1.0 / 11.0, 1e-3, "failed share");
    assert_near(m.busy, 10.0 / 11.0, 1e-3, "busy share");
    assert_eq!(m.failures, 600); // 1100 h / 110 min
    assert_conserved(&r);
}

#[test]
fn busy_time_clock_pauses_while_idle() {
    // Busy half the time, so 100 min of busy time takes about 200 min.
    let s = with(
        DETERMINISTIC,
        &[
            (
                "machine: { process_time: \"30 s\" }",
                "machine:\n    process_time: \"30 s\"\n    failures:\n      - { name: jam, uptime: \"100 min\", downtime: \"1 s\" }",
            ),
            ("run_length: \"10 h\"", "run_length: \"1000 h\""),
        ],
    );
    let failures = run(&s).servers[0].failures;
    assert!((295..=305).contains(&failures), "failures {failures}"); // 60000 min / 200 min

    let clock = with(
        DETERMINISTIC,
        &[
            (
                "machine: { process_time: \"30 s\" }",
                "machine:\n    process_time: \"30 s\"\n    failures:\n      - { name: jam, uptime: \"100 min\", downtime: \"1 s\", counts: clock_time }",
            ),
            ("run_length: \"10 h\"", "run_length: \"1000 h\""),
        ],
    );
    let failures = run(&clock).servers[0].failures;
    assert!((595..=600).contains(&failures), "failures {failures}"); // 60000 min / 100 min
}

#[test]
fn restart_never_finishes_when_failures_come_faster_than_the_work() {
    let model = |on_failure: &str| {
        with(
            DETERMINISTIC,
            &[(
                "machine: { process_time: \"30 s\" }",
                &format!(
                    "machine:\n    process_time: \"100 s\"\n    failures:\n      - {{ name: jam, uptime: \"50 s\", downtime: \"10 s\", counts: clock_time, on_failure: {on_failure} }}"
                ),
            )],
        )
    };
    let resume = run(&model("resume"));
    assert!(resume.servers[0].processed > 0);
    assert_conserved(&resume);

    let restart = run(&model("restart"));
    assert_eq!(restart.servers[0].processed, 0);
    assert_eq!(restart.sinks[0].exited, 0);
    assert_conserved(&restart);
}

#[test]
fn same_seed_same_results_different_replication_different_results() {
    let s = spec(CONVEYOR_LINE);
    let a = simulate(&s, 1, &mut NoTrace);
    let b = simulate(&s, 1, &mut NoTrace);
    let c = simulate(&s, 2, &mut NoTrace);
    assert_eq!(a, b);
    assert_ne!(a.sinks[0].exited, c.sinks[0].exited);
}

#[test]
fn conveyor_line_example_behaves() {
    let s = spec(CONVEYOR_LINE);
    for replication in 1..=5 {
        let r = simulate(&s, replication, &mut NoTrace);
        assert_conserved(&r);
        for server in &r.servers {
            let shares = server.idle + server.busy + server.blocked + server.failed;
            assert_near(shares, 1.0, 1e-9, "state shares");
            assert!(server.failed > 0.0 && server.failures > 0);
        }
        for (c, spec) in r.conveyors.iter().zip(&s.conveyors) {
            assert!(c.max_parts <= u64::from(spec.capacity));
        }
        // Throughput is close to the arrival rate (1/min over 120 h) unless
        // the source spends time blocked.
        let exited = r.sinks[0].exited as f64;
        assert!(
            exited > 0.8 * 7200.0 && exited < 1.1 * 7200.0,
            "exited {exited}"
        );
    }
}

#[test]
fn accumulated_parts_close_up_one_spacing_at_a_time() {
    // c2 is backed up behind the slow server. Each time the slow server takes
    // the head, the next part is 0.5 m back and reaches the end 0.5 s later.
    let mut trace = Vec::new();
    simulate(&spec(TWO_SERVERS), 1, &mut trace);
    let takes: Vec<SimTime> = trace
        .iter()
        .filter(|(_, e)| matches!(e, TraceEvent::StartedProcess { server: 1, .. }))
        .map(|(t, _)| *t)
        .skip(2) // let the queue build up
        .take(10)
        .collect();
    assert_eq!(takes.len(), 10);
    for taken in takes {
        let next_at_end = trace
            .iter()
            .find(|(t, e)| *t > taken && matches!(e, TraceEvent::ReachedEnd { conveyor: 1, .. }))
            .map(|(t, _)| *t)
            .expect("queue is never empty");
        assert_eq!(next_at_end - taken, SimTime::from_secs_f64(0.5));
    }
}
