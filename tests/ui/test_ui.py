"""Drives the web pages through a Modeling session and a Simulation session in
Firefox (Marionette).

Usage: test_ui.py <server url> <marionette port> <screenshot dir>
Run by tests/ui/run.sh, which starts the server (on a new, empty data
folder) and Firefox.
"""
import json, os, sys, time, urllib.request
from marionette import Browser

url, port, out = sys.argv[1].rstrip("/"), int(sys.argv[2]), sys.argv[3]
os.makedirs(out, exist_ok=True)

def check(condition, what):
    if not condition:
        raise AssertionError(what)

def click_text(selector, text):
    b.js(f"[...document.querySelectorAll({selector!r})].find(x => x.textContent.trim().startsWith({text!r})).click()")

def set_value(element_id, value, event="change"):
    b.js(f"const i = document.getElementById({element_id!r}); i.value = {value!r}; i.dispatchEvent(new Event({event!r}))")

def click_in_diagram(title):
    b.js(f"""[...document.querySelectorAll('.diagram rect, .diagram polyline.hit')]
             .find(r => r.querySelector('title')?.textContent.startsWith({title!r}))
             .dispatchEvent(new MouseEvent('click', {{bubbles: true}}))""")

def from_template(template, id=None):
    """Creates a project from a template through the API, like the New project form."""
    request = urllib.request.Request(url + "/api/v1/projects", method="POST",
        data=json.dumps({"id": id or template, "template": template}).encode(), headers={"content-type": "application/json"})
    urllib.request.urlopen(request).close()

def choices():
    return b.js("return [...document.querySelectorAll('.choice b')].map(x => x.textContent)")

b = Browser(port)
b.call("WebDriver:SetWindowRect", {"width": 1400, "height": 950})

# --- First run: no projects, so the page offers to start one. -------------------
b.go(url + "/")
b.wait("return document.querySelector('.creating h2')?.textContent === 'Start your first project'")
check(choices()[0] == "Blank" and "Bank branch" in choices(), f"blank and templates: {choices()}")
check(b.js("return document.querySelector('[data-from=\"\"]').classList.contains('selected')"), "blank by default")
b.js("document.getElementById('template-search').focus()")
set_value("template-search", "bank", "input")
b.wait("return document.querySelectorAll('.choice').length === 2")
check(choices() == ["Blank", "Bank branch"], f"search: {choices()}")
check(b.js("return document.activeElement.id") == "template-search", "typing keeps the focus")
set_value("template-search", "", "input")
b.js("document.querySelector('[data-tag=manufacturing]').click()")
b.js("document.querySelector('[data-tag=rework]').click()")
b.wait("return document.querySelectorAll('.choice').length === 3")
check(choices() == ["Blank", "Washing machine assembly line", "Two cells"], f"tags: {choices()}")
b.screenshot(f"{out}/0-first-run.png")

# The sessions below use two of the templates.
for template in ["two-cells", "bike-assembly"]:
    from_template(template)

# --- The landing page: the projects, each opened to edit or to run. -------------
b.go(url + "/")
b.wait("return document.querySelectorAll('.door.project').length === 2")
listed = b.js("return [...document.querySelectorAll('.door.project h2')].map(h => h.textContent)")
check(listed == ["Bike assembly", "Two cells"], f"projects listed: {listed}")
check(b.js("return document.querySelector('[data-project=two-cells] a[href=\"simulation?project=two-cells\"]')") is not None, "two cells can be run")
b.screenshot(f"{out}/1-landing.png")

# --- Build: a new project, drawn in the browser. ---------------------------------
def obj(name):
    return f"document.querySelector('rect[data-object=\"{name}\"]')"

def click_obj(name):
    b.js(f"{obj(name)}.dispatchEvent(new MouseEvent('click', {{bubbles: true}}))")

def conveyor_length(name):
    click_in_diagram(f"conveyor {name}:")
    b.wait(f"return document.querySelector('.panel h3')?.textContent === {name!r}")
    return b.js(f"return document.getElementById('f-conveyors-{name}-length').value")

