# Task: V13 — final pre-translation slice: project lexicons and coinage hand-off

You are working in `/Users/kisaczka/Desktop/code/slovowiki` (crate `interslavic-wiktionary-lab`). V12 (archived at `docs/history/IMPROVEMENT_PROMPT_V12.md`, PR #109) is verified complete: 210 tests, the 219-word game-vocabulary probe at 147 verified / 44 generated-only / 28 miss, `coin-check` working end-to-end (`jabberwok` fails phonotactics → `žabervok` passes all four axes).

This brief is deliberately minimal: it contains **only what blocks starting a real translation project** (the mrzavec roguelike, whose game text will contain sanctioned coinages inflected at runtime). Three items. Evidence expansion (new native-Wiktionary editions), site polish (#81), and pinnable releases (#74) are all deferred to a later stage — they improve quality at the margins but gate nothing. House rules at the bottom.

## 1. Project-lexicon support in `check-text` (flagship)

The gap: a translation project necessarily contains **sanctioned coinages** (`žabervok`, `kserok`, `akvator`) and inflects them at runtime (`žabervoka`, `žabervokom`, …). Today `check-text` reports every such inflected form as `unknown`, so the project's most valuable QA loop — "run all rendered game text through `check-text --summary --max-unknown 0` as a CI test" — is drowned by its own sanctioned words.

Add `check-text --lexicon <file>`:

- **File format**: TSV, one row per project word: `lemma  pos  gender  animacy  gloss` (gender/animacy blank for non-nouns; document the format in the agent guide). Design it so `coin-check --json` output can be appended into it mechanically (item 2).
- **Behavior**: at load, build each lexicon lemma's full paradigm in memory via the `interslavic` crate — the same machinery `coin-check`'s declinability axis already uses — and index every cell under its folded key. Matching tokens classify with a new status `project` (with the usual `analyses`), distinct from `known-lemma`/`generated`/`unknown`. `--summary` counts `project` separately, and `--max-unknown` no longer counts sanctioned coinage inflections.
- **Consistency check**: when a token is a verification-grade *official* word whose English gloss overlaps the gloss of some lexicon row, but the token's lemma is *not* that row's lemma, emit a `consistency` warning ("text uses `sekyra`, project lexicon maps 'axe' to `topor`") — deterministic gloss-token overlap, same normalization as the English API. This catches register drift: the same source concept rendered by different target words across a large text. Gate optionally via `--max-consistency N`.
- Validate the lexicon file itself on load: every row's lemma must pass `coin-check`'s collision axis *or* be official (a project may pin an official word); reject rows whose declared POS/gender contradict the crate's requirements. Errors, not warnings — a broken lexicon must not silently weaken the gate.

Fixtures: a text with `žabervoka` flags `unknown` without the lexicon and `project` with it; a consistency fixture with two official synonyms; a rejected malformed row.

## 2. `coin-check` metadata overrides and lexicon hand-off

`coin-check` previews only the paradigm the crate would *guess* from the ending. A real consumer controls gender/animacy explicitly (`ISV::noun_with`) — the preview must match what the project will actually do:

- Add `--pos <noun|adj|verb>`, `--gender <m|f|n>`, `--animacy <anim|inanim>`; the declinability axis renders the overridden paradigm and *also* prints the guess, flagging divergence ("ending suggests feminine; you declared masculine animate").
- Print the guessed gender/animacy explicitly in the human report (today it says only "as noun").
- Add `--lexicon-row`: emit the item-1 TSV row for the validated word (declared metadata passed through), so the coinage workflow chains mechanically: `coin-check → append row → check-text --lexicon`. In `--json`, include the same as a `lexicon_row` field.

## 3. Commit the translation probe as a tracked benchmark

The 219-word game-vocabulary probe has steered three releases but lives outside the repo — the V11 PR had to reconstruct a 58-word approximation because of that. Commit the full list as `tools/translation-probe.txt` (categories as comments; it is Rogue-5.4.5 vocabulary, no license concern) plus a tiny runner (`en --batch` + summary report) wired into the validator suite as a *reported metric, not a gate* (coverage moves with data; the report keeps PRs honest without freezing them). Record the current baseline: 147 verified / 44 generated-only / 28 miss.

## Validation (do all, report numbers)

1. `cargo test` green including the new fixtures; clippy; all validators.
2. Full export; every selftest passes; byte-identical double export.
3. Probe via the new tracked runner — must report 147/44/28 (items 1–2 touch no data, so any movement is a bug).
4. Item 1 end-to-end demo in the PR: a sample "game text" containing official words, one sanctioned coinage inflected in three cases, and one consistency violation — with `--summary --max-unknown 0` passing only when the lexicon is supplied.

## House rules (unchanged)

1. **Deterministic and offline** — `export`/`check-text`/`en`/benchmarks never touch the network; committed caches only; no ML; explainable string algorithms.
2. **Benchmark honesty** — accuracy paths untouched; curated knowledge only as test expectations.
3. **Static-only** — plain files, self-tests, no server.
4. **Contract discipline** — keep consumer-visible shapes or bump schemas; agent guide and README updated in the same commit as each feature.
5. **Reviewable commits** — one per numbered item; honest regressions stated plainly.
