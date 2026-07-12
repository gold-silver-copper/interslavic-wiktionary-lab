# Aspect-pair benchmark (aspect-eval)

**Pre-registered denominator:** 1440 deterministic 1:1 same-gloss, morphologically-related official ipf↔pf pairs (ordered manifest `aspect-pairs.tsv`, FNV-1a-64 `5ab3e19ec5d758dd`). **Keep metrics:** both-correct (primary), either-correct, and pairing-correct (generated roots agree). **Leakage:** official aspect/gloss/root spelling selects the evaluation slice only; both baseline forms are independently generated from cognate cells, and pair repair sees only those generated forms plus their scores. The shared seeded hash holds out 432 pairs.

| model | n | both correct | either correct | pairing correct |
|---|---:|---:|---:|---:|
| independent baseline | 1440 | 16.60% | 49.44% | 78.06% |
| +core suffix repair | 1440 | 17.85% | 47.92% | 94.79% |
| +prefix perfectivization (production) | 1440 | 18.33% | 46.81% | 98.47% |
| +secondary imperfectives (experimental; rejected on primary metric) | 1440 | 18.26% | 47.08% | 98.68% |

The secondary `-yva-/-iva-/-ava-` families are implemented behind `AspectConfig.secondary_imperfectives`, but the rung loses one both-correct pair versus the preceding prefix-production rung, so production leaves the flag off under the project's keep-only-if-it-improves rule. The production repair improves the pre-registered primary **both-correct** metric with no breaks and improves root consistency (22 pairs remain unrepaired), but it lowers the secondary either-correct metric; the paired table below discloses that tradeoff rather than relabeling it as a universal accuracy gain.


## Dev / holdout

| model / split | n | both correct | either correct | pairing correct |
|---|---:|---:|---:|---:|
| baseline dev | 1008 | 17.06% | 50.00% | 77.78% |
| baseline holdout | 432 | 15.51% | 48.15% | 78.70% |
| production dev | 1008 | 19.05% | 47.42% | 98.61% |
| production holdout | 432 | 16.67% | 45.37% | 98.15% |

## Paired significance vs independent baseline

| metric | fixed | broke | two-sided sign-test p |
|---|---:|---:|---:|
| both correct | 25 | 0 | 0.0000 |
| either correct | 3 | 41 | 0.0000 |

## Rule census

- `independent-roots-agree`: 1124
- `ipf-ati-to-pf-nuti`: 62
- `pf-iti-to-ipf-jati`: 79
- `pf-nuti-to-ipf-ati`: 2
- `prefix-perfectivization`: 151
- `unrepaired`: 22

## Changed-pair sample

