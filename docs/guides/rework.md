# Rework and scrap

Inspection finds bad parts. Some go back to be fixed, some are thrown away, and a part fixed once may fail again.

```yaml
format: simulancer/0.1
name: rework-loop
attributes:
  tests: { default: 0 }            # how many times a part was tested
  outcome: { default: 0 }          # the test's draw, to use in several routes
entities:
  part: {}
sources:
  arrivals: { creates: part, interarrival: "exponential(3 min)" }
servers:
  machine:
    process_time: "triangular(2 min, 2.5 min, 3.5 min)"
    input: { rule: priority, order: [back_to_machine, inbound] }   # rework first
  test:
    process_time: "1 min"
    sets: { tests: "tests + 1", outcome: "uniform(0, 1)" }
    output:
      rule: by_condition
      routes:
        - { when: "outcome < 0.1 and tests >= 2", to: to_scrap }   # failed again
        - { when: "outcome < 0.1", to: back_to_machine }           # 10% fail
        - { to: to_shipping }
queues:
  inbound: { from: arrivals, to: machine }
  tested: { from: machine, to: test }
  back_to_machine: { from: test, to: machine }
  to_scrap: { from: test, to: scrap }
  to_shipping: { from: test, to: shipping }
sinks:
  shipping: {}
  scrap: {}
experiment:
  replications: 20
  warmup: "1 d"
  run_length: "14 d"
  seed: 1
```

## Choices

- **A simple loop** needs no attributes: `output: { rule: probability, weights: { back_to_machine: 0.1, to_shipping: 0.9 } }` sends 10% back, every time. A part could then loop forever, which is fine when that's what happens (each pass fails independently).
- **Counting passes** needs an attribute: the test adds one to `tests` and routes on it. Draw the outcome once into an attribute (`outcome`) when several routes depend on it: a draw written in a condition is new for each route checked.
- **Rework first or last.** `input: { rule: priority }` lets the machine take reworked parts before new ones; without it, they queue with the rest (`first_come`).
- **Where rework happens.** Here it goes back to the same machine. A separate rework station with its own technician is another server, often with a [resource](shared-resources.md) on a schedule.

## In the results

- Each sink reports its own throughput and time in system, so *scrap* shows the scrap rate and how late scrap is found.
- Reworked parts take longer: compare the time in system at shipping with and without rework (set the 0.1 to 0 in a scenario) to see what quality costs in lead time.
- The machine's *busy* includes rework, which is capacity taken from new parts.
