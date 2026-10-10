"""Drives the web pages through an engineer's and a manager's session in
Firefox (Marionette).

Usage: test_ui.py <server url> <marionette port> <screenshot dir>
Run by tests/ui/run.sh, which starts the server (with the example projects)
and Firefox.
"""
import os, sys, time
from marionette import Browser

url, port, out = sys.argv[1].rstrip("/"), int(sys.argv[2]), sys.argv[3]
os.makedirs(out, exist_ok=True)

def check(condition, what):
    if not condition:
        raise AssertionError(what)

def click_text(selector, text):
    b.js(f"[...document.querySelectorAll({selector!r})].find(x => x.textContent.trim().startsWith({text!r})).click()")

def click_in_diagram(title):
    b.js(f"""[...document.querySelectorAll('.diagram rect, .diagram polyline.hit')]
             .find(r => r.querySelector('title')?.textContent.startsWith({title!r}))
             .dispatchEvent(new MouseEvent('click', {{bubbles: true}}))""")

b = Browser(port)
b.call("WebDriver:SetWindowRect", {"width": 1400, "height": 950})

# --- The landing page: three doors. -------------------------------------------
b.go(url + "/")
b.wait("return document.querySelectorAll('a.door').length === 3")
b.screenshot(f"{out}/1-landing.png")

# --- Engineer: edit, save a version, publish, run. ----------------------------
b.go(url + "/build")
b.wait("return document.querySelectorAll('#projects button').length >= 2")
click_text("#projects button", "Two cells")
b.wait("return document.querySelector('.diagram svg') !== null")
check("published v1" in b.js("return document.querySelector('.versions').textContent"), "v1 in versions")
b.screenshot(f"{out}/2-build.png")

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
b.wait("return document.querySelector('#manager-preview').textContent.includes('can be 1 to 2')", 10)
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
b.wait("return document.querySelector('#manager-preview').textContent.includes('Faster belt (0.6 m/s)')", 10)
pending = b.js("return document.getElementById('pending').textContent")
check(pending == "1 unsaved change, unsaved edits for managers", f"the capacity default stays in the model: {pending}")
check(b.js("return document.getElementById('publish').disabled") is True, "can't publish with unsaved changes")
check(not b.js("return document.getElementById('content').textContent").startswith("null") and "null" not in b.js("return [...document.getElementById('content').childNodes].filter(n => n.nodeType === 3).map(n => n.textContent).join('')"), "no stray text")
b.screenshot(f"{out}/4b-managers.png")
b.js("const m = document.getElementById('save-message'); m.value = 'Mill A capacity and shipping belt for managers'; m.dispatchEvent(new Event('input'))")
b.js("document.getElementById('save').click()")
b.wait("return document.getElementById('pending')?.textContent === 'all changes saved'", 10)
b.js("document.getElementById('publish').click()")
b.wait("return document.querySelector('.status.ok')?.textContent.startsWith('Published v2')", 10)
b.js("[...document.querySelectorAll('.tabs button')].find(x => x.textContent === 'Model').click()")
b.wait("return document.querySelector('.versions .pill')?.textContent === 'published v2'", 10)
versions = b.js("return document.querySelector('.versions').textContent")
check("Mill A capacity and shipping belt for managers" in versions, f"saved version listed: {versions}")
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
b.go(url + "/decide")
b.wait("return document.querySelectorAll('#projects button').length >= 2")
listed = b.js("return [...document.querySelectorAll('#projects button')].map(x => x.textContent)")
check(any(p.startswith("Two cells") and "v2" in p for p in listed), f"manager sees v2: {listed}")
click_text("#projects button", "Two cells")
b.wait("return document.querySelector('.head h2')?.textContent === 'Two cells'")
check("Mill A capacity and shipping belt" in b.js("return document.getElementById('version').textContent"), "version message shown")
check(b.js("return document.querySelector('.diagram, #save, #publish')") is None, "no engineer tools on the manager page")
listed = b.js("return document.querySelector('dl.decisions').textContent")
check("Inspectors" in listed and "Shipping conveyor" in listed and "mill_a: Capacity" in listed, f"decisions listed: {listed}")
check(b.js("return document.getElementById('compare').disabled") is True, "nothing to compare yet")
b.screenshot(f"{out}/7-decide.png")

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
print("web UI sessions (engineer and manager): all checks passed")
