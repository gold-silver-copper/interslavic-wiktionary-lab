# Candidate-generation benchmark

Benchmark: reconstruct the official Interslavic lemma from the modern Slavic cognates in the official dictionary, **without showing the generator the answer**. Evaluated on **16300** benchmarkable single-word entries. Every rule is kept only if it improved measured accuracy.

- **Metrics.** *exact*: identical to the official flavored lemma; *normalized*: identical after reducing both to the standard alphabet (§1.3); *skeleton*: identical after an ASCII fold; *top-3/5*: any of the first N candidates matches (normalized); *mean edit*: mean normalized Levenshtein distance to the official lemma.

## Kept rules — cumulative ablation ladder

Each rung adds exactly one rule to the previous, so its accuracy delta is attributable. The last rung is the kept **production** configuration.

| Rung | exact top-1 | norm top-1 | Δ norm | top-3 | mean edit |
|---|---:|---:|---:|---:|---:|
| baseline | 27.52% | 35.23% | +0.00 pp | 43.26% | 0.252 |
| +branch-consensus | 28.06% | 36.28% | +1.04 pp | 44.68% | 0.251 |
| +six-subgroup | 28.30% | 36.55% | +0.27 pp | 44.47% | 0.251 |
| +lemma-endings | 30.10% | 38.76% | +2.21 pp | 47.12% | 0.239 |
| +internationalism | 31.47% | 40.51% | +1.75 pp | 49.20% | 0.237 |
| +prefixes | 32.28% | 40.89% | +0.38 pp | 49.94% | 0.236 |
| +depleophony | 32.27% | 41.07% | +0.18 pp | 50.14% | 0.235 |
| +nasals | 32.64% | 41.17% | +0.09 pp | 50.24% | 0.235 |
| +proto-derived | 34.99% | 42.31% | +1.14 pp | 52.45% | 0.233 |
| +intl-preference | 35.08% | 42.39% | +0.09 pp | 52.49% | 0.232 |
| +adj-fleeting | 36.32% | 44.07% | +1.68 pp | 54.50% | 0.230 |
| +synonym-alts | 36.32% | 44.07% | +0.00 pp | 54.66% | 0.230 |
| +prefix-strip | 36.64% | 44.08% | +0.01 pp | 54.86% | 0.230 |
| +loan-stem-repair | 38.10% | 45.57% | +1.49 pp | 56.33% | 0.227 |
| +explicit-etymology (production) | 38.42% | 45.50% | -0.07 pp | 56.86% | 0.229 |

- **baseline** — Transliterate the first available form; no branch balancing, no repairs (the original prototype behavior).
- **+branch-consensus** — Branch-balanced skeleton vote + South-Slavic representative.
- **+six-subgroup** — Six dialect-subgroup vote with population tie-break (§4.1).
- **+lemma-endings** — Native POS lemma endings: noun nom.sg, adj -y/-i, verb -ti (§3).
- **+internationalism** — Internationalism ending table: -izm/-cija/-ičny/-alny/-ovati (§5.2).
- **+prefixes** — Normalize verbal/nominal prefixes råz-/prěd- (§2).
- **+depleophony** — Undo East-Slavic pleophony / liquid metathesis (§2).
- **+nasals** — Recover ę/ų nasal vowels from Polish (§2 Phase C).
- **+proto-derived** — Two-stage §4.4: consensus picks the root, the Proto-Slavic rule engine supplies the flavored form (ě/ć/đ/å/ȯ/y) via a leakage-free descendant+gloss link. Requires the proto cache.
- **+intl-preference** — Prefer the internationalism cluster over native synonyms (ISV design criteria favor international roots for modern vocabulary): aeroplan over samolot.
- **+adj-fleeting** — Drop a South-Slavic adjective's fleeting vowel before -y, gated on East/West consonant adjacency (dobar→dobry, zelen stays).
- **+synonym-alts** — Seed alternatives from secondary translations (below every primary candidate) so the official lemma surfaces in top-3/top-5 when it is a 2nd/3rd translation.
- **+prefix-strip** — Grow proto-link coverage: strip a shared prefix off the cognates, link the bare root, re-attach the Interslavic prefix (råzprostirati from *prostirati).
- **+loan-stem-repair** — Repair national adaptation quirks the representative leaks into a loan stem: Polish y→i, South-Slavic epenthetic vowel (akcenat→akcent), -ac→-ec, final -ia→-ija, masculine -a drop — each corroborated by a cognate or the internationalism gate.
- **+explicit-etymology (production)** — Use Wiktionary's stated (lang→ancestor) etymology to pick the Proto-Slavic reconstruction directly, before the fuzzy descendant+gloss link — the precise ancestor the corpus site uses.

