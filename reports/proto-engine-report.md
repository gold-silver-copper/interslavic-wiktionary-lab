# Proto-Slavic engine benchmark

Isolates `proto::generate_with_reflexes` from linking/ranking/consensus: derive the form straight from the linked reconstruction and compare to the official lemma.

- Benchmark entries with modern evidence: **16300**
- Confidently linked to a Proto-Slavic entry: **3301** (20.3% coverage)
- On the linked subset: **exact 48.14%**, **normalized 52.74%**

## Proto-engine accuracy by POS (linked subset)

| POS | linked | exact | normalized |
|---|---:|---:|---:|
| adj | 428 | 51.87% | 59.11% |
| adv | 83 | 12.05% | 14.46% |
| noun | 1828 | 52.46% | 58.26% |
| num | 18 | 22.22% | 22.22% |
| pron | 42 | 66.67% | 71.43% |
| verb | 902 | 40.58% | 41.80% |

## Confident proto-engine errors (sample)

| gloss | official | proto form | *reconstruction | link conf |
|---|---|---|---|---:|
| eat | jedati | jesti | *ěsti | 1.00 |
| navel | pųpȯk | pųp | *pǫpъ | 1.00 |
| willow | iva | vŕba | *vьrba | 0.96 |
| ash, ashes | popel | pepel | *pepelъ | 0.96 |
| to you (sg.), to thee | tobě | tebě | *tebě | 0.96 |
| thin | tȯnky | tėnky | *tьnъkъ | 0.96 |
| fart | bzděti | pŕděti | *pьrděti | 0.95 |
| lie down | legti | leći | *leťi | 0.95 |
| find | najdti | najti | *najьti | 0.95 |
| poplar | topolja | topolj | *topolь | 0.94 |
| flea | blȯha | blha | *blъxa | 0.93 |
| brother | brat | bratr | *bratrъ | 0.93 |
| hornbeam | grab | grabr | *grabrъ | 0.93 |
| heart | sŕdce | sŕdece | *sьrdьce | 0.93 |
| mother’s | mamin | maminy | *maminъ | 0.92 |
| drone | trutenj | trųt | *trǫtъ | 0.92 |
| here | tut | tu | *tu | 0.92 |
| trough | žlěb | koryto | *koryto | 0.92 |
| wart | brådavica | brådavika | *bordavъka | 0.92 |
| cherry (sweet) | čerešnja | črěšnja | *čeršьňa | 0.91 |
| there | tam | tamo | *tamo | 0.91 |
| pitchfork | vily | vila | *vidla | 0.91 |
| health | zdråvje | sdråvje | *sъdorvьje | 0.91 |
| burn | žegti | paliti | *paliti | 0.91 |
| Danube | Dunaj | Dunav | *Dunavь | 0.89 |
| spider | pavųk | paųk | *paǫkъ | 0.89 |
| song | pěsnja | pěsnj | *pěsnь | 0.89 |
| aunt | tetka | teta | *teta | 0.89 |
| richness, wealth | bogatosť | bogatstvo | *bogatьstvo | 0.89 |
| sharp | bridky | ostry | *ostrъ | 0.89 |
| worm | črvjak | čŕv | *čьrvь | 0.89 |
| yoke | igo | jarmo | *arьmo | 0.89 |
| apple | jablȯko | jablko | *ablъko | 0.89 |
| flight | polet | let | *letъ | 0.89 |
| four | četyri | četyre | *četyre | 0.88 |
| cough | kašelj | kašȯlj | *kaš(ь)ľь | 0.88 |
| elk, moose | loś | låś | *olsь | 0.88 |
| ant | mråvka | mråv | *morvъ | 0.88 |
| caterpillar | gųsenica | vųsěnica | *ǫsěnica | 0.88 |
| bosom | pazuha | pazduha | *pazduxa | 0.87 |
| tear | sȯlza | slza | *slьza | 0.87 |
| star | zvězda | gvězda | *gvězda | 0.86 |
| fart | bzdnųti | pŕdnųti | *pьrdnǫti | 0.86 |
| dig | grebti | kopati | *kopati | 0.86 |
| shake | hvějati | tręsti | *tręsti | 0.86 |
| make turbid | smųćati | mųtiti | *mǫtiti | 0.86 |
| heron | čaplja | čapja | *čapľa | 0.85 |
| chisel | dlåto | dlěto | *delto | 0.85 |
| stupidity | durnosť | gluposť | *glupostь | 0.85 |
| needle | iglica | igla | *jьgъla | 0.85 |
| moon | luna | měsęc | *měsęcь | 0.85 |
| resistance | odpor | otpor | *otъporъ | 0.85 |
| sign | oznaka | znak | *znakъ | 0.85 |
| here | sde | tu | *tu | 0.85 |
| lead | svinec | olovo | *olovo | 0.85 |
| width | širokosť | širina | *širina | 0.85 |
| today | tutdenj | dnėś | *dьnьsь | 0.85 |
| basis | zaklad | osnova | *osnova | 0.85 |
| grain | žito | zŕno | *zьrno | 0.85 |
| brotherhood | bratstvo | bratrstvo | *bratrьstvo | 0.85 |
