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
  selected: null,    // { section, name } or null (experiment)
  tab: "model",      // "model" | "decisions" | "results"
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
  list.replaceChildren(...state.runs.slice(0, 15).map(r => el("button", {
    class: state.run && state.run.id === r.id ? "current" : null,
    onclick: () => showRun(r),
  },
  r.label, " ",
  r.status === "done" ? null : el("span", { class: r.status === "failed" ? "badge bad" : "badge" }, r.status),
  el("span", { class: "sub" }, r.id))));
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
    selected: null,
    tab: keepMessage && state.tab === "decisions" ? "decisions" : "model",
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

/** What to send to check and save: model changes, plus the draft if edited. */
function pending() {
  const body = { changes: state.changes };
  if (draftChanged()) Object.assign(body, state.draft);
  if (state.layoutChanged) body.layout = layoutTree();
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
    const model = state.baseLayout?.objects?.[name]?.model;
    objects[name] = model ? { position: [r(x), 0, r(z)], model } : { position: [r(x), 0, r(z)] };
  }
  const conveyors = {};
  for (const c of graph.conveyors) conveyors[c.name] = { path: state.layout.paths[c.name].map(([x, z]) => [r(x), 0, r(z)]) };
  return { units: "m", objects, conveyors };
}

const scheduleCheck = debounce(async () => {
  if (!state.project) return;
  const { data } = await api("POST", `projects/${encodeURIComponent(state.project)}/check`, pending());
  state.errors = data.errors || [];
  state.exposed = { decisions: data.decisions || [], results: data.results || [], decision_problems: data.decision_problems || [] };
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
        el("p", {}, results ? `Run ${state.run.id}` : d?.description || "")),
    ),
    el("div", { class: "tabs" },
      el("button", { class: state.tab === "model" ? "current" : null, onclick: () => { state.tab = "model"; render(); } }, "Model"),
      el("button", { id: "tab-decisions", class: state.tab === "decisions" ? "current" : null, onclick: () => { state.tab = "decisions"; render(); } },
        "Decisions", (state.exposed?.decision_problems || []).length ? el("span", { class: "badge bad", style: "margin-left: 6px" }, "!") : null),
      state.run ? el("button", { class: state.tab === "results" ? "current" : null, onclick: () => { state.tab = "results"; render(); } }, "Results") : null),
    el("div", { id: "content" }));
  if (results) {
    runResults(document.getElementById("content"), state.run, state.view, v => { state.view = v; render(); });
  } else {
    renderEditor();
  }
}

function renderEditor() {
  const content = document.getElementById("content");
  if (!content || (state.tab !== "model" && state.tab !== "decisions")) return;
  if (!state.baseTree) {
    content.replaceChildren(statusBox());
    return;
  }
  if (state.tab === "decisions") return renderDecisions(content);
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
            ? (state.connect.from ? `Now click where parts go from ${state.connect.from}.` : "Click the object parts come from.")
            : "Click an object or conveyor to edit it, drag objects to move them; click empty space for the model settings.")),
        versionsCard()),
      el("div", { class: "card panel" }, panel(tree))),
  ].filter(Boolean));
}

/** Save, publish, run: the same on the Model and Decisions tabs. */
function actionsBar() {
  const modelChanges = Object.keys(state.changes).length;
  const changed = modelChanges + (draftChanged() ? 1 : 0) + (state.layoutChanged ? 1 : 0);
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
    return parts.join(", ");
  };
  return el("div", { class: "actions" },
    el("input", {
      id: "save-message", placeholder: "What changed? (e.g. Faster mill B)", value: state.saveMessage,
      oninput: e => { state.saveMessage = e.target.value; }, "aria-label": "Version message",
    }),
    el("button", { id: "save", onclick: saveVersion, disabled: changed === 0 }, "Save version"),
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
  for (const c of graph.conveyors) if (!L.paths[c.name]) L.paths[c.name] = route(c.from, c.to);
  for (const name of Object.keys(L.positions)) if (!names.includes(name)) delete L.positions[name];
  for (const name of Object.keys(L.paths)) if (!graph.conveyors.some(c => c.name === name)) delete L.paths[name];
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
const allNames = tree => ["sources", "servers", "sinks", "conveyors", "resources"].flatMap(s => Object.keys(tree[s] || {}));

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
  const attached = graphOf(tree).conveyors.filter(c => c.from === name || c.to === name);
  const follows = Object.fromEntries(attached.map(c => {
    const length = meters(tree.conveyors[c.name].length);
    return [c.name, length !== null && Math.abs(length - pathLength(state.layout.paths[c.name])) < 0.06];
  }));
  state.layout.positions[name] = [x, z];
  for (const c of attached) {
    state.layout.paths[c.name] = route(c.from, c.to);
    if (follows[c.name]) setChange(`conveyors.${c.name}.length`, lengthText(pathLength(state.layout.paths[c.name])));
  }
  state.layoutChanged = true;
}

