# Comparing scenarios

To know whether a change helps, compare the model with and without it. Two independent sets of runs would each carry their own noise, and the difference between them would be hard to see. SimuLancer compares on **common random numbers**, so the noise mostly cancels out.

## Common random numbers

Each random stream comes from the seed, the replication number and the name of the object that draws from it. When the base model and the scenario share object names, replication 3 of the scenario sees exactly the arrivals, process times and failures of replication 3 of the base. The only difference between the two runs is the change.

Each replication then gives a **paired difference** (scenario − base) for every measure. Its mean and 95% confidence interval come from those differences, with the same Student t formula as any result (see [Replications](replications.md)), and a verdict:

| Verdict | When |
|---|---|
| *higher* or *lower* | the interval doesn't include zero |
| *no clear difference* | it does |
| *identical* | every pair is equal |

The Simulation page says the same in words, with the change as a percentage of the current value: *"Time in system to shipping: 7.25 min → 5.00 min, down 31% (between 28% and 34%) · better"*.

## How much it helps

`compare` also computes the interval an unpaired comparison would have had (Welch's t), and prints the median gain:

```console
$ simulancer compare examples/conveyor-line.yaml examples/conveyor-line-longer-c2.yaml
...
Common random numbers made the confidence intervals 5.9× narrower (median) than independent runs would.
```

An interval 5.9 times narrower is worth about 35 times as many independent replications. The gain depends on how alike the two runs stay:

- **Small changes to one object** (a buffer size, a process time): large gains, since everything else matches.
- **Changes in routing:** smaller gains. With another routing rule, the same arriving part goes to a different machine and the two runs drift apart. For the routing rules in `examples/two-cells*.yaml` the gain is about 2×.
- **Renamed objects:** a renamed object gets other random numbers and isn't compared. Keep names when you build a scenario.

## Rules

- The two models must share their experiment settings: seed, replications, warm-up and run length. Otherwise `compare` refuses and shows both. `--replications` sets the number for both.
- Measures are matched by object type, object and measure name. Measures only one model has are left out.
- A model compared with itself gives exactly zero differences.
- Differences of percentages are in percentage points (pp).

## Many scenarios at once

Each interval has a 95% chance of covering the true difference. Compare many scenarios on many results and some small differences will look clear by chance. The Simulation page says so under every comparison. Treat a single surprising "better" among many with care, and confirm it with more replications.
