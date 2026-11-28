"""Drives the web pages through a Modeling session and a Simulation session in
Firefox (Marionette).

Usage: test_ui.py <server url> <marionette port> <screenshot dir>
Run by tests/ui/run.sh, which starts the server (on a new, empty data
folder) and Firefox.
"""
import json, os, re, sys, time, urllib.request
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
    b.js(f"""[...document.querySelectorAll('.diagram .object, .diagram polyline.hit')]
             .find(r => r.querySelector('title')?.textContent.startsWith({title!r}))
             .dispatchEvent(new MouseEvent('click', {{bubbles: true}}))""")

def from_template(template, id=None):
    """Creates a project from a template through the API, like the New project form."""
    request = urllib.request.Request(url + "/api/v1/projects", method="POST",
        data=json.dumps({"id": id or template, "template": template}).encode(), headers={"content-type": "application/json"})
    urllib.request.urlopen(request).close()

def leaving_asks():
    """Whether leaving the page now would ask first (the page cancels beforeunload)."""
    return b.js("const e = new Event('beforeunload', {cancelable: true}); window.dispatchEvent(e); return e.defaultPrevented")

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
    return f"document.querySelector('[data-object=\"{name}\"]')"

def click_obj(name):
    b.js(f"{obj(name)}.dispatchEvent(new MouseEvent('click', {{bubbles: true}}))")

def define(item):
    """Opens a definition (part types, resources, schedules...) from the bar on the left."""
    if b.js("return document.getElementById('side-definitions').hidden"):
        b.js("document.getElementById('show-definitions').click()")
    b.wait(f"return document.getElementById('define-{item}')?.disabled === false", 10)
    b.js(f"document.getElementById('define-{item}').click()")

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
  const r = {obj('packer')}, box = r.getBoundingClientRect(), px = document.querySelector('.diagram svg').getScreenCTM().a;
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

# Drag a sink from the Components panel onto the diagram: it lands where it was dropped.
def drag_new(button):
    """Presses `button` and moves to a point on the diagram, without letting go.
    Returns that point in meters, snapped, and whether the drop target shows."""
    return b.js(f"""
      const s = document.querySelector('.diagram svg'), box = s.getBoundingClientRect();
      const [cx, cy] = [box.x + box.width * 0.3, box.y + box.height * 0.8];
      const pt = s.createSVGPoint(); pt.x = cx; pt.y = cy;
      const p = pt.matrixTransform(s.getScreenCTM().inverse());
      const src = document.getElementById({button!r}), from = src.getBoundingClientRect();
      const at = (x, y) => ({{ clientX: x, clientY: y, bubbles: true, button: 0 }});
      src.dispatchEvent(new PointerEvent('pointerdown', at(from.x + 10, from.y + 10)));
      window.dispatchEvent(new PointerEvent('pointermove', at(from.x + 40, from.y + 20)));
      window.dispatchEvent(new PointerEvent('pointermove', at(cx, cy)));
      window.dropAt = at(cx, cy);
      return [Math.round(p.x * 2) / 2, Math.round(p.y * 2) / 2,
              document.querySelector('.diagram.drop-target') !== null && document.querySelector('.add-ghost') !== null];
    """)

def let_go():
    b.js("window.dispatchEvent(new PointerEvent('pointerup', window.dropAt))")

x, z, lit = drag_new("add-sink")
check(lit, "the diagram lights up and a preview follows the pointer")
b.screenshot(f"{out}/2a-adding.png")
let_go()
b.wait(f"return {obj('exit')} !== null")
r = b.js(f"return {obj('exit')}.dataset.at.split(',').map(Number)")
check([round(v, 2) for v in r] == [x, z], f"dropped where the pointer was: {r} vs {[x, z]}")
check(b.js("return document.querySelector('.add-ghost') === null && document.querySelector('.drop-target') === null"), "the preview goes away")
check(b.js("return document.querySelector('.panel h3')?.textContent") == "exit", "the new object is selected")
b.js("document.getElementById('undo').click()")
b.wait(f"return {obj('exit')} === null")

