# Evaluation methodology — statistical instruments

## Overfitting guard — seeded 75/25 dev/holdout split

Entries are split by a deterministic hash of their dictionary id (~25% held out, **4078** entries; the split never changes). Rules are developed against dev; a kept rule must not gain on dev while flat/negative on holdout — that gap is the overfitting signal. The dev−holdout gap for the production config should stay within the holdout's sampling noise (±~1pp).

| Rung | exact dev | exact holdout | gap | norm dev | norm holdout | gap |
|---|---:|---:|---:|---:|---:|---:|
| baseline | 27.91% | 26.34% | +1.57 | 35.64% | 34.01% | +1.63 |
| +branch-consensus | 28.47% | 26.83% | +1.65 | 36.79% | 34.75% | +2.04 |
| +six-subgroup | 28.70% | 27.10% | +1.61 | 37.05% | 35.04% | +2.01 |
| +lemma-endings | 30.78% | 28.96% | +1.82 | 39.60% | 37.22% | +2.38 |
| +internationalism | 32.07% | 30.31% | +1.76 | 41.33% | 38.79% | +2.53 |
| +prefixes | 32.88% | 31.19% | +1.68 | 41.70% | 39.21% | +2.48 |
| +depleophony | 32.87% | 31.17% | +1.70 | 41.88% | 39.41% | +2.47 |
| +nasals | 33.24% | 31.56% | +1.68 | 41.97% | 39.50% | +2.46 |
| +proto-derived | 36.75% | 35.02% | +1.73 | 44.08% | 42.01% | +2.08 |
| +intl-preference | 36.83% | 35.12% | +1.71 | 44.17% | 42.10% | +2.06 |
| +adj-fleeting | 37.90% | 36.54% | +1.36 | 45.67% | 43.85% | +1.83 |
| +synonym-alts | 37.90% | 36.54% | +1.36 | 45.67% | 43.85% | +1.83 |
| +prefix-strip | 38.41% | 37.22% | +1.18 | 45.86% | 44.09% | +1.77 |
| +loan-stem-repair | 39.82% | 38.70% | +1.13 | 47.32% | 45.59% | +1.73 |
| +verb-class | 39.88% | 38.70% | +1.18 | 47.39% | 45.61% | +1.78 |
| +voicing | 39.96% | 38.74% | +1.22 | 47.49% | 45.66% | +1.83 |
| +explicit-etymology | 40.15% | 39.21% | +0.94 | 47.45% | 46.05% | +1.40 |
| +medoid-rep | 41.22% | 40.36% | +0.86 | 49.17% | 47.99% | +1.18 |
| +deriv-suffixes | 41.47% | 40.61% | +0.86 | 49.30% | 48.16% | +1.14 |
| +loan-hiatus | 41.54% | 40.63% | +0.91 | 49.38% | 48.19% | +1.19 |
| +spirantization | 41.86% | 41.00% | +0.86 | 49.85% | 48.72% | +1.13 |
| +stem-class-endings (production) | 41.91% | 41.12% | +0.78 | 49.90% | 48.85% | +1.05 |

## Ladder-rung significance (paired sign test)

Each rung vs the previous rung, paired per entry: `fixed` = newly matched, `broke` = newly missed, on the **exact** metric (the primary keep-metric) and the normalized metric. p is the two-sided sign test on the discordant pairs — a rung whose p ≳ 0.05 on its keep-metric is not distinguishable from noise on this benchmark and should be treated as provisional, not proven.