function addObject(section) {
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
    state.layout.positions[name] = [xs.length ? Math.max(...xs) + 6 : 0, zs.length ? Math.max(...zs) + 4 : 0];
    state.layoutChanged = true;
    state.selected = { section, name };
    state.message = { kind: "ok", text: `Added ${name}. Drag it into place, then use “Connect” to link it with conveyors.` };
  });
}

function connectObjects(from, to) {
  const tree = currentTree();
  const fromSection = sectionOf(tree, from), toSection = sectionOf(tree, to);
  if (from === to || fromSection === "sinks" || toSection === "sources") {
    state.message = { kind: "error", text: "Conveyors go from a source or server to a server or sink." };
    return renderEditor();
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

/** Removes a conveyor, and the routing or merging rule its ends no longer need. */
function deleteConveyor(tree, name) {
  const c = tree.conveyors[name];
  setChange(`conveyors.${name}`, null);
  delete state.layout.paths[name];
  const others = Object.entries(tree.conveyors).filter(([n]) => n !== name);
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
  if (!sel) return;
  batch(() => {
    if (sel.section === "conveyors") {
      deleteConveyor(currentTree(), sel.name);
    } else {
      for (const c of graphOf(currentTree()).conveyors) {
        if (c.from === sel.name || c.to === sel.name) deleteConveyor(currentTree(), c.name);
      }
      setChange(`${sel.section}.${sel.name}`, null);
      delete state.layout.positions[sel.name];
    }
    state.layoutChanged = true;
    state.message = { kind: "ok", text: `Deleted ${sel.name}.` };
    state.selected = null;
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
    if (section === "conveyors") {
      const c = tree.conveyors[oldName];
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
      renameInDraft(`conveyors.${oldName}`, `conveyors.${newName}`, oldName, newName);
    } else {
      for (const [name, c] of Object.entries(tree.conveyors || {})) {
        if (c.from === oldName) setChange(`conveyors.${name}.from`, newName);
        if (c.to === oldName) setChange(`conveyors.${name}.to`, newName);
      }
      state.layout.positions[newName] = state.layout.positions[oldName];
      delete state.layout.positions[oldName];
      renameInDraft(`${section}.${oldName}`, `${section}.${newName}`, oldName, newName);
    }
    state.layoutChanged = true;
    state.selected = { section, name: newName };
  });
}

function diagramToolbar() {
  const tool = (label, onclick, attrs = {}) => el("button", { onclick, ...attrs }, label);
  return el("div", { class: "toolbar" },
    tool("+ Source", () => addObject("sources"), { id: "add-source", title: "Where parts come from" }),
    tool("+ Server", () => addObject("servers"), { id: "add-server", title: "A machine or station that works on parts" }),
    tool("+ Sink", () => addObject("sinks"), { id: "add-sink", title: "Where parts leave" }),
    tool(state.connect ? "Connecting… (Esc)" : "Connect", () => { state.connect = state.connect ? null : { from: null }; renderEditor(); },
      { id: "connect", class: state.connect ? "primary" : null, title: "Click an object, then the next one, to link them with a conveyor" }),
    tool("Delete", deleteSelected, { id: "delete", disabled: state.selected ? null : true, title: "Delete the selected object or conveyor" }),
    el("span", { style: "flex: 1" }),
    tool("↶ Undo", undo, { id: "undo", disabled: state.undo.length ? null : true, title: "Undo (Ctrl+Z)" }),
    tool("↷ Redo", redo, { id: "redo", disabled: state.redo.length ? null : true, title: "Redo (Ctrl+Shift+Z)" }));
}

let drag = null;

function diagram(tree) {
  const graph = graphOf(tree);
  fillLayout(graph);
  const layout = state.layout;
  const b = drag?.bounds || boundsOf(layout);
  const select = (section, name) => e => {
    e.stopPropagation();
    if (drag?.moved) return;
    if (state.connect && section !== "conveyors") {
      if (!state.connect.from) { state.connect.from = name; return renderEditor(); }
      const from = state.connect.from;
      state.connect = null;
      return connectObjects(from, name);
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
    onclick: () => { if (drag?.moved) return; state.selected = null; state.connect = null; renderEditor(); },
  });
  for (const c of graph.conveyors) {
    const path = layout.paths[c.name];
    const points = path.map(p => p.join(",")).join(" ");
    const cls = invalid("conveyors", c.name) ? "invalid" : isSelected("conveyors", c.name) ? "selected" : "";
    root.append(svg("polyline", { points, fill: "none", stroke: "var(--belt)", "stroke-width": 0.5, "stroke-linecap": "round", "stroke-linejoin": "round" }));
    if (cls) root.append(svg("polyline", { points, fill: "none", class: cls, "vector-effect": "non-scaling-stroke", "stroke-linejoin": "round" }));
    const last = path.slice(-2);
    const angle = Math.atan2(last[1][1] - last[0][1], last[1][0] - last[0][0]) * 180 / Math.PI;
    root.append(svg("polygon", { points: "0,-0.35 0.5,0 0,0.35", fill: "var(--muted)", transform: `translate(${last[1][0]},${last[1][1]}) rotate(${angle})` }));
    root.append(svg("polyline", { points, fill: "none", stroke: "transparent", "stroke-width": 1.4, class: "hit", onclick: select("conveyors", c.name) },
      svg("title", {}, `conveyor ${c.name}: ${c.from} → ${c.to}`)));
    const i = Math.max(0, Math.floor((path.length - 1) / 2));
    const [mid, next] = [path[i], path[i + 1]];
    root.append(svg("text", { x: (mid[0] + next[0]) / 2, y: (mid[1] + next[1]) / 2 - 0.45, "text-anchor": "middle", "font-size": 0.6 },
      c.name + (changedObj("conveyors", c.name) ? " •" : "")));
  }
  const objects = [["sources", graph.sources], ["servers", graph.servers], ["sinks", graph.sinks]];
  for (const [section, names] of objects) {
    for (const name of names) {
      const [x, z] = layout.positions[name];
      const s = OBJECT_SIZE;
      const fill = section === "servers" ? "var(--busy)" : "var(--object)";
      const from = state.connect?.from === name;
      const cls = from ? "selected" : invalid(section, name) ? "invalid" : isSelected(section, name) ? "selected" : "";
      root.append(svg("rect", {
        x: x - s / 2, y: z - s / 2, width: s, height: s, rx: section === "servers" ? 0.15 : 0.6,
        fill, class: `hit object ${cls}`, "vector-effect": "non-scaling-stroke", "data-object": name,
        onclick: select(section, name),
        onpointerdown: e => startDrag(e, root, name),
      }, svg("title", {}, `${section.slice(0, -1)} ${name}`)));
      root.append(svg("text", { x, y: z - s / 2 - 0.35, "text-anchor": "middle", "font-size": 0.7 },
        name + (changedObj(section, name) ? " •" : "")));
    }
  }
  return root;
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
  drag = { name, moved: false, before: snapshot(), bounds: boundsOf(state.layout) };
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
  const sel = state.selected;
  if (!sel) return experimentPanel(tree);
  const node = tree[sel.section]?.[sel.name];
  if (!node) { state.selected = null; return experimentPanel(tree); }
  const p = `${sel.section}.${sel.name}`;
  const kind = { sources: "Source", servers: "Server", conveyors: "Conveyor", sinks: "Sink" }[sel.section];
  const header = [el("h3", {}, sel.name), el("div", { class: "kind" }, kind),
    el("div", { class: "field" },
      el("label", { for: "f-name" }, "Name"),
      el("div", { class: "row" },
        el("input", { id: "f-name", value: sel.name, onchange: e => rename(sel.section, sel.name, e.target.value.trim()) }),
        el("button", { style: "flex: 0", onclick: deleteSelected, title: `Delete ${sel.name}` }, "Delete")))];
  const graph = graphOf(tree);
  const outputs = graph.conveyors.filter(c => c.from === sel.name).map(c => c.name);
  const inputs = graph.conveyors.filter(c => c.to === sel.name).map(c => c.name);
  switch (sel.section) {
    case "sources":
      return el("div", {}, header,
        createsField(tree, p),
        distField("Time between arrivals", `${p}.interarrival`),
        field("When the conveyor is full", `${p}.when_blocked`, "select", ["wait", "drop"]),
        scheduleField(tree, `${p}.schedule`, "Parts arrive"),
        outputRule(tree, p, outputs));
    case "servers":
      return el("div", {}, header,
        kindField(tree, p),
        field("Capacity (parts at once)", `${p}.capacity`, "int", [], node.batch ? `${node.batch} (one batch)` : "1 (default)"),
        processTime(tree, p),
        needsField(tree, p),
        scheduleField(tree, `${p}.schedule`, "Works"),
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

function experimentPanel(tree) {
  return el("div", {}, el("h3", {}, "Model settings"), el("div", { class: "kind" }, "Apply to the whole model"),
    el("div", { class: "group-title" }, "Experiment"),
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
    partTypesPanel(tree),
    resourcesPanel(tree),
    schedulesPanel(tree));
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
function scheduleUsers(tree) {
  return ["sources", "servers", "resources"].flatMap(section =>
    Object.entries(tree[section] || {}).filter(([, o]) => o.schedule).map(([n, o]) => [`${section}.${n}.schedule`, o.schedule]));
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
    el("p", { class: "hint", style: "margin: 0 0 8px" }, "Operators, tools or fixtures that servers share. A server holds what it needs while it processes a part."),
    ...resources.map(r => el("div", { class: "resource" },
      el("div", { class: "row", style: "align-items: center" },
        el("input", { value: r, "aria-label": `resource ${r}`, "data-resource": r, onchange: e => renameResource(r, e.target.value.trim()) }),
        el("button", { style: "flex: 0", onclick: () => deleteResource(r), title: `Remove ${r}` }, "Remove")),
      field("Units available", `resources.${r}.capacity`, "int"),
      scheduleField(tree, `resources.${r}.schedule`, "Available"))),
    el("button", { id: "add-resource", onclick: () => setChange(`resources.${freeName(tree, "operator")}`, { capacity: 1 }) }, "+ Resource"));
}

/** Removes a resource and every server's need for it. */
function deleteResource(name) {
  const tree = currentTree();
  batch(() => {
    for (const [server, srv] of Object.entries(tree.servers || {})) {
      if (srv.needs && name in srv.needs) {
        const needs = { ...srv.needs }; delete needs[name];
        setChange(`servers.${server}.needs`, Object.keys(needs).length ? needs : null);
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
      if (srv.needs && oldName in srv.needs) {
        setChange(`servers.${server}.needs`, Object.fromEntries(Object.entries(srv.needs).map(([k, v]) => [k === oldName ? newName : k, v])));
      }
    }
    renameInDraft(`resources.${oldName}`, `resources.${newName}`, oldName, newName);
  });
}

/** The resources a server holds while processing. */
function needsField(tree, p) {
  const resources = Object.keys(tree.resources || {});
  if (!resources.length) return null;
  const path = `${p}.needs`;
  const needs = getAt(tree, path) || {};
  const error = errorFor(path);
  return el("div", { class: `group field${isChanged(path) ? " changed" : ""}${error ? " error" : ""}` },
    el("div", { class: "group-title" }, "Needs while processing",
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path) }, "reset") : null),
    ...resources.map(r => el("div", { class: "row", style: "margin-bottom: 6px; align-items: center" },
      el("span", {}, r),
      el("input", {
        value: needs[r] ?? "", inputmode: "numeric", placeholder: "0", "aria-label": `units of ${r}`, "data-need": r,
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
    el("div", { class: "group-title" }, "Part types"),
    el("p", { class: "hint", style: "margin: 0 0 8px" }, "Sources create these. Process times and routing can differ by type."),
    ...types.map(t => el("div", { class: "row", style: "margin-bottom: 6px; align-items: center" },
      el("input", { value: t, "aria-label": `part type ${t}`, "data-type": t, onchange: e => renamePartType(t, e.target.value.trim()) }),
      el("button", { style: "flex: 0", disabled: types.length < 2 ? true : null, onclick: () => setChange(`entities.${t}`, null),
        title: types.length < 2 ? "A model needs at least one part type" : `Remove ${t}` }, "Remove"))),
    el("button", { id: "add-part-type", onclick: () => setChange(`entities.${freeTypeName(types)}`, {}) }, "+ Part type"));
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
    for (const [name, src] of Object.entries(tree.sources || {})) {
      if (src.creates === oldName) setChange(`sources.${name}.creates`, newName);
      else if (src.creates && typeof src.creates === "object" && oldName in src.creates) setChange(`sources.${name}.creates`, renameKey(src.creates));
      if (src.output?.routes && oldName in src.output.routes) setChange(`sources.${name}.output`, { ...src.output, routes: renameKey(src.output.routes) });
    }
    for (const [name, srv] of Object.entries(tree.servers || {})) {
      if (srv.process_time && typeof srv.process_time === "object" && oldName in srv.process_time) setChange(`servers.${name}.process_time`, renameKey(srv.process_time));
      if (srv.output?.routes && oldName in srv.output.routes) setChange(`servers.${name}.output`, { ...srv.output, routes: renameKey(srv.output.routes) });
    }
  });
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
      types.length > 1 ? el("button", { class: "link", id: "mix", style: "padding: 2px 0", onclick: () => setChange(path, Object.fromEntries(types.map(t => [t, +(1 / types.length).toFixed(4)]))) }, "A mix of part types…") : null,
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
      el("button", { class: "link", style: "padding: 2px 0", onclick: () => setChange(path, Object.keys(value)[0] || types[0]) }, "Only one part type"),
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
    [["single", "One part at a time"], ["batch", "In batches (an oven, a washer)"], ["assembly", "Assembles kits into a new part"]]
      .map(([v, l]) => el("option", { value: v, selected: v === kind ? true : null }, l)));
  const parts = [el("div", { class: "field" }, el("label", { for: "f-kind" }, "Works on"), select)];
  if (kind === "batch") {
    parts.push(field("Batch size (parts processed together)", `${p}.batch`, "int"));
  }
  if (kind === "assembly") {
    const path = `${p}.assembles`;
    const error = errorFor(path);
    parts.push(el("div", { class: `group field${error ? " error" : ""}` },
      el("div", { class: "group-title" }, "Kit: parts it takes"),
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
      field("Makes", `${p}.makes`, "select", types));
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
      el("div", { class: "group-title" }, "Process time by part type"),
      ...Object.keys(value).map(t => distField(t, `${path}.${t}`)),
      ...types.filter(t => !(t in value)).map(t => el("button", { class: "link", onclick: () => setChange(`${path}.${t}`, Object.values(value)[0]) }, `+ A time for ${t}`)),
      el("button", { class: "link", style: "display: block; padding: 2px 0", onclick: () => setChange(path, Object.values(value)[0]) }, "The same for every part type"));
  }
  return el("div", {},
    distField("Process time", path),
    types.length > 1 && !getAt(tree, p).batch && !getAt(tree, p).assembles ? el("button", { class: "link", id: "per-type", style: "padding: 0 0 10px", onclick: () => setChange(path, Object.fromEntries(types.map(t => [t, value]))) }, "Different times per part type…") : null);
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
          : { rule: r });
    },
  }, el("option", { value: "", selected: !value.rule, disabled: true }, "choose a rule"),
  OUTPUT_RULES.map(r => el("option", { value: r, selected: value.rule === r }, r.replaceAll("_", " "))));
  const routes = value.rule === "by_type" ? typesOf(tree).map(t => el("div", { class: "row field", style: "margin-bottom: 6px; align-items: center" },
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
    el("label", { for: `f-${path}-rule` }, "Sends parts to " + outputs.join(", "),
      isChanged(path) ? el("button", { class: "reset", onclick: () => resetChange(path) }, "reset") : null),
    rule, ...routes, ...weights,
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
      distField("Uptime between failures", `${path}.${i}.uptime`),
      distField("Downtime (repair)", `${path}.${i}.downtime`),
      el("div", { class: "row" },
        field("Uptime counts", `${path}.${i}.counts`, "select", ["busy_time", "clock_time"]),
        field("Interrupted part", `${path}.${i}.on_failure`, "select", ["resume", "restart"])))),
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
if (state.project) {
  openProject(state.project);
  loadRuns();
}
setInterval(() => { if (state.runs.some(r => r.status === "queued" || r.status === "running")) loadRuns(); }, 2000);
