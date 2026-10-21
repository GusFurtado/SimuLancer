"use strict";
// The landing page (/): the projects, each opened on the Modeling page
// (/modeling, edit the model) or the Simulation page (/simulation, run
// its scenarios); new projects; renaming, archiving, restoring and
// deleting them. Shared code: common.js.

const state = {
  projects: [],
  archived: [],
  examples: [],
  creating: null,     // the new-project form: { name, from, error }
  menu: null,         // id of the project whose ⋯ menu is open
  editing: null,      // { id, name, description, error }: renaming a project
  notice: null,       // { text, undo? }: what just happened
  showArchived: false,
  deleting: null,     // id of the archived project asking "delete for good?"
};

const enc = encodeURIComponent;

async function loadProjects() {
  const [{ data }, { data: archived }, { data: examples }] = await Promise.all([
    api("GET", "projects"), api("GET", "projects?archived=true"), api("GET", "examples")]);
  state.projects = data.projects || [];
  state.archived = archived.projects || [];
  state.examples = examples.examples || [];
  if (!state.archived.length) state.showArchived = false;
  render();
}

function render() {
  document.getElementById("notice").replaceChildren(...(state.notice ? [noticeBar()] : []));
  document.getElementById("projects").replaceChildren(
    ...state.projects.map(p => state.editing?.id === p.id ? editForm(p) : projectCard(p)),
    state.creating ? newProjectForm() : newProjectCard());
  document.getElementById("archived").replaceChildren(...archivedSection());
  if (state.creating) document.getElementById("new-name").focus();
  else if (state.editing && !document.activeElement?.closest?.(".door.editing")) document.getElementById("edit-name").focus();
}

function noticeBar() {
  const n = state.notice;
  return el("div", { class: "status ok notice" }, n.text,
    n.undo ? el("button", { class: "link", id: "undo", onclick: n.undo }, "Undo") : null,
    el("button", { class: "link close", title: "Dismiss", onclick: () => { state.notice = null; render(); } }, "×"));
}

// --- the projects ------------------------------------------------------------------------

function projectCard(p) {
  return el("div", { class: "door project", "data-project": p.id },
    el("span", { class: "who" }, p.published ? `Published ${p.published.name} · ${new Date(p.published.date).toLocaleDateString()}` : "Not published yet"),
    el("button", { class: "more", title: "More", "aria-label": `More for ${p.name}`, onclick: e => { e.stopPropagation(); state.menu = state.menu === p.id ? null : p.id; render(); } }, "⋯"),
    state.menu === p.id ? el("div", { class: "menu" },
      el("button", { "data-action": "rename", onclick: () => startEditing(p) }, "Rename and describe…"),
      el("button", { "data-action": "archive", onclick: () => archive(p) }, "Archive")) : null,
    el("h2", {}, p.name),
    el("p", {}, p.description || ""),
    el("div", { class: "actions" },
      el("a", { class: "button", href: `modeling?project=${enc(p.id)}` }, icon("modeling"), PAGES.modeling),
      p.published
        ? el("a", { class: "button primary", href: `simulation?project=${enc(p.id)}` }, icon("simulation"), PAGES.simulation)
        : el("span", { class: "hint", style: "margin: 0" }, "Publish a version to simulate scenarios.")));
}

// A click anywhere else closes the ⋯ menu.
document.addEventListener("click", e => {
  if (state.menu && !e.target.closest(".menu")) { state.menu = null; render(); }
});

function startEditing(p) {
  Object.assign(state, { menu: null, creating: null, editing: { id: p.id, name: p.name, description: p.description || "", error: null } });
  render();
}

function editForm(p) {
  const e = state.editing;
  const cancel = () => { state.editing = null; render(); };
  return el("div", { class: "door editing", "data-project": p.id },
    el("div", { class: "field" }, el("label", { for: "edit-name" }, "Name"),
      el("input", { id: "edit-name", value: e.name, oninput: ev => { e.name = ev.target.value; },
        onkeydown: ev => { if (ev.key === "Enter") saveDetails(); else if (ev.key === "Escape") cancel(); } })),
    el("div", { class: "field" }, el("label", { for: "edit-description" }, "Description"),
      el("textarea", { id: "edit-description", rows: 3, placeholder: "What this model is about, in a sentence or two",
        oninput: ev => { e.description = ev.target.value; }, onkeydown: ev => { if (ev.key === "Escape") cancel(); } }, e.description)),
    e.error ? el("div", { class: "status" }, e.error) : null,
    el("p", { class: "hint", style: "margin: 0 0 12px" }, "Saved as a new version of the model. Its address stays the same."),
    el("div", { class: "actions" },
      el("button", { class: "primary", id: "save-details", onclick: saveDetails }, "Save"),
      el("button", { onclick: cancel }, "Cancel")));
}

async function saveDetails() {
  const e = state.editing;
  if (!e.name.trim()) { e.error = "Give the project a name."; return render(); }
  const { ok, data } = await api("PATCH", `projects/${enc(e.id)}`, { name: e.name, description: e.description });
  if (!ok) { e.error = data.error || "The changes couldn't be saved."; return render(); }
  state.editing = null;
  state.notice = data.saved ? { text: `Saved ${data.name}.` } : null;
  loadProjects();
}

