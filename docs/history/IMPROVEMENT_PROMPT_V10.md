# Task: Remove manual curation from slovowiki and strengthen it for translation work

You are working in `/Users/kisaczka/Desktop/code/slovowiki` (crate `interslavic-wiktionary-lab`). Slovowiki's identity is **algorithmic**: deterministic reconstruction from Slavic cognate evidence, honest benchmarks, static self-testing artifacts. Two files violate that identity by injecting hand-written knowledge, and a real translation dry-run (the mrzavec roguelike vocabulary, see "Motivating measurements" below) exposed weaknesses that should be fixed algorithmically at the same time.

Your job, in two parts: (1) remove `data/overrides.toml` and `data/semantic-notes.json` entirely, replacing what they did with algorithms over the existing evidence caches; (2) a set of translation-focused improvements. Everything you add must follow the house rules at the bottom.

## Part 1 — Remove manual curation

### 1a. Remove `data/overrides.toml` and `src/overrides.rs`

Current wiring: `src/overrides.rs` (a 105-line TOML reader) is consumed by `src/generator.rs` (the `Overrides` parameter of `generate_with_official_byforms`, the `overrode` flag on generated results), `src/pipeline.rs`, `src/eval.rs`, `src/site/mod.rs`, and declared in `src/lib.rs`. The file holds 3 entries (computer→`kompjuter`, football→`futbol`, jazz→`džaz`) — internationalisms with idiosyncratic loan adaptation. The benchmark already excludes overrides from pure-algorithm accuracy, so removing them cannot change accuracy metrics; only the production site output for those meanings changes.

Steps:
- Delete the file and module; remove the parameter/flag threading from generator, pipeline, eval, and site; remove override mentions from README, `context.md`, and the agent guide (`agent_guide()` in `src/forms.rs`).
- **Replace, don't just delete.** These three words were overridden precisely because the algorithm could not derive them — so removal is only honest if you also try to close that gap algorithmically. The loan-adaptation knowledge is already written down as *rules*: `data/FLAVORIZATION_SPEC.md` and the respelling conventions referenced in the old override comments (loan `[dʒ]` → `dž`, never etymological `đ`; English `-er` retained, etc.). Extend the generation rules for borrowed cognate sets (ru компьютер / pl komputer / cs počítač…, ru футбол / pl futbol…) so the pipeline itself produces the adapted internationalism as a `generated` candidate.
- Turn the old override list into **test fixtures, not runtime data**: unit tests asserting the algorithm now derives `kompjuter`, `futbol`, `džaz` from cached evidence. If a case genuinely cannot be derived yet, mark the test `#[ignore]` with a comment and accept the honest miss — do NOT reintroduce a curated lookup under another name.

### 1b. Remove `data/semantic-notes.json` and the curated `api/notes.json`

Current wiring: `src/check.rs` (`SEMANTIC_NOTES` const at L27, `SemanticNote` struct, the `notes` map on the checker, the `warning`/`prefer` fields in check-text JSON), `src/site/english_api.rs` (notes loaded ~L339, `warnings`/`prefer` copied onto every English candidate ~L370–421, field docs ~L536–542), the notes artifact emission and site plumbing (`src/site/mod.rs`, `assets.rs`, `special.rs`, `navigation.rs`), and the agent-guide text in `src/forms.rs`. The file holds 12 hand-written false-friend warnings (`pytati` ≠ torture, `jutro` ≠ tomorrow, `čas` ≠ hour, …).

**Replace with algorithmic false-friend detection.** The evidence is already on disk:

- `data/wiktionary-enrich.cache.json` (gzipped JSON): 112,349 entries from *native* ru/pl/cs Wiktionaries — `{lang, word, senses (native-language), synonyms, related, etymology}`.
- The English-Wiktionary wiktextract caches (`data/wiktionary-lab.json`, `data/slavic-lemmas.cache.json`): Slavic words with **English** glosses.
- The official dictionary rows: per-language cognate cells + the official English gloss.

Algorithm sketch (deterministic, no ML):

