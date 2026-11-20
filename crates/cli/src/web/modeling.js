"use strict";
// The Modeling page (/modeling): edit a project's model, choose the decisions and
// results the Simulation page offers, save versions, publish them, run and
// inspect results. Shared code: common.js.

const state = {
  projects: [],
  runs: [],
  project: null,     // id of the open project
  detail: null,      // GET /projects/{id} (working copy)
  baseTree: null,    // the saved working model, as a tree
  baseLayout: null,
  changes: {},       // unsaved changes: path -> value
  errors: [],        // from POST /check
  selected: null,    // { section, name } of an object or connection, or { definition }
  tab: "model",      // "model" | "decisions" | "documentation" | "results"
  draft: null,       // project.yaml's { decisions, results } with unsaved edits
  exposed: null,     // the draft as checked: { decisions, results, decision_problems }
  offer: null,       // the "offer as an option" form: { decision, newLabel, optionLabel, currentLabel }
  written: new Set(), // paths whose defaults were written into the model for a decision
  layout: null,      // { positions: name -> [x, z], paths: conveyor -> [[x, z], ...] } being edited
  layoutChanged: false,
  connect: null,     // null, or { from: null | name } while connecting objects
  undo: [], redo: [],
  batching: false,
  run: null,
  view: "report",
  message: null,     // { kind: "ok" | "error", text, list?, progress? }
  saveMessage: "",
  docs: { text: "", editing: false, html: "" },  // README.md: the draft, whether it's being edited, the draft rendered
};

// --- change paths ---------------------------------------------------------------

/** The model with the current changes applied. */
function currentTree() {
  const tree = clone(state.baseTree);
  for (const [path, value] of Object.entries(state.changes)) {
    if (value === null) deleteAt(tree, path);
    else setAt(tree, path, clone(value));
  }
  return tree;
}

function deleteAt(tree, path) {
  const segments = path.split(".");
  const parent = getAt(tree, segments.slice(0, -1).join(".")) ?? (segments.length === 1 ? tree : undefined);
  if (parent && typeof parent === "object") delete parent[segments[segments.length - 1]];
}


/** Records a change, or removes it when the value is back to the base's. */
function setChange(path, value) {
  remember();
  // A change to a parent path (e.g. a whole failures list) already covers this one.
  const parent = Object.keys(state.changes).find(k => path.startsWith(k + ".") && state.changes[k] !== null);
  if (parent) {
    const holder = { root: clone(state.changes[parent]) };
    if (value === null) deleteAt(holder, "root" + path.slice(parent.length));
    else setAt(holder, "root" + path.slice(parent.length), value);
    state.changes[parent] = holder.root;
    if (equal(holder.root, getAt(state.baseTree, parent))) delete state.changes[parent];
  } else {
    for (const k of Object.keys(state.changes)) if (k.startsWith(path + ".")) delete state.changes[k];
    const base = getAt(state.baseTree, path);
    if (equal(value, base) || (value === null && base === undefined)) delete state.changes[path];
    else state.changes[path] = clone(value);
  }
  if (state.batching) return;
  scheduleCheck();
  renderEditor();
}

// --- undo and redo -------------------------------------------------------------------------

function snapshot() {
  return clone({ changes: state.changes, layout: state.layout, layoutChanged: state.layoutChanged, draft: state.draft, written: [...state.written], selected: state.selected });
}

function restore(snap) {
  Object.assign(state, clone(snap));
  state.written = new Set(snap.written);
  scheduleCheck();
  renderEditor();
}

/** Keeps the state before an edit, for undo. One entry per user action. */
function remember() {
  if (state.batching) return;
  state.undo.push(snapshot());
  if (state.undo.length > 200) state.undo.shift();
  state.redo = [];
}

/** Runs several edits as one undo step, then checks and redraws once. */
function batch(f) {
  remember();
  state.batching = true;
  try { f(); } finally { state.batching = false; }
  scheduleCheck();
  renderEditor();
}

function undo() {
  if (!state.undo.length) return;
  state.redo.push(snapshot());
  restore(state.undo.pop());
}

function redo() {
  if (!state.redo.length) return;
  state.undo.push(snapshot());
  restore(state.redo.pop());
}

document.addEventListener("keydown", e => {
  if (!(e.ctrlKey || e.metaKey) || !state.project || state.tab === "results") return;
  if (["INPUT", "SELECT", "TEXTAREA"].includes(document.activeElement?.tagName)) return;
  const key = e.key.toLowerCase();
  if (key === "z" && !e.shiftKey) { e.preventDefault(); undo(); }
  else if (key === "y" || (key === "z" && e.shiftKey)) { e.preventDefault(); redo(); }
});
document.addEventListener("keydown", e => {
  if (e.key === "Escape" && state.connect) { state.connect = null; renderEditor(); }
});

function resetChange(path) {
  remember();
  for (const k of Object.keys(state.changes)) if (related(k, path)) delete state.changes[k];
  scheduleCheck();
  renderEditor();
}

const isChanged = path => Object.keys(state.changes).some(k => related(k, path));
const errorFor = path => state.errors.find(e => e.change && related(e.change, path));

// --- sidebar -------------------------------------------------------------------------

async function loadRuns() {
  const { data } = await api("GET", "runs");
  state.runs = (data.runs || []).filter(r => r.project === state.project);
  const list = document.getElementById("runs");
  if (!state.runs.length) {
    list.replaceChildren(el("p", { class: "hint", style: "margin: 0 8px" }, "No runs of this project yet."));
    return;
  }
  list.replaceChildren(...state.runs.slice(0, 15).map(r => el("div", { class: "run-item" },
    el("button", {
      class: state.run && state.run.id === r.id ? "current" : null,
      onclick: () => showRun(r),
    },
    r.label, " ",
    r.status === "done" ? null : el("span", { class: r.status === "failed" ? "badge bad" : "badge" }, r.status),
    el("span", { class: "sub" }, runDate(r))),
    r.status === "done" || r.status === "failed"
      ? el("button", { class: "trash", title: "Delete this run", "aria-label": `Delete the run ${r.label} of ${runDate(r)}`, onclick: () => deleteRun(r) }, trash())
      : null)));
}

/** When a run was asked for, in the browser's local time. */
const runDate = r => new Date(r.queued_at * 1000).toLocaleString();

/** Deletes a finished run's results, animation included; it can be run again from the model. */
async function deleteRun(r) {
  if (!confirm(`Delete the run “${r.label}” of ${runDate(r)}? Its results and animation go with it.`)) return;
  const { ok, data } = await api("DELETE", `runs/${encodeURIComponent(r.id)}`);
  if (!ok) {
    state.message = { kind: "error", text: data.error || `The run of ${runDate(r)} couldn't be deleted.` };
  } else if (state.run && state.run.id === r.id) {
    stopAnimation();
    state.run = null;
    if (state.tab === "results") state.tab = "model";
  }
  render();
  loadRuns();
}

function trash() {
  return svg("svg", { class: "icon", viewBox: "0 0 24 24", "aria-hidden": "true" },
    svg("path", { d: "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v6M14 11v6" }));
}

// --- opening a project -------------------------------------------------------------------

async function openProject(id, keepMessage = false) {
  const { ok, data } = await api("GET", `projects/${encodeURIComponent(id)}`);
  if (!ok) return projectMissing(document.getElementById("main"), data.error || `The project ${id} can't be opened.`);
  showProjectName(data.name || id, PAGES.modeling);
  Object.assign(state, {
    project: id,
    detail: data,
    baseTree: data.tree,
    baseLayout: data.layout,
    changes: {},
    errors: (data.errors || []).map(e => ({ change: null, message: `${e.file}${e.line ? ":" + e.line : ""}: ${e.message}` })),
    // Opening the page shows the experiment; saving or publishing keeps what was open.
    selected: keepMessage ? state.selected : { ...EXPERIMENT },
    tab: keepMessage && ["decisions", "documentation"].includes(state.tab) ? state.tab : "model",
    message: keepMessage ? state.message : null,
    saveMessage: "",
    draft: savedDraft(data),
    written: new Set(),
    layout: data.tree ? layoutOf(graphOf(data.tree), data.layout) : null,
    layoutChanged: false,
    connect: null,
    undo: [], redo: [],
    exposed: { decisions: data.decisions, results: data.results, decision_problems: data.decision_problems },
    offer: null,
    docs: { text: data.documentation || "", editing: false, html: data.documentation_html || "" },
  });
  if (!data.tree) {
    state.message = { kind: "error", text: "This model can't be read.", list: state.errors.map(e => e.message) };
  }
  render();
}

/** project.yaml's decisions and results as saved. */
function savedDraft(detail) {
  const file = detail.project_file || {};
  return { decisions: clone(file.decisions || {}), results: clone(file.results || []) };
}

const draftChanged = () => state.draft && !equal(state.draft, savedDraft(state.detail));
const docsChanged = () => state.docs.text !== (state.detail?.documentation || "");
/** How many unsaved edits: model changes, plus decisions, layout and documentation as one each. */
const unsaved = () => Object.keys(state.changes).length + (draftChanged() ? 1 : 0) + (state.layoutChanged ? 1 : 0) + (docsChanged() ? 1 : 0);

// Leaving with unsaved edits (closing, reloading, ← Projects, Simulation) asks first.
// Browsers show their own question; pages can't change its words.
window.addEventListener("beforeunload", e => {
  if (!state.detail || !unsaved()) return;
  e.preventDefault();
  e.returnValue = "";  // older Safari and Chrome
});

/** What to send to check and save: model changes, plus the draft if edited. */
function pending() {
  const body = { changes: state.changes };
  if (draftChanged()) Object.assign(body, state.draft);
  if (state.layoutChanged) body.layout = layoutTree();
  if (docsChanged()) body.documentation = state.docs.text;
  return body;
}

/** The layout being edited, as a layout file (in meters, Y up). */
function layoutTree() {
  const tree = currentTree();
  const graph = graphOf(tree);
  fillLayout(graph);
  const r = v => Math.round(v * 100) / 100;
  const objects = {};
  for (const name of [...graph.sources, ...graph.servers, ...graph.sinks]) {
    const [x, z] = state.layout.positions[name];
    const model = state.layout.drawn?.[name];
    objects[name] = model ? { position: [r(x), 0, r(z)], model } : { position: [r(x), 0, r(z)] };
  }
  const conveyors = {};
  // Queues aren't in the layout file: they're drawn between their objects.
  for (const c of graph.conveyors.filter(c => !c.queue)) conveyors[c.name] = { path: state.layout.paths[c.name].map(([x, z]) => [r(x), 0, r(z)]) };
  const vehicles = {}, entities = {};
  for (const f of Object.keys(tree.vehicles || {})) if (state.layout.fleets?.[f]) vehicles[f] = { model: state.layout.fleets[f] };
  for (const t of typesOf(tree)) if (state.layout.types?.[t]) entities[t] = { model: state.layout.types[t] };
  return {
    units: "m", objects, conveyors,
    ...(Object.keys(vehicles).length ? { vehicles } : {}),
    ...(Object.keys(entities).length ? { entities } : {}),
  };
}

const scheduleCheck = debounce(async () => {
  if (!state.project) return;
  const { data } = await api("POST", `projects/${encodeURIComponent(state.project)}/check`, pending());
  state.errors = data.errors || [];
  state.exposed = { decisions: data.decisions || [], results: data.results || [], decision_problems: data.decision_problems || [] };
  if (data.documentation_html !== undefined) state.docs.html = data.documentation_html;
  renderEditor();
}, 250);

// --- rendering ------------------------------------------------------------------------

function render() {
  const main = document.getElementById("main");
  if (!state.detail) return;
  const d = state.detail;
  const results = state.tab === "results" && state.run;
  main.replaceChildren(
    el("div", { class: "head" },
      el("div", { class: "titles" },
        el("h2", {}, results ? state.run.label : d?.name || state.project),
        el("p", {}, results ? `Run of ${runDate(state.run)}` : d?.description || "")),
    ),
    el("div", { class: "tabs" },
      el("button", { class: state.tab === "model" ? "current" : null, onclick: () => { state.tab = "model"; render(); } }, "Model"),
      el("button", { id: "tab-decisions", class: state.tab === "decisions" ? "current" : null, onclick: () => { state.tab = "decisions"; render(); } },
        "Decisions", (state.exposed?.decision_problems || []).length ? el("span", { class: "badge bad", style: "margin-left: 6px" }, "!") : null),
      el("button", { id: "tab-documentation", class: state.tab === "documentation" ? "current" : null, onclick: () => { state.tab = "documentation"; render(); } }, "Documentation"),
      state.run ? el("button", { class: state.tab === "results" ? "current" : null, onclick: () => { state.tab = "results"; render(); } }, "Results") : null),
    el("div", { id: "content" }));
  if (results) {
    runResults(document.getElementById("content"), state.run, state.view, v => { state.view = v; render(); });
  } else {
    renderEditor();
  }
}

function renderEditor() {
  renderSide();
  const content = document.getElementById("content");
  if (!content || !["model", "decisions", "documentation"].includes(state.tab)) return;
  if (!state.baseTree) {
    content.replaceChildren(statusBox());
    return;
  }
  if (state.tab === "decisions") return renderDecisions(content);
  if (state.tab === "documentation") return renderDocumentation(content);
  const tree = currentTree();
  const general = state.errors.filter(e => !e.change);
  content.replaceChildren(...[
    actionsBar(),
    state.offer ? offerForm() : null,
    statusBox(general),
    el("div", { class: "editor", style: "margin-top: 14px" },
      el("div", { class: "stack" },
        el("div", { class: "card diagram" }, diagramToolbar(), diagram(tree),
          el("p", { class: "hint" }, state.connect
            ? (state.connect.from ? say(`Now click where parts go from ${state.connect.from}.`) : say("Click the object parts come from."))
            : "Click an object, conveyor or queue to edit it, and drag objects to move them. Drag from the dot on an object's edge to connect it. What applies to the whole model is under Definitions, in the bar on the left.")),
        versionsCard()),
      el("div", { class: "card panel" }, panel(tree))),
  ].filter(Boolean));
}

/** Save, publish, run: the same on the Model, Decisions and Documentation tabs. */
function actionsBar() {
  const modelChanges = Object.keys(state.changes).length;
  const changed = unsaved();
  const history = state.detail.history || [];
  const lastSaved = history[0];
  const problems = state.exposed?.decision_problems || [];
  const canPublish = changed === 0 && lastSaved && !lastSaved.published && state.detail.valid && !problems.length;
  const describe = () => {
    if (!changed) return "all changes saved";
    const parts = [];
    if (modelChanges) parts.push(`${modelChanges} unsaved change${modelChanges === 1 ? "" : "s"}`);
    if (draftChanged()) parts.push("unsaved decision edits");
    if (state.layoutChanged && !modelChanges) parts.push("unsaved layout");
    if (docsChanged()) parts.push("unsaved documentation");
    return parts.join(", ");
  };
  return el("div", { class: "actions" },
    el("input", {
      id: "save-message", placeholder: "What changed? (e.g. Faster mill B)", value: state.saveMessage,
      oninput: e => { state.saveMessage = e.target.value; }, "aria-label": "Version message",
    }),
    el("button", { id: "save", class: changed ? "unsaved" : null, onclick: saveVersion, disabled: changed === 0 }, "Save version"),
    el("button", { id: "publish", onclick: publishVersion, disabled: !canPublish,
      title: canPublish ? "Make the last saved version available on the Simulation page"
        : problems.length ? "Fix the problems on the Decisions tab first"
        : "Save your changes first; already-published versions can't be published again" }, "Publish"),
    el("button", { class: "primary", id: "run", onclick: startRun }, "▶ Run"),
    state.tab === "model" && Object.keys(state.changes).some(p => !state.written.has(p)) && !state.offer
      ? el("button", { id: "offer", onclick: startOffer, title: "Instead of saving these changes into the model, offer them as an option on the Simulation page" }, "Offer as an option…")
      : null,
    el("span", { class: "hint", id: "pending", style: "margin: 0" }, describe()));
}

function versionsCard() {
  const history = state.detail.history || [];
  return el("div", { class: "card versions" },
    el("h3", { style: "margin: 0 0 8px; font-size: 15px" }, "Versions"),
    el("p", { class: "hint", style: "margin: 0 0 8px" },
      "Saving keeps a version of the model; publishing makes the last saved version available on the Simulation page."),
    history.slice(0, 12).map(c => el("div", { class: "item" },
      c.message, " ", c.published ? el("span", { class: "pill" }, `published ${c.published}`) : null,
      el("span", { class: "sub" }, `${new Date(c.date).toLocaleString()} · ${c.short}`))));
}

