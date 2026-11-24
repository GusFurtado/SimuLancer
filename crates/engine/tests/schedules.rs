//! Schedules: weekly working times for sources, servers and resources.
//! Time 0 is Monday 00:00.

use simulancer_engine::spec::Schedule;
use simulancer_engine::{NoTrace, ReplicationResult, SimTime, Spec, simulate};

fn spec(yaml: &str) -> Spec {
    match simulancer_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

fn run(spec: &Spec) -> ReplicationResult {
    simulate(spec, 1, &mut NoTrace)
}

const H: i64 = 3600 * 1_000_000;

fn h(hours: i64) -> SimTime {
    SimTime::from_micros(hours * H)
}

#[test]
fn weekly_periods_and_changes() {
    // 22:00-06:00 every day: wraps past midnight, and Sunday's wraps past the week.
    let schedule = spec(&LINE.replace(
        "schedules: {}",
        "schedules:\n  nights: { shifts: [\"22:00-06:00\"] }",
    ))
    .schedules
    .remove(0);
    // Monday 00:00-06:00 comes from Sunday night's shift wrapping around.
    assert_eq!(schedule.periods[0], (h(0), h(6)));
    assert_eq!(schedule.periods.last(), Some(&(h(166), h(168))));
    assert!(schedule.is_on(h(0)));
    assert!(!schedule.is_on(h(6)));
    assert!(schedule.is_on(h(23)));
    assert_eq!(schedule.next_change(h(0)), Some(h(6)));
    assert_eq!(schedule.next_change(h(6)), Some(h(22)));
    // Sunday 22:00 to Monday 06:00 is one shift: no change at the week's end.
    assert_eq!(schedule.next_change(h(166)), Some(h(168 + 6)));
    assert!(schedule.is_on(h(168 + 2)));

    let always = Schedule {
        name: "always".into(),
        periods: vec![(SimTime::ZERO, Schedule::WEEK)],
        finish_work: false,
    };
    assert_eq!(always.next_change(h(5)), None);
}

/// A part every 10 min into a 5 min machine that drains to a sink.
const LINE: &str = r#"
format: simulancer/0.1
name: line
entities: { part: {} }
schedules: {}
sources:
  arrivals: { creates: part, interarrival: "10 min" }
servers:
  machine: { process_time: "5 min" }
conveyors:
  c_in:  { from: arrivals, to: machine, length: "1000 m", speed: "1 m/s", spacing: "0.01 m" }
  c_out: { from: machine, to: done, length: "1 m", speed: "1 m/s", spacing: "0.5 m" }
sinks:
  done: {}
experiment: { replications: 1, run_length: "28 d", seed: 1 }
"#;

fn with_schedule(schedule: &str, on: &[(&str, &str)]) -> Spec {
    let mut yaml = LINE.replace("schedules: {}", &format!("schedules:\n  s: {schedule}"));
    for (from, to) in on {
        assert!(yaml.contains(from), "test bug: {from}");
        yaml = yaml.replacen(from, to, 1);
    }
    spec(&yaml)
}

#[test]
fn sources_only_create_during_their_shifts() {
    // 06:00-14:00 every day: arrivals at 06:00, 06:10, ..., 13:50 = 48 a day.
    // The shift's end at 14:00 comes before the 14:00 arrival (it was
    // scheduled first), so that one doesn't happen.
    let s = with_schedule(
        "{ shifts: [\"06:00-14:00\"] }",
        &[(
            "interarrival: \"10 min\" }",
            "interarrival: \"10 min\", schedule: s }",
        )],
    );
    let r = run(&s);
    assert_eq!(r.sources[0].created, 48 * 28);
}

#[test]
fn servers_pause_off_shift_and_resume() {
    // The machine works 08:00-16:00 on weekdays: 40 of 168 hours.
    let s = with_schedule(
        "{ shifts: [\"08:00-16:00\"], days: [mon, tue, wed, thu, fri] }",
        &[(
            "machine: { process_time: \"5 min\" }",
            "machine: { process_time: \"5 min\", schedule: s }",
        )],
    );
    let r = run(&s);
    let m = &r.servers[0];
    assert!(
        (m.off_shift - 128.0 / 168.0).abs() < 1e-9,
        "off shift {}",
        m.off_shift
    );
    // 6 parts an hour arrive around the clock, but the machine only does 12
    // an hour for 40 hours a week: it's always busy on shift, finishing
    // 12 x 40 x 4 = 1920 parts in four weeks (less the one cut by the end).
    let done = r.sinks[0].exited;
    assert!((1915..=1920).contains(&done), "{done}");
    // It works flat out on shift: states are shares of its working time,
    // so they add up to 1 without the time off shift, and it's busy for
    // practically all of it.
    let states = m.busy + m.blocked + m.failed + m.idle;
    assert!((states - 1.0).abs() < 1e-9);
    assert!(m.busy > 0.999, "busy {}", m.busy);
}

#[test]
fn a_part_interrupted_by_the_shift_end_finishes_in_the_next_shift() {
    // One part a day at 15:58 (the source starts at 15:58 and creates every
    // 24 h); the machine takes 5 min and stops at 16:00: the part spends the
    // night there and finishes at 08:03 the next morning.
    let s = with_schedule(
        "{ shifts: [\"08:00-16:00\"] }",
        &[
            (
                "interarrival: \"10 min\" }",
                "interarrival: \"24 h\", schedule: from_1558 }",
            ),
            (
                "schedules:\n  s:",
                "schedules:\n  from_1558: { shifts: [\"15:58-15:59\"] }\n  s:",
            ),
            (
                "machine: { process_time: \"5 min\" }",
                "machine: { process_time: \"5 min\", schedule: s }",
            ),
            (
                "length: \"1000 m\", speed: \"1 m/s\", spacing: \"0.01 m\"",
                "length: \"1 m\", speed: \"60 m/s\", spacing: \"0.5 m\"",
            ),
            (
                "length: \"1 m\", speed: \"1 m/s\", spacing: \"0.5 m\" }\nsinks",
                "length: \"1 m\", speed: \"60 m/s\", spacing: \"0.5 m\" }\nsinks",
            ),
        ],
    );
    let r = run(&s);
    // Created 15:58, done 08:03 + 1 s on conveyors (2 x 1 m at 60 m/s):
    // 16 h 5 min 0.033 s in the system.
    let t = r.sinks[0].time_in_system.mean();
    let expected = 16.0 * 3600.0 + 5.0 * 60.0 + 2.0 / 60.0;
    assert!((t - expected).abs() < 1e-3, "{t} vs {expected}");
}

#[test]
fn resources_off_shift_hand_out_nothing() {
    // The machine needs an operator who works 08:00-16:00 every day.
    let s = with_schedule(
        "{ shifts: [\"08:00-16:00\"] }",
        &[
            (
                "schedules:",
                "resources:\n  operator: { capacity: 1, schedule: s }\nschedules:",
            ),
            (
                "machine: { process_time: \"5 min\" }",
                "machine: { process_time: \"5 min\", needs: { operator: 1 } }",
            ),
        ],
    );
    let r = run(&s);
    assert!((r.resources[0].off_shift - 16.0 / 24.0).abs() < 1e-9);
    // Without its operator the machine is idle (waiting for resources) at night.
    assert!(r.servers[0].waiting_for_resources > 0.6);
    assert_eq!(r.servers[0].off_shift, 0.0);
    // Parts pile up overnight, so on shift the operator never stops: busy
    // is a share of the operator's shift, not of the whole day.
    assert!(r.resources[0].busy > 0.99, "busy {}", r.resources[0].busy);
}

#[test]
fn on_time_counts_working_time_in_any_window() {
    use simulancer_engine::SimTime;
    use simulancer_engine::spec::Schedule;
    let h = |hours: i64| SimTime::from_micros(hours * 3600 * 1_000_000);
    // Weekdays 06:00-14:00: 8 h a day, 40 h a week.
    let s = Schedule {
        name: "day".into(),
        periods: (0..5).map(|d| (h(24 * d + 6), h(24 * d + 14))).collect(),
        finish_work: false,
    };
    assert_eq!(s.on_time(h(0), h(168)), h(40));
    assert_eq!(s.on_time(h(0), h(4 * 168)), h(160));
    // Windows that cut shifts and weeks: Monday 10:00 to the next Tuesday 08:00.
    assert_eq!(s.on_time(h(10), h(168 + 24 + 8)), h(4 + 32 + 8 + 2));
    assert_eq!(s.on_time(h(15), h(30)), h(0));
}
