# Slovowiki (Interslavic Wiktionary Lab)

The website is generated **locally** with `cargo run --release -- export --out site`
(then open `site/index.html`, or serve it with any static server). It is not
published to GitHub Pages.

An **evidence-based Interslavic (Medžuslovjansky) candidate-generation engine** with a
reproducible accuracy benchmark against the official Interslavic dictionary, plus a
Wiktionary-style website that shows, for every meaning, the generated candidate,
its rule trace, the Slavic evidence by branch, a calibrated confidence, and whether it
matches the official dictionary.

No SQLite / database and no server: the website is a **statically generated** set of
HTML pages + client-side search, hostable on any static host. No hotlinked Wikimedia CSS/JS.

## The site is corpus-driven, not dictionary-driven

The website is **not** limited to the official dictionary's meanings. It is built from the
**whole Wiktionary Slavic-lemma corpus**: every inherited Slavic lemma (noun, verb
infinitive, positive adjective, …) is extracted with its Proto-Slavic ancestor, and
lemmas sharing an ancestor form a **cognate set**. Each set becomes one Interslavic word
— the Proto-Slavic rule engine supplies the form from the *known* reconstruction, the
modern reflexes give the consensus surface — and **confidence scales with how many
languages and branches attest it**: a root seen in one language is a low-confidence
guess; one spread across all three branches is high-confidence.

Two kinds of etymological group are collected:

- **Inherited** lemmas, grouped by their Proto-Slavic ancestor (`*voda`, `*dobrъ`).
- **Borrowings / internationalisms**, grouped by shared phonemic skeleton
  (`компьютер`/`komputer` → `kompjuter`) — the modern Graeco-Latin and other loan
  vocabulary, generated with the internationalism ending rules.

- `cargo run -- extract-lemmas` — stream the dump once → `data/slavic-lemmas.cache.json`
  (~47k lemmas: ~25k inherited + ~22k borrowings, across 15+ Slavic lects incl. OCS).
- `cargo run -- extract-enrich` — stream the **native Russian / Polish / Czech
  Wiktionary** dumps once → `data/wiktionary-enrich.cache.json` (~53k cognate
  entries with native etymology, extra senses, and related/synonym/antonym links).

## Native-Wiktionary enrichment (RU / PL / CS)

Beyond the English-Wiktionary Proto-Slavic etymology, each cognate is enriched
from its **own** language's Wiktionary. Every entry page then shows, per cognate:

- **Three independent etymologies** side by side — the Russian (Vasmer), Polish
  and Czech accounts of the word's Proto-Slavic → PIE history, each linking to the
  source edition. Russian display text is deterministically transliterated into
  the site's Interslavic-style Latin script during every `export` run; source
  links still point to the original Russian Wiktionary pages.
- **Extra meanings** — the native senses (a Russian entry often lists 10+ senses
  where the English gloss gives one).
- **Semantic web** — related, derived, synonym and antonym terms as chips, each
  linking back to its native Wiktionary. `water` alone surfaces 100+ links.

The cache is built by filtering the RU/PL/CS dumps to the ~70k cognate words that
actually appear in the corpus (streamed in seconds), so the enrichment is
committed and the site build stays self-contained.
- `cargo run -- export` — generate the cognate-set site (~21.4k words after merging notation-variant and same-concept duplicates; falls back to the
  dictionary-seeded site if the lemma cache is absent).
- Independent validation: **~4.8k distinct official Interslavic lemmas are reproduced**
  by a generated word (of ~21.4k), one representative page per lemma (homographs and
  duplicate sets deduped), with no leakage from the dictionary into the generation.
- `cargo run -- corpus-eval` scores this site path against the dictionary directly:
  **58.6% exact / 63.1% normalized** on the ~7.4k entries with a known ancestor.
- `data/novel-words.tsv` — the **novel-vocabulary proposal pipeline** (regenerated
  by every `export`): every generated word absent from the official dictionary,
  carrying an **isotonic-calibrated probability** *P(would match an official
  decision)* (`data/score-calibration.json`, fitted on the benchmark's dev split,
  holdout-validated at ECE 0.013) and a bucket at measured operating points —
  **predlog** (p≥0.6: 71.8% precision / 66.3% recall on holdout) or **pregled**
  (p≥0.3: 61.7% / 88.9%). The site's *Predloženja* page renders the propose
  bucket with evidence traces and `data/curation-notes.json` annotations; the
  precision/recall of every threshold is in `target/eval/methodology.md`.

