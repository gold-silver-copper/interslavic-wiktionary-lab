# Task: V15.1 — fix all 16 findings from the PR #115 adversarial review

Review verdicts: 14 confirmed (one demonstrated empirically), 2 plausible. All 16 get fixed — the 10 reported findings and the 6 cut by the output cap. Same discipline as V15: every commit is **Track A** (behavior-preserving, export tree hash proven) or **Track B** (behavior change declared with evidence). Current baseline (pinned env `SLOVOWIKI_BUILD_GIT=v15 SOURCE_DATE_EPOCH=1784592509`): `ad450ab69f154cc28e273afe2416d682ddea0a590f5ae55201590395ca926632`. Items 2 and 8 change page bytes — declared; re-baseline after each and enumerate the delta. One reviewable commit per item below.

## 1. Make the CI accuracy floor fail-closed (findings 1 + 7 + the annotation detail)

`.github/workflows/ci.yml` floor step, three defects, one commit:
- `cargo run --release -- evaluate | tail -12` masks an evaluate failure (no pipefail in the default shell — the repo's own line-80 comment documents this), after which python reads the **stale committed** summary and passes vacuously. Fix: `set -euo pipefail` as the step's first line (keep the pipe), so a broken evaluator fails the step before the check runs.
- Both the floor and `BenchSummary` trust `runs[-1]` positionally. Assert the marker: the python check must verify `run["name"].endswith("(production)")` and hard-fail otherwise ("ladder no longer ends at the production rung — fix eval.rs ordering or this check"). Do the same in `BenchSummary::load` (item 2 owns that code; coordinate, don't duplicate).
- The `::error::` text sits inside a Python `AssertionError` — tracebacks go to stderr and GitHub never renders the annotation. Replace the assert with an explicit `print("::error::…"); sys.exit(1)` on stdout.

## 2. One machine-readable channel for every published metric (findings 2 + 6 + 4, one Track B commit with the page delta enumerated)

The root cause behind three findings: `synonym-eval` and `corpus-eval` publish numbers only as markdown/stdout, so the site either scrapes prose or hardcodes.
- `run_synonym_eval` (eval.rs) additionally writes `reports/synonym-summary.json`: `{schema: 1, synonym_inclusive, strict_exact, strict_normalized, miss_breakdown: {valid_synonym, other_sense, non_official}}` — the same numbers its markdown table states.
- `corpus-eval` (the non-`--fit` report path) additionally writes `reports/corpus-summary.json` with its exact/normalized rates and denominator.
- `BenchSummary::load` reads ONLY json (candidate-generation-summary.json + the two new files); **delete the markdown cell-scraper entirely**. Add the `(production)` marker assert from item 1.
- Wire the two remaining hardcoded metric paragraphs (special.rs:1197 miss breakdown 12,2/7,9/79,8 — measured is 12.3/8.1/79.6; special.rs:1162 corpus-path 58,31/62,84) to `BenchSummary`. The corrected numbers ARE the declared page delta; state before/after values in the commit.
- CI freshness (finding 4): after the floor's `evaluate`, extend the diff guard to `reports/candidate-generation-summary.json`; add a CI step running `synonym-eval` and `corpus-eval` with `git diff --exit-code` on their new summary jsons (if runtime is prohibitive, say so in the commit and guard at minimum the summary jsons' consistency with their md siblings). Fix `BenchSummary`'s doc comment to describe the guard that actually exists — no aspirational claims.

## 3. Loud failure on a non-CSV official file (finding 3, Track B — error path only)

`official::load`: after parsing the header, `ensure!` it contains the `isv` column; the error must name the likely cause ("no `isv` column in the header — the loader reads comma-separated CSV only; got a header of N column(s): first cell `…`"). This catches TSV (demonstrated: header parses as one CSV cell, everything downstream silently classifies unknown), semicolon exports, and any wrong file. No TSV support returns — the deletion stands, the failure just becomes diagnosable. Add a unit test feeding a TSV header and asserting the message.

## 4. build-info.json provenance from the existing truth sources (finding 5 + the sha256 dup)

- Make `release::resolved_pin()` and `release::sha256_file()` `pub(crate)`; `build_info_json` uses them. The Cargo.toml line-trim dies — release.rs's own doc (V14.1 finding 6) already condemned exactly that parse. The stamped value becomes the **resolved** version from Cargo.lock (`0.13.0`, no `=` operator), which is also the truthful value under a `[patch]` override. Keep the NotFound→null wrapper for caches as a thin adapter over `sha256_file`.
- This changes `build-info.json` bytes (`interslavic_pin: "=0.13.0"` → resolved `"0.13.0"`; consider renaming the field `interslavic` to match its new meaning — INTEGRATION.md updated same commit). Declared delta, one file.

## 5. Wall-clock ban that cannot disable itself (finding 8)

Rewrite the CI step: run grep, capture `rc`; `rc==0` → print the hits and fail; `rc==1` → pass; `rc>1` → fail with "grep itself failed". Pattern changes: drop `Instant::now` (monotonic, not a reproducibility hazard — over-ban invites blanket comment-dodging), keep `SystemTime::now|chrono::|OffsetDateTime|Zoned::now`. Extend scope to the committed-report writers outside src/: grep `tools/` for `new Date|Date.now|datetime.now|time.time(` alongside. Fix `tools/search-perf.mjs`'s stale `target/eval` default output path to `reports/` while there (it writes a committed report). Comment-line false positives are an accepted bluntness — say so in the step comment; a code-comment mentioning a banned API is cheap to reword.

## 6. Retire the falsefriends degrade-with-warning ghost (finding 9, Track A)

Delete `warn_if_unreadable` and its three call sites — provably unreachable after V15 item 2 (`load_optional` hard-errors on exists-but-unloadable; the `!loaded && exists()` condition needs a TOCTOU race). Rewrite the doc comment above `compute_from_default_caches` to state the ACTUAL contract: absent caches → fewer notes, silently; corrupt caches → hard error naming the cache. Hash unchanged.

## 7. Restore the export/CLI record equivalence by construction (finding 10, Track A)

In `export` (site/mod.rs), after `write_tsv`, feed the checker index from `crate::novel::parse(&tsv)` instead of the raw in-memory rows. This keeps the disk round-trip dead but restores exactly what the disk trip enforced: quantized probabilities, sanitized glosses, and file-line id numbering, identical to what check-text/coin-check read back. One line plus a comment stating the invariant ("the export index must see the rows exactly as CLI consumers will"). The in-memory `novel_rows` remain for `write_tsv` only. Hash must be unchanged (probabilities never reached artifacts — prove it stays true).

## 8. The six below-cap findings (grouped, small commits)

- **fnv dedup**: eval.rs's private `fn fnv1a` (eval.rs:3126) is byte-identical to `fingerprint::fnv1a64`; delete the private copy, call the public one. `EXPECTED_MANIFEST_FNV` must not move (same algorithm — the test proves it).
- **Test-allow blocks**: replace the 28 per-module `#![allow]` copies (already 3 drifted shapes) with one `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::panic, clippy::unwrap_in_result, clippy::indexing_slicing))]` at the top of src/lib.rs — only the unwrap-family lints; the pedantic entries were redundant with Cargo.toml's global `allow`s and pre-masked future ratchets. Lib-target clippy (compiled without cfg(test)) still enforces the denies on production code; note this in a comment beside the attribute.
- **Doc rot**: CONTRIBUTING.md lines 74/105 `target/eval/` → `reports/`; README + the generated `api/agent-guide.md` gain the `build-info.json` line the V15 brief itself required; datasets.html's novel-words.tsv row drops the false "header-only" claim (page delta — declared, rides item 2's re-baseline or its own); derive.rs module doc points at `derive_eval::run_eval`.
- **Fingerprint test cost**: the pinned test early-returns under `cfg!(debug_assertions)` with an eprintln ("pin enforced in release runs — CI and `cargo test --release`"); debug `cargo test` stops paying the full index build + two ~60MB strings.
- **Missed fold site**: eval.rs:2557–2564 (`ql` lowercased, then `to_standard(&ql)`) becomes `fold_key(query.trim())` — same two steps, one idiom; verify the surrounding uses of `ql` still get the lowercased form they need. eval.rs:865 stays: it is `to_standard` WITHOUT lowercase (a different, pre-existing idiom) — add a one-line comment so the next fold audit doesn't re-flag it.
- **Sanitization test successor**: new site/tests.rs test rendering a corpus-site page (`official_only_page` or `corpus_entry_page`) for an official entry whose `isv` cell is `"foo, bar"`, asserting the raw comma-joined string never appears in the HTML — the coverage the deleted fallback-banner test used to provide.

## Validation (all, report numbers)

1. Per-commit: Track A → export hash unchanged at the then-current baseline; Track B → delta enumerated (files + before/after values). `cargo test --release` green; clippy clean under the V15 table.
2. End state: 237± tests (new: TSV-header test, sanitization successor; unchanged pins: fingerprint `262dd798416d323f` must NOT move — items 7 and 8's fnv dedup are the risky ones, prove it), evaluate exact top-1 42.02%, probe 147/44/28, aspect 17.88/48.71/89.07, `data-manifest` OK.
3. Deliberately break evaluate locally (rename a cache) and confirm the item-1 floor step now FAILS instead of passing on the stale summary — paste the failing output in the PR comment.
4. Feed the item-3 TSV probe and paste the new error message.
5. Push to the existing PR #115 branch (`agent/v15-provable-refactor`); comment on the PR mapping each of the 16 findings to its fix commit.

## House rules (unchanged)

Deterministic and offline; benchmark honesty (nothing here touches an accuracy path); static-only; contract discipline (INTEGRATION.md/README/agent-guide updated in the SAME commit as the artifact they describe); reviewable commits, one per item; honest regressions stated plainly.
