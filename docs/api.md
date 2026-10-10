# HTTP API (v1)

`outliers-sim serve` exposes this API. The web pages use nothing else: the engineer's builder (`/build`), the manager's page (`/decide`), and anything a developer writes, are all clients of this API.

```sh
outliers-sim serve --data ./data --port 8080          # this machine only
outliers-sim serve --data ./data --host 0.0.0.0       # other machines (put it behind your proxy)
```

- All bodies are JSON. Responses are gzip-compressed when the client accepts it.
- There's no built-in authentication. Put the server behind your reverse proxy (HTTPS, SSO) for anything beyond localhost.
- Errors are `{"error": "..."}` with a 4xx/5xx status. Model problems also carry an `errors` list (see below).

## Concepts

- **Projects** live in `<data>/projects/<id>/`, each a git repository with `project.yaml` (name, description, model file), the model and its layout. An empty data folder gets the example projects.
- **Working copy** is the model as the engineer last saved it.
  - **Saving** applies changes and records a version (a git commit with a message).
  - **Publishing** marks the last saved version as `v1`, `v2`, … (a git tag).
  - Managers work with published versions, which never change after publishing.
- **Versions** in requests are `latest` (newest published), a name like `v3`, or absent for the working copy.
- **Changes** map dotted paths in the model to new values, e.g. `{"servers.mill_b.capacity": 2, "sources.arrivals.output": {"rule": "round_robin"}}`. List items by index: `servers.mill_b.failures.0.downtime`. Paths address the `tree` that `GET /projects/{id}` returns.
- **Runs** are queued and executed one at a time, each using all CPU cores (or `--threads`). Each run's results, including the exact model text that ran, go to `<data>/runs/<id>/`. The list of runs lives in memory; result folders stay on disk.

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

`published` is `null` until the first publish.

### `POST /api/v1/projects`

```json
{ "id": "line-2", "name": "Second line", "from": "conveyor-line" }
```

Creates a project from another one's working copy, with a fresh history.

| Status | When |
|---|---|
| `201` | created |
| `409` | the id is taken |
| `400` | bad id: lowercase letters, digits, `-`, `_` |

### `GET /api/v1/projects/{id}[?version=latest|vN]`

| Field | Contents |
|---|---|
| `name`, `description` | From `project.yaml` at that version. |
| `version` | `working`, or the version name (`?version=latest` resolves to it). |
| `model_file`, `source` | The model file's name and text. |
| `tree` | The model as JSON. Change paths address it. |
| `layout` | The checked layout, or `null`. |
| `valid`, `errors` | Whether the model loads, and why not. |
| `versions` | Published versions, newest first: `name`, `number`, `date`, `message`, `commit`. |
| `history` | Working copy only: saved versions, newest first, with `published` set to the version name where published. |

### `POST /api/v1/projects/{id}/check`

```json
{ "changes": { "conveyors.c_a.speed": "5 min" } }
```

Checks changes against the working copy and returns `{ "ok": true, "errors": [] }` or the errors above. Nothing is saved.

### `POST /api/v1/projects/{id}/save`

```json
{ "changes": { "servers.mill_b.capacity": 2 }, "message": "Second spindle on mill B" }
```

Applies the changes to the working copy and saves a version. The model file is rewritten in standard formatting; comments in it aren't kept. Responses:

| Status | When |
|---|---|
| `200` | `{ "ok": true, "saved": <commit> }` |
| `422` | the changes have errors (nothing is saved) |
| `400` | no changes |

### `POST /api/v1/projects/{id}/publish`

```json
{ "message": "optional; defaults to the saved version's message" }
```

Publishes the last saved version as the next `vN`. Responses:

| Status | When |
|---|---|
| `200` | `{ "ok": true, "published": <version> }` |
| `409` | already published, or unsaved edits on disk |
| `422` | the model has errors |

### `POST /api/v1/runs`

```json
{ "project": "two-cells", "version": "latest", "changes": { "...": "..." }, "replications": 10, "label": "optional" }
```

- `version`: absent runs the working copy; `latest` or `vN` runs a published version.
- `changes`: optional, applied on top of that version. That's how unsaved edits, and scenarios, run.
- The model is checked before it's queued: `202` with the run (below), or `422` with errors.

### `GET /api/v1/runs` and `GET /api/v1/runs/{id}`

```json
{ "id": "1791590716-1", "label": "Two cells v2", "project": "two-cells", "version": "v2", "status": "running",
  "progress": { "done": 12, "total": 30 }, "error": null, "files": [] }
```

- `status`: `queued`, `running`, `done` or `failed` (then `error` says why).
- Poll `GET /api/v1/runs/{id}` every 0.5–1 s for progress.
- `GET /api/v1/runs` lists runs newest first.

### `GET /api/v1/runs/{id}/files/{name}`

| `name` | Contents |
|---|---|
| `report.html` | The self-contained report. |
| `summary.json` | Every measure with mean, 95% CI, sd, min, max, in standard units. |
| `replications.csv` | One row per replication × measure. |
| `events.jsonl` | Event log of replication 1, if the model asks for one (`outputs.event_log: jsonl`). |
| `layout.json` | The layout, if the model has one. |
| `model.yaml` | The exact model that ran (the version plus any changes). |

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
