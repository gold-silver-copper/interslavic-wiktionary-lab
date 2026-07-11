# Stage-attribution histogram (V7 §2.3)

For each of the **8207** normalized misses (of 16300 benchmarkable meanings), the last pipeline stage whose output still folded to the official form — i.e. the stage that destroyed, or never produced, the correct answer. Computed by replaying the winning candidate's `RuleStep` trace.

| Stage | misses | share |
|---|---:|---:|
| 3-cluster/vote | 2726 | 33.2% |
| 8-merge-rank | 1829 | 22.3% |
| 0-root-absent | 1792 | 21.8% |
| 1-normalize/representative | 1193 | 14.5% |
| 7-endings | 500 | 6.1% |
| 6-proto-rule | 132 | 1.6% |
| 4-repair | 35 | 0.4% |

## Top causes within each stage

| Stage | detail | misses |
|---|---|---:|
| 3-cluster/vote | wrong-cluster | 2726 |
| 0-root-absent | evidence-gap | 1792 |
| 8-merge-rank | diff-root-editorial | 1673 |
| 1-normalize/representative | residual:length | 532 |
| 7-endings | ending-residual | 493 |
| 1-normalize/representative | residual:substitution | 375 |
| 1-normalize/representative | residual:y/i | 259 |
| 8-merge-rank | same-root-surface | 156 |
| 6-proto-rule | yers | 75 |
| 1-normalize/representative | residual:flavored-letter | 26 |
| 6-proto-rule | proto-rule-residual | 25 |
| 6-proto-rule | endings | 17 |
| 4-repair | liquid-metathesis | 16 |
| 4-repair | loan-epenthesis | 8 |
| 7-endings | adj-hard-y | 6 |
| 6-proto-rule | liquid-metathesis | 4 |
| 6-proto-rule | soft-consonants | 4 |
| 4-repair | loan-y-i | 3 |
| 4-repair | nasal-vowel | 3 |
| 4-repair | spirantization-hg | 3 |
| 6-proto-rule | prothesis | 3 |
| 6-proto-rule | syllabic-liquid | 3 |
| 1-normalize/representative | pick-representative | 1 |
| 4-repair | loan-fem-a | 1 |
| 4-repair | verb-stative-eti | 1 |
| 6-proto-rule | collective-je | 1 |
| 7-endings | noun-ost | 1 |
