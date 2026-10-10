"use strict";
// The manager's page (/decide): published versions only. For now: run the
// published model and read the results. Next: the decisions engineers expose,
// scenarios built from them, and plain-language comparisons.

const state = {
  projects: [],
  project: null,   // id
  detail: null,    // GET /projects/{id}?version=latest
  run: null,
  view: "report",
  message: null,
};

async function loadProjects() {
  const { data } = await api("GET", "projects");
  state.projects = (data.projects || []).filter(p => p.published);
  const list = document.getElementById("projects");
  if (!state.projects.length) {
    list.replaceChildren(el("p", { class: "hint", style: "margin: 0 8px" }, "Nothing published yet. Your engineers publish projects from the Build page."));
    return;
  }
  list.replaceChildren(...state.projects.map(p => el("button", {
    class: p.id === state.project ? "current" : null,
    onclick: () => openProject(p.id),
  }, p.name, el("span", { class: "sub" }, `${p.published.name} · ${new Date(p.published.date).toLocaleDateString()}`))));
}

async function openProject(id) {
  const { ok, data } = await api("GET", `projects/${encodeURIComponent(id)}?version=latest`);
  if (!ok) return;
  stopAnimation();
  Object.assign(state, { project: id, detail: data, run: null, message: null });
  loadProjects();
  render();
}

function render() {
  const main = document.getElementById("main");
  const d = state.detail;
  if (!d) {
    main.replaceChildren(el("p", { class: "empty" }, "Pick a project on the left."));
    return;
  }
  const version = d.versions.find(v => v.name === d.version);
  const content = el("div");
  main.replaceChildren(
    el("div", { class: "head" },
      el("div", { class: "titles" },
        el("h2", {}, d.name),
        el("p", {}, d.description || ""))),
    el("div", { class: "card", style: "margin-bottom: 14px" },
      el("p", { style: "margin: 0 0 4px" }, el("span", { class: "pill" }, d.version), " ",
        version ? `${version.message} · published ${new Date(version.date).toLocaleString()}` : ""),
      el("p", { class: "hint", style: "margin: 0" },
        "The decisions your engineers make available (a faster machine, a bigger buffer…) will appear here, so you can compare scenarios. For now you can run the published model as it is.")),
    el("div", { class: "actions" },
      el("button", { class: "primary", id: "run", onclick: startRun }, "▶ Run the published model")),
    statusBox(),
    content);
  if (state.run) runResults(content, state.run, state.view, v => { state.view = v; render(); });
}

function statusBox() {
  const m = state.message;
  if (!m) return el("div");
  return el("div", { class: m.kind === "ok" ? "status ok" : "status" }, m.text,
    m.progress !== undefined ? el("div", { class: "progress" }, el("div", { style: `width: ${m.progress * 100}%` })) : null);
}

async function startRun() {
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
  state.run = run;
  state.view = "report";
  render();
}

loadProjects();
