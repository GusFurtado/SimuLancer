//! Paced (indexing) lines: stations move their parts on together.

use simulancer_engine::{NoTrace, ReplicationResult, Spec, simulate};

fn spec(yaml: &str) -> Spec {
    match simulancer_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

fn run(yaml: &str) -> ReplicationResult {
    simulate(&spec(yaml), 1, &mut NoTrace)
}

/// Plenty of parts; station a takes 1 minute, b 2; one place between them.
fn model(lines: &str) -> String {
    format!(
        r#"
format: simulancer/0.1
name: paced
entities:
  part: {{}}
sources:
  arrivals: {{ creates: part, interarrival: "10 s" }}
servers:
  a: {{ process_time: "1 min" }}
  b: {{ process_time: "2 min" }}
queues:
  q_in: {{ from: arrivals, to: a, capacity: 5 }}
  q_ab: {{ from: a, to: b, capacity: 1 }}
  q_out: {{ from: b, to: done }}
sinks:
  done: {{}}
{lines}
experiment:
  replications: 1
  warmup: "1 h"
  run_length: "100 h"
  seed: 4
"#
    )
}

#[test]
fn a_slow_takt_sets_the_pace() {
    // Every 3 minutes both stations are done: the line moves 20 times an
    // hour and never stops.
    let r = run(&model(
        "lines:\n  main: { stations: [a, b], takt: \"3 min\" }",
    ));
    let l = &r.lines[0];
    assert_eq!(l.stops, 0);
    assert!((l.moves as f64 - 2000.0).abs() <= 1.0, "{}", l.moves);
    assert!(
        (r.sinks[0].exited as f64 - 2000.0).abs() <= 2.0,
        "{}",
        r.sinks[0].exited
    );
    // Station a waits for the takt with its part done: blocked 2 of 3 minutes.
    assert!(
        (r.servers[0].blocked - 2.0 / 3.0).abs() < 0.01,
        "{}",
        r.servers[0].blocked
    );
}

#[test]
fn a_slow_station_stops_the_line() {
    // Takt 1.5 minutes, but b takes 2: every move waits for b, half a minute
    // past the takt. The line moves every 2 minutes, stopped a quarter of the time.
    let r = run(&model(
        "lines:\n  main: { stations: [a, b], takt: \"1.5 min\" }",
    ));
    let l = &r.lines[0];
    assert!((l.moves as f64 - 3000.0).abs() <= 1.0, "{}", l.moves);
    assert!((l.stops as f64 - l.moves as f64).abs() <= 1.0);
    assert!((l.stopped - 0.25).abs() < 0.01, "{}", l.stopped);
    assert_eq!(l.stopped_by[0], 0, "a is never the one");
    assert!(l.stopped_by[1] as f64 >= l.stops as f64 - 1.0);
}

#[test]
fn line_errors() {
    let errors = |yaml: &str| match simulancer_model::load_str(yaml) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e
            .diagnostics()
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("\n"),
    };
    assert!(
        errors(&model(
            "lines:\n  main: { stations: [a, c], takt: \"1 min\" }"
        ))
        .contains("`c` isn't a server")
    );
    assert!(
        errors(&model("lines:\n  main: { stations: [a], takt: \"1 min\" }"))
            .contains("at least two stations")
    );
    assert!(
        errors(
            &model("lines:\n  main: { stations: [a, b], takt: \"1 min\" }")
                .replace("a: { process_time", "a: { capacity: 2, process_time")
        )
        .contains("one part at a time")
    );
    assert!(errors(&model("lines:\n  one: { stations: [a, b], takt: \"1 min\" }\n  two: { stations: [b, a], takt: \"1 min\" }")).contains("already a station"));
}
