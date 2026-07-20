# Task: V11 — precision, ranking, and trust improvements to slovowiki

You are working in `/Users/kisaczka/Desktop/code/slovowiki` (crate `interslavic-wiktionary-lab`). The V10 brief (archived at `docs/history/IMPROVEMENT_PROMPT_V10.md`, implemented in PR #107) removed both manual-curation files and shipped the translation tooling: algorithmic false friends (`src/falsefriends.rs`), loanword recovery, English morphological normalization, the `en` CLI, ranking-evidence fields, and the `check-text --summary` gate. It worked: on a 219-word real-game vocabulary, verified coverage went 131→147 and outright misses 74→29.

V11 is the quality pass. A post-merge review found the new machinery is *broad but imprecise* in specific, reproducible ways. Every item below comes with observed failing examples — turn each one into a regression test. House rules from V10 still bind (bottom).

## 1. False-friend warning precision

`api/notes.json` now has 3,947 computed notes. An 18-note random sample (seed 42 over sorted keys) shows roughly 3⁄4 are genuine traps — and the failures are systematic, not random:

- **Spelling-variant false positives.** `avtožir`: official "autogyro" vs ru "autogiro" — the same word, one letter apart, zero shared tokens. Fix: before declaring divergence, compare gloss tokens by small edit distance / shared-prefix stem, not just equality (`autogyro`≈`autogiro`, `chirrup`≈`chirp` — the `cvrkot` note is the same bug).
- **Synonym false positives.** `zakuska`: "snack" vs "appetizer, hors d'oeuvre"; `kote`: "kitten" vs "cat". Token comparison can't see synonymy. Deterministic fix available in the caches: build an English-synonym closure from the English-Wiktionary `synonyms`/`related` links (and the official dictionary's own comma-separated gloss lists, which pool synonyms per meaning) and treat closure-mates as overlapping.
- **Secondary/slang senses presented as the reading.** `banan`: pl *banan* primarily means banana; the warning quotes only the slang senses ("cheeser; rich kid") because per-record divergence fires on the slang record. Keep per-record divergence (it exists for the pl-*jutro* reason) but use the wiktextract sense tags/labels (slang, figurative, Internet, colloquial, dated) and sense order: when the *primary untagged* sense agrees, either suppress the note or emit a clearly-worded "colloquially also…" variant with lower severity.
- **Junk glosses.** "YouTube poop, edited videos designed in a humorous and surreal manner" (`pup`). Clean quoted glosses: drop senses tagged as proper-noun/pop-culture/Internet, cap quoted length at a sentence, never mid-word truncate.
- **Severity.** Add a computed severity/confidence field per note (inputs: primary-sense vs secondary-sense divergence, number of languages colliding, official-lemma frequency) so consumers can show strong traps first. 3,947 undifferentiated warnings bury `pytati` under `avtožir`.

Keep the 8 rediscovered curated traps (`pytati`, `jutro`, `čas`, `urok`, `rok`, `slovo`, `koristny`, `trg`) as must-still-fire tests; add the false positives above as must-not-fire (or must-downgrade) tests. Report the before/after note count and re-sample precision in your summary.

## 2. Fix or fail-closed the computed `prefer`

`prefer` quality is far below warning quality — several are wrong in ways that would actively mislead a translator:

- `staja` (trap sense: 'shepherd hut / stable') → prefers `stabiľny` ("stable" the *adjective*) — English polysemy poisons the collision-coverage scoring.
- `banan` (slang 'rich kid') → prefers `dětę` ("child").
- `cvrkot` ('bustle, stir') → prefers `mrdati`; `kazniti` ('execute') → prefers `smŕť` (a noun for a verb sense); `gojiti` → prefers `vaga`.

Fixes, in order: require POS compatibility between the divergent sense and the preferred lemma; score on multi-token / closure-aware overlap rather than any single shared token; and when the best score is below a fixed threshold, **emit no prefer at all** — an empty list is honest, a wrong suggestion is not. Encode the four examples above as regression fixtures. Expect and accept that most notes end up with empty `prefer`.

## 3. English-API ranking: don't let a wrong verified hit outrank the right word

Observed: `en staff` returns verified `načeľnik štaba` ("chief-of-staff", a `gloss-token` match) as the top candidate, with the semantically correct `posoh`/`ščap`/`drevko` (`exact-gloss-head`, generated) below it. A translation agent that takes the first verified candidate gets a comically wrong word. "Verified before generated" is the right *trust* statement but the wrong *relevance* statement.

Do not silently reorder trust tiers. Instead:

