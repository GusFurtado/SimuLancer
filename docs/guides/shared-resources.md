# Shared operators and tools

When two machines share one operator, the second waits whenever the operator is busy at the first. A **resource** is a pool of identical units (people, tools, fixtures) that servers need while they work.

```yaml
format: simulancer/0.1
name: shared-operators
schedules:
  day: { shifts: ["07:00-17:00"], days: [mon, tue, wed, thu, fri] }
resources:
  setter: { capacity: 1, schedule: day }
  operator: { capacity: 2, schedule: day, order: [cnc_b, cnc_a] }   # cnc_b is served first
entities:
  part: {}
sources:
  arrivals: { creates: part, interarrival: "exponential(4 min)", output: { rule: shortest_queue } }
servers:
  cnc_a: { process_time: "triangular(5 min, 6 min, 9 min)", needs: { operator: 1 } }
  cnc_b: { process_time: "triangular(5 min, 6 min, 9 min)", needs: { operator: 1, setter: 1 } }
  deburr: { process_time: "2 min", input: { rule: first_come } }
queues:
  to_a: { from: arrivals, to: cnc_a }
  to_b: { from: arrivals, to: cnc_b }
  a_out: { from: cnc_a, to: deburr }
  b_out: { from: cnc_b, to: deburr }
  done: { from: deburr, to: shipping }
sinks:
  shipping: {}
experiment:
  replications: 20
  warmup: "7 d"
  run_length: "28 d"
  seed: 1
```

## Choices

- **A resource or more capacity?** A server with `capacity: 2` has two slots that work in parallel. Use a resource when the limit is shared between servers (one crane for three bays), or when people come and go on their own schedule.
- **All at once.** A server starts only when it can take every unit it `needs` at once, and gives them back when processing ends, even if the part then waits for room downstream. A server never holds some units while waiting for others, so a model can't deadlock over resources.
- **Who goes first.** Waiting servers are served first come, first served, unless the resource lists an `order`.
- **Units that travel with the part.** `takes` and `gives_back` keep a unit with the part between two servers: pallets and fixtures, beds from admission to discharge, or kanban and CONWIP cards that cap the work in process:

  ```yaml
  resources:
    cards: { capacity: 30 }
  servers:
    release: { process_time: "0 s", takes: { cards: 1 } }
    ship: { process_time: "2 min", gives_back: [cards] }
  ```

## In the results

- Each resource reports *busy*: the share of its units in use (over its shifts, with a schedule).
- Each server that needs resources reports *waiting for resources*: the share of its working time a part was ready but the units weren't there. A high value with an idle machine is the classic sign that people, not machines, are the bottleneck.
- Try one more operator, or a different `order`, as [scenarios](../scenarios.md).
