"use strict";
// The landing page (/): the projects, each opened on the Modeling page
// (/modeling, edit the model) or the Simulation page (/simulation, run
// its scenarios); new projects, blank or from a template (searchable by
// name and tags) or a copy; renaming, archiving, restoring and deleting
// them. Shared code: common.js.

const state = {
  projects: [],
  archived: [],
  templates: [],
  creating: null,     // the new-project form: { name, from, query, tags, error }; from is "", "template:<id>" or a project id
  menu: null,         // id of the project whose ⋯ menu is open
  editing: null,      // { id, name, description, error }: renaming a project
  notice: null,       // { text, undo? }: what just happened
  showArchived: false,
  deleting: null,     // id of the archived project asking "delete for good?"
};

const enc = encodeURIComponent;

async function loadProjects() {
  const [{ data }, { data: archived }, { data: templates }] = await Promise.all([
    api("GET", "projects"), api("GET", "projects?archived=true"), api("GET", "templates")]);
  state.projects = data.projects || [];
  state.archived = archived.projects || [];
  state.templates = templates.templates || [];
  state.decider = (await ME).role === "decider";
  if (!state.archived.length) state.showArchived = false;
  // Nothing yet: the first thing to do is start a project.
  if (!state.decider && !state.projects.length && !state.archived.length && !state.creating) startCreating();
  render();
}