The **benchmark below** still measures generation accuracy against the official dictionary
(a separate, leakage-free evaluation of the engine).

## Core principle

> No algorithmic change is kept unless it improves **measured accuracy** on the
> reproducible benchmark against official Interslavic data.

Every rule is gated behind a flag and measured in isolation on an ablation ladder.
Rules that regress accuracy are reverted and documented (see the *rejected experiments*
in the report).

## Results (production config vs. original prototype)

Benchmark: reconstruct the official Interslavic lemma from the modern Slavic cognates in
the official dictionary, **without ever showing the generator the answer**
(16,300 single-word entries).

| Metric | Baseline (prototype) | Production | Δ |
|---|---:|---:|---:|
| exact top-1 | 27.52% | **41.65%** (95% CI 40.9–42.4) | +14.13 pp |
| normalized top-1 | 35.23% | **49.59%** (95% CI 48.8–50.3) | +14.36 pp |
| normalized top-3 | 43.26% | **60.48%** | +17.22 pp |
| normalized top-5 | — | **63.12%** | — |
| mean normalized edit distance | 0.252 | **0.224** | −0.028 |

The **site's** cognate-set path (`corpus::generate_set`) is benchmarked separately
(`cargo run -- corpus-eval`): **58.6% exact / 63.1% normalized** on the ~7.4k entries
where a Proto-Slavic ancestor or internationalism is known — higher than the pipeline
headline because it only scores words the site actually derives from a known ancestor.

**Synonym-aware accuracy** (`cargo run --release -- synonym-eval`) reframes the
strict metric honestly: Interslavic often has several valid words per concept and
the dictionary records only one as *the* lemma, so a "miss" is frequently a valid
synonym the committee didn't pick. Crediting a prediction that reproduces **any**
official ISV lemma whose gloss matches the concept lifts top-1 from 49.6%
normalized to **55.8% synonym-inclusive**; of the strict misses, ≥12% are
demonstrably valid ISV synonyms (another official lemma for the same concept) and
the rest are a mix of genuine errors and valid synonyms the dictionary doesn't
list separately.

A data-quality **audit** (`cargo run --release -- audit`) classifies every miss and
attributes it to the pipeline **stage** that lost the official form (a full
`RuleStep`-trace replay — see `target/eval/stage-attribution.md`): ~33%
*cluster/vote* (a different, usually editorial, root was chosen), ~22%
*merge/rank* (a correct candidate was generated but demoted — of which only ~1.9%
of all misses are a genuine same-cluster ranking bug, the rest being synonym
word-choice), ~22% *root-absent* (unfixable from evidence), ~15%
*normalize/representative*, ~6% *endings*, and only **~1.6%** the Proto-Slavic
*rule engine*. 89.5% of meanings split across ≥3 cognate clusters. A companion
**oracle ladder** (`cargo run --release -- oracle`, diagnostic-only) measures each
stage's upper-bound headroom: cluster +4.5pp / proto-link +2.7pp /
representative +2.3pp exact. The representative lever was the recoverable one:
shipping the **medoid** representative (below) captured +1.1pp of it, and the V8
derivational-morphology pass (below) converted another +0.6pp of the
endings/representative tail into matches.

The Proto-Slavic rule engine is measured in isolation by a dedicated benchmark
(`cargo run --release -- proto-eval`): on the 20.1% of words it confidently links
to a reconstruction it derives the official lemma with **46.68% exact / 52.74%
normalized** accuracy.