function statusBox(general = []) {
  const m = state.message;
  const lines = [...(m?.list || []), ...general.map(e => e.message)];
  if (!m && !lines.length) return el("div");
  return el("div", { class: m?.kind === "ok" ? "status ok" : "status" },
    m?.text || "Problems with these changes:",
    m?.progress !== undefined ? el("div", { class: "progress" }, el("div", { style: `width: ${m.progress * 100}%` })) : null,
    lines.length ? el("ul", {}, lines.map(l => el("li", {}, l))) : null);
}

// --- diagram ------------------------------------------------------------------------

/** Positions for objects and paths for conveyors that the layout doesn't
 *  have yet (new ones), and none for ones that are gone. */
function fillLayout(graph) {
  const L = state.layout;
  const names = [...graph.sources, ...graph.servers, ...graph.sinks];
  for (const name of names) {
    if (L.positions[name]) continue;
    const xs = Object.values(L.positions).map(p => p[0]);
    L.positions[name] = [xs.length ? Math.max(...xs) + 6 : 0, 0];
  }
  for (const c of graph.conveyors) if (!c.queue && !L.paths[c.name]) L.paths[c.name] = route(c.from, c.to);
  for (const name of Object.keys(L.positions)) if (!names.includes(name)) delete L.positions[name];
  L.drawn ||= {};
  for (const name of Object.keys(L.drawn)) if (!names.includes(name)) delete L.drawn[name];
  for (const name of Object.keys(L.paths)) if (!graph.conveyors.some(c => c.name === name)) delete L.paths[name];
  placeQueues(graph, L.positions, L.paths);
}

/** A conveyor's path between two objects: straight, a Z-bend, or around the
 *  top when it goes backwards. */
function route(from, to) {
  const [a, b] = [state.layout.positions[from], state.layout.positions[to]];
  const gap = OBJECT_SIZE / 2 + 0.1;
  if (Math.abs(a[1] - b[1]) < 0.01 && b[0] > a[0]) return [[a[0] + gap, a[1]], [b[0] - gap, b[1]]];
  if (b[0] - a[0] > 2 * gap + 1) {
    const mid = Math.round((a[0] + b[0]) / 2 * 2) / 2;
    return [[a[0] + gap, a[1]], [mid, a[1]], [mid, b[1]], [b[0] - gap, b[1]]];
  }
  const lane = Math.min(a[1], b[1]) - 2.5;
  return [[a[0], a[1] - gap], [a[0], lane], [b[0], lane], [b[0], b[1] - gap]];
}

const pathLength = path => path.slice(1).reduce((sum, p, i) => sum + Math.hypot(p[0] - path[i][0], p[1] - path[i][1]), 0);
const meters = text => { const m = typeof text === "string" && text.trim().match(/^(\d+(?:\.\d+)?)\s*m$/); return m ? Number(m[1]) : null; };
const lengthText = len => `${Math.max(0.5, Math.round(len * 10) / 10)} m`;
const sectionOf = (tree, name) => ["sources", "servers", "sinks"].find(s => tree[s] && name in tree[s]);
const isName = name => /^[A-Za-z][A-Za-z0-9_-]*$/.test(name);
/** What flows through the model, in its words (`parts: { one, many }`), with capitals kept. */
function say(text) {
  // From the saved model and pending changes, without rebuilding the whole tree.
  const saved = state.baseTree?.parts || {};
  const changed = state.changes.parts === null ? {} : state.changes.parts || {};
  const words = { ...saved, ...changed,
    ...("parts.one" in state.changes ? { one: state.changes["parts.one"] } : {}),
    ...("parts.many" in state.changes ? { many: state.changes["parts.many"] } : {}) };
  const one = words.one || "part", many = words.many || "parts";
  const up = w => w.charAt(0).toUpperCase() + w.slice(1);
  return text.replace(/\bparts\b/g, many).replace(/\bParts\b/g, up(many))
    .replace(/\bpart\b/g, one).replace(/\bPart\b/g, up(one));
}

const allNames = tree => ["sources", "servers", "sinks", "conveyors", "queues", "transports", "vehicles", "resources"].flatMap(s => Object.keys(tree[s] || {}));
/** Conveyors, queues and transports: the kinds of connection. */
const CONNECTIONS = ["conveyors", "queues", "transports"];
const isConnection = section => CONNECTIONS.includes(section);
const connectionSection = (tree, name) => CONNECTIONS.find(s => tree[s] && name in tree[s]);

function freeName(tree, base) {
  const taken = allNames(tree);
  if (!taken.includes(base)) return base;
  for (let i = 2; ; i++) if (!taken.includes(`${base}_${i}`)) return `${base}_${i}`;
}

/** Moves an object and redraws its conveyors. A conveyor whose length
 *  matched its drawing keeps matching it ("the length follows the drawing");
 *  one with a typed length keeps it. */
function moveObject(name, x, z) {
  const tree = currentTree();
  const graph = graphOf(tree);
  // Queues have no length: they're just redrawn.
  const attached = graph.conveyors.filter(c => !c.queue && (c.from === name || c.to === name));
  const follows = Object.fromEntries(attached.map(c => {
    const length = meters(tree.conveyors[c.name].length);
    return [c.name, length !== null && Math.abs(length - pathLength(state.layout.paths[c.name])) < 0.06];
  }));
  state.layout.positions[name] = [x, z];
  for (const c of attached) {
    state.layout.paths[c.name] = route(c.from, c.to);
    if (follows[c.name]) setChange(`conveyors.${c.name}.length`, lengthText(pathLength(state.layout.paths[c.name])));
  }
  placeQueues(graph, state.layout.positions, state.layout.paths);
  state.layoutChanged = true;
}

/** Adds a source, server or sink: where it was dropped on the diagram (`at`, in meters),
 *  or, from a click, beyond the objects already there. */
function addObject(section, at = null) {
  const tree = currentTree();
  const base = { sources: "arrivals", servers: "machine", sinks: "exit" }[section];
  const name = freeName(tree, base);
  const entities = Object.keys(tree.entities || {});
  const defaults = {
    sources: { creates: entities[0] || "part", interarrival: "exponential(1 min)" },
    servers: { capacity: 1, process_time: "triangular(40 s, 50 s, 70 s)" },
    sinks: {},
  }[section];
  batch(() => {
    if (section === "sources" && !entities.length) setChange("entities.part", {});
    setChange(`${section}.${name}`, defaults);
    const xs = Object.values(state.layout.positions).map(p => p[0]);
    const zs = Object.values(state.layout.positions).map(p => p[1]);
    state.layout.positions[name] = at || [xs.length ? Math.max(...xs) + 6 : 0, zs.length ? Math.max(...zs) + 4 : 0];
    state.layoutChanged = true;
    state.selected = { section, name };
    const connect = section === "sinks" ? "drag to it from the dot on another object's edge" : "drag from the dot on its edge to the next object";
    state.message = { kind: "ok", text: at
      ? `Added ${name}. To connect it, ${connect}.`
      : `Added ${name}. Drag it into place; to connect it, ${connect}.` };
  });
}

function connectObjects(from, to, kind = "conveyors") {
  const tree = currentTree();
  const fromSection = sectionOf(tree, from), toSection = sectionOf(tree, to);
  if (from === to || fromSection === "sinks" || toSection === "sources") {
    state.message = { kind: "error", text: "Conveyors and queues go from a source or server to a server or sink." };
    return renderEditor();
  }
  if (kind === "transports") {
    const fleets = Object.keys(tree.vehicles || {});
    if (!fleets.length) {
      state.message = { kind: "error", text: "Add vehicles first (Definitions → Vehicles, in the bar on the left): they carry parts along transports." };
      return renderEditor();
    }
    const name = freeName(tree, `t_${from}_${to}`.slice(0, 40));
    const [a, b] = [state.layout.positions[from], state.layout.positions[to]];
    const meters = Math.max(1, Math.round(Math.hypot(b[0] - a[0], b[1] - a[1])));
    return batch(() => {
      setChange(`transports.${name}`, { from, to, by: fleets[0], distance: `${meters} m` });
      state.layoutChanged = true;
      state.selected = { section: "transports", name };
      state.message = null;
    });
  }
  if (kind === "queues") {
    const name = freeName(tree, `q_${from}_${to}`.slice(0, 40));
    return batch(() => {
      setChange(`queues.${name}`, { from, to });
      state.layoutChanged = true;
      state.selected = { section: "queues", name };
      state.message = null;
    });
  }
  const name = freeName(tree, `c_${from}_${to}`.slice(0, 40));
  const template = Object.values(tree.conveyors || {})[0] || {};
  batch(() => {
    state.layout.paths[name] = route(from, to);
    setChange(`conveyors.${name}`, {
      from, to,
      length: lengthText(pathLength(state.layout.paths[name])),
      speed: template.speed || "0.5 m/s",
      spacing: template.spacing || "0.5 m",
    });
    state.layoutChanged = true;
    state.selected = { section: "conveyors", name };
    state.message = null;
  });
}

/** Removes a conveyor or queue, and the routing or merging rule its ends no longer need. */
function deleteConveyor(tree, name) {
  const section = connectionSection(tree, name);
  const c = tree[section][name];
  setChange(`${section}.${name}`, null);
  delete state.layout.paths[name];
  const others = CONNECTIONS.flatMap(s => Object.entries(tree[s] || {})).filter(([n]) => n !== name);
  const fromPath = `${sectionOf(tree, c.from)}.${c.from}.output`;
  const output = getAt(tree, fromPath);
  if (output) {
    if (others.filter(([, o]) => o.from === c.from).length < 2) setChange(fromPath, null);
    else if (output.weights && name in output.weights) {
      const weights = { ...output.weights }; delete weights[name];
      setChange(fromPath, { ...output, weights });
    }
  }
  const toPath = `${sectionOf(tree, c.to)}.${c.to}.input`;
  const input = getAt(tree, toPath);
  if (input) {
    if (others.filter(([, o]) => o.to === c.to).length < 2) setChange(toPath, null);
    else if (input.order?.includes(name)) setChange(toPath, { ...input, order: input.order.filter(x => x !== name) });
  }
}

function deleteSelected() {
  const sel = state.selected;
  if (!sel?.section) return;
  batch(() => {
    if (isConnection(sel.section)) {
      deleteConveyor(currentTree(), sel.name);
    } else {
      for (const c of graphOf(currentTree()).conveyors) {
        if (c.from === sel.name || c.to === sel.name) deleteConveyor(currentTree(), c.name);
      }
      setChange(`${sel.section}.${sel.name}`, null);
      delete state.layout.positions[sel.name];
      if (state.layout.drawn) delete state.layout.drawn[sel.name];
    }
    state.layoutChanged = true;
    state.message = { kind: "ok", text: `Deleted ${sel.name}.` };
    state.selected = { ...EXPERIMENT };
  });
}

/** Renames in decisions and results too, so they keep pointing at it. */
function renameInDraft(oldPrefix, newPrefix, oldName, newName) {
  const swap = path => path === oldPrefix || path.startsWith(oldPrefix + ".") ? newPrefix + path.slice(oldPrefix.length) : path;
  for (const d of Object.values(state.draft.decisions)) {
    if (d.path) d.path = swap(d.path);
    for (const c of Object.values(d.choices || {})) {
      if (c.changes) c.changes = Object.fromEntries(Object.entries(c.changes).map(([k, v]) => [swap(k), v]));
    }
  }
  if (oldName) for (const r of state.draft.results) if (r.object === oldName) r.object = newName;
}

function rename(section, oldName, newName) {
  const tree = currentTree();
  if (newName === oldName) return;
  if (!isName(newName) || allNames(tree).includes(newName)) {
    state.message = { kind: "error", text: `“${newName}” can't be used: names start with a letter, use letters, digits, _ and -, and must be unique.` };
    return renderEditor();
  }
  batch(() => {
    setChange(`${section}.${oldName}`, null);
    setChange(`${section}.${newName}`, clone(tree[section][oldName]));
    if (isConnection(section)) {
      const c = tree[section][oldName];
      const output = getAt(tree, `${sectionOf(tree, c.from)}.${c.from}.output`);
      if (output?.weights && oldName in output.weights) {
        const weights = Object.fromEntries(Object.entries(output.weights).map(([k, v]) => [k === oldName ? newName : k, v]));
        setChange(`${sectionOf(tree, c.from)}.${c.from}.output`, { ...output, weights });
      }
      const input = getAt(tree, `${sectionOf(tree, c.to)}.${c.to}.input`);
      if (input?.order?.includes(oldName)) {
        setChange(`${sectionOf(tree, c.to)}.${c.to}.input`, { ...input, order: input.order.map(x => x === oldName ? newName : x) });
      }
      state.layout.paths[newName] = state.layout.paths[oldName];
      delete state.layout.paths[oldName];
      renameInDraft(`${section}.${oldName}`, `${section}.${newName}`, oldName, newName);
    } else {
      for (const s of CONNECTIONS) {
        for (const [name, c] of Object.entries(tree[s] || {})) {
          if (c.from === oldName) setChange(`${s}.${name}.from`, newName);
          if (c.to === oldName) setChange(`${s}.${name}.to`, newName);
        }
      }
      for (const [f, v] of Object.entries(tree.vehicles || {})) if (v.home === oldName) setChange(`vehicles.${f}.home`, newName);
      for (const [r, res] of Object.entries(tree.resources || {})) {
        if (res.order?.includes(oldName)) setChange(`resources.${r}.order`, res.order.map(x => x === oldName ? newName : x));
      }
      state.layout.positions[newName] = state.layout.positions[oldName];
      delete state.layout.positions[oldName];
      if (state.layout.drawn?.[oldName]) {
        state.layout.drawn[newName] = state.layout.drawn[oldName];
        delete state.layout.drawn[oldName];
      }
      renameInDraft(`${section}.${oldName}`, `${section}.${newName}`, oldName, newName);
    }
    state.layoutChanged = true;
    state.selected = { section, name: newName };
  });
}

/** Edits of what's on the diagram; adding and connecting are in the Components panel. */
function diagramToolbar() {
  const tool = (label, onclick, attrs = {}) => el("button", { onclick, ...attrs }, label);
  return el("div", { class: "toolbar" },
    tool("Delete", deleteSelected, { id: "delete", disabled: state.selected?.section ? null : true, title: "Delete the selected object or conveyor" }),
    el("span", { style: "flex: 1" }),
    tool("↶ Undo", undo, { id: "undo", disabled: state.undo.length ? null : true, title: "Undo (Ctrl+Z)" }),
    tool("↷ Redo", redo, { id: "redo", disabled: state.redo.length ? null : true, title: "Redo (Ctrl+Shift+Z)" }));
}

// --- side panels: components, definitions and recent runs ----------------------------------

/** The open panel ("components", "definitions", "runs" or null), remembered in this browser. At first,
 *  Components on a wide screen; none on a phone, where the panel covers the diagram. */
state.side = (() => {
  const first = matchMedia("(max-width: 760px)").matches ? null : "components";
  try { const saved = localStorage.getItem("modeling.side"); return saved === null ? first : saved || null; } catch { return first; }
})();

function toggleSide(name) {
  state.side = state.side === name ? null : name;
  try { localStorage.setItem("modeling.side", state.side || ""); } catch { /* not remembered */ }
  renderSide();
}

for (const b of document.querySelectorAll(".activity button")) b.addEventListener("click", () => toggleSide(b.dataset.side));

function renderSide() {
  document.getElementById("side").hidden = !state.side;
  for (const b of document.querySelectorAll(".activity button")) {
    b.classList.toggle("current", b.dataset.side === state.side);
    b.setAttribute("aria-expanded", String(b.dataset.side === state.side));
  }
  document.getElementById("side-runs").hidden = state.side !== "runs";
  const panel = document.getElementById("side-components");
  panel.hidden = state.side !== "components";
  panel.replaceChildren(...componentsPanel());
  const definitions = document.getElementById("side-definitions");
  definitions.hidden = state.side !== "definitions";
  definitions.replaceChildren(...definitionsPanel());
}

/** What the model defines once and objects use: [id, title, names in the model, editor].
 *  Paced lines are made from servers, so they're listed only once there are some. */
