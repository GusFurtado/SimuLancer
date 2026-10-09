use outliers_sim_engine::SimTime;
use outliers_sim_engine::spec::{Distribution, FailureClock, Node, OnFailure, WhenBlocked};
use outliers_sim_model::{LoadError, load_str};

const CONVEYOR_LINE: &str = include_str!("../../../examples/conveyor-line.yaml");

/// Smallest valid model: source -> server -> sink.
const MINIMAL: &str = r#"
format: outliers-sim/0.1
name: minimal
entities:
  part: {}
sources:
  arrivals:
    creates: part
    interarrival: "exponential(1 min)"
servers:
  machine:
    process_time: "50 s"
conveyors:
  c_in:  { from: arrivals, to: machine, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
  c_out: { from: machine, to: done, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
sinks:
  done: {}
experiment:
  replications: 10
  run_length: "8 h"
  seed: 1
"#;

/// Loads `source`, expecting it to fail, and returns (line, message) for each error.
fn errors(source: &str) -> Vec<(Option<u64>, String)> {
    match load_str(source) {
        Ok(_) => panic!("expected the model to be rejected"),
        Err(err) => err
            .diagnostics()
            .iter()
            .map(|d| (d.position.map(|p| p.line), d.message.clone()))
            .collect(),
    }
}

/// Asserts a single error containing `expected`, on the line containing `line_text`.
fn assert_one_error(source: &str, line_text: &str, expected: &str) {
    let errors = errors(source);
    assert_eq!(errors.len(), 1, "expected one error, got {errors:#?}");
    let (line, message) = &errors[0];
    assert!(
        message.contains(expected),
        "`{message}` doesn't contain `{expected}`"
    );
    let expected_line = source
        .lines()
        .position(|l| l.contains(line_text))
        .map(|i| i as u64 + 1);
    assert_eq!(*line, expected_line, "wrong line for `{message}`");
}

fn replace(source: &str, from: &str, to: &str) -> String {
    assert!(source.contains(from), "test bug: `{from}` not in source");
    source.replacen(from, to, 1)
}

#[test]
fn conveyor_line_example_loads() {
    let model = load_str(CONVEYOR_LINE).unwrap();
    let spec = &model.spec;

    assert_eq!(model.name, "conveyor-line");
    assert_eq!(spec.entities, ["part"]);
    assert_eq!(spec.sources[0].when_blocked, WhenBlocked::Wait);
    assert_eq!(
        spec.sources[0].interarrival,
        Distribution::Exponential { mean: 60.0 }
    );

    let lathe = &spec.servers[0];
    assert_eq!(lathe.name, "lathe");
    assert_eq!(
        lathe.failures[0].downtime,
        Distribution::Uniform {
            min: 240.0,
            max: 720.0
        }
    );
    assert_eq!(lathe.failures[0].clock, FailureClock::BusyTime);
    assert_eq!(lathe.failures[0].on_failure, OnFailure::Resume);

    let capacities: Vec<u32> = spec.conveyors.iter().map(|c| c.capacity).collect();
    assert_eq!(capacities, [24, 16, 12]);
    assert_eq!(spec.conveyors[1].from, Node::Server(0));
    assert_eq!(spec.conveyors[1].to, Node::Server(1));
    assert_eq!(spec.servers[1].input, 1);

    assert_eq!(spec.experiment.warmup, SimTime::from_secs_f64(8.0 * 3600.0));
    assert_eq!(
        spec.experiment.run_length,
        SimTime::from_secs_f64(120.0 * 3600.0)
    );
}

#[test]
fn minimal_model_uses_defaults() {
    let model = load_str(MINIMAL).unwrap();
    assert_eq!(model.spec.servers[0].capacity, 1);
    assert_eq!(
        model.spec.servers[0].process_time,
        Distribution::Constant(50.0)
    );
    assert_eq!(model.spec.experiment.warmup, SimTime::ZERO);
}

#[test]
fn default_units_allow_plain_numbers() {
    let source = replace(
        MINIMAL,
        "entities:",
        "units: { time: min, length: m }\nentities:",
    );
    let source = replace(&source, r#"process_time: "50 s""#, "process_time: 2");
    let source = replace(
        &source,
        r#"length: "2 m", speed: "1 m/s""#,
        "length: 3, speed: 30",
    );
    let model = load_str(&source).unwrap();
    assert_eq!(
        model.spec.servers[0].process_time,
        Distribution::Constant(120.0)
    );
    assert_eq!(model.spec.conveyors[0].length, 3.0);
    assert_eq!(model.spec.conveyors[0].speed, 0.5);
}

#[test]
fn plain_number_without_default_unit() {
    let source = replace(MINIMAL, r#"process_time: "50 s""#, "process_time: 50");
    assert_one_error(&source, "process_time", "has no unit");
}

#[test]
fn unsupported_format() {
    let source = replace(MINIMAL, "outliers-sim/0.1", "outliers-sim/9.9");
    assert_one_error(&source, "format:", "unsupported format");
}

#[test]
fn unknown_field_is_a_syntax_error() {
    let source = replace(MINIMAL, "process_time:", "proces_time:");
    let err = load_str(&source).unwrap_err();
    assert!(matches!(err, LoadError::Syntax(_)));
    assert_one_error(&source, "proces_time", "unknown field `proces_time`");
}

#[test]
fn unknown_option_value() {
    let source = replace(
        MINIMAL,
        "interarrival:",
        "when_blocked: block\n    interarrival:",
    );
    assert_one_error(&source, "when_blocked", "unknown variant `block`");
}

#[test]
fn source_creates_unknown_entity() {
    let source = replace(MINIMAL, "creates: part", "creates: widget");
    assert_one_error(&source, "creates", "isn't listed under `entities`");
}

#[test]
fn conveyor_to_missing_object() {
    let source = replace(MINIMAL, "to: done", "to: dnoe");
    // Only the cause: no follow-on "sink has no incoming conveyor".
    assert_one_error(&source, "to: dnoe", "goes to `dnoe`, which doesn't exist");
}

#[test]
fn conveyor_out_of_a_sink() {
    let source = replace(MINIMAL, "from: machine", "from: done");
    assert_one_error(&source, "from: done", "nothing comes out of a sink");
}

#[test]
fn duplicate_name_across_types() {
    let source = replace(MINIMAL, "  done: {}", "  machine: {}");
    let errors = errors(&source);
    assert!(
        errors
            .iter()
            .any(|(_, m)| m.contains("`machine` is already used by a server")),
        "{errors:#?}"
    );
}

#[test]
fn wrong_dimension() {
    let source = replace(MINIMAL, r#"speed: "1 m/s""#, r#"speed: "1 min""#);
    assert_one_error(&source, "c_in", "is a time, but a speed is expected");
}

#[test]
fn bad_distribution() {
    let source = replace(
        MINIMAL,
        "exponential(1 min)",
        "triangular(5 min, 1 min, 3 min)",
    );
    assert_one_error(&source, "interarrival", "`min` <= `mode` <= `max`");
}

#[test]
fn spacing_longer_than_conveyor() {
    let source = replace(MINIMAL, r#"spacing: "0.5 m" }"#, r#"spacing: "3 m" }"#);
    assert_one_error(&source, "c_in", "shorter than one spacing");
}

#[test]
fn two_outgoing_conveyors() {
    let source = replace(
        MINIMAL,
        "sinks:",
        "  c_extra: { from: machine, to: done, length: \"2 m\", speed: \"1 m/s\", spacing: \"0.5 m\" }\nsinks:",
    );
    assert_one_error(&source, "  machine:", "has 2 outgoing conveyors");
}

#[test]
fn loop_of_servers_nothing_feeds() {
    let source = replace(
        MINIMAL,
        "conveyors:",
        "  a:\n    process_time: \"1 s\"\n  b:\n    process_time: \"1 s\"\nconveyors:\n  c_ab: { from: a, to: b, length: \"1 m\", speed: \"1 m/s\", spacing: \"0.5 m\" }\n  c_ba: { from: b, to: a, length: \"1 m\", speed: \"1 m/s\", spacing: \"0.5 m\" }",
    );
    let errors = errors(&source);
    assert_eq!(errors.len(), 2, "{errors:#?}");
    assert!(
        errors
            .iter()
            .all(|(_, m)| m.contains("can't be reached from any source"))
    );
}

#[test]
fn zero_replications() {
    let source = replace(MINIMAL, "replications: 10", "replications: 0");
    assert_one_error(&source, "replications", "at least 1");
}

#[test]
fn reports_all_errors_in_file_order() {
    let source = replace(MINIMAL, "creates: part", "creates: widget");
    let source = replace(&source, "replications: 10", "replications: 0");
    let source = replace(
        &source,
        r#"process_time: "50 s""#,
        r#"process_time: "50 m""#,
    );
    let lines: Vec<Option<u64>> = errors(&source).into_iter().map(|(l, _)| l).collect();
    assert_eq!(lines.len(), 3);
    assert!(lines.is_sorted(), "{lines:?}");
}

#[test]
fn invalid_names() {
    let source = replace(MINIMAL, "  done: {}", "  \"shipping dock\": {}");
    let source = replace(&source, "to: done", "to: shipping dock");
    let errors = errors(&source);
    assert!(
        errors
            .iter()
            .any(|(_, m)| m.contains("is not a valid name")),
        "{errors:#?}"
    );
}
