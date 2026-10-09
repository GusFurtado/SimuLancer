# HTTP API (v1)

`outliers-sim serve` exposes this API. The web interface uses nothing else: anything you can do in the browser, a script can do too.

```sh
outliers-sim serve --models ./models --runs ./runs --port 8080            # this machine only
outliers-sim serve --models ./models --runs ./runs --host 0.0.0.0        # other machines (put it behind your proxy)
```

- All bodies are JSON. Responses are gzip-compressed when the client accepts it; the event log shrinks about 9×.
- There's no built-in authentication. Put the server behind your reverse proxy (HTTPS, SSO) for anything beyond localhost.
- Errors are `{"error": "..."}` with a 4xx/5xx status. Validation problems also carry an `errors` list (see below).

## Concepts

- **Model files** are `*.yaml` files in the models folder (`*.layout.yaml` files are layouts, not models).
- **Scenarios** are a base model plus changes. Clients send them as JSON:

  ```json
  {
    "name": "two-cells-round-robin",
    "description": "optional",
    "base": "two-cells.yaml",
    "changes": { "sources.arrivals.output": { "rule": "round_robin" }, "conveyors.c_a.length": "12 m" }
  }
  ```

  Change paths are dotted (`servers.mill_b.failures.0.downtime`); values are anything the model format accepts there. Saved scenarios are YAML files in the models folder (see the README).
- **Runs** are queued and executed one at a time, each using all CPU cores (or `--threads`). Each run's results go to `<runs>/<id>/`. The list of runs lives in memory; result folders stay on disk across restarts.

## Validation errors

```json
{ "ok": false, "errors": [ { "change": "conveyors.c_a.speed", "file": null, "message": "`5 min` is a time, but a speed is expected here" } ] }
```

| Field | Meaning |
|---|---|
| `change` | The change path the error comes from: the field a UI should mark. `null` when it isn't tied to one change. |
| `file` | Set when the error is in a file instead: e.g. the base model is broken, with `line`. |
| `message` | What's wrong, worded for people. |

For whole files (`GET /models/{file}`, runs of a file), errors are `{ "file", "line", "column", "message" }`.

## Endpoints

### `GET /api/v1/health`

```json
{ "status": "ok", "version": "0.1.0", "runs": { "queued": 0, "running": 1 } }
```

For load balancers and monitoring.

### `GET /api/v1/models`

```json
{ "models": [ { "file": "two-cells-round-robin.yaml", "kind": "scenario", "base": "two-cells.yaml",
                "name": "two-cells-round-robin", "description": "...", "valid": true } ] }
```

### `GET /api/v1/models/{file}`

Everything a form needs:

| Field | Contents |
|---|---|
| `kind` | `model` or `scenario`. For scenarios, also `base` and `changes`. |
| `source` | The file's YAML text. |
| `tree` | The full model as JSON, with a scenario's changes applied. Change paths address this tree: `tree.sources.arrivals.output` ⇔ `"sources.arrivals.output"`. |
| `valid`, `errors` | Whether the model loads, and why not. |
| `layout` | The checked layout (same as `layout.json`), or `null`. |

### `POST /api/v1/check`

Body: a scenario. Response: `{ "ok": true, "errors": [] }` or the errors above. Nothing is saved or run.

### `PUT /api/v1/models/{file}`

Body: a scenario. Validates it, then saves it as a YAML scenario file. Responses:

| Status | When |
|---|---|
| `200` | saved |
| `422` | the scenario has errors (the response lists them; nothing is written) |
| `409` | `{file}` is a base model. Base models are never overwritten through the API. |

### `POST /api/v1/runs`

Body:

```json
{ "file": "two-cells.yaml", "replications": 10 }
{ "scenario": { "base": "two-cells.yaml", "changes": { "...": "..." } } }
```

Exactly one of `file` or `scenario`; `replications` is optional. The model is checked before it's queued:

- `202` with the run (below);
- `422` with errors.

An unsaved scenario is stored with its results as `scenario.yaml`.

### `GET /api/v1/runs` and `GET /api/v1/runs/{id}`

```json
{ "id": "1791568836-1", "label": "two-cells", "status": "running",
  "progress": { "done": 12, "total": 30 }, "error": null, "files": [] }
```

- `status`: `queued`, `running`, `done` or `failed` (then `error` says why).
- Poll `GET /api/v1/runs/{id}` for progress; replications finish in a fraction of a second each, so polling every 0.5–1 s is plenty.
- `files` lists the results once done. `GET /api/v1/runs` lists runs newest first.

### `GET /api/v1/runs/{id}/files/{name}`

| `name` | Contents |
|---|---|
| `report.html` | The self-contained report. |
| `summary.json` | Every measure with mean, 95% CI, sd, min, max, in standard units. |
| `replications.csv` | One row per replication × measure. |
| `events.jsonl` | Event log of replication 1, if the model asks for one (`outputs.event_log: jsonl`). |
| `layout.json` | The layout, if the model has one. |
| `scenario.yaml` | The scenario, for runs of unsaved scenarios. |

## Example: a buffer-size sweep in Python

```python
import time, requests
api = "http://simulator:8080/api/v1"
for meters in [8, 12, 16, 24]:
    run = requests.post(f"{api}/runs", json={"scenario": {
        "base": "conveyor-line.yaml", "changes": {"conveyors.c2.length": f"{meters} m"}}}).json()
    while (run := requests.get(f"{api}/runs/{run['id']}").json())["status"] in ("queued", "running"):
        time.sleep(0.5)
    summary = requests.get(f"{api}/runs/{run['id']}/files/summary.json").json()
    tis = next(m for m in summary["measures"] if m["measure"] == "time in system")
    print(f"c2 = {meters} m: time in system {tis['mean']/60:.2f} ± {tis['half_width']/60:.2f} min")
```
