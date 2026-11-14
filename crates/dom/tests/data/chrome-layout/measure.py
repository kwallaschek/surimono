# Chromium's layout of each probe page here, the expectations of
# crates/dom/tests/layout_chrome.rs. Never edited by hand and never our output.
#
#   python3 -I measure.py            # every page; writes <page>.chrome.json
#
# Playwright's Chromium in an 800 by 600 window (the probes' page in the tests
# is 800 by 600 with no margins), media `screen`, after the page's fonts load.
# For every element from <body> down, in document order ("elements"), and for
# the root element ("root"): its tag, its id, its getBoundingClientRect (x, y,
# width, height), and for an HTML element offsetLeft, offsetTop, offsetWidth,
# offsetHeight and the id (or tag) of its offsetParent. `report2000.chrome.json` is layout_real.rs's `report(2000)`,
# written here the same way, with only its last row.
import json
import os
import sys

from playwright.sync_api import sync_playwright

HERE = os.path.dirname(os.path.abspath(__file__))
PAGES = ["boxes", "boxes-nocv", "text", "gaps", "offsets", "report20", "breaks", "media", "offset-parents",
         "root-offsets", "body-positioned", "root-positioned", "forced-breaks", "viewport-height", "inline-lines"]

SCRIPT = """() => [document.documentElement, document.body, ...document.body.querySelectorAll('*')].map(e => {
    const r = e.getBoundingClientRect();
    const row = [e.localName, e.id, r.x, r.y, r.width, r.height];
    if (e instanceof HTMLElement) {
        const p = e.offsetParent;
        row.push(e.offsetLeft, e.offsetTop, e.offsetWidth, e.offsetHeight,
                 p === null ? null : (p.id || p.localName));
    }
    return row;
})"""


def report(rows):
    """crates/dom/tests/layout_real.rs `report`, character for character."""
    html = ("<!doctype html><html><head><style>body { margin: 0; font: 12px 'Noto Sans' }"
            ".row { display: flex; padding: 2px 4px; border-bottom: 1px solid #ccc }"
            ".l { flex: 1 } .v { width: 80px; text-align: right; font-weight: bold }"
            ".row:nth-child(even) { background: #f6f6f6 }</style></head><body>")
    for i in range(rows):
        html += ("<div class=row><span class=l>Item number %d</span>"
                 "<span class=v style=\"color:#333\">%d,%03d</span></div>" % (i, i % 97, (i * 37) % 1000))
    return html


def main():
    with sync_playwright() as p:
        browser = p.chromium.launch()
        version = browser.version
        page = browser.new_page(viewport={"width": 800, "height": 600})
        for name in PAGES:
            page.goto("file://" + os.path.join(HERE, name + ".html"))
            page.evaluate("document.fonts.ready.then(() => 0)")
            rows = page.evaluate(SCRIPT)
            out = {"chromium": version, "window": [800, 600], "page": name + ".html", "elements": rows[1:],
                   "root": rows[0]}
            with open(os.path.join(HERE, name + ".chrome.json"), "w") as f:
                f.write(json.dumps(out, indent=None, separators=(",", ":")).replace("],[", "],\n[")
                        .replace(',"root":', ',\n"root":'))
                f.write("\n")
        # The fixture fonts by @font-face, as the probes load them, before the
        # generated page's own styles.
        fonts = open(os.path.join(HERE, "boxes.html")).read().split("<style>\n", 1)[1].split("\nbody", 1)[0]
        # set_content has no base URL: the fonts by an absolute one.
        fixtures = "file://" + os.path.abspath(os.path.join(HERE, "../../../../../fixtures")) + "/"
        fonts = fonts.replace("../../../../../fixtures/", fixtures)
        html = report(2000).replace("<style>", "<style>" + fonts, 1)
        page.set_content(html)
        page.evaluate("document.fonts.ready.then(() => 0)")
        last = page.evaluate("""() => { const r = document.body.lastElementChild.getBoundingClientRect();
            return [r.x, r.y, r.width, r.height]; }""")
        with open(os.path.join(HERE, "report2000.chrome.json"), "w") as f:
            json.dump({"chromium": version, "window": [800, 600], "page": "layout_real.rs report(2000)",
                       "last_row": last}, f)
            f.write("\n")
        browser.close()
    print("Chromium", version)


if __name__ == "__main__":
    sys.exit(main())
