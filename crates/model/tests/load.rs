use simulancer_engine::SimTime;
use simulancer_engine::spec::{
    Arrivals, Distribution, FailureClock, Node, OnFailure, PerType, WhenBlocked,
};
use simulancer_model::{LoadError, load_str};

const CONVEYOR_LINE: &str = include_str!("../../../examples/conveyor-line.yaml");

/// Smallest valid model: source -> server -> sink.
const MINIMAL: &str = r#"
format: simulancer/0.1
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
        spec.sources[0].arrivals,
        Arrivals::Interarrival(Distribution::Exponential { mean: 60.0 })
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
    assert_eq!(spec.servers[1].inputs, [1]);

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
        PerType::Same(Distribution::Constant(50.0))
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
        PerType::Same(Distribution::Constant(120.0))
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
    let source = replace(MINIMAL, "simulancer/0.1", "simulancer/9.9");
    assert_one_error(&source, "format:", "unsupported format");
}

#[test]
fn group_sizes_can_be_discrete() {
    let with = |g: &str| {
        replace(
            MINIMAL,
            "    creates: part",
            &format!("    creates: part\n    group: {g}"),
        )
    };
    let model = load_str(&with("\"discrete(1: 30%, 2: 70%)\"")).unwrap();
    assert_eq!(
        model.spec.sources[0].group.as_ref().map(|c| c.mean()),
        Some(1.7)
    );
    assert_one_error(
        &with("\"discrete(0: 100%)\""),
        "group",
        "whole numbers from 1",
    );
    assert_one_error(&with("\"twelve\""), "group", "`discrete(1: 30%, 2: 70%)`");
}

