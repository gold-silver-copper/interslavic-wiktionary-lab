# Aspect-pair benchmark (aspect-eval)

**Frozen reproducible denominator:** 1440 deterministic 1:1 same-gloss, morphologically-related official ipf↔pf pairs (ordered manifest `aspect-pairs.tsv`, FNV-1a-64 `5ab3e19ec5d758dd`). **Keep metrics:** both-correct (primary), either-correct, and pairing-correct (generated roots agree). **Leakage:** official aspect/gloss/root spelling selects the evaluation slice only; both baseline forms are independently generated from cognate cells, and pair repair sees only those generated forms plus their scores. The shared seeded hash holds out 432 pairs.

| model | n | both correct | either correct | pairing correct |
|---|---:|---:|---:|---:|
| independent baseline | 1440 | 16.60% | 49.44% | 78.06% |
| +core suffix repair | 1440 | 17.64% | 48.68% | 89.72% |
| +prefix perfectivization | 1440 | 18.33% | 48.33% | 93.06% |
| +secondary imperfectives and -ovati/-uje (production) | 1440 | 18.40% | 48.19% | 94.65% |

The secondary `-yva-/-iva-/-ava-` and `-ovati/-uje` families are controlled by `AspectConfig.secondary_imperfectives` and retained in production because the final rung improves both-correct over the prefix rung on dev and holdout. The production repair improves the declared primary **both-correct** metric with no breaks and improves root consistency (77 pairs remain unrepaired), but it lowers the secondary either-correct metric; the paired table below discloses that tradeoff rather than relabeling it as a universal accuracy gain.


## Dev / holdout

| model / split | n | both correct | either correct | pairing correct |
|---|---:|---:|---:|---:|
| baseline dev | 1008 | 17.06% | 50.00% | 77.78% |
| baseline holdout | 432 | 15.51% | 48.15% | 78.70% |
| production dev | 1008 | 18.95% | 48.71% | 94.54% |
| production holdout | 432 | 17.13% | 46.99% | 94.91% |

## Paired significance vs independent baseline

| metric | fixed | broke | two-sided sign-test p |
|---|---:|---:|---:|
| both correct | 26 | 0 | 0.0000 |
| either correct | 3 | 21 | 0.0003 |

## Rule census

- `closed-suppletive-pair`: 3
- `independent-roots-agree`: 1124
- `ipf-ati-to-pf-nuti`: 39
- `ipf-jati-to-pf-iti`: 1
- `ovati-to-secondary-ovyvati`: 2
- `pf-iti-to-ipf-jati`: 88
- `pf-nuti-to-ipf-ati`: 2
- `prefix-perfectivization`: 77
- `secondary-ipf-avati`: 13
- `secondary-ipf-avati-reverse`: 1
- `secondary-ipf-ivati`: 5
- `secondary-ipf-ivati-reverse`: 2
- `secondary-ipf-yvati`: 3
- `secondary-ipf-yvati-reverse`: 2
- `secondary-ovyvati-to-ovati`: 1
- `unrepaired`: 77

## Changed-pair sample

