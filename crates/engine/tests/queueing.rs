//! Validation against queueing theory: models with exact analytical answers.
//!
//! Conveyors are made practically invisible (1 ms transit, room for 10,000
//! parts, 0.1 µs to close up a queue) so each line behaves like a textbook
//! queue. Each test runs independent replications and requires the exact
//! answer to fall within 4 standard errors of the estimate, and the estimate
//! to be precise enough (relative standard error below a bound) for that to
//! mean something. Seeds are fixed, so results don't change between runs.

use simulancer_engine::{NoTrace, ReplicationResult, Spec, simulate};

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
format: simulancer/0.1
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
    match simulancer_model::load_str(yaml) {
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
fn mg1_empirical() {
    // empirical(20, 30, 45, 50, 70) s: piecewise linear, each of the 4 gaps
    // a quarter likely. Over a gap [a, b], E[S] = (a + b) / 2 and
    // E[S²] = (a² + ab + b²) / 3.
    let points = [20.0_f64, 30.0, 45.0, 50.0, 70.0];
    let gaps = (points.len() - 1) as f64;
    let mean = points.windows(2).map(|w| (w[0] + w[1]) / 2.0).sum::<f64>() / gaps;
    let second = points
        .windows(2)
        .map(|w| (w[0] * w[0] + w[0] * w[1] + w[1] * w[1]) / 3.0)
        .sum::<f64>()
        / gaps;
    let s = spec(&single_queue(
        "exponential(1 min)",
        "    process_time: \"empirical(45 s, 20 s, 70 s, 30 s, 50 s)\"",
        None,
    ));
    let w = mg1_time_in_system(1.0 / 60.0, mean, second);
    check(
        &replications(&s),
        "M/G/1 (empirical) time in system",
        w + TRANSIT,
        0.02,
        time_in_system,
    );
}

#[test]
fn mg1_discrete() {
    // 20 s, 45 s or 80 s with 30%, 50%, 20%: mean 44.5 s, E[S²] 2412.5 s².
    let s = spec(&single_queue(
        "exponential(1 min)",
        "    process_time: \"discrete(20 s: 30%, 45 s: 50%, 80 s: 20%)\"",
        None,
    ));
    let w = mg1_time_in_system(1.0 / 60.0, 44.5, 2412.5);
    check(
        &replications(&s),
        "M/G/1 (discrete) time in system",
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
format: simulancer/0.1
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
format: simulancer/0.1
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
format: simulancer/0.1
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
format: simulancer/0.1
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
format: simulancer/0.1
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

#[test]
fn mm1_with_impatient_customers_gives_up_as_erlang_a_says() {
    // Erlang-A with one server (M/M/1+M): arrivals λ = 1/min, service μ =
    // 1/min, each waiting customer gives up at rate θ = 1/(5 min). With n in
    // the system, departures happen at μ + (n − 1)θ, so the stationary
    // distribution is a product of λ / (μ + (k − 1)θ). The share that gives
    // up is Σ πn (n − 1)θ / λ; the mean number waiting is Σ πn (n − 1).
    let (lambda, mu, theta) = (1.0_f64, 1.0_f64, 0.2_f64);
    let mut p = vec![1.0_f64];
    for n in 1..400 {
        let prev = p[n - 1];
        p.push(prev * lambda / (mu + (n as f64 - 1.0) * theta));
    }
    let total: f64 = p.iter().sum();
    let waiting: f64 = p
        .iter()
        .enumerate()
        .skip(1)
        .map(|(n, x)| x / total * (n as f64 - 1.0))
        .sum();
    let gave_up = waiting * theta / lambda;
    let yaml = format!(
        r#"
format: simulancer/0.1
name: impatient
entities:
  customer: {{}}
sources:
  arrivals: {{ creates: customer, interarrival: "exponential(1 min)" }}
servers:
  server: {{ process_time: "exponential(1 min)" }}
queues:
  line: {{ from: arrivals, to: server, patience: "exponential(5 min)" }}
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
    check(&results, "M/M/1+M share giving up", gave_up, 0.01, |r| {
        let c = &r.conveyors[0];
        c.gave_up as f64 / (c.gave_up + c.time.count) as f64
    });
    check(&results, "M/M/1+M mean waiting", waiting, 0.02, |r| {
        r.conveyors[0].mean_parts
    });
}

/// Stationary distribution of a continuous-time Markov chain from its rates
/// (`rates[i][j]`, i ≠ j), by Gaussian elimination on πQ = 0 with Σπ = 1.
fn stationary(rates: &[Vec<f64>]) -> Vec<f64> {
    let n = rates.len();
    // Rows of the system Qᵀ π = 0, the last one replaced by Σπ = 1.
    let mut a: Vec<Vec<f64>> = (0..n)
        .map(|j| {
            (0..n)
                .map(|i| {
                    if i == j {
                        -rates[i].iter().sum::<f64>()
                    } else {
                        rates[i][j]
                    }
                })
                .chain([0.0])
                .collect()
        })
        .collect();
    a[n - 1] = vec![1.0; n + 1];
    for col in 0..n {
        let pivot = (col..n)
            .max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs()))
            .unwrap();
        a.swap(col, pivot);
        let pivot_row = a[col].clone();
        for (row, values) in a.iter_mut().enumerate() {
            if row != col {
                let f = values[col] / pivot_row[col];
                for (v, p) in values.iter_mut().zip(&pivot_row).skip(col) {
                    *v -= f * p;
                }
            }
        }
    }
    (0..n).map(|i| a[i][n] / a[i][i]).collect()
}

#[test]
fn two_machine_line_with_a_finite_buffer_blocks_as_the_markov_chain_says() {
    // The textbook two-machine line (Buzacott & Shanthikumar): the
    // first machine never runs out of work, the second never runs out of
    // room, and between them a buffer holds B parts. A part finished on the
    // first machine while the buffer is full stays on it (blocking after
    // service). Exponential times, μ1 = 1/min, μ2 = 1/1.1 min, B = 2.
    //
    // State: k = parts in the buffer plus the one on the second machine
    // (0..=B+1) with the first machine working, or "blocked" (k = B+1 and a
    // finished part waiting on the first machine).
    let (mu1, mu2, b) = (1.0_f64, 1.0 / 1.1, 2_usize);
    let blocked = b + 2;
    let mut rates = vec![vec![0.0; b + 3]; b + 3];
    for k in 0..=b + 1 {
        if k <= b {
            rates[k][k + 1] = mu1;
        } else {
            rates[k][blocked] = mu1;
        }
        if k >= 1 {
            rates[k][k - 1] = mu2;
        }
    }
    // The second machine finishes; the blocked part moves into the buffer.
    rates[blocked][b + 1] = mu2;
    let p = stationary(&rates);
    // The chain is a birth–death one (blocked is level B+2), so p_i ∝ ρ^i with
    // ρ = μ1/μ2: a check on the solver.
    let rho = mu1 / mu2;
    let p0 = (1.0 - rho) / (1.0 - rho.powi(b as i32 + 3));
    assert!((p[0] - p0).abs() < 1e-12 && (p[blocked] - p0 * rho.powi(b as i32 + 2)).abs() < 1e-12);
    let second_busy = 1.0 - p[0];
    let per_hour = mu2 * second_busy * 60.0;
    let in_buffer: f64 =
        (1..=b + 1).map(|k| p[k] * (k - 1) as f64).sum::<f64>() + p[blocked] * b as f64;

    // A source with a short queue in front keeps the first machine supplied.
    let yaml = format!(
        r#"
format: simulancer/0.1
name: two-machine-line
entities:
  part: {{}}
sources:
  arrivals: {{ creates: part, interarrival: "1 ms", when_blocked: wait }}
servers:
  first: {{ process_time: "exponential(1 min)" }}
  second: {{ process_time: "exponential(1.1 min)" }}
queues:
  feed: {{ from: arrivals, to: first, capacity: 3 }}
  buffer: {{ from: first, to: second, capacity: {b} }}
  out: {{ from: second, to: done }}
sinks:
  done: {{}}
experiment:
  replications: {REPLICATIONS}
  warmup: "50 h"
  run_length: "1000 h"
  seed: 20261124
"#
    );
    let results = replications(&spec(&yaml));
    check(
        &results,
        "two-machine line throughput (/h)",
        per_hour,
        0.01,
        |r| r.sinks[0].exited as f64 / (r.measured / 3600.0),
    );
    check(&results, "first machine blocked", p[blocked], 0.02, |r| {
        r.servers[0].blocked
    });
    check(&results, "second machine starved", p[0], 0.02, |r| {
        r.servers[1].idle
    });
    check(&results, "parts in the buffer", in_buffer, 0.01, |r| {
        r.conveyors[1].mean_parts
    });
    // The first machine is never starved: the source keeps it supplied.
    assert!(results.iter().all(|r| r.servers[0].idle < 1e-4));
}

#[test]
fn base_stock_with_waiting_orders_behaves_as_erlang_c() {
    // One-for-one replenishment: the supplier orders a part each time the
    // stock position (stock + on order) falls below S = 5, with exponential
    // lead times (mean 3 min). Orders from customers arrive at 1/min and wait
    // for stock. The position drops when a part is used, so each of the S
    // units is either on the shelf or being replaced, for an exponential
    // time: the units are the servers of an M/M/S queue with offered load
    // a = λL = 3 (Palm). Hence the stock is empty with probability
    // C = Erlang C(S, a), a parts are on order and S − a on the shelf.
    //
    // The picking station takes a part as soon as its kit needs one, so with
    // no order waiting it already holds the part for the next order (and the
    // stock position has reordered it). An order then waits only when another
    // is already waiting: the orders waiting are the M/M/S queue minus one,
    // C r² / (1 − r) on average with r = a/S, and by Little's law an order
    // waits that over λ.
    let (s, a) = (5_u32, 3.0_f64);
    let c = erlang_c(s, a);
    let r = a / f64::from(s);
    let wait = c * r * r / (1.0 - r) * 60.0;
    let yaml = format!(
        r#"
format: simulancer/0.1
name: base-stock
entities:
  part: {{}}
  order: {{}}
  shipped: {{}}
sources:
  supplier: {{ creates: part, restocks: stock, reorder_point: {}, order: 1, lead_time: "exponential(3 min)" }}
  customers: {{ creates: order, interarrival: "exponential(1 min)" }}
servers:
  pick: {{ assembles: {{ order: 1, part: 1 }}, makes: shipped, keeps_clock_of: order, process_time: "0 s", input: {{ rule: first_come }} }}
queues:
  stock: {{ from: supplier, to: pick, starts_with: {s} }}
  backlog: {{ from: customers, to: pick }}
  out: {{ from: pick, to: done }}
sinks:
  done: {{}}
experiment:
  replications: {REPLICATIONS}
  warmup: "50 h"
  run_length: "1000 h"
  seed: 20261124
"#,
        s - 1
    );
    let results = replications(&spec(&yaml));
    check(&results, "base stock: out of stock", c, 0.02, |r| {
        r.conveyors[0].empty
    });
    check(
        &results,
        "base stock: order wait (s)",
        wait,
        0.03,
        time_in_system,
    );
    check(&results, "base stock: parts on order", a, 0.01, |r| {
        r.sources[0].on_order
    });
    check(
        &results,
        "base stock: parts on the shelf",
        f64::from(s) - a,
        0.01,
        |r| r.conveyors[0].mean_parts,
    );
}

#[test]
fn arrivals_in_groups_wait_as_the_batch_formula_says() {
    // M^X/M/1 with groups of k = 4 arriving at 1 per 5 min and service
    // exponential with mean 1 min: ρ = 0.8. A group waits like an M/G/1
    // customer whose service is Erlang-k (Pollaczek-Khinchine):
    // λ k(k+1) / (2μ²(1 − ρ)); a part then waits for the (k − 1)/2 ahead of
    // it in its group on average, then its own service. That gives
    // W = (k + 1) / (2μ(1 − ρ)) = 12.5 min, i.e. L = ρ(k + 1) / (2(1 − ρ)) = 10.
    let (k, lambda, mu) = (4.0_f64, 1.0 / 300.0, 1.0 / 60.0);
    let rho = lambda * k / mu;
    let exact = (k + 1.0) / (2.0 * mu * (1.0 - rho));
    let yaml = format!(
        r#"
format: simulancer/0.1
name: groups
entities:
  part: {{}}
sources:
  arrivals: {{ creates: part, interarrival: "exponential(5 min)", group: 4 }}
servers:
  server: {{ process_time: "exponential(1 min)" }}
queues:
  line: {{ from: arrivals, to: server }}
  out: {{ from: server, to: done }}
sinks:
  done: {{}}
experiment:
  replications: {REPLICATIONS}
  warmup: "50 h"
  run_length: "1000 h"
  seed: 20261124
"#
    );
    let results = replications(&spec(&yaml));
    assert!((exact - 750.0).abs() < 1e-9);
    check(
        &results,
        "M^X/M/1 time in system (s)",
        exact,
        0.03,
        time_in_system,
    );
    check(
        &results,
        "M^X/M/1 parts waiting",
        lambda * k * (exact - 1.0 / mu),
        0.03,
        |r| r.conveyors[0].mean_parts,
    );
}

#[test]
fn arrivals_in_groups_of_random_size_wait_as_the_batch_formula_says() {
    // M^X/M/1 with group sizes 2 or 6, equally likely: E[X] = 4, E[X²] = 20.
    // Groups at 1 per 5 min, service exponential with mean 1 min: ρ = 0.8.
    // A random part's time in system is (E[X²]/E[X] + 1) / (2μ(1 − ρ))
    // (Gross & Harris), 900 s here; with fixed groups of k it's the
    // (k + 1) / (2μ(1 − ρ)) of the test above.
    let (ex, ex2, lambda, mu) = (4.0_f64, 20.0_f64, 1.0 / 300.0, 1.0 / 60.0);
    let rho = lambda * ex / mu;
    let exact = (ex2 / ex + 1.0) / (2.0 * mu * (1.0 - rho));
    let yaml = format!(
        r#"
format: simulancer/0.1
name: groups
entities:
  part: {{}}
sources:
  arrivals: {{ creates: part, interarrival: "exponential(5 min)", group: "discrete(2: 50%, 6: 50%)" }}
servers:
  server: {{ process_time: "exponential(1 min)" }}
queues:
  line: {{ from: arrivals, to: server }}
  out: {{ from: server, to: done }}
sinks:
  done: {{}}
experiment:
  replications: {REPLICATIONS}
  warmup: "50 h"
  run_length: "1000 h"
  seed: 20261127
"#
    );
    let results = replications(&spec(&yaml));
    assert!((exact - 900.0).abs() < 1e-9);
    check(
        &results,
        "M^X/M/1 (random group sizes) time in system (s)",
        exact,
        0.03,
        time_in_system,
    );
}
