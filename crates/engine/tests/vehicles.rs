//! Vehicles carrying parts along transports.

use outliers_sim_engine::{NoTrace, ReplicationResult, Spec, simulate};

fn spec(yaml: &str) -> Spec {
    match outliers_sim_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

fn run(yaml: &str) -> ReplicationResult {
    simulate(&spec(yaml), 1, &mut NoTrace)
}

/// Parts made at a press are carried 60 m to storage by forklifts.
fn model(count: u32, interarrival: &str, capacity: &str) -> String {
    format!(
        r#"
format: outliers-sim/0.1
name: vehicles
entities:
  part: {{}}
vehicles:
  forklifts: {{ count: {count}, speed: "1 m/s", load_time: "10 s", unload_time: "10 s", home: press }}
sources:
  arrivals: {{ creates: part, interarrival: "{interarrival}" }}
servers:
  press: {{ process_time: "0 s" }}
queues:
  q_in: {{ from: arrivals, to: press }}
transports:
  trip: {{ from: press, to: storage, by: forklifts, distance: "60 m"{capacity} }}
sinks:
  storage: {{}}
experiment:
  replications: 1
  run_length: "100 h"
  seed: 2
"#
    )
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
fn a_vehicle_fetches_loads_carries_and_unloads() {
    let r = run(&model(1, "10 min", ""));
    conserved(&r);
    // The first part finds the forklift at the press: 10 s to load, 60 s to
    // drive, 10 s to unload. Every later part waits 60 s for it to come back.
    let parts = r.sinks[0].exited as f64;
    let expected = (80.0 + (parts - 1.0) * 140.0) / parts;
    let t = r.sinks[0].time_in_system.mean();
    assert!((t - expected).abs() < 1e-6, "{t} vs {expected}");
    assert!((r.conveyors[1].pickup_wait.mean() - 60.0 * (parts - 1.0) / parts).abs() < 1e-6);
    // 140 s of every 600 busy, 60 of them empty.
    let f = &r.vehicles[0];
    assert!((f.busy - 140.0 / 600.0).abs() < 1e-3, "busy {}", f.busy);
    assert!((f.empty - 60.0 / 600.0).abs() < 1e-3, "empty {}", f.empty);
    assert_eq!(f.trips, r.sinks[0].exited);
}

#[test]
fn more_vehicles_wait_less() {
    // A part a minute: one forklift (140 s a trip) can't keep up; three can.
    let one = run(&model(1, "1 min", ""));
    let three = run(&model(3, "1 min", ""));
    conserved(&one);
    conserved(&three);
    assert!(one.vehicles[0].busy > 0.99);
    assert!(
        one.conveyors[1].pickup_wait.mean() > 3600.0,
        "an ever longer wait"
    );
    assert!(
        three.conveyors[1].pickup_wait.mean() < 120.0,
        "{}",
        three.conveyors[1].pickup_wait.mean()
    );
    assert!(
        (three.vehicles[0].busy - 140.0 / 180.0).abs() < 0.02,
        "{}",
        three.vehicles[0].busy
    );
}

#[test]
fn a_full_transport_blocks_the_object_feeding_it() {
    // Room for 2 on the transport (waiting or carried): the press blocks.
    let r = run(&model(1, "1 min", ", capacity: 2"));
    conserved(&r);
    assert!(r.servers[0].blocked > 0.3, "{}", r.servers[0].blocked);
    assert!(r.conveyors[1].max_parts <= 2);
}

#[test]
fn vehicle_errors() {
    let errors = |yaml: &str| match outliers_sim_model::load_str(yaml) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e
            .diagnostics()
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("\n"),
    };
    assert!(
        errors(&model(1, "1 min", "").replace("by: forklifts", "by: carts"))
            .contains("there are no vehicles `carts`")
    );
    assert!(errors(&model(0, "1 min", "")).contains("count of at least 1"));
    assert!(
        errors(&model(1, "1 min", "").replace("home: press", "home: q_in"))
            .contains("`home` is a source, server or sink")
    );
    assert!(
        errors(&model(1, "1 min", "").replace("distance: \"60 m\"", "distance: \"60 s\""))
            .contains("a length is expected")
    );
}
