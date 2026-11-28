# Coming from Arena

If you've built models in Rockwell Arena, most of what you know carries over: entities flow through a network, wait in queues, seize resources, and the answers come from replications with confidence intervals. This page maps Arena's concepts onto SimuLancer's, and says plainly where SimuLancer has no equivalent yet.

SimuLancer doesn't imitate Arena's look, and it can't open `.doe` files. A model is a YAML file you can read, diff and review, and the Modeling page draws the same network you'd draw in Arena.

## Words

| Arena | SimuLancer |
|---|---|
| Entity | Part (`parts: { one: customer, many: customers }` renames them for people) |
| Entity type / picture | Part type (`entities`), with how it's drawn in the layout |
| Attribute | Attribute (`attributes`, at most 8 per model, numbers or times) |
| Variable | No global variables; see [the gaps](#what-simulancer-lacks) |
| Replication length, warm-up, number of replications | `experiment: run_length, warmup, replications` |
| Base time units | Every time carries its unit; `units: { time: min }` sets a default |
| Stream numbers | Streams named after objects, so a change to one object doesn't shift another's random numbers |

## Basic Process modules

| Arena module | SimuLancer | Notes |
|---|---|---|
| **Create** | Source | `interarrival`, a schedule, `arrivals_per_hour` by time of day, `group` (entities per arrival), a mix of types (`creates: { a: 0.3, b: 0.7 }`) |
| **Process** (Delay) | Server | `process_time`, `capacity` |
| **Process** (Seize Delay Release) | Server with `needs` | The units are held while processing and released at the end; the server waits for all of them at once |
| **Process** (Seize ... Release across modules) | `takes` and `gives_back` | A part keeps units between two servers: pallets, beds, CONWIP cards |
| **Decide** (2-way or N-way by chance) | `output: { rule: probability, weights }` | |
| **Decide** (by condition) | `output: { rule: by_condition, routes }` | Conditions on attributes, the part type and `now` |
| **Batch** (permanent) | Assembly (`assembles`, `makes`) | A kit of given types becomes one new part |
| **Batch** (temporary) + **Separate** (split) | `batch` | Processed together, then leave one by one |
| **Separate** (duplicate) | `splits_into` | One part becomes N parts of another type |
| **Assign** | `sets` on a source or server | Attributes only |
| **Record** | Built-in results | Every object reports its measures; there's no custom tally or counter yet |
| **Dispose** | Sink | Throughput and time in system per sink, and per part type |

## Data modules

| Arena | SimuLancer |
|---|---|
| **Resource** (capacity, schedule) | Resource (`capacity`, `schedule`); servers `needs` its units |
| **Resource** failures | Failures belong to servers (`failures`), with `busy_time` or `clock_time` clocks |
| **Queue** (FIFO, LIFO, lowest/highest attribute value) | A queue's `order`: `first_in`, `last_in`, `{ lowest: attr }`, `{ highest: attr }`, `{ by_type: [...] }` |
| **Schedule** (capacity over time) | Schedules (`shifts`, `days`): on or off, weekly; arrivals vary by time of day with `arrivals_per_hour` |
| **Set** (resource sets) | Server `capacity`, or one resource with several units |

## Advanced Process and Transfer

| Arena | SimuLancer | Notes |
|---|---|---|
| Station, Route | Conveyors, queues and transports between objects | Each connection says how parts get to the next object |
| Conveyor (accumulating) | Conveyor | Length, speed and spacing; parts queue at the end |
| Conveyor (non-accumulating) | Not yet | Tracked in [#103](https://github.com/GusFurtado/SimuLancer/issues/103) |
| Transporter | Vehicles and transports | A fleet with count, speed, load and unload times; distances per transport |
| Hold, Signal, Match, Search, Pickup, Dropoff | Not yet | Some cases fit an assembly (Match) or a resource carried by parts (Hold until a card is free) |
| Expression, Variable | Formulas on attributes | No global state |
| ReadWrite | Not yet | Inputs come from the model file and the API |

## Results

| Arena report | SimuLancer |
|---|---|
| Entity: total time, wait time, VA/NVA time | Time in system per sink and part type; waiting times per queue. No VA/NVA split |
| Queue: waiting time, number waiting | Every queue: mean and longest wait, 90th percentile, mean and most parts waiting, service level with `target_wait` |
| Resource: utilization, number busy | *busy* per resource, and *busy*, *blocked*, *failed*, *idle* per server; *waiting for resources* per server |
| Half-widths | Every result has its 95% interval, as in Arena's replication reports |
| Output Analyzer, Process Analyzer | Scenarios and paired comparisons on common random numbers, in the CLI (`compare`) and on the Simulation page |
| Input Analyzer | Not yet (see [#96](https://github.com/GusFurtado/SimuLancer/issues/96)) |
| OptQuest | Not yet (see [#99](https://github.com/GusFurtado/SimuLancer/issues/99)) |

## What SimuLancer lacks

Compared with Arena, these are the gaps a migrating modeler is most likely to hit:

- **Logic modules** (Hold, Signal, Search, Match) and **global variables**. SimuLancer's logic is attributes, formulas and routing rules; models that coordinate entities through signals don't fit yet. Whether to add an escape hatch for such logic is open ([#103](https://github.com/GusFurtado/SimuLancer/issues/103)).
- **Non-accumulating conveyors** ([#103](https://github.com/GusFurtado/SimuLancer/issues/103)).
- **Custom statistics** (Record, Statistic): only the built-in measures are reported.
- **Value-added/non-value-added time** and cost reports.
- **Reading and writing files** during a run.

And where SimuLancer goes further than Arena out of the box: models as diffable text in git, one binary or one Docker image with no license server, a browser app for the people who decide, comparisons that say in words what changed, and checks that warn when the warm-up or the replications aren't enough.

## Moving a model over

There's no importer: Arena's `.doe` files are binary and undocumented. Rebuild the model module by module with the tables above, starting from the template closest to it (**New project** on the projects page), and compare the results with the Arena model's on the same inputs. The two won't match number for number (different random numbers), but their confidence intervals should overlap; where they don't, one of the two models does something the other doesn't.
