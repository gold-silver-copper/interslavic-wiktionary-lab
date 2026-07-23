# Pipeline stages: the DAG as it actually is (V15 item 8)

Every stage below declares its command, the files it reads, and the files it
owns (writes). The standing rule: **no stage reads another stage's raw
inputs**. The multi-gigabyte Wiktionary dumps are read by the four `extract-*`
stages and by nothing else; everything downstream reads only the committed
caches and artifacts those stages own. A stage that wants new information from
a dump gets it by changing an extractor, never by opening the dump itself.
Symmetrically, ownership is exclusive: exactly one stage writes each artifact
(`export` owns `data/novel-words.tsv`; `evaluate` owns
`data/score-calibration.json`; `corpus-eval --fit` owns
`data/corpus-calibration.json`).

```
 data/raw-wiktextract-data.jsonl (local, 23 GB)      data/wiktionary/{ru,pl,cs}-extract.jsonl (local)
   │            │             │                                    │
   ▼            ▼             ▼                                    ▼
 extract-    extract-      extract-raw-slavic ──────────────► extract-enrich
 proto       lemmas           │            │                       │  (wanted set =
   │            │             ▼            ▼                       │   lemma ∪ raw caches
   ▼            ▼          raw-slavic-   raw-slavic-               │   ∪ official)
 proto-      slavic-       lemmas.cache  coverage.json             ▼
 slavic.     lemmas.          │                            wiktionary-enrich.cache.json
 cache.json  cache.json       │                                    │
   │            │             │      data/official-isv.csv         │
   │            │             │        (refresh-official)          │
   ├────────────┼─────────────┼────────────┬───────────────────────┤
   ▼            ▼             ▼            ▼                       ▼
 corpus-eval --fit ──► data/corpus-calibration.json ──┐
 evaluate ───────────► data/score-calibration.json ───┤
   │                   reports/candidate-generation-* ┤
   ▼                                                  ▼
 benchmarks ──► reports/*                           export ──► site/ (scratch; Pages deploys)
                                                      └──────► data/novel-words.tsv (committed)
                                                                   │
                                              check-text / coin-check / en / translation-probe
```

`data-manifest` sits outside the DAG: it fingerprints every tracked `data/`
artifact into `data/MANIFEST.json` (the pinnable-release contract, see
[DATA-REFRESH.md](DATA-REFRESH.md) and the root `INTEGRATION.md`).

## Extraction stages (dump → committed cache)

The dumps themselves are gitignored local datasets (`data/raw-wiktextract-data.jsonl`,
`data/wiktionary/`); the caches they produce are committed, schema-stamped, and
are the only extraction products anything downstream may read. A corrupt or
stale-schema cache is a hard error at load (`dump::load_optional`, V15 item 2);
only a genuinely absent optional cache degrades, with a printed notice.

| Stage | Command | Reads | Owns |
|---|---|---|---|
| extract-proto | `make extract-proto` / `cargo run --release -- extract-proto --dump …` | the Wiktextract dump | `data/proto-slavic.cache.json` |
| extract-lemmas | `make extract-lemmas` / `… extract-lemmas --dump …` | the Wiktextract dump | `data/slavic-lemmas.cache.json` |
| extract-raw-slavic | `make extract-raw-slavic` / `… extract-raw-slavic --dump …` | the Wiktextract dump | `data/raw-slavic-lemmas.cache.json`, `data/raw-slavic-coverage.json` (drop-reason tally) |
| extract-enrich | `make extract-enrich` / `… extract-enrich --dir data/wiktionary` | `data/wiktionary/{ru,pl,cs}-extract.jsonl`, `data/slavic-lemmas.cache.json`, `data/raw-slavic-lemmas.cache.json` (optional), `data/official-isv.csv` | `data/wiktionary-enrich.cache.json` |

