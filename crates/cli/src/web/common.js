"use strict";
// Shared by the outliers-sim pages (/, /modeling, /simulation). Uses only the public
// HTTP API (docs/api.md).

const API = "api/v1";
const SPEEDS = [1, 10, 60, 300, 1800, 3600];
const STATES = [["busy", "Busy"], ["blocked", "Blocked (output full)"], ["failed", "Failed"], ["idle", "Idle (starved)"], ["off", "Off shift"]];
const OUTPUT_RULES = ["probability", "round_robin", "first_available", "shortest_queue", "by_type", "by_condition"];
const INPUT_RULES = ["first_come", "priority", "round_robin"];
const OBJECT_SIZE = 1.2;
/** How each kind of object is drawn: sources are circles, servers rounded squares, sinks diamonds. */
const OBJECT_SHAPES = { sources: "circle", servers: "rounded", sinks: "diamond" };

/** An SVG path (also usable as a canvas Path2D) for a shape of width `s` centered on (x, y). */
function shapePath(shape, x, y, s) {
  const h = s / 2, f = n => +n.toFixed(4);
  if (shape === "circle") return `M${f(x - h)},${f(y)}a${f(h)},${f(h)} 0 1,0 ${f(s)},0a${f(h)},${f(h)} 0 1,0 ${f(-s)},0Z`;
  if (shape === "octagon") {
    const r = h / Math.cos(Math.PI / 8);
    return "M" + [...Array(8)].map((_, i) => {
      const a = Math.PI / 8 + i * Math.PI / 4;
      return `${f(x + r * Math.cos(a))},${f(y + r * Math.sin(a))}`;
    }).join("L") + "Z";
  }
  if (shape === "diamond") return `M${f(x)},${f(y - h * 1.15)}L${f(x + h * 1.15)},${f(y)}L${f(x)},${f(y + h * 1.15)}L${f(x - h * 1.15)},${f(y)}Z`;
  const r = shape === "rounded" ? s * 0.125 : 0;
  return `M${f(x - h + r)},${f(y - h)}H${f(x + h - r)}a${f(r)},${f(r)} 0 0,1 ${f(r)},${f(r)}V${f(y + h - r)}a${f(r)},${f(r)} 0 0,1 ${f(-r)},${f(r)}`
    + `H${f(x - h + r)}a${f(r)},${f(r)} 0 0,1 ${f(-r)},${f(-r)}V${f(y - h + r)}a${f(r)},${f(r)} 0 0,1 ${f(r)},${f(-r)}Z`;
}
/** Part type colors: --type-0 ... --type-5 in common.css. */
const TYPE_COLORS = 6;

// --- helpers -----------------------------------------------------------------

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === null || v === false) continue;
    if (k === "class") node.className = v;
    else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else node.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) {
    if (c === null || c === undefined || c === false) continue;
    node.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return node;
}

function svg(tag, attrs = {}, ...children) {
  const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else node.setAttribute(k, v);
  }
  for (const c of children) node.append(c instanceof Node ? c : document.createTextNode(String(c)));
  return node;
}