const DEFINITIONS = [
  ["part-types", () => say("Part types"), tree => typesOf(tree), partTypesPanel],
  ["attributes", () => say("Part attributes"), tree => attributesOf(tree), attributesPanel],
  ["resources", () => "Resources", tree => Object.keys(tree.resources || {}), resourcesPanel],
  ["vehicles", () => "Vehicles", tree => Object.keys(tree.vehicles || {}), vehiclesPanel],
  ["lines", () => "Paced lines", tree => Object.keys(tree.lines || {}), linesPanel],
  ["schedules", () => "Schedules", tree => Object.keys(tree.schedules || {}), schedulesPanel],
  ["experiment", () => "Experiment", tree => [experimentSummary(tree)], experimentPanel],
];

/** What the panel on the right shows when the page opens or the selection is deleted. */
const EXPERIMENT = { definition: "experiment" };

/** "20 replications · 28 d" */
function experimentSummary(tree) {
  const e = tree.experiment || {};
  const n = e.replications ?? 1;
  return [`${n} replication${n === 1 ? "" : "s"}`, e.run_length].filter(Boolean).join(" · ");
}

/** The list of definitions; each opens its editor in the panel on the right. */
function definitionsPanel() {
  const tree = state.baseTree ? currentTree() : null;
  const items = DEFINITIONS.filter(([id, , names]) => id !== "lines" || (tree && names(tree).length));
  return [
    el("h2", {}, "Definitions"),
    ...items.map(([id, title, names]) => {
      const list = tree ? names(tree) : [];
      return el("button", {
        id: `define-${id}`, class: state.selected?.definition === id ? "component current" : "component",
        disabled: tree ? null : true,
        onclick: () => {
          if (matchMedia("(max-width: 760px)").matches) state.side = null;
          state.selected = { definition: id };
          state.connect = null;
          if (state.tab !== "model") { state.tab = "model"; render(); } else renderEditor();
        },
      }, el("b", {}, title()), el("span", {}, list.length ? list.join(", ") : "none yet"));
    }),
    el("p", { class: "hint" }, "Defined once for the whole model; sources, servers and connections use them. The experiment says how the model runs."),
  ];
}

/** What can be added to the model, and the ways to connect objects. */
function componentsPanel() {
  const ready = Boolean(state.baseTree);
  // Adding works from any tab: it switches to the model's diagram. On a phone the
  // panel covers the diagram, so it closes (for now, not remembered).
  const onModel = f => () => {
    if (matchMedia("(max-width: 760px)").matches) state.side = null;
    if (state.tab !== "model") { state.tab = "model"; render(); }
    f();
  };
  const item = (id, name, about, onclick, extra = {}) => el("button", { id, class: "component", onclick, disabled: ready ? null : true, title: about, ...extra },
    el("b", {}, name), el("span", {}, about));
  // An object can be clicked (it goes beyond the others) or dragged onto the diagram.
  const add = (section, id, name, about) => item(id, name, about,
    e => { if (addDrag?.dropped) return; onModel(() => addObject(section))(e); },
    { "data-add": section, onpointerdown: e => startAddDrag(e, section, name) });
  const connect = (kind, id, name, about) => {
    const on = state.connect?.kind === kind;
    return item(id, on ? `${name}: connecting… (Esc)` : name, about,
      onModel(() => { state.connect = on ? null : { from: null, kind }; renderEditor(); }),
      { class: on ? "component on" : "component", "aria-pressed": String(on) });
  };
  return [
    el("h2", {}, "Components"),
    el("h3", {}, "Add"),
    add("sources", "add-source", "Source", say("Where parts come from")),
    add("servers", "add-server", "Server", say("A machine or station that works on parts")),
    add("sinks", "add-sink", "Sink", say("Where parts leave")),
    el("p", { class: "hint" }, "Drag one onto the diagram, or click to add it next to the others."),
    el("h3", {}, "Connect"),
    connect("conveyors", "connect", "Conveyor", say("Parts travel along it")),
    connect("queues", "connect-queue", "Queue", say("Parts wait in line, no travel time")),
    connect("transports", "connect-transport", "Transport", say("Vehicles carry parts between them")),
    el("p", { class: "hint" }, "To connect, drag from the dot on an object's right edge to the next object. Or click a kind, then an object and the next one."),
  ];
}

/** Dragging a new object from the Components panel onto the diagram. */
let addDrag = null;

function startAddDrag(e, section, label) {
  if (e.button !== 0 || e.currentTarget.disabled) return;
  const narrow = matchMedia("(max-width: 760px)").matches;
  const side = document.getElementById("side");
  const start = [e.clientX, e.clientY];
  let ghost = null, target = null;
  addDrag = { section, moved: false, dropped: false };
  const over = ev => document.elementFromPoint(ev.clientX, ev.clientY)?.closest(".diagram svg") || null;
  const place = ev => { ghost.style.left = `${ev.clientX}px`; ghost.style.top = `${ev.clientY}px`; };
  const mark = svgEl => {
    if (svgEl === target) return;
    target?.parentElement.classList.remove("drop-target");
    svgEl?.parentElement.classList.add("drop-target");
    target = svgEl;
  };
  const begin = () => {
    addDrag.moved = true;
    if (state.tab !== "model") { state.tab = "model"; render(); }
    // On a phone the panel covers the diagram: get it out of the way while dragging.
    if (narrow) side.hidden = true;
    const svgEl = document.querySelector(".diagram svg");
    const scale = Math.max(12, svgEl?.getScreenCTM()?.a || 25);
    const look = artFor(section);
    const box = artBox(look, 0, 0);
    ghost = el("div", { class: "add-ghost", "aria-hidden": "true", style: `transform: translate(${box.x * scale}px, ${box.y * scale}px)` },
      svg("svg", { width: box.w * scale, height: box.h * scale, viewBox: `${box.x} ${box.y} ${box.w} ${box.h}` },
        artSvg(look, 0, 0, section === "servers" ? "var(--busy)" : "var(--object)")),
      el("span", {}, label));
    document.body.append(ghost);
  };
  const finish = ev => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
    document.removeEventListener("keydown", esc, true);
    ghost?.remove();
    mark(null);
    const svgEl = ev && addDrag.moved ? over(ev) : null;
    if (addDrag.moved) {
      addDrag.dropped = true; // the click that may follow isn't an "add"
      if (narrow) { if (svgEl) { state.side = null; renderSide(); } else side.hidden = false; }
    }
    if (svgEl) {
      const pt = svgEl.createSVGPoint();
      pt.x = ev.clientX; pt.y = ev.clientY;
      const p = pt.matrixTransform(svgEl.getScreenCTM().inverse());
      const snap = v => Math.round(v * 2) / 2;
      addObject(section, [snap(p.x), snap(p.y)]);
    }
    setTimeout(() => { addDrag = null; }, 0);
  };
  const move = ev => {
    if (!addDrag.moved) {
      if (Math.hypot(ev.clientX - start[0], ev.clientY - start[1]) < 5) return;
      begin();
    }
    place(ev);
    mark(over(ev));
  };
  const up = ev => finish(ev);
  const cancel = () => finish(null);
  const esc = ev => { if (ev.key === "Escape") { ev.stopPropagation(); finish(null); } };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
  document.addEventListener("keydown", esc, true);
}

let drag = null;

function diagram(tree) {
  const graph = graphOf(tree);
  fillLayout(graph);
  const layout = state.layout;
  const b = drag?.bounds || boundsOf(layout, graph);
  const select = (section, name) => e => {
    e.stopPropagation();
    if (drag?.moved) return;
    if (state.connect && !isConnection(section)) {
      if (!state.connect.from) { state.connect.from = name; return renderEditor(); }
      const { from, kind } = state.connect;
      state.connect = null;
      return connectObjects(from, name, kind);
    }
    state.selected = { section, name };
    renderEditor();
  };
  const isSelected = (section, name) => state.selected?.section === section && state.selected?.name === name;
  const invalid = (section, name) => state.errors.some(e => e.change && e.change.startsWith(`${section}.${name}`));
  const changedObj = (section, name) => Object.keys(state.changes).some(k => k.startsWith(`${section}.${name}.`) || k === `${section}.${name}`);
  const root = svg("svg", {
    viewBox: `${b.minX - 2} ${b.minZ - 1} ${b.maxX - b.minX + 4} ${b.maxZ - b.minZ + 2}`,
    role: "img", "aria-label": `Diagram of ${tree.name}`, class: state.connect ? "connecting" : "",
  });
  for (const c of graph.conveyors) {
    const path = drawnPath(graph, layout, c, layout.paths[c.name]);
    // A belt's top is raised; the outline, arrow and click area follow it.
    const lift = c.queue ? 0 : BELT_TOP;
    const points = path.map(([x, z]) => `${+x.toFixed(3)},${+(z - lift).toFixed(3)}`).join(" ");
    const cls = invalid(c.section, c.name) ? "invalid" : isSelected(c.section, c.name) ? "selected" : "";
    // Conveyors are belts, queues are lanes painted on the floor, transports a dashed route.
    root.append(c.transport
      ? svg("polyline", { points, fill: "none", stroke: "var(--accent)", "stroke-width": 0.14, "stroke-dasharray": "0.08 0.3", "stroke-linecap": "round" })
      : artLineSvg(path, c.queue ? ART_LANE : ART_BELT));
    if (cls) root.append(svg("polyline", { points, fill: "none", class: cls, "vector-effect": "non-scaling-stroke", "stroke-linejoin": "round" }));
    // The way parts go: an arrow a little before the end.
    const last = path.slice(-2);
    const seg = Math.hypot(last[1][0] - last[0][0], last[1][1] - last[0][1]) || 1;
    const back = Math.min(0.6, seg / 2);
    const tip = [last[1][0] - (last[1][0] - last[0][0]) / seg * back, last[1][1] - (last[1][1] - last[0][1]) / seg * back - lift];
    const angle = Math.atan2(last[1][1] - last[0][1], last[1][0] - last[0][0]) * 180 / Math.PI;
    root.append(svg("polygon", { points: "-0.25,-0.28 0.2,0 -0.25,0.28", fill: c.transport ? "var(--accent)" : c.queue ? "#B98A12" : "#FFFFFF", "fill-opacity": c.queue ? 1 : 0.85, transform: `translate(${+tip[0].toFixed(3)},${+tip[1].toFixed(3)}) rotate(${+angle.toFixed(1)})` }));
    root.append(svg("polyline", { points, fill: "none", stroke: "transparent", "stroke-width": 1.4, class: "hit", onclick: select(c.section, c.name), "data-connection": c.name },
      svg("title", {}, `${c.transport ? "transport" : c.queue ? "queue" : "conveyor"} ${c.name}: ${c.from} → ${c.to}`)));
    // The label goes on the longest segment, away from the objects at the ends.
    let i = 0;
    for (let k = 1; k < path.length - 1; k++) if (Math.hypot(path[k + 1][0] - path[k][0], path[k + 1][1] - path[k][1]) > Math.hypot(path[i + 1][0] - path[i][0], path[i + 1][1] - path[i][1])) i = k;
    const [mid, next] = [path[i], path[i + 1]];
    // Above a horizontal segment. Beside a vertical one, on the right of the
    // way parts go and 40% of the way along, so two conveyors side by side in
    // opposite directions (a loop) keep their labels apart.
    const dz = next[1] - mid[1];
    const label = Math.abs(dz) > Math.abs(next[0] - mid[0])
      ? { x: mid[0] + (dz > 0 ? -0.65 : 0.65), y: mid[1] + dz * 0.4 - lift, "text-anchor": dz > 0 ? "end" : "start", "dominant-baseline": "middle" }
      : { x: (mid[0] + next[0]) / 2, y: (mid[1] + next[1]) / 2 - 0.65 - lift, "text-anchor": "middle" };
    const node = tree[c.section][c.name];
    const text = c.transport ? `${c.name} · ${node.by}, ${node.distance}`
      : c.queue ? `${c.name} · ${node.capacity ?? "∞"}` : c.name;
    root.append(svg("text", { ...label, "font-size": 0.6 }, text + (changedObj(c.section, c.name) ? " •" : "")));
  }
  // Objects back to front, so a nearer one's drawing goes over a farther one's.
  const names = [...graph.sources, ...graph.servers, ...graph.sinks].sort((m, n) => layout.positions[m][1] - layout.positions[n][1]);
  for (const name of names) {
    const section = sectionIn(graph, name);
    const [x, z] = layout.positions[name];
    const look = lookOf(graph, layout, name);
    const box = artBox(look, x, z);
    const fill = section === "servers" ? "var(--busy)" : "var(--object)";
    const from = state.connect?.from === name;
    const cls = from ? "selected" : invalid(section, name) ? "invalid" : isSelected(section, name) ? "selected" : "";
    // Selection, errors and drop targets show as a ring around the drawing; a
    // transparent box over it takes the clicks and drags.
    const node = svg("g", { class: ["node", cls, isSelected(section, name) ? "current" : ""].filter(Boolean).join(" ") },
      svg("rect", { x: box.x - 0.25, y: box.y - 0.25, width: box.w + 0.5, height: box.h + 0.5, rx: 0.4, class: "halo", "vector-effect": "non-scaling-stroke" }),
      artSvg(look, x, z, fill),
      svg("rect", {
        x: box.x, y: box.y, width: box.w, height: box.h, rx: 0.2, fill: "transparent",
        class: "hit object", "data-object": name, "data-model": look.kind, "data-at": `${x},${z}`,
        onclick: select(section, name),
        onpointerdown: e => startDrag(e, root, name),
      }, svg("title", {}, `${section.slice(0, -1)} ${name}`)));
    // The handle to drag a connection from: parts leave sources and servers, not sinks.
    if (section !== "sinks" && !state.connect) node.append(svg("circle", {
      cx: box.x + box.w, cy: box.y + box.h / 2, r: 0.28, class: "handle", "vector-effect": "non-scaling-stroke", "data-handle": name,
      onpointerdown: e => startConnectDrag(e, name),
    }, svg("title", {}, `Drag to connect ${name} to the next object`)));
    root.append(node);
    const capacity = section === "servers" ? Number(tree.servers[name]?.capacity) || 1 : 1;
    root.append(svg("text", { x: box.x + box.w / 2, y: box.y - 0.35, "text-anchor": "middle", "font-size": 0.7 },
      name + (capacity > 1 && !look.units ? ` ×${capacity}` : "") + (changedObj(section, name) ? " •" : "")));
  }
  return root;
}

/** The ways to connect two objects: [section, name, what it is]. */
const CONNECT_KINDS = [
  ["conveyors", "Conveyor", () => say("Parts travel along it")],
  ["queues", "Queue", () => say("Parts wait in line, no travel time")],
  ["transports", "Transport", () => say("Vehicles carry parts between them")],
];

/** Dragging from an object's handle to the next object, then choosing how they connect. */
function startConnectDrag(e, from) {
  if (e.button !== 0) return;
  e.stopPropagation();
  e.preventDefault();
  closeConnectMenu();
  const [x0, z0] = state.layout.positions[from];
  const start = [e.clientX, e.clientY];
  let line = null, moved = false;
  const current = () => document.querySelector(".diagram svg");
  const toModel = (s, ev) => {
    const pt = s.createSVGPoint();
    pt.x = ev.clientX; pt.y = ev.clientY;
    return pt.matrixTransform(s.getScreenCTM().inverse());
  };
  // Parts go from a source or server to a server or sink.
  const targetAt = ev => {
    const name = document.elementFromPoint(ev.clientX, ev.clientY)?.closest("[data-object]")?.dataset.object;
    return name && name !== from && sectionOf(currentTree(), name) !== "sources" ? name : null;
  };
  const mark = name => {
    for (const r of document.querySelectorAll(".diagram .target")) r.classList.remove("target");
    if (name) document.querySelector(`.diagram [data-object="${CSS.escape(name)}"]`)?.parentElement.classList.add("target");
  };
  const finish = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", finish);
    document.removeEventListener("keydown", esc, true);
    line?.remove();
    mark(null);
  };
  const move = ev => {
    if (!moved && Math.hypot(ev.clientX - start[0], ev.clientY - start[1]) < 4) return;
    moved = true;
    const s = current();
    if (!s) return;
    // A check may redraw the diagram while dragging: the preview goes on the current one.
    if (!line?.isConnected) {
      const box = artBox(lookOf(graphOf(currentTree()), state.layout, from), x0, z0);
      line = svg("line", { x1: box.x + box.w, y1: box.y + box.h / 2, class: "connect-preview", "vector-effect": "non-scaling-stroke" });
      s.append(line);
    }
    const target = targetAt(ev);
    const [x, z] = target ? state.layout.positions[target] : (p => [p.x, p.y])(toModel(s, ev));
    line.setAttribute("x2", x);
    line.setAttribute("y2", z);
    mark(target);
  };
  const up = ev => {
    const target = moved ? targetAt(ev) : null;
    finish();
    if (target) openConnectMenu(from, target, ev.clientX, ev.clientY);
  };
  const esc = ev => { if (ev.key === "Escape") { ev.stopPropagation(); finish(); } };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", finish);
  document.addEventListener("keydown", esc, true);
}

