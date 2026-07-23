# Aspect-pair benchmark (aspect-eval)

**Frozen reproducible inventory:** 1440 deterministic 1:1 same-gloss, morphologically-related official ipf↔pf pairs (ordered manifest `aspect-pairs.tsv`, FNV-1a-64 `5ab3e19ec5d758dd`). **Scored denominator:** 1437 regular pairs; 3 closed suppletive predictions are excluded only when production actually fires the lexical rule, so unrecognized lexical pairs remain honest scored misses. **Keep metrics:** normalized both-correct (primary), normalized either-correct, and consonant-root fingerprint consistency. **Leakage:** official aspect/gloss/root spelling selects the evaluation slice only; both baseline forms are independently generated from cognate cells, and pair repair sees only those generated forms plus their scores. The shared seeded hash holds out 431 scored pairs.

| model | n | normalized both correct | normalized either correct | fingerprint consistency |
|---|---:|---:|---:|---:|
| independent baseline | 1437 | 16.63% | 49.34% | 78.22% |
| +core suffix repair | 1437 | 17.47% | 48.78% | 87.47% |
| +prefix perfectivization (production) | 1437 | 17.88% | 48.71% | 89.07% |
| +secondary imperfectives and -ovati→-ovyvati (experimental; holdout-flat) | 1437 | 17.95% | 48.50% | 91.51% |

The secondary `-yva-/-iva-/-ava-` and `-ovati→-ovyvati` families are controlled by `AspectConfig.secondary_imperfectives`. They remain implemented but disabled in production because the rung is flat on holdout normalized both-correct. The production prefix repair improves the declared primary **normalized both-correct** metric with no breaks and improves consonant-root fingerprint consistency (157 pairs remain unrepaired), but it lowers the secondary normalized either-correct metric. The `-ovati→-uje` present stem is exported and unit-tested grammar metadata, not part of this infinitive-pair accuracy metric; the paired table below discloses that tradeoff rather than relabeling it as a universal accuracy gain.


## Dev / holdout

| model / split | n | normalized both correct | normalized either correct | fingerprint consistency |
|---|---:|---:|---:|---:|
| baseline dev | 1006 | 17.10% | 49.90% | 77.93% |
| baseline holdout | 431 | 15.55% | 48.03% | 78.89% |
| suffix rung dev | 1006 | 17.99% | 49.40% | 87.18% |
| suffix rung holdout | 431 | 16.24% | 47.33% | 88.17% |
| prefix rung dev | 1006 | 18.39% | 49.30% | 88.87% |
| prefix rung holdout | 431 | 16.71% | 47.33% | 89.56% |
| secondary experimental dev | 1006 | 18.49% | 49.01% | 91.25% |
| secondary experimental holdout | 431 | 16.71% | 47.33% | 92.11% |
| production dev | 1006 | 18.39% | 49.30% | 88.87% |
| production holdout | 431 | 16.71% | 47.33% | 89.56% |

## Paired significance vs independent baseline

| metric | fixed | broke | two-sided sign-test p |
|---|---:|---:|---:|
| normalized both correct | 18 | 0 | 0.0000 |
| normalized either correct | 3 | 12 | 0.0352 |

## Rule census

- `independent-roots-agree`: 1124
- `ipf-ati-to-pf-nuti`: 34
- `ipf-jati-to-pf-iti`: 2
- `pf-iti-to-ipf-jati`: 88
- `pf-nuti-to-ipf-ati`: 2
- `prefix-perfectivization`: 30
- `unrepaired`: 157

## Changed-pair sample