- mamiti ↔ omamiti: mamiti / obmanuti → mamiti / obmamiti (prefix-perfectivization)
- viti ↔ sviti: viti / aplesci → viti / aplesci (unrepaired)
- naduživati ↔ nadužiti: zneuživati / zlupotrebiti → zlupotrebjati / zlupotrebiti (pf-iti-to-ipf-jati)
- zloupotrěbjati ↔ zloupotrěbiti: zneuživati / zlupotrebiti → zlupotrebjati / zlupotrebiti (pf-iti-to-ipf-jati)
- sȯvŕšati ↔ sȯvŕšiti: dovršovati / soveršiti → soveršjati / soveršiti (pf-iti-to-ipf-jati)
- nastavjati ↔ nastaviti: regulavac / nastaviti → regulavac / naregulavac (prefix-perfectivization)
- dopušćati ↔ dopustiti: udelovati / dapuscic → udelovati / dapuscic (unrepaired)
- odrađati ↔ odraditi: otgovarivati / odraditi → odradjati / odraditi (pf-iti-to-ipf-jati)
- odčuđati ↔ odčuđiti: otčuždati / otdeliti → otčuždati / otčuždnųti (ipf-ati-to-pf-nuti)
- sŕditi ↔ råzsŕditi: sŕditi / råzrditi → sŕditi / råzsŕditi (prefix-perfectivization)
- obvěšćati ↔ obvěstiti: oznamovati / obhaneti → oznamovati / oznamovnųti (ipf-ati-to-pf-nuti)
- prědstavati ↔ prědstati: postaviti sę / stati → stavati / stati (secondary-ipf-avati)
- odobrjati ↔ odobriti: shvalovati / odobriti → odobrjati / odobriti (pf-iti-to-ipf-jati)
- uręđati ↔ uręditi: uporadočivati / usporadati → usporadivati / usporadati (secondary-ipf-ivati)
- pytati ↔ spytati: pytati / spytac → pytati / spytati (prefix-perfectivization)
- sprašati ↔ sprositi: sprašivati / požadati → sprašivati / posprašivati (prefix-perfectivization)
- zapytyvati ↔ zapytati: pytati / zapytac → pytati / zapytati (prefix-perfectivization)
- uvěrjati ↔ uvěriti: ujištovati / uveriti → uverjati / uveriti (pf-iti-to-ipf-jati)
- ověrjati ↔ ověriti: overovati / podtverditi → podtverdjati / podtverditi (pf-iti-to-ipf-jati)
- upȯlnomoćevati ↔ upȯlnomoćiti: zplnomonjovati / upolnomočiti → zplnomonjovati / uzplnomonjovati (prefix-perfectivization)
- izběgati ↔ izběgti: unikac / vyhnout se → unikac / vyunikac (prefix-perfectivization)
- bajati ↔ nabajati: bajiti / naboltati → bajiti / nabajiti (prefix-perfectivization)
- vȯzkresati ↔ vȯzkresnųti: voskresati / vȯzkrisnųti → vȯzkrisati / vȯzkrisnųti (pf-nuti-to-ipf-ati)
- odbivati ↔ odbiti: odražati / adbic → odražati / adbic (unrepaired)
- vȯzrastati ↔ vȯzråsti: odrastati / vyrasti → odrastati / vyrasti (unrepaired)
- mlåděti ↔ omlåděti: mladnouti / pomladěti → mladnouti / pomladěti (unrepaired)
- umoljati ↔ umoliti: prositi / umoliti → prositi / uprositi (prefix-perfectivization)
- načinati ↔ načęti: počinjati / začęti → počinjati / začęti (unrepaired)
- začinati ↔ začęti: počinjati / začęti → počinjati / začęti (unrepaired)
- ostavjati ↔ ostaviti: zaveštavati / odkazati → zaveštavati / odzaveštavati (prefix-perfectivization)
- urěkati ↔ urěkti: zaklinati / ureći → zaklinati / uzaklinati (prefix-perfectivization)
- odkųšati ↔ odkųsiti: odgrizati / otkusiti → odgrizati / odgriznųti (ipf-ati-to-pf-nuti)
- pozajmati ↔ pozajęti: zaimstvovati / požityčiti → požityčjati / požityčiti (pf-iti-to-ipf-jati)
- prědavati ↔ prědati: vysilati / prědati → prědivati / prědati (secondary-ipf-ivati)
- budovati ↔ izbudovati: stavjati / vibudovati → stavjati / vstavjati (prefix-perfectivization)
- obrěmenjati ↔ obrěmeniti: zatežkavati / obremeniti → zatežkavati / obzatežkavati (prefix-perfectivization)
- obtęžati ↔ obtęžiti: obtažovati / obremeniti → obtažovati / obtažovnųti (ipf-ati-to-pf-nuti)
- žegti ↔ izgorěti: paliti / izgoreti → paliti / izpaliti (prefix-perfectivization)
- prskati ↔ prsknųti: praskati / lopnuti → praskati / prasknųti (ipf-ati-to-pf-nuti)
- kalkulovati ↔ izkalkulovati: kalkulavac / viličic → kalkulavac / vkalkulavac (prefix-perfectivization)