/** The menu where a dragged connection was dropped: conveyor, queue or transport. */
let connectMenu = null;

function closeConnectMenu() {
  if (!connectMenu) return;
  connectMenu.cleanup();
  connectMenu.node.remove();
  connectMenu = null;
}

function openConnectMenu(from, to, x, y) {
  const fleets = Object.keys(currentTree().vehicles || {}).length;
  const choose = kind => () => {
    closeConnectMenu();
    const tree = currentTree();
    if (sectionOf(tree, from) && sectionOf(tree, to)) connectObjects(from, to, kind);
  };
  const node = el("div", { class: "connect-menu", role: "menu", "aria-label": `Connect ${from} to ${to}` },
    el("div", { class: "title" }, `${from} → ${to}`),
    CONNECT_KINDS.map(([kind, name, about]) => {
      const off = kind === "transports" && !fleets;
      return el("button", { role: "menuitem", class: "component", "data-kind": kind, onclick: choose(kind), disabled: off || null },
        el("b", {}, name), el("span", {}, off ? "Add vehicles first (Definitions → Vehicles)" : about()));
    }));
  node.style.left = `${x}px`;
  node.style.top = `${y}px`;
  document.body.append(node);
  // Keep it on the screen.
  const r = node.getBoundingClientRect();
  if (r.right > innerWidth - 8) node.style.left = `${Math.max(8, innerWidth - 8 - r.width)}px`;
  if (r.bottom > innerHeight - 8) node.style.top = `${Math.max(8, innerHeight - 8 - r.height)}px`;
  const outside = ev => { if (!node.contains(ev.target)) closeConnectMenu(); };
  const esc = ev => { if (ev.key === "Escape") { ev.stopPropagation(); closeConnectMenu(); } };
  document.addEventListener("pointerdown", outside, true);
  document.addEventListener("keydown", esc, true);
  connectMenu = { node, cleanup: () => {
    document.removeEventListener("pointerdown", outside, true);
    document.removeEventListener("keydown", esc, true);
  } };
  node.querySelector("button").focus();
}

/** Dragging an object: moves it on a 0.5 m grid; its conveyors follow. */
function startDrag(e, root, name) {
  if (state.connect || e.button !== 0) return;
  const toModel = ev => {
    const pt = root.createSVGPoint();
    pt.x = ev.clientX; pt.y = ev.clientY;
    return pt.matrixTransform(root.getScreenCTM().inverse());
  };
  const start = toModel(e);
  const [x0, z0] = state.layout.positions[name];
  drag = { name, moved: false, before: snapshot(), bounds: boundsOf(state.layout, graphOf(currentTree())) };
  const move = ev => {
    const p = toModel(ev);
    const snap = v => Math.round(v * 2) / 2;
    const [x, z] = [snap(x0 + p.x - start.x), snap(z0 + p.y - start.y)];
    const [cx, cz] = state.layout.positions[name];
    if (x === cx && z === cz) return;
    if (!drag.moved) { drag.moved = true; state.batching = true; }
    moveObject(name, x, z);
    const old = document.querySelector(".diagram svg");
    if (old) {
      const fresh = diagram(currentTree());
      old.replaceWith(fresh);
      root = fresh;
    }
  };
  const up = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    const moved = drag.moved;
    if (moved) {
      state.batching = false;
      state.undo.push(drag.before);
      state.redo = [];
      state.selected = { section: sectionOf(currentTree(), name), name };
      scheduleCheck();
    }
    // Let the click that follows a drag see `moved`, then forget the drag.
    setTimeout(() => { drag = null; if (moved) renderEditor(); }, 0);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
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
    exposeButton(label, path, value, placeholder),
    error ? el("div", { class: "message" }, error.message) : null);
}

// --- distributions -----------------------------------------------------------------------------

/** Distributions and their parameters, as the model language writes them. */
const DISTRIBUTIONS = {
  constant: { label: "Always the same", params: [["value", "Value", "1 min"]] },
  exponential: { label: "Exponential", params: [["mean", "Mean", "1 min"]] },
  uniform: { label: "Uniform", params: [["min", "Min", "1 min"], ["max", "Max", "2 min"]] },
  triangular: { label: "Triangular", params: [["min", "Min", "1 min"], ["mode", "Most likely", "1.5 min"], ["max", "Max", "2 min"]] },
  lognormal: { label: "Lognormal", params: [["mean", "Mean", "1 min"], ["sd", "Std. deviation", "20 s"]], named: true },
  weibull: { label: "Weibull", params: [["shape", "Shape (a number)", "1.5"], ["scale", "Scale", "1 h"]], named: true },
  gamma: { label: "Gamma", params: [["shape", "Shape (a number)", "2"], ["scale", "Scale", "30 s"]], named: true },
};

/** "triangular(1 min, 1.5 min, 2 min)" -> { kind, values }, or null. */
function parseDistribution(text) {
  if (typeof text !== "string") return null;
  const m = text.trim().match(/^([a-z]+)\((.*)\)$/);
  if (!m) return text.trim() ? { kind: "constant", values: [text.trim()] } : null;
  const d = DISTRIBUTIONS[m[1]];
  if (!d) return null;
  const args = m[2].split(",").map(a => a.trim());
  const values = d.params.map(([name], i) => {
    const named = args.find(a => a.startsWith(name + ":"));
    if (named) return named.slice(name.length + 1).trim();
    return args.some(a => a.includes(":")) ? "" : (args[i] ?? "");
  });
  return { kind: m[1], values };
}

function formatDistribution(kind, values) {
  const d = DISTRIBUTIONS[kind];
  if (kind === "constant") return values[0];
  const args = d.params.map(([name], i) => d.named ? `${name}: ${values[i]}` : values[i]);
  return `${kind}(${args.join(", ")})`;
}

/** A time distribution: pick the kind, fill in its parameters. Anything the
 *  picker can't show stays a plain text field. */
function distField(label, path) {
  const value = getAt(currentTree(), path);
  const parsed = parseDistribution(value);
  if (!parsed) return field(label, path);
  const error = errorFor(path);
  const id = `f-${path.replace(/\W/g, "-")}`;
  const d = DISTRIBUTIONS[parsed.kind];
  const kind = el("select", {
    id: `${id}-kind`, "aria-label": `${label}: distribution`,
    onchange: e => {
      const next = DISTRIBUTIONS[e.target.value];
      // Keep what carries over (by position), fill the rest with examples.
      const values = next.params.map(([, , example], i) => parsed.values[i] && !(next.params[i][0] === "shape") ? parsed.values[i] : example);
      setChange(path, formatDistribution(e.target.value, values));
    },
  }, Object.entries(DISTRIBUTIONS).map(([k, x]) => el("option", { value: k, selected: k === parsed.kind }, x.label)));
  const inputs = d.params.map(([name, text], i) => el("label", { class: "param" }, text,
    el("input", {
      id: `${id}-${name}`, value: parsed.values[i],
      onchange: e => {
        const values = [...parsed.values];
        values[i] = e.target.value.trim();
        setChange(path, formatDistribution(parsed.kind, values));
      },
    })));
  return el("div", { class: `field dist${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("label", { for: `${id}-kind` }, label,
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path), title: "Back to the saved value" }, "reset") : null),
    kind,
    el("div", { class: "params" }, ...inputs),
    el("div", { class: "hint", style: "margin: 2px 0 0" }, el("code", { id }, value)),
    error ? el("div", { class: "message" }, error.message) : null);
}

function panel(tree) {
  if (!state.selected) state.selected = { ...EXPERIMENT };
  const sel = state.selected;
  if (sel.definition) {
    const [, title, , editor] = DEFINITIONS.find(([id]) => id === sel.definition);
    return el("div", { class: "definition" }, el("h3", {}, title()), el("div", { class: "kind" }, "Definition"), editor(tree));
  }
  const node = tree[sel.section]?.[sel.name];
  if (!node) { state.selected = { ...EXPERIMENT }; renderSide(); return panel(tree); }
  const p = `${sel.section}.${sel.name}`;
  const kind = { sources: "Source", servers: "Server", conveyors: "Conveyor", queues: "Queue", transports: "Transport", sinks: "Sink" }[sel.section];
  const header = [el("h3", {}, sel.name), el("div", { class: "kind" }, kind),
    el("div", { class: "field" },
      el("label", { for: "f-name" }, "Name"),
      el("div", { class: "row" },
        el("input", { id: "f-name", value: sel.name, onchange: e => rename(sel.section, sel.name, e.target.value.trim()) }),
        el("button", { style: "flex: 0", onclick: deleteSelected, title: `Delete ${sel.name}` }, "Delete")))];
  const graph = graphOf(tree);
  if (["sources", "servers", "sinks"].includes(sel.section)) {
    const look = lookOf(graph, state.layout, sel.name);
    header.push(el("div", { class: "field" },
      el("label", { for: "f-drawn" }, "Drawn as"),
      el("select", {
        id: "f-drawn",
        onchange: e => batch(() => { state.layout.drawn[sel.name] = e.target.value; state.layoutChanged = true; }),
      }, ...artChoices(sel.section).map(([k, label]) => el("option", { value: k, selected: k === look.kind || null }, label)))));
  }
  const outputs = graph.conveyors.filter(c => c.from === sel.name).map(c => c.name);
  const inputs = graph.conveyors.filter(c => c.to === sel.name).map(c => c.name);
  switch (sel.section) {
    case "sources":
      return el("div", {}, header,
        createsField(tree, p),
        arrivalsField(tree, p),
        countField(say("Parts per arrival (e.g. 20, or 20-30)"), `${p}.group`, "1", "f-group"),
        field("When the conveyor is full", `${p}.when_blocked`, "select", ["wait", "drop"]),
        scheduleField(tree, `${p}.schedule`, "Parts arrive"),
        setsField(tree, p, say("on each new part")),
        outputRule(tree, p, outputs));
    case "servers":
      return el("div", {}, header,
        kindField(tree, p),
        field(say("Capacity (parts at once)"), `${p}.capacity`, "int", [], node.batch ? `${node.batch} (one batch)` : "1 (default)"),
        processTime(tree, p),
        changeoverField(tree, p),
        splitField(tree, p),
        needsField(tree, p),
        needsField(tree, p, "takes"),
        givesBackField(tree, p),
        scheduleField(tree, `${p}.schedule`, "Works"),
        setsField(tree, p, say("when a part is done, before it leaves")),
        inputRule(tree, p, inputs),
        outputRule(tree, p, outputs),
        failures(tree, p));
    case "conveyors":
      return el("div", {}, header,
        el("p", { class: "hint", style: "margin: 0 0 12px" }, `${node.from} → ${node.to}`),
        field("Length", `${p}.length`), field("Speed", `${p}.speed`), field(say("Spacing between parts"), `${p}.spacing`));
    case "transports":
      return el("div", {}, header,
        el("p", { class: "hint", style: "margin: 0 0 12px" }, say(`${node.from} → ${node.to}. A vehicle fetches each part, loads it, carries it and unloads it.`)),
        field("Carried by", `${p}.by`, "select", Object.keys(tree.vehicles || {})),
        field("Distance", `${p}.distance`),
        capacityField(`${p}.capacity`));
    case "queues":
      return el("div", {}, header,
        el("p", { class: "hint", style: "margin: 0 0 12px" }, say(`${node.from} → ${node.to}. Parts wait here in line, with no travel time.`)),
        capacityField(`${p}.capacity`),
        queueOrderField(tree, p),
        field("Target wait (for a service level)", `${p}.target_wait`, "text", [], "none, e.g. 5 min"),
        patienceField(tree, p),
        countField(say("Starts with (parts in it at the start)"), `${p}.starts_with`, "none", "f-starts-with"));
    default:
      return el("div", {}, header, el("p", { class: "hint" }, say("Sinks take every part; there's nothing to set.")));
  }
}

/** When parts arrive: a steady time between arrivals, or a rate by time of day. */
function arrivalsField(tree, p) {
  const profile = getAt(tree, `${p}.arrivals_per_hour`);
  const restocks = getAt(tree, `${p}.restocks`);
  const current = restocks ? "restock" : profile ? "profile" : "steady";
  const RESTOCK_FIELDS = ["restocks", "reorder_point", "order", "order_up_to", "lead_time", "review_every"];
  const mode = el("div", { class: "field" },
    el("label", { for: "f-arrivals-mode" }, say("How parts arrive")),
    el("select", {
      id: "f-arrivals-mode",
      onchange: e => batch(() => {
        for (const f of ["interarrival", "arrivals_per_hour", "group", ...RESTOCK_FIELDS]) setChange(`${p}.${f}`, null);
        if (e.target.value === "profile") {
          setChange(`${p}.arrivals_per_hour`, { "08:00-12:00": 60, "12:00-17:00": 40 });
        } else if (e.target.value === "restock") {
          const queue = Object.keys(tree.queues || {})[0] || "";
          setChange(`${p}.restocks`, queue);
          setChange(`${p}.reorder_point`, 20);
          setChange(`${p}.order`, 50);
          setChange(`${p}.lead_time`, "4 h");
        } else {
          setChange(`${p}.interarrival`, "exponential(1 min)");
        }
      }),
    },
    el("option", { value: "steady", selected: current === "steady" ? true : null }, "Steadily: a time between arrivals"),
    el("option", { value: "profile", selected: current === "profile" ? true : null }, "By time of day: arrivals per hour"),
    el("option", { value: "restock", selected: current === "restock" ? true : null }, "When ordered: to keep a queue stocked")));
  if (current === "restock") return el("div", {}, mode, restockFields(tree, p));
  if (!profile) return el("div", {}, mode, distField("Time between arrivals", `${p}.interarrival`));
  const path = `${p}.arrivals_per_hour`;
  const rows = Object.entries(profile);
  const write = entries => setChange(path, Object.fromEntries(entries));
  const error = errorFor(path) || errorFor(p);
  const perDay = rows.reduce((sum, [period, rate]) => {
    const m = period.match(/^(\d\d):(\d\d)-(\d\d):(\d\d)$/);
    if (!m) return sum;
    let hours = (Number(m[3]) * 60 + Number(m[4]) - Number(m[1]) * 60 - Number(m[2])) / 60;
    if (hours <= 0) hours += 24;
    return sum + hours * Number(rate);
  }, 0);
  return el("div", {}, mode,
    el("div", { class: `field profile${isChanged(path) ? " changed" : ""}${error ? " error" : ""}`, id: "profile" },
      el("label", {}, "Arrivals per hour, by time of day (every day)",
        isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path), title: "Back to the base model's value" }, "reset") : null),
      ...rows.map(([period, rate], i) => el("div", { class: "row", style: "margin-bottom: 4px; align-items: center" },
        el("input", { value: period, "aria-label": "Period", "data-period": i,
          onchange: e => write(rows.map(([k, v], j) => j === i ? [e.target.value.trim(), v] : [k, v])) }),
        el("input", { value: rate, inputmode: "decimal", "aria-label": "Arrivals per hour", "data-rate": i, style: "max-width: 90px",
          onchange: e => write(rows.map(([k, v], j) => j === i ? [k, Number(e.target.value)] : [k, v])) }),
        el("button", { style: "flex: 0", disabled: rows.length < 2 ? true : null, onclick: () => write(rows.filter((_, j) => j !== i)) }, "Remove"))),
      el("button", { class: "link", id: "add-period", style: "padding: 2px 0",
        onclick: () => {
          const last = rows.length ? rows[rows.length - 1][0].split("-")[1] : "08:00";
          const [h, m] = (last || "08:00").split(":").map(Number);
          const end = `${String(Math.min(24, (h || 0) + 2)).padStart(2, "0")}:${String(m || 0).padStart(2, "0")}`;
          write([...rows, [`${last}-${end}`, 30]]);
        } }, "+ Period"),
      el("div", { class: "hint", style: "margin-top: 4px" }, say(`About ${Math.round(perDay)} parts a day. Outside these periods, none arrive.`)),
      error ? el("div", { class: "message" }, error.message) : null));
}

/** A restocking source: which queue it keeps stocked, when and how much it
 *  orders, and how long orders take. */
function restockFields(tree, p) {
  const queues = Object.keys(tree.queues || {});
  const upTo = getAt(tree, `${p}.order_up_to`) !== undefined;
  return el("div", { class: "group", id: "restock" },
    field("Keeps this queue stocked", `${p}.restocks`, "select", queues.length ? queues : [""]),
    field(say("Reorder point (parts in the queue and on the way)"), `${p}.reorder_point`, "int"),
    el("div", { class: "field" },
      el("label", { for: "f-order-kind" }, "Orders"),
      el("select", {
        id: "f-order-kind",
        onchange: e => batch(() => {
          const n = getAt(currentTree(), `${p}.order`) ?? getAt(currentTree(), `${p}.order_up_to`) ?? 50;
          setChange(`${p}.order`, e.target.value === "fixed" ? n : null);
          setChange(`${p}.order_up_to`, e.target.value === "up_to" ? Math.max(n, (getAt(currentTree(), `${p}.reorder_point`) ?? 0) + 1) : null);
        }),
      },
      el("option", { value: "fixed", selected: upTo ? null : true }, say("A fixed number of parts")),
      el("option", { value: "up_to", selected: upTo ? true : null }, "Enough to bring the stock up to a level"))),
    upTo ? field("Order up to", `${p}.order_up_to`, "int") : field(say("Parts per order"), `${p}.order`, "int"),
    distField("Lead time (from order to delivery)", `${p}.lead_time`),
    field("Check the stock every (empty: after every part taken)", `${p}.review_every`, "text", [], "after every part"));
}

/** A whole number of parts: `20`, or a range written `20-30` (each value
 *  equally likely); empty is the default. */
function countField(label, path, placeholder, id) {
  const value = getAt(currentTree(), path);
  const text = value && typeof value === "object"
    ? ("min" in value ? `${value.min}-${value.max}` : Object.values(value)[0] ?? "")
    : value ?? "";
  const error = errorFor(path);
  return el("div", { class: `field${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("label", { for: id }, label,
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path), title: "Back to the base model's value" }, "reset") : null),
    el("input", {
      id, value: text, placeholder, inputmode: "numeric",
      onchange: e => setChange(path, parseCount(e.target.value)),
    }),
    error ? el("div", { class: "message" }, error.message) : null);
}