1. For each official ISV lemma `L` with English gloss set `G` (split comma senses), fold `L`'s surface with the standard orthography fold.
2. For each Slavic language `ℓ`, find words in the caches whose folded surface equals (or near-equals, per the existing fold/broadening rules) `L`'s — i.e. words a speaker of `ℓ` will *read as* `L`.
3. Retrieve that word's English glosses from the English-Wiktionary cache; compute gloss overlap with `G` by deterministic token comparison (lowercase, stopword-strip, light suffix-strip — same style as the existing normalization code).
4. If surfaces collide but gloss overlap is zero/below a fixed threshold, emit a **computed** warning record: `{isv_lemma, language, colliding_word, divergent_glosses}` — enough for a consumer to render "official meaning X; `ℓ` speakers may read it as Y".
5. Emit these as the new notes artifact (keep the `warnings` array shape on English-API candidates and check-text output so consumers don't break; the artifact itself is now generated, versioned, and counted in `meta.json`). Drop `prefer` (purely editorial), or compute it as: the official lemma whose gloss set best overlaps the divergent sense.

Validation: the 12 old curated notes are your held-out sanity set — the algorithm should independently rediscover most of them (`pytati`/пытать, `jutro`/pl jutro, `čas`/sr-bg час…). Encode them as test expectations (fixtures again, not runtime data). Report precision honestly: if the detector also fires on collisions the curators never listed, inspect a sample — many will be *correct new findings*, which is the point of doing this algorithmically.

## Part 2 — Translation-focused improvements

### Motivating measurements (mrzavec dry-run, 219 game words through `api/en`)

Canonical gameplay nouns resolved to verified candidates ~2/3 of the time (potions 11/14, scrolls 13/18, rings 8/14, sticks 10/14, weapons 5/9, armor 6/8, monsters 12/26, traps 5/8); flavor vocabulary worse (stones 9/26, woods 14/33). The failures cluster:

- **Morphology misses**: `heal` → `lěčiti` verified, but `healing` → nothing verified; same for `searching`, `mapping`, `invisibility` (official `nevidimy` exists and `-osť` derivatives are generated, but no English key connects them).
- **Genuine gaps**: `teleport` — zero rows anywhere official.
- **Gloss-token traps**: `staff` surfaces only `načeľnik štaba` ("chief-of-staff").

### 2a. English-side morphological normalization (build + query)

- **Build side**: key generated derivatives under *English* derived glosses, computed from their `deriv:<pattern>` tag: `-osť` → gloss + `-ness`/`-ity` ("invisible" → "invisibility"), adverb pattern → `-ly`, `-ńje` → `-ing`/`-tion`, `ne-` → `un-`/`in-` + base gloss. Purely mechanical string transforms on the base's gloss — no dictionaries of exceptions.
- **Query side**: extend the documented retry ladder with deterministic de-suffixing: on a miss, retry the key with `-ing`/`-ation`/`-ition`/`-ity`/`-ness`/plural `-s/-es` stripped (and the reverse where applicable). Add the new steps to `api/en/meta.json`'s normalization contract and to `api/en/selftest.json` samples.

### 2b. An `en-lookup` CLI subcommand

`cargo run --release -- en <query> [--json]`: performs normalization, FNV routing, and the full retry ladder (article strip → per-content-word → de-suffixing) internally and prints ranked candidates. Agents currently reimplement the router by hand; every reimplementation is error surface. Reuse the exact same code path the exporter uses, so CLI and static API cannot drift.

### 2c. Fold ranking evidence into candidate records

Choosing between synonyms today requires joining three files (`api/en` candidates, `entries.json` attestation, official CSV `frequency`). Add to each English-API candidate (and to `api/lemmas.json` rows): `frequency` (official CSV column), `langs` (attesting-language count), `branch_pattern` (`V+Z+J` style), `borrowed`. All four already exist in the pipeline — this is plumbing, not new computation. Bump the relevant schema versions and update the migration notes in the agent guide.

### 2d. `check-text` summary mode for CI gating

`check-text <file> --json --summary` (or a trailing summary object): counts by token status (`known-lemma`/`known-form`/`generated`/`unknown`), agreement-error count, and a nonzero exit code when `unknown > 0` or agreement errors exist (flag-controlled thresholds). Downstream projects (e.g. a game translation) can then run "all rendered messages verify clean" as a test.

### 2e. Internationalism eligibility in generation

The `teleport` family is absent while its cognates (ru телепорт, pl teleport, cs teleport) exist in Wiktionary. Investigate whether borrowed/internationalism cognate sets are being filtered out of candidate generation (check the `borrowed` handling in the corpus/consensus path). Borrowed evidence should still yield `generated` candidates — flagged `borrowed: true`, adapted per the same loan-adaptation rules as 1a — because "the whole Slavic world borrowed this word" is itself pan-Slavic evidence. This, not a curated list, is the algorithmic answer to vocabulary gaps.

### 2f. Documentation and contract updates

Regenerate/update the agent guide text in `src/forms.rs::agent_guide()` for every change above (notes are now computed; `prefer` removed or computed; new retry-ladder steps; new candidate fields; the `en` CLI). Update `README.md`. Bump `api/en/meta.json` schema and the form-index schema only if record shapes change. Every new artifact or router step ships with a selftest, following the existing pattern (`router-selftest.json`, `en/selftest.json`).

## House rules (do not violate)

1. **Deterministic and offline**: build only from committed caches; no network at export time; no ML models or embeddings — plain, explainable string algorithms with fixed thresholds.
2. **Benchmark honesty**: nothing you add may leak hand-picked answers into accuracy metrics. Curated knowledge is only acceptable as *test expectations* — never as a runtime input the pipeline reads.
3. **Static-only**: the API remains plain files + client-side self-tests; no server.
4. **Contract discipline**: keep field shapes where consumers exist; bump schema versions on breaking change; document in the agent guide *in the same commit*.
5. **Verify end-to-end**: `cargo test`, full `cargo run --release -- export --out site`, re-run both selftests against the fresh export, and re-run the mrzavec dry-run categories above to show the coverage numbers moved (report before/after in your summary).

Work in reviewable commits: 1a, 1b, and each 2x item separately. If a replacement algorithm underperforms its curated predecessor, say so plainly in the commit message — an honest regression that keeps the system algorithmic is acceptable; silent quality loss or hidden curation is not.
