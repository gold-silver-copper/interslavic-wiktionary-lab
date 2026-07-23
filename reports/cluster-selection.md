# Cluster-selection headroom (Measurement #2)

The wrong-cluster miss bucket is mostly the official dictionary choosing a different (editorial) root than our plurality vote. This forces the winning cluster by a **leakage-free** rule (except `oracle-cluster`, which reads the answer as the ceiling) and scores the real pipeline over 16300 meanings. `cluster-hit%` is the share of meanings whose official root is in the evidence where the rule's top candidate lands on that root.

| Rule | exact | Δ exact | norm | Δ norm | cluster-hit |
|---|---:|---:|---:|---:|---:|
| production | 41.71% | +0.00pp | 49.64% | +0.00pp | 69.1% |
| max-langs | 41.04% | -0.67pp | 48.83% | -0.81pp | 67.4% |
| max-branches | 41.20% | -0.52pp | 48.88% | -0.76pp | 67.7% |
| intl-first | 41.49% | -0.22pp | 49.33% | -0.31pp | 68.5% |
| oracle-cluster | 46.17% | +4.46pp | 56.31% | +6.67pp | 84.7% |

- **production** — the real branch-balanced six-subgroup vote (reference).
- **max-langs / max-branches** — force the cluster attested by the most distinct languages / branches (a raw recognizability proxy).
- **intl-first** — force any internationalism cluster (tests extending the genesis=I preference to every meaning).
- **oracle-cluster** — force the official cluster (upper bound; reads the answer).