b.js("document.getElementById('new-project').click()")
b.wait("return document.getElementById('new-name') !== null")
set_value("new-name", "Packing line", "input")
b.js("document.getElementById('create-project').click()")
b.wait("return document.querySelector('.head h2')?.textContent === 'Packing line' && document.querySelector('.diagram svg') !== null", 10)
check(b.js("return document.getElementById('project-name').textContent") == "Packing line", "project in the top bar")
check(b.js("return document.getElementById('projects')") is None, "no list of other projects while editing")

b.js("document.getElementById('add-server').click()")
b.wait(f"return {obj('machine_2')} !== null")
set_value("f-name", "packer")
b.wait(f"return {obj('packer')} !== null && document.querySelector('.panel h3')?.textContent === 'packer'")

click_in_diagram("conveyor c2:")
b.wait("return document.querySelector('.panel h3')?.textContent === 'c2'")
b.js("document.getElementById('delete').click()")
b.wait("return ![...document.querySelectorAll('.diagram title')].some(t => t.textContent.startsWith('conveyor c2:'))")

for a, z in [("machine", "packer"), ("packer", "shipping")]:
    b.js("document.getElementById('connect').click()")
    click_obj(a)
    click_obj(z)
    b.wait(f"return [...document.querySelectorAll('.diagram title')].some(t => t.textContent === 'conveyor c_{a}_{z}: {a} → {z}')", 10)

before = conveyor_length("c_machine_packer")
# Drag the packer 4 m to the right: the conveyor's length follows the drawing.
b.js(f"""
  const r = {obj('packer')}, box = r.getBoundingClientRect(), px = box.width / 1.2;
  const at = (dx) => ({{ clientX: box.x + box.width / 2 + dx, clientY: box.y + box.height / 2, bubbles: true, button: 0 }});
  r.dispatchEvent(new PointerEvent('pointerdown', at(0)));
  window.dispatchEvent(new PointerEvent('pointermove', at(2 * px)));
  window.dispatchEvent(new PointerEvent('pointermove', at(4 * px)));
  window.dispatchEvent(new PointerEvent('pointerup', at(4 * px)));
""")
time.sleep(0.3)
after = conveyor_length("c_machine_packer")
check(float(after.split()[0]) > float(before.split()[0]) + 3, f"length follows the drawing: {before} -> {after}")
b.js("document.getElementById('undo').click()")
check(conveyor_length("c_machine_packer") == before, "undo restores the length")
b.js("document.getElementById('redo').click()")
check(conveyor_length("c_machine_packer") == after, "redo applies it again")

click_obj("packer")
b.wait("return document.querySelector('.panel h3')?.textContent === 'packer'")
set_value("f-servers-packer-process_time-kind", "exponential")
b.wait("return document.getElementById('f-servers-packer-process_time')?.textContent === 'exponential(40 s)'")
b.wait("return document.querySelector('.field.error') === null && !document.querySelector('.status:not(.ok)')", 10)
b.screenshot(f"{out}/2a-drawn.png")
set_value("save-message", "Packer after the machine", "input")
b.js("document.getElementById('save').click()")
b.wait("return document.getElementById('pending')?.textContent === 'all changes saved'", 10)
check(b.js(f"return {obj('packer')} !== null"), "the drawing is saved with the layout")
check("Packer after the machine" in b.js("return document.querySelector('.versions').textContent"), "saved as a version")