/** `20` -> 20, `20-30` -> { min: 20, max: 30 }, empty -> null (the default). */
function parseCount(text) {
  const t = text.trim();
  if (t === "") return null;
  const range = t.match(/^(\d+)\s*(?:-|to)\s*(\d+)$/);
  if (range) return { min: Number(range[1]), max: Number(range[2]) };
  return /^\d+$/.test(t) ? Number(t) : t;
}

/** Setup time when the part type changes: one time for any change here;
 *  times per pair of types are kept, and edited in the model file. */
function changeoverField(tree, p) {
  if (typesOf(tree).length < 2) return null;
  const path = `${p}.changeover`;
  const value = getAt(tree, path);
  if (value && typeof value === "object") {
    return el("div", { class: "field" }, el("label", {}, "Changeover (setup when the part type changes)"),
      el("p", { class: "hint", style: "margin: 0" }, Object.entries(value).map(([k, v]) => `${k.replace("->", " → ")}: ${v}`).join(" · ")));
  }
  return field("Changeover (setup when the part type changes)", path, "text", [], "none, e.g. 8 min");
}

/** How long a part waits in a queue before giving up: one time (or a
 *  distribution), or one per part type, shown as written. */
function patienceField(tree, p) {
  const path = `${p}.patience`;
  const value = getAt(tree, path);
  const label = say("Gives up after waiting (each part draws its own)");
  if (value && typeof value === "object") {
    return el("div", { class: "field" }, el("label", {}, label),
      el("p", { class: "hint", style: "margin: 0" }, Object.entries(value).map(([k, v]) => `${k}: ${v}`).join(" · ")));
  }
  return field(label, path, "text", [], "never, e.g. exponential(20 min)");
}

/** What each finished part becomes: a part type and how many (or nothing). */
function splitField(tree, p) {
  const path = `${p}.splits_into`;
  const split = getAt(tree, path);
  const [type, count] = split ? Object.entries(split)[0] || [] : [];
  const types = typesOf(tree);
  const error = errorFor(path);
  return el("div", { class: `field${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("label", { for: "f-split-type" }, say("Splits each finished part into"),
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path), title: "Back to the base model's value" }, "reset") : null),
    el("div", { class: "row" },
      el("select", { id: "f-split-type", onchange: e => setChange(path, e.target.value ? { [e.target.value]: count ?? 2 } : null) },
        el("option", { value: "", selected: split ? null : true }, "— (no split)"),
        ...types.map(t => el("option", { value: t, selected: t === type ? true : null }, t))),
      split ? el("input", {
        id: "f-split-count", style: "max-width: 110px", inputmode: "numeric",
        value: count && typeof count === "object" ? `${count.min}-${count.max}` : count ?? "",
        "aria-label": "How many",
        onchange: e => setChange(path, { [type]: parseCount(e.target.value) ?? 1 }),
      }) : null),
    error ? el("div", { class: "message" }, error.message) : null);
}

/** A queue's capacity: a whole number, or empty for no limit. */
function capacityField(path) {
  const value = getAt(currentTree(), path);
  const error = errorFor(path);
  return el("div", { class: `field${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("label", { for: "f-capacity" }, say("Capacity (parts that can wait)"),
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path), title: "Back to the base model's value" }, "reset") : null),
    el("input", {
      id: "f-capacity", value: value ?? "", placeholder: "no limit", inputmode: "numeric",
      onchange: e => {
        const raw = e.target.value.trim();
        setChange(path, raw === "" ? null : /^\d+$/.test(raw) ? Number(raw) : raw);
      },
    }),
    exposeButton("Capacity", path, value, "no limit"),
    error ? el("div", { class: "message" }, error.message) : null);
}

/** What throughput is counted per: hour, day, week, or a schedule's working hour. */
function throughputPerField(tree) {
  const path = "outputs.throughput_per";
  const value = getAt(tree, path) ?? "hour";
  const options = [["hour", "Hour"], ["day", "Day"], ["week", "Week"],
    ...Object.keys(tree.schedules || {}).map(name => [name, `Working hour of ${name}`])];
  const error = errorFor(path);
  return el("div", { class: `field${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("label", { for: "f-throughput-per" }, "Count throughput per",
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path), title: "Back to the base model's value" }, "reset") : null),
    el("select", { id: "f-throughput-per", onchange: e => setChange(path, e.target.value === "hour" ? null : e.target.value) },
      options.map(([v, label]) => el("option", { value: v, selected: v === value ? true : null }, label))),
    error ? el("div", { class: "message" }, error.message) : null);
}

function experimentPanel(tree) {
  return el("div", { class: "group", id: "experiment" },
    field("Replications", "experiment.replications", "int"),
    field("Warm-up", "experiment.warmup", "text", [], "none"),
    field("Run length (after warm-up)", "experiment.run_length"),
    field("Seed", "experiment.seed", "int"),
    el("label", { class: "check" },
      el("input", {
        type: "checkbox", id: "f-event-log", checked: getAt(tree, "outputs.event_log") === "jsonl" ? true : null,
        onchange: e => setChange("outputs.event_log", e.target.checked ? "jsonl" : null),
      }),
      "Record an animation (replication 1)"),
    throughputPerField(tree));
}

// --- schedules ----------------------------------------------------------------------------------

const DAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

function schedulesPanel(tree) {
  const schedules = Object.entries(tree.schedules || {});
  return el("div", { class: "group", id: "schedules" },
    el("div", { class: "group-title" }, "Schedules"),
    el("p", { class: "hint", style: "margin: 0 0 8px" }, "Working times that repeat every week; the simulation starts on Monday at 00:00. Sources, servers and resources can follow one."),
    ...schedules.map(([name, sch]) => {
      const path = `schedules.${name}`;
      const error = errorFor(path);
      const days = sch.days || DAYS;
      return el("div", { class: `schedule field${error ? " error" : ""}` },
        el("div", { class: "row", style: "align-items: center" },
          el("input", { value: name, "aria-label": `schedule ${name}`, "data-schedule": name, onchange: e => renameSchedule(name, e.target.value.trim()) }),
          el("button", { style: "flex: 0", onclick: () => deleteSchedule(name) }, "Remove")),
        el("label", { style: "margin-top: 6px" }, "Shifts (e.g. 06:00-14:00, 14:00-22:00)"),
        el("input", {
          value: (sch.shifts || []).join(", "), "data-shifts": name,
          onchange: e => setChange(`${path}.shifts`, e.target.value.split(",").map(x => x.trim()).filter(Boolean)),
        }),
        el("label", { class: "check", style: "margin: 6px 0 0" },
          el("input", { type: "checkbox", "data-finish": name, checked: sch.finish_work ? true : null,
            onchange: e => setChange(`${path}.finish_work`, e.target.checked ? true : null) }),
          say("Finish the work in hand after each shift (parts already inside)")),
        el("div", { class: "days" }, ...DAYS.map(d => el("label", { class: "day" },
          el("input", {
            type: "checkbox", checked: days.includes(d) ? true : null, "data-day": `${name}.${d}`,
            onchange: e => {
              const next = DAYS.filter(x => x === d ? e.target.checked : days.includes(x));
              setChange(`${path}.days`, next.length === 7 ? null : next);
            },
          }), d))),
        error ? el("div", { class: "message" }, error.message) : null);
    }),
    el("button", { id: "add-schedule", onclick: () => setChange(`schedules.${freeScheduleName(tree)}`, { shifts: ["08:00-16:00"], days: ["mon", "tue", "wed", "thu", "fri"] }) }, "+ Schedule"));
}

function freeScheduleName(tree) {
  const taken = Object.keys(tree.schedules || {});
  if (!taken.includes("day_shift")) return "day_shift";
  for (let i = 2; ; i++) if (!taken.includes(`schedule_${i}`)) return `schedule_${i}`;
}

/** Everything that follows a schedule: [path of its `schedule` field, name]. */
/** Every place that names a schedule, as [path, schedule]. */
function scheduleUsers(tree) {
  const per = tree.outputs?.throughput_per;
  return ["sources", "servers", "resources"].flatMap(section =>
    Object.entries(tree[section] || {}).filter(([, o]) => o.schedule).map(([n, o]) => [`${section}.${n}.schedule`, o.schedule]))
    .concat(per && per in (tree.schedules || {}) ? [["outputs.throughput_per", per]] : []);
}

function deleteSchedule(name) {
  const tree = currentTree();
  batch(() => {
    for (const [path, s] of scheduleUsers(tree)) if (s === name) setChange(path, null);
    setChange(`schedules.${name}`, null);
  });
}

function renameSchedule(oldName, newName) {
  const tree = currentTree();
  if (newName === oldName) return;
  if (!isName(newName) || newName in (tree.schedules || {})) {
    state.message = { kind: "error", text: `“${newName}” can't be used: names start with a letter, use letters, digits, _ and -, and must be unique.` };
    return renderEditor();
  }
  batch(() => {
    setChange("schedules", Object.fromEntries(Object.entries(tree.schedules).map(([k, v]) => [k === oldName ? newName : k, v])));
    for (const [path, s] of scheduleUsers(tree)) if (s === oldName) setChange(path, newName);
  });
}

/** Which schedule an object follows; none means always. */
function scheduleField(tree, path, label) {
  const names = Object.keys(tree.schedules || {});
  if (!names.length) return null;
  const value = getAt(tree, path);
  const error = errorFor(path);
  const id = `f-${path.replace(/\W/g, "-")}`;
  return el("div", { class: `field${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("label", { for: id }, label,
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path) }, "reset") : null),
    el("select", { id, onchange: e => setChange(path, e.target.value || null) },
      el("option", { value: "", selected: !value ? true : null }, "Always"),
      ...names.map(n => el("option", { value: n, selected: n === value ? true : null }, n))),
    error ? el("div", { class: "message" }, error.message) : null);
}

// --- shared resources ---------------------------------------------------------------------------

function resourcesPanel(tree) {
  const resources = Object.keys(tree.resources || {});
  return el("div", { class: "group", id: "resources" },
    el("div", { class: "group-title" }, "Resources"),
    el("p", { class: "hint", style: "margin: 0 0 8px" }, say("Operators, tools or fixtures that servers share. A server holds what it needs while it processes a part.")),
    ...resources.map(r => el("div", { class: "resource" },
      el("div", { class: "row", style: "align-items: center" },
        el("input", { value: r, "aria-label": `resource ${r}`, "data-resource": r, onchange: e => renameResource(r, e.target.value.trim()) }),
        el("button", { style: "flex: 0", onclick: () => deleteResource(r), title: `Remove ${r}` }, "Remove")),
      field("Units available", `resources.${r}.capacity`, "int"),
      scheduleField(tree, `resources.${r}.schedule`, "Available"),
      resourceOrderField(tree, r))),
    el("button", { id: "add-resource", onclick: () => setChange(`resources.${freeName(tree, "operator")}`, { capacity: 1 }) }, "+ Resource"));
}

/** Resources parts carry that they give back when done at this server. */
function givesBackField(tree, p) {
  const carried = Object.keys(tree.resources || {}).filter(r => Object.values(tree.servers || {}).some(s => s.takes && r in s.takes));
  const path = `${p}.gives_back`;
  const value = getAt(tree, path) || [];
  if (!carried.length && !value.length) return null;
  const error = errorFor(path);
  return el("div", { class: `field${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("label", {}, say("Parts give back, when done here"),
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path) }, "reset") : null),
    ...[...new Set([...carried, ...value])].map(r => el("label", { class: "check" },
      el("input", {
        type: "checkbox", "data-give-back": r, checked: value.includes(r) ? true : null,
        onchange: e => {
          const next = e.target.checked ? [...value, r] : value.filter(x => x !== r);
          setChange(path, next.length ? next : null);
        },
      }), r)),
    error ? el("div", { class: "message" }, error.message) : null);
}

