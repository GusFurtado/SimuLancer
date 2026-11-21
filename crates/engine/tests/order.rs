//! Which part goes next: queue order, input rules by attribute, and
//! priorities for resources.

use outliers_sim_engine::{NoTrace, SimTime, Spec, TraceEvent, simulate};

fn spec(yaml: &str) -> Spec {
    match outliers_sim_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

/// Parts in the order a server started them, with each part's last value
/// of the first attribute (if the model has one).
fn started(spec: &Spec, server: usize) -> Vec<(u64, f64)> {
    let mut trace: Vec<(SimTime, TraceEvent)> = Vec::new();
    simulate(spec, 1, &mut trace);
    let mut value = std::collections::HashMap::new();
    let mut order = Vec::new();
    for (_, e) in trace {
        match e {
            TraceEvent::Set { part, value: v, .. } => {
                value.insert(part, v);
            }
            TraceEvent::StartedProcess { part, server: s } if s == server => {
                order.push((part, value.get(&part).copied().unwrap_or(0.0)));
            }
            _ => {}
        }
    }
    order
}

/// Five parts arrive at once at a busy server, which then takes them one
/// by one; `order` decides which.
fn burst(order: &str) -> String {
    format!(
        r#"
format: outliers-sim/0.1
name: burst
entities:
  part: {{}}
attributes:
  size: {{ default: 0 }}
sources:
  arrivals:
    creates: part
    interarrival: "100 h"
    group: 6
    sets: {{ size: "uniform(1, 100)" }}
servers:
  server: {{ process_time: "1 min" }}
queues:
  line: {{ from: arrivals, to: server{order} }}
  out: {{ from: server, to: done }}
sinks:
  done: {{}}
experiment:
  replications: 1
  run_length: "1 h"
  seed: 5
"#
    )
}

#[test]
fn queues_let_parts_go_in_their_order() {
    // A group arrives at one instant: all six are in line before the
    // server takes the first.
    let ids = |order: &str| -> Vec<u64> {
        started(&spec(&burst(order)), 0)
            .iter()
            .map(|p| p.0)
            .collect()
    };
    assert_eq!(ids(""), [0, 1, 2, 3, 4, 5]);
    assert_eq!(ids(", order: first_in"), [0, 1, 2, 3, 4, 5]);
    assert_eq!(ids(", order: last_in"), [5, 4, 3, 2, 1, 0]);

    let sizes = |order: &str| -> Vec<f64> {
        started(&spec(&burst(order)), 0)
            .iter()
            .map(|p| p.1)
            .collect()
    };
    let lowest = sizes(", order: { lowest: size }");
    assert!(lowest.windows(2).all(|w| w[0] <= w[1]), "{lowest:?}");
    let highest = sizes(", order: { highest: size }");
    assert!(highest.windows(2).all(|w| w[0] >= w[1]), "{highest:?}");
}

#[test]
fn servers_take_the_lowest_or_highest_waiting_part() {
    // Two lines feed one server; each line's front part is compared.
    let yaml = |rule_name: &str| {
        let rule = format!("{{ rule: {rule_name}, by: size }}");
        format!(
            r#"
format: outliers-sim/0.1
name: lines
entities:
  part: {{}}
attributes:
  size: {{ default: 0 }}
sources:
  a: {{ creates: part, interarrival: "100 h", group: 4, sets: {{ size: "uniform(1, 100)" }} }}
  b: {{ creates: part, interarrival: "100 h", group: 4, sets: {{ size: "uniform(1, 100)" }} }}
servers:
  server:
    process_time: "1 min"
    input: {rule}
queues:
  qa: {{ from: a, to: server, order: {{ {rule_name}: size }} }}
  qb: {{ from: b, to: server, order: {{ {rule_name}: size }} }}
  out: {{ from: server, to: done }}
sinks:
  done: {{}}
experiment:
  replications: 1
  run_length: "1 h"
  seed: 9
"#
        )
    };
    // Both lines are sorted the same way, so taking the better of the two
    // fronts each time takes all eight parts in order.
    let sizes =
        |rule: &str| -> Vec<f64> { started(&spec(&yaml(rule)), 0).iter().map(|p| p.1).collect() };
    let lowest = sizes("lowest");
    assert_eq!(lowest.len(), 8);
    assert!(lowest.windows(2).all(|w| w[0] <= w[1]), "{lowest:?}");
    let highest = sizes("highest");
    assert!(highest.windows(2).all(|w| w[0] >= w[1]), "{highest:?}");
}

#[test]
fn resources_serve_their_order_first() {
    // One operator for three machines with the same load; with `order` it
    // goes to `lathe` first whenever several are waiting, so `lathe` waits
    // less for it.
    let yaml = |order: &str| {
        format!(
            r#"
format: outliers-sim/0.1
name: operator
entities:
  part: {{}}
sources:
  a: {{ creates: part, interarrival: "exponential(7.5 min)" }}
  b: {{ creates: part, interarrival: "exponential(7.5 min)" }}
  c: {{ creates: part, interarrival: "exponential(7.5 min)" }}
resources:
  operator: {{ capacity: 1{order} }}
servers:
  mill: {{ process_time: "exponential(2 min)", needs: {{ operator: 1 }} }}
  lathe: {{ process_time: "exponential(2 min)", needs: {{ operator: 1 }} }}
  drill: {{ process_time: "exponential(2 min)", needs: {{ operator: 1 }} }}
queues:
  qa: {{ from: a, to: mill }}
  qb: {{ from: b, to: lathe }}
  qc: {{ from: c, to: drill }}
  oa: {{ from: mill, to: done }}
  ob: {{ from: lathe, to: done }}
  oc: {{ from: drill, to: done }}
sinks:
  done: {{}}
experiment:
  replications: 1
  run_length: "4000 h"
  seed: 4
"#
        )
    };
    let wait = |order: &str| {
        let r = simulate(&spec(&yaml(order)), 1, &mut NoTrace);
        (r.conveyors[0].time.mean(), r.conveyors[1].time.mean())
    };
    let (mill, lathe) = wait("");
    assert!(
        (mill / lathe - 1.0).abs() < 0.1,
        "same load: {mill} vs {lathe}"
    );
    let (mill, lathe) = wait(", order: [lathe]");
    assert!(
        lathe < 0.6 * mill,
        "lathe first: mill {mill}, lathe {lathe}"
    );
}