# Part types: add one, mix it at the source, time it separately at the packer.
b.js("document.querySelector('.diagram svg').dispatchEvent(new MouseEvent('click', {bubbles: true}))")
b.wait("return document.getElementById('add-part-type') !== null")
b.js("document.getElementById('add-part-type').click()")
b.wait("return document.querySelector('[data-type=type_2]') !== null")
b.js("const i = document.querySelector('[data-type=type_2]'); i.value = 'box'; i.dispatchEvent(new Event('change'))")
b.wait("return document.querySelector('[data-type=box]') !== null")
click_obj("arrivals")
b.wait("return document.getElementById('mix') !== null")
b.js("document.getElementById('mix').click()")
b.wait("return document.querySelector('[data-share=box]') !== null")
b.js("const i = document.querySelector('[data-share=box]'); i.value = '0.3'; i.dispatchEvent(new Event('change'))")
b.js("const i = document.querySelector('[data-share=part]'); i.value = '0.7'; i.dispatchEvent(new Event('change'))")
click_obj("packer")
b.wait("return document.getElementById('per-type') !== null")
b.js("document.getElementById('per-type').click()")
b.wait("return document.getElementById('f-servers-packer-process_time-box-kind') !== null")
set_value("f-servers-packer-process_time-box-mean", "90 s")
b.wait("return document.getElementById('f-servers-packer-process_time-box')?.textContent === 'exponential(90 s)'")
b.wait("return document.querySelector('.field.error') === null && !document.querySelector('.status:not(.ok)')", 10)
b.screenshot(f"{out}/2b-part-types.png")
# A shared operator for the machine and the packer, made a decision.
b.js("document.querySelector('.diagram svg').dispatchEvent(new MouseEvent('click', {bubbles: true}))")
b.wait("return document.getElementById('add-resource') !== null")
b.js("document.getElementById('add-resource').click()")
b.wait("return document.querySelector('[data-resource=operator]') !== null")
for machine in ["machine", "packer"]:
    click_obj(machine)
    b.wait(f"return document.querySelector('.panel h3')?.textContent === '{machine}' && document.querySelector('[data-need=operator]') !== null")
    b.js("const i = document.querySelector('[data-need=operator]'); i.value = '1'; i.dispatchEvent(new Event('change'))")
    b.wait(f"return document.querySelector('[data-need=operator]')?.value === '1'")
b.js("document.querySelector('.diagram svg').dispatchEvent(new MouseEvent('click', {bubbles: true}))")
b.wait("return document.querySelector('[data-expose=\"resources.operator.capacity\"]') !== null")
b.js("document.querySelector('[data-expose=\"resources.operator.capacity\"]').click()")
b.wait("return document.querySelector('[data-decision-id=operator_units_available]') !== null", 10)
b.js("[...document.querySelectorAll('.tabs button')].find(x => x.textContent === 'Model').click()")
b.wait("return document.querySelector('.diagram svg') !== null")
b.wait("return document.querySelector('.field.error') === null && !document.querySelector('.status:not(.ok)')", 10)
# A day shift for the packer.
b.js("document.querySelector('.diagram svg').dispatchEvent(new MouseEvent('click', {bubbles: true}))")
b.wait("return document.getElementById('add-schedule') !== null")
b.js("document.getElementById('add-schedule').click()")
b.wait("return document.querySelector('[data-schedule=day_shift]') !== null")
b.js("const i = document.querySelector('[data-shifts=day_shift]'); i.value = '06:00-14:00, 14:00-22:00'; i.dispatchEvent(new Event('change'))")
b.wait("return document.querySelector('[data-shifts=day_shift]')?.value === '06:00-14:00, 14:00-22:00'")
click_obj("packer")
b.wait("return document.getElementById('f-servers-packer-schedule') !== null")
set_value("f-servers-packer-schedule", "day_shift")
b.wait("return document.getElementById('f-servers-packer-schedule')?.value === 'day_shift'")
time.sleep(0.5)
b.wait("return document.querySelector('.field.error') === null && !document.querySelector('.status:not(.ok)')", 10)
set_value("save-message", "Boxes too", "input")
b.js("document.getElementById('save').click()")
b.wait("return document.getElementById('pending')?.textContent === 'all changes saved'", 10)
b.js("document.getElementById('run').click()")
b.wait("return document.querySelector('iframe.report') !== null", 60)
time.sleep(1)
measures = b.js("return document.querySelector('iframe.report').contentDocument.body.textContent")
check("throughput of box" in measures, "per-type measures in the report")
check("waiting for resources" in measures and "operator" in measures, "resource measures in the report")
check("off shift" in measures, "schedule measures in the report")
# New projects record an animation.
click_text(".results-bar button", "Animation")
b.wait("return document.querySelector('.animation canvas') !== null", 30)
# Without recording, the Animation tab says how to turn it on.
b.js("[...document.querySelectorAll('.tabs button')].find(x => x.textContent === 'Model').click()")
b.wait("return document.querySelector('.diagram svg') !== null")
b.js("document.querySelector('.diagram svg').dispatchEvent(new MouseEvent('click', {bubbles: true}))")
b.wait("return document.getElementById('f-event-log')?.checked === true")
b.js("const c = document.getElementById('f-event-log'); c.checked = false; c.dispatchEvent(new Event('change'))")
b.wait("return document.getElementById('pending')?.textContent.includes('1 unsaved change')")
b.js("document.getElementById('run').click()")
b.wait("return document.querySelector('iframe.report') !== null", 60)
click_text(".results-bar button", "Animation")
b.wait("return document.getElementById('no-animation')?.textContent.includes('Record an animation')", 10)

