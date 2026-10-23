//! Resources parts carry from one server to another: CONWIP cards, pallets.

use outliers_sim_engine::{NoTrace, SimTime, Spec, TraceEvent, simulate};

fn spec(yaml: &str) -> Spec {
    match outliers_sim_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

/// A three-station line fed faster than it can work; with `cards`, at most
/// that many parts are between the start of `cut` and the end of `pack`.
fn line(cards: &str) -> String {
    let (resource, takes, gives) = if cards.is_empty() {
        (String::new(), "", "")
    } else {
        (
            format!("resources:\n  cards: {{ capacity: {cards} }}\n"),
            ", takes: { cards: 1 }",
            ", gives_back: [cards]",
        )
    };
    format!(
        r#"
format: outliers-sim/0.1
name: conwip
entities:
  part: {{}}
{resource}sources:
  orders: {{ creates: part, interarrival: "exponential(1 min)" }}
servers:
  cut: {{ process_time: "exponential(50 s)"{takes} }}
  weld: {{ process_time: "exponential(55 s)" }}
  pack: {{ process_time: "exponential(50 s)"{gives} }}
queues:
  backlog: {{ from: orders, to: cut }}
  q1: {{ from: cut, to: weld }}
  q2: {{ from: weld, to: pack }}
  out: {{ from: pack, to: done }}
sinks:
  done: {{}}
experiment:
  replications: 1
  run_length: "500 h"
  seed: 11
"#
    )
}

#[test]
fn cards_cap_the_parts_in_the_line() {
    let s = spec(&line("15"));
    let mut trace: Vec<(SimTime, TraceEvent)> = Vec::new();
    let r = simulate(&s, 1, &mut trace);
    // Parts between starting at cut (server 0) and finishing at pack (server 2).
    let (mut wip, mut most) = (0i64, 0i64);
    for (_, e) in &trace {
        match e {
            TraceEvent::StartedProcess { server: 0, .. } => wip += 1,
            TraceEvent::FinishedProcess { server: 2, .. } => wip -= 1,
            _ => {}
        }
        most = most.max(wip);
    }
    assert_eq!(most, 15);
    // The cards in use are the line's work in process.
    assert!(
        r.resources[0].busy > 0.5 && r.resources[0].busy <= 1.0,
        "{}",
        r.resources[0].busy
    );
    // Against the same line without cards (push): as much shipped, with
    // much less waiting inside the line (it waits in the backlog instead).
    let push = simulate(&spec(&line("")), 1, &mut NoTrace);
    let in_line = |r: &outliers_sim_engine::ReplicationResult| {
        r.conveyors[1].mean_parts + r.conveyors[2].mean_parts
    };
    let shipped = |r: &outliers_sim_engine::ReplicationResult| r.sinks[0].exited as f64;
    assert!(
        in_line(&r) < 0.7 * in_line(&push),
        "in line: conwip {} vs push {}",
        in_line(&r),
        in_line(&push)
    );
    assert!(
        shipped(&r) > 0.99 * shipped(&push),
        "shipped: conwip {} vs push {}",
        shipped(&r),
        shipped(&push)
    );
}