- mamiti ↔ omamiti: mamiti / obmanuti → mamiti / obmamiti (prefix-perfectivization)
- viti ↔ sviti: viti / aplesci → viti / aplesci (unrepaired)
- naduživati ↔ nadužiti: zneuživati / zlupotrebiti → zlupotrebjati / zlupotrebiti (pf-iti-to-ipf-jati)
- zloupotrěbjati ↔ zloupotrěbiti: zneuživati / zlupotrebiti → zlupotrebjati / zlupotrebiti (pf-iti-to-ipf-jati)
- sȯvŕšati ↔ sȯvŕšiti: dovršovati / soveršiti → soveršjati / soveršiti (pf-iti-to-ipf-jati)
- nastavjati ↔ nastaviti: regulavac / nastaviti → regulavac / nastaviti (unrepaired)
- dopušćati ↔ dopustiti: udelovati / dapuscic → udelovati / dapuscic (unrepaired)
- odrađati ↔ odraditi: otgovarivati / odraditi → odradjati / odraditi (pf-iti-to-ipf-jati)
- odčuđati ↔ odčuđiti: otčuždati / otdeliti → otčuždati / otčuždnųti (ipf-ati-to-pf-nuti)
- sŕditi ↔ råzsŕditi: sŕditi / råzrditi → sŕditi / råzsŕditi (prefix-perfectivization)
- obvěšćati ↔ obvěstiti: oznamovati / obhaneti → oznamovati / obhaneti (unrepaired)
- prědstavati ↔ prědstati: postaviti sę / stati → postaviti sę / stati (unrepaired)
- odobrjati ↔ odobriti: shvalovati / odobriti → odobrjati / odobriti (pf-iti-to-ipf-jati)
- uręđati ↔ uręditi: uporadočivati / usporadati → uporadočivati / usporadati (unrepaired)
- pytati ↔ spytati: pytati / spytac → pytati / spytati (prefix-perfectivization)
- sprašati ↔ sprositi: sprašivati / požadati → sprašivati / požadati (unrepaired)
- zapytyvati ↔ zapytati: pytati / zapytac → pytati / zapytati (prefix-perfectivization)
- uvěrjati ↔ uvěriti: ujištovati / uveriti → uverjati / uveriti (pf-iti-to-ipf-jati)
- ověrjati ↔ ověriti: overovati / podtverditi → podtverdjati / podtverditi (pf-iti-to-ipf-jati)
- upȯlnomoćevati ↔ upȯlnomoćiti: zplnomonjovati / upolnomočiti → zplnomonjovati / upolnomočiti (unrepaired)
- izběgati ↔ izběgti: unikac / vyhnout se → unikac / vyhnout se (unrepaired)
- bajati ↔ nabajati: bajiti / naboltati → bajiti / naboltati (unrepaired)
- vȯzkresati ↔ vȯzkresnųti: voskresati / vȯzkrisnųti → vȯzkrisati / vȯzkrisnųti (pf-nuti-to-ipf-ati)
- odbivati ↔ odbiti: odražati / adbic → odražati / adbic (unrepaired)
- vȯzrastati ↔ vȯzråsti: odrastati / vyrasti → odrastati / vyrasti (unrepaired)
- mlåděti ↔ omlåděti: mladnouti / pomladěti → mladnouti / pomladěti (unrepaired)
- umoljati ↔ umoliti: prositi / umoliti → prositi / uprositi (prefix-perfectivization)
- načinati ↔ načęti: počinjati / začęti → počinjati / začęti (unrepaired)
- začinati ↔ začęti: počinjati / začęti → počinjati / začęti (unrepaired)
- ostavjati ↔ ostaviti: zaveštavati / odkazati → zaveštavati / odkazati (unrepaired)
- urěkati ↔ urěkti: zaklinati / ureći → zaklinati / zaklinnųti (ipf-ati-to-pf-nuti)
- odkųšati ↔ odkųsiti: odgrizati / otkusiti → odgrizati / odgriznųti (ipf-ati-to-pf-nuti)
- pozajmati ↔ pozajęti: zaimstvovati / požityčiti → požityčjati / požityčiti (pf-iti-to-ipf-jati)
- prědavati ↔ prědati: vysilati / prědati → vysilati / prědati (unrepaired)
- budovati ↔ izbudovati: stavjati / vibudovati → stavjati / staviti (ipf-jati-to-pf-iti)
- obrěmenjati ↔ obrěmeniti: zatežkavati / obremeniti → zatežkavati / obremeniti (unrepaired)
- obtęžati ↔ obtęžiti: obtažovati / obremeniti → obtažovati / obremeniti (unrepaired)
- žegti ↔ izgorěti: paliti / izgoreti → paliti / izgoreti (unrepaired)
- prskati ↔ prsknųti: praskati / lopnuti → praskati / prasknųti (ipf-ati-to-pf-nuti)
- kalkulovati ↔ izkalkulovati: kalkulavac / viličic → kalkulavac / viličic (unrepaired)