drag_new("add-sink")
b.js("document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))")
let_go()
time.sleep(0.3)
check(b.js(f"return {obj('exit')} === null && document.querySelector('.add-ghost') === null"), "Esc cancels the drag")

# Drag from an object's handle to the next one, then pick how they connect.
def drag_connection(a, z, then=""):
    """Drags from `a`'s handle onto `z` (or onto empty space if z is None) and lets go.
    Returns whether a dashed preview showed and `z` was marked as the target."""
    to = f"{obj(z)}.getBoundingClientRect()" if z else "(() => { const b = document.querySelector('.diagram svg').getBoundingClientRect(); return { x: b.x + 4, y: b.bottom - 14, width: 0, height: 0 }; })()"
    return b.js(f"""
      const h = document.querySelector('[data-handle="{a}"]'), hb = h.getBoundingClientRect(), tb = {to};
      const at = (x, y) => ({{ clientX: x, clientY: y, bubbles: true, button: 0 }});
      const [tx, ty] = [tb.x + tb.width / 2, tb.y + tb.height / 2];
      h.dispatchEvent(new PointerEvent('pointerdown', at(hb.x + hb.width / 2, hb.y + hb.height / 2)));
      window.dispatchEvent(new PointerEvent('pointermove', at(hb.x + 30, hb.y + 10)));
      window.dispatchEvent(new PointerEvent('pointermove', at(tx, ty)));
      const shown = document.querySelector('.connect-preview') !== null && {f"{obj(z)}.parentElement.classList.contains('target')" if z else "true"};
      {then}
      window.dispatchEvent(new PointerEvent('pointerup', at(tx, ty)));
      return shown && document.querySelector('.connect-preview') === null && document.querySelector('.diagram .target') === null;
    """)

menu = "document.querySelector('.connect-menu')"
check(drag_connection("arrivals", "packer"), "a dashed line follows the pointer and the target is marked")
b.wait(f"return {menu} !== null")
check(b.js(f"return {menu}.querySelector('.title').textContent") == "arrivals → packer", "the menu says what it connects")
check(b.js(f"return {menu}.querySelector('[data-kind=transports]').disabled"), "no transport without vehicles")
b.screenshot(f"{out}/2a-connecting.png")
b.js("document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))")
check(b.js(f"return {menu} === null"), "Esc closes the menu")
check(not b.js("return [...document.querySelectorAll('.diagram title')].some(t => t.textContent.startsWith('queue q_arrivals_packer'))"), "and connects nothing")

drag_connection("arrivals", "packer")
b.wait(f"return {menu} !== null")
b.js(f"{menu}.querySelector('[data-kind=queues]').click()")
b.wait("return [...document.querySelectorAll('.diagram title')].some(t => t.textContent === 'queue q_arrivals_packer: arrivals → packer')")
check(b.js(f"return {menu} === null && document.querySelector('.panel h3')?.textContent") == "q_arrivals_packer", "the new queue is selected")
b.js("document.getElementById('undo').click()")
b.wait("return ![...document.querySelectorAll('.diagram title')].some(t => t.textContent.startsWith('queue q_arrivals_packer'))")

drag_connection("machine", "arrivals")
drag_connection("machine", None)
time.sleep(0.2)
check(b.js(f"return {menu} === null"), "no menu over a source or empty space")
check(b.js("return document.querySelector('[data-handle=\"shipping\"]') === null"), "sinks have no handle")

# Sources and sinks look different; a selected server shows a ring apart from its own color.
check(b.js(f"return [{obj('arrivals')}.dataset.model, {obj('machine')}.dataset.model, {obj('shipping')}.dataset.model]") == ["dock", "machine", "dock"], "each kind of object has its default look")
click_obj("machine")
b.wait("return document.querySelector('.panel h3')?.textContent === 'machine'")
ring = b.js(f"const g = {obj('machine')}.parentElement, h = getComputedStyle(g.querySelector('.halo')); return [g.classList.contains('selected'), h.stroke, getComputedStyle({obj('machine')}).stroke]")
check(ring[0] and ring[1] not in ("none", "") and ring[2] in ("none", ""), f"selection is a ring around the server, not its outline: {ring}")

