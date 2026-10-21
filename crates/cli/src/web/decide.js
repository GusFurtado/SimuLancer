"use strict";
// The Simulation page (/decide): published versions only. The decisions the
// Modeling page made available, scenarios built from them, and comparisons with
// the current setup in plain language.

const state = {
  project: null,      // id
  detail: null,       // GET /projects/{id}?version=...
  scenarios: [],      // of this version
  selected: new Set(),
  editing: null,      // null, or { id (null for new), name, decisions }
  comparison: null,   // GET /comparisons/{id}
  details: null,      // { run, name }: a run whose report/animation is open
  view: "report",
  message: null,
};

const enc = encodeURIComponent;

async function openProject(id, version = "latest") {
  const { ok, data } = await api("GET", `projects/${enc(id)}?version=${enc(version)}`);
  if (!ok) {
    const { ok: exists, data: working } = await api("GET", `projects/${enc(id)}`);
    return projectMissing(document.getElementById("main"), exists
      ? `${working.name} isn't published yet. Publish a version from its Modeling page to simulate scenarios here.`
      : data.error || `The project ${id} can't be opened.`);
  }
  showProjectName(data.name || id, PAGES.decide);
  stopAnimation();
  Object.assign(state, { project: id, detail: data, editing: null, details: null, message: null, comparison: null });
  state.selected = new Set();
  await loadScenarios();
  // Pick up the latest comparison of this version, if there is one.
  const { data: list } = await api("GET", "comparisons");
  state.comparison = (list.comparisons || []).find(c => c.project === id && c.version === data.version) || null;
  render();
  if (state.comparison && ["queued", "running"].includes(state.comparison.status)) followComparison(state.comparison);
}

async function loadScenarios() {
  const { data } = await api("GET", `projects/${enc(state.project)}/scenarios?version=${enc(state.detail.version)}`);
  state.scenarios = data.scenarios || [];
  for (const id of [...state.selected]) if (!state.scenarios.some(s => s.id === id)) state.selected.delete(id);
}

// --- describing decisions --------------------------------------------------------------

const number = n => String(Math.round(n * 1e6) / 1e6);

function valueText(decision, value) {
  if (decision.kind === "choice") {
    const choice = decision.choices.find(c => c.id === value);
    return choice ? choice.label : String(value);
  }
  return decision.unit ? `${number(value)} ${decision.unit}` : number(value);
}

// --- rendering ---------------------------------------------------------------------------

function render() {
  const main = document.getElementById("main");
  const d = state.detail;
  if (!d) return;
  const version = d.versions.find(v => v.name === d.version);
  const versionPicker = el("select", {
    id: "version", style: "width: auto",
    onchange: e => openProject(state.project, e.target.value),
  }, ...d.versions.map((v, i) => el("option", { value: v.name, selected: v.name === d.version ? true : null },
    `${v.name}${i === 0 ? " (latest)" : ""} · ${v.message}`)));
  const head = el("div", { class: "head" },
    el("div", { class: "titles" }, el("h2", {}, d.name), el("p", {}, d.description || "")),
    el("label", { class: "hint", style: "margin: 0" }, "Version ", versionPicker));
  const notLatest = d.versions[0] && d.versions[0].name !== d.version
    ? el("div", { class: "status ok" }, `You're looking at ${d.version}. The latest version is ${d.versions[0].name}.`)
    : null;

  if (!d.decisions.length) {
    const content = el("div");
    main.replaceChildren(head, notLatest || el("div"),
      el("div", { class: "card", style: "margin-bottom: 14px" },
        el("p", { style: "margin: 0 0 4px" }, el("span", { class: "pill" }, d.version), " ",
          version ? `${version.message} · published ${new Date(version.date).toLocaleString()}` : ""),
        el("p", { class: "hint", style: "margin: 0" },
          "This version has no decisions to try yet, so there's nothing to compare. You can still run the model as it is.")),
      el("div", { class: "actions" }, el("button", { class: "primary", id: "run", onclick: runAsPublished }, "▶ Run the published model")),
      statusBox(), content);
    if (state.details) runResults(content, state.details.run, state.view, v => { state.view = v; render(); });
    return;
  }

  const details = el("div", { id: "details" });
  main.replaceChildren(head, notLatest || el("div"),
    el("div", { class: "decide" },
      currentSetupCard(),
      scenariosCard()),
    statusBox(),
    comparisonView(),
    details);
  if (state.details) {
    details.prepend(el("h3", { class: "section-title" }, `Details: ${state.details.name}`));
    const box = el("div");
    details.append(box);
    runResults(box, state.details.run, state.view, v => { state.view = v; render(); });
  }
}