| Rung | Δ exact | fixed/broke (exact) | p (exact) | Δ norm | fixed/broke (norm) | p (norm) |
|---|---:|---:|---:|---:|---:|---:|
| +branch-consensus | +0.55pp | 1077/988 | 0.0528 | +1.04pp | 1595/1425 | 0.0021 |
| +six-subgroup | +0.24pp | 138/99 | 0.0134 | +0.27pp | 198/154 | 0.0218 |
| +lemma-endings | +2.02pp | 335/5 | 0.0000 | +2.46pp | 410/9 | 0.0000 |
| +internationalism | +1.31pp | 217/4 | 0.0000 | +1.69pp | 279/4 | 0.0000 |
| +prefixes | +0.82pp | 134/0 | 0.0000 | +0.38pp | 62/0 | 0.0000 |
| +depleophony | -0.01pp | 0/2 | 0.5000 | +0.18pp | 39/9 | 0.0000 |
| +nasals | +0.37pp | 62/1 | 0.0000 | +0.09pp | 23/8 | 0.0107 |
| +proto-derived | +3.50pp | 697/127 | 0.0000 | +2.21pp | 499/138 | 0.0000 |
| +intl-preference | +0.09pp | 28/14 | 0.0436 | +0.09pp | 28/14 | 0.0436 |
| +adj-fleeting | +1.16pp | 195/6 | 0.0000 | +1.56pp | 262/7 | 0.0000 |
| +synonym-alts | +0.00pp | 0/0 | 1.0000 | +0.00pp | 0/0 | 1.0000 |
| +prefix-strip | +0.55pp | 98/8 | 0.0000 | +0.20pp | 41/8 | 0.0000 |
| +loan-stem-repair | +1.43pp | 244/11 | 0.0000 | +1.47pp | 253/14 | 0.0000 |
| +verb-class | +0.04pp | 11/4 | 0.1185 | +0.06pp | 14/4 | 0.0309 |
| +voicing | +0.07pp | 13/1 | 0.0018 | +0.09pp | 15/1 | 0.0005 |
| +explicit-etymology | +0.26pp | 205/163 | 0.0324 | +0.07pp | 218/207 | 0.6277 |
| +medoid-rep | +1.09pp | 548/370 | 0.0000 | +1.77pp | 816/527 | 0.0000 |
| +deriv-suffixes | +0.25pp | 40/0 | 0.0000 | +0.14pp | 23/0 | 0.0000 |
| +loan-hiatus | +0.06pp | 10/0 | 0.0020 | +0.07pp | 11/0 | 0.0010 |
| +spirantization | +0.33pp | 57/3 | 0.0000 | +0.49pp | 83/3 | 0.0000 |
| +stem-class-endings (production) | +0.07pp | 11/0 | 0.0010 | +0.07pp | 11/0 | 0.0010 |

## Headline uncertainty (percentile bootstrap, 1000 seeded resamples)

- exact top-1 **41.71%** (95% CI 40.96–42.47%)
- normalized top-1 **49.64%** (95% CI 48.86–50.39%)

Deltas smaller than ~half this interval width should not be read as real without the paired test above (the paired test is far more sensitive than comparing two independent CIs).

## Score calibration (production top-1 score as P(normalized match))

| score bin | n | mean score | empirical match | gap |
|---|---:|---:|---:|---:|
| 0.3–0.4 | 453 | 0.393 | 0.130 | -0.263 |
| 0.4–0.5 | 4201 | 0.447 | 0.192 | -0.255 |
| 0.5–0.6 | 2866 | 0.547 | 0.391 | -0.156 |
| 0.6–0.7 | 1515 | 0.650 | 0.583 | -0.067 |
| 0.7–0.8 | 1293 | 0.750 | 0.662 | -0.088 |
| 0.8–0.9 | 1264 | 0.846 | 0.739 | -0.107 |
| 0.9–1.0 | 4708 | 0.948 | 0.729 | -0.219 |

- **ECE (expected calibration error): 0.1855** — mean |score − empirical match rate| weighted by bin size; 0 is perfectly calibrated.
- **Brier score: 0.2328** (lower is better; a constant base-rate predictor scores 0.2500).
- The three-way confidence badge (high/medium/low, thresholds 0.72/0.45 in `Confidence::from_score`) is derived from this score; if a bin's gap drifts past ~0.1 the thresholds should be re-fit.

### Isotonic recalibration (fit on dev, validated on holdout)

A monotone score→probability map (decile histogram + pool-adjacent-violators) fit on the dev split only, then applied to the untouched holdout:

| Holdout metric | raw score | recalibrated | Δ |
|---|---:|---:|---:|
| ECE | 0.1942 | 0.0107 | -0.1835 |
| Brier | 0.2334 | 0.1951 | -0.0383 |

The recalibrated probability is what a downstream consumer (site reliability badge, novel-word filter) should read as *P(matches the official lemma)*; the raw score remains the ranking key. Refit whenever the ladder changes.

### Proposal-filter operating points (calibrated p, holdout split)

| threshold | n ≥ t | precision | recall |
|---:|---:|---:|---:|
| ≥ 0.3 | 2863 | 61.9% | 88.9% |
| ≥ 0.4 | 2221 | 69.1% | 77.0% |
| ≥ 0.5 | 2221 | 69.1% | 77.0% |
| ≥ 0.6 | 1845 | 71.9% | 66.6% |
| ≥ 0.7 | 1503 | 73.1% | 55.1% |

The site's novel-word buckets (`export`) read these operating points: **propose** = calibrated p at the high-precision cutoff, **review** = the middle band, below = not shown. The committed calibrator is `data/score-calibration.json`.