# What an object is drawn as is a layout change, offered for its kind and undone like any other.
check(b.js("return [...document.getElementById('f-drawn').options].map(o => o.value)") == ["machine", "lathe", "mill", "oven", "workstation", "counter", "kiosk"], "server looks")
set_value("f-drawn", "lathe")
b.wait(f"return {obj('machine')}.dataset.model === 'lathe'")
check(b.js("return document.getElementById('f-drawn').value") == "lathe", "the panel shows the new look")
b.js("document.getElementById('undo').click()")
b.wait(f"return {obj('machine')}.dataset.model === 'machine'")

click_obj("packer")
b.wait("return document.querySelector('.panel h3')?.textContent === 'packer'")
set_value("f-servers-packer-process_time-kind", "exponential")
b.wait("return document.getElementById('f-servers-packer-process_time')?.textContent === 'exponential(40 s)'")
# Fit from data: the button opens the system's file dialog, which a test can't
# drive, so hand the dialog a file's contents directly.
check(b.js("return document.getElementById('f-servers-packer-process_time-fit') !== null"), "the picker offers a fit")
times = [38, 41, 36, 44, 39, 52, 40, 37, 43, 47, 35, 42, 39, 45, 41, 38, 50, 40, 36, 44, 42, 39, 46, 41, 37, 43, 40, 48, 38, 41]
csv = "order;seconds\n" + "\n".join(f"{i + 1};{t}" for i, t in enumerate(times))
b.js(f"fitDialog('Process time', 'servers.packer.process_time', 'packing.csv', {csv!r})")
b.wait("return document.getElementById('fit-column') !== null")
set_value("fit-column", "seconds")
b.wait("return document.querySelectorAll('.fit-table tbody tr').length >= 6")
set_value("fit-unit", "s")
b.wait("return document.querySelector('.fit-table code')?.textContent.endsWith(' s)')")
check("30 values" in b.js("return document.querySelector('.fit-dialog').textContent"), "the dialog sums up the data")
b.screenshot(f"{out}/2a-fit.png")
best = b.js("return document.querySelector('.fit-table code').textContent")
b.js("document.querySelector('.fit-table button.primary').click()")
b.wait(f"return document.getElementById('f-servers-packer-process_time')?.textContent === {best!r}")
check(b.js("return document.querySelector('.fit-dialog') === null"), "using a fit closes the dialog")
b.js("document.getElementById('undo').click()")
b.wait("return document.getElementById('f-servers-packer-process_time')?.textContent === 'exponential(40 s)'")
b.wait("return document.querySelector('.field.error') === null && !document.querySelector('.status:not(.ok)')", 10)
b.screenshot(f"{out}/2a-drawn.png")
set_value("save-message", "Packer after the machine", "input")
b.js("document.getElementById('save').click()")
b.wait("return document.getElementById('pending')?.textContent === 'all changes saved'", 10)
check(b.js(f"return {obj('packer')} !== null"), "the drawing is saved with the layout")
check("Packer after the machine" in b.js("return document.querySelector('.versions').textContent"), "saved as a version")

# Part types: add one, mix it at the source, time it separately at the packer.
define("part-types")
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
define("resources")
b.wait("return document.getElementById('add-resource') !== null")
b.js("document.getElementById('add-resource').click()")
b.wait("return document.querySelector('[data-resource=operator]') !== null")
check(b.js("return document.querySelector('.panel h3').textContent") == "Resources", "a definition opens in the panel")
check(b.js("return document.getElementById('define-resources').textContent") == "Resourcesoperator", "definitions list what the model has")
check(b.js("return document.getElementById('delete').disabled"), "a definition isn't deleted from the toolbar")
b.screenshot(f"{out}/2d-definitions.png")
for machine in ["machine", "packer"]:
    click_obj(machine)
    b.wait(f"return document.querySelector('.panel h3')?.textContent === '{machine}' && document.querySelector('[data-need=operator]') !== null")
    b.js("const i = document.querySelector('[data-need=operator]'); i.value = '1'; i.dispatchEvent(new Event('change'))")
    b.wait(f"return document.querySelector('[data-need=operator]')?.value === '1'")
