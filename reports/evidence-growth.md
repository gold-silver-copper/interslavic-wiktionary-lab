# Evidence growth vs the root-absent ceiling (evidence-eval)

**Denominator:** 16300 benchmarkable meanings; the Wiktionary lemma cache holds 64504 lemmas. **Leakage story:** the cache never saw the `isv` answer; matching uses English gloss tokens + POS only; augmentation fills ONLY languages the dictionary row does not cite, so the dictionary's own evidence is never displaced.

| Measurement | value |
|---|---:|
| baseline root-absent misses | 1854 (11.4% of meanings) |
| recoverable from the cache (official root present under a gloss-matched lemma) | 60 (3.2% of root-absent) |
| — of which reachable by the conservative rule (root under an uncited language) | 11 |
| — unreachable: root only under an already-cited language (adding it would displace the dictionary's own citation) | 49 |
| — unreachable: root only as a bg/mk verb citation (dropped by the no-infinitive rule) | 0 |
| root-absent after augmentation | 1854 (11.4%) |
| accuracy: baseline → augmented (exact) | 41.71% → 41.71% (+0.00pp) |
| accuracy: baseline → augmented (normalized) | 49.64% → 49.64% (+0.00pp) |
| paired sign test (normalized) | fixed 0 / broke 0, p = 1.0000 |

Disclosed limits of the A/B: candidates need ≥2 shared gloss tokens (or full cover of the shorter gloss); the per-language pick is the highest-overlap candidate; reflexive meanings are excluded from augmentation (added forms would bypass reflexive-marker stripping); and the conservative fill-uncited-only rule cannot reach roots that sit under an already-cited language — the reachable share is reported separately above, and even a perfect recovery of it bounds the gain below 0.07pp exact.

The native uk/sr/bg/sl Wiktionary enrichment named in issue #4 is **data-blocked** (no per-language wiktextract dumps on disk; enrichment affects display only, not benchmark evidence) and is recorded as out of scope here.