function render() {
  // Re-rendering replaces the inputs: keep the focus and cursor where they were.
  const focused = document.activeElement?.id;
  const cursor = document.activeElement?.selectionStart;
  document.getElementById("notice").replaceChildren(...(state.notice ? [noticeBar()] : []));
  const creating = Boolean(state.creating);
  document.getElementById("lead").hidden = creating;
  document.getElementById("create").replaceChildren(...(creating ? [newProjectView()] : []));
  document.getElementById("projects").replaceChildren(...(creating ? [] : [
    // Deciders only see what's published: the rest is a modeler's work in progress.
    ...state.projects.filter(p => !state.decider || p.published).map(p => state.editing?.id === p.id ? editForm(p) : projectCard(p)),
    ...(state.decider ? [] : [newProjectCard()])]));
  document.getElementById("archived").replaceChildren(...(creating || state.decider ? [] : archivedSection()));
  const again = focused && document.getElementById(focused);
  if (again && again !== document.activeElement) {
    again.focus();
    if (cursor !== undefined && cursor !== null && again.setSelectionRange) again.setSelectionRange(cursor, cursor);
  } else if (state.editing && !document.activeElement?.closest?.(".door.editing")) document.getElementById("edit-name").focus();
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
    state.decider ? null : el("button", { class: "more", title: "More", "aria-label": `More for ${p.name}`, onclick: e => { e.stopPropagation(); state.menu = state.menu === p.id ? null : p.id; render(); } }, "⋯"),
    state.menu === p.id ? el("div", { class: "menu" },
      el("button", { "data-action": "rename", onclick: () => startEditing(p) }, "Rename and describe…"),
      el("button", { "data-action": "archive", onclick: () => archive(p) }, "Archive")) : null,
    el("h2", {}, p.name),
    el("p", {}, p.description || ""),
    el("div", { class: "actions" },
      state.decider ? null : el("a", { class: "button", href: `modeling?project=${enc(p.id)}` }, icon("modeling"), PAGES.modeling),
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
  const toggle = el("button", {
    class: "link", id: "show-archived", "aria-expanded": String(state.showArchived),
    onclick: () => { state.showArchived = !state.showArchived; render(); },
  }, svg("svg", { class: "caret", viewBox: "0 0 24 24", "aria-hidden": "true" }, svg("path", { d: "M9 5l7 7-7 7" })),
    `${state.showArchived ? "Hide" : "Show"} archived projects (${state.archived.length})`);
  if (!state.showArchived) return [toggle];
  return [toggle, el("div", { class: "archived-list" }, ...state.archived.map(p => el("div", { class: "archived", "data-archived": p.id },
    el("span", { class: "name" }, el("b", {}, p.name), p.description ? el("span", { class: "sub" }, p.description) : null),
    state.deleting === p.id
      ? el("span", { class: "confirm" }, `Delete ${p.name}, its versions, scenarios and runs for good?`,
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

function startCreating() {
  Object.assign(state, { creating: { name: "", from: "", query: "", tags: new Set(), error: null }, editing: null, menu: null });
}

function newProjectCard() {
  return el("button", { class: "door new", id: "new-project", onclick: () => { startCreating(); render(); document.getElementById("new-name").focus(); } },
    el("h2", {}, "+ New project"),
    el("p", {}, "Start blank or from a template, or copy one of your projects."));
}

const idFor = name => name.toLowerCase().normalize("NFD").replace(/[\u0300-\u036f]/g, "").replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 64);

/** Lowercase, without accents, for searching. */
const plain = text => text.toLowerCase().normalize("NFD").replace(/[\u0300-\u036f]/g, "");

/** Templates matching the search words (in name, description or tags) and every chosen tag. */
function matchingTemplates() {
  const c = state.creating;
  const words = plain(c.query).split(/\s+/).filter(Boolean);
  return state.templates.filter(t => {
    const text = plain([t.name, t.description || "", ...t.tags].join(" "));
    return words.every(w => text.includes(w)) && [...c.tags].every(tag => t.tags.includes(tag));
  });
}

/** Tags worth offering: those of the templates still matching (the most used
 *  first), plus the ones already chosen. */
function offeredTags(found) {
  const count = new Map();
  for (const t of found) for (const tag of t.tags) count.set(tag, (count.get(tag) || 0) + 1);
  const chosen = state.creating.tags;
  const others = [...count.keys()].filter(tag => !chosen.has(tag) && count.get(tag) < found.length)
    .sort((a, b) => count.get(b) - count.get(a) || a.localeCompare(b));
  return [...chosen, ...others];
}

const FEW_TAGS = 10;

const templateNamed = from => from.startsWith("template:") ? state.templates.find(t => `template:${t.id}` === from) : null;

function newProjectView() {
  const c = state.creating;
  const first = !state.projects.length && !state.archived.length;
  const cancel = () => { state.creating = null; render(); };
  const choice = (from, ...children) => el("button", {
    class: "choice" + (c.from === from ? " selected" : ""), "data-from": from, "aria-pressed": c.from === from ? "true" : "false",
    onclick: () => { c.from = from; c.error = null; render(); },
  }, ...children);
  const found = matchingTemplates();
  return el("div", { class: "creating" },
    el("h2", {}, first ? "Start your first project" : "New project"),
    el("div", { class: "field name" }, el("label", { for: "new-name" }, "Name"),
      el("input", {
        id: "new-name", value: c.name, placeholder: templateNamed(c.from)?.name || "e.g. Packing line 2",
        oninput: e => { c.name = e.target.value; },
        onkeydown: e => { if (e.key === "Enter") createProject(); else if (e.key === "Escape" && !first) cancel(); },
      })),
    el("h3", { class: "start-from" }, "Start from"),
    el("div", { class: "finder" },
      el("div", { class: "search" },
        svg("svg", { viewBox: "0 0 24 24", "aria-hidden": "true" }, svg("circle", { cx: 10.5, cy: 10.5, r: 6.5 }), svg("path", { d: "M15.5 15.5 21 21" })),
        el("input", { id: "template-search", type: "search", value: c.query, placeholder: "Search templates: bank, warehouse, shifts, rework…",
          "aria-label": "Search templates", oninput: e => { c.query = e.target.value; render(); } })),
      tagButtons(found)),
    el("div", { class: "choices" },
      choice("", el("b", {}, "Blank"), el("span", {}, "One source, one machine and an exit, to build on.")),
      ...found.map(t => choice(`template:${t.id}`, el("b", {}, t.name), el("span", {}, t.description || ""),
        el("span", { class: "chips" }, ...t.tags.map(tag => el("i", {}, tag))))),
      found.length ? null : el("p", { class: "hint none" }, "No template matches. Try fewer words or tags.")),
    state.projects.length ? el("div", { class: "field copy" }, el("label", { for: "copy-from" }, "Or copy one of your projects"),
      el("select", { id: "copy-from", onchange: e => { c.from = e.target.value; render(); } },
        el("option", { value: "", selected: state.projects.some(p => p.id === c.from) ? null : true }, "—"),
        ...state.projects.map(p => el("option", { value: p.id, selected: c.from === p.id ? true : null }, p.name)))) : null,
    c.error ? el("div", { class: "status" }, c.error) : null,
    el("div", { class: "actions" },
      el("button", { class: "primary", id: "create-project", onclick: createProject }, "Create"),
      first ? null : el("button", { onclick: cancel }, "Cancel")));
}

function tagButtons(found) {
  const c = state.creating;
  const tags = offeredTags(found);
  const shown = c.allTags ? tags : tags.slice(0, Math.max(FEW_TAGS, c.tags.size));
  return el("div", { class: "tags" }, ...shown.map(tag => el("button", {
    class: "tag" + (c.tags.has(tag) ? " on" : ""), "data-tag": tag, "aria-pressed": c.tags.has(tag) ? "true" : "false",
    onclick: () => { c.tags.has(tag) ? c.tags.delete(tag) : c.tags.add(tag); render(); },
  }, tag)),
  tags.length > shown.length || c.allTags && tags.length > FEW_TAGS
    ? el("button", { class: "link", id: "more-tags", onclick: () => { c.allTags = !c.allTags; render(); } },
        c.allTags ? "fewer tags" : `${tags.length - shown.length} more tags`)
    : null);
}

async function createProject() {
  const c = state.creating;
  const template = templateNamed(c.from);
  const name = c.name.trim() || template?.name || "";
  const base = idFor(name);
  if (!base) { c.error = "Give the project a name."; return render(); }
  // Two projects may share a name, not an id (its address): "bank-branch-2".
  const taken = new Set([...state.projects, ...state.archived].map(p => p.id));
  let id = base;
  for (let n = 2; taken.has(id); n++) id = `${base.slice(0, 60)}-${n}`;
  const body = { id, name };
  if (template) body.template = template.id;
  else if (c.from) body.from = c.from;
  const { ok, data } = await api("POST", "projects", body);
  if (!ok) { c.error = data.error || "The project couldn't be created."; return render(); }
  location.href = `modeling?project=${enc(id)}`;
}

loadProjects();
