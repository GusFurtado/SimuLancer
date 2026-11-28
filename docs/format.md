# Format specification

This page specifies SimuLancer's file formats precisely enough for another program to read and write them. [Model files](models.md) explains what each key *does*; this page says what's *valid*. Where they differ, the implementation (`crates/model`) is the reference, and the difference is a bug in one of them.

## Files and versions

Every file starts with its format and version:

| File | `format` | Written by |
|---|---|---|
| Model | `simulancer/0.1` | people, the Modeling page |
| Scenario | `simulancer-scenario/0.1` | people |
| Layout (`<model>.layout.yaml`) | `simulancer-layout/0.1` | the Modeling page |
| Project (`project.yaml`) | `simulancer-project/0.1` | the server |
| Summary (`summary.json`) | `simulancer-summary/0.1` | `run`, the server |
| Event log (`events.jsonl`, first line) | `simulancer-events/0.1` | `run`, the server |

**Compatibility, until 1.0:** a release may change a format. A file in an older version is refused, not converted, with a message naming the last release that reads it, and the release notes announce every format change. Adding an optional key doesn't change the version: older files stay valid, but a file using the new key isn't valid for older releases.

## YAML

- **YAML 1.2**, so `yes`, `no` and `on` are strings.
- **Strict:** unknown keys are errors, reported with their line and column.
- **Strings for anything with a unit** (`"8 h"`, `"0.5 m/s"`) and for distributions, so YAML never guesses their type. Quoting is optional where YAML would read a string anyway (`8 h`), but recommended.
- **Maps keep their order.** Object order affects nothing in the results, but writers should keep it so files diff cleanly.

## Values

```ebnf
number      = ["-"], digits, [".", digits], [("e" | "E"), ["+" | "-"], digits] ;
time-unit   = "ms" | "s" | "min" | "h" | "d" ;
length-unit = "mm" | "cm" | "m" | "km" ;
time        = number, " ", time-unit | number ;           (* bare: needs units.time *)
length      = number, " ", length-unit | number ;         (* bare: needs units.length *)
speed       = number, " ", length-unit, "/", time-unit ;
percentage  = number, "%" ;
clock       = hh, ":", mm ;                               (* "00:00" to "24:00" *)
name        = (letter | "_"), {letter | digit | "_" | "-"} ;
```

- A bare number is in the model's default unit (`units: { time: min, length: m }`); without a default, it's an error.
- Times are rounded to the microsecond when converted.

### Distributions

```ebnf
distribution = time
             | name, "(", [argument, {",", argument}], ")" ;
argument     = value | parameter, ":", value ;     (* positional, then named *)
```

| Name | Parameters, in order |
|---|---|
| `constant` | `value` (time) |
| `exponential` | `mean` (time) |
| `uniform` | `min`, `max` (times; `min` < `max`) |
| `triangular` | `min`, `mode`, `max` (times; `min` ≤ `mode` ≤ `max`, `min` < `max`) |
| `lognormal` | `mean`, `sd` (times: of the distribution itself) |
| `weibull` | `shape` (number), `scale` (time) |
| `gamma` | `shape` (number), `scale` (time); mean = shape × scale |

All parameters are positive (`min` and `constant` may be zero). Inside a [formula](#formulas), a distribution whose arguments have no units draws plain numbers.

### Counts

A whole number of parts, from 1 to 100,000: `20`, or `{ min: 20, max: 30 }` (each value equally likely).

### Formulas

Formulas compute attribute values and route conditions. Precedence, lowest first:

```ebnf
formula    = or ;
or         = and, {"or", and} ;
and        = not, {"and", not} ;
not        = "not", not | comparison ;
comparison = "type", ("==" | "!="), part-type
           | sum, [("<" | "<=" | ">" | ">=" | "==" | "!="), sum] ;
sum        = product, {("+" | "-"), product} ;
product    = unary, {("*" | "/"), unary} ;
unary      = "-", unary | atom ;
atom       = "(", formula, ")" | number, [" ", time-unit] | "now" | attribute | distribution ;
```

Each formula has a kind: a time, a plain number or a condition. Kinds are checked: a time plus a number is an error, a time divided by a time is a number, and only conditions combine with `and`, `or` and `not`. Bare numbers in formulas are plain numbers; default units don't apply.

## A model file

| Key | Required | Contents |
|---|---|---|
| `format` | yes | `simulancer/0.1` |
| `name` | yes | the model's name |
| `description` | no | text |
| `parts` | no | `{ one, many }`: what flows, for people |
| `units` | no | `{ time, length }`: defaults for bare numbers |
| `entities` | yes | part types: `name: {}`, at least one |
| `attributes` | no | `name: { default }`, at most 8 |
| `sources` | yes | at least one |
| `servers`, `conveyors`, `queues`, `transports`, `vehicles`, `lines`, `sinks`, `resources`, `schedules` | no | maps of name → object |
| `experiment` | yes | `replications` (≥ 1), `run_length` (> 0), `seed`; optional `warmup`, `precision` |
| `outputs` | no | `event_log`, `summary`, `throughput_per` |

Each object's keys are in [Model files](models.md). Beyond the keys, a model is valid only if:

- **Names** are unique across sources, servers, conveyors, queues, transports, vehicles, sinks and resources; part types, attributes, schedules and lines each have their own.
- **Connections** (conveyors, queues, transports) go from a source or server to a server or sink. An object with several outgoing connections has an `output` rule; a server with several incoming ones has an `input` rule. Rules name existing connections.
- **Every part can reach a sink**, and every server can be reached from a source.
- **Part types fit:** every part type that can reach a server has a process time and a route; an assembly's kit and the types that reach it match.
- **Carried resources** (`takes`) are given back (`gives_back`) on every way to a sink.
- **Values are in range:** probabilities add up to 1, percentages are between 0 and 100, times aren't negative.

The loader reports every problem it finds, not only the first, each with its line.

## Semantics

How objects behave (when a server takes a part, how a conveyor accumulates, how a failure clock runs, how a shift ends) is described in [Model files](models.md) and, rule by rule, at the top of `crates/engine/src/sim.rs`. Two rules any implementation must follow to give the same numbers:

- **Time is integer microseconds**, and events at the same time run in the order they were scheduled.
- **Random streams are named:** each object draws from a stream derived from the seed, the replication number and the object's name, using SimuLancer's generator and sampling methods (`crates/engine/src/rng.rs`). Results are only identical with the same streams.