# --- Engineer: edit, save a version, publish, run. ----------------------------
# The bike assembly: an assembly with its kit, a batch oven.
# Not published yet: the list says so, and the Simulation page explains it.
b.go(url + "/")
b.wait("return document.querySelector('[data-project=packing-line]') !== null")
check(b.js("return document.querySelector('[data-project=packing-line] a[href^=simulation]')") is None, "unpublished projects can't be run yet")
b.go(url + "/simulation?project=packing-line")
b.wait("return document.body.textContent.includes(\"Packing line isn't published yet\")")
b.go(url + "/modeling?project=bike-assembly")
b.wait("return document.querySelector('.head h2')?.textContent === 'Bike assembly' && document.querySelector('.diagram svg') !== null")
click_obj("assembly")
b.wait("return document.querySelector('[data-kit=wheel]') !== null")
check(b.js("return document.querySelector('[data-kit=wheel]').value") == "2", "two wheels per kit")
check(b.js("return document.getElementById('f-kind').value") == "assembly", "assembly kind")
click_obj("oven")
b.wait("return document.getElementById('f-servers-oven-batch')?.value === '8'")
b.screenshot(f"{out}/2c-bikes.png")
b.go(url + "/")
b.wait("return document.querySelector('[data-project=two-cells]') !== null")
click_text("[data-project=two-cells] a", "Modeling")
b.wait("return document.querySelector('.head h2')?.textContent === 'Two cells' && document.querySelector('.diagram svg') !== null")
check("published v1" in b.js("return document.querySelector('.versions').textContent"), "v1 in versions")
b.screenshot(f"{out}/2-modeling.png")

click_in_diagram("conveyor c_a:")
b.wait("return document.querySelector('.panel h3')?.textContent === 'c_a'")
b.js("const i = document.getElementById('f-conveyors-c_a-speed'); i.value = '5 min'; i.dispatchEvent(new Event('change'))")
b.wait("return document.querySelector('.field.error .message') !== null", 10)
message = b.js("return document.querySelector('.field.error .message').textContent")
check("is a time, but a speed is expected" in message, f"field error: {message}")
check(b.js("return document.getElementById('save').disabled") is False, "save enabled with changes")
b.screenshot(f"{out}/3-error.png")
b.js("document.querySelector('.field.error .reset').click()")
b.wait("return document.querySelector('.field.error') === null", 10)

# Expose mill A's capacity (not written in the model: its default is added).
click_in_diagram("server mill_a")
b.wait("return document.querySelector('.panel h3')?.textContent === 'mill_a'")
b.js("document.querySelector('[data-expose=\"servers.mill_a.capacity\"]').click()")
b.wait("return document.querySelector('[data-decision-id=mill_a_capacity]') !== null")
b.js("const i = document.querySelector('[data-bound=\"mill_a_capacity.max\"]'); i.value = '2'; i.dispatchEvent(new Event('change'))")
b.wait("return document.querySelector('#simulation-preview').textContent.includes('can be 1 to 2')", 10)
check(b.js("return document.getElementById('decision-problems')") is None, "no problems with the new decision")

