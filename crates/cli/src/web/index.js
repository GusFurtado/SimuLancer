"use strict";
// The landing page (/): the projects, each opened on the Build page (edit the
// model) or the Decide page (run its scenarios), and new projects.
// Shared code: common.js.

const state = {
  projects: [],
  creating: null,   // the new-project form: { name, from, error }
};

const enc = encodeURIComponent;

async function loadProjects() {
  const { data } = await api("GET", "projects");
  state.projects = data.projects || [];
  render();
}

function render() {
  const list = document.getElementById("projects");
  list.replaceChildren(...state.projects.map(projectCard), state.creating ? newProjectForm() : newProjectCard());
  if (state.creating) document.getElementById("new-name").focus();
}

function projectCard(p) {
  return el("div", { class: "door project", "data-project": p.id },
    el("span", { class: "who" }, p.published ? `Published ${p.published.name} · ${new Date(p.published.date).toLocaleDateString()}` : "Not published yet"),
    el("h2", {}, p.name),
    el("p", {}, p.description || ""),
    el("div", { class: "actions" },
      el("a", { class: "button", href: `build?project=${enc(p.id)}` }, "Edit the model"),
      p.published
        ? el("a", { class: "button primary", href: `decide?project=${enc(p.id)}` }, "Run scenarios")
        : el("span", { class: "hint", style: "margin: 0" }, "Publish a version to run scenarios.")));
}

function newProjectCard() {
  return el("button", { class: "door new", id: "new-project", onclick: () => { state.creating = { name: "", from: "", error: null }; render(); } },
    el("h2", {}, "+ New project"),
    el("p", {}, "Start from a simple line or a copy of another project."));
}

const idFor = name => name.toLowerCase().normalize("NFD").replace(/[̀-ͯ]/g, "").replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 64);

function newProjectForm() {
  const c = state.creating;
  return el("div", { class: "door new-project" },
    el("h2", {}, "New project"),
    el("div", { class: "field" }, el("label", { for: "new-name" }, "Name"),
      el("input", {
        id: "new-name", value: c.name, placeholder: "e.g. Packing line 2",
        oninput: e => { c.name = e.target.value; },
        onkeydown: e => { if (e.key === "Enter") createProject(); else if (e.key === "Escape") { state.creating = null; render(); } },
      })),
    el("div", { class: "field" }, el("label", { for: "new-from" }, "Start from"),
      el("select", { id: "new-from", onchange: e => { c.from = e.target.value; } },
        el("option", { value: "", selected: c.from === "" ? true : null }, "A simple line (arrivals, one machine, exit)"),
        ...state.projects.map(p => el("option", { value: p.id, selected: c.from === p.id ? true : null }, `A copy of ${p.name}`)))),
    c.error ? el("div", { class: "status" }, c.error) : null,
    el("div", { class: "actions" },
      el("button", { class: "primary", id: "create-project", onclick: createProject }, "Create"),
      el("button", { onclick: () => { state.creating = null; render(); } }, "Cancel")));
}

async function createProject() {
  const c = state.creating;
  const name = c.name.trim();
  const id = idFor(name);
  if (!id) { c.error = "Give the project a name."; return render(); }
  const body = { id, name };
  if (c.from) body.from = c.from;
  const { ok, data } = await api("POST", "projects", body);
  if (!ok) { c.error = data.error || "The project couldn't be created."; return render(); }
  location.href = `build?project=${enc(id)}`;
}

loadProjects();
