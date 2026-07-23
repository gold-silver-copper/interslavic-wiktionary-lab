# Multi-word & aspect-pair benchmark (multiword-eval)

**Denominators:** 1837 multi-word official lemmas (561 reflexive `X sę`, 1083 two-token, 193 longer — the headline benchmark excludes all of them); 1440 morphologically related 1:1 aspect pairs (of 1486 gloss-matched candidates). **Leakage story:** the gold `isv` only selects the slice; generation sees the cognate cells + POS/gender, as in the headline benchmark. **Dev/holdout (seeded id split, normalized, over the scored subsets):** reflexive 31.32%/29.23%, two-token 17.64%/17.97%.

| Slice | n | exact | normalized |
|---|---:|---:|---:|
| reflexive `X sę` (existing pipeline, newly scored) | 561 | 24.96% | 30.84% |
| — of which no reflexive marker detected in the cognates (structural miss: ` sę` is never appended) | 88 | — | — |
| two-token collocation (per-position reconstruction) | 886 of 1083 generatable | 11.85% | 17.72% |

## Aspect pairs (both members through the standard pipeline)

| outcome | share of 1440 pairs |
|---|---:|
| both correct (normalized) | 16.5% |
| exactly one correct | 32.9% |
| neither | 50.6% |

The two-token heuristic (disclosed): position 1 is reconstructed as an adjective and agreed with the head's gender, position 2 as the entry's own POS — right for the dominant modifier+head class, wrong for adv+adv or verb phrases; 'not generatable' means fewer than 2 cognates cite a two-token form.

## Two-token nearest misses (sample)

- a takože → ja i
- adamovo jablȯko → adamovo jablko
- Adriatičsko morje → jadransko morje
- afrikansky mråvojed → afrikansky mravojad
- akcionerny kapital → akcionerniji kapital
- ako by → jako da
- ako by → esly by
- ako by → vny slučaj
- ako ne → aky ne
- anglijsky rožek → anglisky rog
- animovany film → multiplikacijniji film
- apelacijny sųd → apelacijniji sųd
- Arktičny okean → arktičny ocean
- Atlantičny okean → atlantsky okean
- avtobusna postojka → avtobusna zastavka
- Baltičsko morje → baltijsko morje
- barvna olovka → cvetna karandaš
- bazovati na → zakladaty na
- bez malogo → edvy ne
- běžna dråga → behavaja dražka
- Big Ben → bigy ben
- Bizantijska imperija → vizantijska imperija
- blåtna kųpělj → hrazevaja kųpělj
- blåtna lavina → slevyja potok
- Blizky Iztok → blizky vshod
- Blizky Vȯzhod → blizky vshod
- bližnji, bližny → uy bliziny
- bobŕja damba → bobrova bråna
- bojna glåvica → bojova glåva
- botaničny sad → botaničky sad
- božja kråvka → boža karovka
- Brajlovo pismo → braillovo pismo
- brat bliznec → bratry bliznak
- bratska ljubȯv → bratska ljuby
- bronzova doba → bronzovyja věk
- brza pomoć → hitna pomoć
- bufer obměna → bufery pamęt
- buferna pamęť → bufera pamęt
- byti dȯlžen → byti dolžnym
- byti ostråžny → byti ostrožnym