# Offer a faster shipping conveyor as an option instead of changing the model.
b.js("[...document.querySelectorAll('.tabs button')].find(x => x.textContent === 'Model').click()")
b.wait("return document.querySelector('.diagram svg') !== null")
click_in_diagram("conveyor c_ship:")
b.wait("return document.querySelector('.panel h3')?.textContent === 'c_ship'")
b.js("const i = document.getElementById('f-conveyors-c_ship-speed'); i.value = '0.6 m/s'; i.dispatchEvent(new Event('change'))")
b.wait("return document.getElementById('offer') !== null")
b.js("document.getElementById('offer').click()")
b.js("const s = document.getElementById('offer-decision'); s.value = ''; s.dispatchEvent(new Event('change'))")
b.wait("return document.getElementById('offer-new-label') !== null")
b.js("const i = document.getElementById('offer-new-label'); i.value = 'Shipping conveyor'; i.dispatchEvent(new Event('input'))")
b.js("const i = document.getElementById('offer-option-label'); i.value = 'Faster belt (0.6 m/s)'; i.dispatchEvent(new Event('input'))")
b.screenshot(f"{out}/4a-offer.png")
b.js("document.getElementById('offer-confirm').click()")
b.wait("return document.querySelector('[data-decision-id=shipping_conveyor]') !== null", 10)
b.wait("return document.querySelector('#simulation-preview').textContent.includes('Faster belt (0.6 m/s)')", 10)
pending = b.js("return document.getElementById('pending').textContent")
check(pending == "1 unsaved change, unsaved decision edits", f"the capacity default stays in the model: {pending}")
check(b.js("return document.getElementById('publish').disabled") is True, "can't publish with unsaved changes")
check(not b.js("return document.getElementById('content').textContent").startswith("null") and "null" not in b.js("return [...document.getElementById('content').childNodes].filter(n => n.nodeType === 3).map(n => n.textContent).join('')"), "no stray text")
b.screenshot(f"{out}/4b-managers.png")
b.js("const m = document.getElementById('save-message'); m.value = 'Mill A capacity and shipping belt as decisions'; m.dispatchEvent(new Event('input'))")
b.js("document.getElementById('save').click()")
b.wait("return document.getElementById('pending')?.textContent === 'all changes saved'", 10)
b.js("document.getElementById('publish').click()")
b.wait("return document.querySelector('.status.ok')?.textContent.startsWith('Published v2')", 10)
b.js("[...document.querySelectorAll('.tabs button')].find(x => x.textContent === 'Model').click()")
b.wait("return document.querySelector('.versions .pill')?.textContent === 'published v2'", 10)
versions = b.js("return document.querySelector('.versions').textContent")
check("Mill A capacity and shipping belt as decisions" in versions, f"saved version listed: {versions}")
b.screenshot(f"{out}/4-published.png")

b.js("document.getElementById('run').click()")
b.wait("return document.querySelector('iframe.report') !== null", 60)
time.sleep(1.5)
title = b.js("return document.querySelector('iframe.report').contentDocument.querySelector('h1')?.textContent")
check(title == "two-cells", f"report title: {title}")
downloads = b.js("return [...document.querySelectorAll('.results-bar a')].map(a => a.textContent)")
check(downloads == ["summary.json", "replications.csv", "events.jsonl", "layout.json", "model.yaml"], f"downloads: {downloads}")
b.screenshot(f"{out}/5-report.png")
click_text(".results-bar button", "Animation")
b.wait("return document.querySelector('.animation canvas') !== null", 30)
time.sleep(4)
clock = b.js("return document.querySelector('.overlay .clock').textContent")
check(clock.startswith("day 1  00:0") and clock != "day 1  00:00:00", f"animation clock: {clock}")
b.screenshot(f"{out}/6-animation.png")
click_text(".results-bar button", "3D")
b.wait("return document.body.textContent.includes('3D animation: planned')")

# --- Manager: builds scenarios from the decisions, compares them. ---------------
b.go(url + "/")
b.wait("return document.querySelector('[data-project=two-cells]') !== null")
card = b.js("return document.querySelector('[data-project=two-cells]').textContent")
check("Published v2" in card, f"the list shows v2: {card}")
click_text("[data-project=two-cells] a", "Simulation")
b.wait("return document.querySelector('.head h2')?.textContent === 'Two cells'")
check("Mill A capacity and shipping belt" in b.js("return document.getElementById('version').textContent"), "version message shown")
check(b.js("return document.querySelector('.diagram, #save, #publish')") is None, "no engineer tools on the manager page")
listed = b.js("return document.querySelector('dl.decisions').textContent")
check("Inspectors" in listed and "Shipping conveyor" in listed and "mill_a: Capacity" in listed, f"decisions listed: {listed}")
check(b.js("return document.getElementById('compare').disabled") is True, "nothing to compare yet")
b.screenshot(f"{out}/7-simulation.png")