Ordering: `extract-enrich` builds its wanted-word set from the lemma and raw
caches, so it runs after `extract-lemmas` and `extract-raw-slavic`.
`make extract-all` runs all four in dependency order. The raw cache is
deliberately separate from the lemma cache and is **never read by a benchmark
path** — it feeds display-only raw-attestation pages and the enrichment
wanted set.

## Calibrators (fit on committed data, committed back)

| Stage | Command | Reads | Owns |
|---|---|---|---|
| corpus-eval --fit | `cargo run --release -- corpus-eval --fit` | `data/official-isv.csv`, `data/slavic-lemmas.cache.json`, `data/proto-slavic.cache.json` (optional) | `data/corpus-calibration.json` |
| evaluate (refit side effect) | `make eval` / `… evaluate` | `data/official-isv.csv`, `data/proto-slavic.cache.json` | `data/score-calibration.json` (+ its `reports/` outputs below) |

Both calibrators are freshness-guarded in CI: `ci.yml` refits each and
`git diff --exit-code`s the committed file, so neither can silently go stale
against the code that scores. The two calibrators are domain-checked and
mutually incompatible by design: the corpus-coverage score domain must never
be read through the official-row pipeline calibrator or vice versa.

## Export (the site)

| Stage | Command | Reads | Owns |
|---|---|---|---|
| export | `make export` / `cargo run --release -- export --out site` | `data/official-isv.csv`; `data/slavic-lemmas.cache.json` (required for the corpus site); `data/proto-slavic.cache.json`, `data/raw-slavic-lemmas.cache.json`, `data/wiktionary-enrich.cache.json` (optional display caches); `data/corpus-calibration.json`; `data/score-calibration.json`; `data/raw-slavic-coverage.json`; `data/curation-notes.json` (optional); `reports/candidate-generation-summary.json` and `reports/synonym-accuracy.md` (metrics/about page numbers, V15 item 9 — read, never invented) | the `site/` tree (HTML + `api/` + `search/` + root JSON datasets) **and** `data/novel-words.tsv` |

The export also writes `site/build-info.json` (V15 item 8): a
machine-readable provenance stamp — git revision, crate versions, the pinned
`data_release`, and the sha256 of each input cache — so any deployed tree
names its exact inputs even outside the release ritual.

`site/` is scratch (gitignored); `.github/workflows/pages.yml` rebuilds and
deploys it to GitHub Pages on every master push. `data/novel-words.tsv` is the
one committed export output; CI `git diff`-guards it after the production
export. Export determinism is proven, not assumed: CI exports twice and diffs
the sha256 of every file; `SOURCE_DATE_EPOCH` / `SLOVOWIKI_BUILD_GIT` pin the
provenance fields for cross-revision byte comparison.

The single-owner rule for `data/novel-words.tsv` (V15 item 3): `src/novel.rs`
holds the row type, writer, and parser; export builds rows in memory, writes
the TSV once, and hands the same rows to the checker index directly.
`check-text` and `coin-check` read the committed file back through the same
parser — no hand-rolled column splits anywhere.

## Benchmarks

All benchmarks read `data/official-isv.csv` (plus the caches noted) and write
human/machine reports into `reports/` (`--out`, default `reports`). The 22
blessed report snapshots in `reports/` are committed; regenerating them is how
a benchmark change becomes visible in review.

Make-covered:

| Stage | Command | Extra reads | Report(s) |
|---|---|---|---|
| evaluate | `make eval` | proto cache | `reports/candidate-generation-{report.md,summary.json}`, `methodology.md`, `predictions.csv`, `regressions.csv`, `improvements.csv`, `errors-sample.csv` |
| proto-eval | `make proto-eval` | proto cache | `reports/proto-engine-report.md` |
| audit | `make audit` | proto cache | `reports/stage-attribution.md`, `audit-misses.csv` |
| corpus-eval | `make corpus-eval` | lemma + proto caches | stdout only (no report file); `--fit` writes the calibrator above |
| aspect-eval | `make aspect-eval` | — | `reports/aspect-pairs.{md,tsv}` (frozen manifest; CI diff-guards it) |
| coverage | `make coverage` | raw cache + tally, enrich cache | `reports/raw-coverage.{md,json}` |
| translation-probe | `make probe` | a prior `export` (`site/api/en`), `tools/translation-probe.txt` | `reports/translation-probe.md` (reported metric, never a gate; baseline pinned in `src/site/english_api.rs::PROBE_BASELINE` and `data/MANIFEST.json`) |
| search-perf | `make search-perf` | a prior `export` | `reports/search-performance.md` |

