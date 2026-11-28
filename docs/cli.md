# Command line

`simulancer` has four commands. Each takes `--help`. Models and [scenario files](scenarios.md) work anywhere a model is expected.

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
      --data <DATA>                Data folder: `projects/` (one git repository per project), `scenarios/` (managers' scenarios) and `runs/`. Created if missing. A new one starts with no projects [default: data]
      --host <HOST>                Address to listen on. The default only accepts connections from this machine; use 0.0.0.0 to serve other machines (behind IT's proxy) [default: 127.0.0.1]
      --port <PORT>                [default: 8080]
  -t, --threads <THREADS>          Threads per run [default: all CPU cores]
      --role-header <ROLE_HEADER>  Turns roles on: the header the reverse proxy sets with the user's groups (e.g. `X-Forwarded-Groups`). Users in a modeler group can build and publish models; everyone else only builds and compares scenarios. Without it, everyone can do everything
      --modelers <MODELERS>        The groups, in the role header, of people who build models (comma-separated) [default: modeler]
  -h, --help                       Print help
```
