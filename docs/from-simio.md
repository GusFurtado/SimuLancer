# Coming from Simio

If you've built models in Simio, most of what you know carries over. Entities flow through a network of objects and links. They wait for capacity and seize secondary resources. The answers come from an experiment's replications, with confidence intervals and scenarios compared on common random numbers.

This page maps Simio's concepts onto SimuLancer's, and says plainly where SimuLancer has no equivalent yet.

SimuLancer doesn't imitate Simio's look, and it can't open Simio projects. A model is a YAML file you can read, diff and review, and the Modeling page draws the same network you'd lay out in Simio's Facility view.

The biggest difference is where parts wait:

- **Simio** keeps them in the objects' input and output buffers.
- **SimuLancer** keeps them in the connections between objects. A queue is what Simio calls a server's input buffer, with its capacity, ranking and reneging. A finished part that can't leave stays in the server's slot, which Simio models with an output buffer of 0.

## Words

| Simio | SimuLancer |
|---|---|
| Entity (`ModelEntity`) | Part (`parts: { one: customer, many: customers }` renames them for people) |
| Entity type and symbol | Part type (`entities`), with how it's drawn in the layout |
| Entity state | Attribute (`attributes`, at most 8 per model, numbers or times) |
| Model state | No global states; see [the gaps](#what-simulancer-lacks) |
| Property, and a model property an experiment control refers to | A value in the model; **⚑ Make this a decision** turns it into a control people set on the Simulation page |
| Facility view | The Modeling page's diagram |
| Run setup and the experiment's warm-up and replications | `experiment: run_length, warmup, replications` |
| Units in properties | Every time carries its unit; `units: { time: min }` sets a default |
| Random number stream (the stream argument of a distribution) | Streams named after objects, so a change to one object doesn't shift another's random numbers |

## Standard Library objects

| Simio object | SimuLancer | Notes |
|---|---|---|
| **Source** | Source | `interarrival`; `arrivals_per_hour` by time of day (Simio's time-varying arrival rate and rate table); `group` (entities per arrival); a mix of types (`creates: { a: 0.3, b: 0.7 }`); a `schedule`. No arrival table yet ([#96](https://github.com/GusFurtado/SimuLancer/issues/96)) |
| **Server** | Server | `process_time` (one, or one per part type), `capacity`, `schedule` |
| Server: input buffer | A queue in front of it | `capacity`, `order` (the ranking rule), `patience` (reneging) |
| Server: output buffer of 0 | Built in | A finished part waits in its slot until there's room downstream; the server shows as *blocked* |
| Server: secondary resources | `needs` | The units are held while processing, and the server waits for all of them at once |
| Server: changeover logic | `changeover` | One setup time for any change of part type, or one per change (`standard->premium: "8 min"`), like a changeover matrix |
| Server: reliability logic | `failures` | Calendar-time-based is `counts: clock_time`; processing-time-based is `counts: busy_time`. No count-based failures yet |
| Server: task sequences | Not yet | |
| **Combiner** | Assembly (`assembles`, `makes`) | A kit of given types becomes one new part. The parts in it are gone: there's no batch to split again, and no matching members by attribute |
| **Combiner** + **Separator** (split batch) | `batch` | Processed together, then leave one by one |
| **Separator** (make copies) | `splits_into` | Each finished part becomes N parts of another type |
| **Sink** | Sink | Throughput and time in system per sink, and per part type |
| **Resource** | Resource | `capacity`, `schedule`; its `order` says which waiting servers get units first |
| Resource seized in one object and released in another | `takes` and `gives_back` | A part keeps units between two servers: pallets, beds, CONWIP cards |
| **Worker** | A resource drawn as people | The animation shows them walking to the server, but in the model resources don't move and walking takes no time. Workers that carry parts: not yet |
| **Vehicle** | Vehicles and transports | A fleet with `count`, `speed`, `load_time`, `unload_time` and a `home`; on-demand, one part per trip, with distances per transport. No fixed routes, schedules or failures for vehicles yet |

## Links and nodes

| Simio | SimuLancer | Notes |
|---|---|---|
| **Connector** | Queue | No travel time |
| **Path**, **TimePath** | Conveyor, or a transport | A conveyor's length and speed give the travel time, and its spacing limits how many parts fit |
| **Conveyor** (accumulating) | Conveyor | Length, speed and spacing; parts queue at the end |
| **Conveyor** (not accumulating) | Not yet | Tracked in [#103](https://github.com/GusFurtado/SimuLancer/issues/103) |
| Link selection weights | `output: { rule: probability, weights }` | |
| Selection weights that are conditions | `output: { rule: by_condition, routes }` | Conditions on attributes, the part type and `now` |
| Destination from a node list, cyclic | `output: { rule: round_robin }` | |
| Destination from a node list, smallest value | `output: { rule: shortest_queue }` | |
| Destination from a node list, available ones only | `output: { rule: first_available }` | |
| Destination by sequence (sequence tables) | `output: { rule: by_type, routes }` at each object | A route per part type at every branch; no table of visits per part type |
| Ride on a transporter | A transport (`by: forklifts`) | |
| Several links into one object | `input` rule | `first_come`, `priority` (by connection) or `round_robin` |

## Data and definitions

| Simio | SimuLancer |
|---|---|
| **Rate table** | `arrivals_per_hour` by time of day |
| **Work schedule** and day patterns | Schedules (`shifts`, `days`): on or off, weekly. A schedule can't set a capacity value per period |
| **Changeover matrix** | `changeover` per pair of part types |
| **Data tables**, table row referencing | Not yet: values per part type go in the model (process time per type, routes by type). Reading inputs from files or databases is [#102](https://github.com/GusFurtado/SimuLancer/issues/102) |
| **Sequence tables** | Partly: `by_type` routing at each object |
| **States**, **Events** (Fire, Wait) | Attributes only; no global states or events |
| **Processes** and add-on process triggers | None: logic is attributes (`sets` on a source or server), formulas and routing rules. Whether to add an escape hatch is open ([#103](https://github.com/GusFurtado/SimuLancer/issues/103)) |
| **Functions** | Formulas on attributes and `now` |
| **Elements**: failure | `failures` on a server |
| **Elements**: material (bill of materials) | An assembly's kit (`assembles`); restocking a queue from a source (`restocks`, `reorder_point`) |
| **Elements**: tally, state and output statistics | Built-in results only; there's no custom statistic yet |
| **Elements**: timer, monitor | None |
| **Input parameters** (sample size error, response sensitivity) | Not yet ([#96](https://github.com/GusFurtado/SimuLancer/issues/96) covers fitting distributions to data) |
| Your own objects (subclassed or built from processes) | None: models use the built-in objects; the [templates](getting-started.md) are the starting points |

## Experiments and results

| Simio | SimuLancer |
|---|---|
| Experiment **controls** | Decisions: a number with limits, or named options, set per scenario on the Simulation page |
| Experiment **scenarios** | Scenarios, compared in pairs on common random numbers, in the CLI (`compare`) and on the Simulation page |
| **Responses** | Every object reports its measures; the project's key results are the ones the pages and checks look at first |
| Responses with lower and upper bounds | Not yet ([#94](https://github.com/GusFurtado/SimuLancer/issues/94)) |
| Replications required, confidence level | `replications`, at 95%. The **Replications check** says how many would make the key results as precise as `precision` asks |
| Warm-up period | `warmup`; the **Warm-up check** says whether it's long enough and suggests one |
| **SMORE** plots | Each result's 95% interval in the report, and comparisons that say in words what changed. No box-and-percentile plot per scenario |
| **Subset selection**, the KN add-in, **OptQuest** | Not yet ([#99](https://github.com/GusFurtado/SimuLancer/issues/99)) |
| **Pivot grid** | `summary.json` (every measure with its interval) and `replications.csv` (one row per replication and measure), for a spreadsheet, pandas or R |
| Logs and the trace | The event log (`outputs.event_log: jsonl`, replication 1), which the animation replays |
| Reports and dashboards | The run's report and the Simulation page |
| 3D animation | 2D animation of the event log |
| Gantt charts, planning and scheduling | None: SimuLancer is for design decisions, not for daily schedules |

## What SimuLancer lacks

Compared with Simio, these are the gaps a migrating modeler is most likely to hit:

- **Processes**, add-on process triggers, model states and events. SimuLancer's logic is attributes, formulas and routing rules, so models that coordinate entities through processes and events don't fit yet. Whether to add an escape hatch for such logic is open ([#103](https://github.com/GusFurtado/SimuLancer/issues/103)).
- **Data tables** and binding them to spreadsheets or databases ([#102](https://github.com/GusFurtado/SimuLancer/issues/102)).
- **Your own objects:** there's no way to build a new kind of object.
- **Workers that move**, and task sequences on a server.
- **Non-accumulating conveyors** ([#103](https://github.com/GusFurtado/SimuLancer/issues/103)).
- **Schedules with capacity values** (only on or off), schedules and failures for vehicles, and count-based failures.
- **Custom statistics** (tally, state and output statistics).
- **Input analysis:** input parameters, fitting distributions to data ([#96](https://github.com/GusFurtado/SimuLancer/issues/96)), and how much the inputs' sample size adds to the results' uncertainty.
- **Optimization and selection:** OptQuest, KN, subset selection ([#99](https://github.com/GusFurtado/SimuLancer/issues/99)).
- **3D**, Gantt charts and planning.

Where SimuLancer goes further than Simio out of the box:

- models are diffable text in git;
- it's one binary or one Docker image, with no license to manage;
- the people who decide get a browser app;
- comparisons say in words what changed;
- checks warn when the warm-up or the replications aren't enough.

## Moving a model over

There's no importer. One would have to be written and tested against real exported projects.

Rebuild the model object by object with the tables above, starting from the template closest to it (**New project** on the projects page). Then compare its results with the Simio model's on the same inputs. The two won't match number for number, since the random numbers differ, but their confidence intervals should overlap. Where they don't, one of the two models does something the other doesn't.
