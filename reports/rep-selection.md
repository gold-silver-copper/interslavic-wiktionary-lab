# Representative-selection headroom (rep-eval)

Given the right cluster, which attested surface should represent it? This forces the winning group's representative by a **leakage-free** rule (except `oracle-representative`, which reads the answer as the ceiling) and scores the real pipeline over 16300 meanings.

| Rule | exact | Δ exact | norm | Δ norm |
|---|---:|---:|---:|---:|
| production | 41.71% | +0.00pp | 49.64% | +0.00pp |
| medoid | 41.71% | +0.00pp | 49.64% | +0.00pp |
| modal-skeleton | 39.92% | -1.79pp | 47.12% | -2.52pp |
| shortest | 31.93% | -9.79pp | 38.27% | -11.37pp |
| oracle-representative | 43.91% | +2.20pp | 52.98% | +3.34pp |

- **production** — the fixed REP_PRIORITY (sl, hr, sr, pl, …) surface choice.
- **medoid** — the group member minimizing total folded edit distance to the others (most central form).
- **modal-skeleton** — the most common ascii-skeleton in the group, then REP_PRIORITY among its members.
- **shortest** — the shortest attested form (nominatives tend shorter than oblique cases).
- **oracle-representative** — the member folded-closest to the official lemma (ceiling; reads the answer).
