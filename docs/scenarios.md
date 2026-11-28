# Scenarios

A scenario is a model with a few changes. There are two ways to write one: as a file next to the model, or on the Simulation page, from the decisions a published version offers.

## Scenario files

A scenario file names its base model and lists only what changes:

```yaml
format: simulancer-scenario/0.1
name: two-cells-first-available
base: two-cells.yaml            # relative to this file
changes:
  sources.arrivals.output: { rule: first_available }
```

- **Each change replaces the value at a dotted path:** `conveyors.c2.length`, or `servers.mill.failures.0.downtime` for an item of a list (by index). The last part of the path can also be a key the base doesn't set yet, such as `servers.lathe.capacity`.
- `format`, `name` and `description` go at the top of the scenario, not among the changes.
- **Errors point at the change that caused them,** at its line in the scenario file: "with this change: unknown variant `round_robn`, expected one of ...". A wrong path says what exists: "`servers` has no `mil_b` (it has: mill_a, mill_b, inspect)".
- **A scenario works anywhere a model does:** `validate`, `run` and `compare`. It uses its base model's layout.
- Objects can't be removed by a scenario. To change the structure of a model, copy it.

The examples include three: `conveyor-line-longer-c2.yaml`, `two-cells-first-available.yaml` and `two-cells-round-robin.yaml`.

## Comparing

```sh
simulancer compare examples/two-cells.yaml examples/two-cells-first-available.yaml
```

Both models run on the same random numbers, and each measure's difference gets a 95% confidence interval and a verdict: *higher*, *lower*, *no clear difference*, or *identical*. The two must share their experiment settings (seed, replications, warm-up and run length). See [Comparing scenarios](methods/comparisons.md) for why this works and when it helps less.

## Decisions on the Simulation page

People who decide don't need to read YAML. On the Modeling page, the engineer chooses what can change:

- **⚑ Make this a decision** under a number turns it into a decision with limits ("Packers: 1 to 3").
- **Offer as an option…** turns a set of edits into a named option of a decision ("Shipping conveyor: faster belt"), instead of saving them into the model.
- The *Decisions* tab also chooses which results the Simulation page shows. Every run checks those results for [precision](methods/replications.md#how-many-replications).

Once that version is published, the Simulation page lists the decisions with their current values. A scenario is a set of choices, and **Compare** runs the selected scenarios against the current setup and says what changed in words. The [tutorial](tutorial.md) walks through it.

A scenario made on the Simulation page is stored as its choices (decision → option or number) and the version it was made on, so it never changes under its user. Scripts can create and compare them through the [HTTP API](api.md).
