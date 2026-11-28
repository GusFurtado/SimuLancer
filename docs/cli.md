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

## `test`

```text
Run models and check their expected results (`expect:`); fails when one isn't met, for CI

Usage: simulancer test [OPTIONS] <MODELS>...

Arguments:
  <MODELS>...  Model or scenario files to test

Options:
  -r, --replications <REPLICATIONS>  Number of replications, overriding each model's setting
  -t, --threads <THREADS>            Threads to run replications on [default: all CPU cores]
  -o, --output-dir <OUTPUT_DIR>      Where to write each model's result files [default: results/<model name>]
  -h, --help                         Print help
```

`test` runs each model and checks its [expected results](models.md#expected-results-expect). It prints each one with its value and a verdict, and exits with status 1 if any isn't met, so it can run in CI:

```console
$ simulancer test examples/conveyor-line.yaml
examples/conveyor-line.yaml (30 replications): expected results: 3 of 3 met
  throughput at shipping: 55.9 /h ± 0.3 /h, expected the mean at least 54.0 /h · met
  time in system at shipping: 25.76 min ± 0.54 min, expected the whole interval at most 40.00 min · met
  Lathe busy: 77.6% ± 0.4%, expected the mean between 70.0% and 85.0% · met

3 of 3 expected results met
```

`run` prints the same lines after its table, and the report has an *Expected results* section.

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
