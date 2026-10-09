"use strict";
// outliers-sim web interface. Uses only the public HTTP API (docs/api.md).

const API = "api/v1";
const SPEEDS = [1, 10, 60, 300, 1800, 3600];
const STATES = [["busy", "Busy"], ["blocked", "Blocked (output full)"], ["failed", "Failed"], ["idle", "Idle (starved)"]];
const OUTPUT_RULES = ["probability", "round_robin", "first_available", "shortest_queue"];
const INPUT_RULES = ["first_come", "priority", "round_robin"];
const OBJECT_SIZE = 1.2;

const state = {
  models: [],
  runs: [],
  file: null,        // the model or scenario file opened
  kind: null,        // "model" | "scenario"
  base: null,        // base model file
  baseTree: null,    // the base model as a tree
  baseLayout: null,  // layout.json of the base, or null
  changes: {},       // change path -> value
  name: "",          // scenario name
  description: null,
  errors: [],        // from POST /check
  selected: null,    // { section, name } or null (experiment)
  tab: "model",      // "model" | "results"
  run: null,         // the run shown in the results tab
  view: "report",    // "report" | "animation" | "3d"
  message: null,     // { kind: "ok" | "error", text, list? }
};

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

/** The model with the current changes applied. */
function currentTree() {
  const tree = clone(state.baseTree);
  for (const [path, value] of Object.entries(state.changes)) setAt(tree, path, clone(value));
  return tree;
}

const related = (a, b) => a === b || a.startsWith(b + ".") || b.startsWith(a + ".");

/** Records a change, or removes it when the value is back to the base's. */
function setChange(path, value) {
  // A change to a parent path (e.g. a whole failures list) already covers this one.
  const parent = Object.keys(state.changes).find(k => path.startsWith(k + "."));
  if (parent) {
    const holder = { root: clone(state.changes[parent]) };
    setAt(holder, "root" + path.slice(parent.length), value);
    state.changes[parent] = holder.root;
    if (equal(holder.root, getAt(state.baseTree, parent))) delete state.changes[parent];
  } else {
    for (const k of Object.keys(state.changes)) if (k.startsWith(path + ".")) delete state.changes[k];
    if (equal(value, getAt(state.baseTree, path))) delete state.changes[path];
    else state.changes[path] = clone(value);
  }
  scheduleCheck();
  renderEditor();
}

function resetChange(path) {
  for (const k of Object.keys(state.changes)) if (related(k, path)) delete state.changes[k];
  scheduleCheck();
  renderEditor();
}

const isChanged = path => Object.keys(state.changes).some(k => related(k, path));
const errorFor = path => state.errors.find(e => e.change && related(e.change, path));

// --- layout ----------------------------------------------------------------------

/** The model's graph: object names by kind, and conveyors. */
function graphOf(tree) {
  return {
    sources: Object.keys(tree.sources || {}),
    servers: Object.keys(tree.servers || {}),
    sinks: Object.keys(tree.sinks || {}),
    conveyors: Object.entries(tree.conveyors || {}).map(([name, c]) => ({ name, from: c.from, to: c.to })),
  };
}

