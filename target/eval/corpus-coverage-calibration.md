# Corpus coverage calibration

- Score domain: `corpus-coverage-score-v1` (`coverage-languages-branches-v1`)
- Labels: `official-pos-semantic-proxy-sense-ties-v3`. A negative means only that no compatible official sense was found; it is not proof that a reconstruction is linguistically wrong.
- Split: `fnv1a-id-mod-4-holdout-v1`; isotonic/PAVA fit uses train rows only.
- Coverage means recall over holdout semantic positives.
- Inputs: `data/slavic-lemmas.cache.json` `c9611dd774f8a9a1caee14d53fbef0d4192301676bd4f066466bc92080ba283a`; `data/official-isv.csv` `5265761404d6bda07df55d1069d26350b2c107731c63913f827054d793d46cff`.

| split | rows | semantic positives |
|---|---:|---:|
| train | 20203 | 3405 |
| holdout | 6834 | 1135 |

| holdout metric | raw | calibrated |
|---|---:|---:|
| ECE | 0.236172 | 0.010922 |
| Brier | 0.164644 | 0.108696 |

| unfiltered holdout operating point (not proposal-list quality) | selected | hits | precision | coverage |
|---|---:|---:|---:|---:|
| proposal p≥0.6 | 405 | 282 | 0.696296 | 0.248458 |
| review p≥0.3 | 995 | 530 | 0.532663 | 0.466960 |
