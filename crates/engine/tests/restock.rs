//! Stock kept by restocking sources (`restocks`) and queues that start
//! with stock (`starts_with`).

use simulancer_engine::{NoTrace, ReplicationResult, Spec, simulate};

fn spec(yaml: &str) -> Spec {
    match simulancer_model::load_str(yaml) {
        Ok(model) => model.spec,
        Err(err) => panic!("{}", err.render("test.yaml", yaml)),
    }
}

/// A supplier keeps `stock` filled; a machine uses one part a minute from it.
fn model(restock: &str, starts_with: u32, extra: &str) -> String {
    format!(
        r#"
format: simulancer/0.1
name: stock
entities:
  part: {{}}
schedules:
  days: {{ shifts: ["08:00-16:00"] }}
sources:
  supplier: {{ creates: part, restocks: stock, {restock}{extra} }}
servers:
  machine: {{ process_time: "1 min" }}
queues:
  stock: {{ from: supplier, to: machine, starts_with: {starts_with} }}
  out: {{ from: machine, to: done }}
sinks:
  done: {{}}
experiment:
  replications: 1
  run_length: "100 h"
  seed: 5
"#
    )
}

fn run(yaml: &str) -> ReplicationResult {
    simulate(&spec(yaml), 1, &mut NoTrace)
}

fn conserved(r: &ReplicationResult) {
    let t = &r.totals;
    assert_eq!(
        t.created + t.assembled + t.split,
        t.exited + t.lost + t.consumed + t.in_system,
        "{t:?}"
    );
}

#[test]
fn reorder_point_and_quantity_keep_the_stock_from_running_out() {
    // Order 10 when the stock (with what's on order) is at 5 or less; they
    // take 3 minutes, in which 3 are used: the stock never runs out.
    let r = run(&model(
        "reorder_point: 5, order: 10, lead_time: \"3 min\"",
        10,
        "",
    ));
    conserved(&r);
    assert_eq!(r.conveyors[0].empty, 0.0, "never out of stock");
    // The machine works all the time: 6,000 parts in 100 h, the first 10
    // from the starting stock, so about 600 orders of 10.
    // (The last may finish exactly at the end, which isn't counted.)
    assert!(
        (5999..=6000).contains(&r.sinks[0].exited),
        "{}",
        r.sinks[0].exited
    );
    assert!(
        (599..=601).contains(&r.sources[0].orders),
        "{}",
        r.sources[0].orders
    );
    // Every order is on order for 3 minutes, one every 10: 10 × 0.3 parts.
    assert!(
        (r.sources[0].on_order - 3.0).abs() < 0.05,
        "{}",
        r.sources[0].on_order
    );
}

#[test]
fn a_long_lead_time_runs_the_stock_out() {
    // Ordering when 1 is left (as the 9th of 10 is taken), with 10 minutes
    // to wait: the 10th is done a minute later, then the machine waits 8
    // minutes for the delivery. 10 minutes busy in every 18.
    let r = run(&model(
        "reorder_point: 1, order: 10, lead_time: \"10 min\"",
        10,
        "",
    ));
    conserved(&r);
    assert!(
        r.conveyors[0].empty > 0.4,
        "out of stock {}",
        r.conveyors[0].empty
    );
    assert!(
        (r.servers[0].busy - 10.0 / 18.0).abs() < 0.01,
        "busy {}",
        r.servers[0].busy
    );
}

#[test]
fn order_up_to_with_periodic_review() {
    // Every hour, top the stock (with what's on order) up to 80 if it's at
    // 30 or less; deliveries take 30 minutes and 60 are used an hour. A
    // check that finds 20 orders 60, which come 10 minutes after the 20 ran
    // out; one that finds 30 orders 50, just in time. So it runs out for a
    // while in some hours, and never holds more than 80.
    let r = run(&model(
        "reorder_point: 30, order_up_to: 80, lead_time: \"30 min\", review_every: \"1 h\"",
        80,
        "",
    ));
    conserved(&r);
    assert!(
        (80..=100).contains(&r.sources[0].orders),
        "{}",
        r.sources[0].orders
    );
    let empty = r.conveyors[0].empty;
    assert!((0.05..0.2).contains(&empty), "{empty}");
    assert!(r.conveyors[0].max_parts <= 80);
}

#[test]
fn deliveries_wait_for_the_shift() {
    // The supplier only delivers 08:00-16:00; the machine works around the
    // clock, so the stock runs out at night.
    let r = run(&model(
        "reorder_point: 20, order: 60, lead_time: \"30 min\"",
        60,
        ", schedule: days",
    ));
    conserved(&r);
    let night = 16.0 / 24.0;
    assert!(
        r.conveyors[0].empty > 0.5 * night,
        "{}",
        r.conveyors[0].empty
    );
}

#[test]
fn restock_errors() {
    let errors = |yaml: &str| match simulancer_model::load_str(yaml) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e
            .diagnostics()
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("\n"),
    };
    assert!(
        errors(&model("order: 10, lead_time: \"3 min\"", 10, ""))
            .contains("needs a `reorder_point`")
    );
    assert!(
        errors(&model("reorder_point: 5, lead_time: \"3 min\"", 10, "")).contains("needs `order`")
    );
    assert!(
        errors(&model(
            "reorder_point: 5, order_up_to: 5, lead_time: \"3 min\"",
            10,
            ""
        ))
        .contains("above the reorder point")
    );
    assert!(errors(&model("reorder_point: 5, order: 10", 10, "")).contains("needs a `lead_time`"));
    assert!(
        errors(&model(
            "reorder_point: 5, order: 10, lead_time: \"3 min\", interarrival: \"1 min\"",
            10,
            ""
        ))
        .contains("can't also have `interarrival`")
    );
    let unreachable = model("reorder_point: 5, order: 10, lead_time: \"3 min\"", 10, "")
        .replace("restocks: stock", "restocks: spare")
        .replace("  out: {", "  spare: { from: other, to: done }\n  out: {")
        .replace(
            "servers:",
            "  other: { creates: part, interarrival: \"1 h\" }\nservers:",
        );
    assert!(errors(&unreachable).contains("its parts can't get there"));
    assert!(
        errors(
            &model("reorder_point: 5, order: 10, lead_time: \"3 min\"", 10, "")
                .replace("restocks: stock", "restocks: machine")
        )
        .contains("there's no queue `machine`")
    );
}
