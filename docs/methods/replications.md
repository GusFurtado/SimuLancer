# Replications and confidence intervals

A simulation with random arrivals, process times and failures gives a different answer every time it runs. One run is one draw: it can be lucky or unlucky, and on its own you can't tell which. So SimuLancer always runs a model several times, each with its own random numbers (a **replication**), and reports the mean with a **confidence interval**.

## How a result is reported

For each measure, with *n* replications whose values have mean *x̄* and standard deviation *s*:

```text
x̄ ± t(0.975, n − 1) · s / √n
```

`t(0.975, n − 1)` is the Student t quantile for a 95% interval. It's 2.26 for 10 replications and 2.05 for 30. The report also shows the min and max across replications, and a dot per replication in *Variation across replications*.

Read `55.9 /h ± 0.3 /h` as: the long-run throughput of this model is between 55.6 and 56.2 parts per hour, with 95% confidence. It's a statement about the model. Whether the model matches the real line is a separate question, answered by validating the model against data.

With a single replication there's no interval: the report shows "n/a" and a warning.

## How many replications

The interval narrows with the square root of the number of replications: four times as many replications halve it. So a run with *n* replications and half-width *h* needs about

```text
n × (h / wanted)²
```

replications for the half-width you want. Every run does that sum for you, for its **key results**:

- the results a project shows on the Simulation page (chosen on the Modeling page's *Decisions* tab);
- without a project, or when it shows none, each sink's throughput and time in system.

The wanted half-width is the model's **precision**: `experiment.precision` (see [Experiment](../models.md#experiment)), **± 5% of the mean** unless the model says otherwise. Shares of time (busy, blocked, served within the target wait...) are judged in points instead, at a fifth of the precision: ± 1 point for ± 5%. A share near zero would otherwise need endless replications.

When a key result is wider than that, `simulancer run` warns and says how many replications would do:

```text
warning: 3 replications aren't enough for ± 5%: throughput at shipping is ± 6.8%, time in system at shipping is ± 29%; about 102 replications would do (see the report's replications check)
```

The report's *Replications check* lists every key result with its interval now, the one wanted and the replications it needs, and `summary.json` has the same in `precision_check`. The number is an estimate from the run itself: the standard deviation is estimated too, so run the suggested number and check again.

Longer runs also narrow the intervals, and a run that's too short for its warm-up gives biased answers however many replications it has. See [Warm-up](warmup.md).

### When a comparison finds no clear change

*No clear change* can mean that the decision doesn't matter, or that the replications were too few to see that it does. The Simulation page tells the two apart, with the smallest change that matters there, 1% (or 1 point for shares of time):

- if the whole range of the change is within ± 1%, it says *too small to matter*: whatever the true change is, it's negligible;
- otherwise it says how many replications would narrow the range to ± 1%: *"no clear change (between −1.2% and +3.0%) · about 89 replications would narrow it to ± 1%"* (from 20 replications: 20 × 2.1² ≈ 89).

Common random numbers make the range of a change much narrower than either result's own interval (see [Comparing scenarios](comparisons.md)), so a comparison usually needs far fewer replications than the precision check of each run suggests.

## Random numbers and seeds

- **Same seed, same results.** `experiment.seed` fixes every random number. Running the same model with the same seed gives identical files, with any number of threads.
- **Streams are named.** Each random stream comes from the seed, the replication number and the name of the object that draws from it. Changing one object's process time doesn't change the arrivals or another machine's failures.
- That's what makes [comparisons](comparisons.md) fair: two scenarios that share object names see the same arrivals, process times and failures.

## Run in parallel

Replications run in parallel on all cores (`--threads` limits them). The examples run 30 replications of several simulated days in a fraction of a second.