define("resources")
b.wait("return document.querySelector('[data-expose=\"resources.operator.capacity\"]') !== null")
b.js("document.querySelector('[data-expose=\"resources.operator.capacity\"]').click()")
b.wait("return document.querySelector('[data-decision-id=operator_units_available]') !== null", 10)
b.js("[...document.querySelectorAll('.tabs button')].find(x => x.textContent === 'Model').click()")
b.wait("return document.querySelector('.diagram svg') !== null")
b.wait("return document.querySelector('.field.error') === null && !document.querySelector('.status:not(.ok)')", 10)
# A day shift for the packer.
define("schedules")
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
check(b.js("return document.querySelector('.panel h3').textContent") == "packer", "saving keeps what's open")
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
define("experiment")
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
check(b.js("return [...document.querySelectorAll('.results-bar button')].map(x => x.textContent)") == ["Report", "Animation"], "results: report and animation, no 3D placeholder")

# --- Recent runs: the trash deletes the open run and its results. ---------------
if b.js("return document.getElementById('side-runs').hidden"):
    b.js("document.getElementById('show-runs').click()")
runs_before = b.js("return document.querySelectorAll('#runs .run-item').length")
when = b.js("return document.querySelector('#runs .run-item .sub').textContent")
check(not re.fullmatch(r"\d+-\d+", when) and str(time.localtime().tm_year) in when, f"runs show when they were asked for: {when}")
b.js("window.confirm = () => true; document.querySelector('#runs .run-item button.current + .trash').click()")
b.wait(f"return document.querySelectorAll('#runs .run-item').length === {runs_before - 1}")
check(b.js("return document.querySelector('iframe.report') === null"), "deleting the open run closes its results")
b.js("document.getElementById('show-components').click()")  # the page remembers the open panel

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
b.go(url + "/modeling?project=assembly-line")
b.wait(f"return {obj('pack')} !== null", 10)
click_obj("pack")
b.wait("return document.getElementById('f-servers-pack-changeover')?.value === '1.5 min'")
# Taking a resource no server gives back is an error at that field.
click_obj("frame")
b.wait("return document.querySelector('[data-take=technician]') !== null")
b.js("const i = document.querySelector('[data-take=technician]'); i.value = '1'; i.dispatchEvent(new Event('change'))")
b.wait("return [...document.querySelectorAll('.field.error .message')].some(m => m.textContent.includes('still carrying it'))", 10)
b.js("document.querySelector('[data-take=technician]').closest('.field').querySelector('.reset').click()")
b.wait("return document.getElementById('pending')?.textContent === 'all changes saved'", 10)
for template in ["assembly-line", "bank-branch", "warehouse"]:
    b.go(url + f"/modeling?project={template}")
    b.wait("return document.querySelector('.diagram svg') !== null && document.querySelector('.versions') !== null", 10)
    b.screenshot(f"{out}/13-{template}.png")