- Within the response, make match quality a first-class axis: rank `exact-gloss-head` above `gloss-token` *within* each trust tier (already the case — verify), and have the CLI/API flag the situation where the top verified candidate is only `gloss-token` while an `exact-gloss-head` candidate exists in a lower tier (e.g. a `sense_note` on the response, and the CLI printing the exact-head block first with the verified gloss-token hit clearly labeled "phrase/derived sense").
- Use the V10 ranking-evidence fields as deterministic tie-breakers within tier+match: higher `frequency`, then more `langs`, then lexicographic. Document in the agent guide; freeze CLI output for the `staff` case as a test.

## 4. Cross-POS completion of recovered borrowings

The V10 raw-recovery pass (2e) produces nouns well (`teleportacija`) but the **verb `teleport` still misses** (pl `teleportować` vs mk bare-stem adaptations don't share a consonant fingerprint). Close the gap derivationally instead of loosening the fingerprint: for each recovered borrowed noun in `-acija`/`-ija`, generate the matching `-ovati` verb (and its `-uje` present stem) via the existing derivation machinery, gated on at least one raw verb attestation in any language (pl `teleportować`, ru телепортировать(ся) both exist in the raw cache). Emit as `generated`, `borrowed: true`, null probability, `deriv:` analysis pointing at the recovered noun. Target regression: `en teleport` (verb) and `en "teleport to"` produce a candidate. Same pattern likely rescues other -ation/-ate families — report how many.

## 5. Calibrate the uncalibrated (open issue #90, now more urgent)

V10 added 8,667 recovered borrowings and V10's derivative indexing surfaces many more generated candidates in translation flows — all with `probability: null` ("fail closed"). That was right for shipping, but null gives a translation agent nothing to reason with. Fit leakage-free calibrators:

- **Recovered borrowings**: holdout = official internationalisms (`genesis=I` rows). Hide each from the pipeline, run the recovery pass, measure exact-match rate as a function of the gate features (languages, branches, gloss-token agreement); assign the Wilson-95 lower bound per feature bucket, capped like the derivation probabilities.
- **Corpus reconstructions** (the long-standing gap behind #90): the coverage-score calibrator, holdout-validated on official rows reachable by the corpus path only.

Every probability must carry the same contract as before: model-specific, suggestion-never-verification. Update the agent guide's trust rules with the new models' definitions.

## 6. Notes delivery and integration polish

- Shard `api/notes.json` (1.7 MB monolith) like the suggest index — route by `fnv1a32(folded_key) % 64` — so text-check and API consumers fetch per token; keep a `notes-selftest.json`; count shards/bytes in `meta.json`. Bump the notes schema and agent guide together.
- Include the new `severity` (item 1) in check-text output and English-API candidate `warnings`, so `--summary` can gate on severe traps only if desired (`--max-severe-warnings N`).

## 7. `en --batch`: lexicon-building mode

Building a game lexicon means hundreds of queries; today that is one process spawn + selftest + shard reads per word. Add `en --batch <file> [--json]`: one query per line, one selftest pass, shard cache reused across queries, output keyed by input line. Deterministic output order = input order. This is the workhorse for any future translation project — make its JSON the thing an agent can consume directly (per query: best-verified, best-generated, miss).

## Validation (do all, report numbers)

1. `cargo test` — all green, including the new regression fixtures from items 1–4.
2. Full `cargo run --release -- export --out site`; all selftests pass against the fresh export.
3. Re-run the 219-word game-vocabulary probe (categories and baseline in `docs/history/IMPROVEMENT_PROMPT_V10.md`; V10 result: 147 verified / 43 generated-only / 29 miss). Item 4 should move `teleport`-class words; items 1–3 must not regress coverage.
4. Re-sample 18 notes (seed 42, sorted keys) and report the same-meaning false-positive count vs the 4–5 observed pre-V11; spot-check that the four bad `prefer` fixtures now emit either a sensible suggestion or nothing.

## House rules (unchanged from V10)

1. **Deterministic and offline** — committed caches only; no network at export; no ML models; fixed thresholds; explainable string algorithms.
2. **Benchmark honesty** — nothing here may touch pure-algorithm accuracy paths; calibrators are holdout-validated and leakage-free; curated knowledge appears only as test expectations.
3. **Static-only** — plain files, self-tests, no server.
4. **Contract discipline** — keep consumer-visible field shapes or bump schema versions; update `agent_guide()` in `src/forms.rs` and README in the same commit as each feature.
5. **Reviewable commits** — one per numbered item; honest regressions stated plainly in the commit message.
