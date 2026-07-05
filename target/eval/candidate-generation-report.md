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
| +nasals | 32.42% | 40.43% | -0.10 pp | 49.22% | 0.238 |
| +proto-derived (production) | 33.83% | 40.62% | +0.19 pp | 50.94% | 0.237 |

- **baseline** — Transliterate the first available form; no branch balancing, no repairs (the original prototype behavior).
- **+branch-consensus** — Branch-balanced skeleton vote + South-Slavic representative.
- **+six-subgroup** — Six dialect-subgroup vote with population tie-break (§4.1).
- **+lemma-endings** — Native POS lemma endings: noun nom.sg, adj -y/-i, verb -ti (§3).
- **+internationalism** — Internationalism ending table: -izm/-cija/-ičny/-alny/-ovati (§5.2).
- **+prefixes** — Normalize verbal/nominal prefixes råz-/prěd- (§2).
- **+depleophony** — Undo East-Slavic pleophony / liquid metathesis (§2).
- **+nasals** — Recover ę/ų nasal vowels from Polish (§2 Phase C).
- **+proto-derived (production)** — Two-stage §4.4: consensus picks the root, the Proto-Slavic rule engine supplies the flavored form (ě/ć/đ/å/ȯ/y) via a leakage-free descendant+gloss link. Requires the proto cache.

## Rejected rules — tested and reverted

Each is the production config plus one experimental rule. All regress accuracy on the benchmark and are therefore **not** in the production config, per the keep-only-if-it-improves rule.

| Experiment | exact top-1 | Δ exact | norm top-1 | Δ norm |
|---|---:|---:|---:|---:|
| prod+palatals | 33.39% | -0.44 pp | 40.05% | -0.57 pp |
| prod+jat | 33.19% | -0.64 pp | 40.62% | +0.00 pp |
| prod+adj-longform | 31.56% | -2.28 pp | 37.85% | -2.77 pp |
| prod+y-recovery | 26.74% | -7.10 pp | 32.44% | -8.18 pp |

- **prod+palatals** — Recover ć/đ (*tj/*dj) from South Slavic — modern reflexes are too noisy; derive from Proto-Slavic instead.
- **prod+jat** — Reconstruct jat ě from the cross-branch reflex — unreliable from modern reflexes.
- **prod+adj-longform** — Long-form (ru/pl/cs) adjective representative — East/West orthographic quirks outweigh the fleeting-vowel fix.
- **prod+y-recovery** — Recover *y from East/West where South merged *y→i — too aggressive, flips correct i→y.

## POS-specific accuracy (final config)

| POS | n | exact | normalized |
|---|---:|---:|---:|
| adj | 2896 | 22.79% | 29.87% |
| adv | 657 | 19.18% | 28.46% |
| noun | 8362 | 42.39% | 48.21% |
| num | 112 | 9.82% | 23.21% |
| pron | 99 | 32.32% | 33.33% |
| verb | 4174 | 27.34% | 35.43% |

## Branch coverage vs accuracy (final config)

| branches with the consensus form | n | normalized |
|---:|---:|---:|
| 0 | 96 | 46.88% |
| 1 | 4197 | 21.30% |
| 2 | 6705 | 37.03% |
| 3 | 5302 | 60.34% |

## Confidence calibration (final config)

High-confidence candidates should match the official dictionary more often than low-confidence ones.

| confidence | n | normalized match |
|---|---:|---:|
| high | 5328 | 66.29% |
| medium | 8708 | 32.85% |
| low | 2264 | 10.07% |

## Before / after

- Baseline normalized top-1: **34.96%**
- Final normalized top-1: **40.62%** (+5.66 pp)
- Baseline exact top-1: **27.38%**
- Final exact top-1: **33.83%** (+6.45 pp)

## Remaining systematic errors (final config)

Of **9679** misses, **2816** (29%) are near-misses (normalized edit < 0.20 — an ending/one-letter fix) and **6863** are farther (usually a different root chosen by Interslavic).

| Error class | count | share of misses |
|---|---:|---:|
| different root / derivation | 4469 | 46.2% |
| extra letter (epenthesis / ending) | 2189 | 22.6% |
| single-letter substitution | 1244 | 12.9% |
| missing letter (fleeting vowel / cluster) | 1056 | 10.9% |
| y / i distinction | 667 | 6.9% |
| flavored letter (ě/ę/ų/å/ć/đ) not recovered | 54 | 0.6% |

## Next recommended linguistic rules

The Proto-Slavic-derived-form path (§4.4) is implemented — consensus picks the root and the Proto-Slavic rule engine supplies the flavored form via a leakage-free descendant+gloss link. Yer resolution now uses a genuine **tense-yer rule** (yer before *j → i/y) plus **reflex-guided vocalization** (a lexically-ambiguous weak yer is retained when the reflexes vote to keep it: `*pьsati`→`pisati` vs `*bьrati`→`brati`), and a length-free **reflex-shape-agreement** ranking rule replaced the earlier length heuristic. Ranked next steps, from the remaining-error analysis:

1. **Expand Proto-Slavic link coverage.** Only meanings with a matched `sla-pro` reconstruction get the flavored derivation; raising cache coverage and loosening the link gate (without admitting bad links) directly grows the proto-derived slice.
2. **Reduce the reconstruction's non-yer errors** (endings, palatalizations) so the proto form can be trusted even when it disagrees with the reflexes — currently such disagreements defer to the reflexes, capping the proto gain.
3. **Divergent-root modeling (semantic families, §4.2 step 3).** The ~6863 far-misses are mostly cases where Interslavic picked a different root than the plurality skeleton; scoring candidate *roots* (not surface forms) over the six subgroups, clustered by the proto descendant graph, would recover many.
4. **Secondary-imperfective verb stems** (`-yva-/-iva-/-ava-`) and the agentive `-telj`/abstract `-teljstvo` suffixes, seen repeatedly in the verb/noun error tail.
5. **POS-specific gender/animacy inference** to pick the right nominal ending where the modern citation forms disagree.
