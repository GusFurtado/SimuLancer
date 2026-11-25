"""Takes the documentation's screenshots: the tutorial's Packing line, built
from a blank project the way docs/tutorial.md tells it, and the conveyor
line's warm-up check.

Usage: docs_screenshots.py <server url> <marionette port> <screenshot dir> <conveyor-line report.html>
Run by tests/ui/docs-screenshots.sh, which starts the server (on a new, empty
data folder) and Firefox, then converts the screenshots for docs/images/.
"""
import base64, sys, time
from marionette import Browser

url, port, out, report = sys.argv[1].rstrip("/"), int(sys.argv[2]), sys.argv[3], sys.argv[4]
b = Browser(port)
b.call("WebDriver:SetWindowRect", {"width": 1400, "height": 900})

def set_value(element_id, value, event="change"):
    b.js(f"const i = document.getElementById({element_id!r}); i.value = {value!r}; i.dispatchEvent(new Event({event!r}))")

def obj(name):
    return f"document.querySelector('[data-object=\"{name}\"]')"

def click_obj(name):
    b.js(f"{obj(name)}.dispatchEvent(new MouseEvent('click', {{bubbles: true}}))")

def click_in_diagram(title):
    b.js(f"""[...document.querySelectorAll('.diagram .object, .diagram polyline.hit')]
             .find(r => r.querySelector('title')?.textContent.startsWith({title!r}))
             .dispatchEvent(new MouseEvent('click', {{bubbles: true}}))""")

def drag_obj(name, dx_m, dz_m=0):
    b.js(f"""
      const r = {obj(name)}, box = r.getBoundingClientRect(), px = document.querySelector('.diagram svg').getScreenCTM().a;
      const at = (dx, dz) => ({{ clientX: box.x + box.width / 2 + dx * px, clientY: box.y + box.height / 2 + dz * px, bubbles: true, button: 0 }});
      r.dispatchEvent(new PointerEvent('pointerdown', at(0, 0)));
      window.dispatchEvent(new PointerEvent('pointermove', at({dx_m / 2}, {dz_m / 2})));
      window.dispatchEvent(new PointerEvent('pointermove', at({dx_m}, {dz_m})));
      window.dispatchEvent(new PointerEvent('pointerup', at({dx_m}, {dz_m})));
    """)
    time.sleep(0.3)

def drag_connection(a, z, let_go=True):
    b.js(f"""
      const h = document.querySelector('[data-handle="{a}"]'), hb = h.getBoundingClientRect(), tb = {obj(z)}.getBoundingClientRect();
      const at = (x, y) => ({{ clientX: x, clientY: y, bubbles: true, button: 0 }});
      const [tx, ty] = [tb.x + tb.width / 2, tb.y + tb.height / 2];
      h.dispatchEvent(new PointerEvent('pointerdown', at(hb.x + hb.width / 2, hb.y + hb.height / 2)));
      window.dispatchEvent(new PointerEvent('pointermove', at(hb.x + 30, hb.y + 10)));
      window.dispatchEvent(new PointerEvent('pointermove', at(tx, ty)));
      window.dropAt = at(tx, ty);
      {"window.dispatchEvent(new PointerEvent('pointerup', at(tx, ty)));" if let_go else ""}
    """)

def ok():
    b.wait("return document.querySelector('.field.error') === null && !document.querySelector('.status:not(.ok)')", 10)

def shot(name):
    time.sleep(0.4)
    b.screenshot(f"{out}/{name}.png")

def tab(name):
    b.js(f"[...document.querySelectorAll('.tabs button')].find(x => x.textContent === {name!r}).click()")

# 1. A blank project.
b.go(url + "/")
b.wait("return document.querySelector('.creating h2') !== null")
shot("first-run")
b.wait("return document.getElementById('new-name') !== null || document.getElementById('new-project') !== null")
if b.js("return document.getElementById('new-name') === null"):
    b.js("document.getElementById('new-project').click()")
    b.wait("return document.getElementById('new-name') !== null")
set_value("new-name", "Packing line", "input")
b.js("document.getElementById('create-project').click()")
b.wait("return document.querySelector('.head h2')?.textContent === 'Packing line' && document.querySelector('.diagram svg') !== null", 10)
shot("blank")