/** Positions (x, z in meters) and conveyor paths, from layout.json or computed. */
function layoutOf(graph, layout) {
  if (layout && graph.conveyors.every(c => layout.conveyors[c.name]) &&
      [...graph.sources, ...graph.servers, ...graph.sinks].every(n => layout.objects[n])) {
    const positions = {};
    for (const [name, o] of Object.entries(layout.objects)) positions[name] = [o.position[0], o.position[2]];
    const paths = {};
    for (const [name, c] of Object.entries(layout.conveyors)) paths[name] = c.path.map(p => [p[0], p[2]]);
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
  for (const c of graph.conveyors) {
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
  return { positions, paths };
}

function boundsOf(layout) {
  const points = [...Object.values(layout.positions), ...Object.values(layout.paths).flat()];
  const xs = points.map(p => p[0]), zs = points.map(p => p[1]);
  return { minX: Math.min(...xs) - 2, maxX: Math.max(...xs) + 2, minZ: Math.min(...zs) - 2.5, maxZ: Math.max(...zs) + 2 };
}

// --- sidebar -----------------------------------------------------------------------

async function loadModels() {
  const { data } = await api("GET", "models");
  state.models = data.models || [];
  const list = document.getElementById("models");
  list.replaceChildren(...state.models.map(m => el("button", {
    class: m.file === state.file ? "current" : null,
    onclick: () => openModel(m.file),
    title: m.description || "",
  },
  m.name || m.file, " ",
  m.valid ? null : el("span", { class: "badge bad" }, "errors"),
  el("span", { class: "sub" }, m.kind === "scenario" ? `scenario of ${m.base}` : "model"))));
}

async function loadRuns() {
  const { data } = await api("GET", "runs");
  state.runs = data.runs || [];
  const list = document.getElementById("runs");
  if (!state.runs.length) {
    list.replaceChildren(el("p", { class: "hint", style: "margin: 0 8px" }, "No runs yet."));
    return;
  }
  list.replaceChildren(...state.runs.slice(0, 15).map(r => el("button", {
    class: state.run && state.run.id === r.id ? "current" : null,
    onclick: () => showRun(r),
  },
  r.label, " ",
  r.status === "done" ? null : el("span", { class: r.status === "failed" ? "badge bad" : "badge" }, r.status),
  el("span", { class: "sub" }, r.id))));
}

// --- opening a model -----------------------------------------------------------------

async function openModel(file) {
  const { ok, data } = await api("GET", `models/${encodeURIComponent(file)}`);
  if (!ok) return;
  let base = data;
  if (data.kind === "scenario") {
    base = (await api("GET", `models/${encodeURIComponent(data.base)}`)).data;
  }
  Object.assign(state, {
    file,
    kind: data.kind,
    base: data.kind === "scenario" ? data.base : file,
    baseTree: base.tree,
    baseLayout: base.layout,
    changes: data.kind === "scenario" ? clone(data.changes || {}) : {},
    name: data.kind === "scenario" ? (data.tree?.name || file.replace(/\.yaml$/, "")) : `${base.tree?.name || "model"}-scenario`,
    description: data.kind === "scenario" ? data.tree?.description ?? null : null,
    errors: (data.errors || []).map(e => ({ change: null, message: `${e.file}${e.line ? ":" + e.line : ""}: ${e.message}` })),
    selected: null,
    tab: "model",
    message: null,
  });
  if (!base.tree) {
    state.message = { kind: "error", text: "This model can't be read.", list: state.errors.map(e => e.message) };
  }
  loadModels();
  render();
  if (state.kind === "scenario") scheduleCheck();
}

const scheduleCheck = debounce(async () => {
  if (!state.base) return;
  const { data } = await api("POST", "check", scenarioBody());
  state.errors = data.errors || [];
  renderEditor();
}, 250);

function scenarioBody() {
  return { name: state.name, description: state.description ?? undefined, base: state.base, changes: state.changes };
}

// --- rendering ------------------------------------------------------------------------

function render() {
  const main = document.getElementById("main");
  if (!state.file && !state.run) {
    main.replaceChildren(el("p", { class: "empty" }, "Pick a model or scenario on the left."));
    return;
  }
  const tree = state.baseTree ? currentTree() : null;
  const title = state.tab === "results" && state.run ? state.run.label : (state.kind === "scenario" ? state.name : tree?.name || state.file);
  const subtitle = state.tab === "results" && state.run
    ? `Run ${state.run.id}`
    : state.kind === "scenario" ? `Scenario of ${state.base}` : tree?.description || "Model";
  main.replaceChildren(
    el("div", { class: "head" },
      el("div", { class: "titles" }, el("h2", {}, title), el("p", {}, subtitle))),
    el("div", { class: "tabs" },
      state.file ? el("button", { class: state.tab === "model" ? "current" : null, onclick: () => { state.tab = "model"; render(); } }, "Model") : null,
      state.run ? el("button", { class: state.tab === "results" ? "current" : null, onclick: () => { state.tab = "results"; render(); } }, "Results") : null),
    el("div", { id: "content" }));
  if (state.tab === "results" && state.run) renderResults();
  else renderEditor();
}

function renderEditor() {
  const content = document.getElementById("content");
  if (!content || state.tab !== "model") return;
  if (!state.baseTree) {
    content.replaceChildren(statusBox());
    return;
  }
  const tree = currentTree();
  const changed = Object.keys(state.changes).length;
  const general = state.errors.filter(e => !e.change);
  content.replaceChildren(
    el("div", { class: "actions" },
      el("label", { for: "scenario-name", style: "color: var(--ink-2)" }, "Scenario name"),
      el("input", { id: "scenario-name", value: state.name, oninput: e => { state.name = e.target.value; } }),
      el("button", { onclick: saveScenario, disabled: changed === 0 && state.kind === "model" }, "Save as scenario"),
      el("button", { class: "primary", id: "run", onclick: startRun }, "▶ Run"),
      el("span", { class: "hint", style: "margin: 0" },
        changed ? `${changed} change${changed === 1 ? "" : "s"} from ${state.base}` : `no changes from ${state.base}`)),
    statusBox(general),
    el("div", { class: "editor", style: "margin-top: 14px" },
      el("div", { class: "card diagram" }, diagram(tree),
        el("p", { class: "hint" }, "Click an object or conveyor to edit it; click empty space for the experiment settings.")),
      el("div", { class: "card panel" }, panel(tree))));
}

function statusBox(general = []) {
  const m = state.message;
  const lines = [...(m?.list || []), ...general.map(e => e.message)];
  if (!m && !lines.length) return el("div");
  return el("div", { class: m?.kind === "ok" ? "status ok" : "status" },
    m?.text || "Problems with this scenario:",
    m?.progress !== undefined ? el("div", { class: "progress" }, el("div", { style: `width: ${m.progress * 100}%` })) : null,
    lines.length ? el("ul", {}, lines.map(l => el("li", {}, l))) : null);
}

// --- diagram ------------------------------------------------------------------------

function diagram(tree) {
  const graph = graphOf(tree);
  const layout = layoutOf(graph, state.baseLayout);
  const b = boundsOf(layout);
  const select = (section, name) => e => { e.stopPropagation(); state.selected = { section, name }; renderEditor(); };
  const isSelected = (section, name) => state.selected?.section === section && state.selected?.name === name;
  const invalid = (section, name) => state.errors.some(e => e.change && e.change.startsWith(`${section}.${name}`));
  const changedObj = (section, name) => Object.keys(state.changes).some(k => k.startsWith(`${section}.${name}.`) || k === `${section}.${name}`);
  const root = svg("svg", {
    viewBox: `${b.minX} ${b.minZ} ${b.maxX - b.minX} ${b.maxZ - b.minZ}`,
    role: "img", "aria-label": `Diagram of ${tree.name}`,
    onclick: () => { state.selected = null; renderEditor(); },
  });
  for (const c of graph.conveyors) {
    const points = layout.paths[c.name].map(p => p.join(",")).join(" ");
    const cls = invalid("conveyors", c.name) ? "invalid" : isSelected("conveyors", c.name) ? "selected" : "";
    root.append(svg("polyline", { points, fill: "none", stroke: "var(--belt)", "stroke-width": 0.5, "stroke-linecap": "round", "stroke-linejoin": "round" }));
    if (cls) root.append(svg("polyline", { points, fill: "none", class: cls, "vector-effect": "non-scaling-stroke", "stroke-linejoin": "round" }));
    const last = layout.paths[c.name].slice(-2);
    const angle = Math.atan2(last[1][1] - last[0][1], last[1][0] - last[0][0]) * 180 / Math.PI;
    root.append(svg("polygon", { points: "0,-0.35 0.5,0 0,0.35", fill: "var(--muted)", transform: `translate(${last[1][0]},${last[1][1]}) rotate(${angle})` }));
    root.append(svg("polyline", { points, fill: "none", stroke: "transparent", "stroke-width": 1.4, class: "hit", onclick: select("conveyors", c.name) },
      svg("title", {}, `conveyor ${c.name}: ${c.from} → ${c.to}`)));
    const mid = layout.paths[c.name][Math.floor(layout.paths[c.name].length / 2) - (layout.paths[c.name].length === 2 ? 1 : 0)];
    const next = layout.paths[c.name][layout.paths[c.name].indexOf(mid) + 1];
    root.append(svg("text", { x: (mid[0] + next[0]) / 2, y: (mid[1] + next[1]) / 2 - 0.45, "text-anchor": "middle", "font-size": 0.6 },
      c.name + (changedObj("conveyors", c.name) ? " •" : "")));
  }
  const objects = [["sources", graph.sources], ["servers", graph.servers], ["sinks", graph.sinks]];
  for (const [section, names] of objects) {
    for (const name of names) {
      const [x, z] = layout.positions[name];
      const s = OBJECT_SIZE;
      const fill = section === "servers" ? "var(--busy)" : "var(--object)";
      const cls = invalid(section, name) ? "invalid" : isSelected(section, name) ? "selected" : "";
      root.append(svg("rect", {
        x: x - s / 2, y: z - s / 2, width: s, height: s, rx: section === "servers" ? 0.15 : 0.6,
        fill, class: `hit ${cls}`, "vector-effect": "non-scaling-stroke", onclick: select(section, name),
      }, svg("title", {}, `${section.slice(0, -1)} ${name}`)));
      root.append(svg("text", { x, y: z - s / 2 - 0.35, "text-anchor": "middle", "font-size": 0.7 },
        name + (changedObj(section, name) ? " •" : "")));
    }
  }
  return root;
}

// --- property panels -------------------------------------------------------------------

/** A labeled input bound to a change path. `placeholder` shows the default
 *  when the model doesn't set the value. */
function field(label, path, kind = "text", options = [], placeholder = null) {
  const value = getAt(currentTree(), path);
  const error = errorFor(path);
  const id = `f-${path.replace(/\W/g, "-")}`;
  let input;
  if (kind === "select") {
    input = el("select", { id, onchange: e => setChange(path, e.target.value) },
      options.map(o => el("option", { value: o, selected: o === value }, o.replaceAll("_", " "))));
  } else {
    input = el("input", {
      id, value: value ?? "", placeholder, inputmode: kind === "int" ? "numeric" : null,
      onchange: e => {
        const raw = e.target.value.trim();
        setChange(path, kind === "int" && /^\d+$/.test(raw) ? Number(raw) : raw);
      },
    });
  }
  return el("div", { class: `field${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("label", { for: id }, label,
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path), title: "Back to the base model's value" }, "reset") : null),
    input,
    error ? el("div", { class: "message" }, error.message) : null);
}

function panel(tree) {
  const sel = state.selected;
  if (!sel) return experimentPanel(tree);
  const node = tree[sel.section]?.[sel.name];
  if (!node) { state.selected = null; return experimentPanel(tree); }
  const p = `${sel.section}.${sel.name}`;
  const kind = { sources: "Source", servers: "Server", conveyors: "Conveyor", sinks: "Sink" }[sel.section];
  const header = [el("h3", {}, sel.name), el("div", { class: "kind" }, kind)];
  const graph = graphOf(tree);
  const outputs = graph.conveyors.filter(c => c.from === sel.name).map(c => c.name);
  const inputs = graph.conveyors.filter(c => c.to === sel.name).map(c => c.name);
  switch (sel.section) {
    case "sources":
      return el("div", {}, header,
        field("Time between arrivals", `${p}.interarrival`),
        field("When the conveyor is full", `${p}.when_blocked`, "select", ["wait", "drop"]),
        outputRule(tree, p, outputs));
    case "servers":
      return el("div", {}, header,
        field("Capacity (parts at once)", `${p}.capacity`, "int", [], "1 (default)"),
        field("Process time", `${p}.process_time`),
        inputRule(tree, p, inputs),
        outputRule(tree, p, outputs),
        failures(tree, p));
    case "conveyors":
      return el("div", {}, header,
        el("p", { class: "hint", style: "margin: 0 0 12px" }, `${node.from} → ${node.to}`),
        field("Length", `${p}.length`), field("Speed", `${p}.speed`), field("Spacing between parts", `${p}.spacing`));
    default:
      return el("div", {}, header, el("p", { class: "hint" }, "Sinks take every part; there's nothing to set."));
  }
}

function experimentPanel() {
  return el("div", {}, el("h3", {}, "Experiment"), el("div", { class: "kind" }, "Applies to the whole model"),
    field("Replications", "experiment.replications", "int"),
    field("Warm-up", "experiment.warmup", "text", [], "none"),
    field("Run length (after warm-up)", "experiment.run_length"),
    field("Seed", "experiment.seed", "int"));
}

function outputRule(tree, p, outputs) {
  if (outputs.length < 2) return null;
  const path = `${p}.output`;
  const value = getAt(tree, path) || {};
  const error = errorFor(path);
  const rule = el("select", {
    id: `f-${path}-rule`,
    onchange: e => {
      const r = e.target.value;
      setChange(path, r === "probability"
        ? { rule: r, weights: Object.fromEntries(outputs.map(o => [o, +(1 / outputs.length).toFixed(4)])) }
        : { rule: r });
    },
  }, el("option", { value: "", selected: !value.rule, disabled: true }, "choose a rule"),
  OUTPUT_RULES.map(r => el("option", { value: r, selected: value.rule === r }, r.replaceAll("_", " "))));
  const weights = value.rule === "probability" ? outputs.map(o => el("div", { class: "row field", style: "margin-bottom: 6px; align-items: center" },
    el("span", {}, o),
    el("input", {
      value: value.weights?.[o] ?? "", inputmode: "decimal", "aria-label": `weight of ${o}`,
      onchange: e => {
        const updated = clone(value);
        updated.weights = { ...updated.weights, [o]: Number(e.target.value) };
        setChange(path, updated);
      },
    }))) : [];
  return el("div", { class: `group field${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("label", { for: `f-${path}-rule` }, "Sends parts to " + outputs.join(", "),
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path) }, "reset") : null),
    rule, ...weights,
    error ? el("div", { class: "message" }, error.message) : null);
}

function inputRule(tree, p, inputs) {
  if (inputs.length < 2) return null;
  const path = `${p}.input`;
  const value = getAt(tree, path) || {};
  const error = errorFor(path);
  const rule = el("select", {
    id: `f-${path}-rule`,
    onchange: e => setChange(path, e.target.value === "priority" ? { rule: "priority", order: inputs } : { rule: e.target.value }),
  }, el("option", { value: "", selected: !value.rule, disabled: true }, "choose a rule"),
  INPUT_RULES.map(r => el("option", { value: r, selected: value.rule === r }, r.replaceAll("_", " "))));
  const order = value.rule === "priority" ? (value.order || inputs).map((c, i, list) => el("div", { class: "order-item" },
    el("span", {}, `${i + 1}. ${c}`),
    el("button", {
      disabled: i === 0, "aria-label": `raise ${c}`,
      onclick: () => {
        const o = [...list]; [o[i - 1], o[i]] = [o[i], o[i - 1]];
        setChange(path, { rule: "priority", order: o });
      },
    }, "↑"))) : [];
  return el("div", { class: `group field${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("label", { for: `f-${path}-rule` }, "Takes parts from " + inputs.join(", "),
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path) }, "reset") : null),
    rule, ...order,
    error ? el("div", { class: "message" }, error.message) : null);
}

function failures(tree, p) {
  const path = `${p}.failures`;
  const list = getAt(tree, path) || [];
  const add = () => setChange(path, [...clone(list), {
    name: `failure_${list.length + 1}`, uptime: "exponential(1 h)", downtime: "uniform(5 min, 15 min)",
  }]);
  const remove = i => setChange(path, list.filter((_, j) => j !== i));
  return el("div", { class: "group" },
    el("div", { class: "group-title" }, "Failures"),
    list.length ? null : el("p", { class: "hint", style: "margin: 0 0 8px" }, "This server never fails."),
    list.map((f, i) => el("div", { style: "margin-bottom: 10px" },
      el("div", { class: "row", style: "align-items: center; margin-bottom: 6px" },
        el("strong", {}, f.name), el("button", { onclick: () => remove(i), style: "flex: 0" }, "Remove")),
      field("Uptime between failures", `${path}.${i}.uptime`),
      field("Downtime (repair)", `${path}.${i}.downtime`),
      el("div", { class: "row" },
        field("Uptime counts", `${path}.${i}.counts`, "select", ["busy_time", "clock_time"]),
        field("Interrupted part", `${path}.${i}.on_failure`, "select", ["resume", "restart"])))),
    el("button", { onclick: add }, "Add a failure"));
}

// --- saving and running --------------------------------------------------------------------

async function saveScenario() {
  const name = state.name.trim();
  if (!/^[A-Za-z_][A-Za-z0-9_-]*$/.test(name)) {
    state.message = { kind: "error", text: "Scenario names use letters, digits, _ and -, starting with a letter or _." };
    return renderEditor();
  }
  const file = `${name}.yaml`;
  const { ok, data } = await api("PUT", `models/${encodeURIComponent(file)}`, { ...scenarioBody(), name });
  if (ok) {
    state.message = { kind: "ok", text: `Saved as ${file}.` };
    await loadModels();
    await openModel(file);
    state.message = { kind: "ok", text: `Saved as ${file}.` };
    renderEditor();
  } else {
    state.errors = data.errors || [];
    state.message = { kind: "error", text: data.error || "The scenario has errors; nothing was saved." };
    renderEditor();
  }
}

async function startRun() {
  const unchanged = state.kind === "model" ? Object.keys(state.changes).length === 0 : null;
  const body = unchanged ? { file: state.file } : { scenario: scenarioBody() };
  if (state.kind === "scenario") {
    // Saved scenario, unchanged since it was opened: run the file.
    const saved = state.models.find(m => m.file === state.file);
    if (saved) {
      const detail = (await api("GET", `models/${encodeURIComponent(state.file)}`)).data;
      if (equal(detail.changes || {}, state.changes)) { delete body.scenario; body.file = state.file; }
    }
  }
  const { ok, data } = await api("POST", "runs", body);
  if (!ok) {
    state.errors = data.errors || state.errors;
    state.message = { kind: "error", text: data.error || "The run couldn't start." };
    return renderEditor();
  }
  state.message = { kind: "ok", text: `Running ${data.label}…`, progress: 0 };
  renderEditor();
  loadRuns();
  let run = data;
  while (run.status === "queued" || run.status === "running") {
    await new Promise(r => setTimeout(r, 400));
    run = (await api("GET", `runs/${run.id}`)).data;
    state.message = {
      kind: "ok",
      text: run.status === "queued" ? `Waiting for another run to finish…` : `Running ${run.label}: ${run.progress.done} of ${run.progress.total} replications`,
      progress: run.progress.done / Math.max(1, run.progress.total),
    };
    renderEditor();
  }
  state.message = run.status === "failed" ? { kind: "error", text: "The run failed.", list: [run.error] } : null;
  loadRuns();
  if (run.status === "done") showRun(run);
  else renderEditor();
}

// --- results ----------------------------------------------------------------------------------

function showRun(run) {
  stopAnimation();
  state.run = run;
  state.tab = "results";
  if (state.view === "animation" && !run.files.includes("events.jsonl")) state.view = "report";
  render();
  loadRuns();
}

function renderResults() {
  const run = state.run;
  const content = document.getElementById("content");
  const file = name => `${API}/runs/${run.id}/files/${name}`;
  if (run.status !== "done") {
    content.replaceChildren(el("p", { class: "note" }, run.status === "failed" ? `This run failed: ${run.error}` : `This run is ${run.status}.`));
    return;
  }
  const view = (name, label) => el("button", {
    class: state.view === name ? "primary" : null,
    onclick: () => { stopAnimation(); state.view = name; renderResults(); },
  }, label);
  const downloads = ["summary.json", "replications.csv", "events.jsonl", "layout.json", "scenario.yaml"]
    .filter(f => run.files.includes(f))
    .map(f => el("a", { href: file(f), download: f }, f));
  const body = el("div");
  content.replaceChildren(
    el("div", { class: "results-bar" },
      view("report", "Report"),
      view("animation", "Animation"),
      el("button", {
        class: state.view === "3d" ? "primary" : null, title: "3D animation is planned",
        onclick: () => { stopAnimation(); state.view = "3d"; renderResults(); },
      }, "3D"),
      el("span", { class: "spacer" }),
      ...downloads),
    body);
  if (state.view === "report") {
    body.append(el("iframe", { class: "report", src: file("report.html"), title: "Report" }));
  } else if (state.view === "animation") {
    if (run.files.includes("events.jsonl")) startAnimation(body, run);
    else body.append(el("div", { class: "card note" },
      "This model doesn't write an event log, so there's nothing to animate. ",
      "Set ", el("code", {}, "outputs.event_log: jsonl"), " in the model and run it again."));
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
    for (const c of this.header.conveyors) this.conveyors[c.name] = { ...c, belt: [] };
    this.servers = {};
    for (const s of this.header.servers) this.servers[s.name] = { busy: [], blocked: [], failed: "" };
    this.created = this.lost = this.exited = 0;
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
      case "created": this.created++; break;
      case "lost": this.lost++; break;
      case "entered_conveyor": {
        const c = this.conveyors[e.conveyor];
        const from = this.servers[c.from];
        if (from) from.blocked = from.blocked.filter(p => p !== e.part);
        c.belt.push({ part: e.part, pos: 0, since: e.t });
        break;
      }
      case "started_process":
        this.leave(e.part, e.server, e.t);
        this.servers[e.server].busy.push(e.part);
        break;
      case "finished_process": {
        const s = this.servers[e.server];
        s.busy = s.busy.filter(p => p !== e.part);
        s.blocked.push(e.part);
        break;
      }
      case "failed": this.servers[e.server].failed = e.failure; break;
      case "repaired": this.servers[e.server].failed = ""; break;
      case "exited": this.leave(e.part, e.sink, e.t); this.exited++; break;
    }
  }

  leave(part, destination, t) {
    for (const c of Object.values(this.conveyors)) {
      if (c.to === destination && c.belt.length && c.belt[0].part === part) {
        const positions = this.positions(c, t);
        c.belt.forEach((item, i) => { item.pos = positions[i]; item.since = t; });
        c.belt.shift();
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

  state(server) {
    const s = this.servers[server];
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
    conveyors: header.conveyors.map(c => ({ name: c.name, from: c.from, to: c.to })),
  };
  const layout = layoutOf(graph, layoutFile);
  const paths = {};
  for (const c of header.conveyors) {
    const points = layout.paths[c.name];
    const cumulative = [0];
    for (let i = 1; i < points.length; i++) {
      cumulative.push(cumulative[i - 1] + Math.hypot(points[i][0] - points[i - 1][0], points[i][1] - points[i - 1][1]));
    }
    paths[c.name] = { points, cumulative, scale: cumulative[cumulative.length - 1] / c.length };
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
      el("div", { class: "legend" }, STATES.map(([s, label]) => el("span", {}, el("i", { style: `background: var(--${s})` }), label)))),
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
    const colors = Object.fromEntries(["belt", "object", "part", "ink-2", "busy", "blocked", "failed", "idle", "surface"].map(n => [n, cssVar(`--${n}`)]));

    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.strokeStyle = colors.belt;
    ctx.lineWidth = Math.max(3, 0.6 * scale);
    for (const path of Object.values(paths)) {
      ctx.beginPath();
      path.points.forEach(([x, z], i) => (i ? ctx.lineTo(X(x), Y(z)) : ctx.moveTo(X(x), Y(z))));
      ctx.stroke();
    }
    const s = OBJECT_SIZE * scale;
    ctx.font = "12px system-ui, sans-serif";
    ctx.textAlign = "center";
    for (const [name, [x, z]] of Object.entries(layout.positions)) {
      const server = replay.servers[name];
      ctx.fillStyle = server ? colors[replay.state(name)] : colors.object;
      ctx.beginPath();
      ctx.roundRect(X(x) - s / 2, Y(z) - s / 2, s, s, server ? 4 : s / 2);
      ctx.fill();
      ctx.fillStyle = colors["ink-2"];
      ctx.fillText(server?.failed ? `${name} (${server.failed})` : name, X(x), Y(z) - s / 2 - 6);
    }
    const p = Math.max(4, 0.32 * scale);
    ctx.fillStyle = colors.part;
    ctx.strokeStyle = colors.surface;
    ctx.lineWidth = 1;
    for (const [name, path] of Object.entries(paths)) {
      for (const meters of replay.belt(name)) {
        const [x, z] = pointOn(path, meters);
        ctx.fillRect(X(x) - p / 2, Y(z) - p / 2, p, p);
        ctx.strokeRect(X(x) - p / 2, Y(z) - p / 2, p, p);
      }
    }
    for (const [name, sv] of Object.entries(replay.servers)) {
      const [x, z] = layout.positions[name];
      [...sv.busy, ...sv.blocked].forEach((_, i) => {
        ctx.fillRect(X(x) - p / 2 + i * (p + 2), Y(z) - p / 2, p, p);
        ctx.strokeRect(X(x) - p / 2 + i * (p + 2), Y(z) - p / 2, p, p);
      });
    }

    clockText.textContent = clock(replay.time);
    phase.textContent = replay.time < replay.warmup ? `warm-up (statistics start at ${clock(replay.warmup)})` : `measuring · replication ${header.replication}`;
    counts.textContent = `${replay.created} created · ${replay.created - replay.lost - replay.exited} in system · ${replay.exited} exited` + (replay.lost ? ` · ${replay.lost} lost` : "");
    play.textContent = a.playing ? "Pause" : "Play";
    if (document.activeElement !== slider) slider.value = replay.time;
    a.frame = requestAnimationFrame(draw);
  };
  a.frame = requestAnimationFrame(draw);
}

// --- start ------------------------------------------------------------------------------------

loadModels();
loadRuns();
setInterval(() => { if (state.runs.some(r => r.status === "queued" || r.status === "running")) loadRuns(); }, 2000);
