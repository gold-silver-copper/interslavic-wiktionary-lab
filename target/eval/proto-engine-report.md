# Proto-Slavic engine benchmark

Isolates `proto::generate_with_reflexes` from linking/ranking/consensus: derive the form straight from the linked reconstruction and compare to the official lemma.

- Benchmark entries with modern evidence: **16300**
- Confidently linked to a Proto-Slavic entry: **2862** (17.6% coverage)
- On the linked subset: **exact 43.50%**, **normalized 48.29%**

## Proto-engine accuracy by POS (linked subset)

| POS | linked | exact | normalized |
|---|---:|---:|---:|
| adj | 322 | 28.88% | 31.06% |
| adv | 66 | 10.61% | 15.15% |
| noun | 1639 | 49.97% | 56.68% |
| num | 16 | 18.75% | 25.00% |
| pron | 40 | 75.00% | 77.50% |
| verb | 779 | 37.61% | 39.54% |

## Confident proto-engine errors (sample)

| gloss | official | proto form | *reconstruction | link conf |
|---|---|---|---|---:|
| brother | brat | bratr | *bratrъ | 0.93 |
| poplar | topolja | topolj | *topolь | 0.90 |
| Danube | Dunaj | Dunav | *Dunavь | 0.89 |
| to you (sg.), to thee | tobě | tebě | *tebě | 0.89 |
| trough | žlěb | koryto | *koryto | 0.89 |
| there | tam | tamo | *tamo | 0.87 |
| plank, board | dȯska | daska | *dъska | 0.85 |
| worm | črvjak | čŕv | *čьrvь | 0.85 |
| stupidity | durnosť | glupost | *glupostь | 0.85 |
| willow | iva | vŕba | *vьrba | 0.85 |
| sign | oznaka | znak | *znakъ | 0.85 |
| flight | polet | let | *letъ | 0.85 |
| navel | pųpȯk | pųp | *pǫpъ | 0.85 |
| brotherhood | bratstvo | bratrstvo | *bratrьstvo | 0.85 |
| mill | mlyn | mlin | *mъlinъ | 0.83 |
| hornbeam | grab | grabr | *grabrъ | 0.81 |
| yoke | jaŕmo | aramo | *arьmo | 0.81 |
| goodness | dobrosť | dobrota | *dobrota | 0.81 |
| infertile, barren | jalovy | jalov | *jalovъ | 0.81 |
| lie down | legti | leći | *leťi | 0.81 |
| hunter | lovitelj | lovec | *lovьcь | 0.81 |
| resistance | odpor | otpor | *otъporъ | 0.81 |
| ram | oven | baran | *baranъ | 0.81 |
| lead | svinec | olovo | *olovo | 0.81 |
| today | tutdenj | danes | *dьnьsь | 0.81 |
| basis | zaklad | osnova | *osnova | 0.81 |
| burn | žegti | paliti | *paliti | 0.81 |
| grain | žito | zŕno | *zьrno | 0.81 |
| violence | nasiľje | nasilije | *nasilьje | 0.80 |
| spear | kopje | kopije | *kopьje | 0.79 |
| star | zvězda | gvězda | *gvězda | 0.79 |
| shock | šok | sok | *sokъ | 0.78 |
| ash tree | jasenj | asenj | *asenь | 0.78 |
| prince | knęź | knędz | *kъnędzь | 0.78 |
| thread | nitka | nit | *nitь | 0.78 |
| egg | jajce | ajace | *ajьce | 0.78 |
| spider | pavųk | paųk | *paǫkъ | 0.78 |
| herd, flock, drove | črěda | stado | *stado | 0.77 |
| richness, wealth | bogatosť | bogatstvo | *bogatьstvo | 0.77 |
| fart | bzděti | pŕděti | *pьrděti | 0.77 |
| child | čędo | dětę | *dětę | 0.77 |
| limb | člen | ud | *udъ | 0.77 |
| chisel | dlåto | dlěto | *delto | 0.77 |
| friendship | družba | družaba | *družьba | 0.77 |
| boldness, audacity | dŕzosť | smělost | *sъmělostь | 0.77 |
| mind | duh | um | *umъ | 0.77 |
| ready, prepared | gotovy | gotov | *gotovъ | 0.77 |
| fist | grsť | pęst | *pęstь | 0.77 |
| scab | kråsta | strup | *strupъ | 0.77 |
| hazel | leščina | lěska | *lěska | 0.77 |
| hunt | lovitva | lov | *lovъ | 0.77 |
| send to | prislati | poslati | *posъlati | 0.77 |
| bitch (female dog) | psica | suka | *suka | 0.77 |
| word | rěč | slovo | *slovo | 0.77 |
| make turbid | smųćati | mųtiti | *mǫtiti | 0.77 |
| pine | sosna | bor | *borъ | 0.77 |
| bottom | spod | dno | *dъno | 0.77 |
| lighthouse | světiľnik | majak | *majakъ | 0.77 |
| width | širokosť | širina | *širina | 0.77 |
| calf | telętko | telę | *telę | 0.77 |
