"use strict";
// Shared by the SimuLancer pages (/, /modeling, /simulation). Uses only the public
// HTTP API (docs/api.md).

const API = "api/v1";
const SPEEDS = [1, 10, 60, 300, 1800, 3600];
const STATES = [["busy", "Busy"], ["blocked", "Blocked (output full)"], ["failed", "Failed"], ["idle", "Idle (starved)"], ["off", "Off shift"]];
const OUTPUT_RULES = ["probability", "round_robin", "first_available", "shortest_queue", "by_type", "by_condition"];
const INPUT_RULES = ["first_come", "priority", "round_robin"];
const OBJECT_SIZE = 1.2;
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
    units: Object.fromEntries(Object.entries(tree.servers || {}).map(([n, s]) => [n, unitsOf(s?.capacity, s?.batch)])),
  };
}

/** How many units a server is drawn as: one per batch (or part) it works on at
 *  once. A capacity that isn't a number yet counts as one. */
function unitsOf(capacity, batch) {
  const b = Number(batch) > 0 ? Number(batch) : 1;
  const c = capacity === undefined || capacity === null ? b : Number(capacity);
  return Number.isFinite(c) ? Math.max(1, Math.floor(c / b)) : 1;
}

/** A queue's path: straight between its objects' edges, moved 0.5 m to the
 *  right of the way parts go when a queue runs the other way (same as the
 *  server's layout). */
function queuePath(a, b, reverse) {
  const edge = OBJECT_SIZE / 2 + 0.1;
  const [dx, dz] = [b[0] - a[0], b[1] - a[1]];
  const len = Math.hypot(dx, dz);
  if (len <= 2 * edge) return [a, [a[0] + 0.01, a[1]]];
  const [ux, uz] = [dx / len, dz / len];
  const [ox, oz] = reverse ? [-uz * 0.5, ux * 0.5] : [0, 0];
  return [[a[0] + ux * edge + ox, a[1] + uz * edge + oz], [b[0] - ux * edge + ox, b[1] - uz * edge + oz]];
}

/** Draws every queue between its objects' current positions. */
function placeQueues(graph, positions, paths) {
  for (const c of graph.conveyors.filter(c => c.queue)) {
    const reverse = graph.conveyors.some(o => o.queue && o.from === c.to && o.to === c.from);
    if (positions[c.from] && positions[c.to]) paths[c.name] = queuePath(positions[c.from], positions[c.to], reverse);
  }
}

/** Positions (x, z in meters), conveyor paths and what objects, vehicles and
 *  part types look like (`drawn`, `fleets`, `types`: name -> the layout's
 *  `model`), from layout.json or computed. */
function layoutOf(graph, layout) {
  const drawn = {}, fleets = {}, types = {}, crews = {};
  for (const [name, o] of Object.entries(layout?.objects || {})) if (o.model) drawn[name] = o.model;
  for (const [name, f] of Object.entries(layout?.vehicles || {})) if (f.model) fleets[name] = f.model;
  for (const [name, t] of Object.entries(layout?.entities || {})) if (t.model) types[name] = t.model;
  // Resources drawn as people: where their units wait while free, and their look.
  for (const [name, r] of Object.entries(layout?.resources || {})) crews[name] = { at: [r.position[0], r.position[2]], model: r.model };
  if (layout && graph.conveyors.every(c => c.queue || layout.conveyors[c.name]) &&
      [...graph.sources, ...graph.servers, ...graph.sinks].every(n => layout.objects[n])) {
    const positions = {};
    for (const [name, o] of Object.entries(layout.objects)) positions[name] = [o.position[0], o.position[2]];
    const paths = {};
    for (const [name, c] of Object.entries(layout.conveyors)) paths[name] = c.path.map(p => [p[0], p[2]]);
    placeQueues(graph, positions, paths);
    return { positions, paths, drawn, fleets, types, crews };
  }
  return { ...autoLayout(graph), drawn, fleets, types, crews };
}

