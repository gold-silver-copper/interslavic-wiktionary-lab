# check-text benchmark (checktext-eval)

**Denominators:** the committed all-correct fixture `data/checktext-fixture.txt` (79 tokens), 7 gold sentences (agreement false-alarm set) and 4 seeded-error sentences. **Leakage story:** the fixture and sentence sets are hand-written against the official vocabulary; the checker never sees expected labels.

| Measurement | value |
|---|---:|
| fixture classification | 36 known-lemma / 43 known-form / 0 generated / **0 unknown** |
| fixture agreement false alarms | **0** |
| gold-sentence false alarms | **0** |
| seeded errors flagged | **4 / 4** |
| valence gold false alarms | **0** |
| seeded valence errors flagged | **3 / 3** |
| nonsense probe stays unknown | **yes** |

Agreement checks are deliberately conservative: they fire only when NO combination of the neighbouring tokens' analyses is compatible, both tokens are verification-grade, and each token is POS-unambiguous. Gender is enforced in the singular only (ISV plural adjectives mark nom-animacy only).
