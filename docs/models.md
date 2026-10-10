# Model files

A model is one YAML file (format `outliers-sim/0.1`). The Build page writes the same files, so everything here can also be done there.

- `outliers-sim validate model.yaml` checks a model and prints what it understood.
- Errors point to the line and say what's wrong.

```yaml
format: outliers-sim/0.1
name: packing-line
description: Optional text.

entities:          # part types
  part: {}

sources:   { ... } # where parts come from
servers:   { ... } # machines and stations that work on parts
conveyors: { ... } # how parts move between them
sinks:     { ... } # where parts leave
resources: { ... } # operators and tools servers share (optional)
schedules: { ... } # weekly working times (optional)

experiment: { ... }
outputs:    { ... }   # optional
```

## Names

- **Allowed characters:** names use letters, digits, `_` and `-`, and start with a letter or `_`.
- **Uniqueness:** sources, servers, conveyors, sinks and resources share one namespace. Part types have their own.

## Units

- **Units in values:** times and lengths carry their unit: `"5 min"`, `"12 m"`, `"0.5 m/s"`.
  - Time units: `ms`, `s`, `min`, `h`, `d`.
  - Length units: `mm`, `cm`, `m`, `km`.
  - Speeds are a length unit per time unit.
- **Plain numbers** need default units: `units: { time: min, length: m }`. Without them, a plain number is an error.

## Distributions

Times can be random. Write a distribution with its parameters in time units:

| Distribution | Example | Parameters |
|---|---|---|
| Constant | `"45 s"` | the value |
| Exponential | `"exponential(1 min)"` | mean |
| Uniform | `"uniform(30 s, 50 s)"` | min, max |
| Triangular | `"triangular(1 min, 1.4 min, 2.1 min)"` | min, most likely, max |
| Lognormal | `"lognormal(mean: 10 min, sd: 4 min)"` | mean and standard deviation of the times themselves |
| Weibull | `"weibull(shape: 1.5, scale: 1 h)"` | shape (a plain number), scale |
| Gamma | `"gamma(shape: 2, scale: 30 s)"` | shape (a plain number), scale; the mean is shape × scale |

Parameters can be named (`mean: 10 min`) or given in order.

## Part types (`entities`)

```yaml
entities:
  frame: {}
  wheel: {}
```

A model has at least one part type. With several, sources can create a mix, and servers can take different times and send each type its own way. Results then include, for each sink, the throughput and time in system of each type.

## Sources

```yaml
sources:
  arrivals:
    creates: frame                       # or a mix: { frame: 0.25, wheel: 0.75 }
    interarrival: "exponential(1 min)"   # time between arrivals
    when_blocked: wait                   # wait (default) or drop
    output: { rule: round_robin }        # needed with several outgoing conveyors
```

- **The first part** arrives at time 0.
- **A mix of types:** each arrival's type is drawn with the given shares, which must add up to 1.
- **`when_blocked`:** what happens when the outgoing conveyor is full.
  - `wait` stops the source until there's room. The next interarrival time starts when the part gets on.
  - `drop` discards the part and counts it as lost.

## Servers

```yaml
servers:
  mill:
    capacity: 1                          # parts processed at the same time (default 1)
    process_time: "triangular(1 min, 1.4 min, 2.1 min)"
    # or per part type: { frame: "2 min", wheel: "30 s" }
    failures:
      - name: breakdown
        uptime: "exponential(1 h)"       # operating time between failures
        downtime: "uniform(5 min, 15 min)"
        counts: busy_time                # busy_time (default): wears only while processing; clock_time: always
        on_failure: resume               # resume (default) or restart the interrupted part
    input: { rule: first_come }          # needed with several incoming conveyors
    output: { rule: probability, weights: { c_ship: 0.9, c_rework: 0.1 } }
```

- **Processing:** a server takes a part when it's up and has a free slot.
- **Blocking:** a finished part stays in its slot until its outgoing conveyor has room. The server shows as *blocked* during that time.
- **Process time per part type:** every type that can reach the server must have a time.
- **`needs`:** resource units the server holds while it processes a part (see below).

