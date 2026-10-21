//! Parts arriving in groups (`group`) and parts split into others (`splits_into`).

use outliers_sim_engine::{NoTrace, ReplicationResult, Spec, simulate};

fn spec(yaml: &str) -> Spec {
    match outliers_sim_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

/// Trucks of pallets; each pallet is broken into cartons.
fn model(group: &str, split: &str, when_blocked: &str, room: &str) -> String {
    format!(
        r#"
format: outliers-sim/0.1
name: groups
entities:
  pallet: {{}}
  carton: {{}}
sources:
  trucks: {{ creates: pallet, interarrival: "1 h", group: {group}, when_blocked: {when_blocked} }}
servers:
  breaker: {{ process_time: "1 min", splits_into: {split} }}
queues:
  dock: {{ from: trucks, to: breaker{room} }}
  out: {{ from: breaker, to: shipped }}
sinks:
  shipped: {{}}
experiment:
  replications: 1
  run_length: "100 h"
  seed: 3
"#
    )
}

fn run(yaml: &str) -> ReplicationResult {
    simulate(&spec(yaml), 1, &mut NoTrace)
}

fn conserved(r: &ReplicationResult) {
    let t = &r.totals;
    assert_eq!(
        t.created + t.assembled + t.split,
        t.exited + t.lost + t.consumed + t.in_system,
        "{t:?}"
    );
}

#[test]
fn a_group_arrives_at_once_and_each_part_splits() {
    // 100 trucks of 4 pallets (the first at time 0), each pallet into 12 cartons.
    let r = run(&model("4", "{ carton: 12 }", "wait", ""));
    conserved(&r);
    assert_eq!(r.sources[0].created, 100 * 4);
    assert_eq!(r.totals.split, 100 * 4 * 12);
    // Every pallet is done within the hour (4 minutes of work a truck).
    assert_eq!(r.sinks[0].exited, 100 * 4 * 12);
    assert_eq!(
        r.sinks[0].by_type[1].exited,
        100 * 4 * 12,
        "only cartons leave"
    );
    // The breaker handles one pallet at a time: of a truck's pallets, the
    // last waits 3 minutes, so the waits are 0, 1, 2 and 3 min.
    assert!(
        (r.conveyors[0].time.mean() - 90.0).abs() < 1e-6,
        "{}",
        r.conveyors[0].time.mean()
    );
}

#[test]
fn group_sizes_and_split_counts_can_vary() {
    let r = run(&model(
        "{ min: 2, max: 6 }",
        "{ carton: { min: 10, max: 14 } }",
        "wait",
        "",
    ));
    conserved(&r);
    let pallets = r.sources[0].created as f64;
    assert!(
        (pallets / 100.0 - 4.0).abs() < 0.5,
        "about 4 pallets a truck: {pallets}"
    );
    let cartons = r.totals.split as f64;
    assert!(
        (cartons / pallets - 12.0).abs() < 0.5,
        "about 12 cartons a pallet: {cartons}"
    );
}

#[test]
fn the_rest_of_a_group_waits_or_is_lost_when_there_is_no_room() {
    // A group's parts all arrive at the same instant, before the breaker
    // takes anything off the dock: with room for 1, 1 gets on and 3 are lost.
    let lost = run(&model("4", "{ carton: 1 }", "drop", ", capacity: 1"));
    conserved(&lost);
    assert_eq!(lost.sources[0].lost, 100 * 3);
    // Waiting instead: the truck's pallets get on in order as room frees up,
    // and the next truck comes an hour after the last pallet got on.
    let waited = run(&model("4", "{ carton: 1 }", "wait", ", capacity: 1"));
    conserved(&waited);
    assert_eq!(waited.sources[0].lost, 0);
    assert!(waited.sources[0].blocked > 0.0);
}

#[test]
fn group_and_split_errors() {
    let errors = |yaml: &str| match outliers_sim_model::load_str(yaml) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e
            .diagnostics()
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("\n"),
    };
    assert!(errors(&model("0", "{ carton: 12 }", "wait", "")).contains("a whole number from 1"));
    assert!(
        errors(&model("{ min: 5, max: 2 }", "{ carton: 12 }", "wait", "")).contains("min ≤ max")
    );
    assert!(
        errors(&model("4", "{ box: 12 }", "wait", "")).contains("isn't listed under `entities`")
    );
    assert!(
        errors(&model("4", "{ carton: 12, pallet: 1 }", "wait", ""))
            .contains("names one part type")
    );
}