def set_decision(decision, value):
    b.js(f"const c = document.querySelector('[data-decision={decision}]'); c.value = {value!r}; c.dispatchEvent(new Event('change'))")

b.js("document.getElementById('new-scenario').click()")
b.wait("return document.getElementById('scenario-form') !== null")
set_decision("inspectors", "2")
b.wait("return document.getElementById('scenario-name').placeholder === 'Inspectors: 2'")
b.screenshot(f"{out}/8-new-scenario.png")
b.js("document.getElementById('save-scenario').click()")
b.wait("return document.querySelectorAll('.scenario').length === 1", 10)

b.js("document.getElementById('new-scenario').click()")
b.wait("return document.getElementById('scenario-form') !== null")
set_decision("mill_b_repairs", "faster_crew")
b.js("const n = document.getElementById('scenario-name'); n.value = 'Faster repairs'; n.dispatchEvent(new Event('input'))")
b.js("document.getElementById('save-scenario').click()")
b.wait("return document.querySelectorAll('.scenario').length === 2", 10)
check(b.js("return document.querySelectorAll('.scenario input:checked').length") == 2, "new scenarios are selected")

b.js("document.getElementById('compare').click()")
b.wait("return document.querySelector('.compare-table') !== null", 180)
sentences = b.js("return [...document.querySelectorAll('.sentence')].map(x => x.textContent)")
check(len(sentences) == 6, f"three results for each of two scenarios: {sentences}")
lead = [s for s in sentences if s.startswith("Lead time: ")]
check(len(lead) == 2 and all("between" in s or "no clear change" in s for s in lead), f"lead time sentences: {lead}")
headline = b.js("return document.querySelector('.headline')?.textContent")
check(headline and "Lead time: Faster repairs does best" in headline, f"headline: {headline}")
b.screenshot(f"{out}/9-comparison.png")

click_text("#comparison .actions button", "Faster repairs")
b.wait("return document.querySelector('#details iframe.report') !== null", 10)
time.sleep(1.5)
b.screenshot(f"{out}/10-scenario-report.png")

# --- The project list: rename, archive, undo, restore, delete. -------------------
def card(pid):
    return f"document.querySelector('[data-project={pid}]')"

b.go(url + "/")
b.wait(f"return {card('packing-line')} !== null")
b.js(f"{card('packing-line')}.querySelector('.more').click()")
b.wait(f"return {card('packing-line')}.querySelector('.menu') !== null")
b.js(f"{card('packing-line')}.querySelector('[data-action=rename]').click()")
b.wait("return document.getElementById('edit-name') !== null")
set_value("edit-name", "Packing line 2", "input")
set_value("edit-description", "Cartons packed and shipped.", "input")
b.js("document.getElementById('save-details').click()")
b.wait(f"return {card('packing-line')}?.querySelector('h2')?.textContent === 'Packing line 2'")
check("Cartons packed" in b.js(f"return {card('packing-line')}.textContent"), "description shown")
b.go(url + "/modeling?project=packing-line")
b.wait("return document.getElementById('project-name').textContent === 'Packing line 2'")
check("Renamed to Packing line 2" in b.js("return document.querySelector('.versions').textContent"), "renaming is a saved version")

b.go(url + "/")
b.wait(f"return {card('packing-line')} !== null")
b.js(f"{card('packing-line')}.querySelector('.more').click()")
b.js(f"{card('packing-line')}.querySelector('[data-action=archive]').click()")
b.wait(f"return {card('packing-line')} === null && document.querySelector('.notice')?.textContent.includes('Packing line 2 was archived')")
b.js("document.getElementById('undo').click()")
b.wait(f"return {card('packing-line')} !== null")
b.js(f"{card('packing-line')}.querySelector('.more').click()")
b.js(f"{card('packing-line')}.querySelector('[data-action=archive]').click()")
b.wait("return document.getElementById('show-archived')?.textContent.includes('(1)')")
b.js("document.getElementById('show-archived').click()")
b.wait("return document.querySelector('[data-archived=packing-line]') !== null")
b.js("document.querySelector('[data-archived=packing-line] [data-action=delete]').click()")
b.wait("return document.querySelector('[data-archived=packing-line] [data-action=confirm-delete]') !== null")
b.screenshot(f"{out}/11-archive.png")
b.js("document.querySelector('[data-archived=packing-line] [data-action=confirm-delete]').click()")
b.wait("return document.querySelector('.notice')?.textContent.includes('Packing line 2 was deleted') && document.getElementById('show-archived') === null")
check(b.js(f"return {card('packing-line')}") is None, "deleted")

