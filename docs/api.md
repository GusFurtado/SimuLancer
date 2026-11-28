# HTTP API (v1)

`simulancer serve` exposes this API. The web pages use nothing else: the Modeling page (`/modeling`), the Simulation page (`/simulation`), and anything you write yourself, are all clients of this API.

```sh
simulancer serve --data ./data --port 8080          # this machine only
simulancer serve --data ./data --host 0.0.0.0       # other machines (put it behind your proxy)
```

- All bodies are JSON. Responses are gzip-compressed when the client accepts it.
- There's no built-in authentication. Put the server behind your reverse proxy (HTTPS, SSO) for anything beyond localhost.
- Errors are `{"error": "..."}` with a 4xx/5xx status. Model problems also carry an `errors` list (see below).

## Concepts

- **Projects** live in `<data>/projects/<id>/`, each a git repository with `project.yaml` (name, description, model file), the model, its layout and its documentation (`README.md`, optional). A new data folder has none. Projects start blank, from a template (`GET /api/v1/templates`), or as a copy of another project.
- **Working copy** is the model as last saved on the Modeling page.
  - **Saving** applies changes and records a version (a git commit with a message).
  - **Publishing** marks the last saved version as `v1`, `v2`, … (a git tag).
  - The Simulation page works with published versions, which never change after publishing.
- **Versions** in requests are `latest` (newest published), a name like `v3`, or absent for the working copy.
- **Changes** map dotted paths in the model to new values (`null` removes an entry, e.g. `"conveyors.c3": null`), e.g. `{"servers.mill_b.capacity": 2, "sources.arrivals.output": {"rule": "round_robin"}}`. List items by index: `servers.mill_b.failures.0.downtime`. Paths address the `tree` that `GET /projects/{id}` returns.
- **Decisions and results** are what the Simulation page lets people change and read, set in `project.yaml` (below). A **scenario** picks a value for some decisions; it's stored with the published version it was made on, outside the project's git history.
- **Comparisons** run the current setup and some scenarios with the same random numbers and describe each result's change in plain language.
- **Runs** are queued and executed one at a time, each using all CPU cores (or `--threads`). Each run's results, including the exact model text that ran, go to `<data>/runs/<id>/`. Finished runs and comparisons are kept there (`run.json` next to each run's results, comparisons in `<data>/runs/comparisons/`), so a restarted server lists them as before. Runs that were queued or running when it stopped are dropped, and their folders removed.

## Decisions and results in `project.yaml`

```yaml
format: simulancer-project/0.1
name: Two cells
model: model.yaml
decisions:
  arrival_routing:
    label: How new parts are assigned to the cells
    choices:                                   # the first option is the current setup
      shortest_queue: { label: To the cell with the shorter queue }
      round_robin:
        label: Alternate between the cells
        changes: { sources.arrivals.output: { rule: round_robin } }
  inspectors:
    label: Inspectors
    path: servers.inspect.capacity             # a number people set on the Simulation page
    min: 1
    max: 3
  buffer:
    label: Buffer before the mill
    path: conveyors.c2.length
    unit: m                                    # the model writes it as "8 m"
    min: 4
    max: 24
    step: 2
results:
  - { label: Parts shipped per hour, object: shipping, measure: throughput, better: higher }
  - { label: Lead time, object: shipping, measure: time in system, better: lower }
```

