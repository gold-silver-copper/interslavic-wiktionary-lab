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
- `cargo run -- export` — generate the cognate-set site (~22.4k words; falls back to the
  dictionary-seeded site if the lemma cache is absent).
- Independent validation: **~5.2k generated words already exist as official Interslavic
  lemmas** (of ~22.4k), with no leakage from the dictionary into the generation.
- `cargo run -- corpus-eval` scores this site path against the dictionary directly:
  **55.3% exact / 59.4% normalized** on the ~6.9k entries with a known ancestor.
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
| exact top-1 | 27.38% | **38.42%** | +11.04 pp |
| normalized top-1 | 34.96% | **45.50%** | +10.54 pp |
| normalized top-3 | 42.89% | **56.9%** | +14.0 pp |
| normalized top-5 | — | **59.6%** | — |
| mean normalized edit distance | 0.253 | **0.229** | −0.024 |

The **site's** cognate-set path (`corpus::generate_set`) is benchmarked separately
(`cargo run -- corpus-eval`): **56.9% exact / 61.1% normalized** on the ~6.9k entries
where a Proto-Slavic ancestor or internationalism is known — higher than the pipeline
headline because it only scores words the site actually derives from a known ancestor.

A data-quality **audit** (`cargo run --release -- audit`) classifies every miss:
~47% *wrong-cluster* (the official root is in the evidence but a different one
was chosen — mostly editorial synonym choices Interslavic makes), ~32%
*right-cluster-wrong-form* (engine/reconstruction error), ~21% *root-absent*
(the official root is not in any modern cognate — unfixable from evidence).
89.5% of meanings split across ≥3 cognate clusters. This maps the ceiling for
future word-selection work.

The Proto-Slavic rule engine is measured in isolation by a dedicated benchmark
(`cargo run --release -- proto-eval`): on the words it confidently links to a
reconstruction it derives the official lemma with **43.98% exact / 49.36%
normalized** accuracy.

**Confidence calibration** (high-confidence candidates match far more often — as intended):

| confidence | n | normalized match |
|---|---:|---:|
| high | 6,826 | 64% |
| medium | 7,248 | 35% |
| low | 2,226 | 11% |

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
  site.rs          static site generator (export) — HTML pages + search index
data/
  official-isv.csv        the full official dictionary (evidence + gold)
  overrides.toml          manual curation file
  RULE_SPEC.md            authoritative Proto-Slavic → Interslavic rule spec
  proto-slavic.cache.json Proto-Slavic reconstructions (built by extract-proto)
  slavic-lemmas.cache.json every inherited + borrowed Slavic lemma (built by extract-lemmas)
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

# Data-quality / ceiling audit (classifies every miss):
cargo run --release -- audit

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
- the **official-dictionary match status**: *officially attested* / *differs from
  official* (both shown) / *no official entry*;
- full **inflection tables** generated by the local `interslavic` crate.

## Benchmark artifacts

```
target/eval/candidate-generation-summary.json   per-rung metrics (machine-readable)
target/eval/candidate-generation-report.md      full human-readable report
target/eval/regressions.csv                      matched before, not after
target/eval/improvements.csv                     newly matched
target/eval/errors-sample.csv                    nearest remaining misses
```

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
