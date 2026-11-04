# Third-party material in fixtures/

Test-only files under `fixtures/`, with version, licence and source. They are
never compiled into the crates or the gem. The root `THIRD_PARTY.md` lists them
too.

## Fonts (`fixtures/fonts/`)

Pinned per file in `fixtures/fonts/fonts.toml` (source URL at a commit SHA,
sha256, size); `python3 fixtures/fonts/verify.py` checks them.

| Font | Files | Version | Licence | Source |
|---|---|---|---|---|
| Noto Sans (unhinted, static) | NotoSans-Regular.ttf, NotoSans-Bold.ttf | 2.015 | SIL OFL 1.1 (`fixtures/fonts/OFL.txt`) | https://github.com/notofonts/notofonts.github.io at 412a7edb2c8c535d1066157e5b17e661fd5591fe |
| Noto Sans JP (static subset OTF) | NotoSansJP-Regular.otf, NotoSansJP-Bold.otf | 2.004 | SIL OFL 1.1 (`fixtures/fonts/OFL.txt`) | https://github.com/notofonts/noto-cjk at f8d157532fbfaeda587e826d4cd5b21a49186f7c |

## Real-world libraries (`fixtures/real-world/lib/`)

Unmodified npm files, run by Chrome as page scripts for the references. Each file
is pinned by sha256 in `fixtures/real-world/manifest.toml`.

| Library | Version | Licence | Source | Licence file |
|---|---|---|---|---|
| marked | 18.1.3 | MIT | https://github.com/markedjs/marked at ba50aa95636ac8f97f46446bbfeb04592ed22216 | `lib/marked-18.1.3/LICENSE` |
| highlight.js (@highlightjs/cdn-assets) | 11.12.0 | BSD-3-Clause | https://github.com/highlightjs/highlight.js at dce7a3dab8f3fd586138ba6c5f29ecc19c02db9f | `lib/highlight.js-11.12.0/LICENSE` |
| KaTeX | 0.19.1 | MIT | https://github.com/KaTeX/KaTeX at 8000fbd4e29cca2c0cb1e88b745af3ed2f436b66 | `lib/katex-0.19.1/LICENSE` |
| KaTeX fonts (20 woff2 files) | 0.19.1 | OFL-1.1, with Reserved Font Names | same, npm tarball `dist/fonts/` | `lib/katex-0.19.1/fonts/OFL.txt` |
| Alpine.js | 3.17.5 | MIT | https://github.com/alpinejs/alpine at 8d3d9a2f98385d4c879f4b5be080a0221279ed4d | pending (P1-CF-32): the npm tarball has none, see `lib/alpinejs-3.17.5/LICENSE-NOTE.md` |
| @vue/reactivity and @vue/shared, inlined in Alpine's `cdn.min.js` | range ~3.5.40 | MIT | https://registry.npmjs.org/@vue/reactivity/-/reactivity-3.5.40.tgz | `lib/alpinejs-3.17.5/LICENSE-vue-reactivity` |
| D3 | 7.9.1 | ISC | https://github.com/d3/d3 at 78db4998f838eb555548dfe8b44f3a5cb83e4695 | `lib/d3-7.9.1/LICENSE` |