#[test]
fn precision_is_a_percentage() {
    let model = load_str(MINIMAL).unwrap();
    assert_eq!(model.spec.experiment.precision, None);
    let with = |p: &str| {
        replace(
            MINIMAL,
            "  seed: 1",
            &format!("  seed: 1\n  precision: {p}"),
        )
    };
    let model = load_str(&with("2%")).unwrap();
    assert_eq!(model.spec.experiment.precision, Some(0.02));
    let model = load_str(&with("\"2.5 %\"")).unwrap();
    assert_eq!(model.spec.experiment.precision, Some(0.025));
    assert_one_error(&with("0.02"), "precision", "is a percentage, like `5%`");
    assert_one_error(
        &with("5 percent"),
        "precision",
        "is a percentage, like `5%`",
    );
    assert_one_error(&with("0%"), "precision", "between 0% and 100%");
    assert_one_error(&with("150%"), "precision", "between 0% and 100%");
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

const TWO_CELLS: &str = include_str!("../../../examples/two-cells.yaml");

#[test]
fn routing_and_merging_example_loads() {
    use simulancer_engine::spec::{InputRule, Routing};
    let model = load_str(TWO_CELLS).unwrap();
    let spec = &model.spec;
    let names = |list: &[usize]| -> Vec<&str> {
        list.iter()
            .map(|&c| spec.conveyors[c].name.as_str())
            .collect()
    };
    assert_eq!(spec.sources[0].routing, Routing::ShortestQueue);
    let mill_a = &spec.servers[0];
    assert_eq!(mill_a.input_rule, InputRule::Priority);
    // Inputs in priority order, not file order.
    assert_eq!(names(&mill_a.inputs), ["c_rework", "c_a"]);
    let inspect = &spec.servers[2];
    assert_eq!(inspect.input_rule, InputRule::FirstCome);
    assert_eq!(names(&inspect.outputs), ["c_ship", "c_rework"]);
    assert_eq!(inspect.routing, Routing::Probability(vec![0.9, 0.1]));
}

/// The two-cells example with one substitution, expecting one error.
fn two_cells_error(from: &str, to: &str, line_text: &str, expected: &str) {
    assert_one_error(&replace(TWO_CELLS, from, to), line_text, expected);
}

#[test]
fn routing_errors() {
    two_cells_error(
        "    output: { rule: shortest_queue }        # fewer parts on c_a or c_b; ties to c_a\n",
        "",
        "  arrivals:",
        "has 2 outgoing conveyors (`c_a`, `c_b`); add `output:",
    );
    two_cells_error(
        "weights: { c_ship: 0.9, c_rework: 0.1 }",
        "weights: { c_ship: 0.9, c_rework: 0.2 }",
        "rule: probability",
        "`weights` add up to 1.1",
    );
    two_cells_error(
        "weights: { c_ship: 0.9, c_rework: 0.1 }",
        "weights: { c_ship: 0.9, c_rewrok: 0.1 }",
        "c_rewrok",
        "`c_rewrok` isn't an outgoing conveyor of `inspect`",
    );
    two_cells_error(
        "weights: { c_ship: 0.9, c_rework: 0.1 }",
        "weights: { c_ship: 1.0 }",
        "rule: probability",
        "`weights` has no entry for `c_rework`",
    );
    two_cells_error(
        "weights: { c_ship: 0.9, c_rework: 0.1 }",
        "weights: { c_ship: 1.1, c_rework: -0.1 }",
        "c_rework: -0.1",
        "weights must be greater than zero",
    );
    two_cells_error(
        "output: { rule: shortest_queue }",
        "output: { rule: shortest_queue, weights: { c_a: 1 } }",
        "output: { rule: shortest_queue",
        "`weights` only apply to `rule: probability`",
    );
    two_cells_error(
        "output: { rule: shortest_queue }",
        "output: { rule: shortest }",
        "output: { rule: shortest }",
        "unknown variant `shortest`",
    );
}

#[test]
fn merging_errors() {
    two_cells_error(
        "    input: { rule: first_come }\n",
        "",
        "  inspect:",
        "has 2 incoming conveyors (`c_a_out`, `c_b_out`); add `input:",
    );
    two_cells_error(
        "input: { rule: priority, order: [c_rework, c_a] }",
        "input: { rule: priority }",
        "input: { rule: priority }",
        "`rule: priority` needs `order`",
    );
    two_cells_error(
        "order: [c_rework, c_a]",
        "order: [c_rework]",
        "order: [c_rework]",
        "`order` is missing `c_a`",
    );
    two_cells_error(
        "order: [c_rework, c_a]",
        "order: [c_rework, c_a, c_rework]",
        "order: [c_rework, c_a, c_rework]",
        "`c_rework` is listed twice",
    );
    two_cells_error(
        "order: [c_rework, c_a]",
        "order: [c_rework, c_b]",
        "order: [c_rework, c_b]",
        "`c_b` isn't an incoming conveyor of `mill_a`",
    );
    two_cells_error(
        "input: { rule: first_come }",
        "input: { rule: first_come, order: [c_a_out, c_b_out] }",
        "input: { rule: first_come",
        "`order` only applies to `rule: priority`",
    );
}

#[test]
fn parts_must_be_able_to_reach_a_sink() {
    // Point inspect's shipping conveyor at mill_b and drop the sink: parts
    // now circulate mill_a/mill_b -> inspect -> mill_a/mill_b forever.
    let trap = replace(
        TWO_CELLS,
        "  c_ship:   { from: inspect,  to: shipping,",
        "  c_ship:   { from: inspect,  to: mill_b,",
    );
    let trap = replace(&trap, "sinks:\n  shipping: {}\n", "sinks: {}\n");
    // mill_b now merges two inputs and needs a rule.
    let trap = replace(
        &trap,
        "    process_time: \"triangular(1 min, 1.4 min, 2.1 min)\"\n    failures:",
        "    process_time: \"triangular(1 min, 1.4 min, 2.1 min)\"\n    input: { rule: first_come }\n    failures:",
    );
    let messages: Vec<String> = errors(&trap).into_iter().map(|(_, m)| m).collect();
    for object in ["server `mill_a`", "server `mill_b`", "server `inspect`"] {
        assert!(
            messages
                .iter()
                .any(|m| m.contains(&format!("parts at {object} can never reach a sink"))),
            "{messages:#?}"
        );
    }
    assert!(
        messages
            .iter()
            .any(|m| m.contains("parts from source `arrivals` can never reach a sink")),
        "{messages:#?}"
    );
}

const TYPED: &str = r#"
format: simulancer/0.1
name: typed
entities:
  frame: {}
  wheel: {}
sources:
  arrivals: { creates: { frame: 0.5, wheel: 0.5 }, interarrival: "1 min" }
servers:
  mill:
    process_time: { frame: "10 s", wheel: "20 s" }
    output: { rule: by_type, routes: { frame: c_a, wheel: c_b } }
conveyors:
  c_in: { from: arrivals, to: mill, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
  c_a: { from: mill, to: out, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
  c_b: { from: mill, to: out, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
sinks:
  out: {}
experiment: { replications: 1, run_length: "1 h", seed: 1 }
"#;

#[test]
fn part_types_load() {
    let spec = load_str(TYPED).unwrap().spec;
    assert_eq!(spec.sources[0].creates, vec![(0, 0.5), (1, 0.5)]);
    assert!(matches!(spec.servers[0].process_time, PerType::ByType(_)));
}

#[test]
fn part_type_problems_are_reported() {
    for (from, to, expected) in [
        (
            "creates: { frame: 0.5, wheel: 0.5 }",
            "creates: { frame: 0.5, wheel: 0.4 }",
            "add up to 0.9",
        ),
        (
            "creates: { frame: 0.5, wheel: 0.5 }",
            "creates: bike",
            "`bike`, which isn't listed under `entities` (part types: `frame`, `wheel`)",
        ),
        (
            "process_time: { frame: \"10 s\", wheel: \"20 s\" }",
            "process_time: { frame: \"10 s\" }",
            "gets parts of type `wheel`, but `process_time` has no time for them",
        ),
        (
            "routes: { frame: c_a, wheel: c_b }",
            "routes: { frame: c_a }",
            "its `by_type` rule has no route for them",
        ),
        (
            "routes: { frame: c_a, wheel: c_b }",
            "routes: { frame: c_a, wheel: c_in }",
            "`c_in` isn't an outgoing conveyor of `mill`",
        ),
        (
            "output: { rule: by_type, routes: { frame: c_a, wheel: c_b } }",
            "output: { rule: by_type }",
            "needs `routes`",
        ),
        (
            "process_time: { frame: \"10 s\", wheel: \"20 s\" }",
            "process_time: { frame: \"10 s\", wheel: \"soon\" }",
            "`wheel`: ",
        ),
    ] {
        assert!(TYPED.contains(from), "test bug: {from}");
        let errors = errors(&TYPED.replacen(from, to, 1));
        assert!(
            errors.iter().any(|(_, e)| e.contains(expected)),
            "{to}: expected `{expected}` in {errors:?}"
        );
    }
}

#[test]
fn resource_problems_are_reported() {
    let base = TYPED.replace(
        "servers:\n  mill:\n",
        "resources:\n  operator: { capacity: 1 }\nservers:\n  mill:\n    needs: { operator: 1 }\n",
    );
    assert_eq!(load_str(&base).unwrap().spec.servers[0].needs, vec![(0, 1)]);
    for (from, to, expected) in [
        (
            "needs: { operator: 1 }",
            "needs: { operator: 2 }",
            "there are only 1: it could never start",
        ),
        (
            "needs: { operator: 1 }",
            "needs: { helper: 1 }",
            "`helper`, which isn't listed under `resources` (resources: `operator`)",
        ),
        (
            "needs: { operator: 1 }",
            "needs: { operator: 0 }",
            "needs at least 1 `operator`",
        ),
        (
            "operator: { capacity: 1 }",
            "operator: { capacity: 0 }",
            "needs a capacity of at least 1",
        ),
        (
            "operator: { capacity: 1 }",
            "mill: { capacity: 1 }",
            "already used by a server",
        ),
    ] {
        let errors = errors(&base.replacen(from, to, 1));
        assert!(
            errors.iter().any(|(_, e)| e.contains(expected)),
            "{to}: expected `{expected}` in {errors:?}"
        );
    }
}

#[test]
fn schedule_problems_are_reported() {
    let base = TYPED.replace(
        "servers:\n  mill:\n",
        "schedules:\n  days: { shifts: [\"06:00-14:00\"], days: [mon, fri] }\nservers:\n  mill:\n    schedule: days\n",
    );
    let spec = load_str(&base).unwrap().spec;
    assert_eq!(spec.servers[0].schedule, Some(0));
    assert_eq!(spec.schedules[0].periods.len(), 2);
    for (from, to, expected) in [
        (
            "schedule: days",
            "schedule: nights",
            "schedule `nights`, which isn't listed under `schedules` (schedules: `days`)",
        ),
        (
            "\"06:00-14:00\"",
            "\"6am-2pm\"",
            "`6am-2pm` isn't a shift: write it like `06:00-14:00`",
        ),
        (
            "\"06:00-14:00\"",
            "\"06:00-06:00\"",
            "starts and ends at the same time",
        ),
        ("\"06:00-14:00\"", "\"25:00-26:00\"", "isn't a shift"),
        (
            "days: [mon, fri]",
            "days: [mon, funday]",
            "`funday` isn't a day",
        ),
        ("shifts: [\"06:00-14:00\"]", "shifts: []", "has no shifts"),
    ] {
        let errors = errors(&base.replacen(from, to, 1));
        assert!(
            errors.iter().any(|(_, e)| e.contains(expected)),
            "{to}: expected `{expected}` in {errors:?}"
        );
    }
}

#[test]
fn assembly_and_batch_problems_are_reported() {
    const KITS: &str = r#"
format: simulancer/0.1
name: kits
entities: { frame: {}, wheel: {}, bike: {} }
sources:
  frames: { creates: frame, interarrival: "60 s" }
  wheels: { creates: wheel, interarrival: "30 s" }
servers:
  assembly: { assembles: { frame: 1, wheel: 2 }, makes: bike, process_time: "20 s", input: { rule: first_come } }
conveyors:
  c_frames: { from: frames, to: assembly, length: "10 m", speed: "1 m/s", spacing: "1 m" }
  c_wheels: { from: wheels, to: assembly, length: "10 m", speed: "1 m/s", spacing: "1 m" }
  c_bikes:  { from: assembly, to: shipping, length: "2 m", speed: "1 m/s", spacing: "1 m" }
sinks:
  shipping: {}
experiment: { replications: 1, run_length: "1 h", seed: 1 }
"#;
    load_str(KITS).unwrap();
    for (from, to, expected) in [
        ("makes: bike, ", "", "add `makes:`"),
        (
            "assembles: { frame: 1, wheel: 2 }, ",
            "batch: 2, ",
            "`makes` goes with `assembles`",
        ),
        (
            "process_time: \"20 s\"",
            "batch: 3, process_time: \"20 s\"",
            "can batch or assemble, not both",
        ),
        (
            "wheels: { creates: wheel",
            "wheels: { creates: frame",
            "assembles `wheel`, but none can reach it",
        ),
        (
            "{ frame: 1, wheel: 2 }",
            "{ frame: 1 }",
            "gets parts of type `wheel`, which aren't in what it assembles",
        ),
        (
            "process_time: \"20 s\"",
            "process_time: { bike: \"20 s\" }",
            "one process time, not one per part type",
        ),
        (
            "makes: bike",
            "makes: car",
            "makes `car`, which isn't listed",
        ),
    ] {
        assert!(KITS.contains(from), "test bug: {from}");
        let errors = errors(&KITS.replacen(from, to, 1));
        assert!(
            errors.iter().any(|(_, e)| e.contains(expected)),
            "{to}: expected `{expected}` in {errors:?}"
        );
    }
    let batch = TYPED.replace(
        "process_time: { frame: \"10 s\", wheel: \"20 s\" }",
        "batch: 4\n    capacity: 6\n    process_time: \"10 s\"",
    );
    let found = errors(&batch);
    assert!(
        found
            .iter()
            .any(|(_, e)| e.contains("must be a multiple of 4")),
        "{found:?}"
    );
}

/// MINIMAL with its first conveyor replaced by a queue.
fn with_queue(queue: &str) -> String {
    replace(
        MINIMAL,
        "conveyors:\n  c_in:  { from: arrivals, to: machine, length: \"2 m\", speed: \"1 m/s\", spacing: \"0.5 m\" }\n",
        &format!("queues:\n  line: {queue}\nconveyors:\n"),
    )
}

#[test]
fn queues_connect_objects_with_no_travel() {
    let model = load_str(&with_queue("{ from: arrivals, to: machine, capacity: 5 }")).unwrap();
    let spec = &model.spec;
    // Queues come after the conveyors.
    let line = &spec.conveyors[1];
    assert!(line.is_queue() && !spec.conveyors[0].is_queue());
    assert_eq!(
        (line.from, line.to, line.capacity),
        (Node::Source(0), Node::Server(0), 5)
    );
    assert_eq!(line.transit_time(), 0.0);
    assert_eq!(spec.sources[0].outputs, [1]);
    assert_eq!(spec.servers[0].inputs, [1]);

    let unlimited = load_str(&with_queue("{ from: arrivals, to: machine }")).unwrap();
    assert_eq!(unlimited.spec.conveyors[1].capacity, u32::MAX);
    assert_eq!(unlimited.spec.conveyors[1].target_wait, None);

    let target = load_str(&with_queue(
        "{ from: arrivals, to: machine, target_wait: \"5 min\" }",
    ))
    .unwrap();
    assert_eq!(target.spec.conveyors[1].target_wait, Some(300.0));
}

#[test]
fn parts_can_be_called_what_they_are() {
    let default = load_str(MINIMAL).unwrap();
    assert_eq!(
        (default.words.one.as_str(), default.words.many.as_str()),
        ("part", "parts")
    );
    let bank = load_str(&replace(
        MINIMAL,
        "entities:",
        "parts: { one: customer, many: customers }\nentities:",
    ))
    .unwrap();
    assert_eq!(
        (bank.words.one.as_str(), bank.words.many.as_str()),
        ("customer", "customers")
    );
    assert_one_error(
        &replace(
            MINIMAL,
            "entities:",
            "parts: { one: customer, many: \"\" }\nentities:",
        ),
        "parts:",
        "`parts` words are short names",
    );
}

#[test]
fn queue_errors() {
    assert_one_error(
        &with_queue("{ from: arrivals, to: machine, capacity: 0 }"),
        "capacity: 0",
        "queue `line` needs room for at least one part",
    );
    assert_one_error(
        &with_queue("{ from: arrivals, to: machine, target_wait: \"5 m\" }"),
        "target_wait",
        "is a length",
    );
    assert_one_error(
        &with_queue("{ from: done, to: machine }"),
        "from: done",
        "queue `line` can't have `from: done`: nothing comes out of a sink",
    );
    assert_one_error(
        &with_queue("{ from: arrivals, to: c_out }"),
        "to: c_out",
        "conveyors and queues connect objects, not other conveyors or queues",
    );
    let duplicate = errors(&replace(
        &with_queue("{ from: arrivals, to: machine }"),
        "  done: {}",
        "  done: {}\n  line: {}",
    ));
    assert!(
        duplicate
            .iter()
            .any(|(_, m)| m.contains("`line` is already used by a queue")),
        "{duplicate:#?}"
    );
}

/// MINIMAL with attributes, and `sets` on the source and the server.
fn with_attributes(attributes: &str, source_sets: &str, server_sets: &str) -> String {
    let s = replace(
        MINIMAL,
        "sources:\n",
        &format!("attributes:\n{attributes}\nsources:\n"),
    );
    let s = replace(
        &s,
        "    interarrival: \"exponential(1 min)\"\n",
        &format!("    interarrival: \"exponential(1 min)\"\n    sets: {source_sets}\n"),
    );
    replace(
        &s,
        "    process_time: \"50 s\"\n",
        &format!("    process_time: \"50 s\"\n    sets: {server_sets}\n"),
    )
}

#[test]
fn attributes_load_with_their_kinds_and_defaults() {
    let source = with_attributes(
        "  due: { default: \"0 s\" }\n  rework: { default: 0 }",
        "{ due: \"now + 4 h\" }",
        "{ rework: \"rework + 1\" }",
    );
    let model = load_str(&source).unwrap();
    let a = &model.spec.attributes;
    assert_eq!((a[0].name.as_str(), a[0].time), ("due", true));
    assert_eq!(
        (a[1].name.as_str(), a[1].time, a[1].default),
        ("rework", false, 0.0)
    );
    assert_eq!(model.spec.sources[0].sets.len(), 1);
    assert_eq!(model.spec.servers[0].sets[0].0, 1);
}

#[test]
fn attribute_errors_point_at_the_line() {
    let attributes = "  due: { default: \"0 s\" }\n  rework: { default: 0 }";
    let cases = [
        (
            with_attributes(attributes, "{ due: \"now + 4\" }", "{}"),
            "due: \"now + 4\"",
            "can't add a time and a plain number",
        ),
        (
            with_attributes(attributes, "{}", "{ rework: \"now\" }"),
            "rework: \"now\"",
            "`rework` holds a plain number, but `now` is a time",
        ),
        (
            with_attributes(attributes, "{}", "{ priority: 1 }"),
            "priority: 1",
            "`priority` isn't listed under `attributes` (attributes: `due`, `rework`)",
        ),
        (
            with_attributes("  now: { default: 0 }", "{}", "{}"),
            "now: { default",
            "formulas use that word",
        ),
        (
            with_attributes("  due: { default: soon }", "{}", "{}"),
            "due: { default",
            "a plain number (`0`) or a time (`0 s`)",
        ),
    ];
    for (source, line, expected) in cases {
        assert_one_error(&source, line, expected);
    }
}

#[test]
fn at_most_eight_attributes() {
    let many: Vec<String> = (1..=9)
        .map(|i| format!("  a{i}: {{ default: 0 }}"))
        .collect();
    let source = with_attributes(&many.join("\n"), "{}", "{}");
    assert_one_error(&source, "a9:", "at most 8 attributes");
}

/// MINIMAL with a queue into the machine instead of `c_in`, an attribute,
/// and `order` on the queue.
fn queue_with_order(order: &str) -> String {
    let s = replace(
        MINIMAL,
        "  c_in:  { from: arrivals, to: machine, length: \"2 m\", speed: \"1 m/s\", spacing: \"0.5 m\" }\n",
        "",
    );
    let s = replace(
        &s,
        "sources:\n",
        "attributes:\n  due: { default: \"0 s\" }\nsources:\n",
    );
    replace(
        &s,
        "sinks:\n",
        &format!("queues:\n  line: {{ from: arrivals, to: machine, order: {order} }}\nsinks:\n"),
    )
}

#[test]
fn queue_orders_load() {
    for order in [
        "first_in",
        "last_in",
        "{ lowest: due }",
        "{ highest: due }",
        "{ by_type: [part] }",
    ] {
        load_str(&queue_with_order(order)).unwrap_or_else(|e| panic!("{order}: {e:?}"));
    }
}

#[test]
fn queue_order_errors() {
    let cases = [
        (
            "{ lowest: size }",
            "`size` isn't listed under `attributes` (attributes: `due`)",
        ),
        ("{ by_type: [bike] }", "`bike` isn't a part type"),
        ("newest", "`newest` isn't an order"),
        ("{ lowest: due, highest: due }", "an order has one rule"),
    ];
    for (order, expected) in cases {
        let source = queue_with_order(order);
        assert_one_error(&source, "line: {", expected);
    }
}

#[test]
fn input_and_resource_priority_errors() {
    let two_inputs = replace(
        MINIMAL,
        "sources:\n",
        "attributes:\n  due: { default: \"0 s\" }\nresources:\n  operator: { capacity: 1, order: [nobody] }\nsources:\n  more: { creates: part, interarrival: \"1 min\" }\n",
    );
    let two_inputs = replace(
        &two_inputs,
        "sinks:\n",
        "queues:\n  q_more: { from: more, to: machine }\nsinks:\n",
    );
    let with_rule = |rule: &str| {
        replace(
            &two_inputs,
            "    process_time: \"50 s\"\n",
            &format!("    process_time: \"50 s\"\n    input: {rule}\n"),
        )
    };
    let errors_of =
        |source: &str| -> Vec<String> { errors(source).into_iter().map(|e| e.1).collect() };
    let messages = errors_of(&with_rule("{ rule: lowest }"));
    assert!(
        messages
            .iter()
            .any(|m| m.contains("need `by: <attribute>`")),
        "{messages:?}"
    );
    let messages = errors_of(&with_rule("{ rule: first_come, by: due }"));
    assert!(
        messages.iter().any(|m| m.contains("`by` only applies")),
        "{messages:?}"
    );
    let messages = errors_of(&with_rule("{ rule: highest, by: due }"));
    assert_eq!(messages, ["`order` lists servers, and `nobody` isn't one"]);
    let not_needed = replace(
        &with_rule("{ rule: highest, by: due }"),
        "[nobody]",
        "[machine]",
    );
    assert_eq!(
        errors_of(&not_needed),
        ["server `machine` doesn't need `operator`, so it can't be in its `order`"]
    );
}

/// MINIMAL with a second exit and `by_condition` routes out of the machine.
fn with_routes(routes: &str) -> String {
    let s = replace(
        MINIMAL,
        "sources:\n",
        "attributes:\n  rework: { default: 0 }\nsources:\n",
    );
    let s = replace(
        &s,
        "    process_time: \"50 s\"\n",
        &format!(
            "    process_time: \"50 s\"\n    output:\n      rule: by_condition\n      routes:\n{routes}"
        ),
    );
    replace(
        &s,
        "sinks:\n  done: {}\n",
        "queues:\n  q_scrap: { from: machine, to: scrap }\nsinks:\n  done: {}\n  scrap: {}\n",
    )
}

#[test]
fn conditional_routes_load_and_check_their_conditions() {
    let ok = "        - { when: \"rework >= 2 and type == part\", to: q_scrap }\n        - { to: c_out }\n";
    let model = load_str(&with_routes(ok)).unwrap();
    assert!(matches!(
        &model.spec.servers[0].routing,
        simulancer_engine::spec::Routing::ByCondition(r) if r.len() == 2 && r[1].0.is_none()
    ));
    let cases = [
        (
            "        - { when: \"rework + 1\", to: q_scrap }\n        - { to: c_out }\n",
            "when: \"rework + 1\"",
            "`when` is a condition, like `rework >= 2`, but `rework + 1` is a plain number",
        ),
        (
            "        - { to: q_scrap }\n        - { to: c_out }\n",
            "- { to: q_scrap }",
            "only the last route goes without `when`",
        ),
        (
            "        - { when: \"rework >= 2\", to: q_scrap }\n        - { when: \"rework < 2\", to: c_out }\n",
            "when: \"rework < 2\"",
            "the last route has no `when`",
        ),
        (
            "        - { when: \"rework >= 2\", to: nowhere }\n        - { to: c_out }\n",
            "to: nowhere",
            "`nowhere` isn't an outgoing conveyor of `machine`",
        ),
    ];
    for (routes, line, expected) in cases {
        assert_one_error(&with_routes(routes), line, expected);
    }
}

#[test]
fn patience_needs_a_time_for_every_type_that_waits() {
    let source = replace(
        &queue_with_order("first_in"),
        "line: { from: arrivals, to: machine, order: first_in }",
        "line: { from: arrivals, to: machine, patience: { part: \"exponential(10 min)\" } }",
    );
    load_str(&source).unwrap();
    let source = replace(
        &source,
        "{ part: \"exponential(10 min)\" }",
        "{ other: \"10 min\" }",
    );
    let errors = errors(&source);
    assert!(
        errors
            .iter()
            .any(|e| e.1.contains("queue `line` has a patience for `other`")),
        "{errors:?}"
    );
}

#[test]
fn carried_resources_must_be_given_back_before_parts_leave() {
    let base = replace(
        MINIMAL,
        "sources:\n",
        "resources:\n  pallets: { capacity: 4 }\nsources:\n",
    );
    let with = |takes: &str| {
        replace(
            &base,
            "    process_time: \"50 s\"\n",
            &format!("    process_time: \"50 s\"\n{takes}"),
        )
    };
    // Taken and given back at the same server: fine.
    load_str(&with(
        "    takes: { pallets: 1 }\n    gives_back: [pallets]\n",
    ))
    .unwrap();
    let messages =
        |source: &str| -> Vec<String> { errors(source).into_iter().map(|e| e.1).collect() };
    let leak = messages(&with("    takes: { pallets: 1 }\n"));
    assert!(
        leak.iter().any(|m| m.contains("no server gives it back")),
        "{leak:?}"
    );
    assert!(
        leak.iter().any(|m| m
            .contains("parts that take `pallets` at `machine` can reach `done` still carrying it")),
        "{leak:?}"
    );
    let stray = messages(&with("    gives_back: [pallets]\n"));
    assert_eq!(
        stray,
        ["server `machine` gives back `pallets`, but no server `takes` it"]
    );
    let too_many = messages(&with(
        "    takes: { pallets: 5 }\n    gives_back: [pallets]\n",
    ));
    assert!(
        too_many[0].contains("takes 5 `pallets`, but there are only 4"),
        "{too_many:?}"
    );
    let shifts = replace(
        &with("    takes: { pallets: 1 }\n    gives_back: [pallets]\n"),
        "pallets: { capacity: 4 }",
        "pallets: { capacity: 4, schedule: days }\nschedules:\n  days: { shifts: [\"08:00-16:00\"] }",
    );
    assert!(
        messages(&shifts)
            .iter()
            .any(|m| m.contains("can't have a schedule"))
    );
}
