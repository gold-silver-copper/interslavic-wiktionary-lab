# Task: V14.2 — fix the ten round-two review findings (the fixes' own bugs)

You are working in `/Users/kisaczka/Desktop/code/slovowiki` on branch `agent/v14-releases-valence` (PR #114, CI green). The second adversarial review of the full PR found ten issues — **four introduced by the V14.1 fixes themselves** (live-reproduced), the rest latent traps, concessions, and doc drift. Fix on this branch, grouped as the eight items below (findings 3+6+10 share machinery; 7 stands alone; 9 absorbs the doc cluster). One commit per item.

Baseline: 225 tests, probe 147/44/28, evaluate 42.02%, aspect manifest frozen, `data-manifest` OK. **Item 3 deliberately changes the exported `api/forms` bytes** (additive analyses) — everything else must leave the export untouched; the validation section says how to prove both.

## 1. Put the lexicon disposition data IN the JSON, not in front of it (finding 1 — CONFIRMED live)

`check-text --lexicon … --json` emits `lexicon: 5 rows — …` on stdout before the JSON: every parser breaks, and the adoption-visibility data never reaches the machines it was built for.

- The human summary line prints **only in non-JSON mode** (it is part of the human report, not a diagnostic — not stderr).
- `--json --summary` gains an **additive** `"lexicon"` field beside `"summary"`: `{rows, coinages, official_pins, adoptions: [{lemma, adopted_gloss}]}` — deterministic (file order). Bare `--json` stays a bare array (shape kept; consumers wanting dispositions use `--summary`, and the guide says so).
- Regression test: `--json --summary --lexicon` output parses as one JSON object containing `lexicon.adoptions`; bare `--json --lexicon` output parses as a JSON array with NO leading prose. Update the `--json --summary` "one parse" comment and the guide/README (item 8 carries the doc sweep; the shape lands here).

## 2. Adoption must consider EVERY same-POS proposal, and carry its match out (finding 2 — CONFIRMED on live data)

`novel-words.tsv` really contains `tur`/aurochs and `tur`/"prison, jail" (same surface, same POS). The guard binds to `.find(first)`, so the second concept is unadoptable, and `apply_lexicon` re-derives "the" proposal with a weaker predicate to fetch the summary gloss.

- Selection: among colliding generated lemmas, filter POS-matching candidates; a row adopts if **any** candidate passes BOTH guards (exact spelling, gloss-token overlap). Rejections aggregate honestly: if spelling-matched candidates exist but none overlap, quote **each** candidate's gloss; if only respellings exist, name them.
- Carry the match out instead of re-deriving: `RowDisposition::GeneratedAdoption { adopted_gloss: String }` (drop `Copy`; tests use `matches!`/pattern-binds). `apply_lexicon` consumes the payload — **delete** the second `by_key` lookup and its `unwrap_or_default()` (this also closes the round's simplification/efficiency findings on that path). `label()` stays for display.
- Tests: `tur … prison` adopts with `adopted_gloss` containing "prison"; `tur … aurochs` adopts the other; an overlapping-neither gloss rejects quoting BOTH glosses; the existing emuk suite still passes.

## 3. One enrichment, both consumers — shared in forms.rs (findings 3 + 10 + 6)

The animate-accusative readings exist only in check-text's index: the published `api/forms` still says `netopyŕa` = gen-only, the guide's "record layer" sentence misleads API consumers, and the "exported site is untouched" comment is factually wrong (site export calls `build_index` for suggestion shards; byte-stability was accidental). Meanwhile the enrichment matches hardcoded `"gen.jd."`-style literals against labels forms.rs builds from its constants, and the absorbing-insert pattern is hand-rolled at three sites.

- **Move the enrichment into forms.rs** as the shared pass the module doctrine promises: `pub fn enrich_animate_accusatives(records: &mut Vec<FormRecord>, masc_animate_keys: &HashSet<String>)`, with the (gen→akuz) label pairs **derived from the same constants/formatter `paradigm_records` uses** (extract `noun_feature_label(case, number)`; no free-floating literals — a label-shape change then breaks compile/tests, not silently the enrichment). Hoist the cheap analyses check before any `form_key` allocation, and take a precomputed key set (one lookup per record).
- **Both index builders call it**: `check::build_index` (set built from its absorbing maps) and the site export before `write_api` (set built from official entries via a shared `masc_animate_lemma_keys(entries)` helper — same absorbing discipline, one implementation; this is also where the three hand-rolled absorb/gender-char sites collapse into `absorb_insert` + `gender_char`, making the helper's "shared by" doc comment true).
- This is a **deliberate consumer-visible export change**: analyses are additive on existing records (counts unchanged, no new keys, no schema bump — analyses are an open vocabulary; the guide documents the new readings). Fix the now-dead "site untouched" comment; the guide's record-layer sentence becomes true as written; note the api/forms byte change plainly in the commit and PR.
- **Finding 6 (gender gate) is resolved by documentation + canary, not by widening**: the `'m'` gate is linguistically required (feminine a-stems have distinct accusatives — enriching `ženy` would be WRONG), and abstaining on mixed-gender-absorbed keys is the correct conservative choice. Make the latent case a *visible event*: a test asserting current data contains **no animate lemma key with absorbed gender** — a refresh that introduces one fails the test and forces an explicit decision instead of silently shrinking valence coverage. Document the abstention beside the gate.

## 4. `--animacy ""` must reject, not guess (finding 4 — CONFIRMED live)

`animacy.unwrap_or("")` conflates an explicitly empty argument with "not passed"; the old code rejected `Some("")` loudly, the new code silently emits a row with the crate's guessed animacy — the unset-`$ANIMACY` shell trap. Fix: only `None` means undeclared; `Some(raw)` goes through `parse_animacy` **after** an ensure that `raw` is non-empty (parse_animacy's `""`-is-blank arm is for the TSV column and stays). Test: `Overrides::parse(None, None, Some(""), …)` errors; `None` still guesses.

## 5. Pin the plural-object concession as a decision, not an accident (finding 5)

The singular guard silenced a true-positive class the pre-fix code caught (`Hybiš žabervokov`). The concession is CORRECT — an animate genitive plural after an intransitive is indistinguishable from a quantitative genitive (`Pribyvaje vojakov` has no numeral to key on either) — but it is currently undocumented and untested, i.e. an accident waiting to be "fixed" back into a false-positive generator.

- Add a **conceded-class test**: `On spi žabervokov.` (lexicon-loaded) asserts NO flag, with a comment stating the concession and why widening is wrong.
- Document the abstention in the guide's valence sentence ("plural object-shaped forms never fire — conceded to the partitive") and drop the now-misleading half of the enrichment's `akuz.mn.` motivation comment if it implies valence uses it (the plural akuz readings still serve preposition government — `Ględajemo na vojakov` — so they are NOT dead weight; say which consumer uses them).

## 6. Release identity: manifest knows its N, the ritual is enforceable (finding 7)

`data-v1` points at a pre-hardening mid-branch commit whose manifest format the current verifier rejects, and nothing ties a tree to its release number.

- **Delete the remote `data-v1` tag now** (pre-merge, zero consumers — say so in the commit/PR) and re-tag on the merge commit per the ritual.
- Manifest gains `"data_release": N` — **bump `MANIFEST_SCHEMA` to 2** (a versioned schema changed shape; that is what the version is for). `data-manifest --write --release N` sets it; `--write` without `--release` carries the committed manifest's current N forward (so ordinary data changes don't need to restate it); verify mode requires the field. The DATA-REFRESH ritual updates: bump N at release time, and each release adds a `### data-vN` heading atop the changelog section it covers, so entries map to pins.
- CI: a small `on: push: tags: ['data-v*']` job that checks out the tag and runs `cargo run --release -- data-manifest` — a moved or malformed release tag becomes a red X instead of a silent lie.

## 7. CI guard: split the steps, stop conflating exit codes (finding 8)

- Two steps: **"Verify data manifest"** (the cargo line, every event) and **"Refresh changelog guard"** with `if: github.event_name == 'pull_request'` (on master pushes it was vacuous anyway — the fetch cost and its failure surface disappear; direct-push bypass remains, note it as accepted: the repo merges via PRs).
- In the guard: `set -euo pipefail`; fetch failure is a **warn-and-skip** (`::warning::cannot fetch base master — changelog guard skipped`), not a red herring — the manifest step still hard-fails real drift; and the diff's exit code is inspected explicitly (`rc=1` → changed, `rc>1` → fail loudly AS a git error with its own message; no `if ! git diff` polarity trap, no pipe masking — use `git diff --name-only … > files.txt` then grep the file).
- Re-run the depth-1 clone simulations for: clean pass, CSV-without-changelog fire, git-failure path prints the git error (not the policy error).

## 8. Doc truth sweep (finding 9, plus item 1/3 doc landings)

- **Singularize correctly**: tiny `plural(n, "adoption")`-style helper (or `if n == 1`) so output matches the guide's documented `"1 official pin, 1 adoption"`; unit-test the format via the `AppliedLexicon` summary method (extract it from `run()` while there — it becomes testable and reusable, which was a round-one suggestion).
- **README**: add the load-summary line, the `lexicon` JSON field (item 1), `lexicon_row_disposition`, and one sentence on the animate-reading enrichment (now API-visible via item 3) — the same-commit rule was breached twice in this PR; this commit closes the ledger and says so.
- **Make the indeclinable gender requirement real instead of inert**: an indeclinable noun's single surface IS its every case — emit the full case×number analyses set on the indeclinable lemma record (same surface, all readings) in `apply_lexicon`. Adjective agreement then actually consults the declared gender: `zelena emu` (gender mismatch) flags, `zeleny emu` stays clean, `emua` stays unknown — three new assertions. The parse comment's justification becomes true; if this interacts badly with the valence object-shape test (an all-cases form is never object-shaped — verify), state that in the test.

## Validation (do all, report numbers)

1. `cargo test` green with all new tests; clippy `-D warnings`; fmt; three site validators.
2. **Export**: byte-identical double export; vs the pre-V14.2 branch head, the ONLY content change is item 3's additive analyses in `api/forms` shards (+ guide/meta bytes) — prove with the tree-diff file list; record/key/lemma counts unchanged.
3. Probe **147/44/28**; evaluate **42.02%**; aspect manifest + calibrations zero-drift; `data-manifest` OK at schema 2.
4. Live repros re-run: `--json --summary --lexicon | jq .` parses and shows `lexicon.adoptions`; `tur…prison` adopts; `--animacy ""` rejects; `Ględaš črěz netopyŕa` clean in BOTH the CLI and a fresh export's `api/forms` analyses; `zelena emu` flags.
5. ReportFindings outcomes updated per finding; CI green including the split guard and (post-merge) the tag-verify job on the re-tagged `data-v1`.

## House rules (unchanged)

Deterministic/offline; benchmark honesty (item 3 touches no accuracy path — analyses are reading metadata, not generation); static-only; contract discipline (guide + README in the same commit as each behavior change; the api/forms change and MANIFEST_SCHEMA 2 documented where consumers look); reviewable commits, one per item; honest regressions stated plainly.
