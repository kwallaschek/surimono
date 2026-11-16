# What Chromium prints for each page here, and what the printers print from
# surimono's output: the data of crates/surimono/tests/print_media.rs. Never
# edited by hand.
#
#   python3 -I measure.py --cli BRANCH_CLI --cli-main MAIN_CLI \
#       --fulgur FULGUR_0_40 --raikiri RAIKIRI_PRINT
#
# For every page <p>.html:
#
# - <p>.output.html: surimono's printer-safe output (`surimono render --scripts`
#   with the fixture fonts, each `link-*.css` here given with --resource), the
#   output the measurements below are of. The test renders the page again and
#   requires these bytes, so the data can't silently describe other code.
# - <p>.measured.json:
#   - "chrome": the labels (the words of the page's <p> elements) on the
#     pages Chromium prints, for the page itself ("original"), the output of
#     main ("main") and this output ("output"); and whether each output's PDF
#     rasterised at 96 dpi (fixtures/tools' DPI, pdftoppm) is pixel for pixel
#     the original's ("pixels_identical"). Chromium prints as the fixtures'
#     reference pipeline does: Playwright's page.pdf(), preferCSSPageSize,
#     printBackground, from file://, after the fonts load.
#   - "fulgur" and "raikiri": the labels each printer prints for the
#     original, main's output and this output, with the fixtures' fonts only
#     (FONTCONFIG_FILE naming fixtures/fonts), as fixtures/tools/lib/fulgur.mjs
#     runs fulgur; and the share of pixels that differ from Chromium's print
#     of the original, page by page over the longer of the two documents.
#
# Run it holding the fleet's build lock (flock /home/claude/target/.fleet-build-lock).
import argparse
import glob
import hashlib
import json
import os
import subprocess
import tempfile

import numpy
from PIL import Image
from playwright.sync_api import sync_playwright

HERE = os.path.dirname(os.path.abspath(__file__))
FONTS = os.path.normpath(os.path.join(HERE, "../../../../../fixtures/fonts"))
DPI = 96


def fonts():
    files = sorted(f for f in os.listdir(FONTS) if f.endswith((".ttf", ".otf")))
    return [os.path.join(FONTS, f) for f in files]


def render(cli, page, out):
    args = [cli, "render", os.path.join(HERE, page + ".html"), "-o", out, "--scripts"]
    for f in fonts():
        args += ["--font", f]
    for css in sorted(glob.glob(os.path.join(HERE, "link-*.css"))):
        args += ["--resource", "%s=%s" % (os.path.basename(css), css)]
    subprocess.run(args, check=True)


def labels(pdf):
    text = subprocess.run(["pdftotext", pdf, "-"], check=True, capture_output=True, text=True).stdout
    return sorted(set(text.split()))


def raster(pdf, work):
    prefix = os.path.join(work, os.path.basename(pdf)[:-4])
    subprocess.run(["pdftoppm", "-r", str(DPI), "-png", pdf, prefix], check=True)
    pages = sorted(glob.glob(prefix + "-*.png"))
    return [numpy.asarray(Image.open(p).convert("RGB")) for p in pages]


def identical(a, b):
    return len(a) == len(b) and all(x.shape == y.shape and (x == y).all() for x, y in zip(a, b))


def differing(reference, other):
    """Share of pixels that differ, over the pages of the longer document; a
    page one of them lacks counts as all different, and so do pixels outside
    the smaller of two page sizes."""
    total = changed = 0
    for i in range(max(len(reference), len(other))):
        if i >= len(reference) or i >= len(other):
            size = (reference[i] if i < len(reference) else other[i]).shape
            total += size[0] * size[1]
            changed += size[0] * size[1]
            continue
        a, b = reference[i], other[i]
        h, w = max(a.shape[0], b.shape[0]), max(a.shape[1], b.shape[1])
        hh, ww = min(a.shape[0], b.shape[0]), min(a.shape[1], b.shape[1])
        same = (a[:hh, :ww] == b[:hh, :ww]).all(axis=2).sum()
        total += h * w
        changed += h * w - int(same)
    return round(changed / total, 6) if total else 0.0