- **Options** are groups of changes (paths as in [changes](#concepts)). The first option is the model as it is, so it has none.
- **Numbers** set one path, within `min` and `max`, optionally in steps of `step` from `min`. The model must already write the value (in `unit`, if given).
- **Results** name a measure from `summary.json` (`object`, `measure`) and say which way is better (`higher`, `lower`, or leave it out). Without `results`, the Simulation page shows each sink's throughput and time in system.
- **Publishing is refused while anything is wrong:**
  - an option or bound gives an invalid model;
  - two decisions change the same thing;
  - a decision changes `experiment` or `outputs`;
  - a result names a measure the model doesn't report;
  - an option matches the model already (put the option that matches the model first).

## Model errors

```json
{ "ok": false, "errors": [ { "change": "conveyors.c_a.speed", "file": null, "line": null, "column": null,
                              "message": "`5 min` is a time, but a speed is expected here" } ] }
```

| Field | Meaning |
|---|---|
| `change` | The change path the error comes from: the field a UI should mark. |
| `file`, `line`, `column` | Set instead when the error is in a file (the model or its layout). |
| `message` | What's wrong, worded for people. |

## Endpoints

### `GET /api/v1/health`

```json
{ "status": "ok", "version": "0.1.0", "runs": { "queued": 0, "running": 1 } }
```

For load balancers and monitoring.

### `GET /api/v1/projects`

```json
{ "projects": [ { "id": "two-cells", "name": "Two cells", "description": "...",
                  "published": { "name": "v2", "date": "2026-10-10T00:05:16Z", "message": "Second spindle" } } ] }
```

`published` is `null` until the first publish. `?archived=true` lists the archived projects instead (`id`, `name`, `description`).

### `POST /api/v1/projects`

```json
{ "id": "line-2", "name": "Second line", "from": "conveyor-line" }
```

Creates a project from another one's working copy, with a fresh history. With `"template": "bank-branch"` instead of `from`, it starts from a template, saved and published as v1 so it can be simulated right away; the template itself never changes. With neither, the project starts blank: a simple line (arrivals, one machine, exit) with a layout.

| Status | When |
|---|---|
| `201` | created |
| `409` | the id is taken, by a project or an archived one |
| `404` | no such template |
| `400` | bad id (lowercase letters, digits, `-`, `_`), or both `from` and `template` |

### `GET /api/v1/templates`

```json
{ "templates": [ { "id": "bank-branch", "name": "Bank branch", "description": "...",
                   "tags": ["services", "bank", "customers", "queues"] } ] }
```

The built-in templates, to pass as `template` when creating a project. Tags say the kind of system first (manufacturing, services, logistics), then what the model shows; they come from `examples/templates.yaml`.

### `GET /api/v1/projects/{id}[?version=latest|vN]`

| Field | Contents |
|---|---|
| `name`, `description` | The project's current ones (from the working copy), whichever version is asked for. |
| `version` | `working`, or the version name (`?version=latest` resolves to it). |
| `model_file`, `source` | The model file's name and text. |
| `tree` | The model as JSON. Change paths address it. |
| `layout` | The checked layout, or `null`. |
| `valid`, `errors` | Whether the model loads, and why not. |
| `versions` | Published versions, newest first: `name`, `number`, `date`, `message`, `commit`. |
| `history` | Working copy only: saved versions, newest first, with `published` set to the version name where published. |
| `decisions` | What can be changed on the Simulation page: `id`, `label`, `description`, `kind` (`choice` with `choices`: `id`, `label`, `changes`; or `number` with `path`, `min`, `max`, `step`, `unit`) and `current` (the first option's id, or the number in the model). |
| `results` | What the Simulation page shows: `label`, `object`, `measure`, `unit`, `better` (`higher`, `lower`, `neither`). |
| `decision_problems` | What's wrong with them, as sentences. Empty when the version can be published. |
| `project_file` | `project.yaml` as JSON, including the `decisions` and `results` as written. |
| `measures` | What the model reports (`object_type`, `object`, `measure`, `unit`): what results can name. |
| `documentation`, `documentation_html` | `README.md` in that version, as Markdown and rendered. Empty strings when there is none. The HTML is safe to show: HTML written in the Markdown comes out as text, links go only to `http`, `https`, `mailto` or within the page, and images become links. |

### `PATCH /api/v1/projects/{id}`

```json
{ "name": "Second line", "description": "Cartons packed and shipped." }
```

Changes the name and/or description in `project.yaml`, saved as a new version ("Renamed to Second line"). An empty `description` removes it. The id, and so the project's address, never changes. Returns `id`, `name`, `description` and `saved` (`false` when nothing changed).

### `POST /api/v1/projects/{id}/archive` and `POST /api/v1/projects/{id}/restore`

Archiving takes a project off the list, keeping its files, history and scenarios (in `<data>/projects/.archive/`); restoring brings it back. `404` if there's no such project, `409` if the id is taken on the other side.

### `DELETE /api/v1/projects/{id}`

Deletes an **archived** project for good, with its history, scenarios, runs and comparisons: `204`. A project on the list has to be archived first (`409`), so deleting always takes two steps, and its runs have to be finished (`409` while one is queued or running).

### `POST /api/v1/projects/{id}/check`

```json
{ "changes": { "conveyors.c_a.speed": "5 min" } }
```

Checks changes against the working copy. Nothing is saved.

- **Pending decisions and results:** the body may also carry `decisions` and/or `results`, which replace `project.yaml`'s.
- **Response:** `{ "ok": true, "errors": [], "decisions": [...], "results": [...], "decision_problems": [...] }`, where `decisions`, `results` and `decision_problems` are as in `GET /projects/{id}`.
- **A new layout:** `layout` replaces the saved one, as the layout file or `layout.json` would hold it. This is for edits that add, remove or move objects.
- **Documentation:** with `documentation` (Markdown), the response also has `documentation_html`, rendered as in `GET /projects/{id}`, for a preview.
- **When the model itself has errors,** `ok` is false, `errors` lists them as above, and only the structure of the decisions is checked.

### `POST /api/v1/projects/{id}/save`

```json
{ "changes": { "servers.mill_b.capacity": 2 }, "message": "Second spindle on mill B" }
```

Applies the changes to the working copy and saves a version.

- **With `decisions` and/or `results`,** `project.yaml` is updated in the same version. With `layout`, so is the layout file. With `documentation`, so is `README.md`; an empty one removes it.
- **Formatting:** the files are rewritten in standard formatting, and comments in them aren't kept.

Responses:

| Status | When |
|---|---|
| `200` | `{ "ok": true, "saved": <commit>, "decision_problems": [...] }` |
| `422` | the changes have errors (nothing is saved) |
| `400` | no changes, decisions, results, layout or documentation |

### `POST /api/v1/projects/{id}/publish`

```json
{ "message": "optional; defaults to the saved version's message" }
```

Publishes the last saved version as the next `vN`. Responses:

| Status | When |
|---|---|
| `200` | `{ "ok": true, "published": <version> }` |
| `409` | already published, or unsaved edits on disk |
| `422` | the model, or its decisions and results, have errors |

### `GET /api/v1/projects/{id}/scenarios[?version=vN]`

```json
{ "scenarios": [ { "id": "s1", "name": "Second inspector", "version": "v2",
                   "decisions": { "inspectors": 2 }, "created": 1791590716,
                   "summary": "Inspectors: 2" } ] }
```

Oldest first. `summary` is what the scenario changes, in words. `?version=` keeps only the scenarios made on that version.

### `POST /api/v1/projects/{id}/scenarios`

```json
{ "name": "Second inspector", "version": "latest", "decisions": { "inspectors": 2, "arrival_routing": "round_robin" } }
```

- `version`: `latest` (the default) or `vN`; the scenario keeps the version's name (`v2`), so it never changes under its user.
- `decisions`: decision id → option id or number. Decisions left out stay as they are.
- `name`: defaults to the summary of the changes.

| Status | When |
|---|---|
| `201` | the scenario |
| `422` | the picks are invalid (`errors`: unknown decision or option, a number out of bounds or off its steps, or a combination that gives an invalid model) |

### `PUT /api/v1/projects/{id}/scenarios/{scenario}` and `DELETE` …

`PUT` takes `name` and/or `decisions` (checked as above) and returns the scenario. `DELETE` returns `204`.

### `POST /api/v1/comparisons`

```json
{ "project": "two-cells", "scenarios": ["s1", { "name": "Faster repairs", "decisions": { "mill_b_repairs": "faster_crew" } }],
  "version": "v2", "replications": 30 }
```

- **`scenarios`:** saved scenario ids, and/or scenarios written in the request (`name`, `decisions`). From 1 to 12.
- **`version`:** defaults to the saved scenarios' version, or `latest`. Scenarios made on different versions can't be compared together.
- **`replications`:** optional; defaults to the model's.
- **Runs:**
  - The current setup and each scenario run as ordinary runs (see `run` fields below), all with the same random numbers.
  - A run of an identical model with the same replications is reused.
  - Returns `202` with the comparison, or `422`/`404` like scenarios do.

### `GET /api/v1/comparisons` and `GET /api/v1/comparisons/{id}`

```json
{ "id": "c7", "project": "two-cells", "version": "v2", "status": "done",
  "progress": { "done": 90, "total": 90 }, "error": null, "replications": 30,
  "current": { "name": "Current setup", "run": "1791590716-4" },
  "results": [ { "label": "Lead time", "object": "shipping", "measure": "time in system", "unit": "s", "better": "lower" } ],
  "scenarios": [ {
    "name": "Faster repairs", "scenario": null, "decisions": { "mill_b_repairs": "faster_crew" },
    "summary": "Repairs on mill B: Faster crew (3 to 8 minutes)", "run": "1791590716-6",
    "results": [ {
      "label": "Lead time", "object": "shipping", "measure": "time in system", "unit": "s", "better": "lower",
      "current": { "mean": 546.0, "half_width": 21.3 }, "scenario": { "mean": 434.4, "half_width": 9.8 },
      "change": { "mean": -111.6, "low": -124.0, "high": -99.2 },
      "relative": { "mean": -0.204, "low": -0.227, "high": -0.182 },
      "verdict": "lower", "judgment": "better", "small": false,
      "replications_needed": null,
      "text": { "current": "9.10 min", "scenario": "7.24 min", "change": "down 20%", "range": "between 18% and 23%", "more": null },
      "sentence": "Lead time: 9.10 min → 7.24 min, down 20% (between 18% and 23%) · better" } ] } ] }
```

- **`status`, `progress`:** cover all its runs. `results` are `null` until the comparison is `done`.
- **`change`:** the paired difference (scenario − current) in standard units, with its 95% interval.
- **`relative`:** the same as a fraction of the current mean. It is `null` for shares of time, which change in points.
- **`verdict`:** one of the following.
  - `higher` or `lower`: the interval excludes zero.
  - `no_clear_change`: the interval includes zero.
  - `identical`: every replication gave the same value.
  - `unknown`: one replication.
- **`judgment`:** `better` or `worse`, from the result's `better`.
- **`small`:** true for a clear change under 1% (or 1 point), or no clear change whose whole range is that small. It is reported as "too small to matter" and not judged.
- **`replications_needed`:** for no clear change that isn't small, about how many replications would narrow the range to ± 1% (or 1 point); `text.more` says it in words. Otherwise `null`.
- **`text` and `sentence`:** the same, worded for people.

### `POST /api/v1/runs`

```json
{ "project": "two-cells", "version": "latest", "changes": { "...": "..." }, "replications": 10, "label": "optional" }
```

- `version`: absent runs the working copy; `latest` or `vN` runs a published version.
- `changes`: optional, applied on top of that version. That's how unsaved edits, and scenarios, run.
- `layout`: optional, a whole new layout for unsaved edits that add or move objects.
- The model is checked before it's queued: `202` with the run (below), or `422` with errors.

### `GET /api/v1/runs` and `GET /api/v1/runs/{id}`

```json
{ "id": "1791590716-1", "label": "Two cells v2", "project": "two-cells", "version": "v2", "status": "running",
  "progress": { "done": 12, "total": 30 }, "error": null, "files": [], "queued_at": 1791590716 }
```

- `status`: `queued`, `running`, `done` or `failed` (then `error` says why).
- `queued_at`: when the run was asked for, in seconds since 1970 (UTC). Pages show it as a date and time.
- Poll `GET /api/v1/runs/{id}` every 0.5–1 s for progress.
- `GET /api/v1/runs` lists runs newest first.

### `DELETE /api/v1/runs/{id}`

Removes a finished run and its result folder: `204`. A queued or running run gives `409`; an unknown one `404`. A comparison that used the run then fails with "the run is gone".

### `POST /api/v1/fit`

Suggests a distribution for observed durations, as `simulancer fit` does; the Modeling page's **Fit from data…** uses it. Nothing is stored.

```json
{ "name": "service.csv", "text": "order;minutes\n1;2,5\n2;3,1\n...", "column": "minutes", "unit": "min" }
```

- `text`: the file's contents (CSV with `,`, `;` or tabs, or one value per line), up to 32 MB. `name` is only used in messages.
- `column`: a header name or a position (`1` is the first). Leave it out for a single column; with several, the answer is just `{ "columns": [...], "column": null }`, to ask which.
- `unit`: `s`, `min` (the default), `h` or `d`.

```json
{ "columns": ["order", "minutes"], "column": "minutes", "count": 120,
  "mean": "3.02 min", "sd": "0.71 min", "min": "1.6 min", "max": "5.2 min",
  "candidates": [{ "expression": "gamma(18.1, 0.1669 min)", "ks": 0.041, "p": 0.98, "mean": "3.02 min", "sd": "0.71 min" }],
  "empirical": "empirical(1.6 min, ...)", "empirical_max": 1000 }
```

- `candidates`: best first (smallest Kolmogorov-Smirnov distance). The p-values are optimistic, since the parameters come from the same data.
- `empirical`: the values as `empirical(...)`, or `null` above `empirical_max` values.
- `422` when the column isn't numbers (the message names the row) or has fewer than two values; `400` for an unknown unit.

### `GET /api/v1/runs/{id}/files/{name}`

| `name` | Contents |
|---|---|
| `report.html` | The self-contained report. |
| `summary.json` | Every measure with mean, 95% CI, sd, min, max, in standard units; `warmup_check`: parts in the system per time bin (mean across replications and smoothed), and the warm-up the MSER rule suggests (`suggested`, seconds; `settles`); `precision_check`: whether the project's results (or the sinks') are as precise as the model asks, and the replications they need. |
| `replications.csv` | One row per replication × measure. |
| `events.jsonl` | Event log of replication 1, if the model asks for one (`outputs.event_log: jsonl`). |
| `layout.json` | The layout, if the model has one. |
| `model.yaml` | The exact model that ran (the version plus any changes). |

## Example: compare two scenarios from a script

```python
import time, requests
api = "http://simulator:8080/api/v1"
c = requests.post(f"{api}/comparisons", json={"project": "two-cells", "scenarios": [
    {"name": "Second inspector", "decisions": {"inspectors": 2}},
    {"name": "Faster repairs", "decisions": {"mill_b_repairs": "faster_crew"}}]}).json()
while (c := requests.get(f"{api}/comparisons/{c['id']}").json())["status"] in ("queued", "running"):
    time.sleep(0.5)
for s in c["scenarios"]:
    print(s["name"])
    for r in s["results"]:
        print("  ", r["sentence"])
```

## Example: a buffer-size sweep in Python

```python
import time, requests
api = "http://simulator:8080/api/v1"
for meters in [8, 12, 16, 24]:
    run = requests.post(f"{api}/runs", json={
        "project": "conveyor-line", "version": "latest",
        "changes": {"conveyors.c2.length": f"{meters} m"}}).json()
    while (run := requests.get(f"{api}/runs/{run['id']}").json())["status"] in ("queued", "running"):
        time.sleep(0.5)
    summary = requests.get(f"{api}/runs/{run['id']}/files/summary.json").json()
    tis = next(m for m in summary["measures"] if m["measure"] == "time in system")
    print(f"c2 = {meters} m: time in system {tis['mean']/60:.2f} ± {tis['half_width']/60:.2f} min")
```