**Inflection validation** (`cargo run --release -- inflect-eval`): every
unique single-word official lemma through the inflection engine — 14,625
lemmas / 231,977 paradigm cells, **0 blank** (the export's blank cells all
come from machine-generated reconstruction headwords), with RULE_SPEC §3
grammar invariants checked with their legitimate exemptions modeled
(pluralia tantum, §3.5 indeclinables, masculine ā-stems, substantivized
adjectives, multi-variant cells): nom.sg echoes the citation form (99.9%),
masc/neut gen.sg carries the diagnostic `-a` (99.8%), adjective agreement
100%, and the §3.1 suppletive plurals (`člověk→ljudi`, `oko→oči`, …) verified
**from the inflector itself** (the pinned crate implements them, heteroclite
byforms included). The remaining ~12 failures are the genuine inflector
worklist (soft `-o` loans like *adadžo*, unmarked indeclinables like *kakao*).
Canonical paradigm cells are pinned by unit tests so an inflector-crate rev
bump that reshapes declension fails CI. Report:
`target/eval/inflection-report.md`.

**Evidence ceiling, measured** (`cargo run --release -- evidence-eval`): the
~22% *root-absent* miss bucket was hypothesized to be an extraction gap. It is
not: only **2.8%** of root-absent misses (51 of 1,854) have the official root
anywhere in the 46k-lemma Wiktionary cache under a gloss-matched lemma — and
**zero** of those are reachable without displacing the dictionary's own
citations (all 51 sit under a language the row already cites with a different
synonym: the editorial phenomenon again). The conservative augmentation A/B
measures exactly **+0.00pp, 0 fixed / 0 broke**. The bucket is a genuine
evidence ceiling. Report: `target/eval/evidence-growth.md`.

**Multi-word & aspect slices** (`cargo run --release -- multiword-eval`): the
headline benchmark excludes all 1,837 multi-word official lemmas; this scores
them separately — reflexive `X sę` (561 lemmas, the existing pipeline just never
scored them): **25.0% exact / 30.8% normalized**; two-token collocations
reconstructed per position with gender agreement (886 of 1,083 generatable):
**11.9% / 17.7%**; and 1,440 morphologically related 1:1 ipf/pf **aspect
pairs**: both members correct 16.5%, one 32.9%, neither 50.6%. Full report:
`target/eval/multiword-aspect.md`.

**Word-formation layer** (`src/derive.rs`, `cargo run --release -- derive-eval`):
from one citation form the engine derives its regular family — abstract `-osť`,
adverb, verbal noun `-ńje` (with iotation: prositi→prošeńje, roditi→rođeńje),
agentive `-telj`/`-teljstvo`/`-teljka`, denominal `-ny`/`-sky` (with first
palatalization: kniga→knižny, Grek→grečsky), diminutive `-ka`/`-ica`, negation
`ne-`. Benchmarked on **2,115 derivationally related official lemma pairs**
(mined by inverse suffix lookup, deduped across duplicate rows; the layer
derives the official base forward and never sees the derivative): **96.0% exact
/ 99.7% normalized**, vs 47.9% / 83.6% for naive concatenation — the seam
morphophonemics is worth **+48pp exact**. Every entry page (generated and
official-only) shows the family ("Slovotvorstvo") with official members
cross-linked and unattested members marked as machine proposals; families
derived from an unmatched reconstruction are flagged as hypothetical.

**Confidence calibration** (high-confidence candidates match far more often — as intended):

| confidence | n | normalized match |
|---|---:|---:|
| high | 6,988 | 72% |
| medium | 7,097 | 39% |
| low | 2,215 | 12% |

Beyond the three-way badge, `target/eval/methodology.md` now carries a full
**reliability table** (score decile → empirical match rate), **ECE** and **Brier**
scores, plus an **isotonic recalibration** fit on the dev split and validated on
the untouched holdout: holdout ECE drops from 0.195 (raw score, systematically
overconfident) to **0.013** recalibrated — the recalibrated probability is what a
downstream consumer (reliability badge, novel-word filter) should read as
*P(matches the official lemma)*; the raw score remains the ranking key.

Full metrics, POS-specific accuracy, branch-coverage analysis, regression/improvement
lists and the remaining-error breakdown are regenerated into `target/eval/` (a committed
snapshot is under version control).

## What was kept (each improved measured accuracy)

1. **Branch-balanced consensus** — vote on a consonant-skeleton alignment key counting
   Slavic *branches*, not languages, so Russian/Polish can't dominate.
