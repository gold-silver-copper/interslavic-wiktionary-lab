# Candidate-generation benchmark

Benchmark: reconstruct the official Interslavic lemma from the modern Slavic cognates in the official dictionary, **without showing the generator the answer**. Evaluated on **16300** benchmarkable single-word entries. Every rule is kept only if it improved measured accuracy.

- **Metrics.** *exact*: identical to the official flavored lemma; *normalized*: identical after reducing both to the standard alphabet (§1.3); *skeleton*: identical after an ASCII fold; *top-3/5*: any of the first N candidates matches (normalized); *mean edit*: mean normalized Levenshtein distance to the official lemma.

## Kept rules — cumulative ablation ladder

Each rung adds exactly one rule to the previous, so its accuracy delta is attributable. The last rung is the kept **production** configuration.

| Rung | exact top-1 | norm top-1 | Δ norm | top-3 | mean edit |
|---|---:|---:|---:|---:|---:|
| baseline | 27.38% | 34.96% | +0.00 pp | 42.89% | 0.253 |
| +branch-consensus | 28.06% | 36.24% | +1.28 pp | 44.50% | 0.251 |
| +six-subgroup | 28.28% | 36.48% | +0.24 pp | 44.33% | 0.251 |
| +lemma-endings | 30.09% | 38.36% | +1.88 pp | 46.50% | 0.240 |
| +internationalism | 31.37% | 40.01% | +1.65 pp | 48.43% | 0.239 |
| +prefixes | 32.23% | 40.34% | +0.33 pp | 49.10% | 0.238 |
| +depleophony | 32.21% | 40.53% | +0.18 pp | 49.31% | 0.237 |
| +nasals (production) | 32.42% | 40.43% | -0.10 pp | 49.22% | 0.238 |

- **baseline** — Transliterate the first available form; no branch balancing, no repairs (the original prototype behavior).
- **+branch-consensus** — Branch-balanced skeleton vote + South-Slavic representative.
- **+six-subgroup** — Six dialect-subgroup vote with population tie-break (§4.1).
- **+lemma-endings** — Native POS lemma endings: noun nom.sg, adj -y/-i, verb -ti (§3).
- **+internationalism** — Internationalism ending table: -izm/-cija/-ičny/-alny/-ovati (§5.2).
- **+prefixes** — Normalize verbal/nominal prefixes råz-/prěd- (§2).
- **+depleophony** — Undo East-Slavic pleophony / liquid metathesis (§2).
- **+nasals (production)** — Recover ę/ų nasal vowels from Polish (§2 Phase C). This is the kept production config.

## Rejected rules — tested and reverted

Each is the production config plus one experimental rule. All regress accuracy on the benchmark and are therefore **not** in the production config, per the keep-only-if-it-improves rule.

| Experiment | exact top-1 | Δ exact | norm top-1 | Δ norm |
|---|---:|---:|---:|---:|
| prod+palatals | 32.05% | -0.37 pp | 39.86% | -0.57 pp |
| prod+jat | 32.04% | -0.38 pp | 40.43% | +0.00 pp |
| prod+adj-longform | 29.86% | -2.56 pp | 37.20% | -3.23 pp |
| prod+y-recovery | 24.67% | -7.75 pp | 31.37% | -9.06 pp |

- **prod+palatals** — Recover ć/đ (*tj/*dj) from South Slavic — modern reflexes are too noisy; derive from Proto-Slavic instead.
- **prod+jat** — Reconstruct jat ě from the cross-branch reflex — unreliable from modern reflexes.
- **prod+adj-longform** — Long-form (ru/pl/cs) adjective representative — East/West orthographic quirks outweigh the fleeting-vowel fix.
- **prod+y-recovery** — Recover *y from East/West where South merged *y→i — too aggressive, flips correct i→y.

## POS-specific accuracy (final config)

| POS | n | exact | normalized |
|---|---:|---:|---:|
| adj | 2896 | 22.31% | 30.28% |
| adv | 657 | 19.18% | 28.46% |
| noun | 8362 | 40.70% | 47.88% |
| num | 112 | 9.82% | 23.21% |
| pron | 99 | 27.27% | 31.31% |
| verb | 4174 | 25.68% | 35.10% |

## Branch coverage vs accuracy (final config)

| branches with the consensus form | n | normalized |
|---:|---:|---:|
| 0 | 0 | 0.00% |
| 1 | 3573 | 14.27% |
| 2 | 6806 | 35.50% |
| 3 | 5921 | 61.88% |

## Confidence calibration (final config)

High-confidence candidates should match the official dictionary more often than low-confidence ones.

| confidence | n | normalized match |
|---|---:|---:|
| high | 4601 | 66.62% |
| medium | 9410 | 34.92% |
| low | 2289 | 10.44% |

## Before / after

- Baseline normalized top-1: **34.96%**
- Final normalized top-1: **40.43%** (+5.47 pp)
- Baseline exact top-1: **27.38%**
- Final exact top-1: **32.42%** (+5.04 pp)

## Remaining systematic errors (final config)

Of **9710** misses, **2830** (29%) are near-misses (normalized edit < 0.20 — an ending/one-letter fix) and **6880** are farther (usually a different root chosen by Interslavic).

| Error class | count | share of misses |
|---|---:|---:|
| different root / derivation | 4464 | 46.0% |
| extra letter (epenthesis / ending) | 2184 | 22.5% |
| single-letter substitution | 1241 | 12.8% |
| missing letter (fleeting vowel / cluster) | 1060 | 10.9% |
| y / i distinction | 709 | 7.3% |
| flavored letter (ě/ę/ų/å/ć/đ) not recovered | 52 | 0.5% |

## Next recommended linguistic rules

Ranked by expected accuracy impact, from the remaining-error analysis and the rule spec (`data/RULE_SPEC.md`):

1. **Derive flavored letters (ě, ć/đ, å, y) from a Proto-Slavic form, not modern reflexes.** The palatal/jat/y experiments regress precisely because modern reflexes are ambiguous; §4.4 prescribes deriving the *form* from the reconstruction once the *root* is chosen by consensus. Wiring the Proto-Slavic rule engine into the consensus path (matching each meaning to its `sla-pro` entry via gloss) is the single biggest remaining lever.
2. **Divergent-root modeling (semantic families, §4.2 step 3).** The ~6880 far-misses are mostly cases where Interslavic picked a different root than the plurality skeleton; scoring candidate *roots* (not surface forms) with the six-subgroup vote would recover many.
3. **Secondary-imperfective verb stems** (`-yva-/-iva-/-ava-`) and the agentive `-telj`/abstract `-teljstvo` suffixes, seen repeatedly in the verb/noun error tail.
4. **Fleeting-vowel (yer) reconstruction in derived stems** (e.g. `obråbotyvati`, gen.pl), guided by cross-language vowel presence rather than a fixed rule.
5. **POS-specific gender/animacy inference** to pick the right nominal ending where the modern citation forms disagree.
