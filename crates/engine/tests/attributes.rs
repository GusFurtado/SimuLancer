//! Attributes: values parts carry, set by sources and servers with formulas.

use simulancer_engine::{NoTrace, SimTime, Spec, TraceEvent, simulate};

fn spec(yaml: &str) -> Spec {
    match simulancer_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

/// Every attribute set, as (time in s, part, attribute name, value).
fn sets(spec: &Spec) -> Vec<(f64, u64, String, f64)> {
    let mut trace: Vec<(SimTime, TraceEvent)> = Vec::new();
    simulate(spec, 1, &mut trace);
    trace
        .into_iter()
        .filter_map(|(t, e)| match e {
            TraceEvent::Set {
                part,
                attribute,
                value,
            } => Some((
                t.as_secs_f64(),
                part,
                spec.attributes[attribute].name.clone(),
                value,
            )),
            _ => None,
        })
        .collect()
}

/// Orders get a due date and a size at arrival; two inspections each count
/// a pass in `checks`.
const ORDERS: &str = r#"
format: simulancer/0.1
name: orders
entities:
  order: {}
attributes:
  due: { default: "0 s" }
  size: { default: 0 }
  checks: { default: 0 }
sources:
  orders:
    creates: order
    interarrival: "exponential(5 min)"
    sets: { due: "now + 2 h", size: "uniform(1, 10)" }
servers:
  first:
    process_time: "exponential(2 min)"
    sets: { checks: "checks + 1" }
  second:
    process_time: "exponential(2 min)"
    sets: { checks: "checks + 1" }
queues:
  q1: { from: orders, to: first }
  q2: { from: first, to: second }
  q3: { from: second, to: done }
sinks:
  done: {}
experiment:
  replications: 1
  run_length: "48 h"
  seed: 3
"#;

#[test]
fn sources_and_servers_set_attributes_with_formulas() {
    let s = spec(ORDERS);
    let all = sets(&s);
    let due: Vec<_> = all.iter().filter(|x| x.2 == "due").collect();
    assert!(due.len() > 400, "{} orders", due.len());
    for (t, _, _, value) in &due {
        assert!((value - (t + 7200.0)).abs() < 1e-6, "due {value} at {t}");
    }
    for (_, _, _, size) in all.iter().filter(|x| x.2 == "size") {
        assert!((1.0..=10.0).contains(size), "size {size}");
    }
    // Each server adds one: the second inspection of a part always sets 2.
    let checks: Vec<_> = all.iter().filter(|x| x.2 == "checks").collect();
    for part in due.iter().map(|x| x.1).take(100) {
        let values: Vec<f64> = checks.iter().filter(|x| x.1 == part).map(|x| x.3).collect();
        assert!(values.is_empty() || values[0] == 1.0, "{values:?}");
        assert!(values.len() < 2 || values[1] == 2.0, "{values:?}");
    }
}

#[test]
fn sets_draw_from_their_own_streams() {
    // Adding attributes changes no other draw: the same parts flow the same way.
    let plain = ORDERS
        .replace(
            r#"    sets: { due: "now + 2 h", size: "uniform(1, 10)" }"#,
            "",
        )
        .replace(r#"    sets: { checks: "checks + 1" }"#, "");
    let (a, b) = (
        simulate(&spec(ORDERS), 1, &mut NoTrace),
        simulate(&spec(&plain), 1, &mut NoTrace),
    );
    assert_eq!(a.sinks[0].exited, b.sinks[0].exited);
    assert_eq!(
        a.sinks[0].time_in_system.mean(),
        b.sinks[0].time_in_system.mean()
    );
}

#[test]
fn split_and_assembled_parts_keep_attributes() {
    // A tray of 3 is split into cups; the cups are packed with a lid; the
    // pack keeps the cup's attributes (the cup keeps the clock).
    let s = spec(
        r#"
format: simulancer/0.1
name: inherit
entities:
  tray: {}
  cup: {}
  lid: {}
  pack: {}
attributes:
  batch: { default: 0 }
  seen: { default: 0 }
sources:
  trays:
    creates: tray
    interarrival: "10 min"
    sets: { batch: "now / 1 min" }
  lids:
    creates: lid
    interarrival: "1 min"
servers:
  split:
    process_time: "1 min"
    splits_into: { cup: 3 }
  pack:
    process_time: "10 s"
    assembles: { cup: 1, lid: 1 }
    input: { rule: first_come }
    makes: pack
    keeps_clock_of: cup
  check:
    process_time: "1 s"
    sets: { seen: "batch" }
queues:
  q1: { from: trays, to: split }
  q2: { from: split, to: pack }
  q3: { from: lids, to: pack }
  q4: { from: pack, to: check }
  q5: { from: check, to: done }
sinks:
  done: {}
experiment:
  replications: 1
  run_length: "2 h"
  seed: 1
"#,
    );
    let all = sets(&s);
    let batches: Vec<f64> = all.iter().filter(|x| x.2 == "batch").map(|x| x.3).collect();
    let seen: Vec<f64> = all.iter().filter(|x| x.2 == "seen").map(|x| x.3).collect();
    assert_eq!(batches[..3], [0.0, 10.0, 20.0]);
    // Three packs per tray, each with its tray's batch.
    assert_eq!(seen[..6], [0.0, 0.0, 0.0, 10.0, 10.0, 10.0]);
}

#[test]
fn routes_by_condition_send_parts_by_their_attributes() {
    // Each inspection fails half the parts, which go back through repair;
    // a part's third inspection scraps it. So 1/4 of parts are scrapped.
    let s = spec(
        r#"
format: simulancer/0.1
name: rework
entities:
  part: {}
attributes:
  checks: { default: 0 }
sources:
  arrivals: { creates: part, interarrival: "exponential(10 min)" }
servers:
  inspect:
    process_time: "1 min"
    sets: { checks: "checks + 1" }
    input: { rule: first_come }
    output:
      rule: by_condition
      routes:
        - { when: "checks >= 3", to: q_scrap }
        - { when: "uniform(0, 1) < 0.5", to: q_repair }
        - { to: q_ok }
  repair: { process_time: "2 min" }
queues:
  q_in: { from: arrivals, to: inspect }
  q_repair: { from: inspect, to: repair }
  q_back: { from: repair, to: inspect }
  q_scrap: { from: inspect, to: scrap }
  q_ok: { from: inspect, to: ok }
sinks:
  scrap: {}
  ok: {}
experiment:
  replications: 1
  run_length: "4000 h"
  seed: 2
"#,
    );
    let r = simulate(&s, 1, &mut NoTrace);
    let (scrap, ok) = (r.sinks[0].exited as f64, r.sinks[1].exited as f64);
    let share = scrap / (scrap + ok);
    let se = (0.25 * 0.75 / (scrap + ok)).sqrt();
    assert!((share - 0.25).abs() < 4.0 * se, "scrapped {share}");
    // Scrapped parts were repaired twice; of the good ones, a third (those
    // that failed once) were repaired once.
    let repairs = r.servers[1].processed as f64;
    let expected = 2.0 * scrap + ok / 3.0;
    assert!(
        (repairs - expected).abs() < 0.05 * expected,
        "repairs {repairs} vs {expected}"
    );
}
