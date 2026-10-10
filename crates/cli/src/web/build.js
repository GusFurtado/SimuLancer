"use strict";
// The Build page (/build): edit a project's model, save versions, publish
// them to the Decide page, run and inspect results. Shared code: common.js.

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
  tab: "model",      // "model" | "results"
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
    tab: "model",
    message: keepMessage ? state.message : null,
    saveMessage: "",
  });
  if (!data.tree) {
    state.message = { kind: "error", text: "This model can't be read.", list: state.errors.map(e => e.message) };
  }
  loadProjects();
  render();
}

const scheduleCheck = debounce(async () => {
  if (!state.project) return;
  const { data } = await api("POST", `projects/${encodeURIComponent(state.project)}/check`, { changes: state.changes });
  state.errors = data.errors || [];
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
  if (!content || state.tab !== "model") return;
  if (!state.baseTree) {
    content.replaceChildren(statusBox());
    return;
  }
  const tree = currentTree();
  const changed = Object.keys(state.changes).length;
  const general = state.errors.filter(e => !e.change);
  const history = state.detail.history || [];
  const lastSaved = history[0];
  const canPublish = changed === 0 && lastSaved && !lastSaved.published && state.detail.valid && !state.detail.decision_problems.length;
  content.replaceChildren(
    el("div", { class: "actions" },
      el("input", {
        id: "save-message", placeholder: "What changed? (e.g. Faster mill B)", value: state.saveMessage,
        oninput: e => { state.saveMessage = e.target.value; }, "aria-label": "Version message",
      }),
      el("button", { id: "save", onclick: saveVersion, disabled: changed === 0 }, "Save version"),
      el("button", { id: "publish", onclick: publishVersion, disabled: !canPublish,
        title: canPublish ? "Make the last saved version available on the Decide page" : "Save your changes first; already-published versions can't be published again" }, "Publish"),
      el("button", { class: "primary", id: "run", onclick: startRun }, "▶ Run"),
      el("span", { class: "hint", style: "margin: 0" },
        changed ? `${changed} unsaved change${changed === 1 ? "" : "s"}` : "all changes saved")),
    statusBox(general),
    el("div", { class: "editor", style: "margin-top: 14px" },
      el("div", { class: "stack" },
        el("div", { class: "card diagram" }, diagram(tree),
          el("p", { class: "hint" }, "Click an object or conveyor to edit it; click empty space for the experiment settings.")),
        managersCard(),
        versionsCard()),
      el("div", { class: "card panel" }, panel(tree))));
}

function versionsCard() {
  const history = state.detail.history || [];
  return el("div", { class: "card versions" },
    el("h3", { style: "margin: 0 0 8px; font-size: 15px" }, "Versions"),
    el("p", { class: "hint", style: "margin: 0 0 8px" },
      "Saving keeps a version of the model; publishing makes the last saved version available on the Decide page."),
    history.slice(0, 12).map(c => el("div", { class: "item" },
      c.message, " ", c.published ? el("span", { class: "pill" }, `published ${c.published}`) : null,
      el("span", { class: "sub" }, `${new Date(c.date).toLocaleString()} · ${c.short}`))));
}

/** The decisions and results the Decide page offers (project.yaml). */
function managersCard() {
  const d = state.detail;
  const problems = d.decision_problems || [];
  const number = n => String(Math.round(n * 1e6) / 1e6);
  const describe = dec => dec.kind === "choice"
    ? dec.choices.map((c, i) => i === 0 ? `${c.label} (now)` : c.label).join(" · ")
    : `${number(dec.min)} to ${number(dec.max)}${dec.unit ? " " + dec.unit : ""}${dec.step ? `, steps of ${number(dec.step)}` : ""} (now ${dec.current === null ? "?" : number(dec.current)})`;
  return el("div", { class: "card", id: "managers" },
    el("h3", { style: "margin: 0 0 8px; font-size: 15px" }, "Decisions"),
    problems.length ? el("div", { class: "status" }, "Fix these before publishing:", el("ul", {}, problems.map(p => el("li", {}, p)))) : null,
    d.decisions.length
      ? el("dl", { class: "decisions" }, ...d.decisions.flatMap(dec => [el("dt", {}, dec.label), el("dd", {}, describe(dec))]))
      : el("p", { class: "hint", style: "margin: 0" }, "None yet: the Decide page can only run the model as published."),
    d.results.length ? el("p", { class: "hint" }, "Results shown: ", d.results.map(r => r.label + (r.better === "higher" ? " (higher is better)" : r.better === "lower" ? " (lower is better)" : "")).join(" · ")) : null,
    el("p", { class: "hint" }, "Defined in the project's project.yaml (decisions and results)."));
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

// --- saving, publishing and running --------------------------------------------------------

async function saveVersion() {
  const id = encodeURIComponent(state.project);
  const { ok, data } = await api("POST", `projects/${id}/save`, { changes: state.changes, message: state.saveMessage });
  if (!ok) {
    state.errors = data.errors || state.errors;
    state.message = { kind: "error", text: data.error || "Couldn't save." };
    return renderEditor();
  }
  state.message = { kind: "ok", text: `Saved: ${data.saved.message}. Publish it to make it available on the Decide page.` };
  await openProject(state.project, true);
}

async function publishVersion() {
  const id = encodeURIComponent(state.project);
  const { ok, data } = await api("POST", `projects/${id}/publish`, { message: state.saveMessage });
  state.message = ok
    ? { kind: "ok", text: `Published ${data.published.name}: it's now on the Decide page.` }
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
