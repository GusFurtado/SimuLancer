# Model files

A model is one YAML file (format `simulancer/0.1`). The Modeling page writes the same files, so everything here can also be done there.

- `simulancer validate model.yaml` checks a model and prints what it understood.
- Errors point to the line and say what's wrong.
- **Format versions:** until SimuLancer 1.0, a release can change the format. A file in an older format is then refused with a message saying which release reads it; it isn't converted. The release notes announce every format change. The same goes for scenario, project and layout files.

```yaml
format: simulancer/0.1
name: packing-line
description: Optional text.
parts: { one: box, many: boxes }   # optional: what flows, in your words

entities:          # part types
  part: {}

sources:   { ... } # where parts come from
servers:   { ... } # machines and stations that work on parts
conveyors: { ... } # how parts move between them
queues:    { ... } # where parts wait in line between them, with no travel time
sinks:     { ... } # where parts leave
resources: { ... } # operators and tools servers share (optional)
schedules: { ... } # weekly working times (optional)

experiment: { ... }
outputs:    { ... }   # optional
```

## Names

- **Allowed characters:** names use letters, digits, `_` and `-`, and start with a letter or `_`.
- **Uniqueness:** sources, servers, conveyors, queues, sinks and resources share one namespace. Part types have their own.

## What flows (`parts`)

```yaml
parts: { one: customer, many: customers }
```

The model language calls whatever flows through a model a *part*. `parts` tells the pages, the report and `validate` what to call them instead: "customers" in a bank, "orders" in a warehouse, "patients" in a clinic. Measure names in result files stay the same (`mean parts`, `throughput`), so results and scripts don't change.

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

## Attributes

```yaml
attributes:
  due: { default: "0 s" }        # a time
  rework: { default: 0 }         # a plain number
sources:
  orders:
    creates: order
    interarrival: "exponential(5 min)"
    sets: { due: "now + uniform(2 h, 6 h)" }
servers:
  repair:
    process_time: "10 min"
    sets: { rework: "rework + 1" }
```

Attributes are values each part carries, like a due date or how many times it was reworked. Part types already tell parts apart by kind; attributes are for values that differ from part to part.

- **Declared once,** with a default: a plain number (`0`) or a time (`0 s`). That's what each part starts with, and what the attribute holds from then on. At most 8 per model.
- **`sets` on a source** applies to each new part. **`sets` on a server** applies when a part's processing finishes, before it picks its way out. Formulas apply in the order written, each seeing the values set before it.
- **Split parts** inherit the finished part's attributes. **An assembled part** keeps those of the component whose clock it keeps (`keeps_clock_of`, or its earliest component).
- **Formulas** use attribute names, `now` (the simulation clock), numbers, times with units (`4 h`), `+ - * /`, parentheses and distributions, which draw a new value each time: `uniform(2 h, 6 h)` draws a time, and `uniform(1, 10)` a plain number. Plain numbers never take the model's default unit here.
- **Units are checked:** a time plus a plain number, or a time set into a number attribute, is an error at that line. A time divided by a time is a plain number (`(due - now) / 1 h`).
- **Names:** attribute names use letters, digits and `_` (`-` means minus), and can't be a word formulas use (`now`, `type`, `and`, `or`, `not`, a unit or a distribution).
- **Event log:** each value set is a `set` event (`part`, `attribute`, `value`; times in seconds), and the header lists the attributes.

## Sources

```yaml
sources:
  arrivals:
    creates: frame                       # or a mix: { frame: 0.25, wheel: 0.75 }
    interarrival: "exponential(1 min)"   # time between arrivals
    when_blocked: wait                   # wait (default) or drop
    output: { rule: round_robin }        # needed with several outgoing conveyors
```

- **The first part** arrives at time 0 (with `interarrival`).
- **Arrivals by time of day:** instead of `interarrival`, give `arrivals_per_hour`, a rate for each period of the day:

  ```yaml
  sources:
    door:
      creates: customer
      arrivals_per_hour: { "10:00-12:00": 40, "12:00-14:00": 70, "14:00-16:00": 45 }
  ```

  Arrivals are random (a Poisson process) at each period's rate, the same every day, and none outside the periods. A period can run past midnight (`"22:00-02:00"`). With a `schedule` too, the source only creates parts while both say so (e.g. the rate by time of day, on weekdays only).
