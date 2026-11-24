//! Part types: mixes at sources, process times per type, routing by type.

use simulancer_engine::{NoTrace, ReplicationResult, Spec, simulate};

fn spec(yaml: &str) -> Spec {
    match simulancer_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

fn run(spec: &Spec) -> ReplicationResult {
    simulate(spec, 1, &mut NoTrace)
}

/// Frames and wheels from one source; the mill takes 10 s for a frame and
/// 30 s for a wheel; after it, each type has its own exit.
const MIX: &str = r#"
format: simulancer/0.1
name: mix
entities:
  frame: {}
  wheel: {}
sources:
  arrivals:
    creates: { frame: 0.25, wheel: 0.75 }
    interarrival: "exponential(60 s)"
servers:
  mill:
    process_time: { frame: "10 s", wheel: "30 s" }
    output: { rule: by_type, routes: { frame: c_frames, wheel: c_wheels } }
conveyors:
  c_in:     { from: arrivals, to: mill, length: "50 m", speed: "1 m/s", spacing: "0.5 m" }
  c_frames: { from: mill, to: frames_out, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
  c_wheels: { from: mill, to: wheels_out, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
sinks:
  frames_out: {}
  wheels_out: {}
experiment:
  replications: 1
  run_length: "2000 h"
  seed: 7
"#;

#[test]
fn types_are_mixed_by_their_shares_and_routed_by_type() {
    let s = spec(MIX);
    let r = run(&s);
    let (frames, wheels) = (&r.sinks[0], &r.sinks[1]);
    // Routing by type is exact: each exit only sees its own type.
    assert_eq!(frames.by_type[1].exited, 0);
    assert_eq!(wheels.by_type[0].exited, 0);
    assert_eq!(frames.by_type[0].exited, frames.exited);
    assert_eq!(wheels.by_type[1].exited, wheels.exited);
    // About 120 000 parts: the share of frames is 0.25 within a few standard errors.
    let n = (frames.exited + wheels.exited) as f64;
    let share = frames.exited as f64 / n;
    let se = (0.25 * 0.75 / n).sqrt();
    assert!((share - 0.25).abs() < 4.0 * se, "share of frames {share}");
}

#[test]
fn process_times_differ_by_type() {
    // Utilization = arrival rate x mean process time = (0.25 x 10 + 0.75 x 30) / 60.
    let r = run(&spec(MIX));
    let expected = (0.25 * 10.0 + 0.75 * 30.0) / 60.0;
    assert!(
        (r.servers[0].busy - expected).abs() < 0.01,
        "busy {} vs {expected}",
        r.servers[0].busy
    );
    // Time in system: transit (50 m + 2 m) + process time + queueing; wheels
    // take 20 s longer to process than frames.
    let frame = r.sinks[0].by_type[0].time_in_system.mean();
    let wheel = r.sinks[1].by_type[1].time_in_system.mean();
    assert!(
        (wheel - frame - 20.0).abs() < 3.0,
        "frame {frame}, wheel {wheel}"
    );
}

#[test]
fn a_single_type_draws_nothing_extra() {
    // The same model with one type gives the same arrivals as before part
    // types existed: the type stream is only used for mixes.
    let one = MIX
        .replace("creates: { frame: 0.25, wheel: 0.75 }", "creates: frame")
        .replace(
            "output: { rule: by_type, routes: { frame: c_frames, wheel: c_wheels } }",
            "output: { rule: round_robin }",
        );
    let r = run(&spec(&one));
    assert_eq!(
        r.sinks[0].by_type[1].exited + r.sinks[1].by_type[1].exited,
        0
    );
    // Listing a type nobody creates changes nothing either.
    let only_frames = one.replace("  wheel: {}\n", "").replace(
        r#"{ frame: "10 s", wheel: "30 s" }"#,
        r#"{ frame: "10 s" }"#,
    );
    let r2 = run(&spec(&only_frames));
    assert_eq!(r.sinks[0].exited, r2.sinks[0].exited);
    assert_eq!(r.sinks[1].exited, r2.sinks[1].exited);
    assert_eq!(r.servers[0].busy, r2.servers[0].busy);
}
