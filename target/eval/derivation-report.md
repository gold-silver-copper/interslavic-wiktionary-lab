# Derivation benchmark (derive-eval)

**Denominator:** 2214 derivationally related official lemma pairs, mined by inverse suffix lookup over the official dictionary (18458 entries). **Leakage story:** the layer receives the official *base* lemma + POS and must produce the official *derivative* forward; it never sees the derivative. Pair *selection* shares alternation knowledge with the layer (a disclosed bias — pairs the miner cannot align are excluded), but forward generation must still choose the right suffix allomorph, seam alternation and flavored spelling. **Dev/holdout (seeded id split):** normalized 99.69% / 99.83% (589 held out).

| Metric | seam-aware layer | naive concat baseline | Δ |
|---|---:|---:|---:|
| exact | **96.12%** | 48.06% | +48.06pp |
| normalized | **99.73%** | 83.56% | +16.17pp |

## Per pattern

| pattern | pairs | exact | normalized | naive exact | naive normalized |
|---|---:|---:|---:|---:|---:|
| adv | 371 | 99.73% | 100.00% | 97.30% | 97.57% |
| dimka | 24 | 91.67% | 100.00% | 58.33% | 66.67% |
| ica | 13 | 92.31% | 100.00% | 84.62% | 92.31% |
| ne | 171 | 99.42% | 100.00% | 99.42% | 100.00% |
| ny | 477 | 86.58% | 99.16% | 60.80% | 73.38% |
| ost | 428 | 99.53% | 100.00% | 0.00% | 100.00% |
| sky | 145 | 91.72% | 98.62% | 76.55% | 83.45% |
| telj | 92 | 100.00% | 100.00% | 100.00% | 100.00% |
| teljka | 6 | 100.00% | 100.00% | 100.00% | 100.00% |
| teljstvo | 9 | 100.00% | 100.00% | 100.00% | 100.00% |
| vnoun | 478 | 99.37% | 100.00% | 0.00% | 59.21% |

## Nearest misses (sample)

```
pattern,base,official,derived,naive
sky,dětę,dětsky,dětęsky,dětęsky
sky,frank,franksky,frančsky,franksky
ny,konopja,konopjany,konopjny,konopjny
ny,nazva,nazvany,nazvny,nazvny
ny,vŕh,vŕhny,vŕšny,vŕhny
ny,zemja,zemjany,zemjny,zemjny
```
