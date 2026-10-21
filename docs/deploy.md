# Deploying outliers-sim

outliers-sim is one program. `outliers-sim serve` runs the web pages (`/` lists the projects; each opens on `/modeling` to edit its model or `/simulation` to compare scenarios) and their HTTP API, and runs simulations on the same machine. Users only need a browser.

- **There's no database.** One data folder holds `projects/` (one git repository per project) and `runs/` (each run's results). The first start in an empty folder creates three example projects (an assembly line, a bank branch, a warehouse); every example can also be started from the New project form.
- **The only runtime dependency is git**, used to version projects. The Linux binary itself is statically linked. The Docker image includes git.

## Option 1: binary + systemd

Download `outliers-sim-<version>-x86_64-linux.tar.gz` from the releases page, then:

```sh
tar xzf outliers-sim-*-x86_64-linux.tar.gz && cd outliers-sim-*-x86_64-linux
sudo apt install git                     # or your distribution's package
sudo useradd --system --home /var/lib/outliers-sim --shell /usr/sbin/nologin outliers-sim
sudo cp outliers-sim /usr/local/bin/
sudo cp deploy/outliers-sim.service /etc/systemd/system/
sudo systemctl enable --now outliers-sim
curl http://127.0.0.1:8080/api/v1/health
```

The service:

- runs as the unprivileged `outliers-sim` user;
- can only write to `/var/lib/outliers-sim`;
- listens on `127.0.0.1:8080` only.

## Option 2: Docker

```sh
docker run -d --name outliers-sim --restart unless-stopped \
  -p 127.0.0.1:8080:8080 -v outliers-sim-data:/data \
  ghcr.io/gusfurtado/outliers-sim:latest
```

- **Image:** about 41 MB: the binary plus git on Alpine. It runs as an unprivileged user (uid 65532).
- **Data:** `/data/projects` (an empty volume gets the example projects) and `/data/runs` live in the volume.
- **Host folder instead of a volume:** make it writable by uid 65532 (`sudo chown -R 65532:65532 /srv/outliers-sim`).
- **Stopping:** `docker stop` shuts down cleanly in well under a second.
- **Building it yourself:** `docker build -t outliers-sim .`

## HTTPS, logins and network access

The simulator has **no built-in authentication**. Put it behind the reverse proxy you already run, and let that handle HTTPS and logins (SSO, LDAP, or basic auth). `deploy/nginx.conf` is a working example; `nginx -t` accepts it.

- **Listening address:** the server listens on `127.0.0.1` by default. Only use `--host 0.0.0.0` when a proxy on another machine must reach it, and firewall the port so nothing else can.
- **Sub-paths:** the web interface uses relative URLs, so it can live under a sub-path (`https://intranet/simulator/`).
- **Health checks:** `GET /api/v1/health` returns `200` with the version and queue size, for load balancers and monitoring.

## Sizing

| Resource | What drives it |
|---|---|
| CPU | Each run uses **all cores**, one run at a time; other runs wait in a queue. More cores means faster runs. `--threads N` limits a run to N cores, e.g. on a shared host. |
| Memory | Small: tens of MB, plus the event log of the run being written. |
| Disk | Each run keeps its results. The event log is the big file, about 7 MB per 128 simulated hours of the conveyor-line example; it's only written for models that ask for one (`outputs.event_log: jsonl`). |

For scale: 30 replications of the examples take about 0.2 s on 12 cores.

## Operating it

- **Backups:** back up the `projects` folder; it's the only thing people create. Each project is a git repository, so its whole history comes along, and `git clone` works for copies. Run folders can be regenerated, since the same model and seed give identical results.
- **Cleanup:** delete old folders in the runs folder whenever you like. The list of runs in the web interface is kept in memory and starts empty after a restart; result folders stay on disk until you remove them.
- **Upgrading:** replace the binary (or pull the new image) and restart. Projects, models and scenarios carry a `format:` version, and the server says clearly if a file needs updating.
- **Logs:** the server logs to stdout/stderr, so `journalctl -u outliers-sim` or `docker logs outliers-sim`.

## Programs and scripts

Everything the web interface does goes through the HTTP API. See `docs/api.md`, including a Python example.