/** The section ("sources", "servers" or "sinks") an object is in. */
const sectionIn = (graph, name) => graph.sources.includes(name) ? "sources" : graph.servers.includes(name) ? "servers" : "sinks";

/** What an object looks like (see art.js): a server working on several parts
 *  at once is that many units. */
function lookOf(graph, layout, name) {
  const look = artFor(sectionIn(graph, name), layout.drawn?.[name]);
  const units = graph.units?.[name] || 1;
  return artGroups(look, units) ? { ...look, units } : look;
}

/** A connection's path as drawn: cut where it runs under the objects at its
 *  ends, so a belt stops at a machine's edge and its parts stay in sight. */
function drawnPath(graph, layout, c, points) {
  if (c.transport) return transportPath(graph, layout, c);
  const footprint = name => {
    const p = layout.positions[name];
    return p && artFootprint(lookOf(graph, layout, name), p[0], p[1]);
  };
  const cut = cutUnder(points, footprint(c.from));
  return cutUnder([...cut].reverse(), footprint(c.to)).reverse();
}

/** Where vehicles stop at an object: on the floor in front of it. */
function stopAt(graph, layout, name) {
  const [x, z] = layout.positions[name];
  return [x, artFootprint(lookOf(graph, layout, name), x, z).z1 + 0.85];
}

// --- resources drawn as people ---------------------------------------------------------

const WALK_SPEED = 1.4;   // m/s
const LINGER = 120;       // s a free unit stays where it worked before going back
const MAX_CREW = 24;      // units drawn at most

/** Where `n` units of a resource wait while free, around `at`: rows of up to six. */
function crewSpots(at, n) {
  const cols = Math.min(6, n);
  return Array.from({ length: n }, (_, i) => [at[0] + ((i % cols) - (cols - 1) / 2) * 0.7, at[1] + Math.floor(i / cols) * 0.7]);
}

/** Which unit of a server's drawing works in `slot`. */
function unitOfSlot(info, units, slot) {
  const per = Math.max(1, Math.ceil((info?.capacity || 1) / units));
  return Math.min(units - 1, Math.floor(slot / per));
}

/** Where each unit of the resources the layout draws goes, from the event
 *  log's resource_taken and resource_returned. The engine doesn't move
 *  resources, so this is only a picture: the nearest free unit walks to the
 *  server's crew spot (on the unit working the part) while the work goes on,
 *  stays there while free, and goes back to its spot in the layout after
 *  LINGER without work. Returns { resource: [{ home, legs }] }, legs being
 *  { t, from, to, dur } in time order. */
function planWalks(header, events, graph, layout) {
  const plans = {};
  for (const r of header.resources || []) {
    if (!layout.crews?.[r.name]) continue;
    plans[r.name] = crewSpots(layout.crews[r.name].at, Math.min(r.capacity, MAX_CREW))
      .map(home => ({ home, legs: [], job: null, idleSince: -Infinity, atHome: true }));
  }
  if (!Object.keys(plans).length) return plans;
  const looks = {};
  // The part's slot comes with its started_process, just after.
  const spotFor = (server, part, i) => {
    let slot = 0;
    for (let k = i + 1; k < Math.min(events.length, i + 200); k++) {
      const e = events[k];
      if (e.type === "started_process" && e.part === part && e.server === server) { slot = e.slot ?? 0; break; }
    }
    const look = looks[server] ||= lookOf(graph, layout, server);
    const [x, z] = layout.positions[server];
    const units = artUnits(look, x, z);
    return artCrew(look, ...units[unitOfSlot(header.servers.find(s => s.name === server), units.length, slot)]);
  };
  const walk = (u, t, to) => {
    const from = walkerAt(u, t);
    u.legs.push({ t, from, to, dur: Math.hypot(to[0] - from[0], to[1] - from[1]) / WALK_SPEED });
  };
  const goHome = (u, now) => {
    if (u.job || u.atHome || u.idleSince + LINGER > now) return;
    walk(u, u.idleSince + LINGER, u.home);
    u.atHome = true;
  };
  events.forEach((e, i) => {
    const units = plans[e.resource];
    if (!units) return;
    const job = `${e.server}/${e.part}`;
    if (e.type === "resource_taken") {
      if (!layout.positions[e.server]) return;
      const target = spotFor(e.server, e.part, i);
      units.forEach(u => goHome(u, e.t));
      for (let k = 0; k < e.units; k++) {
        const free = units.filter(u => !u.job);
        if (!free.length) break;
        const away = u => { const [x, z] = walkerAt(u, e.t); return Math.hypot(x - target[0], z - target[1]); };
        const u = free.reduce((a, b) => (away(b) < away(a) ? b : a));
        walk(u, e.t, [target[0] + k * 0.5, target[1]]);
        Object.assign(u, { job, atHome: false });
      }
    } else if (e.type === "resource_returned") {
      for (const u of units) if (u.job === job) Object.assign(u, { job: null, idleSince: e.t });
    }
  });
  for (const units of Object.values(plans)) units.forEach(u => goHome(u, Infinity));
  return plans;
}