function currentSetupCard() {
  const d = state.detail;
  return el("div", { class: "card" },
    el("h3", { class: "card-title" }, "What you can change"),
    el("p", { class: "hint", style: "margin: 0 0 10px" }, "Set in ", el("b", {}, d.version), " on the Modeling page. Each scenario changes some of these; everything else stays as it is now."),
    el("dl", { class: "decisions" }, ...d.decisions.flatMap(dec => [
      el("dt", {}, dec.label, dec.description ? el("span", { class: "sub" }, dec.description) : null),
      el("dd", {}, el("span", { class: "now" }, "Now: "), valueText(dec, dec.current),
        dec.kind === "number" ? el("span", { class: "sub" }, `can be ${valueText(dec, dec.min)} to ${valueText(dec, dec.max)}`) : null),
    ])));
}

function scenariosCard() {
  const rows = state.scenarios.map(s => el("div", { class: "scenario" + (state.selected.has(s.id) ? " selected" : "") },
    el("label", {},
      el("input", {
        type: "checkbox", checked: state.selected.has(s.id) ? true : null, "data-scenario": s.id,
        onchange: e => { e.target.checked ? state.selected.add(s.id) : state.selected.delete(s.id); render(); },
      }),
      el("span", {}, el("b", {}, s.name), s.summary === s.name ? null : el("span", { class: "sub" }, s.summary || "Same as now"))),
    el("button", { class: "link", title: "Change this scenario", onclick: () => startEditing(s) }, "Edit"),
    el("button", { class: "link danger", title: "Delete this scenario", onclick: () => deleteScenario(s) }, "Delete")));
  const compare = el("button", {
    class: "primary", id: "compare", disabled: state.selected.size ? null : true,
    onclick: startComparison,
  }, state.selected.size ? `Compare ${state.selected.size} with the current setup` : "Compare with the current setup");
  return el("div", { class: "card" },
    el("h3", { class: "card-title" }, "Scenarios"),
    rows.length ? el("div", { class: "scenarios" }, ...rows)
      : el("p", { class: "hint", style: "margin: 0 0 10px" }, "No scenarios yet. Create one to see what a change would do."),
    state.editing ? scenarioForm() : null,
    el("div", { class: "actions", style: "margin-top: 12px" },
      state.editing ? null : el("button", { id: "new-scenario", onclick: () => startEditing(null) }, "+ New scenario"),
      el("span", { style: "flex: 1" }),
      compare));
}

function startEditing(scenario) {
  const d = state.detail;
  const decisions = {};
  for (const dec of d.decisions) decisions[dec.id] = scenario && dec.id in scenario.decisions ? scenario.decisions[dec.id] : dec.current;
  state.editing = { id: scenario ? scenario.id : null, name: scenario ? scenario.name : "", decisions, error: null };
  render();
  const first = document.querySelector("#scenario-form [data-decision]");
  if (first) first.focus();
}

/** "Inspectors: 2 · Mill: A 10% faster mill": what the picks change. */
function suggestedName(decisions) {
  return state.detail.decisions
    .filter(dec => !equal(decisions[dec.id], dec.current))
    .map(dec => `${dec.label}: ${valueText(dec, decisions[dec.id])}`)
    .join(" · ");
}

function scenarioForm() {
  const e = state.editing;
  const controls = state.detail.decisions.map(dec => {
    let control;
    const changed = !equal(e.decisions[dec.id], dec.current);
    if (dec.kind === "choice") {
      control = el("select", {
        "data-decision": dec.id,
        onchange: ev => { e.decisions[dec.id] = ev.target.value; render(); },
      }, ...dec.choices.map(c => el("option", { value: c.id, selected: c.id === e.decisions[dec.id] ? true : null },
        c.id === dec.current ? `${c.label} (now)` : c.label)));
    } else {
      control = el("div", { class: "number" },
        el("input", {
          type: "number", "data-decision": dec.id, min: dec.min, max: dec.max, step: dec.step || "any",
          value: e.decisions[dec.id],
          onchange: ev => { e.decisions[dec.id] = Number(ev.target.value); render(); },
        }),
        dec.unit ? el("span", {}, dec.unit) : null,
        el("span", { class: "sub" }, `now ${valueText(dec, dec.current)} · ${number(dec.min)} to ${number(dec.max)}`));
    }
    return el("div", { class: "field" + (changed ? " changed" : "") }, el("label", {}, dec.label), control);
  });
  const suggestion = suggestedName(e.decisions);
  return el("div", { class: "scenario-form", id: "scenario-form" },
    el("div", { class: "group-title" }, e.id ? "Change scenario" : "New scenario"),
    ...controls,
    el("div", { class: "field" },
      el("label", {}, "Name"),
      el("input", {
        id: "scenario-name", value: e.name, placeholder: suggestion || "Same as now",
        oninput: ev => { e.name = ev.target.value; },
      })),
    e.error ? el("div", { class: "status" }, e.error) : null,
    el("div", { class: "actions" },
      el("button", { class: "primary", id: "save-scenario", onclick: saveScenario }, e.id ? "Save changes" : "Create scenario"),
      el("button", { onclick: () => { state.editing = null; render(); } }, "Cancel")));
}

