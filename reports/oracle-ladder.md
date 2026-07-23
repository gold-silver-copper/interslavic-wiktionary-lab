# Oracle ladder (V7 §2.4) — DIAGNOSTIC ONLY

Each row makes ONE pipeline stage perfect (by reading the official answer) while everything downstream stays the real production engine, over **16300** benchmarkable meanings. This path can never feed production; it exists only to rank stages by recoverable headroom. Spend effort top-down by Δ exact.

| Stage oracle | exact top-1 | Δ exact | norm top-1 | Δ norm |
|---|---:|---:|---:|---:|
| baseline (production) | 41.71% | — | 49.64% | — |
| oracle-cluster | 46.18% | +4.47pp | 56.31% | +6.67pp |
| oracle-representative | 43.91% | +2.20pp | 52.98% | +3.34pp |
| oracle-proto-link | 44.34% | +2.63pp | 53.18% | +3.54pp |
| oracle-all | 51.07% | +9.36pp | 63.81% | +14.17pp |

- **oracle-cluster** — force the vote to the cluster whose consonant key matches the official lemma; representative + repairs then run on the right cluster.
- **oracle-representative** — pick the winning group's member whose folded form is closest to the official lemma.
- **oracle-proto-link** — link the reconstruction whose derived form is closest to the official lemma (linker upper bound).
- **oracle-all** — all three at once (an approximate ceiling for the stages below word-selection).