## Routing and merging

Several outgoing conveyors need an `output` rule; several incoming ones need an `input` rule. There are no hidden defaults.

| `output` rule | Parts go to |
|---|---|
| `probability` | a conveyor drawn at random, with `weights` (adding up to 1) |
| `round_robin` | each conveyor in turn |
| `first_available` | the first conveyor, in order, with room |
| `shortest_queue` | the conveyor holding the fewest parts, among those with room |
| `by_type` | the conveyor given for the part's type: `routes: { frame: c_paint, wheel: c_ship }` |

- **Waiting:** with `probability`, `round_robin` and `by_type`, a part waits for its conveyor. With the other two, it takes whichever gets room first.
- **`by_type`:** every type that can reach the object must have a route.

| `input` rule | The server takes |
|---|---|
| `first_come` | the part that reached the end of its conveyor first |
| `priority` | from the first conveyor in `order: [c_rework, c_new]` that has a part waiting |
| `round_robin` | from each conveyor in turn, skipping empty ones |

## Conveyors

```yaml
conveyors:
  c1: { from: arrivals, to: mill, length: "12 m", speed: "0.5 m/s", spacing: "0.5 m" }
```

- **Accumulating:** parts queue up at the end when the next object can't take them.
- **Capacity:** a conveyor holds `floor(length / spacing)` parts.

## Resources

```yaml
resources:
  operator: { capacity: 2 }        # units available
servers:
  lathe_a: { process_time: "30 s", needs: { operator: 1 } }
  lathe_b: { process_time: "30 s", needs: { operator: 1 } }
```

- **Starting:** a server starts a part only when it can take every unit it needs at once. It never holds some units while waiting for others, so servers can't deadlock over them.
- **Holding:** units are held while processing, including during a failure mid-part. They go back as soon as processing ends, even if the part then waits for room on its conveyor.
- **Waiting:** servers waiting for units are served first come, first served.
- **Results:** each resource reports *busy*, the share of its units in use. Each server that needs resources reports *waiting for resources*, the share of time a part was ready but the units weren't. That time is part of *idle*.

## Schedules

```yaml
schedules:
  two_shifts:
    shifts: ["06:00-14:00", "14:00-22:00"]   # a shift ending at or before its start ends the next day
    days: [mon, tue, wed, thu, fri]          # optional; every day when absent
servers:
  lathe: { process_time: "5 min", schedule: two_shifts }
resources:
  operator: { capacity: 2, schedule: two_shifts }
sources:
  arrivals: { creates: part, interarrival: "10 min", schedule: two_shifts }
```

Schedules repeat every week, and the simulation starts on **Monday at 00:00**. Shifts are written in 24-hour clock time, from `00:00` to `24:00`; a night shift is written `22:00-06:00`. Overlapping shifts merge.

Off shift:

- **A server stops, as if it had failed.**
  - The part being processed pauses and resumes at the next shift.
  - Nothing is taken in or sent out.
  - Failure clocks pause.
  - A repair in progress carries on.
- **A resource hands out no units.** Units already in use finish their part first.
- **A source creates nothing.** The first part of each shift arrives at the shift's start.
- **Results:** servers and resources that follow a schedule report *off shift*, the share of time they were off. For servers it's one of the states that add up to 1; off shift counts before failed.

## Sinks

```yaml
sinks:
  shipping: {}
```

Every part must be able to reach a sink, and every server must be reachable from a source.

## Experiment

```yaml
experiment:
  replications: 30
  warmup: "8 h"          # optional: statistics start after it
  run_length: "120 h"    # measured time, after the warm-up
  seed: 20261009         # same seed, same results
```

Each replication uses its own random streams, named after the objects that draw from them. Changing one object doesn't change the random numbers of the others, which keeps comparisons between scenarios fair.

## Outputs

```yaml
outputs:
  event_log: jsonl       # write events.jsonl (replication 1) for the animation
  summary: [throughput, time_in_system, server_states, conveyor_occupancy, source_blocked_time]
```

`summary` lists which measures to report; all of them by default.
