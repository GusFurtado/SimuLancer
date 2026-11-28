# Priorities, due dates and impatience

Not every part is equal: urgent orders jump the queue, appointments come before walk-ins, and people who wait too long leave.

```yaml
format: simulancer/0.1
name: service-desk
parts: { one: customer, many: customers }
attributes:
  due: { default: "0 s" }
entities:
  appointment: {}
  walk_in: {}
sources:
  door:
    creates: { appointment: 0.3, walk_in: 0.7 }
    interarrival: "exponential(2 min)"
    sets: { due: "now + uniform(10 min, 40 min)" }
servers:
  desk:
    capacity: 3
    process_time: { appointment: "lognormal(mean: 8 min, sd: 3 min)", walk_in: "lognormal(mean: 4 min, sd: 2 min)" }
queues:
  line:
    from: door
    to: desk
    capacity: 20
    order: { by_type: [appointment, walk_in] }     # appointments first
    patience: { appointment: "1 h", walk_in: "exponential(15 min)" }
    target_wait: "10 min"
  served: { from: desk, to: exit }
sinks:
  exit: {}
experiment:
  replications: 20
  warmup: "2 h"
  run_length: "7 d"
  seed: 1
```

## Choices

- **Order in a queue.** `order` picks the next part: `first_in` (the default), `last_in`, `{ by_type: [...] }`, or an attribute, `{ lowest: due }` for the earliest due date first or `{ highest: value }`. Ties go first in, first out. A part in service is never interrupted for a more urgent one.
- **Order at a server's inputs.** With several incoming connections, `input: { rule: priority, order: [...] }` takes from the first that has a part, and `lowest` or `highest` compare an attribute across all of them.
- **Due dates** are attributes set when a part is created (`sets: { due: "now + ..." }`), and routes can test them: `when: "due < now"` sends late parts to an express lane. See [Attributes](../models.md#attributes).
- **Impatience.** `patience` draws, for each part as it gets in line, how long it will wait; if it's still waiting then, it leaves and counts as lost. Patience can differ by part type. Leaving makes room in a full line.
- **Service level.** `target_wait` adds *within target wait*: the share served within that time, the way call centers and banks state their targets.

## In the results

- The queue reports *waiting time*, its 90th percentile, *longest wait*, and *waiting time of* each part type: priorities help one type at the expense of the other, and both show.
- *Gave up* is the share of customers who left without being served; *within target wait* is the service level.
- A full line blocks the source, or loses the arrival with `when_blocked: drop`: lost customers before even joining the line.