/** Which waiting servers get a resource's units first. */
function resourceOrderField(tree, r) {
  const users = Object.entries(tree.servers || {}).filter(([, s]) => s.needs && r in s.needs).map(([n]) => n);
  if (users.length < 2) return null;
  const path = `resources.${r}.order`;
  const order = getAt(tree, path);
  const error = errorFor(path);
  const listed = order ? [...order.filter(s => users.includes(s)), ...users.filter(s => !order.includes(s))] : [];
  return el("div", { class: `field${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("label", { for: `f-order-${r}` }, "When several servers wait for it",
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path) }, "reset") : null),
    el("select", { id: `f-order-${r}`, "data-resource-order": r, onchange: e => setChange(path, e.target.value === "order" ? users : null) },
      el("option", { value: "", selected: order ? null : true }, "First come, first served"),
      el("option", { value: "order", selected: order ? true : null }, "Serve in this order")),
    ...orderList(listed, o => setChange(path, o)),
    error ? el("div", { class: "message" }, error.message) : null);
}

function vehiclesPanel(tree) {
  const fleets = Object.keys(tree.vehicles || {});
  const homes = ["sources", "servers", "sinks"].flatMap(x => Object.keys(tree[x] || {}));
  return el("div", { class: "group", id: "vehicles" },
    el("div", { class: "group-title" }, "Vehicles"),
    el("p", { class: "hint", style: "margin: 0 0 8px" }, say("Forklifts, carts or AGVs that carry parts along transports, one at a time.")),
    ...fleets.map(f => el("div", { class: "resource" },
      el("div", { class: "row", style: "align-items: center" },
        el("input", { value: f, "aria-label": `vehicles ${f}`, "data-fleet": f, onchange: e => renameFleet(f, e.target.value.trim()) }),
        el("button", { style: "flex: 0", onclick: () => deleteFleet(f), title: `Remove ${f}` }, "Remove")),
      fleetLookField(f),
      field("How many", `vehicles.${f}.count`, "int"),
      field("Speed", `vehicles.${f}.speed`),
      el("div", { class: "row" },
        field("Load time", `vehicles.${f}.load_time`, "text", [], "0 s"),
        field("Unload time", `vehicles.${f}.unload_time`, "text", [], "0 s")),
      field("Start at", `vehicles.${f}.home`, "select", homes))),
    el("button", { id: "add-vehicles", onclick: () => setChange(`vehicles.${freeName(tree, "forklifts")}`, { count: 1, speed: "1.5 m/s", home: homes[0] || "" }) }, "+ Vehicles"));
}

/** Paced lines: stations that move on together every takt. The stations
 *  are listed in the model file; the takt is edited here. */
function linesPanel(tree) {
  const lines = Object.entries(tree.lines || {});
  if (!lines.length) return null;
  return el("div", { class: "group", id: "lines" },
    el("div", { class: "group-title" }, "Paced lines"),
    el("p", { class: "hint", style: "margin: 0 0 8px" }, say("Stations that move their parts on together every takt; a station that isn't done stops the line.")),
    ...lines.map(([name, l]) => el("div", { class: "resource" },
      el("b", {}, name), el("span", { class: "sub" }, (l.stations || []).join(" → ")),
      field("Takt", `lines.${name}.takt`),
      el("button", { class: "link danger", onclick: () => setChange(`lines.${name}`, null) }, "Remove the line"))));
}

/** What a fleet's vehicles are drawn as (a layout change, like an object's). */
function fleetLookField(f) {
  const look = artFor("vehicles", state.layout.fleets?.[f]);
  return el("div", { class: "field" },
    el("label", { for: `f-drawn-${f}` }, "Drawn as"),
    el("select", {
      id: `f-drawn-${f}`, "data-fleet-look": f,
      onchange: e => batch(() => { (state.layout.fleets ||= {})[f] = e.target.value; state.layoutChanged = true; }),
    }, ...artChoices("vehicles").map(([k, label]) => el("option", { value: k, selected: k === look.kind || null }, label))));
}

/** Removes vehicles and the transports they carry. */
function deleteFleet(name) {
  const tree = currentTree();
  batch(() => {
    for (const [t, tr] of Object.entries(tree.transports || {})) if (tr.by === name) deleteConveyor(currentTree(), t);
    setChange(`vehicles.${name}`, null);
    if (state.layout.fleets?.[name]) { delete state.layout.fleets[name]; state.layoutChanged = true; }
  });
}

function renameFleet(oldName, newName) {
  const tree = currentTree();
  if (newName === oldName) return;
  if (!isName(newName) || allNames(tree).includes(newName)) {
    state.message = { kind: "error", text: `“${newName}” can't be used: names start with a letter, use letters, digits, _ and -, and must be unique.` };
    return renderEditor();
  }
  batch(() => {
    setChange(`vehicles.${oldName}`, null);
    setChange(`vehicles.${newName}`, clone(tree.vehicles[oldName]));
    for (const [t, tr] of Object.entries(tree.transports || {})) if (tr.by === oldName) setChange(`transports.${t}.by`, newName);
    if (state.layout.fleets?.[oldName]) {
      state.layout.fleets[newName] = state.layout.fleets[oldName];
      delete state.layout.fleets[oldName];
      state.layoutChanged = true;
    }
  });
}

/** Removes a resource and every server's need for it. */
function deleteResource(name) {
  const tree = currentTree();
  batch(() => {
    for (const [server, srv] of Object.entries(tree.servers || {})) {
      for (const key of ["needs", "takes"]) {
        if (srv[key] && name in srv[key]) {
          const units = { ...srv[key] }; delete units[name];
          setChange(`servers.${server}.${key}`, Object.keys(units).length ? units : null);
        }
      }
      if (srv.gives_back?.includes(name)) {
        const rest = srv.gives_back.filter(r => r !== name);
        setChange(`servers.${server}.gives_back`, rest.length ? rest : null);
      }
    }
    setChange(`resources.${name}`, null);
  });
}

function renameResource(oldName, newName) {
  const tree = currentTree();
  if (newName === oldName) return;
  if (!isName(newName) || allNames(tree).includes(newName)) {
    state.message = { kind: "error", text: `“${newName}” can't be used: names start with a letter, use letters, digits, _ and -, and must be unique.` };
    return renderEditor();
  }
  batch(() => {
    setChange("resources", Object.fromEntries(Object.entries(tree.resources).map(([k, v]) => [k === oldName ? newName : k, v])));
    for (const [server, srv] of Object.entries(tree.servers || {})) {
      for (const key of ["needs", "takes"]) {
        if (srv[key] && oldName in srv[key]) {
          setChange(`servers.${server}.${key}`, Object.fromEntries(Object.entries(srv[key]).map(([k, v]) => [k === oldName ? newName : k, v])));
        }
      }
      if (srv.gives_back?.includes(oldName)) setChange(`servers.${server}.gives_back`, srv.gives_back.map(r => r === oldName ? newName : r));
    }
    renameInDraft(`resources.${oldName}`, `resources.${newName}`, oldName, newName);
  });
}

/** The resources a server holds while processing (`needs`), or that each
 *  part takes here and keeps until a server gives them back (`takes`). */
function needsField(tree, p, key = "needs") {
  const resources = Object.keys(tree.resources || {});
  if (!resources.length) return null;
  const path = `${p}.${key}`;
  const needs = getAt(tree, path) || {};
  const error = errorFor(path);
  return el("div", { class: `group field${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("div", { class: "group-title" }, key === "needs" ? "Needs while processing" : say("Each part takes and keeps (until given back)"),
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path) }, "reset") : null),
    ...resources.map(r => el("div", { class: "row", style: "margin-bottom: 6px; align-items: center" },
      el("span", {}, r),
      el("input", {
        value: needs[r] ?? "", inputmode: "numeric", placeholder: "0", "aria-label": `units of ${r}${key === "needs" ? "" : " taken"}`, [key === "needs" ? "data-need" : "data-take"]: r,
        onchange: e => {
          const n = Number(e.target.value);
          const next = { ...needs };
          if (!e.target.value.trim() || !n) delete next[r]; else next[r] = n;
          setChange(path, Object.keys(next).length ? next : null);
        },
      }))),
    error ? el("div", { class: "message" }, error.message) : null);
}

// --- part types --------------------------------------------------------------------------------

const typesOf = tree => Object.keys(tree.entities || {});

function partTypesPanel(tree) {
  const types = typesOf(tree);
  return el("div", { class: "group", id: "part-types" },
    el("div", { class: "group-title" }, say("Part types")),
    el("p", { class: "hint", style: "margin: 0 0 8px" }, "Sources create these. Process times and routing can differ by type."),
    wordsField(tree),
    ...types.map(t => el("div", { class: "row", style: "margin-bottom: 6px; align-items: center" },
      el("input", { value: t, "aria-label": `part type ${t}`, "data-type": t, onchange: e => renamePartType(t, e.target.value.trim()) }),
      typeLookSelect(t),
      el("button", { style: "flex: 0", disabled: types.length < 2 ? true : null, onclick: () => deletePartType(t),
        title: types.length < 2 ? say("A model needs at least one part type") : `Remove ${t}` }, "Remove"))),
    el("button", { id: "add-part-type", onclick: () => setChange(`entities.${freeTypeName(types)}`, {}) }, say("+ Part type")));
}

/** What flows through the model, in people's words: customers, orders, boxes. */
function wordsField(tree) {
  const words = tree.parts || {};
  const set = (key, value) => {
    const next = { one: words.one || "part", many: words.many || "parts", [key]: value.trim() };
    setChange("parts", next.one === "part" && next.many === "parts" ? null : next);
  };
  return el("div", { class: `field${isChanged("parts") ? " changed" : ""}${errorFor("parts") ? " error" : ""}` },
    el("label", {}, "What flows through the model (one, many)"),
    el("div", { class: "row" },
      el("input", { id: "f-words-one", value: words.one || "", placeholder: "part", "aria-label": "One", onchange: e => set("one", e.target.value || "part") }),
      el("input", { id: "f-words-many", value: words.many || "", placeholder: "parts", "aria-label": "Many", onchange: e => set("many", e.target.value || "parts") })),
    errorFor("parts") ? el("div", { class: "message" }, errorFor("parts").message) : null);
}

/** What a part type's parts are drawn as (a layout change, like an object's). */
function typeLookSelect(t) {
  const look = artFor("entities", state.layout.types?.[t]);
  return el("select", {
    "aria-label": `${t} drawn as`, title: "Drawn as", "data-type-look": t, style: "flex: 0 0 auto; width: auto",
    onchange: e => batch(() => { (state.layout.types ||= {})[t] = e.target.value; state.layoutChanged = true; }),
  }, ...artChoices("entities").map(([k, label]) => el("option", { value: k, selected: k === look.kind || null }, label)));
}

function deletePartType(t) {
  batch(() => {
    setChange(`entities.${t}`, null);
    if (state.layout.types?.[t]) { delete state.layout.types[t]; state.layoutChanged = true; }
  });
}

function freeTypeName(types) {
  for (let i = types.length + 1; ; i++) if (!types.includes(`type_${i}`)) return `type_${i}`;
}

/** Renames a part type everywhere the model names it. */
function renamePartType(oldName, newName) {
  const tree = currentTree();
  if (newName === oldName) return;
  if (!isName(newName) || typesOf(tree).includes(newName)) {
    state.message = { kind: "error", text: `“${newName}” can't be used: names start with a letter, use letters, digits, _ and -, and must be unique.` };
    return renderEditor();
  }
  const renameKey = obj => Object.fromEntries(Object.entries(obj).map(([k, v]) => [k === oldName ? newName : k, v]));
  batch(() => {
    setChange("entities", renameKey(tree.entities));
    if (state.layout.types?.[oldName]) {
      state.layout.types[newName] = state.layout.types[oldName];
      delete state.layout.types[oldName];
      state.layoutChanged = true;
    }
    for (const [name, src] of Object.entries(tree.sources || {})) {
      if (src.creates === oldName) setChange(`sources.${name}.creates`, newName);
      else if (src.creates && typeof src.creates === "object" && oldName in src.creates) setChange(`sources.${name}.creates`, renameKey(src.creates));
      if (src.output?.routes && oldName in src.output.routes) setChange(`sources.${name}.output`, { ...src.output, routes: renameKey(src.output.routes) });
    }
    for (const [name, srv] of Object.entries(tree.servers || {})) {
      if (srv.process_time && typeof srv.process_time === "object" && oldName in srv.process_time) setChange(`servers.${name}.process_time`, renameKey(srv.process_time));
      if (srv.output?.routes && oldName in srv.output.routes) setChange(`servers.${name}.output`, { ...srv.output, routes: renameKey(srv.output.routes) });
    }
    for (const [q, queue] of Object.entries(tree.queues || {})) {
      if (queue.order?.by_type?.includes(oldName)) setChange(`queues.${q}.order`, { by_type: queue.order.by_type.map(t => t === oldName ? newName : t) });
    }
    // Formulas name types in `type == <type>`.
    const escaped = oldName.replace(/[-]/g, "\\-");
    rewriteFormulas(tree, text => text.replace(new RegExp(`(type\\s*[!=]=\\s*)${escaped}(?![\\w-])`, "g"), `$1${newName}`));
  });
}

// --- attributes --------------------------------------------------------------------------------

const attributesOf = tree => Object.keys(tree.attributes || {});
/** Words formulas use, which can't name an attribute. */
const FORMULA_WORDS = ["now", "type", "and", "or", "not", "ms", "s", "min", "h", "d", "mm", "cm", "m", "km", ...Object.keys(DISTRIBUTIONS)];

function attributesPanel(tree) {
  const names = attributesOf(tree);
  return el("div", { class: "group", id: "attributes" },
    el("div", { class: "group-title" }, say("Part attributes")),
    el("p", { class: "hint", style: "margin: 0 0 8px" }, say("Values each part carries, like a due date or how many times it was reworked. Sources and servers set them with formulas.")),
    ...names.map(a => el("div", { class: "resource" },
      el("div", { class: "row", style: "align-items: center" },
        el("input", { value: a, "aria-label": `attribute ${a}`, "data-attribute": a, onchange: e => renameAttribute(a, e.target.value.trim()) }),
        el("button", { style: "flex: 0", onclick: () => deleteAttribute(a), title: `Remove ${a}` }, "Remove")),
      field("Starts at (a number, or a time like 0 s)", `attributes.${a}.default`))),
    names.length < 8 ? el("button", { id: "add-attribute", onclick: () => setChange(`attributes.${freeAttributeName(names)}`, { default: 0 }) }, "+ Attribute") : null);
}

function freeAttributeName(names) {
  for (let i = names.length + 1; ; i++) if (!names.includes(`attribute_${i}`)) return `attribute_${i}`;
}

/** Applies `rewrite` to every formula in sources' and servers' `sets`. */
function rewriteFormulas(tree, rewrite, renameKey = k => k) {
  for (const section of ["sources", "servers"]) {
    for (const [name, node] of Object.entries(tree[section] || {})) {
      if (!node.sets) continue;
      const next = Object.fromEntries(Object.entries(node.sets).map(([k, v]) => [renameKey(k), typeof v === "string" ? rewrite(v) : v]));
      if (!equal(next, node.sets)) setChange(`${section}.${name}.sets`, next);
    }
  }
}

/** Renames an attribute everywhere: its `sets` and the formulas that read it. */
function renameAttribute(oldName, newName) {
  const tree = currentTree();
  if (newName === oldName) return;
  if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(newName) || FORMULA_WORDS.includes(newName) || attributesOf(tree).includes(newName)) {
    state.message = { kind: "error", text: `“${newName}” can't be used: attribute names use letters, digits and _, must be unique, and can't be a word formulas use (like now, min or uniform).` };
    return renderEditor();
  }
  const word = new RegExp(`\\b${oldName}\\b`, "g");
  batch(() => {
    setChange("attributes", Object.fromEntries(Object.entries(tree.attributes).map(([k, v]) => [k === oldName ? newName : k, v])));
    rewriteFormulas(tree, text => text.replace(word, newName), k => k === oldName ? newName : k);
    // Queue orders and input rules that compare it.
    for (const [q, queue] of Object.entries(tree.queues || {})) {
      for (const kind of ["lowest", "highest"]) {
        if (queue.order?.[kind] === oldName) setChange(`queues.${q}.order`, { [kind]: newName });
      }
    }
    for (const [s, server] of Object.entries(tree.servers || {})) {
      if (server.input?.by === oldName) setChange(`servers.${s}.input`, { ...server.input, by: newName });
    }
    renameInDraft(`attributes.${oldName}`, `attributes.${newName}`, oldName, newName);
  });
}

/** Removes an attribute and stops setting it; formulas still reading it show an error. */
function deleteAttribute(name) {
  const tree = currentTree();
  batch(() => {
    for (const section of ["sources", "servers"]) {
      for (const [node, value] of Object.entries(tree[section] || {})) {
        if (!value.sets || !(name in value.sets)) continue;
        const sets = { ...value.sets }; delete sets[name];
        setChange(`${section}.${node}.sets`, Object.keys(sets).length ? sets : null);
      }
    }
    setChange(attributesOf(tree).length > 1 ? `attributes.${name}` : "attributes", null);
  });
}

/** The attributes a source or server sets, each with a formula. */
function setsField(tree, p, when) {
  const names = attributesOf(tree);
  if (!names.length) return null;
  const path = `${p}.sets`;
  const sets = getAt(tree, path) || {};
  const update = next => setChange(path, Object.keys(next).length ? next : null);
  const unset = names.filter(a => !(a in sets));
  return el("div", { class: `group field${isChanged(path) ? " changed" : ""}`, id: "sets" },
    el("div", { class: "group-title" }, `Sets ${when}`,
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path) }, "reset") : null),
    ...Object.entries(sets).map(([a, formula]) => {
      const error = errorFor(`${path}.${a}`);
      return el("div", { class: error ? "error" : null, style: "margin-bottom: 10px" },
        el("div", { class: "row", style: "align-items: center; margin-bottom: 4px" },
          el("select", {
            "aria-label": "attribute", "data-set-attribute": a,
            onchange: e => update(Object.fromEntries(Object.entries(sets).map(([k, v]) => [k === a ? e.target.value : k, v]))),
          }, [a, ...unset].map(n => el("option", { value: n, selected: n === a ? true : null }, n))),
          el("button", { style: "flex: 0", onclick: () => { const next = { ...sets }; delete next[a]; update(next); }, title: `Stop setting ${a}` }, "Remove")),
        el("input", { value: formula, "data-set": a, "aria-label": `formula for ${a}`, spellcheck: "false", onchange: e => update({ ...sets, [a]: e.target.value.trim() || a }) }),
        error ? el("div", { class: "message" }, error.message) : null);
    }),
    unset.length ? el("button", { id: "add-set", onclick: () => update({ ...sets, [unset[0]]: unset[0] }) }, "+ Set an attribute") : null,
    el("p", { class: "hint", style: "margin: 6px 0 0" }, "Formulas use attributes, now, times like 4 h, + - * / and draws like uniform(1, 10). Examples: now + 4 h, rework + 1."));
}

