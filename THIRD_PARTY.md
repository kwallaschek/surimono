# Third-party code

Bundled JavaScript libraries and fonts, with version, licence and source. Each
keeps its licence file next to it. Rust dependencies are checked by cargo-deny
(`deny.toml`) and are not listed here.

| Component | Version | Licence | Source | Location |
|---|---|---|---|---|
| Noto Sans (test fixture font, not shipped) | 2.015 | OFL-1.1 | https://github.com/notofonts/notofonts.github.io (412a7edb2c8c535d1066157e5b17e661fd5591fe) | fixtures/fonts/ |
| Noto Sans JP (test fixture font, not shipped) | 2.004 | OFL-1.1 | https://github.com/notofonts/noto-cjk (f8d157532fbfaeda587e826d4cd5b21a49186f7c) | fixtures/fonts/ |
| Apache ECharts | 6.1.0 | Apache-2.0 (NOTICE; embeds d3 under BSD-3-Clause) | npm `echarts` | crates/islands/bundles/echarts/ |
| zrender | 6.1.0 | BSD-3-Clause | npm `zrender` (inside the ECharts bundle) | crates/islands/bundles/echarts/ |
| tslib | 2.3.0 | 0BSD | npm `tslib` (inside the ECharts bundle) | crates/islands/bundles/echarts/ |
| Vega | 6.4.0 | BSD-3-Clause | npm `vega` | crates/islands/bundles/vega/ |
| Vega-Lite | 6.5.0 | BSD-3-Clause | npm `vega-lite` | crates/islands/bundles/vega/ |
| Vega's 51 bundled dependencies | per lockfile | BSD-3-Clause, ISC, Unlicense (robust-predicates) | npm | crates/islands/bundles/vega/THIRD_PARTY_LICENSES.txt |