/** Where a planned unit is at time `t`. */
function walkerAt(u, t) {
  let lo = 0, hi = u.legs.length - 1, k = -1;
  while (lo <= hi) {
    const m = (lo + hi) >> 1;
    if (u.legs[m].t <= t) { k = m; lo = m + 1; } else hi = m - 1;
  }
  if (k < 0) return u.home;
  const l = u.legs[k];
  const f = l.dur > 0 ? Math.min(1, (t - l.t) / l.dur) : 1;
  return [l.from[0] + (l.to[0] - l.from[0]) * f, l.from[1] + (l.to[1] - l.from[1]) * f];
}

/** Where the `i`th of `n` vehicles parks, from where they stop at an object:
 *  side by side, up to three abreast, more rows in front. [dx, dz] in meters. */
function parkOffset(look, i, n) {
  const cols = Math.min(3, n), col = i % cols, row = Math.floor(i / cols);
  return [(col - (cols - 1) / 2) * (look.art.w + 0.3), row * (look.art.d + 1.2)];
}

/** A transport's route: straight between where vehicles stop at its objects,
 *  moved 0.6 m to the right of the way parts go when another transport runs the
 *  other way. */
function transportPath(graph, layout, c) {
  const [a, b] = [stopAt(graph, layout, c.from), stopAt(graph, layout, c.to)];
  const len = Math.hypot(b[0] - a[0], b[1] - a[1]);
  if (len < 0.01) return [a, [a[0] + 0.01, a[1]]];
  const reverse = graph.conveyors.some(o => o.transport && o.from === c.to && o.to === c.from);
  const [ox, oz] = reverse ? [-(b[1] - a[1]) / len * 0.6, (b[0] - a[0]) / len * 0.6] : [0, 0];
  return [[a[0] + ox, a[1] + oz], [b[0] + ox, b[1] + oz]];
}

/** The path from where it leaves the rectangle `r` it starts in (unchanged if it
 *  doesn't start in it or never leaves it). */
function cutUnder(points, r) {
  const inside = ([x, z]) => r && x > r.x0 && x < r.x1 && z > r.z0 && z < r.z1;
  if (!inside(points[0])) return points;
  const k = points.findIndex(p => !inside(p));
  if (k < 0) return points;
  const [a, b] = [points[k - 1], points[k]];
  const [dx, dz] = [b[0] - a[0], b[1] - a[1]];
  const t = Math.min(
    dx > 0 ? (r.x1 - a[0]) / dx : dx < 0 ? (r.x0 - a[0]) / dx : Infinity,
    dz > 0 ? (r.z1 - a[1]) / dz : dz < 0 ? (r.z0 - a[1]) / dz : Infinity);
  return [[a[0] + dx * t, a[1] + dz * t], ...points.slice(k)];
}