# --- New project from a template: published right away, a free address. ---------
from_template("bank-branch")
b.go(url + "/")
b.wait("return document.getElementById('new-project') !== null")
b.js("document.getElementById('new-project').click()")
b.wait("return document.querySelector('[data-from=\"template:bank-branch\"]') !== null")
b.js("document.querySelector('[data-from=\"template:bank-branch\"]').click()")
b.wait("return document.getElementById('new-name').placeholder === 'Bank branch'")
b.js("document.getElementById('create-project').click()")
b.wait("return location.search === '?project=bank-branch-2' && document.querySelector('.diagram svg') !== null", 10)
check("published v1" in b.js("return document.querySelector('.versions').textContent"), "templates start published")
b.go(url + "/simulation?project=bank-branch-2")
b.wait("return document.querySelector('dl.decisions')?.textContent.includes('Tellers on duty')")
b.screenshot(f"{out}/12-bank-simulation.png")
for template in ["assembly-line", "warehouse"]:
    from_template(template)
for template in ["assembly-line", "bank-branch", "warehouse"]:
    b.go(url + f"/modeling?project={template}")
    b.wait("return document.querySelector('.diagram svg') !== null && document.querySelector('.versions') !== null", 10)
    b.screenshot(f"{out}/13-{template}.png")
check(b.js("return document.getElementById('f-throughput-per').value") == "operations", "throughput per working hour of the warehouse's operations")

# --- Queues: a capacity, no length; dragging redraws them without changing the model.
b.go(url + "/modeling?project=bank-branch")
b.wait("return document.querySelector('[data-connection=teller_line]') !== null", 10)
# The bank's model calls what flows "customers".
check(b.js("return document.querySelector('#part-types .group-title').textContent") == "Customer types", "the model's words")
check(b.js("return document.getElementById('add-source').title") == "Where customers come from", "the model's words in tooltips")
# Customers arrive by time of day: the door's panel shows the periods.
click_obj("door")
b.wait("return document.getElementById('f-arrivals-mode')?.value === 'profile'")
check(b.js("return document.querySelectorAll('#profile [data-period]').length") == 3, "three periods")
check("About 310 customers a day" in b.js("return document.getElementById('profile').textContent"), "customers a day")
b.js("document.querySelector('[data-connection=teller_line]').dispatchEvent(new MouseEvent('click', {bubbles: true}))")
b.wait("return document.querySelector('.panel .kind')?.textContent === 'Queue'")
check(b.js("return document.getElementById('f-capacity').value") == "15", "queue capacity shown")
check("teller_line · 15" in b.js("return document.querySelector('.diagram svg').textContent"), "capacity on the diagram")
b.js(f"""
  const r = {obj('tellers')}, box = r.getBoundingClientRect(), px = box.width / 1.2;
  const at = (dx) => ({{ clientX: box.x + box.width / 2 + dx, clientY: box.y + box.height / 2, bubbles: true, button: 0 }});
  r.dispatchEvent(new PointerEvent('pointerdown', at(0)));
  window.dispatchEvent(new PointerEvent('pointermove', at(3 * px)));
  window.dispatchEvent(new PointerEvent('pointerup', at(3 * px)));
""")
# Only the layout changed: no model change (the queue has no length to follow the drawing).
b.wait("return document.getElementById('pending')?.textContent === 'unsaved layout'", 10)
b.js("document.querySelector('[data-connection=teller_line]').dispatchEvent(new MouseEvent('click', {bubbles: true}))")
b.wait("return document.getElementById('f-capacity') !== null")
set_value("f-capacity", "")
b.wait("return document.querySelector('.diagram svg').textContent.includes('teller_line · ∞')", 10)
b.screenshot(f"{out}/14-queues.png")
print("web UI sessions (Modeling and Simulation): all checks passed")
