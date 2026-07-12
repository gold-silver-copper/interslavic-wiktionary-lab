# Synonym-aware accuracy (synonym-eval)

The strict benchmark scores agreement with the ONE official headword. But ~49% of misses are editorial word-choice (see the cluster-selection measurement): the engine produced a valid Interslavic word the committee did not pick as *the* lemma. This credits a prediction that reproduces **any** official ISV lemma whose gloss matches the concept.

| Metric | top-1 |
|---|---:|
| exact | 41.71% |
| normalized (strict) | 49.64% |
| **synonym-inclusive** | **55.83%** |

## What the 8209 strict misses actually are

| Class | count | share of misses |
|---|---:|---:|
| valid ISV synonym (another official lemma, same concept) | 1010 | 12.3% |
| another official lemma, different sense | 663 | 8.1% |
| not any official lemma (novel form or genuine error) | 6536 | 79.6% |
