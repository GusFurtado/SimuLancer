//! Shared resources: servers hold units while processing.

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

/// Two lathes fed in turn, a part every 20 s, 30 s each: without help, both
/// keep up (each gets a part every 40 s). Both need the one operator.
const SHARED: &str = r#"
format: simulancer/0.1
name: shared
entities: { part: {} }
resources:
  operator: { capacity: 1 }
sources:
  arrivals: { creates: part, interarrival: "20 s", when_blocked: drop, output: { rule: round_robin } }
servers:
  lathe_a: { process_time: "30 s", needs: { operator: 1 } }
  lathe_b: { process_time: "30 s", needs: { operator: 1 } }
conveyors:
  c_a:   { from: arrivals, to: lathe_a, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
  c_b:   { from: arrivals, to: lathe_b, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
  c_a_out: { from: lathe_a, to: done, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
  c_b_out: { from: lathe_b, to: done, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
sinks:
  done: {}
experiment: { replications: 1, warmup: "1 h", run_length: "100 h", seed: 3 }
"#;

#[test]
fn one_operator_limits_two_machines() {
    let r = run(&spec(SHARED));
    // One part every 30 s at most: 120 parts per hour, operator always busy.
    let per_hour = r.sinks[0].exited as f64 / 100.0;
    assert!((per_hour - 120.0).abs() < 0.5, "{per_hour} parts/h");
    assert!(
        r.resources[0].busy > 0.999,
        "operator busy {}",
        r.resources[0].busy
    );
    // The lathes share the work, and each spends time waiting for the operator.
    let busy = r.servers[0].busy + r.servers[1].busy;
    assert!((busy - 1.0).abs() < 0.001, "lathes busy {busy} in total");
    assert!(r.servers[0].waiting_for_resources > 0.2);
    assert!(r.servers[1].waiting_for_resources > 0.2);

    // With two operators, each lathe keeps up: 180 parts per hour.
    let two = SHARED.replace("operator: { capacity: 1 }", "operator: { capacity: 2 }");
    let r = run(&spec(&two));
    let per_hour = r.sinks[0].exited as f64 / 100.0;
    assert!((per_hour - 180.0).abs() < 0.5, "{per_hour} parts/h");
    assert!(
        (r.resources[0].busy - 0.75).abs() < 0.01,
        "{}",
        r.resources[0].busy
    );
    assert_eq!(r.servers[0].waiting_for_resources, 0.0);
}

#[test]
fn an_operator_limits_a_multi_slot_server_like_one_slot() {
    // M/M/1 in disguise: three slots, but one operator. Utilization λ/μ = 0.6.
    let yaml = r#"
format: simulancer/0.1
name: mm1
entities: { part: {} }
resources:
  operator: { capacity: 1 }
sources:
  arrivals: { creates: part, interarrival: "exponential(100 s)" }
servers:
  machine: { capacity: 3, process_time: "exponential(60 s)", needs: { operator: 1 } }
conveyors:
  c_in:  { from: arrivals, to: machine, length: "500 m", speed: "1 m/s", spacing: "0.01 m" }
  c_out: { from: machine, to: done, length: "2 m", speed: "1 m/s", spacing: "0.5 m" }
sinks:
  done: {}
experiment: { replications: 1, warmup: "10 h", run_length: "4000 h", seed: 11 }
"#;
    let r = run(&spec(yaml));
    assert!(
        (r.resources[0].busy - 0.6).abs() < 0.01,
        "{}",
        r.resources[0].busy
    );
    // At most one slot works at a time: the server's busy share is a third of that.
    assert!(
        (r.servers[0].busy - 0.2).abs() < 0.005,
        "{}",
        r.servers[0].busy
    );
}

#[test]
fn units_are_given_back_when_processing_ends_not_when_parts_leave() {
    // Lathe A's output is a tiny conveyor into a slow machine: A ends up
    // blocked most of the time, but the operator is free to serve B.
    let yaml = SHARED
        .replace(
            "c_a_out: { from: lathe_a, to: done,",
            "c_a_out: { from: lathe_a, to: slow,",
        )
        .replace(
            "lathe_b: { process_time: \"30 s\", needs: { operator: 1 } }",
            "lathe_b: { process_time: \"30 s\", needs: { operator: 1 } }\n  slow: { process_time: \"600 s\" }",
        )
        .replace(
            "sinks:\n  done: {}",
            "  c_slow: { from: slow, to: done, length: \"2 m\", speed: \"1 m/s\", spacing: \"0.5 m\" }\nsinks:\n  done: {}",
        );
    let r = run(&spec(&yaml));
    assert!(
        r.servers[0].blocked > 0.9,
        "lathe A blocked {}",
        r.servers[0].blocked
    );
    // B gets a part every 40 s and takes 30 s: busy 75%, never short of the operator for long.
    assert!(
        (r.servers[1].busy - 0.75).abs() < 0.02,
        "lathe B busy {}",
        r.servers[1].busy
    );
}

/// The event log says which units a server takes for which part and when
/// they come back, and which slot each part is processed in, so a viewer can
/// follow the operators: never more units out than there are, every unit
/// taken comes back for the same part at the same server, and no two parts
/// share a slot.
#[test]
fn the_log_says_who_holds_the_units_and_which_slot_works() {
    use simulancer_engine::{SimTime, TraceEvent};
    use std::collections::HashMap;
    let two_slots = SHARED
        .replace("operator: { capacity: 1 }", "operator: { capacity: 2 }")
        .replace(r#"interarrival: "20 s""#, r#"interarrival: "10 s""#)
        .replace(
            r#"lathe_a: { process_time: "30 s", needs: { operator: 1 } }"#,
            r#"lathe_a: { capacity: 2, process_time: "30 s", needs: { operator: 1 } }"#,
        );
    let spec = spec(&two_slots);
    let mut log: Vec<(SimTime, TraceEvent)> = Vec::new();
    simulate(&spec, 1, &mut log);
    let mut out = 0u32;
    let mut held: HashMap<u64, (usize, u32)> = HashMap::new();
    let mut slots: HashMap<(usize, usize), u64> = HashMap::new();
    let (mut taken, mut started) = (0, 0);
    for (_, e) in &log {
        match *e {
            TraceEvent::ResourceTaken {
                resource,
                units,
                server,
                part,
            } => {
                assert_eq!(resource, 0);
                out += units;
                assert!(out <= 2, "{out} operators out of 2");
                assert!(held.insert(part, (server, units)).is_none());
                taken += 1;
            }
            TraceEvent::ResourceReturned {
                units,
                server,
                part,
                ..
            } => {
                assert_eq!(held.remove(&part), Some((server, units)));
                out -= units;
            }
            TraceEvent::StartedProcess { part, server, slot } => {
                assert!(slot < spec.servers[server].capacity as usize);
                assert!(held.contains_key(&part), "a part starts with its operator");
                if let Some(other) = slots.insert((server, slot), part) {
                    assert!(
                        !held.contains_key(&other),
                        "slot {slot} of server {server} is taken"
                    );
                }
                started += 1;
            }
            _ => {}
        }
    }
    assert!(
        taken > 1000 && taken == started,
        "{taken} taken, {started} started"
    );
    assert!(
        slots.keys().any(|&(s, slot)| s == 0 && slot == 1),
        "lathe_a uses its second slot"
    );
}
