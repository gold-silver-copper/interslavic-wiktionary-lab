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
| +internationalism | 31.44% | 40.47% | +1.71 pp | 49.17% | 0.237 |
| +prefixes | 32.25% | 40.85% | +0.38 pp | 49.90% | 0.236 |
| +depleophony | 32.23% | 41.04% | +0.18 pp | 50.10% | 0.235 |
| +nasals | 32.61% | 41.13% | +0.09 pp | 50.20% | 0.235 |
| +proto-derived | 34.96% | 42.27% | +1.14 pp | 52.42% | 0.233 |
| +intl-preference | 35.04% | 42.36% | +0.09 pp | 52.45% | 0.232 |
| +adj-fleeting | 36.28% | 44.04% | +1.68 pp | 54.47% | 0.230 |
| +synonym-alts | 36.28% | 44.04% | +0.00 pp | 54.63% | 0.230 |
| +prefix-strip | 36.61% | 44.04% | +0.01 pp | 54.83% | 0.230 |
| +loan-stem-repair | 37.76% | 45.21% | +1.17 pp | 56.00% | 0.228 |
| +explicit-etymology (production) | 38.10% | 45.18% | -0.04 pp | 56.52% | 0.230 |

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
| prod+palatals | 37.82% | -0.28 pp | 44.76% | -0.42 pp |
| prod+jat | 37.26% | -0.85 pp | 45.18% | +0.00 pp |
| prod+adj-longform | 35.24% | -2.87 pp | 41.53% | -3.64 pp |
| prod+y-recovery | 35.35% | -2.75 pp | 41.69% | -3.48 pp |

- **prod+palatals** — Recover ć/đ (*tj/*dj) from South Slavic — modern reflexes are too noisy; derive from Proto-Slavic instead.
- **prod+jat** — Reconstruct jat ě from the cross-branch reflex — unreliable from modern reflexes.
- **prod+adj-longform** — Long-form (ru/pl/cs) adjective representative — East/West orthographic quirks outweigh the fleeting-vowel fix.
- **prod+y-recovery** — Recover *y from East/West where South merged *y→i — too aggressive, flips correct i→y.

## POS-specific accuracy (final config)

| POS | n | exact | normalized |
|---|---:|---:|---:|
| adj | 2896 | 30.63% | 39.54% |
| adv | 657 | 19.94% | 29.22% |
| noun | 8362 | 46.41% | 52.81% |
| num | 112 | 11.61% | 24.11% |
| pron | 99 | 37.37% | 38.38% |
| verb | 4174 | 30.23% | 37.04% |

## Branch coverage vs accuracy (final config)

| branches with the consensus form | n | normalized |
|---:|---:|---:|
| 0 | 47 | 46.81% |
| 1 | 3553 | 17.65% |
| 2 | 5594 | 39.04% |
| 3 | 7106 | 63.76% |

## Confidence calibration (final config)

High-confidence candidates should match the official dictionary more often than low-confidence ones.

| confidence | n | normalized match |
|---|---:|---:|
| high | 6828 | 65.45% |
| medium | 7246 | 36.45% |
| low | 2226 | 11.41% |

## Before / after

- Baseline normalized top-1: **35.23%**
- Final normalized top-1: **45.18%** (+9.94 pp)
- Baseline exact top-1: **27.52%**
- Final exact top-1: **38.10%** (+10.59 pp)

## Remaining systematic errors (final config)

Of **8936** misses, **2373** (27%) are near-misses (normalized edit < 0.20 — an ending/one-letter fix) and **6563** are farther (usually a different root chosen by Interslavic).

| Error class | count | share of misses |
|---|---:|---:|
| different root / derivation | 4460 | 49.9% |
| extra letter (epenthesis / ending) | 1750 | 19.6% |
| missing letter (fleeting vowel / cluster) | 1092 | 12.2% |
| single-letter substitution | 1080 | 12.1% |
| y / i distinction | 505 | 5.7% |
| flavored letter (ě/ę/ų/å/ć/đ) not recovered | 49 | 0.5% |

## Next recommended linguistic rules

The Proto-Slavic-derived-form path (§4.4) is implemented — consensus picks the root and the Proto-Slavic rule engine supplies the flavored form via a leakage-free descendant+gloss link. Yer resolution now uses a genuine **tense-yer rule** (yer before *j → i/y) plus **reflex-guided vocalization** (a lexically-ambiguous weak yer is retained when the reflexes vote to keep it: `*pьsati`→`pisati` vs `*bьrati`→`brati`), and a length-free **reflex-shape-agreement** ranking rule replaced the earlier length heuristic. Ranked next steps, from the remaining-error analysis:

1. **Expand Proto-Slavic link coverage.** Only meanings with a matched `sla-pro` reconstruction get the flavored derivation; raising cache coverage and loosening the link gate (without admitting bad links) directly grows the proto-derived slice.
2. **Reduce the reconstruction's non-yer errors** (endings, palatalizations) so the proto form can be trusted even when it disagrees with the reflexes — currently such disagreements defer to the reflexes, capping the proto gain.
3. **Divergent-root modeling (semantic families, §4.2 step 3).** The ~6563 far-misses are mostly cases where Interslavic picked a different root than the plurality skeleton; scoring candidate *roots* (not surface forms) over the six subgroups, clustered by the proto descendant graph, would recover many.
4. **Secondary-imperfective verb stems** (`-yva-/-iva-/-ava-`) and the agentive `-telj`/abstract `-teljstvo` suffixes, seen repeatedly in the verb/noun error tail.
5. **POS-specific gender/animacy inference** to pick the right nominal ending where the modern citation forms disagree.
