# Validating your model

A model can run without errors, pass its [expected results](../models.md#expected-results-expect) and still be wrong about the real system. **Verification** asks whether the model does what you meant; **validation** asks whether what you meant matches reality. SimuLancer [validates its engine](validation.md) against queueing theory, so if a model disagrees with the plant, the cause is in the model or in its data. This page is about finding that out before anyone decides anything from it.

## Face validity first

Show the animation and the report to the people who run the system: the supervisor, the operators, the planner. Ask what looks wrong. Queues that never form, a machine that's never blocked or parts that skip a station are cheaper to catch this way than with statistics. Write what they confirmed, and what they doubted, in the project's documentation.

## Comparing with historical data

Record what the real system did next to the model, in its file, under `observed:`:

```yaml
observed:
  - object: shipping
    measure: throughput
    values: [59.2, 60.8, 61.5, 58.9, 60.3]    # one per week
    source: "Shipping scanner, weekly averages, Oct–Nov 2026"
  - { label: Inspection busy, object: inspect, measure: busy, value: 73% }
```

- **`value`** is one period. **`values`** has one value per period: a week, a shift, a month.
- Values are in the measure's terms, as for expected results: a time with its unit, a percentage for shares of time, a plain number otherwise.
- **`source`** says where the numbers come from, so the next person can check them.
- **`close_within`** (optional, `10%` by default) sets how far from the model's mean still counts as *close*.

After every run, `run` prints a *compared with the real system* list, the report has a section with the same name, and `summary.json` has `observations`. Each gets a verdict:

| Verdict | With several observed values | With one value |
|---|---|---|
| *consistent* | the 95% interval of the difference between the observed mean and the model's (Welch) includes zero | it falls inside the range one replication gives 95% of the time (mean ± t · sd · √(1 + 1/n)) |
| *close* | a clear difference, but within `close_within` of the model's mean (10%, or 2 points for shares of time) | the same |
| *off* | further than that | the same |

*Off* means the model doesn't match the real system on that result yet. Look for the cause, such as an input distribution, a missing failure mode, a shift pattern or a routing rule, before using the model to compare decisions.

## Why one month of data isn't a confidence interval

One observed month is one draw from the real system, like one replication is one draw from the model. It can be a good month or a bad one. With a single value, SimuLancer compares it with the spread of single replications, not with the model's confidence interval: that interval is about the model's long-run mean, and it gets narrower with every replication. A single month will almost never fall inside it, even when the model is right.

With several periods, the observed values have their own spread, and their mean has its own interval. The comparison then asks whether the two means differ by more than both intervals explain. That's the textbook test (Law, *Simulation Modeling and Analysis*, chapter 5), and it needs:

- **Periods that are alike.** Weeks of the same product mix and staffing, not a peak week next to a holiday.
- **Like with like.** A replication's length should match the period (5 days of measured time for weekly values), with the warm-up covering the start of the period. Measure the result the same way: lead time from the same start event to the same end event, and utilization over the same hours.
- **Independent periods.** Consecutive weeks are often correlated, which makes the interval too narrow; leave gaps, or treat a *consistent* verdict on two or three values with care.

## What counts as validated

There's no single test. A model is validated *for a purpose* when:

1. the people who know the system agree it behaves like it (face validity);
2. the results that matter for the decision are *consistent* with historical data, or *close* with a reason that's understood and written down;
3. its inputs come from data or from people who know, and the project's documentation says which;
4. it reproduces a past change, if there is one: a line that got a second packer, a shift added. Run the model before and after, and compare with what happened.

Keep the observed values in the model file. They're versioned with it, and every later run checks them again, so a change that breaks the match shows up.
