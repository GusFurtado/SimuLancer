//! Accumulating and non-accumulating conveyors, on a deterministic line where
//! the answer can be worked out by hand.
//!
//! A part every 3 s gets on a 10 m belt moving at 1 m/s (spacing 1 m), at
//! whose end a server takes 100 s per part. Part 0 gets on at 0 s and reaches
//! the end at 10 s, straight into the server. Part 1 (on at 3 s) reaches the
//! end at 13 s and has to wait for the server until 110 s.
//! - Accumulating: the parts behind it keep moving and queue up behind it;
//!   part 2 waits 1 m from the end and gets there at 111 s.
//! - Non-accumulating: the whole belt stops at 13 s with its gaps of 3 m, so
//!   part 2 is 3 m from the end and only gets there at 113 s.

use simulancer_engine::{SimTime, Spec, TraceEvent, simulate};

fn spec(kind: &str) -> Spec {
    let yaml = format!(
        r#"
format: simulancer/0.1
name: belt
entities:
  part: {{}}
sources:
  arrivals: {{ creates: part, interarrival: "3 s" }}
servers:
  slow: {{ process_time: "100 s" }}
conveyors:
  belt: {{ from: arrivals, to: slow, length: "10 m", speed: "1 m/s", spacing: "1 m", type: {kind} }}
queues:
  out: {{ from: slow, to: done }}
sinks:
  done: {{}}
experiment:
  replications: 1
  run_length: "1 h"
  seed: 1
"#
    );
    match simulancer_model::load_str(&yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", &yaml)),
    }
}

/// When each part started at the server, in seconds.
fn starts(spec: &Spec) -> Vec<f64> {
    let mut trace: Vec<(SimTime, TraceEvent)> = Vec::new();
    simulate(spec, 1, &mut trace);
    trace
        .iter()
        .filter(|(_, e)| matches!(e, TraceEvent::StartedProcess { .. }))
        .map(|(t, _)| t.as_secs_f64())
        .collect()
}

/// When a part reached the end of the belt (conveyor 0), in seconds.
fn reached(spec: &Spec) -> Vec<f64> {
    let mut trace: Vec<(SimTime, TraceEvent)> = Vec::new();
    simulate(spec, 1, &mut trace);
    trace
        .iter()
        .filter(|(_, e)| matches!(e, TraceEvent::ReachedEnd { conveyor: 0, .. }))
        .map(|(t, _)| t.as_secs_f64())
        .collect()
}

#[test]
fn an_accumulating_belt_queues_parts_at_its_end() {
    let s = starts(&spec("accumulating"));
    assert_eq!(&s[..4], &[10.0, 110.0, 210.0, 310.0]);
    assert_eq!(&reached(&spec("accumulating"))[..3], &[10.0, 13.0, 111.0]);
    let r = simulancer_engine::simulate(&spec("accumulating"), 1, &mut simulancer_engine::NoTrace);
    assert_eq!(r.conveyors[0].max_parts, 10);
}

#[test]
fn a_non_accumulating_belt_stops_with_its_gaps() {
    // The server is the bottleneck either way, so parts start at the same
    // times; what differs is where they wait.
    let s = starts(&spec("non_accumulating"));
    assert_eq!(&s[..3], &[10.0, 110.0, 210.0]);
    // Part 1 stops the belt at 13 s with part 2 at 7 m; at 110 s part 1
    // goes in and the belt moves on: part 2 reaches the end 3 s later.
    assert_eq!(
        &reached(&spec("non_accumulating"))[..3],
        &[10.0, 13.0, 113.0]
    );
}