2. **Six-subgroup vote** (§4.1 of the rule spec) — one vote each for RU / UK+BE / PL /
   CZ+SK / SL+HR+SR+BS / BG+MK, with population as a tie-break only.
3. **POS lemma endings** (§3) — noun nom.sg, adjective `-y`/`-i`, verb infinitive `-ti`.
4. **Internationalism table** (§5.2) — `-ism→-izm`, `-tion→-cija`, `-ic/-ical→-ičny`,
   `-al→-alny`, `-ive→-ivny`, verbs→`-ovati`, plus `au→av`/`eu→ev`/`th→t` **gated to
   recognized internationalisms** (so native `naučiti`/`sauna`/`snosny` are untouched).
5. **Prefix normalization** — `roz-/ras-/raz-/ros- → råz-`, `pred- → prěd-`.
6. **De-pleophony** (liquid metathesis) and **nasal recovery** (`ę/ų` from Polish).
7. **g-preserving representative** — Interslavic keeps *g, so g-languages outrank the
   Czech/Slovak *g→h forms when picking the surface.
8. **Proto-Slavic-derived form (two-stage, §4.4)** — consensus picks the *root*, then the
   Proto-Slavic rule engine derives the *form* with the correct flavored letters
   (`ě/ć/đ/å/ȯ/y`, prothetic `j-/v-`). Each meaning is linked to its `sla-pro`
   reconstruction by a **leakage-free** signal (descendant membership + derived-form
   similarity + gloss overlap), and the derivation supplies the flavored spelling for the
   consensus form. Yer resolution uses a real **tense-yer rule** (yer before *j → `i`/`y`,
   `novъjь`→`novy`) and **reflex-guided vocalization** — a lexically-ambiguous weak yer is
   kept when the reflexes vote to keep it (`*pьsati`→`pisati`) and dropped when they drop
   it (`*bьrati`→`brati`) — resolved by evidence, not a length heuristic. A length-free
   **reflex-shape-agreement** rule governs when the reconstruction may override the
   consensus. This rung adds **+2.4 pp exact / +2.2 pp top-3** over the consensus-only
   config, and a further **+2.0 pp exact** comes from **explicit etymology** — using
   Wiktionary's stated `(lang → ancestor)` map to pick the reconstruction when ≥2
   cognates agree, before the fuzzy descendant+gloss link.
9. **Internationalism preference** — for concepts the dictionary marks international
   (`genesis=I`), prefer the international cluster over a native synonym (`aeroplan`).
10. **Adjective fleeting-vowel drop** — collapse a South-Slavic short adjective's
    fleeting vowel before `-y`, gated on East/West consonant adjacency (`dobar→dobry`,
    `zelen` stays). The single biggest lever (+1.2 pp exact).
11. **Prefix-stripped proto links** — when a whole word doesn't link, strip a shared
    prefix, link the bare root, re-attach the Interslavic prefix (`napisati`).
12. **Lemmas only** — drop bg/mk present-tense verb citations (no infinitive), and
    reflexive verbs are cited `<lemma> sę` after stripping the cognates' markers.
13. **Synonym alternatives** — surface secondary translations as top-3/top-5
    alternatives (scored below every primary candidate; never changes top-1).
14. **Medoid representative** — pick the winning cluster's surface as the *medoid*
    (the member minimizing total folded edit distance to the others — the most
    central attested form) instead of a fixed language-priority list, avoiding
    dialectal/oblique outliers. Found by the `rep-eval` probe, which measured this
    against the diagnostic oracle-representative ceiling; **+1.09 pp exact** — the
    single biggest generation win after the two-stage proto model, and the first
    representative-selection rule to beat the fixed priority.
15. **Derivational-suffix normalization** (root-consistency invariant `[DERIV]`) —
    each categorical in the dictionary: `-telj-` kept before derivational suffixes
    (53 official -teljstvo/-teljny/-teljsky vs **zero** hard -tel- there; the old
    word-final `-telj` rule missed the whole derived family), feminine i-stems end
    soft `-sť` (516 vs 0: kosť, radosť, zabolěvajemosť), and the deverbal
    adjective suffix is `-livy` (152 vs 0 -ljivy). +0.25 pp exact, 40 fixed / 0
    broken.
