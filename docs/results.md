# Results

`simulancer run` writes its results to `results/<model name>/` (`--output-dir` to change it). A run on the web app keeps the same files, which you can download from the *Results* tab.

| File | What it holds |
|---|---|
| `report.html` | The report: one self-contained file with charts, which works offline and can be attached to an email. |
| `summary.json` | Every measure with its confidence interval, and the warm-up check. |
| `replications.csv` | The raw value of every measure in every replication. |
| `events.jsonl` | The event log behind the animation, for one replication, when the model asks for it (`outputs.event_log: jsonl`). |
| `layout.json` | The drawing the animation uses, when the model has a layout file. |

The same model with the same seed gives byte-identical files, whatever the number of threads.

## The report

- **Headline tiles:** throughput and time in system at the sinks, and how often sources were stopped or lost parts. Each comes with its 95% confidence interval and the range across replications.
- **Where each server's time goes:** busy, blocked (its output was full), failed and idle (nothing to work on), as shares of its capacity over the measured time (over its shifts, for a server with a schedule).
- **How full the conveyors run:** mean parts against capacity, and how often each conveyor was full.
- **Queues:** how many wait on average and at most, how long each waits (mean and 90th percentile), the share served within the target wait where the model sets one, and how often a limited queue is full.
- **Variation across replications:** one dot per replication with the mean and its confidence interval, so a single lucky run can't pass for the answer.
- **Warm-up check:** parts in the system over time, with the model's warm-up shaded and a suggested one. See [Warm-up](methods/warmup.md).
- **Replications check:** whether the key results' intervals are as narrow as the model's precision asks (± 5% by default), and about how many replications would make them so. See [How many replications](methods/replications.md#how-many-replications).
- **All measures:** the full table.

Every chart has tooltips with the confidence intervals, and every value is also in the table, so nothing depends on hovering.

## `summary.json`

Values are in standard units (seconds, fractions, parts, parts per hour), with the unit spelled out:

```json
{
  "format": "simulancer-summary/0.1",
  "simulancer_version": "0.1.0",
  "model": "conveyor-line",
  "seed": 20261009,
  "replications": 30,
  "warmup": 28800.0,
  "run_length": 432000.0,
  "confidence": 0.95,
  "measures": [
    {
      "object_type": "source", "object": "arrivals", "measure": "blocked",
      "unit": "fraction", "n": 30,
      "mean": 0.0717, "half_width": 0.0049, "sd": 0.0130,
      "min": 0.0396, "max": 0.1040
    }
  ],
  "warmup_check": {
    "response": "parts in the system", "bin": 3840.0,
    "mean": [...], "smoothed": [...], "window": 5,
    "cut_by": "bin", "suggested": 38400.0, "settles": true
  },
  "precision_check": {
    "target": 0.05, "replications": 30,
    "results": [
      {
        "label": "time in system at shipping",
        "object_type": "sink", "object": "shipping", "measure": "time in system",
        "scale": "relative", "half_width": 0.021, "wanted": 0.05,
        "enough": true, "replications_needed": null
      }
    ]
  }
}
```

`expectations` (when the model has [expected results](models.md#expected-results-expect)) has one entry per expectation, with its limits in standard units, the mean and half-width, and `met`.

`observations` (when the model has [observed values](models.md#observed-values-observed)) has one entry per observed result: the values and their mean, the model's mean and half-width, the difference with its 95% interval, and the `verdict` (`consistent`, `close` or `off`).

`precision_check` has one entry per key result. `scale` is `relative` (the half-width as a share of the mean) or `points` (shares of time, where 0.01 is one point); `replications_needed` is filled in when the interval is wider than `wanted`.

## `replications.csv`

One row per replication and measure, ready for a spreadsheet, pandas or R:

```text
replication,object_type,object,measure,unit,value
1,source,arrivals,blocked,fraction,0.05759960605324074
2,source,arrivals,blocked,fraction,0.07628457237037037
```

## The event log

`events.jsonl` is [JSON Lines](https://jsonlines.org): a header with the model's structure, then one event per line, with its time in seconds (exact to the microsecond) and objects referred to by name. It covers the whole run, warm-up included. It's the largest file, about 6 MB for five days of the conveyor line, so models only write it when they ask for it.
