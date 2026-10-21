//! Arrival rates that change with the time of day (`arrivals_per_hour`).

use outliers_sim_engine::{Observer, SimTime, Spec, TraceEvent, simulate};

fn spec(yaml: &str) -> Spec {
    match outliers_sim_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

/// Arrivals straight to a sink: when each part was created.
fn model(arrivals: &str, schedule: &str) -> String {
    format!(
        r#"
format: outliers-sim/0.1
name: profile
entities:
  part: {{}}
schedules:
  weekdays: {{ shifts: ["00:00-24:00"], days: [mon, tue, wed, thu, fri] }}
sources:
  door: {{ creates: part, {arrivals}{schedule} }}
queues:
  q: {{ from: door, to: out }}
sinks:
  out: {{}}
experiment:
  replications: 1
  run_length: "28 d"
  seed: 7
"#
    )
}

#[derive(Default)]
struct Arrivals(Vec<SimTime>);

impl Observer for Arrivals {
    fn record(&mut self, time: SimTime, event: TraceEvent) {
        if let TraceEvent::Created { .. } = event {
            self.0.push(time);
        }
    }
}

fn arrival_hours(spec: &Spec) -> Vec<f64> {
    let mut seen = Arrivals::default();
    for r in 1..=5 {
        simulate(spec, r, &mut seen);
    }
    seen.0.iter().map(|t| t.as_secs_f64() / 3600.0).collect()
}

#[test]
fn arrivals_follow_the_rate_of_each_period() {
    // 60 an hour 08:00-10:00, 120 an hour 10:00-12:00, none otherwise: 360 a day.
    let s = spec(&model(
        r#"arrivals_per_hour: { "08:00-10:00": 60, "10:00-12:00": 120 }"#,
        "",
    ));
    let hours = arrival_hours(&s);
    let of_day: Vec<f64> = hours.iter().map(|h| h.rem_euclid(24.0)).collect();
    assert!(
        of_day.iter().all(|&h| (8.0..12.0).contains(&h)),
        "only while the rate is above zero"
    );
    let first = of_day.iter().filter(|&&h| h < 10.0).count() as f64;
    let second = of_day.len() as f64 - first;
    // 5 replications × 28 days: 16,800 expected in the first period, 33,600
    // in the second. A Poisson count's sd is its square root (~130 and ~183).
    let expected = 5.0 * 28.0 * 120.0;
    assert!(
        (first - expected).abs() < 4.0 * expected.sqrt(),
        "{first} vs {expected}"
    );
    assert!(
        (second - 2.0 * expected).abs() < 4.0 * (2.0 * expected).sqrt(),
        "{second} vs {}",
        2.0 * expected
    );
}

#[test]
fn a_profile_combines_with_a_schedule() {
    // The rate repeats every day, and the schedule switches the source off at weekends.
    let s = spec(&model(
        r#"arrivals_per_hour: { "22:00-02:00": 30 }"#,
        ", schedule: weekdays",
    ));
    let hours = arrival_hours(&s);
    for h in &hours {
        let of_day = h.rem_euclid(24.0);
        assert!(of_day >= 22.0 || of_day < 2.0, "{h}: outside the period");
        let day = (h.rem_euclid(168.0) / 24.0) as u32; // 0 = Monday
        assert!(day < 5, "{h}: on a weekend");
    }
    // On weekdays both ends of the period count (00:00-02:00 and
    // 22:00-24:00): 4 h × 30/h × 5 days = 600 a week, for 4 weeks × 5
    // replications.
    let expected = 600.0 * 4.0 * 5.0;
    let n = hours.len() as f64;
    assert!(
        (n - expected).abs() < 4.0 * expected.sqrt(),
        "{n} vs {expected}"
    );
}

#[test]
fn profile_errors() {
    let errors = |arrivals: &str| match outliers_sim_model::load_str(&model(arrivals, "")) {
        Ok(_) => panic!("{arrivals}: expected an error"),
        Err(e) => e
            .diagnostics()
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("\n"),
    };
    assert!(
        errors(r#"arrivals_per_hour: { "08:00-10:00": 10, "09:00-11:00": 5 }"#)
            .contains("overlaps")
    );
    assert!(errors(r#"arrivals_per_hour: { "8 to 10": 10 }"#).contains("isn't a time of day"));
    assert!(
        errors(r#"arrivals_per_hour: { "08:00-10:00": 0 }"#).contains("never has any arrivals")
    );
    assert!(
        errors(r#"interarrival: "1 min", arrivals_per_hour: { "08:00-10:00": 5 }"#)
            .contains("keep one")
    );
    assert!(errors("when_blocked: wait").contains("needs `interarrival`"));
}
