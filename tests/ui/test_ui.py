"""Drives the web interface through a user session in Firefox (Marionette).

Usage: test_ui.py <server url> <marionette port> <screenshot dir>
Run by tests/ui/run.sh, which starts the server and Firefox.
"""
import os, sys, time
from marionette import Browser

url, port, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
os.makedirs(out, exist_ok=True)

def check(condition, what):
    if not condition:
        raise AssertionError(what)

b = Browser(port)
b.call("WebDriver:SetWindowRect", {"width": 1400, "height": 950})
b.go(url)
b.wait("return document.querySelectorAll('#models button').length > 0")
models = b.js("return [...document.querySelectorAll('#models button')].map(x => x.textContent)")
check(any(m.startswith("two-cells-round-robin") and "scenario of two-cells.yaml" in m for m in models), f"models listed: {models}")
b.screenshot(f"{out}/1-home.png")

b.js("[...document.querySelectorAll('#models button')].find(x => x.textContent.startsWith('two-cells') && x.textContent.includes('model')).click()")
b.wait("return document.querySelector('.diagram svg') !== null")
b.screenshot(f"{out}/2-editor.png")

# Select the inspect server in the diagram.
b.js("""[...document.querySelectorAll('.diagram rect')].find(r => r.querySelector('title')?.textContent === 'server inspect')
       .dispatchEvent(new MouseEvent('click', {bubbles: true}))""")
b.wait("return document.querySelector('.panel h3')?.textContent === 'inspect'")
b.screenshot(f"{out}/3-inspect.png")

# A bad value on a conveyor: the error must show on the field.
b.js("""[...document.querySelectorAll('.diagram polyline.hit')].find(p => p.querySelector('title')?.textContent.startsWith('conveyor c_a:'))
       .dispatchEvent(new MouseEvent('click', {bubbles: true}))""")
b.wait("return document.querySelector('.panel h3')?.textContent === 'c_a'")
b.js("const i = document.getElementById('f-conveyors-c_a-speed'); i.value = '5 min'; i.dispatchEvent(new Event('change'))")
b.wait("return document.querySelector('.field.error .message') !== null", 10)
message = b.js("return document.querySelector('.field.error .message').textContent")
check("is a time, but a speed is expected" in message, f"field error: {message}")
b.screenshot(f"{out}/4-error.png")
b.js("document.querySelector('.field.error .reset').click()")
b.wait("return document.querySelector('.field.error') === null", 10)

# Change the source's routing rule, then run with 5 replications.
b.js("""[...document.querySelectorAll('.diagram rect')].find(r => r.querySelector('title')?.textContent === 'source arrivals')
       .dispatchEvent(new MouseEvent('click', {bubbles: true}))""")
b.wait("return document.querySelector('.panel h3')?.textContent === 'arrivals'")
b.js("const s = document.getElementById('f-sources.arrivals.output-rule'); s.value = 'round_robin'; s.dispatchEvent(new Event('change'))")
b.js("const n = document.getElementById('scenario-name'); n.value = 'ui-round-robin'; n.dispatchEvent(new Event('input'))")
b.wait("return document.querySelector('.actions .hint').textContent.includes('1 change')")
b.screenshot(f"{out}/5-changed.png")
b.js("document.getElementById('run').click()")
b.wait("return document.querySelector('iframe.report') !== null", 60)
time.sleep(1.5)
b.screenshot(f"{out}/6-report.png")
downloads = b.js("return [...document.querySelectorAll('.results-bar a')].map(a => a.textContent)")
check(downloads == ["summary.json", "replications.csv", "events.jsonl", "layout.json", "scenario.yaml"], f"downloads: {downloads}")
report_title = b.js("return document.querySelector('iframe.report').contentDocument.querySelector('h1')?.textContent")
check(report_title == "ui-round-robin", f"report title: {report_title}")

b.js("[...document.querySelectorAll('.results-bar button')].find(x => x.textContent === 'Animation').click()")
b.wait("return document.querySelector('.animation canvas') !== null", 30)
time.sleep(4)
clock = b.js("return document.querySelector('.overlay .clock').textContent")
check(clock.startswith("day 1  00:0") and clock != "day 1  00:00:00", f"animation clock: {clock}")
b.screenshot(f"{out}/7-animation.png")

b.js("[...document.querySelectorAll('.results-bar button')].find(x => x.textContent === '3D').click()")
b.wait("return document.body.textContent.includes('3D animation: planned')")
b.screenshot(f"{out}/8-3d.png")

b.js("[...document.querySelectorAll('.tabs button')].find(x => x.textContent === 'Model').click()")
b.wait("return document.querySelector('.actions') !== null")
b.js("document.querySelector('.actions button').click()")  # Save as scenario
b.wait("return document.querySelector('.head p')?.textContent === 'Scenario of two-cells.yaml'", 10)
current = b.js("return document.querySelector('#models .current')?.textContent")
check(current.startswith("ui-round-robin"), f"saved scenario selected: {current}")
print("web UI session: all checks passed")
