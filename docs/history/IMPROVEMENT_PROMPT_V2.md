# Implementation prompt V2 — Proto-Slavic-derived forms, root modeling, and website integration

Continue improving `interslavic-wiktionary-lab`. The engine, benchmark, and website
already exist and work (see `README.md`). This phase targets the two dominant, measured
error sources and wires the Proto-Slavic rule engine into the live pipeline.

## Starting point (do not re-derive — measure against it)

Current **production** config on the leakage-free benchmark (16,300 official single-word
entries, `cargo run --release -- evaluate`):

- exact top-1 **32.42%**, normalized top-1 **40.43%**, normalized top-3 **49.2%**.
- Confidence calibration is monotonic (high 67% / med 35% / low 10%) — **keep it that way**.

The remaining-error breakdown (in `target/eval/candidate-generation-report.md`) is the
map for this phase:

- **~46% of misses = different root / derivation.** Interslavic chose a different root
  than the plurality skeleton. Needs root/semantic-family modeling (§C below).
- **~21% = extra letter (epenthesis/ending), ~12% = missing letter, ~14% = single-letter
  substitution, ~6.5% = y/i.** Most of these are *flavored-letter* recovery failures
  (`ě`, `ć/đ`, `å`, `ȯ`, `y`, nasals) that the consensus path cannot get from modern
  reflexes — which is exactly why the `palatals`, `jat`, and `y-recovery` experiments
  **regressed** and were rejected.

## Core hypothesis (the thing to prove or falsify with numbers)

> The flavored letters must be derived from a **Proto-Slavic reconstruction**, not
> guessed from modern reflexes. Consensus should pick the *root*; the Proto-Slavic rule
> engine (already built and unit-tested in `src/proto.rs`) should supply the *form*.
> This is §4.4 of `data/RULE_SPEC.md`.

If wired correctly, re-enabling `ć/đ`, jat, `y`, `å`, and strong-yer `ȯ` — this time
sourced from the reconstruction — should **raise exact top-1** without the regressions
the modern-reflex versions caused.

## Non-negotiable rules (unchanged from V1)

1. **No change is kept unless it improves measured accuracy** on the benchmark, gated by
   an ablation rung, with regressions tracked. Add rungs to `kept_ladder()` /
   `rejected_experiments()` in `src/eval.rs`; a rejected rule stays documented, not merged
   into `ConsensusConfig::production()`.
2. **No leakage.** The benchmark path (`consensus::generate` / the proto linker) must
   never read `OfficialEntry::isv`. The proto reconstruction must be found from the dump
   via gloss/descendant evidence, *not* by looking up the answer.
3. Prefer **exact top-1** as the primary metric now (the official lemmas are flavored);
   keep normalized/top-3/edit-distance/POS/calibration reporting.
4. Keep it dependency-light, no SQLite, local CSS only.

---

## A. Build a Proto-Slavic cache from the dump (one-time stream)

The dump is **23 GB** (`/Users/kisaczka/Desktop/code/english/raw-wiktextract-data.jsonl`).
Stream it **once** and write a compact cache so `evaluate`/`build` never rescan it.

- New module `src/dump.rs`:
  - `extract_proto_cache(dump: &Path, out: &Path)` — stream line-by-line (reuse the
    cheap `top_level_lang_code` prefilter idea from the old `main.rs`, still in git
    history at commit `b1725e5`), keep only `lang_code == "sla-pro"`.
  - For each `sla-pro` page capture: `word` (the reconstruction, keep yers/nasals/jat),
    the head form with accent, `pos`, all `senses[].glosses`, the flattened
    **descendants** tree (lang_code + word, so we can match modern cognates), the
    Proto-Balto-Slavic / PIE references, and the stem-class hints from `categories`
    (e.g. "hard o-stem", "i-stem", "a-stem", accent paradigm).
  - Write `data/proto-slavic.cache.json` (gitignored; a few hundred MB is fine).
  - Add CLI: `cargo run --release -- extract-proto --dump <jsonl> --out data/proto-slavic.cache.json`.
- Load the cache with a struct like:
  ```rust
  struct ProtoEntry {
      word: String, pos: Pos, glosses: Vec<String>,
      descendants: Vec<(String /*lang_code*/, String /*form*/)>,
      pbs: String, pie: String, stem_class: Option<String>,
  }
  ```
- Build two indexes: by normalized gloss token, and by normalized descendant form
  (`lang_code:ascii_skeleton(form)`).

## B. Link each meaning to its Proto-Slavic entry (leakage-free)

For each `OfficialEntry`, find the best-matching `ProtoEntry` **without** using `isv`:

1. **Descendant-membership match (strongest).** Count how many of the official entry's
   per-language cognates (from `official.cells`, normalized via `normalize::normalize_cell`)
   appear in a proto entry's descendant tree (compare on `ascii_skeleton`). A proto entry
   whose descendants include the RU/PL/CS/... forms *is* the reconstruction for this root.
2. **Gloss overlap** (English gloss tokens ∩ proto glosses), as a tiebreak/fallback.
3. **POS agreement** as a filter.
- Emit a **link confidence** ∈ [0,1] from #descendant hits / #branches and gloss overlap.
  Only use the proto path when confidence clears a threshold (tune it on the benchmark).
