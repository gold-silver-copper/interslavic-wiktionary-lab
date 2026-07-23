# V8 report — derivational morphology, correspondence repair, and a statistics-grade evaluation harness

V8 re-analyzed the engine end-to-end with the V7 instruments (stage attribution,
oracle ladder), mined the *full* residual list instead of the capped sample, and
shipped where the dictionary itself proves a rule categorical. It also upgraded
the evaluation methodology from "one number per rung" to a statistics-grade
harness: seeded holdout split, paired significance, bootstrap CIs, and validated
probability calibration.

## 0. Reproduced baseline

`evaluate` reproduced the V7.1 shipped numbers exactly before any change:
**41.01% exact / 48.88% normalized** (16,300 benchmarkable meanings), audit and
oracle artifacts byte-consistent with the committed snapshots.

## 1. Where the analysis pointed

The stage-attribution histogram said the remaining recoverable error lives in
`1-normalize/representative` (15.5%) and `7-endings` (6.2%), not the proto rule
engine (1.6%). Mining every miss (not the 400-row sample) against the official
lexicon surfaced suffix families where the dictionary is **categorical** — the
strongest possible evidence a rule is standard-Interslavic morphology rather
than an overfit:

| Pattern | official for | official against |
|---|---:|---:|
| `-telj-` kept before -stvo/-ny/-sky/-no/-ka | 59 | **0** |
| feminine i-stem `-sť` (kosť, radosť, …osť) | 516 | **0** |
| deverbal `-livy` (not South `-ljivy`) | 152 | **0** |
| loan hiatus `-ial-` (socialny), not `-ijal-` | 24 | **0** |
| loan midword `-io-` (sociolog), not `-ijo-` | 139 | 1 (kopijovati, a verb) |

The nearest-miss list also showed a systematic `h`-for-`g` leak
(interlinhvističny, blahosklonnosť, kalihrafičny) whenever the medoid picks a
Czech/Slovak/Ukrainian/Belarusian surface — the *g→h spirantization isogloss,
which Interslavic explicitly does not have (RULE_SPEC §2: "No g→h
spirantization rule").

## 2. Rules shipped (each a gated `ConsensusConfig` flag + ladder rung + tests)

Cumulative **41.01 → 41.65% exact (+0.64pp)**, **48.88 → 49.59% norm (+0.71pp)**;
top-3 59.57 → 60.48%, mean edit 0.226 → 0.224. All three are significant at
p < 0.005 (paired sign test) and hold their full gain on the held-out quarter.

| Rung | Rule | Δ exact | fixed/broke (exact) | p |
|---|---|---:|---:|---:|
| `+deriv-suffixes` | derivational-suffix normalization: `-telj-` before suffixes (izdateljstvo, bditeljny, neprijateljsky, izključiteljno), feminine `-sť` (gender-gated; abstract `-osť` behind a closed most/post/tost/hvost skip list), `-ljiv-→-liv-` in suffix position only (šljiva safe) | **+0.25** | 40/0 | <1e-4 |
| `+loan-hiatus` | Graeco-Latin hiatus kept in loans: `ijal→ial`, `ijazm→iazm`, `ijast→iast`; midword `ijo→io` gated to nouns/adjectives (kopijovati keeps its glide) | **+0.06** | 10/0 | 0.004 |
| `+spirantization` | per-consonant-position `h→g` repair when the representative is cs/sk/uk/be and ≥2 g-preserving cognates (ru/pl/South) attest `g` at the aligned position; genuine *x/loan `h` (duh, alkohol) survives because the g-preserving lects write `h` there too | **+0.33** | 57/3 | <1e-4 |

Why these are not overfitting: each is (a) categorical in the official lexicon,
(b) derivable from the published word-formation rules (`[DERIV]`
root-consistency invariant, §3 lemma conventions, §2 phonology), and (c)
verified to hold on the seeded holdout split the rules were never tuned on —
the dev−holdout gap is unchanged (+0.96pp) after adding all three.

Downstream: corpus-eval (site path) 57.9→**58.6% exact / 62.2→63.1% norm**;
synonym-inclusive top-1 **55.8%**; proto-eval unchanged (the rules are
consensus-side). Site regenerated; 4,793 official lemmas reproduced.

## 3. Evaluation-methodology upgrades (`target/eval/methodology.md`, all seeded/deterministic)

1. **Overfitting guard — seeded 75/25 dev/holdout split.** Entries are split by
   a stable FNV-1a hash of the dictionary id, so the held-out quarter never
   changes. Every rung is reported on both splits. Finding: the whole historic
   ladder generalizes (dev−holdout ≈ +1pp, within the holdout's ±1.5pp sampling
   noise), and the V8 rules leave the gap unchanged.
2. **Paired significance per rung** (two-sided sign test on discordant pairs,
   both metrics). Findings the old ladder hid: `+explicit-etymology` is noise on
   the normalized metric (215 fixed / 205 broke, p=0.66; kept for its
   exact-metric gain, p=0.02); `+depleophony` nets −2 entries on exact;
   `+verb-class` is marginal (p=0.12 exact). These are now flagged in the
   report instead of counting as proven wins.
3. **Bootstrap 95% CIs on the headline** (1000 seeded resamples): exact 41.65%
   (40.9–42.4), normalized 49.59% (48.8–50.3) — the yardstick against which
   sub-0.1pp "gains" should be read.
4. **Calibration measurement + validated fix.** Reliability table (score decile
   → empirical match), **ECE 0.185**, Brier 0.232: the raw score is
   systematically overconfident (the 0.9–1.0 bin matches only 73%). An
   **isotonic recalibration** (decile histogram + pool-adjacent-violators) fit
   on dev only and applied to the untouched holdout drops holdout ECE to
   **0.013** and Brier to 0.195. The recalibrated probability is what a
   downstream consumer should read as P(matches official); the raw score stays
   the ranking key.
5. **Full-resolution artifacts.** `predictions.csv` (every entry: prediction,
   split, score, hit flags) and an uncapped `audit-misses.csv` (every miss with
   per-stage blame) — the V8 suffix rules were found by mining exactly these
   residuals, which the previous 400/500-row caps had truncated.
6. **CI floor raised 36.0 → 39.5%** exact top-1 (still measuring the shipped
   `runs.last()` config).

## 4. Post-V8 blame map

8,332 → **8,217 misses**. The endings bucket shrank 518→501 and
normalize/representative 1,288→1,200 (the V8 rules converted their tail);
`same-root-surface` ranking bugs are down to 156 (1.9% of misses). What remains
is ~77% editorial-or-evidence (cluster/vote 33.3% + merge-rank diff-root 20.3%
+ root-absent 21.8%) — consistent with the oracle ladder (cluster +4.5pp is the
ceiling and it is mostly answer-reading). The honest recoverable levers left:
representative +2.3pp, proto-link +2.7pp exact (oracle-measured upper bounds).

## 5. Tried and considered, not shipped

- **`-ijan-→-ian-` hiatus**: 13 official -ijan- vs 17 -ian- — genuinely mixed
  (indijansky vs veterinar); no rule.
- **`-ijat-→-iat-`**: 17 vs 14 — mixed; no rule.
- **`-nn-` degemination/gemination** (heterogennosť): depends on the underlying
  adjective's -n-stem, which the surface alone doesn't identify; left to the
  proto path.
- **Recalibrated score feeding the site badge**: the calibrator is demonstrated
  and holdout-validated in `methodology.md`, but the product-side score/badge
  is unchanged this round — swapping the badge source is a display decision,
  and the 3-way badge is already ordinally sound (72/39/12%).
