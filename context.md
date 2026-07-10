# Code Context

## Files Retrieved
1. `README.md` (lines 1-75) - project overview: Slovowiki is a static Wiktionary-style site driven by the whole Slavic Wiktionary lemma corpus, with extract/export commands and counts.
2. `src/main.rs` (lines 39-100, 249-258) - CLI entry points and export fallback from corpus site to official-dictionary-seeded site.
3. `src/dump.rs` (lines 1-155) - Wiktextract data pipeline types and `extract_lemmas`, including language list, lemma filtering, gloss/category capture.
4. `src/corpus.rs` (lines 1-137, 356-503) - cognate-set construction and generation; no minimum evidence filter, confidence derived from language/branch coverage.
5. `src/site.rs` (lines 234-832, 2134-2248) - corpus export, dedup/suppression, official-only page inclusion, search index schema, and client-side search implementation.

## Key Code

- CLI data locations (`src/main.rs` lines 39-50): defaults include `data/slavic-lemmas.cache.json`, `data/wiktionary-enrich.cache.json`, and the raw Wiktextract path.
- Export path (`src/main.rs` lines 249-258): `cargo run -- export` uses `site::export_corpus` when `data/slavic-lemmas.cache.json` exists, otherwise falls back to `site::export` from official dictionary evidence.
- Lemma extraction (`src/dump.rs` lines 41-75, 97-155): `LemmaEntry` contains `lang`, `word`, `pos`, English `gloss`, `proto`/`etymon`, `etymology`, `categories`, `topics`, `tags`; `extract_lemmas` streams Wiktextract and keeps Slavic entries with Proto-Slavic ancestry or borrowing templates.
- Corpus grouping (`src/corpus.rs` lines 75-137): inherited words group by normalized Proto-Slavic ancestor + POS class; borrowings group by union-find over Slavic phonemic skeleton and source etymon skeleton.
- Low-evidence behavior (`src/corpus.rs` lines 356-503): `generate_set` accepts every nonempty set; `coverage_confidence` marks 1-2 language/single-branch items Low, but does not exclude them.
- Search index rows (`src/site.rs` lines 714-737, 788-811): rows are `[id, form, gloss, pos, status, confidence, score, keys, n_langs, n_branches, borrowed, quality, first, ancestor]`.
- Searchable keys (`src/site.rs` lines 2134-2167): `search_keys` indexes generated candidate forms, standard folds, and ASCII skeletons; matched official entries additionally add official English gloss tokens (`src/site.rs` lines 715-722). Official-only pages index folded ISV forms only (`src/site.rs` lines 788-795).
- Client search (`src/site.rs` lines 2176-2248): `scoreAll` matches display form, folded form, candidate keys, exact/split English gloss (`e[2]`), and substring of English gloss. It does not search Slavic-language meanings/senses from native Wiktionary enrichment.

## Architecture

Pipeline:
1. `extract-lemmas` reads English Wiktextract JSONL and writes `data/slavic-lemmas.cache.json` (observed: 46,654 entries).
2. `corpus::build_sets` groups lemma entries into inherited and borrowed cognate sets.
3. `corpus::generate_set` creates ranked Interslavic candidates and coverage confidence.
4. `site::export_corpus` renders entry pages, special pages, `search.json`, and `search.html`. Official dictionary is display/cross-link layer only: generated matches get official headwords; official lemmas not covered by candidates get official-only pages.
5. `extract-enrich` writes native RU/PL/CS enrichment (`data/wiktionary-enrich.cache.json`, observed: 52,819 entries), loaded by `site::export_corpus` for display blocks and semantic links, not for search scoring.

## Current gaps / issue candidates

- medium: `src/site.rs` lines 714-737 and 2176-2248 - search by meaning is English-centric. The index stores one truncated English gloss plus candidate-form keys; native RU/PL/CS enrichment senses are displayed elsewhere but are not added to `search.json` or searched. Issue: add compact multilingual meaning keys (RU/PL/CS, maybe all Slavic Wiktionary glosses from `LemmaEntry.gloss`) with language tags and weighted scoring.
- medium: `src/site.rs` lines 788-811 - official-only pages only index ISV form folds, not English gloss tokens in `keys`; they remain searchable through row gloss substring (`e[2]`) but not through normalized token/fold strategy. If adding Slavic meanings, official-only pages need parity.
- low/needs-decision: `src/site.rs` lines 428-457 - same-concept suppression removes duplicate pages from rendering/search when same folded form and overlapping gloss token has stronger set. This is intentional dedup, but if the goal is literally “include all Slavic Wiktionary dataset words,” issue should clarify whether suppressed duplicate cognate sets need discoverability (e.g., alternate senses on kept page) rather than standalone pages.
- low/needs-verification: `src/corpus.rs` lines 356-503 - low-evidence/raw interslavicized sets are not filtered by coverage; they are rendered as Low confidence if generation produces a nonempty form. However entries with empty form are skipped in `site::export_corpus` line ~331. Issue should target auditing skipped/empty-form sets and exposing low-confidence raw forms if desired.
- low: `src/dump.rs` lines 127-155 - extraction currently requires single-token lemma, nonempty Proto-Slavic ancestor or borrowing etymon, and a gloss. Multiword Wiktionary lemmas and entries without etymological links are outside the dataset by design; “all Slavic Wiktionary dataset words” should be scoped to `LemmaCorpus`, not all Wiktionary Slavic pages.

## Start Here

Open `src/site.rs` at lines 714-737 and 2134-2248 first. That is where `search.json` is built and where client-side search scoring decides which fields are searchable. For inclusion/completeness questions, then open `src/corpus.rs` lines 75-137 and 356-503.

## Commands Run

- `ls` - repository top-level scan.
- `find src`, `find site`, `find data` - mapped code, generated site, and data caches.
- `grep search|slovowiki|Wiktionary|evidence|meaning|gloss` - located search/export/data-pipeline code.
- `python3` cache metadata inspection - confirmed `data/slavic-lemmas.cache.json` has 46,654 entries and `data/wiktionary-enrich.cache.json` has 52,819 entries.

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "Concrete findings include file paths/line ranges and severity-tagged issue candidates for search meanings and corpus inclusion gaps."
    }
  ],
  "changedFiles": [
    "/Users/kisaczka/Desktop/code/interslavic-wiktionary-lab/context.md"
  ],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "ls; find src/site/data; grep targeted terms; python3 cache metadata inspection",
      "result": "passed",
      "summary": "Mapped repository entry points, pipeline, search implementation, and confirmed cache counts."
    }
  ],
  "validationOutput": [
    "Wrote concise scouting report to context.md with review-findings and residual-risks."
  ],
  "residualRisks": [
    "Did not exhaustively inspect all enrichment rendering code; search gap assessment is based on index construction and client scoring paths.",
    "Line numbers around long src/site.rs may drift if files change."
  ],
  "noStagedFiles": true,
  "diffSummary": "Added context.md scouting findings only.",
  "reviewFindings": [
    "medium: src/site.rs:714-737,2176-2248 - search index/scoring omits native Slavic-language meanings/senses from enrichment and lemma glosses.",
    "low/needs-decision: src/site.rs:428-457 - same-concept suppression may conflict with a literal requirement to expose every Wiktionary cognate set.",
    "low/needs-verification: src/site.rs:331 and src/corpus.rs:356-503 - low-evidence sets are generally included, but empty generated forms are skipped and need audit if raw inclusion is required."
  ],
  "manualNotes": "No code changes beyond writing the requested context report."
}
```