## Rejected rules — tested and reverted

Each is the production config plus one experimental rule. All regress accuracy on the benchmark and are therefore **not** in the production config, per the keep-only-if-it-improves rule.

| Experiment | exact top-1 | Δ exact | norm top-1 | Δ norm |
|---|---:|---:|---:|---:|
| prod+palatals | 38.13% | -0.28 pp | 45.09% | -0.42 pp |
| prod+jat | 37.55% | -0.87 pp | 45.50% | -0.01 pp |
| prod+adj-longform | 35.55% | -2.87 pp | 41.86% | -3.64 pp |
| prod+y-recovery | 35.63% | -2.79 pp | 41.98% | -3.52 pp |

- **prod+palatals** — Recover ć/đ (*tj/*dj) from South Slavic — modern reflexes are too noisy; derive from Proto-Slavic instead.
- **prod+jat** — Reconstruct jat ě from the cross-branch reflex — unreliable from modern reflexes.
- **prod+adj-longform** — Long-form (ru/pl/cs) adjective representative — East/West orthographic quirks outweigh the fleeting-vowel fix.
- **prod+y-recovery** — Recover *y from East/West where South merged *y→i — too aggressive, flips correct i→y.

## POS-specific accuracy (final config)

| POS | n | exact | normalized |
|---|---:|---:|---:|
| adj | 2896 | 30.63% | 39.54% |
| adv | 657 | 19.94% | 29.22% |
| noun | 8362 | 46.88% | 53.29% |
| num | 112 | 11.61% | 24.11% |
| pron | 99 | 37.37% | 38.38% |
| verb | 4174 | 30.52% | 37.35% |

## Branch coverage vs accuracy (final config)

| branches with the consensus form | n | normalized |
|---:|---:|---:|
| 0 | 47 | 46.81% |
| 1 | 3553 | 17.68% |
| 2 | 5594 | 39.49% |
| 3 | 7106 | 64.14% |

## Confidence calibration (final config)

High-confidence candidates should match the official dictionary more often than low-confidence ones.

| confidence | n | normalized match |
|---|---:|---:|
| high | 6828 | 65.82% |
| medium | 7246 | 36.78% |
| low | 2226 | 11.59% |

## Before / after

- Baseline normalized top-1: **35.23%**
- Final normalized top-1: **45.50%** (+10.27 pp)
- Baseline exact top-1: **27.52%**
- Final exact top-1: **38.42%** (+10.90 pp)

## Remaining systematic errors (final config)

Of **8883** misses, **2342** (26%) are near-misses (normalized edit < 0.20 — an ending/one-letter fix) and **6541** are farther (usually a different root chosen by Interslavic).

| Error class | count | share of misses |
|---|---:|---:|
| different root / derivation | 4456 | 50.2% |
| extra letter (epenthesis / ending) | 1752 | 19.7% |
| single-letter substitution | 1073 | 12.1% |
| missing letter (fleeting vowel / cluster) | 1061 | 11.9% |
| y / i distinction | 492 | 5.5% |
| flavored letter (ě/ę/ų/å/ć/đ) not recovered | 49 | 0.6% |

## Next recommended linguistic rules

The Proto-Slavic-derived-form path (§4.4) is implemented — consensus picks the root and the Proto-Slavic rule engine supplies the flavored form via a leakage-free descendant+gloss link. Yer resolution now uses a genuine **tense-yer rule** (yer before *j → i/y) plus **reflex-guided vocalization** (a lexically-ambiguous weak yer is retained when the reflexes vote to keep it: `*pьsati`→`pisati` vs `*bьrati`→`brati`), and a length-free **reflex-shape-agreement** ranking rule replaced the earlier length heuristic. Ranked next steps, from the remaining-error analysis:

1. **Expand Proto-Slavic link coverage.** Only meanings with a matched `sla-pro` reconstruction get the flavored derivation; raising cache coverage and loosening the link gate (without admitting bad links) directly grows the proto-derived slice.
2. **Reduce the reconstruction's non-yer errors** (endings, palatalizations) so the proto form can be trusted even when it disagrees with the reflexes — currently such disagreements defer to the reflexes, capping the proto gain.
3. **Divergent-root modeling (semantic families, §4.2 step 3).** The ~6541 far-misses are mostly cases where Interslavic picked a different root than the plurality skeleton; scoring candidate *roots* (not surface forms) over the six subgroups, clustered by the proto descendant graph, would recover many.
4. **Secondary-imperfective verb stems** (`-yva-/-iva-/-ava-`) and the agentive `-telj`/abstract `-teljstvo` suffixes, seen repeatedly in the verb/noun error tail.
5. **POS-specific gender/animacy inference** to pick the right nominal ending where the modern citation forms disagree.