16. **Graeco-Latin hiatus in loans** — ISV keeps `-ia-`/`-io-` (socialny,
    entuziazm, sociolog) where Slavic cognates insert a glide `-ija-`: 24 official
    -ial- vs 0 -ijal-, 139 midword -io- vs 1 -ijo- (kopijovati, hence the
    noun/adjective gate). +0.06 pp exact, 10 fixed / 0 broken.
17. **Spirantization repair** — a Czech/Slovak/Ukrainian/Belarusian representative
    leaks its *g→h shift into the surface (blahosklonnost, kalihrafija); each `h`
    is checked per consonant position against the g-preserving cognates
    (ru/pl/South) and restored to `g` on ≥2 corroborating witnesses. Genuine *x/loan
    `h` (duh, alkohol) stays — the g-preserving lects write `h` there too.
    +0.33 pp exact / +0.49 pp normalized, 57 fixed / 3 broken.

## What was rejected (regressed the benchmark)

Recovering flavored letters (`ć/đ`, jat `ě`, `*y`) from *modern reflexes* is too noisy —
each experiment regressed accuracy. The correct source (rule spec §4.4) is the
**Proto-Slavic reconstruction**, which the `+proto-derived` stage above now uses. The
consensus-level `palatals`/`jat`/`y-recovery` toggles remain in the report's *rejected
experiments* table as documented negatives.

## Testing

`cargo test` runs the unit suite — 100+ tests across `proto`, `normalize`,
`orthography`, `morph`, `derive`, `consensus`, `corpus`, `dump`, `eval`,
`forms` (API round-trips, wire-format stability) and `check`
(self-verification: sampled official lemmas and paradigm cells must resolve
as known, garbage as unknown). Every rule was **adversarially
audited and triple-checked** (a finder plus two independent verifiers reproducing each
bug against the binary); the confirmed bugs were fixed with a regression test each. CI
(`.github/workflows/ci.yml`) runs `fmt` + `build` + the tests **and fails if exact
top-1 drops below a floor** — the floor measures the *shipped* production config
(`runs.last()`), not the best ablation rung, and a test asserts the ladder ends at
`ConsensusConfig::production()`, so a production regression can't slip through.

`evaluate` additionally writes **statistical instruments** to
`target/eval/methodology.md` (all deterministic/seeded, reproducible
byte-for-byte):

- **Overfitting guard** — a seeded 75/25 dev/holdout split (stable FNV hash of the
  entry id, so the held-out quarter never changes); every rung is reported on both
  splits and a kept rule must generalize to the holdout. The three V8 rules hold
  their gains on the holdout and leave the dev−holdout gap unchanged (+0.96pp).
- **Paired significance** — each rung vs the previous, two-sided sign test on the
  discordant entries (fixed/broke), on both metrics. This exposed that
  `+explicit-etymology` is noise on the normalized metric (215 fixed / 205 broke,
  p=0.66 — it is kept for its exact-metric gain, p=0.02) and `+depleophony`
  actually nets −2 entries on exact.
- **Bootstrap 95% CIs** on the headline (1000 seeded resamples), so ladder deltas
  are read against sampling noise.
- **Calibration** — reliability table (score decile → empirical match rate), ECE
  and Brier, plus a dev-fit / holdout-validated **isotonic recalibration** (see
  the calibration section above).
- **Full predictions dump** — `target/eval/predictions.csv` (every entry with
  prediction, split, score, hit flags) and an uncapped `audit-misses.csv` (every
  miss with per-stage blame), for offline pattern mining; the V8 suffix rules
  were found by mining exactly these residuals.

The benchmark is **leakage-free w.r.t. the answer form**: the generator sees the modern
cognates plus the official row's POS/gender/`genesis` metadata, but never the `isv`
lemma. Two paths are measured separately — the consensus **pipeline** (headline above)
and the **site's** `corpus::generate_set` (`corpus-eval`).

## Architecture

