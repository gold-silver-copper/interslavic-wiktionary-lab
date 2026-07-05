# Interslavic Wiktionary Lab

An **evidence-based Interslavic (Medžuslovjansky) candidate-generation engine** with a
reproducible accuracy benchmark against the official Interslavic dictionary, plus a
local Wiktionary-style website that shows, for every meaning, the generated candidate,
its rule trace, the Slavic evidence by branch, a calibrated confidence, and whether it
matches the official dictionary.

No SQLite / database. No hotlinked Wikimedia CSS/JS. Everything is native Rust with an
in-memory index.

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
| exact top-1 | 27.38% | **33.83%** | +6.45 pp |
| normalized top-1 | 34.96% | **40.62%** | +5.66 pp |
| normalized top-3 | 42.89% | **50.94%** | +8.1 pp |
| mean normalized edit distance | 0.253 | **0.237** | −0.016 |

**Confidence calibration** (high-confidence candidates match far more often — as intended):

| confidence | n | normalized match |
|---|---:|---:|
| high | 4,601 | 67% |
| medium | 9,410 | 35% |
| low | 2,289 | 10% |

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
   `-al→-alny`, `-ive→-ivny`, verbs→`-ovati`, plus `au→av`, `eu→ev`.
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
   consensus. This rung adds **+1.4 pp exact / +1.7 pp top-3** over the consensus-only
   config.

## What was rejected (regressed the benchmark)

Recovering flavored letters (`ć/đ`, jat `ě`, `*y`) from *modern reflexes* is too noisy —
each experiment regressed accuracy. The correct source (rule spec §4.4) is the
**Proto-Slavic reconstruction**, which the `+proto-derived` stage above now uses. The
consensus-level `palatals`/`jat`/`y-recovery` toggles remain in the report's *rejected
experiments* table as documented negatives.

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
  proto_link.rs    leakage-free meaning → reconstruction linker (3 signals)
  pipeline.rs      two-stage §4.4 merge (consensus root + proto-derived form)
  overrides.rs     manual curation (TOML), excluded from pure-algorithm accuracy
  generator.rs     orchestrator: pipeline + overrides + official match status
  eval.rs          reproducible benchmark, ablation ladder, report writers
  site.rs          build + serve the local Wiktionary-style website
data/
  official-isv.csv        the full official dictionary (evidence + gold)
  overrides.toml          manual curation file
  RULE_SPEC.md            authoritative Proto-Slavic → Interslavic rule spec
  proto-slavic.cache.json Proto-Slavic reconstructions (built by extract-proto)
```

## Commands

```bash
# One-time: stream the 23 GB dump into the Proto-Slavic cache (enables the
# +proto-derived stage). Skip it and the engine falls back to consensus only.
cargo run --release -- extract-proto --dump /Users/kisaczka/Desktop/code/english/raw-wiktextract-data.jsonl

# Reproducible benchmark against the official dictionary (fast, no dump needed):
cargo run --release -- evaluate --official data/official-isv.csv --out target/eval

# The acceptance-criteria invocation also works (the metadata TSV lacks
# translations, so it transparently falls back to the bundled full export):
cargo run --release -- evaluate \
  --dump /Users/kisaczka/Desktop/code/english/raw-wiktextract-data.jsonl \
  --official /Users/kisaczka/Desktop/code/interslavic-rs/crates/interslavic/data/dictionary_metadata.tsv

# Build the website dataset and serve it:
cargo run --release -- build --dump /Users/kisaczka/Desktop/code/english/raw-wiktextract-data.jsonl
cargo run --release -- serve            # http://127.0.0.1:8765

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

## Provenance / license note

Slavic evidence and the official lemmas come from the Interslavic dictionary
(interslavic-dictionary.com) and English Wiktionary/Wiktextract. Generated data keeps
source URLs; a public deployment needs a proper attribution/license page.