async function api(method, path, body) {
  const response = await fetch(`${API}/${path}`, {
    method,
    headers: body ? { "content-type": "application/json" } : {},
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await response.text();
  let data;
  try { data = JSON.parse(text); } catch { data = text; }
  return { ok: response.ok, status: response.status, data };
}

const clone = v => (v === undefined ? undefined : JSON.parse(JSON.stringify(v)));
const equal = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const cssVar = name => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

function debounce(f, ms) {
  let timer;
  return (...args) => { clearTimeout(timer); timer = setTimeout(() => f(...args), ms); };
}

function clock(seconds) {
  const s = Math.floor(seconds);
  const pad = n => String(n).padStart(2, "0");
  return `day ${Math.floor(s / 86400) + 1}  ${pad(Math.floor(s % 86400 / 3600))}:${pad(Math.floor(s % 3600 / 60))}:${pad(s % 60)}`;
}

// --- change paths --------------------------------------------------------------

function getAt(tree, path) {
  let node = tree;
  for (const segment of path.split(".")) {
    if (node === null || node === undefined) return undefined;
    node = Array.isArray(node) ? node[Number(segment)] : node[segment];
  }
  return node;
}

function setAt(tree, path, value) {
  const segments = path.split(".");
  let node = tree;
  for (const segment of segments.slice(0, -1)) {
    const key = Array.isArray(node) ? Number(segment) : segment;
    if (node[key] === undefined) node[key] = {};
    node = node[key];
  }
  const last = segments[segments.length - 1];
  node[Array.isArray(node) ? Number(last) : last] = value;
}

const related = (a, b) => a === b || a.startsWith(b + ".") || b.startsWith(a + ".");

// --- layout ----------------------------------------------------------------------

/** The model's graph: object names by kind, and connections: conveyors and
 *  queues (`queue: true`, with `section` saying which part of the model). */
function graphOf(tree) {
  const connections = section => Object.entries(tree[section] || {})
    .map(([name, c]) => ({ name, from: c.from, to: c.to, queue: section !== "conveyors", transport: section === "transports", section }));
  return {
    sources: Object.keys(tree.sources || {}),
    servers: Object.keys(tree.servers || {}),
    sinks: Object.keys(tree.sinks || {}),
    conveyors: [...connections("conveyors"), ...connections("queues"), ...connections("transports")],
  };
}

/** A queue's path: straight between its objects' edges, moved 0.3 m to the
 *  right of the way parts go when a queue runs the other way (same as the
 *  server's layout). */
function queuePath(a, b, reverse) {
  const edge = OBJECT_SIZE / 2 + 0.1;
  const [dx, dz] = [b[0] - a[0], b[1] - a[1]];
  const len = Math.hypot(dx, dz);
  if (len <= 2 * edge) return [a, [a[0] + 0.01, a[1]]];
  const [ux, uz] = [dx / len, dz / len];
  const [ox, oz] = reverse ? [-uz * 0.3, ux * 0.3] : [0, 0];
  return [[a[0] + ux * edge + ox, a[1] + uz * edge + oz], [b[0] - ux * edge + ox, b[1] - uz * edge + oz]];
}

/** Draws every queue between its objects' current positions. */
function placeQueues(graph, positions, paths) {
  for (const c of graph.conveyors.filter(c => c.queue)) {
    const reverse = graph.conveyors.some(o => o.queue && o.from === c.to && o.to === c.from);
    if (positions[c.from] && positions[c.to]) paths[c.name] = queuePath(positions[c.from], positions[c.to], reverse);
  }
}

/** Positions (x, z in meters) and conveyor paths, from layout.json or computed. */
function layoutOf(graph, layout) {
  if (layout && graph.conveyors.every(c => c.queue || layout.conveyors[c.name]) &&
      [...graph.sources, ...graph.servers, ...graph.sinks].every(n => layout.objects[n])) {
    const positions = {};
    for (const [name, o] of Object.entries(layout.objects)) positions[name] = [o.position[0], o.position[2]];
    const paths = {};
    for (const [name, c] of Object.entries(layout.conveyors)) paths[name] = c.path.map(p => [p[0], p[2]]);
    placeQueues(graph, positions, paths);
    return { positions, paths };
  }
  return autoLayout(graph);
}

/** Columns by distance from the sources; backward conveyors loop around the top
 *  (same as the Godot viewer's automatic layout). */
function autoLayout(graph) {
  const depth = {};
  const queue = [...graph.sources];
  graph.sources.forEach(s => (depth[s] = 0));
  while (queue.length) {
    const node = queue.shift();
    for (const c of graph.conveyors) {
      if (c.from === node && !(c.to in depth)) { depth[c.to] = depth[node] + 1; queue.push(c.to); }
    }
  }
  const columns = {};
  for (const name of [...graph.sources, ...graph.servers, ...graph.sinks]) {
    const d = depth[name] ?? 0;
    (columns[d] ||= []).push(name);
  }
  const positions = {};
  for (const [d, names] of Object.entries(columns)) {
    names.forEach((n, i) => (positions[n] = [d * 10, (i - (names.length - 1) / 2) * 6]));
  }
  const top = Math.min(0, ...Object.values(positions).map(p => p[1]));
  const gap = OBJECT_SIZE / 2 + 0.2;
  const paths = {};
  let loops = 0;
  for (const c of graph.conveyors.filter(c => !c.queue)) {
    const [a, b] = [positions[c.from], positions[c.to]];
    if ((depth[c.to] ?? 0) > (depth[c.from] ?? 0)) {
      const len = Math.hypot(b[0] - a[0], b[1] - a[1]);
      const [ux, uz] = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
      paths[c.name] = [[a[0] + ux * gap, a[1] + uz * gap], [b[0] - ux * gap, b[1] - uz * gap]];
    } else {
      loops += 1;
      const lane = top - 2 - loops * 1.5;
      const out = [a[0] + gap / 2, a[1] - gap];
      const into = [b[0] - gap / 2, b[1] - gap];
      paths[c.name] = [out, [out[0], lane], [into[0], lane], into];
    }
  }
  placeQueues(graph, positions, paths);
  return { positions, paths };
}

function boundsOf(layout) {
  const points = [...Object.values(layout.positions), ...Object.values(layout.paths).flat()];
  const xs = points.map(p => p[0]), zs = points.map(p => p[1]);
  return { minX: Math.min(...xs) - 2, maxX: Math.max(...xs) + 2, minZ: Math.min(...zs) - 2.5, maxZ: Math.max(...zs) + 2 };
}

// --- the top bar of /modeling and /simulation -------------------------------------

/** The project the page is open on (`?project=`). Without one, back to the project list. */
function pageProject() {
  const id = new URLSearchParams(location.search).get("project");
  if (!id) { location.replace("./"); return null; }
  for (const page of ["modeling", "simulation"]) {
    const link = document.getElementById(`to-${page}`);
    link.href = `${page}?project=${encodeURIComponent(id)}`;
    link.replaceChildren(icon(page), PAGES[page]);
  }
  document.getElementById("project-name").textContent = id;
  return id;
}

/** The two ways into a project: their names and small line icons. */
const PAGES = { modeling: "Modeling", simulation: "Simulation" };

function icon(page) {
  const shapes = page === "modeling"
    // Modeling: three connected nodes, a flow drawn as a graph.
    ? [["circle", { cx: 5, cy: 6, r: 2.5 }], ["circle", { cx: 5, cy: 18, r: 2.5 }], ["circle", { cx: 19, cy: 12, r: 2.5 }],
       ["path", { d: "M7.5 6.5 16.6 11.2M7.5 17.5 16.6 12.8" }]]
    // Simulation: a play button trailed by moving parts.
    : [["path", { d: "M10 5.5 20 12 10 18.5Z" }], ["circle", { class: "dot", cx: 2.5, cy: 12, r: 1.2 }], ["circle", { class: "dot", cx: 6, cy: 12, r: 1.5 }]];
  return svg("svg", { class: "icon", viewBox: "0 0 24 24", "aria-hidden": "true" }, ...shapes.map(([tag, attrs]) => svg(tag, attrs)));
}

/** Shows the project's name in the top bar and the tab title. */
function showProjectName(name, page) {
  document.getElementById("project-name").textContent = name;
  document.title = `${name} · ${page} · outliers-sim`;
}

/** Fills `main` with a note that the project can't be opened, and a way back. */
function projectMissing(main, text) {
  main.replaceChildren(el("div", { class: "empty" }, el("p", {}, text), el("p", {}, el("a", { href: "./" }, "Back to the projects"))));
}

// --- runs and results -------------------------------------------------------------

/** Polls a run until it's done or failed, calling `onProgress(run)` meanwhile. */
async function waitForRun(run, onProgress) {
  while (run.status === "queued" || run.status === "running") {
    onProgress(run);
    await new Promise(r => setTimeout(r, 400));
    run = (await api("GET", `runs/${run.id}`)).data;
  }
  return run;
}

function progressText(run) {
  return run.status === "queued"
    ? "Waiting for another run to finish…"
    : `Running ${run.label}: ${run.progress.done} of ${run.progress.total} replications`;
}

/** A finished run's results: the report, the 2D animation, the 3D placeholder
 *  and downloads. `view` is "report", "animation" or "3d"; `onView` switches. */
function runResults(container, run, view, onView) {
  const file = name => `${API}/runs/${run.id}/files/${name}`;
  if (run.status !== "done") {
    container.replaceChildren(el("p", { class: "note" }, run.status === "failed" ? `This run failed: ${run.error}` : `This run is ${run.status}.`));
    return;
  }
  const tab = (name, label, title) => el("button", {
    class: view === name ? "primary" : null, title,
    onclick: () => { stopAnimation(); onView(name); },
  }, label);
  const downloads = ["summary.json", "replications.csv", "events.jsonl", "layout.json", "model.yaml"]
    .filter(f => run.files.includes(f))
    .map(f => el("a", { href: file(f), download: f }, f));
  const body = el("div");
  container.replaceChildren(
    el("div", { class: "results-bar" },
      tab("report", "Report"), tab("animation", "Animation"), tab("3d", "3D", "3D animation is planned"),
      el("span", { class: "spacer" }), ...downloads),
    body);
  if (view === "report") {
    body.append(el("iframe", { class: "report", src: file("report.html"), title: "Report" }));
  } else if (view === "animation" && !run.files.includes("events.jsonl")) {
    body.append(el("div", { class: "card note", id: "no-animation" },
      el("h3", { style: "margin-top: 0" }, "Nothing to animate in this run"),
      el("p", {}, "The animation replays what happened in the first replication, and this model doesn't record it."),
      el("p", {}, "To turn it on, open the project's Modeling page, check “Record an animation” under Definitions → Experiment, and run again. (In the model file it's ", el("code", {}, "outputs: { event_log: jsonl }"), ".)")));
  } else if (view === "animation") {
    startAnimation(body, run);
  } else {
    body.append(el("div", { class: "card note" },
      el("h3", { style: "margin-top: 0" }, "3D animation: planned"),
      el("p", {}, "A 3D view (the Godot viewer, exported for the web) will open here. It downloads about 8–10 MB the first time, so it will load only when you ask for it."),
      el("p", {}, "Until then, the 2D animation shows the same replay.")));
  }
}

// --- 2D animation ----------------------------------------------------------------------------------

/** Rebuilds the simulation state at any time from the event log, with the
 *  engine's conveyor rules (a port of viewer/scripts/replay.gd). */
class Replay {
  constructor(header, events) {
    this.header = header;
    this.events = events;
    this.end = header.end;
    this.warmup = header.warmup;
    this.reset();
  }

  reset() {
    this.conveyors = {};
    // Transports also have parts waiting for a vehicle (`pickup`).
    for (const c of this.header.conveyors) this.conveyors[c.name] = { ...c, belt: [], pickup: [] };
    // Vehicles: where each one is going, from where, and what it carries.
    this.vehicles = {};
    for (const f of this.header.vehicles || []) {
      this.vehicles[f.name] = Array.from({ length: f.count }, () => ({ from: f.home, to: f.home, start: 0, until: 0, part: null }));
    }
    this.servers = {};
    // waiting: parts collected into a batch or kit, not yet processing.
    for (const s of this.header.servers) this.servers[s.name] = { busy: [], blocked: [], waiting: [], failed: "", off: false };
    this.created = this.lost = this.exited = this.assembled = this.consumed = this.split = 0;
    this.types = new Map();   // part -> index into header.entities, while it's in the system
    this.time = 0;
    this.cursor = 0;
  }

  seek(t) {
    t = Math.max(0, Math.min(t, this.end));
    if (t < this.time) this.reset();
    while (this.cursor < this.events.length && this.events[this.cursor].t <= t) this.apply(this.events[this.cursor++]);
    this.time = t;
  }

  apply(e) {
    switch (e.type) {
      case "created":
        this.created++;
        this.types.set(e.part, Math.max(0, (this.header.entities || []).indexOf(e.entity)));
        break;
      case "lost": this.lost++; this.types.delete(e.part); break;
      case "gave_up": {
        // Left a queue after waiting too long.
        const c = this.conveyors[e.conveyor];
        c.belt = c.belt.filter(item => item.part !== e.part);
        this.lost++;
        this.types.delete(e.part);
        break;
      }
      case "stocked":
        // In a queue when the run starts.
        this.created++;
        this.types.set(e.part, Math.max(0, (this.header.entities || []).indexOf(e.entity)));
        this.conveyors[e.conveyor].belt.push({ part: e.part, pos: 0, since: e.t });
        break;
      case "entered_conveyor": {
        const c = this.conveyors[e.conveyor];
        const from = this.servers[c.from];
        if (from) from.blocked = from.blocked.filter(p => p !== e.part);
        if (c.transport) c.pickup.push(e.part);
        else c.belt.push({ part: e.part, pos: 0, since: e.t });
        break;
      }
      case "vehicle_trip": {
        const v = this.vehicles[e.fleet][e.vehicle];
        Object.assign(v, { from: e.from, to: e.to, start: e.t, until: e.until, part: e.part });
        if (e.part !== null) {
          for (const c of Object.values(this.conveyors)) c.pickup = c.pickup.filter(p => p !== e.part);
        }
        break;
      }
      case "delivered": {
        const c = this.conveyors[e.conveyor];
        c.belt.push({ part: e.part, pos: 0, since: e.t });
        for (const vs of Object.values(this.vehicles)) for (const v of vs) if (v.part === e.part) v.part = null;
        break;
      }
      case "started_process": {
        const s = this.servers[e.server];
        if (s.waiting.includes(e.part)) s.waiting = s.waiting.filter(p => p !== e.part);
        else this.leave(e.part, e.server, e.t);
        s.busy.push(e.part);
        break;
      }
      case "collected":
        this.leave(e.part, e.server, e.t);
        this.servers[e.server].waiting.push(e.part);
        break;
      case "consumed": {
        // A kit's component (collected), or a finished part that was split.
        const s = this.servers[e.server];
        s.waiting = s.waiting.filter(p => p !== e.part);
        s.blocked = s.blocked.filter(p => p !== e.part);
        this.types.delete(e.part);
        this.consumed++;
        break;
      }
      case "assembled":
        this.assembled++;
        this.types.set(e.part, Math.max(0, (this.header.entities || []).indexOf(e.entity)));
        this.servers[e.server].waiting.push(e.part);
        break;
      case "split":
        this.split++;
        this.types.set(e.part, Math.max(0, (this.header.entities || []).indexOf(e.entity)));
        this.servers[e.server].blocked.push(e.part);
        break;
      case "finished_process": {
        const s = this.servers[e.server];
        s.busy = s.busy.filter(p => p !== e.part);
        s.blocked.push(e.part);
        break;
      }
      case "failed": this.servers[e.server].failed = e.failure; break;
      case "repaired": this.servers[e.server].failed = ""; break;
      case "shift_ended": this.servers[e.server].off = true; break;
      case "shift_started": this.servers[e.server].off = false; break;
      case "exited": this.leave(e.part, e.sink, e.t); this.exited++; this.types.delete(e.part); break;
    }
  }

  leave(part, destination, t) {
    // The head of a conveyor; anywhere in a queue that serves by priority.
    for (const c of Object.values(this.conveyors)) {
      const at = c.to === destination ? c.belt.findIndex(item => item.part === part) : -1;
      if (at >= 0) {
        const positions = this.positions(c, t);
        c.belt.forEach((item, i) => { item.pos = positions[i]; item.since = t; });
        c.belt.splice(at, 1);
        return;
      }
    }
  }

  positions(c, t) {
    let limit = c.length;
    return c.belt.map(item => {
      const pos = Math.min(item.pos + c.speed * (t - item.since), limit);
      limit = pos - c.spacing;
      return pos;
    });
  }

  belt(name) {
    const c = this.conveyors[name];
    return this.positions(c, this.time);
  }

  /** The part type (index into the header's entities) of a part in the system. */
  typeOf(part) {
    return this.types.get(part) ?? 0;
  }

  state(server) {
    const s = this.servers[server];
    if (s.off) return "off";
    if (s.failed) return "failed";
    if (s.blocked.length) return "blocked";
    if (s.busy.length) return "busy";
    return "idle";
  }
}

let animation = null;

function stopAnimation() {
  if (animation) { cancelAnimationFrame(animation.frame); animation = null; }
}

async function startAnimation(container, run) {
  container.append(el("p", { class: "hint" }, "Loading the event log…"));
  const [events, layoutFile] = await Promise.all([
    fetch(`${API}/runs/${run.id}/files/events.jsonl`).then(r => r.text()),
    run.files.includes("layout.json") ? fetch(`${API}/runs/${run.id}/files/layout.json`).then(r => r.json()) : null,
  ]);
  const lines = events.split("\n").filter(Boolean).map(l => JSON.parse(l));
  const header = lines.shift();
  const replay = new Replay(header, lines);
  const graph = {
    sources: header.sources, servers: header.servers.map(s => s.name), sinks: header.sinks,
    conveyors: header.conveyors.map(c => ({ name: c.name, from: c.from, to: c.to, queue: Boolean(c.queue || c.transport) })),
  };
  const layout = layoutOf(graph, layoutFile);
  const paths = {};
  for (const c of header.conveyors) {
    const points = layout.paths[c.name];
    const cumulative = [0];
    for (let i = 1; i < points.length; i++) {
      cumulative.push(cumulative[i - 1] + Math.hypot(points[i][0] - points[i - 1][0], points[i][1] - points[i - 1][1]));
    }
    const drawn = cumulative[cumulative.length - 1];
    // Queues have no length: their parts line up back from the end, drawn.
    const straight = Boolean(c.queue || c.transport);
    paths[c.name] = { points, cumulative, queue: straight, drawn, scale: straight ? 1 : drawn / c.length };
  }

  const canvas = el("canvas", { "aria-label": "Animation of the run" });
  const clockText = el("div", { class: "clock" });
  const phase = el("div"), counts = el("div");
  const play = el("button", { class: "primary", style: "min-width: 80px" }, "Pause");
  const speed = el("select", { "aria-label": "Speed" }, SPEEDS.map((s, i) => el("option", { value: i, selected: s === 60 }, `${s}×`)));
  const slider = el("input", { type: "range", min: 0, max: header.end, step: "any", value: 0, "aria-label": "Time" });
  const fit = el("button", {}, "Fit");
  stopAnimation();
  container.replaceChildren(
    el("div", { class: "animation" }, canvas,
      el("div", { class: "overlay" }, clockText, phase, counts),
      el("div", { class: "legend" }, STATES
        .filter(([s]) => s !== "off" || header.servers.some(x => x.schedule))
        .map(([s, label]) => el("span", {}, el("i", { style: `background: var(--${s})` }), label)),
        (header.entities || []).length > 1
          ? header.entities.map((t, i) => el("span", {}, el("i", { style: `background: var(--type-${i % TYPE_COLORS})` }), t))
          : null)),
    el("div", { class: "player" }, play, speed, slider, fit));

  const a = { frame: 0, playing: true, speedIndex: SPEEDS.indexOf(60), view: null, last: performance.now() };
  animation = a;
  const b = boundsOf(layout);
  const fitView = () => {
    const w = canvas.clientWidth, h = canvas.clientHeight;
    const scale = Math.min(w / (b.maxX - b.minX), h / (b.maxZ - b.minZ)) * 0.92;
    a.view = { scale, x: w / 2 - scale * (b.minX + b.maxX) / 2, y: h / 2 - scale * (b.minZ + b.maxZ) / 2 };
  };
  play.onclick = () => {
    if (replay.time >= replay.end) replay.seek(0);
    a.playing = !a.playing;
  };
  speed.onchange = () => (a.speedIndex = Number(speed.value));
  slider.oninput = () => replay.seek(Number(slider.value));
  fit.onclick = fitView;

  // Zoom with the wheel or a pinch, pan by dragging.
  const pointers = new Map();
  const zoomAt = (factor, px, py) => {
    a.view.x = px - (px - a.view.x) * factor;
    a.view.y = py - (py - a.view.y) * factor;
    a.view.scale *= factor;
  };
  canvas.addEventListener("wheel", e => {
    e.preventDefault();
    const r = canvas.getBoundingClientRect();
    zoomAt(e.deltaY < 0 ? 1.15 : 1 / 1.15, e.clientX - r.left, e.clientY - r.top);
  }, { passive: false });
  canvas.addEventListener("pointerdown", e => { canvas.setPointerCapture(e.pointerId); pointers.set(e.pointerId, [e.clientX, e.clientY]); });
  canvas.addEventListener("pointerup", e => pointers.delete(e.pointerId));
  canvas.addEventListener("pointercancel", e => pointers.delete(e.pointerId));
  canvas.addEventListener("pointermove", e => {
    const before = pointers.get(e.pointerId);
    if (!before) return;
    if (pointers.size === 2) {
      const other = [...pointers.entries()].find(([id]) => id !== e.pointerId)[1];
      const d0 = Math.hypot(before[0] - other[0], before[1] - other[1]);
      const d1 = Math.hypot(e.clientX - other[0], e.clientY - other[1]);
      const r = canvas.getBoundingClientRect();
      if (d0 > 0) zoomAt(d1 / d0, (e.clientX + other[0]) / 2 - r.left, (e.clientY + other[1]) / 2 - r.top);
    } else {
      a.view.x += e.clientX - before[0];
      a.view.y += e.clientY - before[1];
    }
    pointers.set(e.pointerId, [e.clientX, e.clientY]);
  });
  canvas.addEventListener("dblclick", fitView);

  const pointOn = (path, meters) => {
    const d = meters * path.scale;
    const { points, cumulative } = path;
    for (let i = 1; i < points.length; i++) {
      if (d <= cumulative[i] || i === points.length - 1) {
        const seg = cumulative[i] - cumulative[i - 1];
        const f = seg > 0 ? Math.max(0, Math.min(1, (d - cumulative[i - 1]) / seg)) : 0;
        return [points[i - 1][0] + (points[i][0] - points[i - 1][0]) * f, points[i - 1][1] + (points[i][1] - points[i - 1][1]) * f];
      }
    }
    return points[points.length - 1];
  };

  const draw = now => {
    if (animation !== a) return;
    const dpr = window.devicePixelRatio || 1;
    const w = canvas.clientWidth, h = canvas.clientHeight;
    if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
      if (!a.view) fitView();
    }
    if (a.playing) {
      replay.seek(replay.time + (now - a.last) / 1000 * SPEEDS[a.speedIndex]);
      if (replay.time >= replay.end) a.playing = false;
    }
    a.last = now;
    const ctx = canvas.getContext("2d");
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const { scale, x: ox, y: oy } = a.view;
    const X = x => ox + x * scale, Y = z => oy + z * scale;
    const colors = Object.fromEntries(["belt", "object", "part", "ink-2", "busy", "blocked", "failed", "idle", "off", "surface"].map(n => [n, cssVar(`--${n}`)]));
    const typed = (header.entities || []).length > 1;
    const typeColors = [...Array(TYPE_COLORS).keys()].map(i => cssVar(`--type-${i}`));
    const partColor = part => typed ? typeColors[replay.typeOf(part) % TYPE_COLORS] : colors.part;

    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.strokeStyle = colors.belt;
    ctx.lineWidth = Math.max(3, 0.6 * scale);
    for (const path of Object.values(paths)) {
      ctx.setLineDash(path.queue ? [0.3 * scale, 0.3 * scale] : []);
      ctx.lineWidth = path.queue ? Math.max(1.5, 0.15 * scale) : Math.max(3, 0.6 * scale);
      ctx.beginPath();
      path.points.forEach(([x, z], i) => (i ? ctx.lineTo(X(x), Y(z)) : ctx.moveTo(X(x), Y(z))));
      ctx.stroke();
    }
    ctx.setLineDash([]);
    const s = OBJECT_SIZE * scale;
    ctx.font = "12px system-ui, sans-serif";
    ctx.textAlign = "center";
    for (const [name, [x, z]] of Object.entries(layout.positions)) {
      const server = replay.servers[name];
      ctx.fillStyle = server ? colors[replay.state(name)] : colors.object;
      ctx.fill(new Path2D(shapePath(OBJECT_SHAPES[server ? "servers" : graph.sinks.includes(name) ? "sinks" : "sources"], X(x), Y(z), s)));
      ctx.fillStyle = colors["ink-2"];
      ctx.fillText(server?.failed ? `${name} (${server.failed})` : name, X(x), Y(z) - s / 2 - 6);
    }
    const p = Math.max(4, 0.32 * scale);
    ctx.fillStyle = colors.part;
    ctx.strokeStyle = colors.surface;
    ctx.lineWidth = 1;
    for (const [name, path] of Object.entries(paths)) {
      const items = replay.conveyors[name].belt;
      const step = 0.45;
      replay.belt(name).forEach((meters, i) => {
        // A queue's parts stand one behind the other from its end; those that
        // don't fit the drawing pile up at its start.
        const [x, z] = path.queue ? pointOn(path, Math.max(0, path.drawn - i * step)) : pointOn(path, meters);
        ctx.fillStyle = partColor(items[i].part);
        ctx.fillRect(X(x) - p / 2, Y(z) - p / 2, p, p);
        ctx.strokeRect(X(x) - p / 2, Y(z) - p / 2, p, p);
      });
    }
    // Parts waiting for a vehicle, beside where they'll be picked up.
    for (const [name, path] of Object.entries(paths)) {
      const waiting = replay.conveyors[name].pickup;
      waiting.forEach((part, i) => {
        const [x, z] = pointOn(path, Math.min(path.drawn, i * 0.45));
        ctx.fillStyle = partColor(part);
        ctx.fillRect(X(x) - p / 2, Y(z) - p / 2, p, p);
        ctx.strokeRect(X(x) - p / 2, Y(z) - p / 2, p, p);
      });
    }
    // Vehicles, between objects, with what they carry.
    const vs = Math.max(6, 0.55 * scale);
    for (const fleet of Object.values(replay.vehicles)) {
      for (const v of fleet) {
        const a = layout.positions[v.from], b = layout.positions[v.to];
        if (!a || !b) continue;
        const f = v.until > v.start ? Math.max(0, Math.min(1, (replay.time - v.start) / (v.until - v.start))) : 1;
        const [x, z] = [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f + 0.9];
        ctx.fillStyle = colors["ink-2"];
        ctx.fillRect(X(x) - vs / 2, Y(z) - vs / 2, vs, vs);
        if (v.part !== null) {
          ctx.fillStyle = partColor(v.part);
          ctx.fillRect(X(x) - p / 2, Y(z) - p / 2 - vs / 2, p, p);
        }
      }
    }
    for (const [name, sv] of Object.entries(replay.servers)) {
      const [x, z] = layout.positions[name];
      [...sv.busy, ...sv.blocked, ...sv.waiting].forEach((part, i) => {
        ctx.fillStyle = partColor(part);
        ctx.fillRect(X(x) - p / 2 + i * (p + 2), Y(z) - p / 2, p, p);
        ctx.strokeRect(X(x) - p / 2 + i * (p + 2), Y(z) - p / 2, p, p);
      });
    }

    clockText.textContent = clock(replay.time);
    phase.textContent = replay.time < replay.warmup ? `warm-up (statistics start at ${clock(replay.warmup)})` : `measuring · replication ${header.replication}`;
    const inSystem = replay.created + replay.assembled + replay.split - replay.consumed - replay.lost - replay.exited;
    counts.textContent = `${replay.created} created · ${inSystem} in system · ${replay.exited} exited` + (replay.lost ? ` · ${replay.lost} lost` : "") + (replay.assembled ? ` · ${replay.assembled} assembled` : "");
    play.textContent = a.playing ? "Pause" : "Play";
    if (document.activeElement !== slider) slider.value = replay.time;
    a.frame = requestAnimationFrame(draw);
  };
  a.frame = requestAnimationFrame(draw);
}