/** What a source creates: one type, or a mix with shares. */
function createsField(tree, p) {
  const types = typesOf(tree);
  const path = `${p}.creates`;
  const value = getAt(tree, path);
  const error = errorFor(path);
  const mixed = value && typeof value === "object";
  let body;
  if (!mixed) {
    body = [
      el("select", { id: "f-creates", onchange: e => setChange(path, e.target.value) },
        types.map(t => el("option", { value: t, selected: t === value }, t))),
      types.length > 1 ? el("button", { class: "link", id: "mix", style: "padding: 2px 0", onclick: () => setChange(path, Object.fromEntries(types.map(t => [t, +(1 / types.length).toFixed(4)]))) }, say("A mix of part types…")) : null,
    ];
  } else {
    body = [
      ...types.map(t => el("div", { class: "row", style: "margin-bottom: 6px; align-items: center" },
        el("span", {}, t),
        el("input", {
          value: value[t] ?? "", inputmode: "decimal", placeholder: "0", "aria-label": `share of ${t}`, "data-share": t,
          onchange: e => {
            const shares = { ...value };
            const n = Number(e.target.value);
            if (e.target.value.trim() === "" || n === 0) delete shares[t]; else shares[t] = n;
            setChange(path, shares);
          },
        }))),
      el("div", { class: "hint", style: "margin: 0" }, "Shares of the arrivals; they must add up to 1."),
      el("button", { class: "link", style: "padding: 2px 0", onclick: () => setChange(path, Object.keys(value)[0] || types[0]) }, say("Only one part type")),
    ];
  }
  return el("div", { class: `field${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("label", { for: "f-creates" }, "Creates",
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path) }, "reset") : null),
    ...body,
    error ? el("div", { class: "message" }, error.message) : null);
}

// --- batches and assembly -------------------------------------------------------------------

/** What a server does with its parts: one at a time, in batches, or assembling kits. */
function kindField(tree, p) {
  const node = getAt(tree, p);
  const types = typesOf(tree);
  const kind = node.assembles ? "assembly" : node.batch ? "batch" : "single";
  const choose = e => {
    const k = e.target.value;
    const time = typeof node.process_time === "object" ? Object.values(node.process_time)[0] : node.process_time;
    batch(() => {
      setChange(`${p}.batch`, k === "batch" ? 4 : null);
      setChange(`${p}.assembles`, k === "assembly" ? Object.fromEntries(types.slice(0, 2).map(t => [t, 1])) : null);
      setChange(`${p}.makes`, k === "assembly" ? (types[types.length - 1] || null) : null);
      if (k !== "single") setChange(`${p}.process_time`, time);
      if (k === "batch" && node.capacity) setChange(`${p}.capacity`, null);
    });
  };
  const select = el("select", { id: "f-kind", onchange: choose },
    [["single", say("One part at a time")], ["batch", "In batches (an oven, a washer)"], ["assembly", say("Assembles kits into a new part")]]
      .map(([v, l]) => el("option", { value: v, selected: v === kind ? true : null }, l)));
  const parts = [el("div", { class: "field" }, el("label", { for: "f-kind" }, "Works on"), select)];
  if (kind === "batch") {
    parts.push(field(say("Batch size (parts processed together)"), `${p}.batch`, "int"));
  }
  if (kind === "assembly") {
    const path = `${p}.assembles`;
    const error = errorFor(path);
    parts.push(el("div", { class: `group field${error ? " error" : ""}` },
      el("div", { class: "group-title" }, say("Kit: parts it takes")),
      ...types.map(t => el("div", { class: "row", style: "margin-bottom: 6px; align-items: center" },
        el("span", {}, t),
        el("input", {
          value: node.assembles[t] ?? "", inputmode: "numeric", placeholder: "0", "aria-label": `${t} per kit`, "data-kit": t,
          onchange: e => {
            const n = Number(e.target.value);
            const next = { ...node.assembles };
            if (!e.target.value.trim() || !n) delete next[t]; else next[t] = n;
            setChange(path, next);
          },
        }))),
      error ? el("div", { class: "message" }, error.message) : null),
      field("Makes", `${p}.makes`, "select", types),
      el("div", { class: "field" },
        el("label", { for: "f-keeps-clock" }, say("Time the new part from")),
        el("select", { id: "f-keeps-clock", onchange: e => setChange(`${p}.keeps_clock_of`, e.target.value || null) },
          el("option", { value: "", selected: node.keeps_clock_of ? null : true }, "Its earliest component"),
          ...Object.keys(node.assembles || {}).map(t => el("option", { value: t, selected: node.keeps_clock_of === t ? true : null }, `Its ${t} (e.g. the order)`)))));
  }
  return el("div", {}, ...parts);
}

/** A process time, the same for every part type or one per type. */
function processTime(tree, p) {
  const types = typesOf(tree);
  const path = `${p}.process_time`;
  const value = getAt(tree, path);
  if (value && typeof value === "object") {
    return el("div", { class: "group" },
      el("div", { class: "group-title" }, say("Process time by part type")),
      ...Object.keys(value).map(t => distField(t, `${path}.${t}`)),
      ...types.filter(t => !(t in value)).map(t => el("button", { class: "link", onclick: () => setChange(`${path}.${t}`, Object.values(value)[0]) }, `+ A time for ${t}`)),
      el("button", { class: "link", style: "display: block; padding: 2px 0", onclick: () => setChange(path, Object.values(value)[0]) }, say("The same for every part type")));
  }
  return el("div", {},
    distField("Process time", path),
    types.length > 1 && !getAt(tree, p).batch && !getAt(tree, p).assembles ? el("button", { class: "link", id: "per-type", style: "padding: 0 0 10px", onclick: () => setChange(path, Object.fromEntries(types.map(t => [t, value]))) }, say("Different times per part type…")) : null);
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
        : r === "by_type"
          ? { rule: r, routes: Object.fromEntries(typesOf(tree).map(t => [t, outputs[0]])) }
          : r === "by_condition"
            ? { rule: r, routes: [{ when: "uniform(0, 1) < 0.5", to: outputs[0] }, { to: outputs[1] }] }
            : { rule: r });
    },
  }, el("option", { value: "", selected: !value.rule, disabled: true }, "choose a rule"),
  OUTPUT_RULES.map(r => el("option", { value: r, selected: value.rule === r }, r.replaceAll("_", " "))));
  const routes = value.rule === "by_condition" && Array.isArray(value.routes) ? conditionRoutes(path, value, outputs)
    : value.rule === "by_type" ? typesOf(tree).map(t => el("div", { class: "row field", style: "margin-bottom: 6px; align-items: center" },
    el("span", {}, t),
    el("select", {
      "aria-label": `conveyor for ${t}`, "data-route": t,
      onchange: e => setChange(path, { ...value, routes: { ...value.routes, [t]: e.target.value } }),
    }, outputs.map(o => el("option", { value: o, selected: value.routes?.[t] === o }, o))))) : [];
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
    el("label", { for: `f-${path}-rule` }, say("Sends parts to ") + outputs.join(", "),
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path) }, "reset") : null),
    rule, ...routes, ...weights,
    error ? el("div", { class: "message" }, error.message) : null);
}

/** `by_condition`: the first route whose condition holds; the last takes the rest. */
function conditionRoutes(path, value, outputs) {
  const routes = value.routes;
  const update = next => setChange(path, { ...value, routes: next });
  const to = (route, i) => el("select", {
    "aria-label": i === routes.length - 1 ? "otherwise to" : `route ${i + 1} to`, "data-route-to": i,
    onchange: e => update(routes.map((r, k) => k === i ? { ...r, to: e.target.value } : r)),
  }, outputs.map(o => el("option", { value: o, selected: route.to === o ? true : null }, o)));
  return [
    ...routes.slice(0, -1).map((route, i) => {
      const error = errorFor(`${path}.routes.${i}`);
      return el("div", { class: `route${error ? " error" : ""}` },
        el("div", { class: "row", style: "align-items: center; margin-bottom: 4px" },
          el("span", { style: "flex: 0 0 auto" }, i === 0 ? "When" : "Else when"),
          el("input", {
            value: route.when ?? "", "data-when": i, "aria-label": `condition ${i + 1}`, spellcheck: "false",
            onchange: e => update(routes.map((r, k) => k === i ? { ...r, when: e.target.value.trim() } : r)),
          })),
        el("div", { class: "row", style: "align-items: center; margin-bottom: 8px" },
          el("span", { style: "flex: 0 0 auto" }, "go to"), to(route, i),
          routes.length > 2 ? el("button", { style: "flex: 0", title: "Remove this route", onclick: () => update(routes.filter((_, k) => k !== i)) }, "Remove") : null),
        error ? el("div", { class: "message" }, error.message) : null);
    }),
    el("div", { class: "row", style: "align-items: center; margin-bottom: 6px" },
      el("span", { style: "flex: 0 0 auto" }, "Otherwise go to"), to(routes[routes.length - 1], routes.length - 1)),
    el("button", { "data-action": "add-route", onclick: () => update([...routes.slice(0, -1), { when: "uniform(0, 1) < 0.5", to: outputs[0] }, routes[routes.length - 1]]) }, "+ Condition"),
    el("p", { class: "hint", style: "margin: 6px 0 0" }, "Conditions compare attributes, now and draws (rework >= 2, due < now, uniform(0, 1) < 0.1), and can use and, or, not, and type == <part type>."),
  ];
}

function inputRule(tree, p, inputs) {
  if (inputs.length < 2) return null;
  const path = `${p}.input`;
  const value = getAt(tree, path) || {};
  const error = errorFor(path);
  const attributes = attributesOf(tree);
  const byAttribute = ["lowest", "highest"].includes(value.rule);
  const rules = [...INPUT_RULES, ...(attributes.length || byAttribute ? ["lowest", "highest"] : [])];
  const rule = el("select", {
    id: `f-${path}-rule`,
    onchange: e => {
      const r = e.target.value;
      setChange(path, r === "priority" ? { rule: r, order: inputs } : ["lowest", "highest"].includes(r) ? { rule: r, by: value.by || attributes[0] } : { rule: r });
    },
  }, el("option", { value: "", selected: !value.rule, disabled: true }, "choose a rule"),
  rules.map(r => el("option", { value: r, selected: value.rule === r },
    r === "lowest" ? "lowest attribute first" : r === "highest" ? "highest attribute first" : r.replaceAll("_", " "))));
  const by = byAttribute ? el("select", { id: `f-${path}-by`, "aria-label": "attribute", style: "margin-top: 6px", onchange: e => setChange(path, { rule: value.rule, by: e.target.value }) },
    attributes.map(a => el("option", { value: a, selected: a === value.by ? true : null }, a))) : null;
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
    el("label", { for: `f-${path}-rule` }, say("Takes parts from ") + inputs.join(", "),
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path) }, "reset") : null),
    rule, by, ...order,
    error ? el("div", { class: "message" }, error.message) : null);
}

/** A list to put in order, highest priority first, with a ↑ on each item. */
function orderList(items, onOrder, label = x => x) {
  return items.map((x, i) => el("div", { class: "order-item" },
    el("span", {}, `${i + 1}. ${label(x)}`),
    el("button", {
      disabled: i === 0 ? true : null, "aria-label": `raise ${x}`,
      onclick: () => { const o = [...items]; [o[i - 1], o[i]] = [o[i], o[i - 1]]; onOrder(o); },
    }, "↑")));
}

/** Which part a queue lets go next. */
function queueOrderField(tree, p) {
  const path = `${p}.order`;
  const value = getAt(tree, path);
  const error = errorFor(path);
  const types = typesOf(tree);
  const current = !value ? "first_in" : typeof value === "string" ? value
    : "lowest" in value ? `lowest:${value.lowest}` : "highest" in value ? `highest:${value.highest}` : "by_type";
  const options = [["first_in", "First in, first out"], ["last_in", "Last in, first out"],
    ...attributesOf(tree).flatMap(a => [[`lowest:${a}`, `Lowest ${a} first`], [`highest:${a}`, `Highest ${a} first`]]),
    ...(types.length > 1 || current === "by_type" ? [["by_type", say("By part type, in this order")]] : [])];
  if (!options.some(([v]) => v === current)) options.push([current, current.replace(":", " ")]);
  const choose = v => {
    const [kind, a] = v.split(":");
    setChange(path, kind === "first_in" ? null : kind === "last_in" ? kind : kind === "by_type" ? { by_type: types } : { [kind]: a });
  };
  const listed = current === "by_type" ? [...value.by_type, ...types.filter(t => !value.by_type.includes(t))] : [];
  return el("div", { class: `field${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("label", { for: "f-queue-order" }, say("Which part goes next"),
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path) }, "reset") : null),
    el("select", { id: "f-queue-order", onchange: e => choose(e.target.value) },
      options.map(([v, text]) => el("option", { value: v, selected: v === current ? true : null }, text))),
    ...orderList(listed, o => setChange(path, { by_type: o })),
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
      distField("Uptime between failures", `${path}.${i}.uptime`),
      distField("Downtime (repair)", `${path}.${i}.downtime`),
      el("div", { class: "row" },
        field("Uptime counts", `${path}.${i}.counts`, "select", ["busy_time", "clock_time"]),
        field(say("Interrupted part"), `${path}.${i}.on_failure`, "select", ["resume", "restart"])))),
    el("button", { onclick: add }, "Add a failure"));
}

// --- what the Simulation page offers: the Decisions tab -------------------------------------------

/** A number as the model writes it: `2`, or `8 m`. */
function parseNumber(value) {
  if (typeof value === "number") return { n: value, unit: null };
  const m = typeof value === "string" && value.trim().match(/^(-?\d+(?:\.\d+)?)\s+(\S+)$/);
  return m ? { n: Number(m[1]), unit: m[2] } : null;
}

const numberDecisionAt = path => Object.entries(state.draft.decisions).find(([, d]) => d.path === path);

/** "⚑ Make this a decision" under number fields. */
function exposeButton(label, path, value, placeholder) {
  if (path.startsWith("experiment.") || !state.draft) return null;
  const existing = numberDecisionAt(path);
  if (existing) {
    return el("button", { class: "expose on", onclick: () => { state.tab = "decisions"; render(); },
      title: "This is a decision on the Simulation page; set its limits on the Decisions tab" }, `⚑ Decision: ${existing[1].label}`);
  }
  const parsed = parseNumber(value ?? (placeholder ? parseFloat(placeholder) : undefined));
  if (!parsed || Number.isNaN(parsed.n)) return null;
  return el("button", { class: "expose", "data-expose": path, onclick: () => exposeNumber(label, path, value, parsed) }, "⚑ Make this a decision");
}

function slug(text, taken) {
  const base = (text.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "").slice(0, 40)) || "decision";
  let id = base;
  for (let i = 2; taken.includes(id); i++) id = `${base}_${i}`;
  return id;
}

function exposeNumber(label, path, value, { n, unit }) {
  if (value === undefined) {
    // The model must write the value for a decision to set it: the default.
    setChange(path, n);
    state.written.add(path);
  }
  const object = path.split(".")[1];
  const integer = unit === null && Number.isInteger(n);
  const decision = integer
    ? { label: `${object}: ${label}`, path, min: Math.max(1, Math.floor(n / 2)), max: Math.max(n * 2, n + 2) }
    : { label: `${object}: ${label}`, path, min: n / 2, max: n * 2 };
  if (unit) decision.unit = unit;
  const id = slug(`${object} ${label.replace(/\(.*\)/, "")}`, Object.keys(state.draft.decisions));
  state.draft.decisions[id] = decision;
  state.tab = "decisions";
  state.message = { kind: "ok", text: `“${decision.label}” is now a decision on the Simulation page. Check its name and limits, then save a version.` };
  scheduleCheck();
  render();
}

function startOffer() {
  const choices = Object.entries(state.draft.decisions).filter(([, d]) => d.choices);
  state.offer = { decision: choices.length ? choices[0][0] : "", newLabel: "", optionLabel: state.saveMessage || "", currentLabel: "As it is now" };
  renderEditor();
}