- Handle homographs/multiple senses: a proto entry may spawn several official senses;
  pick per (root, POS) and allow the same proto entry to serve multiple meanings.

## C. Wire proto into the consensus path (spec §4.4 two-stage model)

Change `generator::generate` (and the benchmark's direct consensus call) so that:

1. Consensus still runs and picks the **winning cognate group** (the root) via the
   six-subgroup vote — unchanged, this is the anti-domination mechanism.
2. If a **confident proto link** exists for that root, run `proto::generate(proto_word,
   pos, gender)` and make it the **primary form candidate**; the modern-consensus surface
   becomes a scored alternative. Attach both traces so the entry page can show the
   reconstruction *and* the consensus.
3. If no confident link, fall back to today's consensus surface (current behavior).
- Add a `ConsensusConfig` (or a new orchestration flag) `proto_derived_form: bool` so
  this is a **benchmark rung**, not a global switch. Measure exact/normalized deltas.
- Now **re-test the previously-rejected rules as proto-sourced**:
  - `ć/đ` come straight out of `proto::palatals` — no South-reflex guessing.
  - jat `ě`, nasals `ę/ų`, `å` (liquid metathesis), strong-yer `ȯ/e` come from the
    reconstruction. Add rungs `+proto-palatals`, `+proto-jat`, `+proto-yers` and keep only
    those that improve.
- **`*y` handling:** the reconstruction carries `y` directly (e.g. `*bykъ`, `*dymъ`),
  so proto-derived forms should fix the `język/jęzik` class without the aggressive
  `y-recovery` that regressed. Verify on the y/i error bucket.

Expect the biggest lift in **exact top-1** (flavored letters) and in the
"flavored-letter not recovered" + y/i buckets. Watch for **regressions** where a proto
link is wrong (bad gloss match) — that is why link confidence gates the path.

## D. Root / semantic-family modeling (the ~46% bucket)

Where Interslavic picked a *different root* than the plurality skeleton:

1. Implement §4.2-step-3 of the spec: for a meaning, gather **candidate roots** across the
   six subgroups, and score each root by six-subgroup vote (not surface-form vote). When
   two+ subgroups sit on different roots, prefer the root that (a) is present in the most
   subgroups and (b) has the most "average" meaning.
2. Use the proto descendant graph to cluster cognates into families (forms sharing a
   proto ancestor are one root), so near-synonyms don't fragment the vote.
3. This is exploratory — gate it, expect modest gains, and keep the error analysis honest
   (many of these are genuine editorial choices no algorithm will reproduce).

## E. Website integration

Extend `src/site.rs` entry pages (and the `SiteEntry` build) to surface the new evidence:

- Show the **Proto-Slavic reconstruction** as the etymological source, with the
  Balto-Slavic / PIE references when present, linking to the Wiktionary reconstruction
  page.
- Show the **derivation trace from the reconstruction** (the `proto::RuleStep`s:
  yer-fall, metathesis, tj/dj, nasals, endings) alongside the consensus trace, so a
  reader sees both "which root (consensus)" and "which form (Proto-Slavic)".
- When the generated form now matches official because of the proto path, keep the
  `MatchStatus` banner; when it still differs, show the reconstruction so the user can
  judge.
- Add a small **provenance badge** on each candidate: `ProtoSlavicRule` vs
  `BranchConsensus` (already in `CandidateSource`).
- Optionally add a `/proto/<word>` page rendering a single reconstruction's full
  derivation (useful for spot-checking; reuse `eval::explain`'s shape).

## Acceptance criteria

- `cargo run --release -- extract-proto --dump <jsonl>` produces the cache; `evaluate`
  and `build` consume it and **do not rescan the 23 GB dump**.
- The report shows new rungs `+proto-derived-form`, `+proto-palatals`, `+proto-jat`,
  `+proto-yers`, `+root-modeling`, each with its measured delta; only accuracy-improving
  rungs are folded into `production()`.
- **Exact top-1 rises meaningfully** over the current 32.42% (target: clear the low-40s;
  the flavored-letter + y/i buckets total ~40% of misses), with **normalized top-1 not
  regressing** and **calibration still monotonic**.
- `regressions.csv` is inspected: any new regressions are from wrong proto links, and are
  bounded by the link-confidence gate (report the count).
- Website entry pages render the reconstruction, the PBS/PIE evidence, and the derivation
  trace; `serve` still works with local CSS only.
- `cargo fmt`, `cargo check` (0 warnings), `cargo test` (proto tests + any new link tests)
  all green.
- Spot-check via `explain` that these now match exactly (currently near-misses):
  `język`, `blågo`, `råzprostirati`, `měsęc`, `pęt`, plus the V1 set
  (`bog voda duša dobry pisati město oko`).

## Pitfalls (learned in V1)

- **Leakage** is the cardinal sin — link to proto by evidence, never by `isv`.
- Modern-reflex flavored-letter recovery **regresses**; do not re-add it. Source flavored
  letters from the reconstruction only.
- Gloss matching is noisy (homographs, multi-sense pages). Prefer descendant-membership
  linking; gate by confidence; report link coverage and precision.
- Dump performance: stream once, cache, never hold the whole dump in memory.
- Keep every change a benchmark rung. If it doesn't move the number, revert it and write
  down why (that is a result, not a failure).
