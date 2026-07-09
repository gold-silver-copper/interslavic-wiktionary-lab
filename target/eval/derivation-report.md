# Derivation benchmark (derive-eval)

**Denominator:** 2115 derivationally related official lemma pairs, mined by inverse suffix lookup over the official dictionary (18458 entries). **Leakage story:** the layer receives the official *base* lemma + POS and must produce the official *derivative* forward; it never sees the derivative. Pair *selection* shares alternation knowledge with the layer (a disclosed bias — pairs the miner cannot align are excluded), but forward generation must still choose the right suffix allomorph, seam alternation and flavored spelling. A small share of mined pairs are string coincidences rather than true derivations (e.g. vino→vinny 'wine→guilty'); they inflate both layers symmetrically and are counted in the disclosed selection bias. **Dev/holdout (seeded id split):** normalized 99.68% / 99.82% (559 held out).

| Metric | seam-aware layer | naive concat baseline | Δ |
|---|---:|---:|---:|
| exact | **96.03%** | 47.85% | +48.18pp |
| normalized | **99.72%** | 83.59% | +16.12pp |

## Per pattern

| pattern | pairs | exact | normalized | naive exact | naive normalized |
|---|---:|---:|---:|---:|---:|
| adv | 360 | 99.72% | 100.00% | 97.22% | 97.50% |
| dimka | 24 | 91.67% | 100.00% | 58.33% | 66.67% |
| ica | 13 | 92.31% | 100.00% | 84.62% | 92.31% |
| ne | 158 | 99.37% | 100.00% | 99.37% | 100.00% |
| ny | 446 | 86.10% | 99.10% | 59.64% | 72.65% |
| ost | 414 | 99.52% | 100.00% | 0.00% | 100.00% |
| sky | 145 | 91.72% | 98.62% | 76.55% | 83.45% |
| telj | 88 | 100.00% | 100.00% | 100.00% | 100.00% |
| teljka | 6 | 100.00% | 100.00% | 100.00% | 100.00% |
| teljstvo | 9 | 100.00% | 100.00% | 100.00% | 100.00% |
| vnoun | 452 | 99.34% | 100.00% | 0.00% | 59.51% |

## Off-official-base holdout (issue #37) — shipped derivative probability

The `generated` derivatives the export ships off attested official bases are ABSENT from the dictionary, so they have no gold and cannot be scored directly. This is the leakage-free proxy: hold out a slice of official derivatives by `is_holdout_id` (the shared seeded split), hide them from view, derive them off their still-visible official base, and score the derivation. Because `derive_family` never consults the dictionary, the hidden derivative is genuinely unseen. The shipped `probability` for a pattern is the **Wilson 95% lower bound of its holdout EXACT-match rate** (conservative: it shrinks toward 0 as the sample thins), capped at 0.90 — an irreducible existence/semantics margin the form-accuracy proxy cannot measure (the holdout asks *did we spell the derivative right*, not *is the derivative a real word*). Overall holdout exact **95.53%** over **559** held-out pairs. This is NOT the 96.03% derive-eval headline above, which scores a different, both-attested population.

| pattern | holdout pairs | exact | normalized | shipped probability |
|---|---:|---:|---:|---:|
| adv | 98 | 98.98% | 100.00% | 0.900 |
| dimka | 3 | 100.00% | 100.00% | 0.439 |
| ica | 2 | 50.00% | 100.00% | 0.095 |
| ne | 40 | 97.50% | 100.00% | 0.871 |
| ny | 118 | 87.29% | 99.15% | 0.801 |
| ost | 111 | 99.10% | 100.00% | 0.900 |
| sky | 38 | 89.47% | 100.00% | 0.759 |
| telj | 21 | 100.00% | 100.00% | 0.845 |
| teljka | 1 | 100.00% | 100.00% | 0.207 |
| vnoun | 127 | 98.43% | 100.00% | 0.900 |

## Nearest misses (dev split only — holdout misses are never published)

```
pattern,base,official,derived,naive
sky,dětę,dětsky,dětęsky,dětęsky
sky,frank,franksky,frančsky,franksky
ny,konopja,konopjany,konopjny,konopjny
ny,vŕh,vŕhny,vŕšny,vŕhny
ny,zemja,zemjany,zemjny,zemjny
```
