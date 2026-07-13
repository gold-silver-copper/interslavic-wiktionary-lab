# Proto-Slavic and Modern-Consensus → Interslavic Candidate Generation Rules

**A single implementable specification for a Rust rule engine.**
Standard/merged Medžuslovjansky (post-2017). Every rule carries a source citation. Where sources conflict, the winner is stated inline with a reason.

**Source tags used throughout:**

| Tag | Source |
|---|---|
| `[ORTH]` | interslavic.fun/learn/orthography/ (van Steenbergen) |
| `[PHON]` | interslavic.fun/learn/phonology/ |
| `[NOUN]` | interslavic.fun/learn/grammar/nouns/ |
| `[WF]` | interslavic.fun/learn/vocabulary/word-formation/ |
| `[DERIV]` | interslavic.fun/learn/vocabulary/derivation/ (the Proto-Slavic→ISV correspondence table) |
| `[DESIGN]` | interslavic.fun/learn/misc/design-criteria/ |
| `[INTRO]` | interslavic.fun/learn/introduction/ |
| `[STEEN-G]` | steen.free.fr grammar cluster (orthography/phonology/nouns/adjectives/verbs.html) |
| `[STEEN-D]` | steen.free.fr derivation.html + flavorizacija.html + design_criteria.html |

**Global design axioms** (drive every downstream decision):
- ISV forms are **never borrowed directly** from a modern Slavic language. The modern-language consensus picks *which root*; the *form* is derived from the Proto-Slavic/OCS reconstruction via the fixed correspondence table. `[DESIGN][STEEN-D][DERIV]`
- **Root-consistency invariant:** the same root must surface identically in every derivative. Normalize a root once, then reuse. `[DERIV]`
- The **etymological (scientific) alphabet is the intermediate representation ("source code").** Generate the etymological form first; the standard lemma is obtained by mechanically stripping diacritics. `[ORTH]`
- A modernized-Proto-Slavic derivation and a modern-comparative derivation "differ only in details"; when they disagree, prefer the reading that maximizes pan-Slavic intelligibility. `[INTRO]`

---

## 1. Target Orthography

### 1.1 Standard alphabets (the final lemma is written in these)

**Standard Latin (27 letters)** `[ORTH][STEEN-G]`
`A B C Č D DŽ E Ě F G H I J K L LJ M N NJ O P R S Š T U V Y Z Ž`
= 23 base-Latin letters (no q, w, x) + carons `š ž č ě` + digraphs `dž lj nj`.

**Standard Cyrillic (29 letters)** `[ORTH][STEEN-G]`
`А Б В Г Д ДЖ Е Є Ж З И Ы Ј К Л Љ М Н Њ О П Р С Т У Ф Х Ц Ч Ш`

Standard orthography **keeps `ě` (jat)** and no other etymological diacritic. `[ORTH]`

### 1.2 The etymological / scientific ("flavored") alphabet — extra letters

Full extra set: **`Ę Ų Å Ė Ȯ   Ć Đ   Ĺ Ń Ŕ T́ D́ Ś Ź`** `[ORTH][STEEN-G][STEEN-D]`

