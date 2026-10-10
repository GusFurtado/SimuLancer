//! Assembly (kits of components make a new part) and batches.

use outliers_sim_engine::{NoTrace, ReplicationResult, Spec, simulate};

fn spec(yaml: &str) -> Spec {
    match outliers_sim_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

fn run(spec: &Spec) -> ReplicationResult {
    simulate(spec, 1, &mut NoTrace)
}

/// Every part is accounted for: made by a source or an assembly, and then
/// exited, lost, used up as a component, or still in the system.
fn assert_conserved(r: &ReplicationResult) {
    let t = &r.totals;
    assert_eq!(
        t.created + t.assembled,
        t.exited + t.lost + t.consumed + t.in_system,
        "{t:?}"
    );
}

/// A frame a minute, a wheel every 30 s; a bike is a frame and two wheels.
const BIKES: &str = r#"
format: outliers-sim/0.1
name: bikes
entities: { frame: {}, wheel: {}, bike: {} }
sources:
  frames: { creates: frame, interarrival: "60 s" }
  wheels: { creates: wheel, interarrival: "30 s" }
servers:
  assembly:
    assembles: { frame: 1, wheel: 2 }
    makes: bike
    process_time: "20 s"
    input: { rule: first_come }
conveyors:
  c_frames: { from: frames, to: assembly, length: "10 m", speed: "1 m/s", spacing: "1 m" }
  c_wheels: { from: wheels, to: assembly, length: "10 m", speed: "1 m/s", spacing: "1 m" }
  c_bikes:  { from: assembly, to: shipping, length: "2 m", speed: "1 m/s", spacing: "1 m" }
sinks:
  shipping: {}
experiment: { replications: 1, warmup: "1 h", run_length: "10 h", seed: 5 }
"#;

#[test]
fn kits_become_new_parts() {
    let r = run(&spec(BIKES));
    assert_conserved(&r);
    assert_eq!(r.totals.consumed, 3 * r.totals.assembled);
    // 60 bikes an hour, all of type bike.
    assert_eq!(r.sinks[0].exited, 600);
    assert_eq!(r.sinks[0].by_type[2].exited, 600);
    // A bike's time in system starts with its earliest component: the frame
    // and first wheel are made at the same moment, and wait 10 s on their
    // conveyors; the second wheel comes 30 s later and travels 10 s: the
    // kit is complete 40 s after the frame was made. Then 20 s of work and
    // 2 s to shipping.
    let t = r.sinks[0].time_in_system.mean();
    assert!((t - 62.0).abs() < 1e-6, "{t}");
}

#[test]
fn the_scarcest_component_sets_the_pace() {
    // A wheel a minute: only 30 bikes an hour; frames back up.
    let slow = BIKES.replace("interarrival: \"30 s\"", "interarrival: \"60 s\"");
    let r = run(&spec(&slow));
    assert_conserved(&r);
    assert_eq!(r.sinks[0].exited, 300);
    assert!(
        r.sources[0].blocked > 0.4,
        "frames held back {}",
        r.sources[0].blocked
    );
}

/// A part a minute into an oven that bakes 10 at a time for 5 minutes.
const OVEN: &str = r#"
format: outliers-sim/0.1
name: oven
entities: { part: {} }
resources:
  baker: { capacity: 1 }
sources:
  arrivals: { creates: part, interarrival: "60 s" }
servers:
  oven: { batch: 10, process_time: "5 min", needs: { baker: 1 } }
conveyors:
  c_in:  { from: arrivals, to: oven, length: "1 m", speed: "1 m/s", spacing: "0.5 m" }
  c_out: { from: oven, to: done, length: "1 m", speed: "1 m/s", spacing: "0.05 m" }
sinks:
  done: {}
experiment: { replications: 1, run_length: "10 h", seed: 1 }
"#;

#[test]
fn batches_are_processed_together() {
    let s = spec(OVEN);
    assert_eq!(s.servers[0].capacity, 10, "one batch at a time by default");
    let r = run(&s);
    assert_conserved(&r);
    // A batch every 10 minutes: 60 parts an hour (the last batch of the run
    // is still baking or collecting).
    assert!(
        (590..=600).contains(&r.sinks[0].exited),
        "{}",
        r.sinks[0].exited
    );
    // 10 slots busy for 5 of every 10 minutes.
    assert!(
        (r.servers[0].busy - 0.5).abs() < 0.01,
        "{}",
        r.servers[0].busy
    );
    // The baker is held once per batch, not once per part.
    assert!(
        (r.resources[0].busy - 0.5).abs() < 0.01,
        "{}",
        r.resources[0].busy
    );
    // Waiting for the batch to fill: 9, 8, ..., 0 minutes, 4.5 on average,
    // plus 5 minutes of baking and 1 s on each conveyor (parts leave one by
    // one 0.05 s apart).
    let t = r.sinks[0].time_in_system.mean();
    assert!((t - (4.5 * 60.0 + 300.0 + 2.0)).abs() < 1.0, "{t}");
}
