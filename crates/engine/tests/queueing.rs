//! Validation against queueing theory: models with exact analytical answers.
//!
//! Conveyors are made practically invisible (1 ms transit, room for 10,000
//! parts, 0.1 µs to close up a queue) so each line behaves like a textbook
//! queue. Each test runs independent replications and requires the exact
//! answer to fall within 4 standard errors of the estimate, and the estimate
//! to be precise enough (relative standard error below a bound) for that to
//! mean something. Seeds are fixed, so results don't change between runs.

use outliers_sim_engine::{NoTrace, ReplicationResult, Spec, simulate};

const REPLICATIONS: u32 = 20;

/// A conveyor that adds 1 ms and never blocks (unless `capacity` says so).
fn conveyor(name: &str, from: &str, to: &str) -> String {
    format!(
        "  {name}: {{ from: {from}, to: {to}, length: \"1 m\", speed: \"1000 m/s\", spacing: \"0.1 mm\" }}\n"
    )
}

/// A single-server line: arrivals -> server -> sink.
fn single_queue(interarrival: &str, server: &str, extra_conveyor_in: Option<&str>) -> String {
    let c_in = match extra_conveyor_in {
        Some(custom) => custom.to_string(),
        None => conveyor("c_in", "arrivals", "server"),
    };
    format!(
        r#"
format: outliers-sim/0.1
name: queue
entities:
  part: {{}}
sources:
  arrivals: {{ creates: part, interarrival: "{interarrival}" }}
servers:
  server:
{server}
conveyors:
{c_in}{c_out}sinks:
  done: {{}}
experiment:
  replications: {REPLICATIONS}
  warmup: "50 h"
  run_length: "1000 h"
  seed: 20261009
"#,
        c_out = conveyor("c_out", "server", "done"),
    )
}

/// Seconds spent on conveyors by every part (2 × 1 ms).
const TRANSIT: f64 = 0.002;

