# Breakdowns and repairs

Machines fail, jam, run out of tooling or stop for a quality check. Each of these is a **failure** of a server: it stops for a while, and what was in it waits.

```yaml
format: simulancer/0.1
name: press-line
entities:
  part: {}
sources:
  arrivals: { creates: part, interarrival: "exponential(2 min)" }
servers:
  press:
    process_time: "triangular(1 min, 1.4 min, 2 min)"
    failures:
      - name: tool_wear
        uptime: "weibull(shape: 2.5, scale: 6 h)"      # working time between failures
        downtime: "uniform(20 min, 40 min)"
        counts: busy_time                               # wears only while pressing
      - name: power_cut
        uptime: "exponential(2 d)"
        downtime: "lognormal(mean: 1 h, sd: 30 min)"
        counts: clock_time                              # happens whether it works or not
queues:
  inbound: { from: arrivals, to: press, capacity: 20 }
  outbound: { from: press, to: shipping }
sinks:
  shipping: {}
experiment:
  replications: 20
  warmup: "1 d"
  run_length: "28 d"
  seed: 1
```

## Choices

- **One failure per cause.** Jams, tool changes and power cuts have different clocks and different repair times. One failure with the average of all of them gets the variation wrong, and variation is what makes queues.
- **`counts: busy_time` or `clock_time`.** Wear-out happens while working: tool wear, jams, a belt that stretches. Power cuts, a supplier's shortage or a sensor that drifts happen with the clock. A machine that's idle half the time fails half as often with `busy_time`.
- **`on_failure: resume` or `restart`.** `resume` (the default) continues the part where it stopped, like a machine that's paused. `restart` starts it again from the beginning, like a heat treatment that has to begin again.
- **Uptime from records.** Maintenance records usually give a *mean time between failures* (MTBF) and a *mean time to repair* (MTTR). Use MTBF as the uptime's mean if it was measured over working time; if it's calendar time, use `clock_time` or convert it. Downtime is everything from the stop to the restart: waiting for the technician and the spare part, not only the repair itself.
- **Shapes.** Exponential uptime says failures don't depend on age. Wear-out, where an old tool fails sooner, is a Weibull with a shape above 1. Repairs are rarely exponential: most are short, a few are long, which a lognormal or a gamma captures. See [Choosing distributions](distributions.md).

## In the results

- Each server reports *failed* (share of its time down), *busy*, *blocked* and *idle*, and *failures* (how many).
- **Availability** is 1 − *failed*. Check it against what the plant reports: a mismatch usually means the downtimes or the clocks are off.
- A failure also stops the stations around it: upstream ones get *blocked* when the buffer between them fills, downstream ones *idle* when it empties. Bigger buffers between stations absorb short failures; try them as a scenario.

Failures are counted only after the warm-up, but they start from time 0, so a short warm-up can hide that every machine starts "new". With wear-out failures, the warm-up should be longer than the scale of the uptime.
