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
| +proto-derived | 36.75% | 34.99% | +1.76 | 44.09% | 42.01% | +2.09 |
| +intl-preference | 36.84% | 35.09% | +1.74 | 44.17% | 42.10% | +2.07 |
| +adj-fleeting | 37.91% | 36.54% | +1.37 | 45.68% | 43.87% | +1.81 |
| +synonym-alts | 37.91% | 36.54% | +1.37 | 45.68% | 43.87% | +1.81 |
| +prefix-strip | 38.41% | 37.20% | +1.21 | 45.86% | 44.09% | +1.77 |
| +loan-stem-repair | 39.82% | 38.67% | +1.15 | 47.32% | 45.59% | +1.73 |
| +verb-class | 39.88% | 38.67% | +1.21 | 47.39% | 45.61% | +1.78 |
| +voicing | 39.96% | 38.72% | +1.24 | 47.49% | 45.66% | +1.83 |
| +explicit-etymology | 40.18% | 39.14% | +1.04 | 47.46% | 45.98% | +1.49 |
| +medoid-rep | 41.25% | 40.29% | +0.96 | 49.20% | 47.94% | +1.26 |
| +deriv-suffixes | 41.50% | 40.53% | +0.96 | 49.34% | 48.11% | +1.23 |
| +loan-hiatus | 41.57% | 40.56% | +1.01 | 49.42% | 48.14% | +1.28 |
| +spirantization (production) | 41.89% | 40.93% | +0.96 | 49.89% | 48.68% | +1.22 |

## Ladder-rung significance (paired sign test)

Each rung vs the previous rung, paired per entry: `fixed` = newly matched, `broke` = newly missed, on the **exact** metric (the primary keep-metric) and the normalized metric. p is the two-sided sign test on the discordant pairs — a rung whose p ≳ 0.05 on its keep-metric is not distinguishable from noise on this benchmark and should be treated as provisional, not proven.

| Rung | Δ exact | fixed/broke (exact) | p (exact) | Δ norm | fixed/broke (norm) | p (norm) |
|---|---:|---:|---:|---:|---:|---:|
| +branch-consensus | +0.55pp | 1077/988 | 0.0528 | +1.04pp | 1595/1425 | 0.0021 |
| +six-subgroup | +0.24pp | 138/99 | 0.0136 | +0.27pp | 198/154 | 0.0219 |
| +lemma-endings | +2.02pp | 335/5 | 0.0000 | +2.46pp | 410/9 | 0.0000 |
| +internationalism | +1.31pp | 217/4 | 0.0000 | +1.69pp | 279/4 | 0.0000 |
| +prefixes | +0.82pp | 134/0 | 0.0000 | +0.38pp | 62/0 | 0.0000 |
| +depleophony | -0.01pp | 0/2 | 0.4795 | +0.18pp | 39/9 | 0.0000 |
| +nasals | +0.37pp | 62/1 | 0.0000 | +0.09pp | 23/8 | 0.0119 |
| +proto-derived | +3.50pp | 692/122 | 0.0000 | +2.22pp | 493/131 | 0.0000 |
| +intl-preference | +0.09pp | 28/14 | 0.0449 | +0.09pp | 28/14 | 0.0449 |
| +adj-fleeting | +1.17pp | 196/6 | 0.0000 | +1.57pp | 263/7 | 0.0000 |
| +synonym-alts | +0.00pp | 0/0 | 1.0000 | +0.00pp | 0/0 | 1.0000 |
| +prefix-strip | +0.54pp | 96/8 | 0.0000 | +0.19pp | 39/8 | 0.0000 |
| +loan-stem-repair | +1.43pp | 244/11 | 0.0000 | +1.47pp | 253/14 | 0.0000 |
| +verb-class | +0.04pp | 11/4 | 0.1213 | +0.06pp | 14/4 | 0.0339 |
| +voicing | +0.07pp | 13/1 | 0.0033 | +0.09pp | 15/1 | 0.0012 |
| +explicit-etymology | +0.27pp | 204/160 | 0.0242 | +0.06pp | 215/205 | 0.6605 |
| +medoid-rep | +1.09pp | 550/372 | 0.0000 | +1.79pp | 821/529 | 0.0000 |
| +deriv-suffixes | +0.25pp | 40/0 | 0.0000 | +0.15pp | 24/0 | 0.0000 |
| +loan-hiatus | +0.06pp | 10/0 | 0.0044 | +0.07pp | 11/0 | 0.0026 |
| +spirantization (production) | +0.33pp | 57/3 | 0.0000 | +0.49pp | 83/3 | 0.0000 |

## Headline uncertainty (percentile bootstrap, 1000 seeded resamples)

- exact top-1 **41.65%** (95% CI 40.91–42.37%)
- normalized top-1 **49.59%** (95% CI 48.83–50.33%)

Deltas smaller than ~half this interval width should not be read as real without the paired test above (the paired test is far more sensitive than comparing two independent CIs).

## Score calibration (production top-1 score as P(normalized match))

| score bin | n | mean score | empirical match | gap |
|---|---:|---:|---:|---:|
| 0.3–0.4 | 453 | 0.393 | 0.130 | -0.263 |
| 0.4–0.5 | 4213 | 0.447 | 0.192 | -0.256 |
| 0.5–0.6 | 2887 | 0.547 | 0.390 | -0.157 |
| 0.6–0.7 | 1522 | 0.650 | 0.582 | -0.068 |
| 0.7–0.8 | 1303 | 0.750 | 0.662 | -0.088 |
| 0.8–0.9 | 1264 | 0.846 | 0.744 | -0.102 |
| 0.9–1.0 | 4658 | 0.948 | 0.730 | -0.218 |

- **ECE (expected calibration error): 0.1848** — mean |score − empirical match rate| weighted by bin size; 0 is perfectly calibrated.
- **Brier score: 0.2324** (lower is better; a constant base-rate predictor scores 0.2500).
- The three-way confidence badge (high/medium/low, thresholds 0.72/0.45 in `Confidence::from_score`) is derived from this score; if a bin's gap drifts past ~0.1 the thresholds should be re-fit.

### Isotonic recalibration (fit on dev, validated on holdout)

A monotone score→probability map (decile histogram + pool-adjacent-violators) fit on the dev split only, then applied to the untouched holdout:

| Holdout metric | raw score | recalibrated | Δ |
|---|---:|---:|---:|
| ECE | 0.1947 | 0.0128 | -0.1819 |
| Brier | 0.2339 | 0.1953 | -0.0386 |

The recalibrated probability is what a downstream consumer (site reliability badge, novel-word filter) should read as *P(matches the official lemma)*; the raw score remains the ranking key. Refit whenever the ladder changes.

### Proposal-filter operating points (calibrated p, holdout split)

| threshold | n ≥ t | precision | recall |
|---:|---:|---:|---:|
| ≥ 0.3 | 2859 | 61.7% | 88.9% |
| ≥ 0.4 | 2212 | 68.9% | 76.8% |
| ≥ 0.5 | 2212 | 68.9% | 76.8% |
| ≥ 0.6 | 1835 | 71.8% | 66.3% |
| ≥ 0.7 | 1490 | 72.9% | 54.7% |

The site's novel-word buckets (`export`) read these operating points: **propose** = calibrated p at the high-precision cutoff, **review** = the middle band, below = not shown. The committed calibrator is `data/score-calibration.json`.