/** Turn the pending model changes into an option to pick on the Simulation page. */
function offerForm() {
  const o = state.offer;
  const choices = Object.entries(state.draft.decisions).filter(([, d]) => d.choices);
  const isNew = o.decision === "";
  const input = (key, placeholder, id) => el("input", { id, value: o[key], placeholder, oninput: e => { o[key] = e.target.value; } });
  return el("div", { class: "card offer", id: "offer-form" },
    el("div", { class: "group-title" }, "Offer these changes as an option"),
    el("p", { class: "hint", style: "margin: 0 0 10px" },
      "These pending changes won't go into the model. Instead, they become an option to pick in a scenario on the Simulation page; the model stays as it is."),
    el("div", { class: "field" }, el("label", {}, "Decision"),
      el("select", { id: "offer-decision", onchange: e => { o.decision = e.target.value; renderEditor(); } },
        ...choices.map(([id, d]) => el("option", { value: id, selected: o.decision === id ? true : null }, d.label)),
        el("option", { value: "", selected: isNew ? true : null }, "A new decision…"))),
    isNew ? el("div", { class: "field" }, el("label", {}, "The decision (e.g. Repairs on mill B)"), input("newLabel", "Decision", "offer-new-label")) : null,
    isNew ? el("div", { class: "field" }, el("label", {}, "The current setup is called"), input("currentLabel", "As it is now", "offer-current-label")) : null,
    el("div", { class: "field" }, el("label", {}, "This option is called (e.g. Faster crew)"), input("optionLabel", "Option", "offer-option-label")),
    el("div", { class: "actions" },
      el("button", { class: "primary", id: "offer-confirm", onclick: confirmOffer }, "Offer as an option"),
      el("button", { onclick: () => { state.offer = null; renderEditor(); } }, "Cancel")));
}

function confirmOffer() {
  const o = state.offer;
  const optionLabel = o.optionLabel.trim();
  if (!optionLabel || (o.decision === "" && !o.newLabel.trim())) {
    state.message = { kind: "error", text: "Name the option (and the decision, if it's new)." };
    return renderEditor();
  }
  let id = o.decision;
  if (id === "") {
    id = slug(o.newLabel, Object.keys(state.draft.decisions));
    state.draft.decisions[id] = { label: o.newLabel.trim(), choices: { current: { label: o.currentLabel.trim() || "As it is now" } } };
  }
  const decision = state.draft.decisions[id];
  const optionId = slug(optionLabel, Object.keys(decision.choices));
  // Defaults written for number decisions stay in the model.
  const offered = Object.fromEntries(Object.entries(state.changes).filter(([path]) => !state.written.has(path)));
  decision.choices[optionId] = { label: optionLabel, changes: clone(offered) };
  for (const path of Object.keys(offered)) delete state.changes[path];
  state.offer = null;
  state.tab = "decisions";
  state.message = { kind: "ok", text: `“${optionLabel}” is now an option of “${decision.label}”. The model is back as it was. Save a version to keep this.` };
  scheduleCheck();
  render();
}

function renderDecisions(content) {
  const d = state.draft;
  const problems = state.exposed?.decision_problems || [];
  const editDecision = (id, f) => { f(d.decisions[id]); scheduleCheck(); renderEditor(); };
  const text = (value, onchange, attrs = {}) => el("input", { value: value ?? "", onchange: e => onchange(e.target.value), ...attrs });
  const num = (value, onchange, attrs = {}) => el("input", { type: "number", step: "any", value: value ?? "", onchange: e => onchange(e.target.value === "" ? undefined : Number(e.target.value)), ...attrs });
  const show = v => typeof v === "string" ? v : JSON.stringify(v);

  const decisionCards = Object.entries(d.decisions).map(([id, dec]) => {
    let body;
    if (dec.choices) {
      const options = Object.entries(dec.choices);
      body = el("div", { class: "options" }, ...options.map(([cid, c], i) => el("div", { class: "option" },
        el("div", { class: "row" },
          text(c.label, v => editDecision(id, x => { x.choices[cid].label = v; }), { "aria-label": "Option name" }),
          i === 0 ? el("span", { class: "pill" }, "current setup")
            : el("button", { class: "link danger", onclick: () => editDecision(id, x => { delete x.choices[cid]; }) }, "Remove")),
        i === 0 ? el("p", { class: "hint", style: "margin: 2px 0 0" }, "The model as it is.")
          : el("ul", { class: "changes" }, ...Object.entries(c.changes || {}).map(([path, v]) => el("li", {}, el("code", {}, path), " → ", el("code", {}, show(v))))))),
      el("p", { class: "hint", style: "margin: 4px 0 0" }, "To add an option, make the changes on the Model tab, then click “Offer as an option…”."));
    } else {
      body = el("div", {},
        el("p", { class: "hint", style: "margin: 0 0 8px" }, "Sets ", el("code", {}, dec.path), dec.unit ? ` (in ${dec.unit})` : ""),
        el("div", { class: "row" },
          el("div", { class: "field" }, el("label", {}, "Lowest"), num(dec.min, v => editDecision(id, x => { x.min = v; }), { "data-bound": `${id}.min` })),
          el("div", { class: "field" }, el("label", {}, "Highest"), num(dec.max, v => editDecision(id, x => { x.max = v; }), { "data-bound": `${id}.max` })),
          el("div", { class: "field" }, el("label", {}, "In steps of (optional)"), num(dec.step, v => editDecision(id, x => { if (v === undefined) delete x.step; else x.step = v; })))));
    }
    return el("div", { class: "card decision-edit", "data-decision-id": id },
      el("div", { class: "row", style: "align-items: center" },
        text(dec.label, v => editDecision(id, x => { x.label = v; }), { class: "title-input", "aria-label": "Decision name" }),
        el("button", { class: "link danger", style: "flex: 0", onclick: () => { delete d.decisions[id]; scheduleCheck(); renderEditor(); } }, "Delete")),
      el("div", { class: "field", style: "margin-top: 8px" }, el("label", {}, "Explanation (optional)"),
        text(dec.description, v => editDecision(id, x => { if (v.trim()) x.description = v; else delete x.description; }))),
      body);
  });

  const measures = state.detail.measures || [];
  const resultLabel = m => `${m.object} · ${m.measure}`;
  const resultsEditor = d.results.length
    ? el("div", { class: "results-edit" }, ...d.results.map((r, i) => el("div", { class: "row result-row" },
        text(r.label, v => { r.label = v; scheduleCheck(); renderEditor(); }, { "aria-label": "Result name" }),
        el("span", { class: "hint", style: "margin: 0; align-self: center" }, `${r.object} · ${r.measure}`),
        el("select", { onchange: e => { if (e.target.value === "neither") delete r.better; else r.better = e.target.value; scheduleCheck(); renderEditor(); } },
          ...[["higher", "higher is better"], ["lower", "lower is better"], ["neither", "neither"]].map(([v, l]) =>
            el("option", { value: v, selected: (r.better || "neither") === v ? true : null }, l))),
        el("button", { class: "link danger", style: "flex: 0", onclick: () => { d.results.splice(i, 1); scheduleCheck(); renderEditor(); } }, "Remove"))))
    : el("p", { class: "hint", style: "margin: 0" }, "The Simulation page shows the defaults: ",
        (state.exposed?.results || []).map(r => r.label).join(" · ") || "nothing", ".");
  const addResult = el("select", {
    id: "add-result",
    onchange: e => {
      const m = measures[Number(e.target.value)];
      if (!m) return;
      if (!d.results.length) d.results = clone((state.exposed?.results || []).map(r => ({ label: r.label, object: r.object, measure: r.measure, ...(r.better !== "neither" ? { better: r.better } : {}) })));
      const better = m.measure === "throughput" ? "higher" : ["time in system", "blocked", "failed", "full", "lost"].includes(m.measure) ? "lower" : undefined;
      d.results.push({ label: `${m.object} ${m.measure}`, object: m.object, measure: m.measure, ...(better ? { better } : {}) });
      scheduleCheck();
      renderEditor();
    },
  }, el("option", { value: "" }, "+ Add a result…"), ...measures.map((m, i) => el("option", { value: i }, resultLabel(m))));

  content.replaceChildren(...[
    actionsBar(),
    statusBox(),
    problems.length ? el("div", { class: "status", id: "decision-problems" }, "Fix these before publishing:", el("ul", {}, problems.map(p => el("li", {}, p)))) : null,
    el("div", { class: "editor", style: "margin-top: 14px" },
      el("div", { class: "stack" },
        el("div", {},
          el("h3", { class: "section-title", style: "margin-top: 0" }, "Decisions"),
          decisionCards.length ? el("div", { class: "stack" }, ...decisionCards)
            : el("p", { class: "hint" }, "None yet. On the Model tab, click “⚑ Make this a decision” under a number, or make changes and click “Offer as an option…”.")),
        el("div", { class: "card" },
          el("h3", { class: "card-title" }, "Results shown"),
          resultsEditor,
          el("div", { class: "field", style: "margin: 10px 0 0" }, addResult))),
      el("div", { class: "card", id: "simulation-preview" },
        el("h3", { class: "card-title" }, "Preview of the Simulation page"),
        el("p", { class: "hint", style: "margin: 0 0 10px" }, "On the Simulation page, once you publish."),
        previewDecisions(state.exposed?.decisions || []),
        el("p", { class: "hint" }, "They read: ", (state.exposed?.results || []).map(r => r.label).join(" · ") || "nothing"))),
  ].filter(Boolean));
}

function previewDecisions(decisions) {
  if (!decisions.length) return el("p", { class: "hint", style: "margin: 0" }, "Nothing to change: the Simulation page can only run the model as published.");
  const number = n => String(Math.round(n * 1e6) / 1e6);
  const value = (dec, v) => dec.kind === "choice" ? (dec.choices.find(c => c.id === v)?.label ?? v) : `${number(v)}${dec.unit ? " " + dec.unit : ""}`;
  return el("dl", { class: "decisions" }, ...decisions.flatMap(dec => [
    el("dt", {}, dec.label, dec.description ? el("span", { class: "sub" }, dec.description) : null),
    el("dd", {}, el("span", { class: "now" }, "Now: "), dec.current === null ? "?" : value(dec, dec.current),
      el("span", { class: "sub" }, dec.kind === "choice"
        ? `or: ${dec.choices.slice(1).map(c => c.label).join(" · ")}`
        : `can be ${value(dec, dec.min)} to ${value(dec, dec.max)}`)),
  ]));
}

// --- documentation ---------------------------------------------------------------------------

const DOCS_TEMPLATE = `# Purpose
What question the model answers, and for whom.

## Modeling assumptions
- Why an object or rule is modeled the way it is.

## Business rules
- How the real system decides (priorities, routing, schedules).

## Data
Where the numbers come from, and when they were measured.

## Not modeled
- What was left out, and why it doesn't matter for the question.`;

/** The project's README.md: read rendered, edit as Markdown with a live preview. */
function renderDocumentation(content) {
  const docs = state.docs;
  const html = docsChanged() ? docs.html : state.detail.documentation_html || "";
  // While typing, only the preview and the save bar change: the text box keeps its focus.
  const preview = document.getElementById("doc-preview");
  if (docs.editing && preview && document.getElementById("doc-text")) {
    fillMarkdown(preview, html);
    content.querySelector(".actions").replaceWith(actionsBar());
    content.querySelector(".doc-bar").replaceWith(docBar(html));
    return;
  }
  let body;
  if (docs.editing) {
    body = el("div", { class: "doc-edit" },
      el("div", { class: "doc-source" },
        el("textarea", {
          id: "doc-text", "aria-label": "Documentation, in Markdown", spellcheck: "true", placeholder: DOCS_TEMPLATE,
          oninput: e => { docs.text = e.target.value; scheduleCheck(); renderEditor(); },
        }, docs.text),
        el("p", { class: "hint" }, "Markdown: ", el("code", {}, "# Heading"), " ", el("code", {}, "**bold**"), " ",
          el("code", {}, "- list"), " ", el("code", {}, "[link](https://…)"), " ", el("code", {}, "| table |"))),
      fillMarkdown(el("div", { class: "card markdown", id: "doc-preview", "aria-label": "Preview" }), html));
  } else if (html) {
    body = fillMarkdown(el("div", { class: "card markdown", id: "doc-view" }), html);
  } else {
    body = el("div", { class: "card doc-empty", id: "doc-view" },
      el("p", {}, "No documentation yet. Write down what the model can't say for itself, for whoever works on it next:"),
      el("ul", {},
        el("li", {}, "the question it answers;"),
        el("li", {}, "modeling assumptions: why something is modeled the way it is;"),
        el("li", {}, "business rules: priorities, routing, schedules;"),
        el("li", {}, "where the data comes from, and what's left out.")),
      el("button", { class: "primary", id: "doc-write", onclick: editDocs }, pencil(), "Write the documentation"));
  }
  content.replaceChildren(actionsBar(), statusBox(), docBar(html), body);
}

function editDocs() {
  state.docs.editing = true;
  renderEditor();
  document.getElementById("doc-text").focus();
}

/** Edit or Done, and a way back to the saved text. */
function docBar(html) {
  const docs = state.docs;
  return el("div", { class: "doc-bar" },
    docs.editing
      ? el("button", { id: "doc-done", onclick: () => { docs.editing = false; renderEditor(); } }, "Done")
      : html ? el("button", { id: "doc-edit", onclick: editDocs }, pencil(), "Edit") : null,
    docs.editing && docsChanged()
      ? el("button", { class: "link danger", id: "doc-discard", onclick: () => { docs.text = state.detail.documentation || ""; docs.editing = false; renderEditor(); } }, "Discard changes")
      : null,
    el("span", { class: "hint" }, docs.editing
      ? "Saved with the model when you save a version."
      : "README.md in the project, versioned with the model."));
}

/** Puts the server-rendered Markdown (already made safe) in `node`; links open in a new tab. */
function fillMarkdown(node, html) {
  node.innerHTML = html || '<p class="hint">Nothing to show yet.</p>';
  for (const a of node.querySelectorAll("a[href]")) {
    if (a.getAttribute("href").startsWith("#")) continue;
    a.target = "_blank";
    a.rel = "noopener noreferrer";
  }
  return node;
}

function pencil() {
  return svg("svg", { class: "icon", viewBox: "0 0 24 24", "aria-hidden": "true" },
    svg("path", { d: "M4 20h4L19 9l-4-4L4 16v4z" }), svg("path", { d: "M13.5 6.5l4 4" }));
}

// --- saving, publishing and running --------------------------------------------------------

async function saveVersion() {
  const id = encodeURIComponent(state.project);
  const { ok, data } = await api("POST", `projects/${id}/save`, { ...pending(), message: state.saveMessage });
  if (!ok) {
    state.errors = data.errors || state.errors;
    state.message = { kind: "error", text: data.error || "Couldn't save." };
    return renderEditor();
  }
  state.message = { kind: "ok", text: `Saved: ${data.saved.message}. Publish it to make it available on the Simulation page.` };
  await openProject(state.project, true);
}

async function publishVersion() {
  const id = encodeURIComponent(state.project);
  const { ok, data } = await api("POST", `projects/${id}/publish`, { message: state.saveMessage });
  state.message = ok
    ? { kind: "ok", text: `Published ${data.published.name}: it's now on the Simulation page.` }
    : { kind: "error", text: data.error || "Couldn't publish.", list: (data.errors || []).map(e => e.message) };
  await openProject(state.project, true);
}

async function startRun() {
  const body = { project: state.project, changes: state.changes };
  if (state.layoutChanged) body.layout = layoutTree();
  const { ok, data } = await api("POST", "runs", body);
  if (!ok) {
    state.errors = data.errors || state.errors;
    state.message = { kind: "error", text: data.error || "The run couldn't start." };
    return renderEditor();
  }
  loadRuns();
  const run = await waitForRun(data, r => {
    state.message = { kind: "ok", text: progressText(r), progress: r.progress.done / Math.max(1, r.progress.total) };
    renderEditor();
  });
  state.message = run.status === "failed" ? { kind: "error", text: "The run failed.", list: [run.error] } : null;
  loadRuns();
  if (run.status === "done") {
    state.view = "report";   // a new run opens on its report
    showRun(run);
  } else renderEditor();
}

function showRun(run) {
  stopAnimation();
  state.run = run;
  state.tab = "results";
  render();
  loadRuns();
}

// --- start -----------------------------------------------------------------------------------

state.project = pageProject();
renderSide();
if (state.project) {
  openProject(state.project);
  loadRuns();
}
setInterval(() => { if (state.runs.some(r => r.status === "queued" || r.status === "running")) loadRuns(); }, 2000);
