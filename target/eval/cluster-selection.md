# Cluster-selection headroom (Measurement #2)

The wrong-cluster miss bucket is mostly the official dictionary choosing a different (editorial) root than our plurality vote. This forces the winning cluster by a **leakage-free** rule (except `oracle-cluster`, which reads the answer as the ceiling) and scores the real pipeline over 16300 meanings. `cluster-hit%` is the share of meanings whose official root is in the evidence where the rule's top candidate lands on that root.

| Rule | exact | Δ exact | norm | Δ norm | cluster-hit |
|---|---:|---:|---:|---:|---:|
| production | 41.73% | +0.00pp | 49.65% | +0.00pp | 69.1% |
| max-langs | 41.04% | -0.69pp | 48.83% | -0.82pp | 67.5% |
| max-branches | 41.20% | -0.53pp | 48.88% | -0.77pp | 67.8% |
| intl-first | 41.51% | -0.22pp | 49.34% | -0.31pp | 68.5% |
| oracle-cluster | 46.20% | +4.47pp | 56.34% | +6.69pp | 84.8% |

- **production** — the real branch-balanced six-subgroup vote (reference).
- **max-langs / max-branches** — force the cluster attested by the most distinct languages / branches (a raw recognizability proxy).
- **intl-first** — force any internationalism cluster (tests extending the genesis=I preference to every meaning).
- **oracle-cluster** — force the official cluster (upper bound; reads the answer).