Internal (no Make target; run via `cargo run --release -- <cmd>`). These are
maintainer benchmarks and audits — CI runs `inflect-eval` directly as a guard:

| Stage | Nature | Report(s) |
|---|---|---|
| inflect-eval | inflection census + RULE_SPEC §3 invariants (CI gate) | `reports/inflection-report.md` |
| derive-eval | word-formation layer vs naive baseline | `reports/derivation-report.md` |
| multiword-eval | multi-word slices + historical aspect baseline | `reports/multiword-aspect.md` |
| evidence-eval | root-absent recoverability + augmentation A/B | `reports/evidence-growth.md` |
| checktext-eval | fixture classification + agreement gold/error sets | `reports/checktext-report.md` |
| synonym-eval | synonym-inclusive accuracy (also read by export's about page) | `reports/synonym-accuracy.md` |

Diagnostic-only, **answer-reading** (they read the official lemma to bound
headroom and can never feed production):

| Stage | Report(s) |
|---|---|
| oracle | `reports/oracle-ladder.md` |
| select-eval | `reports/cluster-selection.md` |
| rep-eval | `reports/rep-selection.md` |

## Release and refresh tools

| Stage | Command | Reads | Owns |
|---|---|---|---|
| data-manifest | `make manifest` / `… data-manifest [--write [--release N]]` | every git-tracked `data/` file, the crate pin, `data/refresh-changelog.md`'s newest `### data-vN` heading | `data/MANIFEST.json` (with `--write`; default mode verifies) |
| refresh-official | `… refresh-official <downloaded.csv>` | a manually downloaded interslavic-dictionary.com export (local file; nothing here touches the network) | `data/official-isv.csv`, prepends to `data/refresh-changelog.md` |

Both are governed by the ceremony in [DATA-REFRESH.md](DATA-REFRESH.md);
`data-release.yml` re-verifies every pushed `data-v*` tag.

| Tool | Command | Reads | Owns |
|---|---|---|---|
| dump-output | `… dump-output [--out FILE]` | `data/official-isv.csv`, `data/novel-words.tsv` | nothing committed — prints the FNV-1a fingerprint of the canonical record dump (pinned in a unit test) |
| diff-output | `… diff-output BEFORE AFTER` | two `dump-output` files | nothing — enumerates the record-level diff, turning "the fingerprint moved" into a reviewable list |

## Consumer CLIs (not pipeline stages — they only read)

`check-text`, `coin-check`, `en`, `explain`. They read the committed artifacts
(official CSV, `data/novel-words.tsv`, the evidence caches for false-friend
warnings) or a prior export's `site/api/`, and write nothing into the repo.
Their output contracts are specified in the root `INTEGRATION.md`.

## Committed vs scratch

| Committed (pinned by `data/MANIFEST.json` where under `data/`) | Scratch |
|---|---|
| `data/official-isv.csv`, the four `data/*.cache.json` caches, `data/raw-slavic-coverage.json` | `site/` (gitignored; rebuilt from scratch by `export` and by `pages.yml`) |
| `data/novel-words.tsv`, `data/score-calibration.json`, `data/corpus-calibration.json` | `target/` (all of it — nothing tracked lives there) |
| `data/MANIFEST.json`, `data/refresh-changelog.md` | `data/raw-wiktextract-data.jsonl`, `data/wiktionary/` (local source datasets) |
| `reports/` — the 22 blessed benchmark snapshots (moved out of gitignored `target/eval` in V15 item 9) | `data/*.tmp` |