- **A mix of types:** each arrival's type is drawn with the given shares, which must add up to 1.
- **Arrivals in groups:** `group: 20` (or `group: { min: 20, max: 30 }`, each size equally likely) makes each arrival bring that many parts at once, like a truck of pallets. They get on in order; once one has to wait, the rest wait behind it, and the next arrival time starts when the last has got on. With `when_blocked: drop`, each part that finds no room is lost. All of a group arrive at the same instant, so a queue with room for one takes one of them before the next object takes anything.
- **`when_blocked`:** what happens when the outgoing conveyor is full.
  - `wait` stops the source until there's room. The next interarrival time starts when the part gets on.
  - `drop` discards the part and counts it as lost.

## Restocking

A source can keep a queue stocked instead of creating parts on a clock:

```yaml
sources:
  trucks:
    creates: pallet
    restocks: racks              # the queue it keeps stocked
    reorder_point: 250           # order when the stock position falls to this
    order: 25                    # parts per order; or order_up_to: 300
    lead_time: "uniform(18 h, 30 h)"
    review_every: "1 d"          # optional: check periodically instead of all the time
queues:
  racks: { from: putaway, to: break_pallet, capacity: 300, starts_with: 150 }
```

- **The stock position** is what the queue holds, plus what the source has delivered that hasn't got to the queue yet (on the dock, being put away), plus what's on order. When it's at or below `reorder_point`, the source orders: `order` parts at a time, as many orders as it takes to get above the reorder point, or with `order_up_to`, enough to bring the position up to that level.
- **When it checks:** every time a part leaves the queue, or with `review_every`, at that interval from time 0.
- **Deliveries** come after the `lead_time`, all at once, like a group arrival. With a `schedule`, a delivery that comes off shift waits for the next shift.
- **`starts_with`** puts parts in a queue at time 0: `150`, or `{ pallet: 150 }` when more than one part type travels on it.
- **Results:** the source reports *orders placed* and *on order* (mean parts ordered and not yet delivered); a stocked queue reports *empty*, the share of time it was out of stock.

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
- **Changeovers:** `changeover: "8 min"` adds a setup whenever a station starts a part of another type than the last it started; `changeover: { standard->premium: "8 min", premium->standard: "5 min" }` gives a time per change (changes not listed take none). With a capacity above 1, each slot remembers its own last type. The setup is part of the processing time (and of *busy*); servers report *changeovers* (how many) and *changeover* (the share of working time spent setting up).
- **Splitting:** `splits_into: { carton: 10 }` (or `{ carton: { min: 8, max: 12 } }`) turns each part the server finishes into that many parts of another type, like a pallet broken into cartons. They leave one by one, and keep the time the original part arrived, so time in system counts from then.
- **Batches and kits:** see [Batches and assembly](#batches-and-assembly).

## Batches and assembly

```yaml
servers:
  oven:
    batch: 8                          # processes 8 parts together
    process_time: "20 min"
  assembly:
    assembles: { frame: 1, wheel: 2 }   # a kit
    makes: bike                         # the part type a kit becomes
    process_time: "5 min"
```

- **Batches:**
  - A batch server collects parts from its inputs until the batch is full, then processes them all at once, with one process time and one set of resource units.
  - The parts then leave one by one.
  - Capacity defaults to one batch. A capacity of 16 with `batch: 8` processes two batches at once.
  - A partial batch waits until it's full.
- **Kits:**
  - An assembly server collects one kit at a time.
  - It takes a waiting part only if the kit still needs its type. Other parts wait on their conveyor.
  - A complete kit becomes one new part of the `makes` type and is processed like any part. Its time in system counts from its earliest component, or, with `keeps_clock_of: order`, from its earliest component of that type: an order picked from stock is timed from when the order came in, not from when the stock arrived.
- **Collecting** happens while the server is up, even if all its slots are busy. A batch or kit starts when there are enough free slots and resource units.
- **One process time:** batch and assembly servers take a single time, not one per part type.
- **Checks for assemblies:**
  - Every part type that can reach the server must be in the kit.
  - Every type in the kit must be able to reach it.
  - Routing after the server sees only the `makes` type.

## Routing and merging

Objects connect with conveyors and queues (together, *connections*). Several outgoing connections need an `output` rule; several incoming ones need an `input` rule. There are no hidden defaults. Rules name connections, whether conveyors or queues.

| `output` rule | Parts go to |
|---|---|
| `probability` | a conveyor drawn at random, with `weights` (adding up to 1) |
| `round_robin` | each conveyor in turn |
| `first_available` | the first conveyor, in order, with room |
| `shortest_queue` | the conveyor holding the fewest parts, among those with room |
| `by_type` | the conveyor given for the part's type: `routes: { frame: c_paint, wheel: c_ship }` |
| `by_condition` | the first route whose condition holds (see below) |

- **Waiting:** with `probability`, `round_robin`, `by_type` and `by_condition`, a part waits for its conveyor. With the other two, it takes whichever gets room first.
- **`by_type`:** every type that can reach the object must have a route.
- **`by_condition`:** a list of routes, each with a condition, ending with one without: it takes every other part, so no part is left without a way out.

  ```yaml
  output:
    rule: by_condition
    routes:
      - { when: "rework >= 2", to: c_scrap }
      - { when: "due < now", to: c_express }
      - { to: c_ok }
  ```

  Conditions are [formulas](#attributes) that compare (`<`, `<=`, `>`, `>=`, `==`, `!=`) and combine (`and`, `or`, `not`, parentheses), and can test the part's type: `type == frame`. They're checked when the part is ready to leave, after the server's `sets`. A draw in a condition (`uniform(0, 1) < 0.1`) is new for each part; to use one draw in several routes, set it into an attribute first (see the assembly-line template).

| `input` rule | The server takes |
|---|---|
| `first_come` | the part that reached the end of its conveyor first |
| `priority` | from the first conveyor in `order: [c_rework, c_new]` that has a part waiting |
| `round_robin` | from each conveyor in turn, skipping empty ones |
| `lowest` | the waiting part with the lowest value of an attribute: `{ rule: lowest, by: due }` (ties: the one that got there first) |
| `highest` | the waiting part with the highest value: `{ rule: highest, by: value }` |

## Conveyors

```yaml
conveyors:
  c1: { from: arrivals, to: mill, length: "12 m", speed: "0.5 m/s", spacing: "0.5 m" }
```

- **Accumulating:** parts queue up at the end when the next object can't take them.
- **Capacity:** a conveyor holds `floor(length / spacing)` parts.

## Queues

```yaml
queues:
  teller_line: { from: kiosk, to: tellers, capacity: 15, target_wait: "5 min", patience: "exponential(20 min)" }
  backlog:     { from: orders, to: pick }        # no capacity: no limit
```

- **Waiting in line:** a queue connects two objects like a conveyor, but parts don't travel: they're at its end as soon as they get on, and leave in the queue's `order`.
- **Order:** which part goes next. `first_in` (the default), `last_in`, `{ lowest: <attribute> }` (e.g. the earliest due date), `{ highest: <attribute> }`, or `{ by_type: [appointment, walk_in] }` (types not listed come after those listed). Ties go first in, first out. A part is never taken out of service for a more urgent one (no preemption).
- **Capacity:** how many parts can wait. Without it, there's no limit. When it's full, the object feeding it is blocked (or a source drops the part, with `when_blocked: drop`), as with a full conveyor.
- **Drawing:** queues aren't in the layout file; they're drawn straight between their objects, as a lane painted on the floor where parts wait, so moving an object never changes a queue.
- **Giving up:** with `patience`, each part draws how long it will wait as it gets on (one time or distribution, or one per part type: `{ appointment: "30 min", walk_in: "exponential(10 min)" }`). If it's still waiting when that time is up, it leaves and is counted as lost, which makes room in a full queue. The queue reports *gave up*: the share of those that left the line without being served. Parts a queue starts with (`starts_with`) don't give up.
- **Service level:** with `target_wait`, the queue also reports *within target wait*, the share of parts that waited at most that long ("95% served within 5 minutes").
- **Results:** mean and most parts waiting; *gave up* (with `patience`); *waiting time* (the mean), *waiting time (90th percentile)* and *longest wait*; *waiting time of* each part type when several wait there; the share of time full (for a limited queue); and throughput. The percentile comes from a histogram with bins 1% apart, so it's within about half a percent of the exact value.

Use a queue when the time to get from one object to the next doesn't matter, or for any line of people or store of goods. Use a conveyor when travel time and room along the way matter.

## Paced lines

```yaml
lines:
  front: { stations: [frame, drum], takt: "2.5 min" }
```

- **Stations move on together:** the stations of a paced line (servers in flow order, each holding one part) let their finished parts go only when the line moves: every `takt`. If a station isn't done when the takt is up, the whole line waits for it, and the next takt counts from when it moves. A failure at one station stops the whole line.
- **Between stations**, use a queue or a short conveyor with room for one part, so the line has no buffers.
- **Results:** the line reports *moves*, *stops* (takts that had to wait), *stopped* (the share of its working time it waited) and *stops caused by* each station (the last one to finish).

## Vehicles and transports

```yaml
vehicles:
  forklifts: { count: 3, speed: "2 m/s", load_time: "20 s", unload_time: "20 s", home: unload }
transports:
  to_racks: { from: unload, to: putaway, by: forklifts, distance: "60 m" }
```

- **A transport** connects two objects like a conveyor or queue, but a vehicle carries each part. A part ready to leave on it waits where it is; the nearest idle vehicle drives there (empty), loads it, drives the `distance`, unloads it, and stays there until it's needed again. Parts waiting longest are fetched first.
- **Empty trips** are as long as a transport between the two places (either way); without one, as long as the transport the vehicle is fetching for.
- **Capacity** (optional) counts parts waiting, carried and delivered; when it's full, the object feeding the transport is blocked.
- **Results:** the transport reports *waiting for pickup* (from ready to the vehicle's arrival), mean parts and throughput; the vehicles report *busy* (travelling, loading or unloading), *empty travel* and *trips*. Vehicles work around the clock: they have no schedule yet.
- Transports aren't in layout files: they're drawn straight between where vehicles stop at their objects, and the animation shows the vehicles moving. What the vehicles look like (a forklift, an AGV, a pallet jack, a tugger train or a truck) is in the layout file, under `vehicles`.

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
- **Waiting:** servers waiting for units are served first come, first served, including against the server that just gave them back.
- **Priority:** `order: [lathe_b, lathe_a]` serves the servers listed first, in that order, when several are waiting; servers not listed follow, first come, first served. A server that can't start (it needs more units than are free) doesn't hold back one behind it that can.
- **Results:** each resource reports *busy*, the share of its units in use (while on shift, if it has a schedule). Each server that needs resources reports *waiting for resources*, the share of its working time a part was ready but the units weren't. That time is part of *idle*.

### Resources parts carry

```yaml
resources:
  cards: { capacity: 30 }                                  # at most 30 orders between release and ship (CONWIP)
servers:
  release: { process_time: "0 s", takes: { cards: 1 } }    # each part takes a card as it starts here...
  ship:    { process_time: "2 min", gives_back: [cards] }  # ...and gives it back when done here
```

- **`takes`** works like `needs` (all at once, together with `needs`; for a batch, for each part of it), but the part keeps the units after it leaves, until it's done at a server whose **`gives_back`** names the resource. The same model covers CONWIP and kanban cards, pallets and fixtures, and beds taken at admission and given back at discharge.
- **Carried along:** an assembled part carries what its components did; when a part is split, the first part made carries what it did. A part that gives up waiting in a queue gives back what it carries.
- **Checked when loading:** every resource parts take is given back somewhere, and no part can reach a sink still carrying it (each way from a `takes` passes a `gives_back`). A resource parts carry can't have a schedule. At most 4 resources can be carried.
- **Results:** *busy* is the share of units out, held by a server or carried by a part: for CONWIP cards, the work in process over the cap.

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

- **Finishing the work in hand:** with `finish_work: true` on a schedule, servers and resources that follow it keep working after a shift ends while there's still work for them: parts in the server, on their way in, or still being worked on upstream. They stop once there's none left, and report *overtime per week*. Sources stop on time, so nobody new comes in: a bank closes its doors at 16:00 and its staff serve everyone inside, then go home.

Schedules repeat every week, and the simulation starts on **Monday at 00:00**. Shifts are written in 24-hour clock time, from `00:00` to `24:00`; a night shift is written `22:00-06:00`. Overlapping shifts merge.

Off shift:

- **A server stops, as if it had failed.**
  - The part being processed pauses and resumes at the next shift.
  - Nothing is taken in or sent out.
  - Failure clocks pause.
  - A repair in progress carries on.
- **A resource hands out no units.** Units already in use finish their part first.
- **A source creates nothing.** The first part of each shift arrives at the shift's start.
- **Results over working time:** for servers and resources that follow a schedule, *busy*, *blocked*, *failed* and *idle* are shares of their working time, so a teller busy 54% of their shift reads 54%, not 12% of the week. *Off shift* is the share of the whole week they were off. While off shift a server counts as off, even if it's being repaired.

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
  precision: 5%          # optional: how narrow the key results' intervals should be
```

Each replication uses its own random streams, named after the objects that draw from them. Changing one object doesn't change the random numbers of the others, which keeps comparisons between scenarios fair.

`precision` doesn't change the run: it's the half-width of the 95% interval wanted for the key results, as a share of their mean (± 5% if it's left out; shares of time are judged at a fifth of it, in points). When a key result is wider, `run` warns and the report says how many replications would do. See [How many replications](methods/replications.md#how-many-replications).

## Outputs

```yaml
outputs:
  event_log: jsonl       # events.jsonl (replication 1) for the animation; parquet for analysis; or [jsonl, parquet]
  summary: [throughput, time_in_system, server_states, conveyor_occupancy, source_blocked_time]
  throughput_per: two_shifts   # hour (default), day, week, or a schedule's name
```

- `event_log` writes every event of one replication (the first, or `simulancer run --log-replication`): `jsonl` is what the animation replays, `parquet` is the same events in a file about a tenth of the size, for pandas, R or DuckDB, and `[jsonl, parquet]` writes both. None by default. See [The event log](results.md#the-event-log).
- `summary` lists which measures to report; all of them by default.
- `throughput_per` sets what every throughput is counted per. `hour`, `day` and `week` are calendar time: a line that works 80 hours a week and ships 1,900 machines a week reports 11.3 /h. A schedule's name counts per hour of that schedule's working time instead: the same line reports 23.8 /working h with `throughput_per: two_shifts`.

## Layout files

A model's drawing lives next to it in `<model>.layout.yaml`. The engine never reads it, so moving things around never changes a result. The Modeling page writes it as you draw.

```yaml
model: conveyor-line.yaml
units: m
objects:
  arrivals: { position: [0, 0, 0], model: dock }
  lathe:    { position: [13.5, 0, 0], model: lathe }
conveyors:
  c1: { path: [[0.5, 0, 0], [12.5, 0, 0]] }   # 12 m
vehicles:
  forklifts: { model: forklift }
entities:
  part: { model: box }
resources:
  operator: { position: [6, 0, 3], model: worker }
```

- `position` is `[x, y, z]` with Y up: the floor is the X-Z plane.
- `model` is what the object is drawn as. Leave it out for the default.

  | Object | Can be drawn as |
  |---|---|
  | Source | `dock` (a truck at a dock, the default), `door`, `generic` |
  | Server | `machine` (the default), `lathe`, `mill`, `oven`, `workstation` (a bench with an operator), `counter` (a service desk), `kiosk` |
  | Sink | `dock` (the default), `door`, `bin`, `generic` |

  Anything else is a warning, not an error: the object is drawn as the default. `machine_small`, from older layouts, means `machine`.

  A server that works on several parts at once is drawn as that many units, side by side (up to four abreast, more rows behind) and centered on its position, each with its own state; a batch server is one unit per batch. An `oven`, or a server with more than 12 units, stays one drawing with `×N` in its name. Where the server `needs` a resource, its operators are drawn only on the units working.
- Every conveyor has a `path`. Queues and transports aren't listed: they're drawn straight between their objects. A path can start and end under an object's drawing (its position is a fine end point): the drawing stops it at the object's edge.
- `vehicles` (optional) says what each fleet is drawn as: `forklifts: { model: forklift }`. A fleet can be a `forklift` (the default), an `agv`, a `pallet_jack` (walked behind by its operator), a `tugger` (a tow tractor with two carts) or a `truck` (a flatbed). Vehicles stop on the floor in front of an object, drive straight between those stops facing the way they go, and carry their load on the forks, the deck, the first cart or the bed. The Modeling diagram shows each fleet parked where it starts (`home`). A fleet the model doesn't have, or a look that isn't one of these, is a warning.
- `entities` (optional) says what each part type is drawn as: `customer: { model: person }`. A part type can be a `box` (the default), a `pallet` or a `person`. Where a model has several part types, a box or pallet gets a sticker in the type's color and a person a shirt in it. People wait in front of a server rather than on it. A part type the model doesn't have, or a look that isn't one of these, is a warning.
- `resources` (optional) draws a resource's units as people: `operator: { position: [6, 0, 3], model: worker }`. `position` is where they wait while free (in rows of up to six), and the look is a `worker` (the default, with a vest and helmet) or a `clerk`. In the animation the nearest free one walks to the server that takes it and stands at the unit working (behind a counter, at a bench, else at the front right corner); when it's free it stays there for two minutes, in case more work comes, then walks back. This is only a picture: in the model, resources don't move and walking takes no time. A server whose `needs` are all drawn this way doesn't draw its own operators. A resource the model doesn't have, or a look that isn't one of these, is a warning. Resources the layout doesn't list aren't drawn.
