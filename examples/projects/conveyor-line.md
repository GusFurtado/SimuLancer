# Conveyor line

A small serial line: parts go through a lathe and then a mill, with accumulating conveyors between the steps. Both machines fail at random.

```
arrivals → c1 → lathe → c2 → mill → c3 → shipping
```

## Modeling assumptions

- **Arrivals:** about one part a minute (exponential). When conveyor c1 is full, arrivals wait.
- **Capacities are close to balanced on purpose:** after failures, each machine is only slightly faster than the arrivals, so small disturbances spread through the whole line.
- **Failures** wear only while a machine is working. The lathe breaks about every 45 minutes of work (4–12 minutes to repair), the mill every 60 (about 10). A part interrupted by a failure continues where it stopped.
- **Conveyors** hold 24, 16 and 12 parts (length divided by spacing).
- **Results** are measured over 120 hours after an 8-hour warm-up, 30 replications.

## Data

Illustrative. It's a small model for learning how blocking and starving spread along a line.