```
src/
  model.rs         Candidate / RuleStep / Evidence / Confidence / MatchStatus / Pos
  lang.rs          Slavic language + branch + script metadata
  normalize.rs     per-language script → common phonemic Latin (keeps ě/ę/ǫ/č/ć/đ)
  orthography.rs   flavored↔standard folding, ASCII skeleton, edit distance
  official.rs      official dictionary loader (quote-aware CSV / TSV)
  consensus.rs     branch-balanced modern-Slavic consensus engine (gated rules)
  morph.rs         POS lemma endings, internationalism table, derivational suffixes
  derive.rs        productive word-formation layer (families) + derive-eval benchmark
  proto.rs         Proto-Slavic → Interslavic ordered rule engine (+ tests)
  dump.rs          stream the 23 GB dump → Proto-Slavic cache + indexes
  proto_link.rs    leakage-free linker: explicit Wiktionary etymology + 3-signal fuzzy match
  pipeline.rs      two-stage §4.4 merge (consensus root + proto-derived form)
  overrides.rs     manual curation (TOML), excluded from pure-algorithm accuracy
  generator.rs     orchestrator: pipeline + overrides + official match status
  eval.rs          benchmarks: ablation ladder, holdout split, significance,
                   multiword/aspect + evidence-growth audits, report writers
  calibrate.rs     the persisted isotonic score→probability calibrator
  forms.rs         FormRecord pipeline: paradigm cells (single source for the
                   site's inflection tables AND the sharded static api/)
  check.rs         check-text: tokenizer, form lookup, semantic-trap warnings
  corpus.rs        Wiktionary-corpus cognate-set dictionary + confidence model
  thesaurus.rs     dictionary-derived ISV synonym thesaurus
  enrich.rs        native RU/PL/CS Wiktionary enrichment (etymology/senses/links)
  flavorize.rs     display flavorization of source words into ISV orthography
                   (winyl→vinyl, дело→dělo) + RU running-text transliteration
  site.rs          static site generator (export) — HTML pages, search, api/
data/
  official-isv.csv        the full official dictionary (evidence + gold)
  overrides.toml          manual curation file
  RULE_SPEC.md            authoritative Proto-Slavic → Interslavic rule spec
  FLAVORIZATION_SPEC.md   display flavorization of raw source words (issue #62)
  proto-slavic.cache.json Proto-Slavic reconstructions (built by extract-proto)
  slavic-lemmas.cache.json every inherited + borrowed Slavic lemma (built by extract-lemmas)
  wiktionary-enrich.cache.json native RU/PL/CS etymology/senses/links (built by extract-enrich)
  novel-words.tsv         novel-vocabulary proposals with calibrated probability + bucket
  score-calibration.json  the isotonic calibrator (refit by every `evaluate` run)
  semantic-notes.json     curated false-friend warnings (applied by check-text)
  curation-notes.example.json  format of the optional human curation notes
```

## Commands

```bash
# One-time: stream the 23 GB dump into the Proto-Slavic cache (enables the
# +proto-derived stage). Skip it and the engine falls back to consensus only.
cargo run --release -- extract-proto   # dump path defaults to data const; see --dump

# Reproducible benchmark against the official dictionary (fast, no dump needed):
cargo run --release -- evaluate --official data/official-isv.csv --out target/eval

# Proto-engine-only benchmark (isolates the rule engine's accuracy on linked words):
cargo run --release -- proto-eval

# One-time: stream the dump into the Slavic-lemma corpus (drives the cognate-set site):
cargo run --release -- extract-lemmas

# Benchmark the SITE's generation path (corpus::generate_set) against the dictionary:
cargo run --release -- corpus-eval

# Data-quality / ceiling audit (classifies every miss + per-stage attribution):
cargo run --release -- audit

# Benchmark the derivation layer (word families): mined official base→derivative
# pairs, seam-aware morphology vs naive concatenation:
cargo run --release -- derive-eval

# Multi-word benchmark plus the historical aspect baseline:
cargo run --release -- multiword-eval

# Dedicated aspect-pair ladder: both/either/pairing correctness, dev/holdout,
# paired sign test, core suffixes and secondary imperfectives:
cargo run --release -- aspect-eval

# Evidence-growth audit: root-absent recoverability + augmentation A/B:
cargo run --release -- evidence-eval

# Inflection validation: blank-cell census + RULE_SPEC §3 grammar invariants:
cargo run --release -- inflect-eval

# check-text benchmark: fixture classification + agreement gold/error sets:
cargo run --release -- checktext-eval

# Diagnostic-only oracle ladder (per-stage upper-bound headroom; reads the answer,
# never feeds production):
cargo run --release -- oracle

# Generate the static website locally (no server; not published anywhere):
cargo run --release -- export --out site
# Preview locally with any static server, e.g.:
#   (cd site && python3 -m http.server 8765)   # or: make serve

# Explain one word/gloss (manual spot-check with full rule trace):
cargo run -- explain duša
cargo run -- explain "computer"

# Verify an Interslavic text against the lexicon (tokens classified as
# known-lemma / known-form / generated / unknown, false-friend warnings,
# nearest-lemma suggestions; --json for agents):
cargo run --release -- check-text tekst.txt
cargo run --release -- check-text tekst.txt --json
```

