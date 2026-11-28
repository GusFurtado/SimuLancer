# Shifts and working hours

Most systems don't work around the clock. A **schedule** says when; servers, resources and sources follow it.

```yaml
format: simulancer/0.1
name: two-shift-cell
schedules:
  two_shifts:
    shifts: ["06:00-14:00", "14:00-22:00"]
    days: [mon, tue, wed, thu, fri]
    finish_work: true               # finish the parts in hand after 22:00
entities:
  part: {}
sources:
  orders:
    creates: part
    arrivals_per_hour: { "06:00-10:00": 12, "10:00-18:00": 20, "18:00-22:00": 8 }
    schedule: two_shifts            # and only on weekdays
servers:
  cell:
    capacity: 2
    process_time: "triangular(4 min, 5 min, 8 min)"
    schedule: two_shifts
queues:
  waiting: { from: orders, to: cell }
  done: { from: cell, to: shipping }
sinks:
  shipping: {}
experiment:
  replications: 20
  warmup: "7 d"
  run_length: "28 d"
  seed: 1
outputs:
  throughput_per: two_shifts        # per working hour, not per calendar hour
```

## Choices

- **One schedule per pattern.** A night crew, a maintenance window on Saturday morning and the office hours of a planner are three schedules. Overlapping shifts merge, so `["06:00-14:00", "13:30-22:00"]` is one long day.
- **Who follows it.** A server with a schedule stops off shift, as if it had failed; a resource with one hands out no units; a source with one creates nothing. A server without a schedule works around the clock, even if its operator (a resource) goes home: then the resource is the one that stops it.
- **Busier hours.** `arrivals_per_hour` gives a rate per period of the day, the same every day. With a `schedule` too, the source only works when both allow it.
- **Overtime.** `finish_work: true` keeps servers and resources working after the shift while there's work in hand, and reports *overtime per week*. Without it, a part in process pauses until the next shift.
- **Throughput per working hour.** With shifts, a calendar-hour throughput mixes working hours with nights and weekends. `throughput_per: two_shifts` counts per hour of that schedule instead.

## In the results

- For servers and resources with a schedule, *busy*, *blocked*, *failed* and *idle* are shares of their **working** time, and *off shift* is the share of the week they were off.
- *Overtime per week*, with `finish_work`, is what a closing time really costs.
- The warm-up check works in whole weeks when there are shifts: a warm-up shorter than a week rarely makes sense, since the first Monday isn't like the others.

The simulation starts on Monday at 00:00. A run length in whole weeks (`28 d`) avoids measuring an extra Monday.
