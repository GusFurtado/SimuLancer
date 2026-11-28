# Choosing distributions

Every random time in a model needs a distribution. The choice matters most where variation drives queues: arrivals, process times at the bottleneck, failures and repairs.

| Situation | A good start | Why |
|---|---|---|
| Arrivals of many independent customers or orders | `exponential(mean)` | Independent arrivals form a Poisson process; busier hours change the rate, not the shape (`arrivals_per_hour`) |
| A machine cycle with little variation | `"45 s"` (constant), or `uniform`/`triangular` around it | Automated cycles barely vary |
| A manual task | `triangular(min, most likely, max)` from people who do it, or `lognormal(mean, sd)` from data | Manual times are skewed: rarely much shorter, sometimes much longer |
| Repairs | `lognormal` or `gamma` | Most repairs are quick, a few take hours |
| Time between failures | `exponential` (random failures) or `weibull` with shape > 1 (wear-out) | Age matters for wear |
| No data at all | `triangular` from three questions: fastest, usual, slowest | Easy to ask, and honest about the range |

## From data

Plot the data before fitting anything: a histogram shows whether it's skewed, has two humps (two kinds of work mixed together) or has outliers that are really errors. Then:

- **Fit the distribution's parameters to the data:** for a lognormal, its mean and standard deviation; for a gamma, shape × scale is the mean and shape × scale² the variance; for an exponential, the mean.
- **Check the tail:** a fitted distribution can draw values longer than anything observed. For process times that's usually realistic; for physical limits (a conveyor can't be faster than its speed), cut the range with `uniform` or `triangular` instead.
- **Mixed causes, separate distributions.** Two products with different times are two part types with a time each (`process_time: { frame: ..., wheel: ... }`), not one wide distribution.

## Without data

Ask the people who do the work for the shortest, the usual and the longest time, and use a triangular. Then find which inputs matter: run the model with each one pushed up and down (a [scenario](../scenarios.md) each). Inputs that barely move the results don't need better data; the ones that do are where to measure.

## What not to do

- **Averages instead of distributions.** A constant 1 minute where the real time varies from 30 s to 3 min hides the queues the model is for.
- **Normal for times.** It can draw negative values, and it's symmetric where times are skewed. SimuLancer doesn't offer it.
- **Exponential for everything.** It has as much variation as its mean; manual tasks and machine cycles have much less, and models with it predict queues that are too long.
