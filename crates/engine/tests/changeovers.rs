//! Changeovers: setup time when a station starts a different part type.

use outliers_sim_engine::{NoTrace, ReplicationResult, Spec, simulate};

fn spec(yaml: &str) -> Spec {
    match outliers_sim_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

fn run(yaml: &str) -> ReplicationResult {
    simulate(&spec(yaml), 1, &mut NoTrace)
}

/// Two models, half each, one every 10 minutes: no queue ever forms.
fn line(changeover: &str) -> String {
    format!(
        r#"
format: outliers-sim/0.1
name: changeovers
entities:
  standard: {{}}
  premium: {{}}
sources:
  release: {{ creates: {{ standard: 0.5, premium: 0.5 }}, interarrival: "10 min" }}
servers:
  station: {{ process_time: "1 min", changeover: {changeover} }}
queues:
  q_in: {{ from: release, to: station }}
  q_out: {{ from: station, to: done }}
sinks:
  done: {{}}
experiment:
  replications: 1
  run_length: "200 h"
  seed: 9
"#
    )
}

#[test]
fn a_change_of_type_adds_the_setup() {
    let r = run(&line("\"2 min\""));
    let s = &r.servers[0];
    let parts = r.sinks[0].exited as f64;
    // About half the parts are a change from the one before.
    let share = s.changeovers as f64 / parts;
    assert!((share - 0.5).abs() < 0.05, "{share}");
    // Every part takes 1 minute, plus 2 for each changeover.
    let expected = 60.0 + 120.0 * s.changeovers as f64 / parts;
    let t = r.sinks[0].time_in_system.mean();
    assert!((t - expected).abs() < 1.0, "{t} vs {expected}");
    // The share of time spent setting up.
    let setup = s.changeovers as f64 * 120.0 / (200.0 * 3600.0);
    assert!(
        (s.changeover - setup).abs() < 1e-3,
        "{} vs {setup}",
        s.changeover
    );
    assert!(s.changeover < s.busy, "setup is part of busy");
}

#[test]
fn changeovers_can_differ_by_direction() {
    // Only standard -> premium takes a setup: about a quarter of parts.
    let r = run(&line("{ standard->premium: \"8 min\" }"));
    let s = &r.servers[0];
    let share = s.changeovers as f64 / r.sinks[0].exited as f64;
    assert!((share - 0.25).abs() < 0.04, "{share}");
    assert!((s.changeover - s.changeovers as f64 * 480.0 / (200.0 * 3600.0)).abs() < 1e-3);
}

#[test]
fn no_changeover_when_the_type_stays_the_same() {
    let r = run(&line("\"2 min\"").replace("{ standard: 0.5, premium: 0.5 }", "standard"));
    assert_eq!(r.servers[0].changeovers, 0);
    assert!((r.sinks[0].time_in_system.mean() - 60.0).abs() < 1e-6);
}

#[test]
fn changeover_errors() {
    let errors = |yaml: &str| match outliers_sim_model::load_str(yaml) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e
            .diagnostics()
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("\n"),
    };
    assert!(
        errors(&line("{ standard-premium: \"8 min\" }")).contains("isn't a change of part type")
    );
    assert!(
        errors(&line("{ standard->deluxe: \"8 min\" }")).contains("isn't listed under `entities`")
    );
    assert!(errors(&line("{ standard->standard: \"8 min\" }")).contains("same part type"));
}