def printer(binary, html, pdf, work):
    conf = os.path.join(work, "fonts.conf")
    if not os.path.exists(conf):
        with open(conf, "w") as f:
            f.write('<?xml version="1.0"?>\n<fontconfig><dir>%s</dir><cachedir>%s</cachedir></fontconfig>\n'
                    % (FONTS, os.path.join(work, "fc-cache")))
    args = [binary, "render"]
    for f in fonts():
        args += ["--font", f]
    args += ["--creation-date", "2026-01-01", "-o", pdf, html]
    env = dict(os.environ, FONTCONFIG_FILE=conf)
    subprocess.run(args, check=True, env=env, capture_output=True, timeout=120)


def sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", required=True)
    ap.add_argument("--cli-main", required=True)
    ap.add_argument("--fulgur", required=True)
    ap.add_argument("--raikiri", required=True)
    ap.add_argument("pages", nargs="*")
    a = ap.parse_args()
    pages = a.pages or sorted(os.path.basename(p)[:-5] for p in glob.glob(os.path.join(HERE, "*.html"))
                              if not p.endswith(".output.html"))
    fulgur_version = subprocess.run([a.fulgur, "--version"], capture_output=True, text=True).stdout.strip()
    with sync_playwright() as p, tempfile.TemporaryDirectory() as work:
        browser = p.chromium.launch(channel="chromium")
        for name in pages:
            original = os.path.join(HERE, name + ".html")
            output = os.path.join(HERE, name + ".output.html")
            main_out = os.path.join(work, name + ".main.html")
            render(a.cli, name, output)
            render(a.cli_main, name, main_out)
            # Chromium loads what the output names relative to it, as the page does.
            main_here = os.path.join(HERE, ".main-" + name + ".html")
            os.replace(main_out, main_here)
            try:
                docs = {"original": original, "main": main_here, "output": output}
                chrome_pdf, chrome_png = {}, {}
                for key, html in docs.items():
                    page = browser.new_page()
                    page.goto("file://" + html)
                    page.evaluate("document.fonts.ready.then(() => 0)")
                    pdf = os.path.join(work, "%s.chrome.%s.pdf" % (name, key))
                    page.pdf(path=pdf, prefer_css_page_size=True, print_background=True)
                    page.close()
                    chrome_pdf[key] = pdf
                    chrome_png[key] = raster(pdf, work)
                result = {
                    "chromium": browser.version,
                    "output_sha256": sha256(output),
                    "chrome": {
                        key: labels(pdf) for key, pdf in chrome_pdf.items()
                    },
                    "pixels_identical": {
                        key: identical(chrome_png["original"], chrome_png[key]) for key in ("main", "output")
                    },
                }
                for label, binary in (("fulgur", a.fulgur), ("raikiri", a.raikiri)):
                    row = {"labels": {}, "differing_pixels": {}}
                    for key, html in docs.items():
                        pdf = os.path.join(work, "%s.%s.%s.pdf" % (name, label, key))
                        printer(binary, html, pdf, work)
                        row["labels"][key] = labels(pdf)
                        row["differing_pixels"][key] = differing(chrome_png["original"], raster(pdf, work))
                    result[label] = row
                result["fulgur"]["version"] = fulgur_version
                result["fulgur"]["sha256"] = sha256(a.fulgur)
                result["raikiri"]["sha256"] = sha256(a.raikiri)
            finally:
                os.remove(main_here)
            with open(os.path.join(HERE, name + ".measured.json"), "w") as f:
                json.dump(result, f, indent=1, sort_keys=True)
                f.write("\n")
            print(name, "done")
        browser.close()


if __name__ == "__main__":
    main()
