# Proto-Slavic engine benchmark

Isolates `proto::generate_with_reflexes` from linking/ranking/consensus: derive the form straight from the linked reconstruction and compare to the official lemma.

- Benchmark entries with modern evidence: **16300**
- Confidently linked to a Proto-Slavic entry: **2912** (17.9% coverage)
- On the linked subset: **exact 43.99%**, **normalized 49.38%**

## Proto-engine accuracy by POS (linked subset)

| POS | linked | exact | normalized |
|---|---:|---:|---:|
| adj | 327 | 30.58% | 33.03% |
| adv | 65 | 10.77% | 15.38% |
| noun | 1650 | 50.55% | 58.30% |
| num | 16 | 18.75% | 25.00% |
| pron | 40 | 75.00% | 77.50% |
| verb | 814 | 37.71% | 39.68% |

## Confident proto-engine errors (sample)

| gloss | official | proto form | *reconstruction | link conf |
|---|---|---|---|---:|
| brother | brat | bratr | *bratrъ | 0.93 |
| poplar | topolja | topolj | *topolь | 0.90 |
| Danube | Dunaj | Dunav | *Dunavь | 0.89 |
| to you (sg.), to thee | tobě | tebě | *tebě | 0.89 |
| trough | žlěb | koryto | *koryto | 0.89 |
| elk, moose | loś | las | *olsь | 0.88 |
| there | tam | tamo | *tamo | 0.87 |
| lie down | legti | leći | *leťi | 0.86 |
| burn | žegti | paliti | *paliti | 0.86 |
| plank, board | dȯska | daska | *dъska | 0.85 |
| worm | črvjak | čŕv | *čьrvь | 0.85 |
| stupidity | durnosť | glupost | *glupostь | 0.85 |
| willow | iva | vŕba | *vьrba | 0.85 |
| sign | oznaka | znak | *znakъ | 0.85 |
| flight | polet | let | *letъ | 0.85 |
| navel | pųpȯk | pųp | *pǫpъ | 0.85 |
| brotherhood | bratstvo | bratrstvo | *bratrьstvo | 0.85 |
| mill | mlyn | mlin | *mъlinъ | 0.83 |
| spear | kopje | kopije | *kopьje | 0.83 |
| hornbeam | grab | grabr | *grabrъ | 0.81 |
| yoke | jaŕmo | aramo | *arьmo | 0.81 |
| fart | bzděti | pŕděti | *pьrděti | 0.81 |
| send to | prislati | poslati | *posъlati | 0.81 |
| make turbid | smųćati | mųtiti | *mǫtiti | 0.81 |
| goodness | dobrosť | dobrota | *dobrota | 0.81 |
| infertile, barren | jalovy | jalov | *jalovъ | 0.81 |
| hunter | lovitelj | lovec | *lovьcь | 0.81 |
| butcher | męsnik | męsar | *męsarь | 0.81 |
| resistance | odpor | otpor | *otъporъ | 0.81 |
| ram | oven | baran | *baranъ | 0.81 |
| lead | svinec | olovo | *olovo | 0.81 |
| today | tutdenj | danes | *dьnьsь | 0.81 |
| basis | zaklad | osnova | *osnova | 0.81 |
| grain | žito | zŕno | *zьrno | 0.81 |
| violence | nasiľje | nasilije | *nasilьje | 0.80 |
| star | zvězda | gvězda | *gvězda | 0.79 |
| shock | šok | sok | *sokъ | 0.78 |
| ash tree | jasenj | asenj | *asenь | 0.78 |
| prince | knęź | knędz | *kъnędzь | 0.78 |
| thread | nitka | nit | *nitь | 0.78 |
| egg | jajce | ajace | *ajьce | 0.78 |
| spider | pavųk | paųk | *paǫkъ | 0.78 |
| herd, flock, drove | črěda | stado | *stado | 0.77 |
| richness, wealth | bogatosť | bogatstvo | *bogatьstvo | 0.77 |
| child | čędo | dětę | *dětę | 0.77 |
| limb | člen | ud | *udъ | 0.77 |
| work | dělo | rabota | *orbota | 0.77 |
| chisel | dlåto | dlěto | *delto | 0.77 |
| friendship | družba | družaba | *družьba | 0.77 |
| boldness, audacity | dŕzosť | smělost | *sъmělostь | 0.77 |
| mind | duh | um | *umъ | 0.77 |
| ready, prepared | gotovy | gotov | *gotovъ | 0.77 |
| fist | grsť | pęst | *pęstь | 0.77 |
| scab | kråsta | strup | *strupъ | 0.77 |
| hazel | leščina | lěska | *lěska | 0.77 |
| hunt | lovitva | lov | *lovъ | 0.77 |
| bitch (female dog) | psica | suka | *suka | 0.77 |
| word | rěč | slovo | *slovo | 0.77 |
| pine | sosna | bor | *borъ | 0.77 |
| bottom | spod | dno | *dъno | 0.77 |
