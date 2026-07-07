# Synonym-aware accuracy (synonym-eval)

The strict benchmark scores agreement with the ONE official headword. But ~49% of misses are editorial word-choice (see the cluster-selection measurement): the engine produced a valid Interslavic word the committee did not pick as *the* lemma. This credits a prediction that reproduces **any** official ISV lemma whose gloss matches the concept.

| Metric | top-1 |
|---|---:|
| exact | 41.01% |
| normalized (strict) | 48.88% |
| **synonym-inclusive** | **55.01%** |

## What the 8332 strict misses actually are

| Class | count | share of misses |
|---|---:|---:|
| valid ISV synonym (another official lemma, same concept) | 999 | 12.0% |
| another official lemma, different sense | 645 | 7.7% |
| not any official lemma (novel form or genuine error) | 6688 | 80.3% |