- mamiti ↔ omamiti: mamiti / obmanuti → mamiti / obmamiti (prefix-perfectivization)
- viti ↔ sviti: viti / aplesci → viti / aplesci (unrepaired)
- naduživati ↔ nadužiti: zneuživati / zlupotrebiti → zlupotrebjati / zlupotrebiti (pf-iti-to-ipf-jati)
- zloupotrěbjati ↔ zloupotrěbiti: zneuživati / zlupotrebiti → zlupotrebjati / zlupotrebiti (pf-iti-to-ipf-jati)
- sȯvŕšati ↔ sȯvŕšiti: dovršovati / soveršiti → soveršjati / soveršiti (pf-iti-to-ipf-jati)
- nastavjati ↔ nastaviti: regulavac / nastaviti → regulavac / naregulavac (prefix-perfectivization)
- dopušćati ↔ dopustiti: udelovati / dapuscic → udelovati / udelovnųti (ipf-ati-to-pf-nuti)
- odrađati ↔ odraditi: otgovarivati / odraditi → odradjati / odraditi (pf-iti-to-ipf-jati)
- odčuđati ↔ odčuđiti: otčuždati / otdeliti → otdeljati / otdeliti (pf-iti-to-ipf-jati)
- sŕditi ↔ råzsŕditi: sŕditi / råzrditi → sŕditi / råzsŕditi (prefix-perfectivization)
- obvěšćati ↔ obvěstiti: oznamovati / obhaneti → oznamovati / oznamovnųti (ipf-ati-to-pf-nuti)
- prědstavati ↔ prědstati: postaviti sę / stati → postaviti sę / spostaviti sę (prefix-perfectivization)
- odobrjati ↔ odobriti: shvalovati / odobriti → odobrjati / odobriti (pf-iti-to-ipf-jati)
- uręđati ↔ uręditi: uporadočivati / usporadati → uporadočivati / uporadočivnųti (ipf-ati-to-pf-nuti)
- pytati ↔ spytati: pytati / spytac → pytati / spytati (prefix-perfectivization)
- sprašati ↔ sprositi: sprašivati / požadati → sprašivati / posprašivati (prefix-perfectivization)
- zapytyvati ↔ zapytati: pytati / zapytac → pytati / zapytati (prefix-perfectivization)
- uvěrjati ↔ uvěriti: ujištovati / uveriti → uverjati / uveriti (pf-iti-to-ipf-jati)
- ověrjati ↔ ověriti: overovati / podtverditi → overovati / podoverovati (prefix-perfectivization)
- upȯlnomoćevati ↔ upȯlnomoćiti: zplnomonjovati / upolnomočiti → upolnomočjati / upolnomočiti (pf-iti-to-ipf-jati)
- izběgati ↔ izběgti: unikac / vyhnout se → unikac / vyunikac (prefix-perfectivization)
- bajati ↔ nabajati: bajiti / naboltati → bajiti / nabajiti (prefix-perfectivization)
- vȯzkresati ↔ vȯzkresnųti: voskresati / vȯzkrisnųti → vȯzkrisati / vȯzkrisnųti (pf-nuti-to-ipf-ati)
- odbivati ↔ odbiti: odražati / adbic → odražati / odražnųti (ipf-ati-to-pf-nuti)
- vȯzrastati ↔ vȯzråsti: odrastati / vyrasti → odrastati / vyodrastati (prefix-perfectivization)
- mlåděti ↔ omlåděti: mladnouti / pomladěti → mladnouti / pomladnouti (prefix-perfectivization)
- umoljati ↔ umoliti: prositi / umoliti → prositi / uprositi (prefix-perfectivization)
- načinati ↔ načęti: počinjati / začęti → počinjati / započinjati (prefix-perfectivization)
- začinati ↔ začęti: počinjati / začęti → počinjati / započinjati (prefix-perfectivization)
- ostavjati ↔ ostaviti: zaveštavati / odkazati → zaveštavati / odzaveštavati (prefix-perfectivization)
- urěkati ↔ urěkti: zaklinati / ureći → zaklinati / uzaklinati (prefix-perfectivization)
- odkųšati ↔ odkųsiti: odgrizati / otkusiti → otkusjati / otkusiti (pf-iti-to-ipf-jati)
- pozajmati ↔ pozajęti: zaimstvovati / požityčiti → požityčjati / požityčiti (pf-iti-to-ipf-jati)
- prědavati ↔ prědati: vysilati / prědati → vysilati / prěvysilati (prefix-perfectivization)
- budovati ↔ izbudovati: stavjati / vibudovati → stavjati / vstavjati (prefix-perfectivization)
- obrěmenjati ↔ obrěmeniti: zatežkavati / obremeniti → zatežkavati / obzatežkavati (prefix-perfectivization)
- obtęžati ↔ obtęžiti: obtažovati / obremeniti → obtažovati / obtažovnųti (ipf-ati-to-pf-nuti)
- žegti ↔ izgorěti: paliti / izgoreti → paliti / izpaliti (prefix-perfectivization)
- prskati ↔ prsknųti: praskati / lopnuti → praskati / prasknųti (ipf-ati-to-pf-nuti)
- kalkulovati ↔ izkalkulovati: kalkulavac / viličic → kalkulavac / vkalkulavac (prefix-perfectivization)
