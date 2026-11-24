//! Closing the doors but serving who's inside: `finish_work` on a schedule.

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

/// Customers come 09:00-17:00 at about the rate two desks in a row can
/// serve, so there's a line at closing; staff work the same hours.
fn bank(finish_work: bool, needs: &str) -> String {
    format!(
        r#"
format: simulancer/0.1
name: closing
entities:
  customer: {{}}
schedules:
  doors: {{ shifts: ["09:00-17:00"] }}
  staff: {{ shifts: ["09:00-17:00"], finish_work: {finish_work} }}
resources:
  clerk: {{ capacity: 1, schedule: staff }}
sources:
  door: {{ creates: customer, interarrival: "exponential(5 min)", schedule: doors }}
servers:
  desk: {{ process_time: "exponential(4.5 min)", schedule: staff{needs} }}
  cashier: {{ process_time: "exponential(4.5 min)", schedule: staff }}
queues:
  line: {{ from: door, to: desk }}
  next: {{ from: desk, to: cashier }}
  out: {{ from: cashier, to: exit }}
sinks:
  exit: {{}}
experiment:
  replications: 1
  run_length: "28 d"
  seed: 11
"#
    )
}

#[test]
fn everyone_inside_is_served_before_the_staff_go_home() {
    let r = run(&bank(true, ""));
    // Nobody waits overnight: the longest stay is a few hours at most.
    let longest = r.sinks[0].time_in_system.max;
    assert!(
        longest < 4.0 * 3600.0,
        "longest stay {} h",
        longest / 3600.0
    );
    // The cashier works past 17:00 more than the desk: it waits for the
    // desk's last customers too.
    let (desk, cashier) = (r.servers[0].overtime, r.servers[1].overtime);
    assert!(
        desk > 0.0 && cashier > desk,
        "desk {desk}, cashier {cashier}"
    );
    // Overtime is time on shift, not off: states still add up.
    let s = &r.servers[1];
    assert!((s.busy + s.blocked + s.failed + s.idle - 1.0).abs() < 1e-9);
}

#[test]
fn without_finish_work_the_line_waits_for_the_morning() {
    let r = run(&bank(false, ""));
    let longest = r.sinks[0].time_in_system.max;
    assert!(
        longest > 14.0 * 3600.0,
        "longest stay {} h",
        longest / 3600.0
    );
    assert_eq!(r.servers[0].overtime, 0.0);
}

#[test]
fn a_resource_on_the_schedule_keeps_working_too() {
    let r = run(&bank(true, ", needs: { clerk: 1 }"));
    let longest = r.sinks[0].time_in_system.max;
    assert!(
        longest < 4.0 * 3600.0,
        "longest stay {} h",
        longest / 3600.0
    );
    assert!(r.resources[0].overtime > 0.0);
}