check(b.js("return document.getElementById('f-throughput-per').value") == "operations", "throughput per working hour of the warehouse's operations")
check(b.js("return document.querySelector('.panel h3').textContent") == "Experiment", "the page opens on the experiment")
click_obj("trucks")
b.wait("return document.querySelector('.panel h3')?.textContent === 'trucks'")
b.js("document.querySelector('.diagram svg').dispatchEvent(new MouseEvent('click', {bubbles: true}))")
time.sleep(0.3)
check(b.js("return document.querySelector('.panel h3').textContent") == "trucks", "clicking empty space keeps what's open")
click_obj("trucks")
b.wait("return document.getElementById('f-arrivals-mode')?.value === 'restock'")
check(b.js("return document.getElementById('f-sources-trucks-reorder_point').value") == "250", "reorder point")
click_obj("pick")
b.wait("return document.getElementById('f-keeps-clock')?.value === 'order'")
# Forklifts carry pallets along a transport.
b.js("document.querySelector('[data-connection=to_racks]').dispatchEvent(new MouseEvent('click', {bubbles: true}))")
b.wait("return document.querySelector('.panel .kind')?.textContent === 'Transport'")
check(b.js("return document.getElementById('f-transports-to_racks-by').value") == "forklifts", "carried by forklifts")
check("to_racks · forklifts, 60 m" in b.js("return document.querySelector('.diagram svg').textContent"), "transport on the diagram")
# The three forklifts are parked at their home on the diagram; clicking them
# opens the Vehicles definition, where their look is chosen (a layout change).
parked = "return [document.querySelector('[data-vehicles=forklifts]')?.dataset.model, document.querySelectorAll('[data-vehicles=forklifts] > g.art').length]"
check(b.js(parked) == ["forklift", 3], f"forklifts parked on the diagram: {b.js(parked)}")
b.js("document.querySelector('[data-vehicles=forklifts]').dispatchEvent(new MouseEvent('click', {bubbles: true}))")
b.wait("return document.querySelector('[data-fleet-look=forklifts]') !== null")
check(b.js("return [...document.querySelector('[data-fleet-look=forklifts]').options].map(o => o.value)") == ["forklift", "agv", "pallet_jack", "tugger", "truck"], "vehicle looks")
check(b.js("return document.querySelector('[data-fleet-look=forklifts]').value") == "forklift", "the warehouse's forklifts")
b.js("const s = document.querySelector('[data-fleet-look=forklifts]'); s.value = 'tugger'; s.dispatchEvent(new Event('change'))")
b.wait("return document.querySelector('[data-fleet-look=forklifts]')?.value === 'tugger' && document.getElementById('undo').disabled === false")
check(b.js(parked) == ["tugger", 3], "the diagram follows the look")
b.js("document.getElementById('undo').click()")
b.wait("return document.querySelector('[data-fleet-look=forklifts]')?.value === 'forklift'")
click_obj("break_pallet")
b.wait("return document.getElementById('f-split-type')?.value === 'carton' && document.getElementById('f-split-count').value === '10'")

