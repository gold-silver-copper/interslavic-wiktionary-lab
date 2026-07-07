# Interslavic Wiktionary Lab

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
  source edition.
- **Extra meanings** — the native senses (a Russian entry often lists 10+ senses
  where the English gloss gives one).
- **Semantic web** — related, derived, synonym and antonym terms as chips, each
  linking back to its native Wiktionary. `water` alone surfaces 100+ links.

The cache is built by filtering the RU/PL/CS dumps to the ~70k cognate words that
actually appear in the corpus (streamed in seconds), so the enrichment is
committed and the site build stays self-contained.
- `cargo run -- export` — generate the cognate-set site (~22.4k words; falls back to the
  dictionary-seeded site if the lemma cache is absent).
- Independent validation: **~6.0k generated words already exist as official Interslavic
  lemmas** (of ~22.4k), with no leakage from the dictionary into the generation.
- `cargo run -- corpus-eval` scores this site path against the dictionary directly:
  **56.6% exact / 61.0% normalized** on the ~7.4k entries with a known ancestor.
- `data/novel-words.tsv` — 2,066 high/medium-confidence words the engine derived that
  are **not** in the official dictionary (candidate new vocabulary, with ancestors).

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
| exact top-1 | 27.52% | **39.92%** | +12.40 pp |
| normalized top-1 | 35.23% | **47.09%** | +11.86 pp |
| normalized top-3 | 43.26% | **57.90%** | +14.64 pp |
| normalized top-5 | — | **60.62%** | — |
| mean normalized edit distance | 0.252 | **0.226** | −0.026 |

The **site's** cognate-set path (`corpus::generate_set`) is benchmarked separately
(`cargo run -- corpus-eval`): **56.6% exact / 61.0% normalized** on the ~7.4k entries
where a Proto-Slavic ancestor or internationalism is known — higher than the pipeline
headline because it only scores words the site actually derives from a known ancestor.

A data-quality **audit** (`cargo run --release -- audit`) classifies every miss and
attributes it to the pipeline **stage** that lost the official form (a full
`RuleStep`-trace replay — see `target/eval/stage-attribution.md`): ~31%
*cluster/vote* (a different, usually editorial, root was chosen), ~21%
*merge/rank* (a correct candidate was generated but demoted — of which only ~2.6%
of all misses are a genuine same-cluster ranking bug, the rest being synonym
word-choice), ~21% *root-absent* (unfixable from evidence), ~18%
*normalize/representative*, ~7% *endings*, and only **~1.6%** the Proto-Slavic
*rule engine*. 89.5% of meanings split across ≥3 cognate clusters. A companion
**oracle ladder** (`cargo run --release -- oracle`, diagnostic-only) measures each
stage's upper-bound headroom: cluster +3.9pp / representative +3.7pp / proto-link
+2.6pp exact — the single biggest *non-editorial* lever is representative
selection.

The Proto-Slavic rule engine is measured in isolation by a dedicated benchmark
(`cargo run --release -- proto-eval`): on the 20.1% of words it confidently links
to a reconstruction it derives the official lemma with **46.68% exact / 52.74%
normalized** accuracy.

**Confidence calibration** (high-confidence candidates match far more often — as intended):

| confidence | n | normalized match |
|---|---:|---:|
| high | 6,996 | 68% |
| medium | 7,089 | 37% |
| low | 2,215 | 12% |

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

## What was rejected (regressed the benchmark)

Recovering flavored letters (`ć/đ`, jat `ě`, `*y`) from *modern reflexes* is too noisy —
each experiment regressed accuracy. The correct source (rule spec §4.4) is the
**Proto-Slavic reconstruction**, which the `+proto-derived` stage above now uses. The
consensus-level `palatals`/`jat`/`y-recovery` toggles remain in the report's *rejected
experiments* table as documented negatives.

## Testing

`cargo test` runs the unit suite (rules across `proto`, `normalize`, `orthography`,
`morph`, `consensus`, `corpus`, `dump`, `eval`). Every rule was **adversarially
audited and triple-checked** (a finder plus two independent verifiers reproducing each
bug against the binary); the confirmed bugs were fixed with a regression test each. CI
(`.github/workflows/ci.yml`) runs `fmt` + `build` + the tests **and fails if exact
top-1 drops below a floor** — the floor measures the *shipped* production config
(`runs.last()`), not the best ablation rung, and a test asserts the ladder ends at
`ConsensusConfig::production()`, so a production regression can't slip through.

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
  morph.rs         POS lemma endings + internationalism ending table
  proto.rs         Proto-Slavic → Interslavic ordered rule engine (+ tests)
  dump.rs          stream the 23 GB dump → Proto-Slavic cache + indexes
  proto_link.rs    leakage-free linker: explicit Wiktionary etymology + 3-signal fuzzy match
  pipeline.rs      two-stage §4.4 merge (consensus root + proto-derived form)
  overrides.rs     manual curation (TOML), excluded from pure-algorithm accuracy
  generator.rs     orchestrator: pipeline + overrides + official match status
  eval.rs          reproducible benchmark, ablation ladder, report writers
  corpus.rs        Wiktionary-corpus cognate-set dictionary + confidence model
  enrich.rs        native RU/PL/CS Wiktionary enrichment (etymology/senses/links)
  site.rs          static site generator (export) — HTML pages + search index
data/
  official-isv.csv        the full official dictionary (evidence + gold)
  overrides.toml          manual curation file
  RULE_SPEC.md            authoritative Proto-Slavic → Interslavic rule spec
  proto-slavic.cache.json Proto-Slavic reconstructions (built by extract-proto)
  slavic-lemmas.cache.json every inherited + borrowed Slavic lemma (built by extract-lemmas)
  wiktionary-enrich.cache.json native RU/PL/CS etymology/senses/links (built by extract-enrich)
  novel-words.tsv         engine-derived words absent from the official dictionary
```

## Commands

```bash
# One-time: stream the 23 GB dump into the Proto-Slavic cache (enables the
# +proto-derived stage). Skip it and the engine falls back to consensus only.
cargo run --release -- extract-proto --dump /Users/kisaczka/Desktop/code/english/raw-wiktextract-data.jsonl

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
```

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
- full **inflection tables** generated by the local `interslavic` crate.

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