async function saveScenario() {
  const e = state.editing;
  // Send only what differs from now: the scenario then reads as its changes.
  const decisions = {};
  for (const dec of state.detail.decisions) if (!equal(e.decisions[dec.id], dec.current)) decisions[dec.id] = e.decisions[dec.id];
  const body = { name: e.name.trim() || suggestedName(e.decisions) || undefined, decisions };
  const url = `projects/${enc(state.project)}/scenarios`;
  const { ok, data } = e.id
    ? await api("PUT", `${url}/${enc(e.id)}`, body)
    : await api("POST", url, { ...body, version: state.detail.version });
  if (!ok) {
    e.error = (data.errors && data.errors.length) ? data.errors.map(x => x.message).join("; ") : data.error;
    return render();
  }
  state.editing = null;
  state.selected.add(data.id);
  await loadScenarios();
  render();
}

async function deleteScenario(s) {
  if (!confirm(`Delete the scenario “${s.name}”?`)) return;
  await api("DELETE", `projects/${enc(state.project)}/scenarios/${enc(s.id)}`);
  state.selected.delete(s.id);
  await loadScenarios();
  render();
}

function statusBox() {
  const m = state.message;
  if (!m) return el("div");
  return el("div", { class: m.kind === "ok" ? "status ok" : "status" }, m.text,
    m.progress !== undefined ? el("div", { class: "progress" }, el("div", { style: `width: ${m.progress * 100}%` })) : null);
}

// --- comparing ----------------------------------------------------------------------------

async function startComparison() {
  const scenarios = state.scenarios.filter(s => state.selected.has(s.id)).map(s => s.id);
  const { ok, data } = await api("POST", "comparisons", { project: state.project, version: state.detail.version, scenarios });
  if (!ok) {
    state.message = { kind: "error", text: data.error || (data.errors || []).map(e => e.message).join("; ") || "The comparison couldn't start." };
    return render();
  }
  state.details = null;
  followComparison(data);
}

async function followComparison(comparison) {
  let c = comparison;
  while (c.status === "queued" || c.status === "running") {
    state.comparison = null;
    state.message = {
      kind: "ok",
      text: c.status === "queued" ? "Waiting for other runs to finish…" : `Simulating the current setup and ${c.scenarios.length} scenario${c.scenarios.length > 1 ? "s" : ""}: ${c.progress.done} of ${c.progress.total} replications`,
      progress: c.progress.done / Math.max(1, c.progress.total),
    };
    render();
    await new Promise(r => setTimeout(r, 500));
    const { ok, data } = await api("GET", `comparisons/${c.id}`);
    if (!ok) break;
    c = data;
    if (state.project !== c.project) return;   // they moved on
  }
  state.message = c.status === "failed" ? { kind: "error", text: `The comparison failed: ${c.error}` } : null;
  state.comparison = c.status === "done" ? c : null;
  render();
  const view = document.getElementById("comparison");
  if (view) view.scrollIntoView({ behavior: "smooth", block: "start" });
}

function verdictBadge(r) {
  if (r.small) return el("span", { class: "verdict same" }, "≈ too small to matter");
  if (r.judgment) return el("span", { class: `verdict ${r.judgment}` }, r.verdict === "higher" ? "▲ " : "▼ ", r.judgment);
  if (r.verdict === "no_clear_change" || r.verdict === "identical") return el("span", { class: "verdict same" }, "= no clear change");
  if (r.verdict === "higher" || r.verdict === "lower") return el("span", { class: "verdict neutral" }, r.verdict === "higher" ? "▲ higher" : "▼ lower");
  return el("span", { class: "verdict same" }, "can't tell");
}

/** For each result with a "better" direction, the scenario that does best
 *  (only if it's clearly better than now). */