| Etym. letter | Etymological meaning (PSl / OCS source) | → Standard | IPA |
|---|---|---|---|
| `ě` | jat *ě (OCS ѣ) — **kept in standard** | `ě` | ʲɛ |
| `y` | *y (OCS ꙑ/ъи) — **kept in standard** | `y` | i~ɨ |
| `ę` | front nasal *ę (OCS ѧ, little yus) | `e` | ʲæ |
| `ų` | back nasal *ǫ (OCS ѫ, big yus) | `u` | o~ʊ |
| `å` | liquid-diphthong vowel of *(C)orC/*(C)olC | `a` | ɒ |
| `ė` | **strong** front yer *ь (ĭ) | `e` | ɛ~ə |
| `ȯ` | **strong** back yer *ъ (ŭ) | `o` | ə~ʌ |
| `ĺ` | soft *l (l + weak ь), only before a consonant | `l` | ʎ~l |
| `ń` | soft *n (n + weak ь), only before a consonant | `n` | n~ɲ |
| `ŕ` | soft *r (rь/rj) | `r` | rʲ~r̝ |
| `t́` (stored as `ť`) | soft *t (tь) | `t` | tʲ~c |
| `d́` (stored as `ď`) | soft *d (dь) | `d` | dʲ~ɟ |
| `ś` | soft *s (sь) | `s` | sʲ~ɕ |
| `ź` | soft *z (zь) | `z` | zʲ~ʑ |
| `ć` | *tj / *kt(ь) reflex (OCS щ /št/) | `č` | t͡ɕ |
| `đ` | *dj / *gd(ь) reflex (OCS жд /žd/) | `dž` | d͡ʑ |

Note: `ĺ`/`ń` are phonetically identical to `lj`/`nj`; they are simply not written in standard. `[ORTH]`

### 1.3 Etymological → Standard reduction (deterministic strip)

```
ě → ě   (KEPT)      y → y   (KEPT)
ę → e   ų → u       å → a
ė → e   ȯ → o
ĺ → l   ń → n   ŕ → r   t́ → t   d́ → d   ś → s   ź → z
ć → č   đ → dž
g → g   (kept; realized [ɦ]/h in UK/BY/CZ/SK but written g)
```
Rule (verbatim): *"there is never any need to represent them in some other way than by simply leaving out the diacritic; the only exceptions are ć and đ, which in standard orthography should be replaced with č and dž."* `[ORTH]`

**Implementation notes:**
- Letters come in obligatory pairs: `ę/ų`, `ė/ȯ`, `t́/d́`, `ś/ź`. If one is emitted, emit its partner consistently. `[ORTH]`
- The prose notation `t́/d́` denotes soft *t/*d. Repository wire/storage form follows the official dictionary and uses the precomposed glyphs `ť/ď`; `orthography::to_standard` and exported APIs likewise treat `ť/ď` as canonical flavored input. Other display fallbacks when a glyph is unavailable include `ĺ→ľ`, `ė→è`, `ȯ→ò`. `[ORTH][STEEN-G]`
- The etymological alphabet has **no Cyrillic form** and **no letters for non-Slavic borrowings** (no OCS ѳ/ѵ, no ü) and **no length/tone marks.** `[ORTH]`
- Standard hard-to-type fallbacks (NOT etymological): `č š ž → cz sz ż` (or `cx sx zx`); `ě→e`; `y→i`; Cyrillic `ј→й`, `љ→ль`, `њ→нь`. `[ORTH]`
- **Conflict — `dž` ambiguity:** etymological `đ` (from *dj) reduces to `dž`, but native/loan `dž` (e.g. `budžet`, `džaz`, `menedžer`) is also written `dž`. **Resolution:** in the *etymological* representation, *dj* reflexes must be spelled `đ` and loan/native `dž` spelled `dž`; both collapse to standard `dž`. The engine must track provenance on the etymological layer, not the standard layer. `[PHON][STEEN-G]`

---

## 2. Proto-Slavic → ISV — Ordered Rule List

Rules fire in the order listed. **Ordering rationale:** yer-strength must be assigned (Havlík) *before* deletion/vocalization; liquid metathesis and *tj/*dj cluster resolution operate on the intact PSl string and therefore run before yer deletion collapses clusters. Output is the **etymological** form; then apply §1.3 to get the standard lemma.

Each rule: **id** — condition — `before → after (etym / std)` — example — source.

### Phase A — Prosody / cluster setup

**`assign-yer-strength`** — Assign strong/weak to every yer by Havlík's law: counting from the word end, the last yer in a run is weak; then alternate strong/weak leftward. A yer before a syllable containing another yer is strong; a final yer is weak. Weak yers are deleted (Phase E), strong yers vocalize (Phase E).
`ь/ъ → {strong|weak}` — *sъnъ*: final ъ weak, first ъ strong → `sȯn`/`son`. `[STEEN-G][DERIV expert-note: only strong yers tabulated; weak yers drop]`

**`liquid-metathesis-back`** (TorT/TolT) — o-grade liquid diphthong between consonants → `rå`/`lå` (std `ra`/`la`). Runs before yer-fall.
`(C)orC → (C)råC`, `(C)olC → (C)låC` — *golva → glåva/glava; gordъ → gråd/grad; korva → kråva/krava; moldъ → mlådy/mlady`. `[DERIV][ORTH]`

**`liquid-metathesis-front`** (TerT/TelT) — e-grade liquid diphthong → `rě`/`lě` (kept in std).
`(C)erC → (C)rěC`, `(C)elC → (C)lěC` — *bergъ → brěg; melko → mlěko; perdъ → prěd`. `[DERIV]`

**`initial-orT-olT`** — Word-initial liquid diphthong follows the same TraT/TlaT / TrěT/TlěT outcomes (South-Slavic type). *(Falls under the two rules above; no distinct special letter beyond `å`/`ě`.)* `[ORTH expert-note]`

### Phase B — Palatal cluster resolution (*tj/*dj/*kt/*gt, *sj/*zj)

**`tj-kt-to-c-caron`** — *tj and *kt/*gt before a front vowel → `ć` (std `č`).
`tj, ktь, gtь → ć / č` — *světja → svěća/svěča; *noktь → noć/noč; *pektь → (verb kept transparent, see note)*. `[DERIV][DESIG N][PHON]`

**`dj-to-d-bar`** — *dj (and *gd) → `đ` (std `dž`).
`dj, gdь → đ / dž` — *medja → među/medžu; *gordja → …đ/…dž`. `[DERIV][DESIGN]`
> **Conflict resolution (tj/dj outcome).** Reflexes diverge across branches (ESl č/ž, PL c/dz, CZ/SK c/z, SL č/j, BCMS ć/đ, MK ḱ/ǵ, BG št/žd). **ISV picks `č`/`dž`** — chosen by `[DESIGN]` as "the most intermediary and most regular solution," with the bonus that `dž` is the voiced pair of `č`. The etymological layer records the compromise as `ć`/`đ`. Winner: `[DESIGN]`/`[DERIV]` (explicit, authoritative) over any single-branch reflex.

**`kt-gt-verb-exception`** — In verb infinitives ISV keeps the transparent cluster instead of the *kt→č outcome: `mogti, pekti, běgti` (not moči/peči/běči). `[STEEN-G verbs]`

**`sj-zj-to-hushers`** — *sj → `š`, *zj → `ž` (everywhere; same in both alphabets).
`sj → š`, `zj → ž` — *prosjǫ → prošų/prošu; *tęzjenьje → tęženje/teženje`. `[DERIV][PHON]`

**`cluster-iotation`** — *stj/*skj → `šč` (etym `šć`), *zdj/*zgj → `ždž` (etym `žđ`).
`stj → šč`, `zdj → ždž` — surfaces in derivation/conjugation. `[PHON][STEEN-G]`

**`labial-plus-j`** — Labials p/b/m/v/f + j stay hard, written `pj bj mj vj fj` (ISV does **not** insert epenthetic l, unlike ESl/SSl `-plj-`).
`pj → pj` — *kupjǫ → kupjų/kupju; *zemja → zemja`. `[DERIV][PHON]`
> **Conflict resolution.** ESl/SSl have `plj/blj`, WSl have `p/b`. ISV writes `pj/bj` "for reasons of clarity and regularity." Winner: `[STEEN-G]`/`[DERIV]`.

### Phase C — Nasal vowels

**`front-nasal`** — *ę → `ę` (std `e`).
`ę → ę / e` — *językъ → język/jezyk; *svętъ → svęty/svety; *pęть → pęt́/pet`. `[DERIV][ORTH]`

**`back-nasal`** — *ǫ → `ų` (std `u`). **Note the asymmetry: only the *front* nasal maps to an `ę`-type letter; *ǫ maps to `ų`/`u`, never `ę`.**
`ǫ → ų / u` — *rǫka → rųka/ruka; *pǫtь → pųt́/put`. `[DERIV][ORTH]`

**`initial-back-nasal-prothesis`** — Word-initial *ǫ- → `vų-` (std `vu-`).
`ǫ- → vų- / vu-` — *ǫtroba → vųtroby/vutroby; *(pa)ǫkъ → pavųk/pavuk`. `[DERIV]`

### Phase D — Jat

**`jat`** — *ě → `ě` (kept in standard; softens the preceding consonant in ~96% of speakers).
`ě → ě` — *světъ → svět; *rěka → rěka; *město → město`. `[DERIV][ORTH][PHON]`
> `ě` always follows a hard consonant; legitimate simplification `ě→e`. In fem. dat/loc sg, `ě→i` after a soft stem (see §3). `[STEEN-G]`

### Phase E — Yer resolution

**`strong-front-yer`** — Strong *ь → `ė` (std `e`).
`ьSTRONG → ė / e` — *otьcь → otėc/otec; *pьsъ → pės/pes`. `[DERIV][ORTH][PHON]`

**`strong-back-yer`** — Strong *ъ → `ȯ` (std `o`).
`ъSTRONG → ȯ / o` — *sъnъ → sȯn/son; *pěsъkъ → pěsȯk/pěsok`. `[DERIV][ORTH]`
> **Conflict resolution.** A `[DESIGN]` expert note claimed strong *ъ→e. This is **wrong**; the authoritative correspondence table `[DERIV]` gives strong *ъ→ȯ→o and strong *ь→ė→e. Winner: `[DERIV]` (explicit table) over the expert gloss. (West-Slavic *flavour* may render `ȯ` as `e`, but that is flavourisation, not standard.) `[STEEN-D]`

**`weak-yer-deletion`** — Weak *ь and *ъ are deleted with no reflex. This is what creates fleeting/mobile vowels: the strong-yer vowel appears in nom.sg but the weak yer in an oblique form drops (`otėc → gen otca`; `pės → gen psa`; `sȯn → gen sna`). `[STEEN-G][PHON]`

### Phase F — Soft consonants (consonant + weak/lost ь) and syllabic liquids

**`soft-consonant-marking`** — A consonant palatalized by a following (now-lost) front yer is written with its soft letter, which reduces to the plain consonant in standard.
`tь→t́/t, dь→d́/d, sь→ś/s, zь→ź/z, lь→ĺ/l (→lj), nь→ń/n (→nj), rь→ŕ/r` — *kostь → `kost́` (repository glyph fallback `kosť`)/`kost`; *lъžь…; *dъždь → `dȯžd́` (fallback `dȯžď`)/`dožd`; *losь → loś/los; *knęzь → knęź/knez`. `[DERIV][ORTH]`

**`palatal-l-n`** — *lь/lj → `lj`, *nь/nj → `nj` (kept as digraphs in both alphabets; `ĺ/ń` only pre-consonantally).
`lj → lj`, `nj → nj` — *ljubiti → ljubiti; *dьnь → denj`. `[DERIV]`

**`soft-r-j`** — *rь/*rj → `ŕ` (std `r`) or `rj`.
`rь → ŕ / r`, `rj → rj` — *carь → caŕ/car; *tvorjenьje → tvorjenje`. `[DERIV]`

**`syllabic-r-hard`** — *CъrC → syllabic `r` (kept `r` in both).
`CъrC → r` — *tъrgъ → trg; *kъrčьma → krčma`. `[DERIV]`

**`syllabic-r-soft`** — *CьrC → `ŕ` (std `r`), syllabic.
`CьrC → ŕ / r` — *dьržati → dŕžati/držati; *sьmьrtь → smŕt́/smrt`. `[DERIV]`

**`syllabic-l`** — *CъlC and *CьlC → `ȯl` (std `ol`).
`CъlC/CьlC → ȯl / ol` — *dъlgъ → dȯlg/dolg; *tьlstъ → tȯlsty/tolsty; *vьlkъ → vȯlk/volk`. `[DERIV]`

**`palatalized-je-clusters`** — Soft consonant + `-je`/`-ьje` sequences: `-ĺje/-ńje/-ŕje/-t́je/-d́je/-śje/-źje` (std `-lje/-nje/-rje/-tje/-dje/-sje/-zje`).
— *usilьje → usiĺje/usilje; *dělanьje → dělańje/dělanje; *žitьje → žit́je/žitje; *orǫdьje → orųd́je/orudje`. `[DERIV]`

### Phase G — Consonant retentions

**`tl-dl-simplification`** — *tl/*dl → `l`.
`tl,dl → l` — *modliti → moliti; *gърdlo → grlo`. `[DERIV]`
> Conflict: WSl keeps `tl/dl`, ESl/SSl → `l`. ISV → `l`. Winner: `[DERIV]`.

**`g-retention`** — *g stays `g` (written g even though realized [ɦ]/h in UK/BY/CZ/SK).
`g → g` — *golva → glåva/glava; *jego → jego`. `[DERIV]`
> **No g→h spirantization rule.** `[ORTH][DERIV]`

**`x-as-h`** — PSl *x is written with the letter `h` (= Cyrillic х), value [x]. There is no separate `x` letter; `g`≠`h`.
`*x → h` — *xvala → hvala; *duxъ → duh`. `[ORTH][PHON][STEEN-G]`
> Note: the `[DERIV]` table has no explicit *x row for native words; the *x→h identity comes from the alphabet/inventory `[ORTH][PHON]`. For internationalisms the [x]/`ch` sound → `h` (see §5).

**`sc-cluster`** — *šč (*skj/*stj type) kept as `šč`.
`šč → šč` — *ščetъka → ščetka`. `[DERIV]`

### Phase H — Palatalizations (velars)

**`first-palatalization`** — Velars k/g/h (and c) → `č/ž/š` before a front vowel or *j, in the historically-conditioned contexts. In ISV this is **synchronically live only** in: masc. vocative sg before `-e`; present-tense before `-e/-eš`; derived `-i-` verbs; and before the suffixes `-an(in), -ba, -ec, -ica, -ina, -išče, -je, -ji, -nik, -ny, -ok/-ka/-ko, -sky, -stvo`.
`k→č, g→ž, h→š, c→č` — *Bogъ voc → Bože; *pekti pres → pečeš; *rǫka+ny → rųčny; *muxa+ji → mušji`. `[PHON][NOUN][WF][STEEN-G]`

**`no-second-palatalization-in-inflection`** — **Critical negative rule:** apart from the vocative, velars are **never** palatalized in noun/adjective/pronoun declension. No 2nd/regressive palatalization in plurals.
`Čeh → pl. Čehi` (NOT *Česi); `dȯlgy → pl. dȯlgi/dȯlge`. `[PHON][STEEN-G]`
> This overrides any naive historical 2nd-palatalization expectation. Winner: `[STEEN-G]`/`[PHON]` (explicit).

### Not covered by sources (do not invent)
- Prothetic `j-` (only `v-` prothesis before initial *ǫ- is specified). `[DERIV]`
- General 3rd/progressive palatalization outcomes beyond the above. `[PHON]`

---

## 3. Proto-Slavic Ending → ISV Lemma-Ending Rules (POS-aware)

Citation/lemma conventions `[NOUN][STEEN-G][DESIGN]`:
- **Nouns** — cited in **nominative singular**; dictionary also stores **gender** (+ irregular oblique stem where needed).
- **Adjectives** — cited in **masc. nom. sg.**: hard `-y`, soft `-i`.
- **Verbs** — cited in **infinitive**, always `-ti`.

Soft stems (trigger vowel shifts) end in: `š ž č dž c j lj nj ŕ t́ d́ ś ź ć đ`. `c` is phonetically hard but grammatically soft. `[PHON][STEEN-G]`

### 3.1 Noun lemma-ending rules

**`noun-masc-cons`** — masc. o-stem/consonant-stem → nom.sg **zero ending** (consonant-final). Animacy is lexical (acc.sg = gen for animate, = nom for inanimate).
Ex: *bratъ → brat; *mǫžь → muž; *domъ → dom; *krajь → kraj. `[NOUN][STEEN-G]`

**`noun-neut-o`** — neuter hard o-stem → **`-o`**.
Ex: *slovo → slovo. `[NOUN]`

**`noun-neut-e-soft`** — neuter after a soft consonant → **`-e`** (etym. can be `-ę`).
Ex: *morje → morje. `[NOUN]`

**`noun-fem-a`** — feminine ā-stem → **`-a`**.
Ex: *žena → žena; *zemja → zemja. `[NOUN]`

**`noun-fem-cons`** — feminine i-stem → nom.sg **zero ending** (consonant-final).
Ex: *kostь → `kost́` (repository glyph fallback `kosť`), standard `kost`. The zero ending drops the yer but preserves stem-final softness. `[NOUN][ORTH]`

**`noun-neut-e-athematic`** — neuter athematic `-e`; pick oblique stem by the preceding consonant:
- preceded by `m` → `-men-` stem: `ime` (gen `imene`, pl `imena`).
- preceded by a hard consonant → `-ęt-`/`-et-` stem (young animals): `tele` (gen `telete`, pl `teleta`).
- `-es-` stem: `nebo` (gen `nebese`). `[NOUN][STEEN-G]`
> Rule of thumb: **a noun ending in `-e` is always neuter** (standard `-e` may reflect etymological `-ę`). `[NOUN]`

**`noun-diagnostic-gensg-a`** — Masculine (and neuter) genitive singular is uniformly **`-a`** — the pan-Slavic common denominator, used as the diagnostic ending. `[DESIGN][STEEN-G]`

**`noun-irregular-plurals`** (lexicalized, suppletive): `člověk→ljudi; děte→děti; oko→oči; uho→uši`. `[NOUN]`

**Full standard singular endings** (soft variant in parentheses) `[STEEN-G]`:

| Case | masc.anim | masc.inan | neut | fem-ā | fem-i |
|---|---|---|---|---|---|
| Nom | -Ø | -Ø | -o (-e) | -a | -Ø |
| Acc | -a | -Ø | -o (-e) | -u | -Ø |
| Gen | -a | -a | -a | -y (-e) | -i |
| Dat | -u | -u | -u | -ě (-i) | -i |
| Ins | -om (-em) | -om (-em) | -om (-em) | -oju (-eju) | -ju |
| Loc | -u | -u | -u | -ě (-i) | -i |
| Voc | -e (-u) | -e (-u) | -o (-e) | -o | -i |

Plural (all types): Dat `-am`, Ins `-ami`, Loc `-ah`; Nom masc.anim `-i`, masc.inan `-y(-e)`, neut `-a`, fem-ā `-y(-e)`, fem-i `-i`. `[STEEN-G][NOUN]`

### 3.2 Adjective lemma-ending rules

**`adj-hard-y`** — hard-stem adjective → masc.nom.sg **`-y`**. Ex: *dobrъ(jь) → dobry. `[STEEN-G][WF]`
**`adj-soft-i`** — soft-stem adjective (stem in `š ž č j`) → **`-i`**. Ex: svěži. `[STEEN-G]`
**`adj-adverb`** — adverb = neut.sg `-o` (`-e` after soft). Ex: dobry→dobro. `[WF]`
Adjective declension endings: Gen.sg masc/neut `-ogo(-ego)`, Dat `-omu(-emu)`, Ins `-ym(-im)`, Loc `-om(-em)`; fem `-oj(-ej)`; pl.nom masc.anim `-i`, else `-e`; obliques `-yh/-ym/-ymi (-ih/-im/-imi)`. `[STEEN-G]`

### 3.3 Verb infinitive lemma-ending rules

**`verb-inf-ti`** — infinitive lemma always ends **`-ti`**; infinitive stem = drop `-ti`. Ex: *dělati → dělati; *prositi → prositi; *nesti → nesti. `[STEEN-G][DERIV]`
> Northern flavour uses `-ť`; standard is `-ti`. `[STEEN-D]`

Present-stem derivation from the infinitive (stored only when irregular) `[STEEN-G]`:
- consonant stem: unchanged (`nesti → nes-e-`);
- `-ati`/`-ěti`/monosyllabic-vowel: add `-j-` (`dělati → dělaj-`, `uměti → uměj-`);
- `-ovati → -uj-` (`kovati → kuj-`);
- `-nuti → -n-` (`tegnuti → tegn-`);
- 2nd conj `-iti`/most `-ěti → -i-` (`hvaliti → hval-i-`).

Verbal-noun / participle lemma tails (for derivation §deriv): gerund `-nje`/`-tje`; L-participle `-l/-la/-lo/-li`. `[STEEN-G][WF]`

### 3.4 Morphophonemic ending alternations (apply when attaching any ending)

- **O⇒E:** after a soft consonant, endings `-o, -ov, -om, -ogo, -oj → -e, -ev, -em, -ego, -ej`. `[PHON][NOUN]`
- **Y⇒I/E:** after a soft consonant — in **adjectives/pronouns** `y→i`; in **nouns** `y→e`. Ex: `domy` vs `kraje`; `ženy` vs `zemje`; `dobryh` vs `mojih`. `[PHON][STEEN-G]`
- **Ě⇒I:** in fem. dat/loc sg, `ě→i` after a soft stem (`ženě` vs `zemji`) — opposite direction to Y⇒E. `[PHON][STEEN-G]`
- **Fill/fleeting vowel:** insert `-e-` before `-j` or after a soft consonant, `-o-` between hard consonants, to break impossible clusters (reflex of yers): `okno → gen.pl okėn/okon`; `pism- → pisėmny/pisemny`. `[NOUN][PHON][WF]`
- **Vocative palatalization:** `k g h → č ž š` before voc `-e` (`člověk→člověče`, `Bog→Bože`; `-ec` words → `-če`, `otec→otče`). `[NOUN][STEEN-G]`

### 3.5 Indeclinables
International nouns ending `-e/-i/-u` (`alibi, hobi, intervju, kafe, kakao, kliše, menju, tabu, taksi`) and abbreviations are **indeclinable** (one invariant citation form). `[NOUN][STEEN-G]`

---

## 4. The Modern-Consensus / Prototype Method

Two-stage model `[DESIGN][STEEN-D][DERIV]`: **(A)** choose the ROOT by a branch-balanced vote over living languages; **(B)** fix the FORM by deriving from the Proto-Slavic/OCS reconstruction through §2 (never by copying a modern word). The engine then reconstructs the flavored spelling and strips it to the standard lemma.

### 4.1 Branch-balanced voting — the six-subgroup system

Six subgroups, **one vote each** (this is the East/West/South balancing mechanism — it prevents Russian and the BCMS cluster from over-counting) `[DESIGN][STEEN-D]`:

| Vote | Subgroup | Members |
|---|---|---|
| 1 | Russian | RU |
| 2 | Ukr+Bel | UK, BE |
| 3 | Polish | PL |
| 4 | Czecho-Slovak | CZ, SK |
| 5 | SL + BCMS | SL, HR, SR, BS |
| 6 | Bulgaro-Macedonian | BG, MK |

- Languages < 1M speakers (Sorbian, Kashubian, Rusyn) **do not vote** (but are "taken into consideration"). `[DESIGN]`
- Within a subgroup that disagrees, cast **½ vote each**. `[DESIGN]`
- **Tie-break: population** — practically "Russian always wins" (~70% of Slavs know it). `[DESIGN]`
- Override: *"when another solution is obviously better, common sense should prevail."* `[DESIGN]`

### 4.2 Root-selection cascade (run in order) `[DESIGN][STEEN-D]`

1. Root present in **all** languages, same meaning → adopt directly.
2. Only **one or two** languages diverge → adopt the shared majority root.
3. **Two+ groups** each on a different root → build "semantic families": for each candidate root, gather cognates + meanings across the remaining languages; pick the word occurring in **all or most** languages and assign the **most 'average' meaning**.
4. Fallbacks (alternatives, not strict priority): word in most languages; most intuitively understandable even if in one language; create synonyms; give bigger/better-known languages predominance; use an international equivalent; last resort — engineer a neologism (portmanteau `katka`, calque `kolokrěslo`, coinage `gradnik`, descriptive `časina`).

### 4.3 Concrete scoring scheme (implementable)

For a target concept with candidate root-forms `C₁…Cₙ`, each attested in a set of languages:

```
GROUPS = [ [RU], [UK,BE], [PL], [CZ,SK], [SL,HR,SR,BS], [BG,MK] ]
POP    = { RU:1, UK:0.42,BE:0.10, PL:0.44, CZ:0.10,SK:0.05,
           SL:0.02,HR:0.05,SR:0.09,BS:0.03, BG:0.08,MK:0.02 }  // relative speaker weights

fn group_vote(candidate C, group G):
    langs_for_C = members of G whose word is cognate with C
    return |langs_for_C| / |G|          // 1.0 if whole group agrees; 0.5 on a split

fn score(C):
    branch_score = Σ_{G in GROUPS} group_vote(C, G)   // max 6.0
    return branch_score

pick = argmax_C score(C)
if tie:  pick = argmax_C  Σ_{lang votes for C} POP[lang]   // population tie-break
// Exclude langs <1M from both sums.
```

- `branch_score` implements one-vote-per-subgroup with ½-votes on intra-group splits.
- Population is a **strict tie-break only**, never a primary weight (that is what keeps branches equal). `[DESIGN]`
- Enforce the **root-consistency invariant**: cache the winning root's normalized form; every derivative reuses it. `[DERIV]`
- **Preserve maximal distinctions** a large branch keeps (do not collapse hard-L/soft-LJ just because Czech merges them). `[INTRO]`

### 4.4 Reconstruct flavored spelling from the consensus

1. From the chosen root, obtain/reconstruct the **Proto-Slavic/OCS form** (OCS is the usual attestation). `[DESIGN][DERIV]`
2. Run §2 rules → **etymological (flavored) spelling** (the "source code"), carrying provenance so `ć/đ` vs loan `dž` and soft consonants are distinguishable. `[ORTH][DERIV]`
3. Attach POS endings (§3). Apply morphophonemic alternations (§3.4).
4. Strip diacritics per §1.3 → **standard lemma**.
5. (Optional) Emit flavourised variants by re-running the vote with some languages down-weighted; Northern/Southern spelling levers `[STEEN-D]`:

| Etym. | Standard | Northern | Southern |
|---|---|---|---|
| `y` | y | y | i |
| `ě` | ě | e | ě |
| `ę / ų` | e / u | ja / u | e / u |
| `å` | a | o | a |
| `ė / ȯ` | e / o | e / o | e / ă |
| syllabic `r/ŕ` | r | or / er | r |
| `ĺ ń ŕ t́ d́ ś ź` | l n r t d s z (lj/nj) | ľ ń ŕ ť ď ś ź | lj nj r t d s z |
| `ć / đ` | č / dž | č / dž | ć / đ |
| `šč` | šč | šč | št |

> **Note on ę flavour:** standard `ę→e`; the `[PHON]` page's "ę as ja" is the **Northern flavour**, not the standard reduction. Standard wins for the lemma. `[DERIV][STEEN-D]`

### 4.5 Proto-Slavic vs modern consensus — which wins
They "differ only in details" and are designed to agree. Modern consensus decides **which root and its meaning**; Proto-Slavic supplies the **canonical form**. On disagreement in details, prefer the reading that maximizes intelligibility. `[INTRO][DESIGN]` No source gives a numeric override; treat regular §2 derivation as authoritative for form, consensus as authoritative for root choice.

---

## 5. Internationalism / Loanword Adaptation

**Admission gate:** an internationalism is adopted only if used in **most** Slavic languages, recognizable to all/most Slavs, and not a false friend. `[DESIGN]` Loanwords use the **standard alphabet only** — no etymological glyphs, no length/tone marks, no special letters for non-Slavic sounds. `[ORTH]`

**General principle:** stay as close as possible to the original spelling, adapting only as far as orthography requires. `[DERIV]`

### 5.1 Phonological adaptations (Graeco-Latin) `[DERIV]`

| Source | → ISV | Example |
|---|---|---|
| geminate consonant | single | `gramofon, grupa, masa` |
| Greek θ (th), φ (ph) | `t`, `f` | `teatr, fenomen` |
| Greek υ (y) | `i` | `sistem, fizika` |
| /k/ (c, k) | always `k` (never `c`) | `kontakt` |
| /x/ (often `ch`) | `h` | `psiholog` |
| /y/ (Ger. ü, Fr. u) | `ju` | `bjuro` |
| intervocalic `-s-` [z] | `z` | `baza` |
| intervocalic `-ss-` [s] | `s` | `masa` |

### 5.2 Ending adaptations `[DERIV]`

| Source ending | → ISV | Example |
|---|---|---|
| Latin verbs -ate/-fy/-ise/-ize, Ger. -ieren | `-ovati` | `organizovati, komunikovati` |
| Latin -ia / Eng. -ia,-y | `-ija` | `ekonomija` |
| Greek/Eng. -sis | `-za` | `kriza` |
| -ium (elements) | `-ij` | `helij, kriterij` |
| -um, -us | kept | `forum, korpus` |
| Eng. -ty (Lat. -tas) | `-tet` | `universitet` |
| Eng. -ics | `-ika` | `ekonomika` |
| Eng. -ism | `-izm` | `komunizm` |
| Eng. -ist | `-ist` | `komunist` |
| Eng. -ssion | `-sija` | `diskusija` |
| Eng. -nsion/-rsion | `-nsija/-rsija` | `pensija, versija` |
| Eng. -sion | `-zija` | `televizija` |
| Eng. -tion | `-cija` | `akcija` |
| adj. from these nouns | `-ijny` | `televizijny, tradicijny` |
| Eng. -al (Lat. -alis) | `-alny` | `neutralny` |
| Eng. -ic/-ical (Lat. -icus) | `-ičny` | `specifičny, komičny` |
| Eng. -ive (Lat. -ivus) | `-ivny` | `pozitivny` |
| Eng. -ous (Lat. -osus) | `-ozny` | `seriozny` |

### 5.3 English (non-Graeco-Latin) loans `[DERIV]`
Latin script may keep original spelling (`bypass, knockout, jazz, teenager`), but because texts are transliterated to Cyrillic, a **phonetic respelling** is preferred: `bajpas, nokaut, džaz, tinejdžer, budžet, biznes, mjuzikl, futbol, koktejl`. Borrowed `[d͡ʒ]` is written `dž` (never etymological `đ`). `[PHON]`

### 5.4 Compounds `[WF]`
Two roots join with connector `-o-` (`-e-` after a soft consonant): `voda+padati → vodopad`; `myš+loviti → myšelovka`; `zemja+tresenje → zemjetresenje`. English first-member borrowings may take a hyphen: `rok-muzika, veb-stranica`.

### 5.5 Slavic-vs-international caution
Prefer international forms where a Slavic word is a false friend (month names: `listopad` = November in PL/CZ but October in HR). `[DESIGN]`

---

## 6. Implementation Priority & Hard Cases

### 6.1 Highest-value rules to implement first (ranked by accuracy impact)

1. **Etymological→standard diacritic strip (§1.3)** — the terminal step for *every* lemma; trivially correct, unblocks everything. `[ORTH]`
2. **Yers: strong→ė/ȯ→e/o, weak→∅, Havlík ordering, fleeting vowels (§2 Phase A/E, §3.4)** — pervasive; drives `-ec/-ok` nouns, gen.pl, prepositions. Highest single lexical coverage. `[DERIV][STEEN-G]`
3. **Nasals ę→e, ǫ→u (§2 Phase C)** — extremely frequent; simple; note the ę/ų asymmetry and `vų-` prothesis. `[DERIV]`
4. **Jat ě→ě (§2 Phase D)** — high frequency, kept letter, near-trivial. `[DERIV]`
5. **tj/dj→č/dž and sj/zj→š/ž (§2 Phase B)** — the flagship compromise; well-defined. `[DESIGN][DERIV]`
6. **Liquid diphthongs: TorT→ra, TolT→la, TerT→rě, TelT→lě; syllabic r/l (§2 Phase A/F)** — many core roots (`grad, glava, mlěko, brěg, trg, volk`). `[DERIV]`
7. **POS lemma endings + gender diagnostics (§3.1–3.3)** — required to emit a headword at all; gen.sg `-a`, inf `-ti`, adj `-y/-i`. `[STEEN-G][NOUN]`
8. **O⇒E / Y⇒I,E / Ě⇒I soft-stem alternations (§3.4)** — needed for every soft-stem paradigm. `[PHON]`
9. **First palatalization in its live contexts + the no-2nd-palatalization negative rule (§2 Phase H)** — prevents systematic over-application. `[PHON][STEEN-G]`
10. **Six-subgroup voting with ½-votes and population tie-break (§4)** — root selection; the accuracy ceiling for novel words. `[DESIGN]`
11. **Internationalism adaptation table (§5)** — large, self-contained, high-yield vocabulary slice. `[DERIV]`

### 6.2 Known hard cases / systematic pitfalls

- **`dž` collapse ambiguity.** Etymological `đ` (*dj) and native/loan `dž` both → standard `dž`. Keep provenance on the etymological layer; never round-trip standard→etymological blindly. `[PHON][STEEN-G]`
- **ę/ǫ asymmetry.** Only the *front* nasal maps to an `ę`-type letter; *ǫ→`ų`/`u`. A symmetric "both nasals → ę" rule is wrong. `[DERIV]`
- **Strong-yer direction.** Strong *ъ→o (`ȯ`), strong *ь→e (`ė`) — **not** *ъ→e. The `[DESIGN]` expert gloss is erroneous; §2 `strong-back-yer` follows `[DERIV]`. `[DERIV]`
- **Havlík alternation.** Yer strength depends on position/counting; getting the parity wrong flips fleeting-vowel placement (`otėc/otca` vs. wrong `otc/oteca`). Must be computed before deletion. `[STEEN-G]`
- **No 2nd palatalization in inflection.** `Čeh→Čehi`, not `Česi`; `dȯlgy→dȯlgi/dȯlge`. Only the vocative palatalizes velars in paradigms. `[PHON][STEEN-G]`
- **Y⇒E (nouns) vs Y⇒I (adjectives/pronouns), and Ě⇒I (fem dat/loc) going the *opposite* way.** Three soft-stem vowel rules with divergent targets; keying them off the wrong POS silently corrupts endings. `[PHON][STEEN-G]`
- **Labials + j.** Write `pj/bj/vj/mj`, not ESl/SSl `plj/blj/…`. `[DERIV][PHON]`
- **kt/gt verb exception.** Infinitives keep `mogti/pekti/běgti`; do not apply *kt→č there. `[STEEN-G]`
- **`lj/nj` are single sounds except** in Latin/Greek-prefixed loans (`konjunktura`, `injekcija`) and across suffix boundaries (`-je/-ji/-ju`) — where `n`+`j` are separate. `[STEEN-G]`
- **Athematic neuter `-e` stem selection** (`-men-` vs `-ęt-` vs `-es-`) depends on the preceding consonant and must be lexicalized where irregular. `[NOUN]`
- **Iotation blocking.** A prefix before a `j-`-initial word blocks iotation: `s+jesti→sjesti` (not *šesti). `[PHON]`
- **Standard `-e` may hide etymological `-ę`** in neuter athematics (`ime`, `tele`) — relevant only when regenerating the flavored layer. `[NOUN][ORTH]`
- **Flavour vs standard confusion.** `ę→ja`, `ě→e`, `y→i`, `å→o` are *flavourisation* outputs, not the standard lemma reduction; keep them in a separate output register. `[PHON][STEEN-D]`
- **Prothetic j- and general 2nd/3rd palatalization are unspecified** by the sources — do not invent; only `v-` before initial *ǫ- is licensed. `[DERIV]`
- **Sources not covering morphology defer, they don't contradict.** `[ORTH][PHON][WF][INTRO]` explicitly delegate endings to `[NOUN]`/`[STEEN-G]`; treat "not covered on this page" as silence, not a competing rule.