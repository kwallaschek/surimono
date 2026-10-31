# fulgur baseline (fulgur 0.40.0)

fulgur printing Chrome's own island SVGs, compared with the Chrome references. Shape: worst 32x32 window's share of near-mismatched ink (pass at or below 10%), with pixelmatch's share of differing ink in brackets. Colour: worst window of solid pixels off by more than the colour tolerance (pass at or below 10%). Hue: worst window of stroke pixels whose ink direction differs (pass at or below 30%). Every check must pass, with the same page size and count as Chrome.

| Fixture | Page shape | Page colour | Page hue | Page verdict | Islands shape | Islands colour | Islands hue | Island verdict |
|---|---|---|---|---|---|---|---|---|
| echarts-bar-simple-ja | 0.0% (pixelmatch 4.21%) | 0.0% | 0.0% | pass | 0.0% (pixelmatch 2.51%) | 0.0% | 0.0% | pass |
| echarts-bar-simple-latin | 0.0% (pixelmatch 3.86%) | 0.0% | 0.0% | pass | 0.0% (pixelmatch 0.17%) | 0.0% | 0.0% | pass |
| echarts-line-dense-latin | 0.0% (pixelmatch 12.19%) | 0.0% | 18.9% | pass | 0.0% (pixelmatch 0.28%) | 0.0% | 0.8% | pass |
| echarts-mixed-dense-ja | 2.5% (pixelmatch 5.82%) | 0.0% | 8.7% | pass | 2.9% (pixelmatch 3.99%) | 0.0% | 9.3% | pass |
| echarts-pie-ja | 0.0% (pixelmatch 5.19%) | 0.0% | 0.0% | pass | 0.0% (pixelmatch 5.38%) | 0.0% | 0.0% | pass |
| echarts-two-charts-latin | 0.0% (pixelmatch 2.60%) | 0.0% | 0.0% | pass | 0.0% (pixelmatch 0.02%), 0.0% (pixelmatch 0.00%) | 0.0%, 0.0% | 0.0%, 0.0% | pass, pass |
| vega-bar-simple-latin | 0.0% (pixelmatch 2.99%) | 0.0% | 0.0% | pass | 0.0% (pixelmatch 0.19%) | 0.0% | 0.0% | pass |
| vegalite-bar-simple-latin | 0.0% (pixelmatch 4.28%) | 0.0% | 0.0% | pass | 0.0% (pixelmatch 0.09%) | 0.0% | 0.0% | pass |
| vegalite-layered-dense-ja | 4.5% (pixelmatch 20.61%) | 0.0% | 12.0% | pass | 3.9% (pixelmatch 1.05%) | 0.0% | 11.4% | pass |
| vegalite-line-ja | 1.3% (pixelmatch 6.73%) | 0.0% | 3.0% | pass | 0.0% (pixelmatch 9.79%) | 0.0% | 1.7% | pass |
| vegalite-scatter-dense-latin | 0.1% (pixelmatch 8.90%) | 0.0% | 1.1% | pass | 0.0% (pixelmatch 9.54%) | 0.0% | 0.4% | pass |

23 of 23 comparisons pass.
