# Vendored WebIDL

Copied unmodified from [w3c/webref](https://github.com/w3c/webref) `ed/idl/`
at commit `09f7739aa691c3b9784f1e45069b8fa375e61423` (2026-10-31, "Update of ed
report from new reffy run"). webref extracts the IDL from each spec's editor's
draft. Builds and tests never fetch anything; `../fetch-webref.sh <commit>`
refreshes these files by hand.

| File | Spec | Publisher | Licence of the IDL | sha256 (first 16) |
|---|---|---|---|---|
| `console.idl` | [Console Standard](https://console.spec.whatwg.org/) | WHATWG | BSD-3-Clause | f182fd96abdddfa5 |
| `dom.idl` | [DOM Standard](https://dom.spec.whatwg.org/) | WHATWG | BSD-3-Clause | 129fac34c83e2f64 |
| `encoding.idl` | [Encoding Standard](https://encoding.spec.whatwg.org/) | WHATWG | BSD-3-Clause | 4df61c233ae9d7a4 |
| `html.idl` | [HTML Standard](https://html.spec.whatwg.org/multipage/) | WHATWG | BSD-3-Clause | df0499a53d5b49e5 |
| `url.idl` | [URL Standard](https://url.spec.whatwg.org/) | WHATWG | BSD-3-Clause | 6f83df8f28112f0d |
| `webidl.idl` | [Web IDL Standard](https://webidl.spec.whatwg.org/) | WHATWG | BSD-3-Clause | 4ca946b52a70ff05 |
| `css-font-loading.idl` | [CSS Font Loading Module Level 3](https://drafts.csswg.org/css-font-loading-3/) | W3C | W3C Software and Document License 2023 | 21fc3211834b434d |
| `cssom.idl` | [CSS Object Model (CSSOM) Module Level 1](https://drafts.csswg.org/cssom-1/) | W3C | W3C Software and Document License 2023 | 25794bc3978439b8 |
| `cssom-view.idl` | [CSSOM View Module Level 1](https://drafts.csswg.org/cssom-view-1/) | W3C | W3C Software and Document License 2023 | 2ce593bc372dc722 |
| `geometry.idl` | [Geometry Interfaces Module Level 1](https://drafts.csswg.org/geometry-1/) | W3C | W3C Software and Document License 2023 | 222657107b3ec0e8 |
| `hr-time.idl` | [High Resolution Time](https://w3c.github.io/hr-time/) | W3C | W3C Software and Document License 2023 | 20c66ab62bd1122b |
| `resize-observer.idl` | [Resize Observer Module Level 1](https://drafts.csswg.org/resize-observer-1/) | W3C | W3C Software and Document License 2023 | 72d67e3fab8f09c4 |
| `SVG.idl` | [Scalable Vector Graphics (SVG) 2](https://w3c.github.io/svgwg/svg2-draft/) | W3C | W3C Software and Document License 2023 | 5d2ff4e113daafe3 |
| `uievents.idl` | [UI Events](https://w3c.github.io/uievents/) | W3C | W3C Software and Document License 2023 | 928b9ba3244788b5 |

## Licences

- **The specs' IDL.** WHATWG Living Standards are CC BY 4.0, and "to the extent
  portions of it are incorporated into source code, such portions in the source
  code are licensed under the BSD 3-Clause License instead": see
  `LICENSE-WHATWG-BSD-3-Clause`. The W3C drafts above state "W3C liability,
  trademark and permissive document license rules apply", which is the W3C
  Software and Document License (2023): see
  `LICENSE-W3C-software-and-document-2023`, which carries the required notice
  for each document. Both are permissive; both ask for the notice to travel
  with copies.
- **webref** (the extraction tooling and repository) is MIT: see
  `LICENSE-webref-MIT`.
- Checked on 2026-10-31 against each spec's copyright section.

Generated code (`../src/stubs.rs`) is derived from this IDL; THIRD_PARTY.md
lists these files.