# 2. A packer between the machine and shipping.
drag_obj("shipping", 12)
click_in_diagram("conveyor c2:")
b.wait("return document.querySelector('.panel h3')?.textContent === 'c2'")
b.js("document.getElementById('delete').click()")
b.wait("return ![...document.querySelectorAll('.diagram title')].some(t => t.textContent.startsWith('conveyor c2:'))")
b.js("""
  const s = document.querySelector('.diagram svg'), m = document.querySelector('[data-object="machine"]').getBoundingClientRect(),
        sh = document.querySelector('[data-object="shipping"]').getBoundingClientRect();
  const [cx, cy] = [(m.x + m.width / 2 + sh.x + sh.width / 2) / 2, m.y + m.height / 2];
  const src = document.getElementById('add-server'), from = src.getBoundingClientRect();
  const at = (x, y) => ({ clientX: x, clientY: y, bubbles: true, button: 0 });
  src.dispatchEvent(new PointerEvent('pointerdown', at(from.x + 10, from.y + 10)));
  window.dispatchEvent(new PointerEvent('pointermove', at(from.x + 40, from.y + 20)));
  window.dispatchEvent(new PointerEvent('pointermove', at(cx, cy)));
  window.dropAt = at(cx, cy);
""")
shot("adding")
b.js("window.dispatchEvent(new PointerEvent('pointerup', window.dropAt))")
b.wait(f"return {obj('machine_2')} !== null")
set_value("f-name", "packer")
b.wait(f"return {obj('packer')} !== null")
at = lambda n: b.js(f"return {obj(n)}.dataset.at.split(',').map(Number)")
m, p = at("machine"), at("packer")
drag_obj("packer", 0, m[1] - p[1])
drag_connection("machine", "packer")
b.wait("return document.querySelector('.connect-menu') !== null")
shot("connecting")
b.js("document.querySelector('.connect-menu [data-kind=conveyors]').click()")
b.wait("return [...document.querySelectorAll('.diagram title')].some(t => t.textContent.startsWith('conveyor c_machine_packer'))")
drag_connection("packer", "shipping")
b.wait("return document.querySelector('.connect-menu') !== null")
b.js("document.querySelector('.connect-menu [data-kind=conveyors]').click()")
b.wait("return [...document.querySelectorAll('.diagram title')].some(t => t.textContent.startsWith('conveyor c_packer_shipping'))")
click_obj("packer")
b.wait("return document.querySelector('.panel h3')?.textContent === 'packer'")
set_value("f-servers-packer-process_time-kind", "exponential")
b.wait("return document.getElementById('f-servers-packer-process_time')?.textContent === 'exponential(40 s)'")
set_value("f-servers-packer-process_time-mean", "50 s")
b.wait("return document.getElementById('f-servers-packer-process_time')?.textContent === 'exponential(50 s)'")
ok()
set_value("save-message", "Packer after the machine", "input")
shot("drawn")

# 3. Save, run, read the report and the animation.
b.js("document.getElementById('save').click()")
b.wait("return document.getElementById('pending')?.textContent === 'all changes saved'", 10)
b.js("document.getElementById('run').click()")
b.wait("return document.querySelector('iframe.report') !== null", 60)
time.sleep(1.5)
shot("report")
b.js("[...document.querySelectorAll('.results-bar button')].find(x => x.textContent.trim().startsWith('Animation')).click()")
b.wait("return document.querySelector('.animation canvas') !== null", 30)
time.sleep(5)
shot("animation")

# 4. The packer's capacity as a decision; publish.
tab("Model")
b.wait("return document.querySelector('.diagram svg') !== null")
click_obj("packer")
b.wait("return document.querySelector('.panel h3')?.textContent === 'packer'")
b.js("document.querySelector('[data-expose=\"servers.packer.capacity\"]').click()")
b.wait("return document.querySelector('[data-decision-id=packer_capacity]') !== null", 10)
b.js("const i = document.querySelector('[data-bound=\"packer_capacity.max\"]'); i.value = '3'; i.dispatchEvent(new Event('change'))")
b.wait("return document.querySelector('#simulation-preview').textContent.includes('can be 1 to 3')", 10)
b.js("""const card = document.querySelector('[data-decision-id=packer_capacity]');
  const [name, about] = card.querySelectorAll('input:not([type=number]), textarea');
  for (const [i, v] of [[name, 'Packers'], [about, 'How many boxes the packing station can work on at once.']]) {
    i.value = v; i.dispatchEvent(new Event('input')); i.dispatchEvent(new Event('change')); }""")
b.wait("return document.querySelector('#simulation-preview').textContent.includes('Packers')", 10)
shot("decisions")
set_value("save-message", "Packer capacity as a decision", "input")
b.js("document.getElementById('save').click()")
b.wait("return document.getElementById('pending')?.textContent === 'all changes saved'", 10)
b.js("document.getElementById('publish').click()")
b.wait("return document.querySelector('.status.ok')?.textContent.startsWith('Published')", 10)

# 5. Simulation: a scenario with two packers, compared with now.
b.go(url + "/simulation?project=packing-line")
b.wait("return document.querySelector('.head h2')?.textContent === 'Packing line'")
b.js("document.getElementById('new-scenario').click()")
b.wait("return document.getElementById('scenario-form') !== null")
b.js("const c = document.querySelector('[data-decision=packer_capacity]'); c.value = '2'; c.dispatchEvent(new Event('change'))")
shot("new-scenario")
b.js("document.getElementById('save-scenario').click()")
b.wait("return document.querySelectorAll('.scenario').length === 1", 10)
b.js("document.getElementById('compare').click()")
b.wait("return document.querySelector('.compare-table') !== null", 180)
b.js("document.querySelector('.compare-table').scrollIntoView({block: 'center'})")
b.js("document.querySelector('.compare-table').closest('section, div').scrollIntoView({block: 'start'})")
b.js("window.scrollBy(0, -60)")
shot("comparison")

# 6. The warm-up check of the conveyor line (docs/methods/warmup.md).
b.call("WebDriver:SetWindowRect", {"width": 1000, "height": 900})
b.go("file://" + report)
b.wait("return [...document.querySelectorAll('h2')].some(h => h.textContent === 'Warm-up check')")
section = b.call("WebDriver:FindElement", {"using": "xpath", "value": "//h2[text()='Warm-up check']/.."})["value"]
data = b.call("WebDriver:TakeScreenshot", {"id": list(section.values())[0], "full": False})["value"]
open(f"{out}/warmup-check.png", "wb").write(base64.b64decode(data))
