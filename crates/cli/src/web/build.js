"use strict";
// The engineer's builder (/build): edit a project's model, choose what
// managers may change and read, save versions, publish them for managers, run
// and inspect results. Shared code: common.js.

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
  tab: "model",      // "model" | "managers" | "results"
  draft: null,       // project.yaml's { decisions, results } with unsaved edits
  exposed: null,     // the draft as checked: { decisions, results, decision_problems }
  offer: null,       // the "offer as an option" form: { decision, newLabel, optionLabel, currentLabel }
  written: new Set(), // paths whose defaults were written into the model for a decision
  run: null,
  view: "report",
  message: null,     // { kind: "ok" | "error", text, list?, progress? }
  saveMessage: "",
};

// --- change paths ---------------------------------------------------------------

/** The model with the current changes applied. */
function currentTree() {
  const tree = clone(state.baseTree);
  for (const [path, value] of Object.entries(state.changes)) setAt(tree, path, clone(value));
  return tree;
}


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

// --- sidebar -------------------------------------------------------------------------

async function loadProjects() {
  const { data } = await api("GET", "projects");
  state.projects = data.projects || [];
  document.getElementById("projects").replaceChildren(...state.projects.map(p => el("button", {
    class: p.id === state.project ? "current" : null,
    onclick: () => openProject(p.id),
    title: p.description || "",
  }, p.name, el("span", { class: "sub" }, p.published ? `published ${p.published.name}` : "not published yet"))));
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

// --- opening a project -------------------------------------------------------------------

async function openProject(id, keepMessage = false) {
  const { ok, data } = await api("GET", `projects/${encodeURIComponent(id)}`);
  if (!ok) return;
  Object.assign(state, {
    project: id,
    detail: data,
    baseTree: data.tree,
    baseLayout: data.layout,
    changes: {},
    errors: (data.errors || []).map(e => ({ change: null, message: `${e.file}${e.line ? ":" + e.line : ""}: ${e.message}` })),
    selected: null,
    tab: keepMessage && state.tab === "managers" ? "managers" : "model",
    message: keepMessage ? state.message : null,
    saveMessage: "",
    draft: savedDraft(data),
    written: new Set(),
    exposed: { decisions: data.decisions, results: data.results, decision_problems: data.decision_problems },
    offer: null,
  });
  if (!data.tree) {
    state.message = { kind: "error", text: "This model can't be read.", list: state.errors.map(e => e.message) };
  }
  loadProjects();
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
  return body;
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
  if (!state.project && !state.run) {
    main.replaceChildren(el("p", { class: "empty" }, "Pick a project on the left."));
    return;
  }
  const d = state.detail;
  const results = state.tab === "results" && state.run;
  main.replaceChildren(
    el("div", { class: "head" },
      el("div", { class: "titles" },
        el("h2", {}, results ? state.run.label : d?.name || state.project),
        el("p", {}, results ? `Run ${state.run.id}` : d?.description || "")),
    ),
    el("div", { class: "tabs" },
      state.project ? el("button", { class: state.tab === "model" ? "current" : null, onclick: () => { state.tab = "model"; render(); } }, "Model") : null,
      state.project ? el("button", { id: "tab-managers", class: state.tab === "managers" ? "current" : null, onclick: () => { state.tab = "managers"; render(); } },
        "For managers", (state.exposed?.decision_problems || []).length ? el("span", { class: "badge bad", style: "margin-left: 6px" }, "!") : null) : null,
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
  if (!content || (state.tab !== "model" && state.tab !== "managers")) return;
  if (!state.baseTree) {
    content.replaceChildren(statusBox());
    return;
  }
  if (state.tab === "managers") return renderManagers(content);
  const tree = currentTree();
  const general = state.errors.filter(e => !e.change);
  content.replaceChildren(...[
    actionsBar(),
    state.offer ? offerForm() : null,
    statusBox(general),
    el("div", { class: "editor", style: "margin-top: 14px" },
      el("div", { class: "stack" },
        el("div", { class: "card diagram" }, diagram(tree),
          el("p", { class: "hint" }, "Click an object or conveyor to edit it; click empty space for the experiment settings.")),
        versionsCard()),
      el("div", { class: "card panel" }, panel(tree))),
  ].filter(Boolean));
}

/** Save, publish, run: the same on the Model and For managers tabs. */
function actionsBar() {
  const modelChanges = Object.keys(state.changes).length;
  const changed = modelChanges + (draftChanged() ? 1 : 0);
  const history = state.detail.history || [];
  const lastSaved = history[0];
  const problems = state.exposed?.decision_problems || [];
  const canPublish = changed === 0 && lastSaved && !lastSaved.published && state.detail.valid && !problems.length;
  const describe = () => {
    if (!changed) return "all changes saved";
    const parts = [];
    if (modelChanges) parts.push(`${modelChanges} unsaved change${modelChanges === 1 ? "" : "s"}`);
    if (draftChanged()) parts.push("unsaved edits for managers");
    return parts.join(", ");
  };
  return el("div", { class: "actions" },
    el("input", {
      id: "save-message", placeholder: "What changed? (e.g. Faster mill B)", value: state.saveMessage,
      oninput: e => { state.saveMessage = e.target.value; }, "aria-label": "Version message",
    }),
    el("button", { id: "save", onclick: saveVersion, disabled: changed === 0 }, "Save version"),
    el("button", { id: "publish", onclick: publishVersion, disabled: !canPublish,
      title: canPublish ? "Make the last saved version available to managers"
        : problems.length ? "Fix the problems on the For managers tab first"
        : "Save your changes first; already-published versions can't be published again" }, "Publish"),
    el("button", { class: "primary", id: "run", onclick: startRun }, "▶ Run"),
    state.tab === "model" && Object.keys(state.changes).some(p => !state.written.has(p)) && !state.offer
      ? el("button", { id: "offer", onclick: startOffer, title: "Instead of saving these changes into the model, let managers choose them" }, "Offer as an option…")
      : null,
    el("span", { class: "hint", id: "pending", style: "margin: 0" }, describe()));
}

function versionsCard() {
  const history = state.detail.history || [];
  return el("div", { class: "card versions" },
    el("h3", { style: "margin: 0 0 8px; font-size: 15px" }, "Versions"),
    el("p", { class: "hint", style: "margin: 0 0 8px" },
      "Saving keeps a version of the model; publishing makes the last saved version available to managers."),
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
    exposeButton(label, path, value, placeholder),
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

// --- what managers may change: the For managers tab ------------------------------------------

/** A number as the model writes it: `2`, or `8 m`. */
function parseNumber(value) {
  if (typeof value === "number") return { n: value, unit: null };
  const m = typeof value === "string" && value.trim().match(/^(-?\d+(?:\.\d+)?)\s+(\S+)$/);
  return m ? { n: Number(m[1]), unit: m[2] } : null;
}

const numberDecisionAt = path => Object.entries(state.draft.decisions).find(([, d]) => d.path === path);

/** "⚑ Let managers change this" under number fields. */
function exposeButton(label, path, value, placeholder) {
  if (path.startsWith("experiment.") || !state.draft) return null;
  const existing = numberDecisionAt(path);
  if (existing) {
    return el("button", { class: "expose on", onclick: () => { state.tab = "managers"; render(); },
      title: "Managers can change this; set its limits on the For managers tab" }, `⚑ Managers can change this (${existing[1].label})`);
  }
  const parsed = parseNumber(value ?? (placeholder ? parseFloat(placeholder) : undefined));
  if (!parsed || Number.isNaN(parsed.n)) return null;
  return el("button", { class: "expose", "data-expose": path, onclick: () => exposeNumber(label, path, value, parsed) }, "⚑ Let managers change this");
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
  state.tab = "managers";
  state.message = { kind: "ok", text: `Managers can now change “${decision.label}”. Check its name and limits, then save a version.` };
  scheduleCheck();
  render();
}

function startOffer() {
  const choices = Object.entries(state.draft.decisions).filter(([, d]) => d.choices);
  state.offer = { decision: choices.length ? choices[0][0] : "", newLabel: "", optionLabel: state.saveMessage || "", currentLabel: "As it is now" };
  renderEditor();
}

/** Turn the pending model changes into an option managers can pick. */
function offerForm() {
  const o = state.offer;
  const choices = Object.entries(state.draft.decisions).filter(([, d]) => d.choices);
  const isNew = o.decision === "";
  const input = (key, placeholder, id) => el("input", { id, value: o[key], placeholder, oninput: e => { o[key] = e.target.value; } });
  return el("div", { class: "card offer", id: "offer-form" },
    el("div", { class: "group-title" }, "Offer these changes to managers as an option"),
    el("p", { class: "hint", style: "margin: 0 0 10px" },
      "These pending changes won't go into the model. Instead, managers can pick them in a scenario; the model stays as it is."),
    el("div", { class: "field" }, el("label", {}, "Decision"),
      el("select", { id: "offer-decision", onchange: e => { o.decision = e.target.value; renderEditor(); } },
        ...choices.map(([id, d]) => el("option", { value: id, selected: o.decision === id ? true : null }, d.label)),
        el("option", { value: "", selected: isNew ? true : null }, "A new decision…"))),
    isNew ? el("div", { class: "field" }, el("label", {}, "What managers decide (e.g. Repairs on mill B)"), input("newLabel", "Decision", "offer-new-label")) : null,
    isNew ? el("div", { class: "field" }, el("label", {}, "The current setup is called"), input("currentLabel", "As it is now", "offer-current-label")) : null,
    el("div", { class: "field" }, el("label", {}, "This option is called (e.g. Faster crew)"), input("optionLabel", "Option", "offer-option-label")),
    el("div", { class: "actions" },
      el("button", { class: "primary", id: "offer-confirm", onclick: confirmOffer }, "Offer to managers"),
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
  state.tab = "managers";
  state.message = { kind: "ok", text: `“${optionLabel}” is now an option of “${decision.label}”. The model is back as it was. Save a version to keep this.` };
  scheduleCheck();
  render();
}

function renderManagers(content) {
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
      el("div", { class: "field", style: "margin-top: 8px" }, el("label", {}, "Explanation for managers (optional)"),
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
    : el("p", { class: "hint", style: "margin: 0" }, "Managers read the defaults: ",
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
          el("h3", { class: "section-title", style: "margin-top: 0" }, "Decisions managers can make"),
          decisionCards.length ? el("div", { class: "stack" }, ...decisionCards)
            : el("p", { class: "hint" }, "None yet. On the Model tab, click “⚑ Let managers change this” under a number, or make changes and click “Offer as an option…”.")),
        el("div", { class: "card" },
          el("h3", { class: "card-title" }, "What managers read"),
          resultsEditor,
          el("div", { class: "field", style: "margin: 10px 0 0" }, addResult))),
      el("div", { class: "card", id: "manager-preview" },
        el("h3", { class: "card-title" }, "What managers will see"),
        el("p", { class: "hint", style: "margin: 0 0 10px" }, "On the Decide page, once you publish."),
        previewDecisions(state.exposed?.decisions || []),
        el("p", { class: "hint" }, "They read: ", (state.exposed?.results || []).map(r => r.label).join(" · ") || "nothing"))),
  ].filter(Boolean));
}

function previewDecisions(decisions) {
  if (!decisions.length) return el("p", { class: "hint", style: "margin: 0" }, "Nothing to change: managers can only run the model as published.");
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
  state.message = { kind: "ok", text: `Saved: ${data.saved.message}. Publish it when managers should see it.` };
  await openProject(state.project, true);
}

async function publishVersion() {
  const id = encodeURIComponent(state.project);
  const { ok, data } = await api("POST", `projects/${id}/publish`, { message: state.saveMessage });
  state.message = ok
    ? { kind: "ok", text: `Published ${data.published.name}: managers now see this version.` }
    : { kind: "error", text: data.error || "Couldn't publish.", list: (data.errors || []).map(e => e.message) };
  await openProject(state.project, true);
}

async function startRun() {
  const body = { project: state.project, changes: state.changes };
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
  if (run.status === "done") showRun(run);
  else renderEditor();
}

function showRun(run) {
  stopAnimation();
  state.run = run;
  state.tab = "results";
  render();
  loadRuns();
}

// --- start -----------------------------------------------------------------------------------

loadProjects();
loadRuns();
setInterval(() => { if (state.runs.some(r => r.status === "queued" || r.status === "running")) loadRuns(); }, 2000);
