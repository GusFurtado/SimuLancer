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

click_in_diagram("source arrivals")
b.wait("return document.querySelector('.panel h3')?.textContent === 'arrivals'")
b.js("const s = document.getElementById('f-sources.arrivals.output-rule'); s.value = 'round_robin'; s.dispatchEvent(new Event('change'))")
b.wait("return document.querySelector('.actions .hint').textContent.includes('1 unsaved change')")
check(b.js("return document.getElementById('publish').disabled") is True, "can't publish with unsaved changes")
b.js("const m = document.getElementById('save-message'); m.value = 'Alternate cells at the source'; m.dispatchEvent(new Event('input'))")
b.js("document.getElementById('save').click()")
b.wait("return document.querySelector('.actions .hint')?.textContent === 'all changes saved'", 10)
versions = b.js("return document.querySelector('.versions').textContent")
check("Alternate cells at the source" in versions, f"saved version listed: {versions}")
b.js("document.getElementById('publish').click()")
b.wait("return document.querySelector('.versions .pill')?.textContent === 'published v2'", 10)
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

# --- Manager: sees the published version, runs it. ------------------------------
b.go(url + "/decide")
b.wait("return document.querySelectorAll('#projects button').length >= 2")
listed = b.js("return [...document.querySelectorAll('#projects button')].map(x => x.textContent)")
check(any(p.startswith("Two cells") and "v2" in p for p in listed), f"manager sees v2: {listed}")
click_text("#projects button", "Two cells")
b.wait("return document.querySelector('.head h2')?.textContent === 'Two cells'")
check("Alternate cells at the source" in b.js("return document.querySelector('main').textContent"), "version message shown")
check(b.js("return document.querySelector('.diagram, #save, #publish')") is None, "no engineer tools on the manager page")
b.screenshot(f"{out}/7-decide.png")
b.js("document.getElementById('run').click()")
b.wait("return document.querySelector('iframe.report') !== null", 60)
time.sleep(1.5)
b.screenshot(f"{out}/8-decide-report.png")
print("web UI sessions (engineer and manager): all checks passed")