# --- Queues: a capacity, no length; dragging redraws them without changing the model.
b.go(url + "/modeling?project=bank-branch")
b.wait("return document.querySelector('[data-connection=teller_line]') !== null", 10)
# A server with capacity 5 is five counters, without "×5" in its name.
units = "return document.querySelector('[data-object=%s]').parentNode.querySelectorAll(':scope > g.art > g.art').length"
check(b.js(units % "tellers") == 5 and b.js(units % "advisors") == 3, "servers drawn as their units")
check(b.js("return [...document.querySelectorAll('.diagram svg text')].map(t => t.textContent).filter(t => t.startsWith('tellers'))") == ["tellers"], "no ×N where units are drawn")
# Tellers and advisors are drawn as clerks where they wait; clicking them opens
# Resources, where "Not drawn" takes them off (a layout change, undoable).
crew = "return [...document.querySelectorAll('[data-crew]')].map(g => [g.dataset.crew, g.dataset.model, g.querySelectorAll(':scope > g.art').length])"
check(b.js(crew) == [["teller", "clerk", 3], ["advisor", "clerk", 2]], f"resources drawn where they wait: {b.js(crew)}")
b.js("document.querySelector('[data-crew=teller]').dispatchEvent(new MouseEvent('click', {bubbles: true}))")
b.wait("return document.querySelector('[data-crew-look=teller]') !== null")
check(b.js("return [...document.querySelector('[data-crew-look=teller]').options].map(o => o.value)") == ["", "worker", "clerk"], "resource looks")
b.js("const s = document.querySelector('[data-crew-look=teller]'); s.value = ''; s.dispatchEvent(new Event('change'))")
b.wait("return document.querySelector('[data-crew=teller]') === null && document.getElementById('undo').disabled === false")
b.js("document.getElementById('undo').click()")
b.wait("return document.querySelector('[data-crew=teller]') !== null && document.querySelector('[data-crew-look=teller]')?.value === 'clerk'")
# The bank's model calls what flows "customers".
define("part-types")
b.wait("return document.getElementById('part-types') !== null")
check(b.js("return document.getElementById('define-part-types').textContent").startswith("Customer types"), "the model's words in definitions")
check(b.js("return document.querySelector('#part-types .group-title').textContent") == "Customer types", "the model's words")
check(b.js("return document.getElementById('add-source').title") == "Where customers come from", "the model's words in tooltips")
# Customers are drawn as people, a layout change chosen with the type.
check(b.js("return [...document.querySelectorAll('[data-type-look]')].map(s => s.value)") == ["person", "person", "person"], "customers drawn as people")
check(b.js("return [...document.querySelector('[data-type-look=quick]').options].map(o => o.value)") == ["box", "pallet", "person"], "part looks")
b.js("const s = document.querySelector('[data-type-look=quick]'); s.value = 'box'; s.dispatchEvent(new Event('change'))")
b.wait("return document.querySelector('[data-type-look=quick]')?.value === 'box' && document.getElementById('undo').disabled === false")
b.js("document.getElementById('undo').click()")
b.wait("return document.querySelector('[data-type-look=quick]')?.value === 'person'")
# The staff finish serving after closing.
define("schedules")
b.wait("return document.querySelector('[data-finish=staff_hours]') !== null")
check(b.js("return document.querySelector('[data-finish=staff_hours]').checked") is True, "finish work")
# Customers arrive by time of day: the door's panel shows the periods.
click_obj("door")
b.wait("return document.getElementById('f-arrivals-mode')?.value === 'profile'")
check(b.js("return document.querySelectorAll('#profile [data-period]').length") == 3, "three periods")
check("About 310 customers a day" in b.js("return document.getElementById('profile').textContent"), "customers a day")
b.js("document.querySelector('[data-connection=teller_line]').dispatchEvent(new MouseEvent('click', {bubbles: true}))")
b.wait("return document.querySelector('.panel .kind')?.textContent === 'Queue'")
check(b.js("return document.getElementById('f-capacity').value") == "15", "queue capacity shown")
check(b.js("return document.getElementById('f-queues-teller_line-patience').value") == "exponential(20 min)", "patience shown")
check("teller_line · 15" in b.js("return document.querySelector('.diagram svg').textContent"), "capacity on the diagram")
b.js(f"""
  const r = {obj('tellers')}, box = r.getBoundingClientRect(), px = document.querySelector('.diagram svg').getScreenCTM().a;
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

# --- Documentation: README.md, read rendered, edited with a preview, saved as a version.
b.go(url + "/modeling?project=bank-branch")
b.wait("return document.getElementById('tab-documentation') !== null", 10)
b.js("document.getElementById('tab-documentation').click()")
b.wait("return document.querySelector('#doc-view h1')?.textContent === 'Bank branch'", 10)
b.js("document.getElementById('doc-edit').click()")
b.wait("return document.activeElement?.id === 'doc-text'")
set_value("doc-text", "# Bank branch\n\n## Business rules\n\n- Advisors see **booked** customers first. <script>bad()</script>\n", "input")
b.wait("return document.querySelector('#doc-preview strong')?.textContent === 'booked'", 10)
check(b.js("return document.querySelector('#doc-preview script')") is None, "HTML in the documentation is text")
check(b.js("return document.activeElement.id") == "doc-text", "typing keeps the focus")
check(b.js("return document.getElementById('pending').textContent") == "unsaved documentation", "documentation is a pending change")
check(b.js("return document.getElementById('save').classList.contains('unsaved')"), "Save version stands out")
check(leaving_asks(), "leaving with unsaved changes asks first")
b.screenshot(f"{out}/15-documentation.png")
set_value("save-message", "Explained the advisors", "input")
b.js("document.getElementById('save').click()")
b.wait("return document.getElementById('pending')?.textContent === 'all changes saved'", 10)
check(b.js("return document.querySelector('.tabs .current').textContent") == "Documentation", "stays on the tab after saving")
check(not leaving_asks(), "nothing to ask once saved")
check(not b.js("return document.getElementById('save').classList.contains('unsaved')"), "Save version back to normal")

# --- Side panels: each icon opens its panel, and clicking it again closes it.
b.js("document.getElementById('show-runs').click()")
check(b.js("return !document.getElementById('side').hidden && !document.getElementById('side-runs').hidden"), "recent runs open")
check(b.js("return document.getElementById('side-components').hidden"), "one panel at a time")
b.js("document.getElementById('show-runs').click()")
check(b.js("return document.getElementById('side').hidden"), "clicking the open panel's icon closes it")
b.js("document.getElementById('show-components').click()")
check(b.js("return !document.getElementById('side-components').hidden"), "components open")
check(b.js("return document.querySelector('#doc-view h2').textContent") == "Business rules", "saved documentation shown")
# --- Attributes: declared in the model settings, set with formulas checked as you type.
b.go(url + "/modeling?project=bank-branch")
define("attributes")
b.wait("return document.getElementById('add-attribute') !== null", 10)
b.js("document.getElementById('add-attribute').click()")
b.wait("return document.querySelector('[data-attribute=attribute_1]') !== null")
b.js("const i = document.querySelector('[data-attribute=attribute_1]'); i.value = 'due'; i.dispatchEvent(new Event('change'))")
b.wait("return document.getElementById('f-attributes-due-default') !== null")
set_value("f-attributes-due-default", "0 s")
click_obj("door")
b.wait("return document.getElementById('add-set') !== null")
b.js("document.getElementById('add-set').click()")
b.wait("return document.querySelector('[data-set=due]') !== null")
b.js("const i = document.querySelector('[data-set=due]'); i.value = 'now + 4'; i.dispatchEvent(new Event('change'))")
b.wait("return document.querySelector('#sets .message')?.textContent.includes(\"can't add a time and a plain number\")", 10)
b.js("const i = document.querySelector('[data-set=due]'); i.value = 'now + 4 h'; i.dispatchEvent(new Event('change'))")
b.wait("return document.querySelector('#sets') !== null && document.querySelector('#sets .message') === null && document.getElementById('save') && !document.getElementById('save').disabled", 10)
b.screenshot(f"{out}/16-attributes.png")
set_value("save-message", "Due dates", "input")
b.js("document.getElementById('save').click()")
b.wait("return document.getElementById('pending')?.textContent === 'all changes saved'", 10)
model = json.load(urllib.request.urlopen(url + "/api/v1/projects/bank-branch"))["tree"]
check(model["attributes"] == {"due": {"default": "0 s"}}, f"attribute saved: {model.get('attributes')}")
check(model["sources"]["door"]["sets"] == {"due": "now + 4 h"}, f"sets saved: {model['sources']['door'].get('sets')}")
# The teller line serves the earliest due date first.
b.js("document.querySelector('[data-connection=teller_line]').dispatchEvent(new MouseEvent('click', {bubbles: true}))")
b.wait("return document.getElementById('f-queue-order') !== null")
check("Lowest due first" in b.js("return [...document.querySelectorAll('#f-queue-order option')].map(o => o.textContent)"), "attributes as queue orders")
set_value("f-queue-order", "lowest:due")
b.wait("return document.getElementById('pending')?.textContent === '1 unsaved change'", 10)
set_value("save-message", "Earliest due first", "input")
b.js("document.getElementById('save').click()")
b.wait("return document.getElementById('pending')?.textContent === 'all changes saved'", 10)
model = json.load(urllib.request.urlopen(url + "/api/v1/projects/bank-branch"))["tree"]
check(model["queues"]["teller_line"]["order"] == {"lowest": "due"}, f"order saved: {model['queues']['teller_line'].get('order')}")
# The kiosk sends customers by condition; a typo in a condition is shown on its route.
click_obj("kiosk")
b.wait("return document.getElementById('f-servers.kiosk.output-rule') !== null")
set_value("f-servers.kiosk.output-rule", "by_condition")
b.wait("return document.querySelector('[data-when=\"0\"]') !== null")
b.js("const i = document.querySelector('[data-when=\"0\"]'); i.value = 'due < nwo'; i.dispatchEvent(new Event('change'))")
b.wait("return document.querySelector('.route .message')?.textContent.includes(\"`nwo` in `due < nwo` isn't an attribute\")", 10)
b.js("const i = document.querySelector('[data-when=\"0\"]'); i.value = 'due < now'; i.dispatchEvent(new Event('change'))")
b.wait("return document.querySelector('[data-when]') !== null && document.querySelector('.route .message') === null", 10)
b.screenshot(f"{out}/17-routes-by-condition.png")
print("web UI sessions (Modeling and Simulation): all checks passed")