## Lexical verification API (for humans and AI agents)

Every `export` also writes a **static, deterministic lexical API** under
`site/api/` (issue #11) — one `FormRecord` pipeline feeds both the website's
inflection tables and the machine-readable artifacts, so they cannot drift:

- `api/forms/<n>.json` — the **sharded form index** (schema 3, ~517k analysis
  records: every official lemma + full paradigm, **declined participles,
  comparatives/superlatives with adverbs, pronoun & numeral paradigms** from
  the STEEN-G tables, byform variants split, syncretic cells merged). Shard
  routing: `n = fnv1a32(key) % 2048` over the folded key — mirrored in the
  site's client-side JS, which verifies itself against
  `api/router-selftest.json` before trusting lookups.
- `api/lemmas.json` — every headword with status (`official` /
  `official-only` / `generated`) and calibrated probability for generated
  lemmas; official verb rows additionally carry grammatical aspect and an
  array of `[entry_id, lemma]` partners. Generated lemmas deliberately have
  **no inflection records**:
  an inflected form of a wrong reconstruction is confidently wrong.
- `api/aspect-pairs.json` — the production pair model's official endpoints,
  linked entry IDs, jointly reconciled generated forms/rule, and `-ovati/-uje`
  present stems where applicable.
- `api/meta.json` — schema version, counts, sizes, license, router spec.
  **Schema 3 migration:** v2's six-field lemma tuple is now eight fields;
  consumers must accept trailing `aspect` and `aspect_partners` (an array,
  empty for unpaired/non-official rows).
- `api/agent-guide.md` — the lookup protocol, fold table and trust rules
  (p < 0.6 ⇒ suggestion, never verification).

Website twins of the API: **`forms.html`** (reverse lookup of any inflected
form → analyses + entry links; also linked from every inflection table) and
**`text-check.html`** (paste text, every token verified client-side). The
CLI equivalent:

```bash
cargo run --release -- check-text tekst.txt          # human summary
cargo run --release -- check-text tekst.txt --json   # for agents
```

classifies every token (known-lemma / known-form / generated / unknown with
nearest-lemma suggestions; multi-word official lemmas resolve via trigram →
bigram lookup), runs **conservative grammar-agreement checks** (adjacent
adjective–noun case/number/gender — gender in the singular only, preposition
government parsed from the dictionary's own `(+N)` annotations, pronoun–verb
person/number; a warning fires only when NO combination of analyses is
compatible, never across punctuation) and applies
the curated false-friend notes in `data/semantic-notes.json` (each note
anchored to the official gloss; the web twin reads the same notes from
`api/notes.json`). CI-tested: round-trip (rendered table cells appear in the
records — unit-scale per POS) and self-verification (sampled official lemmas
and paradigm cells resolve as known; garbage as unknown). Determinism is
by construction (no timestamps in `api/`, BTreeMap ordering) and was
verified by hashing two consecutive exports.

## Website

Each entry page shows:

- the **top candidate** headword with a **provenance** pill (proto-derived / consensus /
  override) and a calibrated **reliability** badge;
- the **Proto-Slavic reconstruction** it was derived from, with Balto-Slavic / PIE
  ancestors and the link confidence;
- **alternative** candidates with scores and branch coverage;
- the **rule trace** (each transformation, before→after, with a doc citation);
- the **evidence by Slavic branch** (East / West / South), linking back to Wiktionary;
- **native-Wiktionary enrichment** — per-cognate Russian/Polish/Czech etymology,
  extra senses, and related/synonym/antonym links (see above);
- the **official-dictionary match status**: *officially attested* / *differs from
  official* (both shown) / *no official entry*;
- full **inflection tables** generated by the local `interslavic` crate, each
  linking to the headword's reverse-index view on `forms.html`;
- the **word-formation family** ("Slovotvorstvo"): regular derivatives with
  seam morphophonemics, official members cross-linked, machine proposals
  marked (families of unmatched reconstructions flagged as hypothetical);
- **dictionary synonyms** cross-linked via the ISV thesaurus, and optional
  **curation notes** from `data/curation-notes.json`.

Site-wide tools beyond the entry pages:

- **`forms.html`** — reverse lookup of any inflected form → all analyses
  (lemma, case/number/gender, entry link), backed by the same sharded index
  agents use;
- **`text-check.html`** — paste Interslavic text; every token is verified
  client-side against the lexicon, with false-friend warnings (the static twin
  of `check-text`);
- **`proposals.html`** ("Predloženja novyh slov") — the ranked novel-vocabulary
  proposals with calibrated probabilities and curation notes;
- **`metrics.html`** ("Statistiky točnosti") — every accuracy metric explained,
  with current numbers;
- **`datasets.html`** — all machine-readable artifacts (`api/`, `entries.json`,
  `graph.json`, `novel-words.tsv`, `build.json`, …).

## Benchmark artifacts

```
target/eval/candidate-generation-summary.json   per-rung metrics (machine-readable)
target/eval/candidate-generation-report.md      full human-readable report
target/eval/stage-attribution.md                 per-stage blame histogram (audit)
target/eval/oracle-ladder.md                     per-stage upper-bound headroom (oracle)
target/eval/audit-misses.csv                     misses with stage + stage_detail columns
target/eval/proto-engine-report.md               proto-engine per-rule error worklist
target/eval/regressions.csv                      matched before, not after
target/eval/improvements.csv                     newly matched
target/eval/errors-sample.csv                    nearest remaining misses
target/eval/methodology.md                       holdout split, rung significance, bootstrap CIs, calibration
target/eval/predictions.csv                      every entry's prediction (full dump, for offline mining)
target/eval/derivation-report.md                 derive-eval: word-family layer vs naive baseline
target/eval/multiword-aspect.md                  multi-word slices + ipf/pf aspect-pair accuracy
target/eval/evidence-growth.md                   root-absent recoverability + augmentation A/B
target/eval/inflection-report.md                 inflection census + RULE_SPEC §3 grammar invariants
target/eval/synonym-accuracy.md                  synonym-inclusive accuracy (thesaurus-based)
target/eval/rep-selection.md                     representative-selection probe (medoid vs oracle)
target/eval/cluster-selection.md                 cluster-selection probe (blind rules vs oracle)
```

The V7 full-pipeline review (stage-attribution histogram, oracle ladder, and the
ranked list of kept/reverted fixes) is written up in **[IMPROVEMENT_REPORT_V7.md](IMPROVEMENT_REPORT_V7.md)**.

## License & attribution

- **Source code** — [MIT](LICENSE).
- **Bundled data & machine-generated content** — CC BY-SA 4.0 (+ GFDL where
  inherited from Wiktionary), because it derives from ShareAlike sources.

Slavic evidence and official lemmas come from the Interslavic dictionary
(interslavic-dictionary.com) and Interslavic reference materials by Jan van
Steenbergen (interslavic.fun, steen.free.fr); etymological data from English
Wiktionary via Wiktextract (CC BY-SA / GFDL). Generated Interslavic forms are
**machine-generated reconstructions**, not authoritative standard Interslavic.

Full credits and reuse terms: **[ATTRIBUTION.md](ATTRIBUTION.md)**.