fn spec(yaml: &str) -> Spec {
    match outliers_sim_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

fn replications(spec: &Spec) -> Vec<ReplicationResult> {
    (1..=REPLICATIONS)
        .map(|r| simulate(spec, r, &mut NoTrace))
        .collect()
}

/// Checks a measure against its exact value across replications.
fn check(
    results: &[ReplicationResult],
    what: &str,
    exact: f64,
    max_relative_se: f64,
    measure: impl Fn(&ReplicationResult) -> f64,
) {
    let values: Vec<f64> = results.iter().map(measure).collect();
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let sd = (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
    let se = sd / n.sqrt();
    println!(
        "{what}: exact {exact:.4}, estimate {mean:.4} ± {:.4} (95%), {:+.2} SE",
        1.96 * se,
        (mean - exact) / se
    );
    assert!(
        se / exact.abs() <= max_relative_se,
        "{what}: too imprecise to validate (relative SE {:.4})",
        se / exact.abs()
    );
    assert!(
        (mean - exact).abs() <= 4.0 * se,
        "{what}: exact {exact}, estimate {mean} (SE {se}) is {:.1} SE off",
        (mean - exact) / se
    );
}

fn time_in_system(r: &ReplicationResult) -> f64 {
    r.sinks[0].time_in_system.mean()
}

/// Erlang C: probability an arrival waits in an M/M/c queue.
fn erlang_c(c: u32, offered_load: f64) -> f64 {
    let rho = offered_load / f64::from(c);
    let mut term = 1.0; // a^k / k!
    let mut sum = 0.0;
    for k in 0..c {
        sum += term;
        term *= offered_load / f64::from(k + 1);
    }
    let last = term / (1.0 - rho);
    last / (sum + last)
}

/// Pollaczek-Khinchine: mean time in system of an M/G/1 queue.
fn mg1_time_in_system(lambda: f64, mean: f64, second_moment: f64) -> f64 {
    let rho = lambda * mean;
    mean + lambda * second_moment / (2.0 * (1.0 - rho))
}

#[test]
fn mm1() {
    // λ = 1/min, mean service 42 s: ρ = 0.7, W = 1 / (μ - λ) = 140 s.
    let (lambda, mean) = (1.0 / 60.0, 42.0);
    let s = spec(&single_queue(
        "exponential(1 min)",
        "    process_time: \"exponential(42 s)\"",
        None,
    ));
    let results = replications(&s);
    let w = 1.0 / (1.0 / mean - lambda);
    check(
        &results,
        "M/M/1 time in system",
        w + TRANSIT,
        0.02,
        time_in_system,
    );
    check(&results, "M/M/1 utilization", lambda * mean, 0.01, |r| {
        r.servers[0].busy
    });
}

#[test]
fn mmc() {
    // Three servers, mean service 2.1 min: offered load 2.1, ρ = 0.7.
    let (lambda, mean, c) = (1.0 / 60.0, 126.0, 3);
    let s = spec(&single_queue(
        "exponential(1 min)",
        "    capacity: 3\n    process_time: \"exponential(126 s)\"",
        None,
    ));
    let results = replications(&s);
    let load = lambda * mean;
    let wq = erlang_c(c, load) / (f64::from(c) / mean - lambda);
    check(
        &results,
        "M/M/3 time in system",
        wq + mean + TRANSIT,
        0.02,
        time_in_system,
    );
    check(
        &results,
        "M/M/3 utilization",
        load / f64::from(c),
        0.01,
        |r| r.servers[0].busy,
    );
}

#[test]
fn md1() {
    let (lambda, mean) = (1.0 / 60.0, 42.0);
    let s = spec(&single_queue(
        "exponential(1 min)",
        "    process_time: \"42 s\"",
        None,
    ));
    let w = mg1_time_in_system(lambda, mean, mean * mean);
    check(
        &replications(&s),
        "M/D/1 time in system",
        w + TRANSIT,
        0.02,
        time_in_system,
    );
}

#[test]
fn mg1_triangular() {
    // triangular(20, 40, 66) s: mean 42 s.
    let (a, b, c) = (20.0, 40.0, 66.0);
    let mean: f64 = (a + b + c) / 3.0;
    let variance = (a * a + b * b + c * c - a * b - a * c - b * c) / 18.0;
    let s = spec(&single_queue(
        "exponential(1 min)",
        "    process_time: \"triangular(20 s, 40 s, 66 s)\"",
        None,
    ));
    let w = mg1_time_in_system(1.0 / 60.0, mean, variance + mean * mean);
    check(
        &replications(&s),
        "M/G/1 (triangular) time in system",
        w + TRANSIT,
        0.02,
        time_in_system,
    );
}

#[test]
fn mg1_lognormal() {
    let (mean, sd) = (42.0, 30.0);
    let s = spec(&single_queue(
        "exponential(1 min)",
        "    process_time: \"lognormal(mean: 42 s, sd: 30 s)\"",
        None,
    ));
    let w = mg1_time_in_system(1.0 / 60.0, mean, sd * sd + mean * mean);
    check(
        &replications(&s),
        "M/G/1 (lognormal) time in system",
        w + TRANSIT,
        0.02,
        time_in_system,
    );
}

#[test]
fn mg1_gamma() {
    // gamma(shape 2, scale 21 s): mean 42 s, variance 2 × 21².
    let (shape, scale) = (2.0, 21.0);
    let (mean, variance) = (shape * scale, shape * scale * scale);
    let s = spec(&single_queue(
        "exponential(1 min)",
        "    process_time: \"gamma(2, 21 s)\"",
        None,
    ));
    let w = mg1_time_in_system(1.0 / 60.0, mean, variance + mean * mean);
    check(
        &replications(&s),
        "M/G/1 (gamma) time in system",
        w + TRANSIT,
        0.02,
        time_in_system,
    );
}

#[test]
fn mg1_weibull() {
    // weibull(shape 2, scale 50 s): mean 50 Γ(1.5) = 25√π,
    // variance 50² (Γ(2) − Γ(1.5)²) = 2500 (1 − π/4).
    let pi = std::f64::consts::PI;
    let (mean, variance) = (25.0 * pi.sqrt(), 2500.0 * (1.0 - pi / 4.0));
    let s = spec(&single_queue(
        "exponential(1 min)",
        "    process_time: \"weibull(2, 50 s)\"",
        None,
    ));
    let results = replications(&s);
    let w = mg1_time_in_system(1.0 / 60.0, mean, variance + mean * mean);
    check(
        &results,
        "M/G/1 (Weibull) time in system",
        w + TRANSIT,
        0.02,
        time_in_system,
    );
    check(
        &results,
        "M/G/1 (Weibull) utilization",
        mean / 60.0,
        0.01,
        |r| r.servers[0].busy,
    );
}

#[test]
fn mg1_with_breakdowns() {
    // Exponential failures on busy time with preemptive resume: each part's
    // completion time is C = S + the repairs during S. Failures arrive at
    // rate f during service, so C is i.i.d. and the queue is M/G/1 in C:
    //   E[C]   = E[S] (1 + f E[R])
    //   Var[C] = Var[S] (1 + f E[R])² + E[S] f E[R²]
    let lambda = 1.0 / 60.0;
    let (s_mean, s_var) = (36.0, 36.0 * 36.0); // exponential(36 s)
    let f: f64 = 1.0 / 600.0; // uptime exponential(10 min)
    let (r_mean, r_second) = (60.0, 300.0 + 3600.0); // uniform(30 s, 90 s)
    let c_mean = s_mean * (1.0 + f * r_mean);
    let c_var = s_var * (1.0 + f * r_mean).powi(2) + s_mean * f * r_second;
    let s = spec(&single_queue(
        "exponential(1 min)",
        "    process_time: \"exponential(36 s)\"\n    failures:\n      - { name: breakdown, uptime: \"exponential(10 min)\", downtime: \"uniform(30 s, 90 s)\" }",
        None,
    ));
    let results = replications(&s);
    let w = mg1_time_in_system(lambda, c_mean, c_var + c_mean * c_mean);
    check(
        &results,
        "M/G/1 with breakdowns time in system",
        w + TRANSIT,
        0.02,
        time_in_system,
    );
    // Down for E[R] per failure, f E[S] failures per part, λ parts per second.
    check(
        &results,
        "failed share",
        lambda * f * s_mean * r_mean,
        0.02,
        |r| r.servers[0].failed,
    );
    check(&results, "busy share", lambda * s_mean, 0.01, |r| {
        r.servers[0].busy
    });
}

#[test]
fn tandem_mm1_queues() {
    // Burke's theorem: the output of an M/M/1 queue is Poisson, so two in
    // series are independent M/M/1 queues and the times add up.
    let lambda = 1.0 / 60.0;
    let yaml = format!(
        r#"
format: outliers-sim/0.1
name: tandem
entities:
  part: {{}}
sources:
  arrivals: {{ creates: part, interarrival: "exponential(1 min)" }}
servers:
  first: {{ process_time: "exponential(36 s)" }}
  second: {{ process_time: "exponential(42 s)" }}
conveyors:
{}{}{}sinks:
  done: {{}}
experiment:
  replications: {REPLICATIONS}
  warmup: "50 h"
  run_length: "1000 h"
  seed: 20261009
"#,
        conveyor("c1", "arrivals", "first"),
        conveyor("c2", "first", "second"),
        conveyor("c3", "second", "done"),
    );
    let w = 1.0 / (1.0 / 36.0 - lambda) + 1.0 / (1.0 / 42.0 - lambda);
    check(
        &replications(&spec(&yaml)),
        "tandem M/M/1 time in system",
        w + 1.5 * TRANSIT,
        0.02,
        time_in_system,
    );
}

#[test]
fn mm1k_with_dropped_arrivals() {
    // Room for 4 on the conveyor + 1 in the server: M/M/1/5. Dropped
    // arrivals are the ones that find the system full (PASTA).
    let (lambda, mean, k) = (1.0 / 60.0, 42.0, 5);
    let rho: f64 = lambda * mean;
    let small_conveyor = "  c_in: { from: arrivals, to: server, length: \"4 mm\", speed: \"1000 m/s\", spacing: \"1 mm\" }\n";
    let yaml = single_queue(
        "exponential(1 min)",
        "    process_time: \"exponential(42 s)\"",
        Some(small_conveyor),
    )
    .replace(
        "interarrival: \"exponential(1 min)\" }",
        "interarrival: \"exponential(1 min)\", when_blocked: drop }",
    );
    let results = replications(&spec(&yaml));

    let p_full = (1.0 - rho) * rho.powi(k) / (1.0 - rho.powi(k + 1));
    check(
        &results,
        "M/M/1/5 share of arrivals lost",
        p_full,
        0.02,
        |r| r.sources[0].lost as f64 / r.sources[0].created as f64,
    );

    let in_system =
        rho / (1.0 - rho) - f64::from(k as u32 + 1) * rho.powi(k + 1) / (1.0 - rho.powi(k + 1));
    let w = in_system / (lambda * (1.0 - p_full));
    // The tiny conveyor adds 4 µs instead of 1 ms.
    check(
        &results,
        "M/M/1/5 time in system",
        w + 0.001 + 0.000_004,
        0.02,
        time_in_system,
    );
}

#[test]
fn poisson_split_into_two_mm1_queues() {
    // A random split of a Poisson stream gives independent Poisson streams:
    // two M/M/1 queues, λ₁ = 0.4/min (mean service 1.5 min, ρ = 0.6) and
    // λ₂ = 0.6/min (mean service 1.2 min, ρ = 0.72). W = 1 / (μ − λ).
    let yaml = format!(
        r#"
format: outliers-sim/0.1
name: split
entities: {{ part: {{}} }}
sources:
  arrivals:
    creates: part
    interarrival: "exponential(1 min)"
    output: {{ rule: probability, weights: {{ c1: 0.4, c2: 0.6 }} }}
servers:
  s1: {{ process_time: "exponential(1.5 min)" }}
  s2: {{ process_time: "exponential(1.2 min)" }}
conveyors:
{}{}{}{}sinks:
  done1: {{}}
  done2: {{}}
experiment:
  replications: {REPLICATIONS}
  warmup: "50 h"
  run_length: "1000 h"
  seed: 20261009
"#,
        conveyor("c1", "arrivals", "s1"),
        conveyor("c2", "arrivals", "s2"),
        conveyor("o1", "s1", "done1"),
        conveyor("o2", "s2", "done2"),
    );
    let results = replications(&spec(&yaml));
    let w = |lambda: f64, mean: f64| 1.0 / (1.0 / mean - lambda);
    check(
        &results,
        "split M/M/1 #1 time in system",
        w(0.4 / 60.0, 90.0) + TRANSIT,
        0.02,
        |r| r.sinks[0].time_in_system.mean(),
    );
    check(
        &results,
        "split M/M/1 #2 time in system",
        w(0.6 / 60.0, 72.0) + TRANSIT,
        0.02,
        |r| r.sinks[1].time_in_system.mean(),
    );
}

#[test]
fn mm1_with_bernoulli_feedback() {
    // After service a part goes back to the same queue with probability
    // p = 0.2. Arrivals at the server: λ/(1 − p) = 1.25/min; mean service
    // 36 s (μ = 1.667/min, ρ = 0.75). With exponential service the mean
    // number in system doesn't depend on the order parts are taken, so by
    // Little's law T = 1 / ((1 − p)(μ − λ/(1 − p))) = 3 min.
    let yaml = format!(
        r#"
format: outliers-sim/0.1
name: feedback
entities: {{ part: {{}} }}
sources:
  arrivals: {{ creates: part, interarrival: "exponential(1 min)" }}
servers:
  server:
    process_time: "exponential(36 s)"
    input: {{ rule: first_come }}
    output: {{ rule: probability, weights: {{ out: 0.8, back: 0.2 }} }}
conveyors:
{}{}{}sinks:
  done: {{}}
experiment:
  replications: {REPLICATIONS}
  warmup: "50 h"
  run_length: "1000 h"
  seed: 20261009
"#,
        conveyor("c_in", "arrivals", "server"),
        conveyor("back", "server", "server"),
        conveyor("out", "server", "done"),
    );
    let results = replications(&spec(&yaml));
    let (lambda, p, mean) = (1.0 / 60.0, 0.2, 36.0);
    let t = 1.0 / ((1.0 - p) * (1.0 / mean - lambda / (1.0 - p)));
    // Conveyors: c_in and out once, back 0.25 times on average (1 ms each).
    let transit = 0.001 * (2.0 + p / (1.0 - p));
    let results_ref = &results;
    check(
        results_ref,
        "M/M/1 with feedback time in system",
        t + transit,
        0.02,
        time_in_system,
    );
    check(
        results_ref,
        "M/M/1 with feedback utilization",
        lambda / (1.0 - p) * mean,
        0.01,
        |r| r.servers[0].busy,
    );
}

/// A single-server line built with queues instead of conveyors: no transit
/// time at all, so the results are the textbook ones exactly.
fn queue_line(
    interarrival: &str,
    process: &str,
    capacity: Option<u32>,
    when_blocked: &str,
) -> String {
    let capacity = capacity.map_or(String::new(), |c| format!(", capacity: {c}"));
    format!(
        r#"
format: outliers-sim/0.1
name: queues
entities:
  part: {{}}
sources:
  arrivals: {{ creates: part, interarrival: "{interarrival}", when_blocked: {when_blocked} }}
servers:
  server: {{ process_time: "{process}" }}
queues:
  line: {{ from: arrivals, to: server{capacity}, target_wait: "5 min" }}
  out: {{ from: server, to: done }}
sinks:
  done: {{}}
experiment:
  replications: {REPLICATIONS}
  warmup: "50 h"
  run_length: "1000 h"
  seed: 20261021
"#
    )
}

#[test]
fn mm1_with_a_queue_waits_as_the_formula_says() {
    // λ = 1/min, μ = 1.25/min: ρ = 0.8, Wq = ρ / (μ − λ) = 3.2 min, W = 4 min.
    let results = replications(&spec(&queue_line(
        "exponential(1 min)",
        "exponential(0.8 min)",
        None,
        "wait",
    )));
    check(&results, "M/M/1 wait in queue (s)", 192.0, 0.03, |r| {
        r.conveyors[0].time.mean()
    });
    check(
        &results,
        "M/M/1 time in system (s)",
        240.0,
        0.03,
        time_in_system,
    );
    // The outgoing queue is never waited in: the sink takes parts at once.
    assert!(results.iter().all(|r| r.conveyors[1].time.max == 0.0));
    // The wait is 0 with probability 1 − ρ, otherwise exponential with rate
    // μ − λ: P(Wq ≤ t) = 1 − ρ e^(−(μ−λ)t), and its 90th percentile is
    // ln(ρ / 0.1) / (μ − λ).
    let within = 1.0 - 0.8 * (-0.25_f64 * 5.0).exp();
    check(
        &results,
        "M/M/1 share waiting at most 5 min",
        within,
        0.01,
        |r| r.conveyors[0].within_target as f64 / r.conveyors[0].time.count as f64,
    );
    let p90 = (0.8_f64 / 0.1).ln() / 0.25 * 60.0;
    check(&results, "M/M/1 90th percentile wait (s)", p90, 0.03, |r| {
        r.conveyors[0].times.as_ref().unwrap().quantile(0.9)
    });
}

#[test]
fn mm1k_with_a_finite_queue_loses_as_the_formula_says() {
    // Room for 2 in the queue plus 1 in service: K = 3. With ρ = 0.8 an
    // arrival is lost with probability (1 − ρ) ρ^K / (1 − ρ^(K+1)).
    let (rho, k) = (0.8_f64, 3);
    let exact = (1.0 - rho) * rho.powi(k) / (1.0 - rho.powi(k + 1));
    let results = replications(&spec(&queue_line(
        "exponential(1 min)",
        "exponential(0.8 min)",
        Some(2),
        "drop",
    )));
    check(&results, "M/M/1/3 loss probability", exact, 0.02, |r| {
        r.sources[0].lost as f64 / r.sources[0].created as f64
    });
    assert!(results.iter().all(|r| r.conveyors[0].max_parts <= 2));
}

#[test]
fn mm1_with_two_priority_classes_waits_as_cobham_says() {
    // Non-preemptive priority, two classes with the same exponential service
    // (mean 1 min) and arrival rates 0.4/min each: ρ1 = ρ2 = 0.4. With
    // W0 = Σ λi E[Si²] / 2 = 0.8 min, class k waits
    // W0 / ((1 − σ_{k−1})(1 − σ_k)), σ_k = ρ1 + … + ρk (Cobham, 1954):
    // urgent 0.8 / 0.6 = 1.333 min, normal 0.8 / (0.6 × 0.2) = 6.667 min.
    let yaml = format!(
        r#"
format: outliers-sim/0.1
name: priorities
entities:
  urgent: {{}}
  normal: {{}}
sources:
  arrivals: {{ creates: {{ urgent: 0.5, normal: 0.5 }}, interarrival: "exponential(1.25 min)" }}
servers:
  server: {{ process_time: "exponential(1 min)" }}
queues:
  line: {{ from: arrivals, to: server, order: {{ by_type: [urgent, normal] }} }}
  out: {{ from: server, to: done }}
sinks:
  done: {{}}
experiment:
  replications: {REPLICATIONS}
  warmup: "50 h"
  run_length: "1000 h"
  seed: 20261023
"#
    );
    let results = replications(&spec(&yaml));
    check(&results, "urgent wait (s)", 80.0, 0.03, |r| {
        r.conveyors[0].time_by_type[0].mean()
    });
    check(&results, "normal wait (s)", 400.0, 0.03, |r| {
        r.conveyors[0].time_by_type[1].mean()
    });
}
