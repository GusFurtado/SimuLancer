# Deploying SimuLancer

SimuLancer is one program. `simulancer serve` runs the web pages (`/` lists the projects; each opens on `/modeling` to edit its model or `/simulation` to compare scenarios) and their HTTP API, and runs simulations on the same machine. Users only need a browser.

- **There's no database.** One data folder holds `projects/` (one git repository per project) and `runs/` (each run's results, kept across restarts until they're deleted from the Modeling page or their project is deleted). A new data folder starts with no projects; people start them blank or from the built-in templates.
- **The only runtime dependency is git**, used to version projects. The Linux binary itself is statically linked. The Docker image includes git.

## Option 1: binary + systemd

Download `simulancer-<version>-x86_64-linux.tar.gz` from the [releases page](https://github.com/GusFurtado/SimuLancer/releases), then:

```sh
tar xzf simulancer-*-x86_64-linux.tar.gz && cd simulancer-*-x86_64-linux
sudo apt install git                     # or your distribution's package
sudo useradd --system --home /var/lib/simulancer --shell /usr/sbin/nologin simulancer
sudo cp simulancer /usr/local/bin/
sudo cp deploy/simulancer.service /etc/systemd/system/
sudo systemctl enable --now simulancer
curl http://127.0.0.1:8080/api/v1/health
```

The service:

- runs as the unprivileged `simulancer` user;
- can only write to `/var/lib/simulancer`;
- listens on `127.0.0.1:8080` only.

## Option 2: Docker

```sh
docker run -d --name simulancer --restart unless-stopped \
  -p 127.0.0.1:8080:8080 -v simulancer-data:/data \
  ghcr.io/gusfurtado/simulancer:latest
```

- **Image:** about 41 MB: the binary plus git on Alpine. It runs as an unprivileged user (uid 65532).
- **Data:** `/data/projects` and `/data/runs` live in the volume.
- **Host folder instead of a volume:** make it writable by uid 65532 (`sudo chown -R 65532:65532 /srv/simulancer`).
- **Stopping:** `docker stop` shuts down cleanly in well under a second.
- **Building it yourself:** `docker build -t simulancer .`

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
- **Cleanup:** runs are kept until someone deletes them from the Modeling page; most of their size is the event log behind the animation. Deleting a run's folder by hand while the server is stopped works too: it's gone from the list on the next start.
- **Upgrading:** replace the binary (or pull the new image) and restart. Projects, models and scenarios carry a `format:` version, and the server says clearly if a file needs updating.
- **Logs:** the server logs to stdout/stderr, so `journalctl -u simulancer` or `docker logs simulancer`.

## Programs and scripts

Everything the web interface does goes through the HTTP API. See [HTTP API](api.md), including a Python example.