/** Columns by distance from the sources; backward conveyors loop around the top. */
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
  // Columns 10 m apart and objects 6 m apart in a column, more where a server
  // drawn as several units needs the room.
  const size = n => artBox(lookOf(graph, {}, n), 0, 0);
  const positions = {};
  let at = 0, before = null;
  for (const [d, names] of Object.entries(columns)) {
    const w = Math.max(...names.map(n => size(n).w));
    if (before !== null) at += Math.max(10, (before + w) / 2 + 6);
    before = w;
    const pitch = Math.max(6, ...names.map(n => size(n).h + 3));
    names.forEach((n, i) => (positions[n] = [at, (i - (names.length - 1) / 2) * pitch]));
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

/** What a layout covers, with room around it: its objects' drawings (and a
 *  label above each) when `graph` is given, else their positions. */
function boundsOf(layout, graph) {
  const points = [...Object.values(layout.positions), ...Object.values(layout.paths).flat(), ...Object.values(layout.crews || {}).flatMap(c => [c.at, [c.at[0], c.at[1] - 1.9]])];
  if (graph) {
    for (const [name, [x, z]] of Object.entries(layout.positions)) {
      const b = artBox(lookOf(graph, layout, name), x, z);
      points.push([b.x, b.y - 1], [b.x + b.w, b.y + b.h]);
    }
  }
  const xs = points.map(p => p[0]), zs = points.map(p => p[1]);
  const m = graph ? 1 : 2;
  return { minX: Math.min(...xs) - m, maxX: Math.max(...xs) + m, minZ: Math.min(...zs) - (graph ? m : 2.5), maxZ: Math.max(...zs) + m };
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
  document.title = `${name} · ${page} · SimuLancer`;
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

/** A finished run's results: the report, the animation and downloads.
 *  `view` is "report" or "animation"; `onView` switches. */
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
  const downloads = ["summary.json", "replications.csv", "replications.parquet", "events.jsonl", "events.parquet", "layout.json", "model.yaml"]
    .filter(f => run.files.includes(f))
    .map(f => el("a", { href: file(f), download: f }, f));
  const body = el("div");
  container.replaceChildren(
    el("div", { class: "results-bar" },
      tab("report", "Report"), tab("animation", "Animation"),
      el("span", { class: "spacer" }), ...downloads),
    body);
  if (view === "report") {
    const report = el("iframe", { class: "report", src: file("report.html"), title: "Report" });
    report.addEventListener("load", () => syncReportMode(report));
    body.append(report);
  } else if (view === "animation" && !run.files.includes("events.jsonl")) {
    body.append(el("div", { class: "card note", id: "no-animation" },
      el("h3", { style: "margin-top: 0" }, "Nothing to animate in this run"),
      el("p", {}, run.files.includes("events.parquet")
        ? "The animation replays what happened in the first replication. This run saved those events for analysis (events.parquet), but the animation needs its own log."
        : "The animation replays what happened in the first replication, and this model doesn't record it."),
      el("p", {}, "To turn it on, open the project's Modeling page, check “Record an animation” under Definitions → Experiment, and run again. (In the model file it's ", el("code", {}, "outputs: { event_log: jsonl }"), ".)")));
  } else {
    startAnimation(body, run);
  }
}

// --- 2D animation ----------------------------------------------------------------------------------

/** Rebuilds the simulation state at any time from the event log, with the
 *  engine's conveyor rules; tests/replay/test_replay.py checks it against the engine. */
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
    this.slots = new Map();   // part -> the server slot it's processed in, when the log says
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
        if (e.slot !== undefined) this.slots.set(e.part, e.slot);
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
        this.slots.delete(e.part);
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
      case "exited": this.leave(e.part, e.sink, e.t); this.exited++; this.types.delete(e.part); this.slots.delete(e.part); break;
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
    units: Object.fromEntries(header.servers.map(s => [s.name, unitsOf(s.capacity, s.batch)])),
    conveyors: header.conveyors.map(c => ({ name: c.name, from: c.from, to: c.to, queue: Boolean(c.queue || c.transport), transport: Boolean(c.transport) })),
  };
  const layout = layoutOf(graph, layoutFile);
  const walks = planWalks(header, lines, graph, layout);
  const paths = {};
  for (const c of header.conveyors) {
    const points = drawnPath(graph, layout, c, layout.paths[c.name]);
    const cumulative = [0];
    for (let i = 1; i < points.length; i++) {
      cumulative.push(cumulative[i - 1] + Math.hypot(points[i][0] - points[i - 1][0], points[i][1] - points[i - 1][1]));
    }
    const drawn = cumulative[cumulative.length - 1];
    // Queues have no length: their parts line up back from the end, drawn.
    const straight = Boolean(c.queue || c.transport);
    paths[c.name] = { points, cumulative, queue: straight, transport: Boolean(c.transport), drawn, scale: straight ? 1 : drawn / c.length };
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

  const typeLooks = (header.entities || ["part"]).map(t => artFor("entities", layout.types?.[t]));
  // Which way each vehicle faces (true: left), kept while it stands still.
  const facing = new Map();
  // Which unit of a server each part is on, kept while it's there, so a part
  // doesn't move when another leaves: server → [part or null per slot].
  const slots = new Map();
  const a = { frame: 0, playing: true, speedIndex: SPEEDS.indexOf(60), view: null, last: performance.now() };
  animation = a;
  const b = boundsOf(layout, graph);
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
    const colors = Object.fromEntries(["object", "muted", "ink-2", "busy", "blocked", "failed", "idle", "off", "surface"].map(n => [n, cssVar(`--${n}`)]));
    const typed = (header.entities || []).length > 1;
    const typeColors = [...Array(TYPE_COLORS).keys()].map(i => cssVar(`--type-${i}`));
    // The sticker that tells part types apart, where there is more than one.
    const typeOf = part => typed ? typeColors[replay.typeOf(part) % TYPE_COLORS] : null;
    // What a part looks like: its type's look in the layout (a box by default).
    const partLook = part => typeLooks[replay.typeOf(part)] || typeLooks[0];

    // Floor first: queue lanes, transport routes and belts.
    for (const path of Object.values(paths)) {
      if (path.transport) {
        ctx.strokeStyle = colors.muted;
        ctx.lineWidth = Math.max(1.5, 0.12 * scale);
        ctx.lineCap = "round";
        ctx.setLineDash([0.08 * scale, 0.4 * scale]);
        ctx.beginPath();
        path.points.forEach(([x, z], i) => (i ? ctx.lineTo(X(x), Y(z)) : ctx.moveTo(X(x), Y(z))));
        ctx.stroke();
        ctx.setLineDash([]);
      } else {
        artLineCanvas(ctx, path.points, path.queue ? ART_LANE : ART_BELT, X, Y, scale);
      }
    }
    // Then everything standing on it, back to front, so nearer things go over
    // farther ones: objects (with the parts they hold), parts on belts and in
    // queues, vehicles.
    const things = [];
    const waiting = [];   // [where, how many more] for queues longer than their drawing
    const part = (x, z, lift, id) => things.push([z, () => artPartCanvas(ctx, partLook(id), x, z, lift, typeOf(id), X, Y, scale)]);
    const looks = Object.fromEntries(Object.keys(layout.positions).map(n => [n, lookOf(graph, layout, n)]));
    for (const name of Object.keys(layout.positions)) {
      const [x, z] = layout.positions[name];
      const server = replay.servers[name];
      if (!server) {
        const f = artFootprint(looks[name], x, z);
        things.push([(f.z0 + f.z1) / 2, () => artCanvas(ctx, looks[name], x, z, colors.object, X, Y, scale)]);
        continue;
      }
      // Each unit has its own parts and state. Off shift or failed is the
      // whole server's. Staff that a resource stands for (`needs`) are there
      // only while the unit works, or not at all where the resource is drawn
      // as people walking to it.
      const unit = artUnit(looks[name]);
      const units = artUnits(looks[name], x, z);
      const info = header.servers.find(s => s.name === name);
      const working = [...server.busy, ...server.blocked];
      const on = units.map(() => []);
      if (working.every(p => replay.slots.has(p))) {
        // The log says which slot each part is in.
        for (const p of working) on[unitOfSlot(info, units.length, replay.slots.get(p))].push(p);
      } else {
        // Older logs: keep each part on the unit it was first drawn on.
        const kept = (slots.get(name) || []).map(p => working.includes(p) ? p : null);
        for (const p of working) {
          if (kept.includes(p)) continue;
          const free = kept.indexOf(null);
          if (free >= 0) kept[free] = p; else kept.push(p);
        }
        slots.set(name, kept);
        const perUnit = Math.max(1, Math.ceil(Math.max(kept.length, info?.capacity || 1) / units.length));
        kept.forEach((p, i) => p !== null && on[Math.min(units.length - 1, Math.floor(i / perUnit))].push(p));
      }
      server.waiting.forEach((p, i) => on[i % units.length].push(p));
      const walked = info?.needs?.length > 0 && info.needs.every(r => walks[r]);
      const whole = replay.state(name);
      units.forEach(([ux, uz], k) => {
        const f = artFootprint(unit, ux, uz);
        const held = on[k];
        const state = whole === "off" || whole === "failed" ? whole
          : held.some(p => server.blocked.includes(p)) ? "blocked"
          : held.some(p => server.busy.includes(p)) ? "busy" : "idle";
        const staffed = state !== "off" && !walked && (!info?.needs?.length || held.length > 0);
        things.push([(f.z0 + f.z1) / 2, () => artCanvas(ctx, unit, ux, uz, colors[state], X, Y, scale, staffed)]);
        // Parts in a server sit on top of it, side by side; people stand in front.
        const step = Math.max(0.6, 9 / scale);
        held.forEach((id, i) => {
          const look = partLook(id);
          const at = ux + (i - (held.length - 1) / 2) * Math.max(step, look.art.w + 0.1);
          if (look.art.stands) part(at, f.z1 + 0.3, 0, id);
          else things.push([(f.z0 + f.z1) / 2 + 0.01, () => artPartCanvas(ctx, look, at, uz, unit.art.h / 2, typeOf(id), X, Y, scale)]);
        });
      });
    }
    // How much room a part takes in a line: its own size, and enough to tell
    // parts apart zoomed out.
    const room = id => Math.max(partLook(id).art.w + 0.1, 9 / scale);
    for (const [name, path] of Object.entries(paths)) {
      const items = replay.conveyors[name].belt;
      // On a transport, parts wait a little way along the route, clear of
      // where the vehicles stop: picked up at its start, delivered at its end.
      const clear = path.transport ? Math.min(1.8, path.drawn / 3) : 0;
      // A queue's parts stand one behind the other from its end. Those that
      // don't fit the drawing wait at its start: one is drawn there, with how
      // many more there are.
      let back = clear, more = 0;
      replay.belt(name).forEach((meters, i) => {
        const id = items[i].part;
        if (path.queue && path.drawn - back < 0) {
          if (more++ === 0) part(...pointOn(path, 0), 0, id);
          return;
        }
        const [x, z] = path.queue ? pointOn(path, path.drawn - back) : pointOn(path, meters);
        back += room(id);
        part(x, z, path.queue ? 0 : BELT_TOP, id);
      });
      if (more > 1) waiting.push([pointOn(path, 0), more - 1]);
      let ahead = clear;
      replay.conveyors[name].pickup.forEach(id => {
        const [x, z] = pointOn(path, Math.min(path.drawn, ahead));
        ahead += room(id);
        part(x, z, 0, id);
      });
    }
    // Resources drawn as people, where they are.
    for (const [name, units] of Object.entries(walks)) {
      const look = artFor("resources", layout.crews[name].model);
      for (const u of units) {
        const [wx, wz] = walkerAt(u, replay.time);
        things.push([wz, () => artCanvas(ctx, look, wx, wz, colors.object, X, Y, scale)]);
      }
    }
    // Vehicles drive between where they stop at objects, facing the way they
    // go, with what they carry. Each parks in its own spot beside the others,
    // so those at the same object don't cover each other.
    for (const [fleet, vehicles] of Object.entries(replay.vehicles)) {
      const look = artFor("vehicles", layout.fleets?.[fleet]);
      vehicles.forEach((v, i) => {
        if (!layout.positions[v.from] || !layout.positions[v.to]) return;
        const [a, b] = [stopAt(graph, layout, v.from), stopAt(graph, layout, v.to)];
        const f = v.until > v.start ? Math.max(0, Math.min(1, (replay.time - v.start) / (v.until - v.start))) : 1;
        const key = `${fleet}/${i}`;
        if (Math.abs(b[0] - a[0]) > 0.01) facing.set(key, b[0] < a[0]);
        const left = facing.get(key) ?? false;
        // Parked side by side, up to three abreast, more rows in front. On a
        // trip it pulls out of its spot onto the route over the first quarter
        // and into its spot at the other end over the last.
        const [px, pz] = parkOffset(look, i, vehicles.length);
        const parked = v.from === v.to || f >= 1 ? 1 : Math.max(0, 1 - 4 * f, 4 * f - 3);
        const x = a[0] + (b[0] - a[0]) * f + parked * px;
        const z = a[1] + (b[1] - a[1]) * f + parked * pz;
        things.push([z, () => {
          const turned = { ...look, mirror: left };
          artCanvas(ctx, turned, x, z, colors.object, X, Y, scale);
          if (v.part !== null) {
            const [lx, lz, lh] = look.art.load;
            const dx = lx - look.art.w / 2;
            artPartCanvas(ctx, partLook(v.part), x + (left ? -dx : dx), z + lz - look.art.d / 2, lh, typeOf(v.part), X, Y, scale, 5);
          }
        }]);
      });
    }
    things.sort((m, n) => m[0] - n[0]).forEach(([, draw]) => draw());
    // How many more wait where a queue is longer than its drawing.
    ctx.font = "600 11px system-ui, sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "bottom";
    for (const [[x, z], n] of waiting) {
      const text = `+${n}`;
      ctx.lineWidth = 3;
      ctx.strokeStyle = colors.surface;
      const above = Y(z) - Math.max(14, 1.1 * scale);
      ctx.strokeText(text, X(x), above);
      ctx.fillStyle = colors["ink-2"];
      ctx.fillText(text, X(x), above);
    }
    // Names above the drawings, and how many at once where a server takes more than one.
    ctx.font = "600 12px system-ui, sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "alphabetic";
    for (const name of Object.keys(layout.positions)) {
      const [x, z] = layout.positions[name];
      const box = artBox(looks[name], x, z);
      const server = replay.servers[name];
      const capacity = header.servers.find(s => s.name === name)?.capacity || 1;
      const label = (server?.failed ? `${name} (${server.failed})` : name) + (capacity > 1 && !looks[name].units ? ` ×${capacity}` : "");
      ctx.lineWidth = 3;
      ctx.strokeStyle = colors.surface;
      ctx.strokeText(label, X(box.x + box.w / 2), Y(box.y) - 6);
      ctx.fillStyle = colors["ink-2"];
      ctx.fillText(label, X(box.x + box.w / 2), Y(box.y) - 6);
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

// --- themes (web/theme.js) ---------------------------------------------------

// The report is a file of its own, with its own light and dark tokens: it follows the
// page's mode, not its class.
function syncReportMode(frame) {
  try {
    frame.contentDocument.documentElement.setAttribute("data-theme", document.documentElement.dataset.mode || "light");
  } catch (e) { /* not loaded yet */ }
}
document.addEventListener("themechange", () => document.querySelectorAll("iframe.report").forEach(syncReportMode));
document.querySelectorAll("[data-theme-picker]").forEach(btn => btn.addEventListener("click", () => THEME.picker(btn)));