async function archive(p) {
  state.menu = null;
  const { ok, data } = await api("POST", `projects/${enc(p.id)}/archive`);
  state.notice = ok
    ? { text: `${p.name} was archived.`, undo: () => restore({ id: p.id, name: p.name }) }
    : { text: data.error || `${p.name} couldn't be archived.` };
  loadProjects();
}

// --- the archive -------------------------------------------------------------------------

function archivedSection() {
  if (!state.archived.length) return [];
  const toggle = el("button", { class: "link", id: "show-archived", onclick: () => { state.showArchived = !state.showArchived; render(); } },
    `${state.showArchived ? "Hide" : "Show"} archived projects (${state.archived.length})`);
  if (!state.showArchived) return [toggle];
  return [toggle, el("div", { class: "archived-list" }, ...state.archived.map(p => el("div", { class: "archived", "data-archived": p.id },
    el("span", { class: "name" }, el("b", {}, p.name), p.description ? el("span", { class: "sub" }, p.description) : null),
    state.deleting === p.id
      ? el("span", { class: "confirm" }, `Delete ${p.name}, its versions and scenarios for good?`,
          el("button", { class: "danger", "data-action": "confirm-delete", onclick: () => remove(p) }, "Delete"),
          el("button", { onclick: () => { state.deleting = null; render(); } }, "Cancel"))
      : el("span", { class: "row-actions" },
          el("button", { "data-action": "restore", onclick: () => restore(p) }, "Restore"),
          el("button", { class: "link danger", "data-action": "delete", onclick: () => { state.deleting = p.id; render(); } }, "Delete…")))))];
}

async function restore(p) {
  const { ok, data } = await api("POST", `projects/${enc(p.id)}/restore`);
  state.notice = { text: ok ? `${p.name} is back.` : data.error || `${p.name} couldn't be restored.` };
  loadProjects();
}

async function remove(p) {
  const { ok, data } = await api("DELETE", `projects/${enc(p.id)}`);
  state.deleting = null;
  state.notice = { text: ok ? `${p.name} was deleted.` : data.error || `${p.name} couldn't be deleted.` };
  loadProjects();
}

// --- new projects ------------------------------------------------------------------------

function newProjectCard() {
  return el("button", { class: "door new", id: "new-project", onclick: () => { Object.assign(state, { creating: { name: "", from: "", error: null }, editing: null }); render(); } },
    el("h2", {}, "+ New project"),
    el("p", {}, "Start from a simple line, an example, or a copy of another project."));
}

const idFor = name => name.toLowerCase().normalize("NFD").replace(/[\u0300-\u036f]/g, "").replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 64);

function newProjectForm() {
  const c = state.creating;
  return el("div", { class: "door new-project" },
    el("h2", {}, "New project"),
    el("div", { class: "field" }, el("label", { for: "new-name" }, "Name"),
      el("input", {
        id: "new-name", value: c.name, placeholder: exampleNamed(c.from)?.name || "e.g. Packing line 2",
        oninput: e => { c.name = e.target.value; },
        onkeydown: e => { if (e.key === "Enter") createProject(); else if (e.key === "Escape") { state.creating = null; render(); } },
      })),
    el("div", { class: "field" }, el("label", { for: "new-from" }, "Start from"),
      el("select", { id: "new-from", onchange: e => { c.from = e.target.value; render(); } },
        el("option", { value: "", selected: c.from === "" ? true : null }, "A simple line (arrivals, one machine, exit)"),
        el("optgroup", { label: "Examples" },
          ...state.examples.map(x => el("option", { value: `example:${x.id}`, selected: c.from === `example:${x.id}` ? true : null }, x.name))),
        state.projects.length ? el("optgroup", { label: "A copy of" },
          ...state.projects.map(p => el("option", { value: p.id, selected: c.from === p.id ? true : null }, p.name))) : null),
      exampleNamed(c.from)?.description ? el("p", { class: "hint", style: "margin: 4px 0 0" }, exampleNamed(c.from).description) : null),
    c.error ? el("div", { class: "status" }, c.error) : null,
    el("div", { class: "actions" },
      el("button", { class: "primary", id: "create-project", onclick: createProject }, "Create"),
      el("button", { onclick: () => { state.creating = null; render(); } }, "Cancel")));
}

/** The example chosen in "Start from", if it's one. */
const exampleNamed = from => from.startsWith("example:") ? state.examples.find(x => `example:${x.id}` === from) : null;

async function createProject() {
  const c = state.creating;
  const example = exampleNamed(c.from);
  const name = c.name.trim() || example?.name || "";
  const base = idFor(name);
  if (!base) { c.error = "Give the project a name."; return render(); }
  // Two projects may share a name, not an id (its address): "bank-branch-2".
  const taken = new Set([...state.projects, ...state.archived].map(p => p.id));
  let id = base;
  for (let n = 2; taken.has(id); n++) id = `${base.slice(0, 60)}-${n}`;
  const body = { id, name };
  if (example) body.example = example.id;
  else if (c.from) body.from = c.from;
  const { ok, data } = await api("POST", "projects", body);
  if (!ok) { c.error = data.error || "The project couldn't be created."; return render(); }
  location.href = `modeling?project=${enc(id)}`;
}

loadProjects();