function bestByResult(c) {
  return c.results.map((spec, i) => {
    if (spec.better === "neither") return null;
    let best = null;
    c.scenarios.forEach((s, k) => {
      const r = s.results[i];
      if (!r || r.judgment !== "better") return;
      const score = spec.better === "higher" ? r.scenario.mean : -r.scenario.mean;
      if (!best || score > best.score) best = { k, score };
    });
    return best ? best.k : null;
  });
}

function comparisonView() {
  const c = state.comparison;
  if (!c) return el("div");
  const best = bestByResult(c);
  // The headline: for each result with a direction, the best scenario.
  const headline = el("ul", { class: "headline" }, ...c.results.flatMap((spec, i) => {
    if (spec.better === "neither") return [];
    const k = best[i];
    if (k === null) return [el("li", {}, el("b", {}, spec.label), ": none of these scenarios clearly improves it.")];
    const r = c.scenarios[k].results[i];
    return [el("li", {}, el("b", {}, spec.label), ": ", el("b", {}, c.scenarios[k].name), ` does best, ${r.text.change} (${r.text.range}).`)];
  }));
  const header = el("tr", {}, el("th", {}, "Result"), el("th", {}, "Now"),
    ...c.scenarios.map(s => el("th", {}, s.name, s.summary === s.name ? null : el("span", { class: "sub" }, s.summary || "Same as now"))));
  const rows = c.results.map((spec, i) => el("tr", {},
    el("th", { scope: "row" }, spec.label,
      el("span", { class: "sub" }, spec.better === "higher" ? "higher is better" : spec.better === "lower" ? "lower is better" : "")),
    el("td", { class: "now" }, c.scenarios[0] && c.scenarios[0].results[i] ? c.scenarios[0].results[i].text.current : ""),
    ...c.scenarios.map((s, k) => {
      const r = s.results[i];
      if (!r) return el("td", {}, "–");
      return el("td", { class: best[i] === k ? "best" : null, title: r.sentence },
        el("span", { class: "value" }, r.text.scenario),
        el("span", { class: "change" }, r.text.change),
        r.text.range ? el("span", { class: "range" }, r.text.range) : null,
        verdictBadge(r),
        best[i] === k ? el("span", { class: "best-tag" }, "best") : null);
    })));
  const words = c.scenarios.map(s => el("div", { class: "words" },
    el("h4", {}, s.name),
    el("ul", {}, ...s.results.map(r => el("li", { class: "sentence" }, r.sentence)))));
  const detailButtons = [
    el("button", { onclick: () => openDetails(c.current.run, "Current setup") }, "Current setup"),
    ...c.scenarios.map(s => el("button", { onclick: () => openDetails(s.run, s.name) }, s.name)),
  ];
  return el("section", { class: "comparison", id: "comparison" },
    el("h3", { class: "section-title" }, "Compared with the current setup"),
    headline,
    el("div", { class: "table-wrap" }, el("table", { class: "compare-table" }, el("thead", {}, header), el("tbody", {}, ...rows))),
    el("h3", { class: "section-title" }, "In words"),
    el("div", { class: "words-grid" }, ...words),
    el("p", { class: "hint" },
      `Each scenario was simulated ${c.replications} times with the same random events as the current setup (the same arrivals, breakdowns and process times), so the differences come from the decisions, not from luck. `,
      "Ranges show where the true change lies, with 95% confidence. When you compare many scenarios, now and then a very small change may look clear by chance."),
    el("div", { class: "actions" }, el("span", { class: "hint", style: "margin: 0" }, "Reports and animations:"), ...detailButtons));
}

async function openDetails(runId, name) {
  const { ok, data } = await api("GET", `runs/${runId}`);
  if (!ok) return;
  stopAnimation();
  state.details = { run: data, name };
  state.view = "report";
  render();
  document.getElementById("details").scrollIntoView({ behavior: "smooth", block: "start" });
}

async function runAsPublished() {
  const { ok, data } = await api("POST", "runs", { project: state.project, version: state.detail.version, label: `${state.detail.name} ${state.detail.version}` });
  if (!ok) {
    state.message = { kind: "error", text: data.error || "The run couldn't start." };
    return render();
  }
  const run = await waitForRun(data, r => {
    state.message = { kind: "ok", text: progressText(r), progress: r.progress.done / Math.max(1, r.progress.total) };
    render();
  });
  state.message = run.status === "failed" ? { kind: "error", text: `The run failed: ${run.error}` } : null;
  state.details = { run, name: "As published" };
  state.view = "report";
  render();
}

const opened = pageProject();
if (opened) openProject(opened);
