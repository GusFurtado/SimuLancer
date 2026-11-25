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

The interval narrows with the square root of the number of replications: four times as many replications halve it. Pick the number that makes the intervals of the results you care about narrow enough for the decision at hand:

1. Run with the model's setting (`experiment.replications`; `--replications` overrides it on the command line).
2. Look at the half-width of the key results.
3. If it's too wide, multiply the replications by (current half-width / wanted half-width)².

Longer runs also narrow the intervals, and a run that's too short for its warm-up gives biased answers however many replications it has. See [Warm-up](warmup.md).

## Random numbers and seeds

- **Same seed, same results.** `experiment.seed` fixes every random number. Running the same model with the same seed gives identical files, with any number of threads.
- **Streams are named.** Each random stream comes from the seed, the replication number and the name of the object that draws from it. Changing one object's process time doesn't change the arrivals or another machine's failures.
- That's what makes [comparisons](comparisons.md) fair: two scenarios that share object names see the same arrivals, process times and failures.

## Run in parallel

Replications run in parallel on all cores (`--threads` limits them). The examples run 30 replications of several simulated days in a fraction of a second.
