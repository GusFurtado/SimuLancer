//! Routing (several outgoing conveyors) and merging (several incoming).

use outliers_sim_engine::{NoTrace, ReplicationResult, SimTime, Spec, TraceEvent, simulate};

const TWO_CELLS: &str = include_str!("../../../examples/two-cells.yaml");

fn spec(yaml: &str) -> Spec {
    match outliers_sim_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

fn run(spec: &Spec) -> ReplicationResult {
    simulate(spec, 1, &mut NoTrace)
}

fn conveyor(spec: &Spec, r: &ReplicationResult, name: &str) -> u64 {
    let i = spec.conveyors.iter().position(|c| c.name == name).unwrap();
    r.conveyors[i].entered
}

fn conserved(r: &ReplicationResult) {
    let t = &r.totals;
    assert_eq!(t.created, t.exited + t.lost + t.in_system, "{t:?}");
}

const C: &str = r#"length: "2 m", speed: "1 m/s", spacing: "0.5 m""#;

/// A source split over two conveyors straight into two sinks.
fn split(output: &str, interarrival: &str) -> String {
    format!(
        r#"
format: outliers-sim/0.1
name: split
entities: {{ part: {{}} }}
sources:
  arrivals: {{ creates: part, interarrival: "{interarrival}", output: {output} }}
conveyors:
  left:  {{ from: arrivals, to: sink_left, {C} }}
  right: {{ from: arrivals, to: sink_right, {C} }}
sinks:
  sink_left: {{}}
  sink_right: {{}}
experiment: {{ replications: 1, run_length: "1000 h", seed: 3 }}
"#
    )
}

#[test]
fn probability_split_matches_weights() {
    let s = spec(&split(
        "{ rule: probability, weights: { left: 0.3, right: 0.7 } }",
        "exponential(10 s)",
    ));
    let r = run(&s);
    let (left, right) = (conveyor(&s, &r, "left"), conveyor(&s, &r, "right"));
    let share = left as f64 / (left + right) as f64;
    // ~360,000 parts: the binomial standard error is under 0.001.
    assert!((share - 0.3).abs() < 0.004, "left share {share}");
    conserved(&r);
}

#[test]
fn round_robin_split_alternates_exactly() {
    let s = spec(&split("{ rule: round_robin }", "exponential(10 s)"));
    let r = run(&s);
    let (left, right) = (conveyor(&s, &r, "left"), conveyor(&s, &r, "right"));
    assert!(left.abs_diff(right) <= 1, "left {left}, right {right}");
}

/// A source split over two conveyors to two servers: `a` (process time
/// given) and `b` (always fast).
fn split_to_servers(output: &str, a_process: &str) -> String {
    format!(
        r#"
format: outliers-sim/0.1
name: split-servers
entities: {{ part: {{}} }}
sources:
  arrivals: {{ creates: part, interarrival: "10 s", output: {output} }}
servers:
  a: {{ process_time: "{a_process}" }}
  b: {{ process_time: "1 s" }}
conveyors:
  to_a: {{ from: arrivals, to: a, {C} }}
  to_b: {{ from: arrivals, to: b, {C} }}
  a_out: {{ from: a, to: done, {C} }}
  b_out: {{ from: b, to: done, {C} }}
sinks:
  done: {{}}
experiment: {{ replications: 1, run_length: "10 h", seed: 1 }}
"#
    )
}

#[test]
fn first_available_uses_later_outputs_only_when_earlier_ones_are_full() {
    // `a` keeps up (5 s per part, one every 10 s): `to_a` never fills.
    let s = spec(&split_to_servers("{ rule: first_available }", "5 s"));
    let r = run(&s);
    assert_eq!(conveyor(&s, &r, "to_b"), 0);

    // `a` is too slow (25 s): `to_a` fills, the overflow goes to `b`.
    let s = spec(&split_to_servers("{ rule: first_available }", "25 s"));
    let r = run(&s);
    assert!(
        conveyor(&s, &r, "to_b") > 1000,
        "to_b {}",
        conveyor(&s, &r, "to_b")
    );
    conserved(&r);
}

#[test]
fn shortest_queue_balances_by_parts_on_the_conveyor() {
    // `a` is slow, so `to_a` holds more parts and `b` gets most of the work.
    let s = spec(&split_to_servers("{ rule: shortest_queue }", "25 s"));
    let r = run(&s);
    let (to_a, to_b) = (conveyor(&s, &r, "to_a"), conveyor(&s, &r, "to_b"));
    assert!(to_b > to_a, "to_a {to_a}, to_b {to_b}");
    // Both queues stay short: nothing ever waits at the source.
    assert_eq!(r.sources[0].blocked, 0.0);
    // Equal (empty) queues: ties go to the output listed first.
    let s = spec(&split_to_servers("{ rule: shortest_queue }", "1 s"));
    let r = run(&s);
    assert_eq!(conveyor(&s, &r, "to_b"), 0);
}

/// Two sources merging into one server, which takes a part every 6 s while
/// each source offers one every 5 s: the server can't keep up with both.
fn merge(input: &str) -> String {
    format!(
        r#"
format: outliers-sim/0.1
name: merge
entities: {{ part: {{}} }}
sources:
  first: {{ creates: part, interarrival: "5 s" }}
  second: {{ creates: part, interarrival: "5 s" }}
servers:
  server: {{ process_time: "6 s", input: {input} }}
conveyors:
  c_first: {{ from: first, to: server, {C} }}
  c_second: {{ from: second, to: server, {C} }}
  out: {{ from: server, to: done, {C} }}
sinks:
  done: {{}}
experiment: {{ replications: 1, run_length: "10 h", seed: 1 }}
"#
    )
}

#[test]
fn priority_merge_starves_the_low_priority_input() {
    let s = spec(&merge("{ rule: priority, order: [c_second, c_first] }"));
    let r = run(&s);
    // `second` alone overloads the server, so `first` is served only until
    // its conveyor is full and then never again.
    assert!(
        conveyor(&s, &r, "c_first") <= 5,
        "c_first {}",
        conveyor(&s, &r, "c_first")
    );
    assert!(r.sources[0].blocked > 0.99);
    conserved(&r);
}

#[test]
fn round_robin_merge_alternates() {
    let s = spec(&merge("{ rule: round_robin }"));
    let r = run(&s);
    let (a, b) = (conveyor(&s, &r, "c_first"), conveyor(&s, &r, "c_second"));
    assert!(a.abs_diff(b) <= 5, "c_first {a}, c_second {b}");
    assert!(r.servers[0].busy > 0.99);
}

#[test]
fn first_come_merge_serves_both_inputs() {
    let s = spec(&merge("{ rule: first_come }"));
    let r = run(&s);
    let (a, b) = (conveyor(&s, &r, "c_first"), conveyor(&s, &r, "c_second"));
    let share = a as f64 / (a + b) as f64;
    assert!((0.45..=0.55).contains(&share), "c_first share {share}");
}

#[test]
fn simultaneous_arrivals_follow_the_rule_not_the_event_order() {
    // Both sources create a part at t = 0 and both reach the server at t = 2 s.
    // `first` is processed first by event order, but priority says `second`.
    let s = spec(&merge("{ rule: priority, order: [c_second, c_first] }"));
    let mut trace = Vec::new();
    simulate(&s, 1, &mut trace);
    let first_start = trace
        .iter()
        .find_map(|(t, e)| match e {
            TraceEvent::StartedProcess { part, .. } => Some((*t, *part)),
            _ => None,
        })
        .unwrap();
    // Part 0 is from `first` (created first), part 1 from `second`.
    assert_eq!(first_start, (SimTime::from_secs_f64(2.0), 1));
}

#[test]
fn first_come_ties_go_to_the_input_listed_first() {
    // Both parts reach the server at t = 2 s. `first`'s events run first,
    // but the tie rule says the conveyor listed first wins: here `c_second`.
    // The decision has to wait until every arrival due at t = 2 s is known.
    let yaml = merge("{ rule: first_come }").replace(
        &format!("  c_first: {{ from: first, to: server, {C} }}\n  c_second: {{ from: second, to: server, {C} }}\n"),
        &format!("  c_second: {{ from: second, to: server, {C} }}\n  c_first: {{ from: first, to: server, {C} }}\n"),
    );
    assert!(
        yaml.contains("c_second: { from: second"),
        "test bug: conveyors not swapped"
    );
    let mut trace = Vec::new();
    simulate(&spec(&yaml), 1, &mut trace);
    let first_start = trace
        .iter()
        .find_map(|(t, e)| match e {
            TraceEvent::StartedProcess { part, .. } => Some((*t, *part)),
            _ => None,
        })
        .unwrap();
    assert_eq!(first_start, (SimTime::from_secs_f64(2.0), 1));
}

#[test]
fn rework_loop_example() {
    let s = spec(TWO_CELLS);
    for replication in 1..=3 {
        let r = simulate(&s, replication, &mut NoTrace);
        conserved(&r);
        let rework = conveyor(&s, &r, "c_rework") as f64;
        let shipped = conveyor(&s, &r, "c_ship") as f64;
        let share = rework / (rework + shipped);
        assert!((share - 0.1).abs() < 0.01, "rework share {share}");
        assert!(r.sinks[0].exited > 6000);
    }
}
