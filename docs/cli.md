# Command line

`simulancer` has five commands. Each takes `--help`. Models and [scenario files](scenarios.md) work anywhere a model is expected.

The text below is the output of `simulancer <command> --help`.

## `validate`

```text
Check a model or scenario file for errors without running it

Usage: simulancer validate [OPTIONS] <MODEL>

Arguments:
  <MODEL>  Path to the model or scenario YAML file

Options:
      --layout <LAYOUT>  Layout file to check [default: <model>.layout.yaml next to the model, if present]
  -h, --help             Print help
```

## `run`

```text
Run a model's experiment, print a summary and write result files

Usage: simulancer run [OPTIONS] <MODEL>

Arguments:
  <MODEL>  Path to the model or scenario YAML file

Options:
  -r, --replications <REPLICATIONS>
          Number of replications, overriding the model's `experiment.replications`
  -t, --threads <THREADS>
          Threads to run replications on [default: all CPU cores]
  -o, --output-dir <OUTPUT_DIR>
          Where to write result files [default: results/<model name>]
      --log-replication <LOG_REPLICATION>
          Which replication to write the event log for (when the model asks for one) [default: 1]
      --layout <LAYOUT>
          Layout file for the animation [default: <model>.layout.yaml next to the model, if present]
  -h, --help
          Print help
```

## `compare`

```text
Compare a scenario against a base model: paired replications with common random numbers, and a confidence interval on each difference

Usage: simulancer compare [OPTIONS] <BASE> <SCENARIO>

Arguments:
  <BASE>      The base model
  <SCENARIO>  The scenario to compare against the base. Its experiment settings (seed, replications, warm-up, run length) must match the base's

Options:
  -r, --replications <REPLICATIONS>  Number of replications for both, overriding the models' setting
  -t, --threads <THREADS>            Threads to run replications on [default: all CPU cores]
  -o, --output-dir <OUTPUT_DIR>      Where to write comparison.json [default: results/<base>-vs-<scenario>]
  -h, --help                         Print help
```

## `serve`

```text
Serve the web application and its HTTP API

Usage: simulancer serve [OPTIONS]

Options:
      --data <DATA>        Data folder: `projects/` (one git repository per project), `scenarios/` (managers' scenarios) and `runs/`. Created if missing. A new one starts with no projects [default: data]
      --host <HOST>        Address to listen on. The default only accepts connections from this machine; use 0.0.0.0 to serve other machines (behind IT's proxy) [default: 127.0.0.1]
      --port <PORT>        [default: 8080]
  -t, --threads <THREADS>  Threads per run [default: all CPU cores]
  -h, --help               Print help
```

## `fit`

```text
Suggest a distribution for observed durations: a column of a CSV file, or a plain list with one value per line

Usage: simulancer fit [OPTIONS] <FILE>

Arguments:
  <FILE>  The CSV file (or plain list) with the observed durations

Options:
  -c, --column <COLUMN>  The column to fit, by header name or position (1 is the first). Needed when the file has more than one column
  -u, --unit <UNIT>      The unit the values are in: s, min, h or d [default: min]
      --empirical        Print the values as `empirical(...)`, to use the data as they are
  -h, --help             Print help
```

For example, for a CSV file with the minutes each order took in a column named `minutes`:

```console
$ simulancer fit service.csv --column minutes
service.csv: 200 values, mean 4.544 min, sd 2.64 min, from 0.85 to 12.9 min

  distribution                                 KS D       p        mean          sd
  lognormal(mean: 4.593 min, sd: 3.057 min)   0.049   0.718   4.593 min   3.057 min
  gamma(3.05, 1.49 min)                       0.063   0.388   4.545 min   2.602 min
  weibull(1.84, 5.143 min)                    0.077   0.175   4.569 min   2.575 min
  triangular(0.85 min, 0.85 min, 12.9 min)    0.094   0.053   4.867 min    2.84 min
  exponential(4.545 min)                      0.236   0.000   4.545 min   4.545 min
  uniform(0.85 min, 12.9 min)                 0.325   0.000   6.875 min   3.479 min

best fit: lognormal(mean: 4.593 min, sd: 3.057 min)
```

The file can be separated by commas, semicolons or tabs, with or without a header row; with semicolons or tabs, a decimal comma is read too. Each distribution's parameters are estimated by maximum likelihood (uniform and triangular from the range, with the mode that matches the mean). *KS D* is the largest gap between the fitted distribution and the data's, so smaller is better. The p-values come from the Kolmogorov distribution and are optimistic, since the parameters were estimated from the same data: use them to rank, not as a test. Several distributions often fit about equally well; then prefer the simpler one, or the one that matches how the process works (Law, *Simulation Modeling and Analysis*, chapter 6).
