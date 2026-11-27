# Validation

A simulator can be wrong in ways nobody notices: a queue that serves in the wrong order, a failure clock that keeps running during repairs, a buffer that holds one part too many. SimuLancer checks its engine against models whose exact answers are known from queueing theory. The tests run on every build (`crates/engine/tests/queueing.rs`).

## The cases

| Model | Exact answer from | Checks |
|---|---|---|
| M/M/1, ρ = 0.7 | 1 / (μ − λ) | time in system, utilization |
| M/M/3 | Erlang C | time in system, utilization |
| M/D/1, M/G/1 with triangular and lognormal service | Pollaczek-Khinchine | time in system |
| M/G/1 with failures while busy (preemptive resume) | Pollaczek-Khinchine on the completion time: service plus the repairs during it | time in system, share failed, share busy |
| Two M/M/1 in series | Burke's theorem | time in system |
| M/M/1/5, arrivals lost when full | Finite-capacity formulas and PASTA | share of arrivals lost, time in system |
| Two exponential machines with a buffer of 2 between them, blocking after service, the first never starved | A Markov chain, solved in the test and checked against its closed form (Buzacott & Shanthikumar) | throughput, first machine blocked, second machine starved, parts in the buffer |
| Base stock: one-for-one restocking (S = 5), Poisson orders, exponential lead times, orders wait for stock | M/M/S with Erlang C: the S units are servers, busy while being replaced (Palm's theorem) | out of stock, order wait, on order, on the shelf |
| Arrivals in groups of 4 (M^X/M/1), ρ = 0.8 | Pollaczek-Khinchine with Erlang group service, plus each part's place in its group | time in system, parts waiting |

Conveyors in these models are made practically invisible (1 ms transit and room for 10,000 parts), so the results are the queueing system's alone.

## What counts as a pass

- 20 replications of 1000 hours each, after a 50-hour warm-up.
- The exact value must be within **4 standard errors** of the estimate.
- The estimate must also be precise: a relative standard error of at most 2% (1% for utilization). Otherwise a wide interval would pass anything.
- Seeds are fixed, so a test never fails by chance from one build to the next.

## Same seed, same numbers

The random number generator and the distributions are SimuLancer's own, not a library's, and time is kept in integers, so a model run with the same seed gives byte-identical results, whatever the number of threads. On Linux, the release binary and a build from source give the same files.

That holds across versions too: every example's `summary.json` is pinned in `tests/pinned/`, and every build runs the examples and compares the numbers exactly. A change that moves any of them fails the build. When a change is meant to move results (a bug fix in a distribution, say), the pinned copies are regenerated in the same pull request, so the diff shows what moved, and the release notes say *Results change*. A release whose notes don't say so gives the same numbers as the one before. `summary.json` records the version that ran it (`simulancer_version`).

No release has changed results so far.

## No sign of bias

With the default seed, every estimate came out slightly low, between −0.4 and −1.7 standard errors. Those estimates share the arrival stream through common random numbers, so they move together. Under seeds 7, 12345 and 99 the signs change, so there's no sign of a systematic bias. The newer cases pass under the same seeds.

## References

- Buzacott, J. A., & Shanthikumar, J. G. (1993). *Stochastic Models of Manufacturing Systems*. Prentice Hall.
- Gross, D., Shortle, J. F., Thompson, J. M., & Harris, C. M. *Fundamentals of Queueing Theory*. Wiley.
