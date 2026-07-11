# Synonym-aware accuracy (synonym-eval)

The strict benchmark scores agreement with the ONE official headword. But ~49% of misses are editorial word-choice (see the cluster-selection measurement): the engine produced a valid Interslavic word the committee did not pick as *the* lemma. This credits a prediction that reproduces **any** official ISV lemma whose gloss matches the concept.

| Metric | top-1 |
|---|---:|
| exact | 41.73% |
| normalized (strict) | 49.65% |
| **synonym-inclusive** | **55.85%** |

## What the 8207 strict misses actually are

| Class | count | share of misses |
|---|---:|---:|
| valid ISV synonym (another official lemma, same concept) | 1010 | 12.3% |
| another official lemma, different sense | 653 | 8.0% |
| not any official lemma (novel form or genuine error) | 6544 | 79.7% |
