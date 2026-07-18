# Implementation prompt V4 — cognate-aware word selection, pipeline hygiene, and statistical consensus

You are improving `interslavic-wiktionary-lab`: an evidence-based engine that generates
Interslavic (Medžuslovjansky) lemmas from Slavic cognate sets and is scored against the
official Interslavic dictionary. Read `README.md`, then `IMPROVEMENT_PROMPT_V2.md` and
`IMPROVEMENT_PROMPT_V3.md` (the arc so far), and `data/VOTING_MACHINE_NOTES.md`.

Your mandate: **rework how the engine chooses words, using advanced statistical and
comparative-linguistics techniques, and audit every stage of the pipeline for data that is
silently corrupting the result — until the most accurate engine possible is reached.** Every
change is proven or rejected by measured accuracy against the canonical dictionary.

---

## 0. Ground rules (non-negotiable — the whole project rests on these)

1. **Keep only if it improves measured accuracy.** The reproducible benchmark is
   `cargo run --release -- evaluate` (leakage-free: the generator only sees the modern
   Slavic cognates, never the official lemma). Add every change as an **ablation rung** in
   `src/eval.rs` (`kept_ladder` / `rejected_experiments`). If a change does not improve
   **exact top-1** (the primary metric) without regressing **normalized top-1** or the
   **confidence calibration**, revert it and record why. A negative result is a result —
   V3's rejected experiments and `VOTING_MACHINE_NOTES.md` are the template.
2. **No leakage, ever.** Nothing on the benchmark path may read `OfficialEntry::isv`.
3. **Don't silently break existing wins.** Current production: **exact 34.72% / normalized
   41.48% / top-3 51.35%**; proto-engine alone **43.25%** on the 17.5% of words it links.
   Calibration is monotonic (high ≫ med ≫ low). Beat these; keep them monotonic.
4. **Interpretability is a feature.** Prefer transparent, traceable methods (every candidate
   carries a `RuleStep` trace and `Evidence`). Statistical/ML methods are welcome, but a
   black box that can't explain a choice is worth less here than a clear rule of equal
   accuracy. If you train anything, hold out data and report honestly.

## 1. The central problem: meaning-grouping conflates non-cognate synonyms

The engine builds a `MeaningInput` from the official dictionary's per-language translations
of one English gloss (`consensus::source_forms_from_cells`, `eval::build_input`). It then
groups those forms by a consonant-skeleton key and votes (`consensus::generate`,
six-subgroup vote). **~46% of remaining misses are "different root"** — the languages don't
all use the *same* root for a meaning, so the vote is polluted by **synonyms that are not
etymologically related**.

Example: gloss "beautiful", official `krasny`. The dictionary's cells are RU *krasivy*,
CS *krásný* (cognate, kras-), but PL *piękny*, SL *lep* (different roots entirely). The
`piękny`/`lep` forms are real Slavic words but **not cognate** with the winning root; feeding
them into the vote and the surface/representative selection corrupts the output.

**This is the biggest untapped lever, and it is exactly what you must fix first:** before
voting, decide which forms in a meaning group are *cognate with each other*, cluster them
into etymological families, choose the family (root), and build the form from that family
only — treating non-cognate synonyms as outliers, not votes.

## 2. Part A — Pipeline hygiene audit (do this before and alongside modeling)

Walk **every stage** and hunt for data that quietly ruins the result. For each, build a
diagnostic that dumps suspicious cases to a CSV/report under `target/eval/` so problems are
visible, then fix what you find and re-benchmark. Stages:

1. **Cell parsing** (`normalize::split_cell`, `official.rs`): Are `!`-coinage flags,
   parentheticals (`(anat.)`), and multi-value separators (`,` `;` `/`) handled? Are
   multi-word phrases, abbreviations, and empty/garbage cells filtered? Dump cells that
   produce zero or >4 variants, or non-alphabetic tokens.
2. **Transliteration** (`normalize::to_phonemic_latin`): Cyrillic↔Latin per language. Dump
   any form containing characters that survived unmapped. Spot-check round-trips. Verify
   language-specific rules (UA/BY г→h, Bulgarian ъ, Serbian/Macedonian specials).
3. **Normalization & the alignment key** (`orthography::consonant_key`,
   `ascii_skeleton`): Is the key **over-merging** distinct roots into one group, or
   **under-merging** true cognates? Measure both directions. Dump meaning groups whose
   winning key pools forms with high pairwise distance (a smell of over-merge) and groups
   that split cognates that a linguist would call one root (under-merge).
4. **Grouping & voting** (`consensus::generate`): Verify subgroup/branch counts, the
   population tie-break, and that a language can only vote once per group. Dump the vote
   tallies for a sample so they can be eyeballed.
5. **Proto linking** (`proto_link.rs`): **Measure link precision**, not just coverage
   (currently 17.5%). A wrong link (a synonym reconstruction) injects a wrong flavored form.
   Build a check: for confidently-linked meanings, how often does the linked reconstruction's
   descendants actually contain the meaning's own cognates? Dump low-precision links.
6. **Ranking/gate** (`pipeline.rs`): Confirm the confidence-gated override and dedup behave;
   dump cases where a wrong candidate outranks a right alternative.

Fix real bugs (they may be worth more than any model). Every fix is benchmarked.

## 3. Part B — Cognate cohesion & clustering (the core new capability)

Add a stage that, for each meaning group, quantifies cognacy and clusters forms into
etymological families, then feeds only the chosen family into the existing consensus/proto
pipeline. Techniques to implement, compare, and keep the best of:

- **Pairwise similarity/distance between forms.** Start with normalized edit distance on the
  aligned phonemic Latin, then move to **sound-correspondence-aware alignment** — a
  Needleman–Wunsch / Levenshtein with a *substitution-cost matrix that encodes regular Slavic
  correspondences* (e.g. cheap costs for *g/h, o/a pleophony, ě/e/i, nasal↔u, č/ć/c, t/ć).
  Consider the computational-historical-linguistics standards: **ALINE**, **SCA** (sound-class
  alignment), and **LexStat**-style scoring (List/LingPy). You can port the ideas without the
  library.
- **Cognacy graph as an oracle.** Where a Proto-Slavic reconstruction links (via
  `proto_link`), its **descendant tree** in `data/proto-slavic.cache.json` is ground-truth
  cognacy: two modern forms sharing a proto ancestor are cognate. Use this to label clusters
  and to validate the distance-based clustering.
- **Clustering.** Single-/average-linkage (UPGMA) or Infomap on the distance matrix; or a
  simple threshold on the correspondence-aware distance. Each meaning → one or more clusters.
- **Family selection = the real vote.** Vote over *clusters* (roots), not surface forms,
  using the six-subgroup / population machinery already in `consensus.rs`. Pick the family
  present in the most subgroups; break ties by population and by cohesion.
- **Outlier / borrowing removal.** Drop forms that don't join any cluster (isolated
  synonyms) and detect internationalisms/loans (a form matching the international shape, or an
  outlier by distance) so they don't distort inherited-root selection — but keep the existing
  internationalism path (`morph.rs` §5) for genuinely international meanings.
- **Cohesion → confidence.** A meaning whose forms cluster tightly across branches is a
  high-confidence reconstruction; a low-cohesion meaning (many roots) is inherently uncertain
  — thread this into `Confidence` and re-check calibration.

Expected payoff is largest on the ~46% "different root" bucket. Prove it on the benchmark.

## 4. Part C — Statistical consensus & calibration

- **Probabilistic / weighted voting.** Replace or augment the current hard vote with a
  scored model: each form's weight = subgroup balance × cohesion × (1 − outlier score).
  Compare Bayesian-flavored consensus vs the current deterministic vote.
- **Correspondence-informed reconstruction.** When choosing the *surface/representative*,
  use per-position majority across the aligned cognate set (a multiple-sequence alignment of
  the cluster) rather than picking one representative language — reconstruct the consensus
  segment-by-segment. This can subsume several V2/V3 repairs (nasal/jat/y recovery) with one
  principled mechanism.
- **Confidence calibration.** Build a reliability diagram; fit isotonic regression / Platt
  scaling on a **held-out** split so `Confidence` buckets are truly calibrated, and the site's
  "≈X% match" claims are accurate. Never fit on the test fold.

## 5. Part D — Comparative-linguistics depth

- **Regular sound correspondences as first-class data.** Derive (or hand-encode) the
  correspondence sets across branches and use them both in the distance matrix and in
  reconstruction. Optionally *learn* correspondence weights via PMI over aligned cognate
  pairs (unsupervised), then inspect them for sanity.
- **Borrowing vs inheritance.** Distinguish inherited cognates from loans/reshapings; loans
  break regular correspondences and should be down-weighted for inherited-root selection.
- **Proto-graph coverage.** Grow `proto_link` coverage (V3 §D): strip prefixes/suffixes to
  link a derived word to its **bare-root** reconstruction, then re-attach — many "different
  root" and unlinked cases are derivations of a root that *does* reconstruct.

## 6. Methodology — how to test without fooling yourself

- **Metrics to add** (in the reports): cognate-cluster purity (vs the proto oracle), meaning
  cohesion distribution, proto-link precision, and the existing exact/normalized/top-k/edit/
  POS/branch/calibration/regressions/improvements suite.
- **Ablation, always.** One coherent change per rung; attribute its delta; keep or revert.
- **Guard against overfitting the 16,300-entry benchmark.** Hold out a random split (fixed
  seed passed via `args`/config, since `Math.random` is unavailable in some contexts — use a
  deterministic split); tune thresholds on train, report on held-out. Watch for rules that
  help exact but hurt normalized or calibration.
- **Regression review.** Inspect `target/eval/regressions.csv` after every kept change;
  bound and explain new regressions.
- Add a **cognate-clustering unit-test set** (a dozen hand-labeled meaning groups: which
  forms are cognate) so clustering quality is testable independent of the full benchmark.

## 7. Acceptance criteria

- Full-benchmark **exact top-1 clearly beats 34.72%** (stretch: push normalized past 42–43%),
  driven primarily by the cognate-aware family selection reducing the "different root" bucket.
- **Calibration stays monotonic**; the confidence buckets are (ideally) numerically
  calibrated on held-out data.
- A **pipeline-audit report** enumerates the data-quality issues found and what was fixed,
  with before/after numbers for each fix.
- New metrics (cluster purity, cohesion, link precision) are reported; ablation ladder shows
  every kept/rejected technique's measured effect.
- `cargo fmt` clean, `cargo check` 0 warnings, `cargo test` green (existing + new tests),
  website still builds and serves.
- Spot-checks stay correct: `bog · duša · glåva · blågo · měsęc · język · pisati · brati ·
  dobry · noć · grad · voda · oko`, plus new hard synonym cases you add
  (e.g. "beautiful", "language", "city").

## 8. Pitfalls (learned the hard way — see V3 and VOTING_MACHINE_NOTES)

- **Leakage is the cardinal sin.** Cluster/link/reconstruct from evidence, never from `isv`.
- **Porting a reference's normalization wholesale regresses us** — the voting-machine tables
  were tuned to a different downstream (documented in `data/VOTING_MACHINE_NOTES.md`). Adapt
  ideas; measure everything.
- **Aggressive folding is safe *within* a true cognate set but dangerous across synonyms** —
  which is exactly why cognate clustering must come *before* the aggressive alignment key,
  not after.
- **Don't over-cluster or under-cluster.** Validate against the proto descendant oracle and
  the hand-labeled test set, not intuition.
- **Don't chase exact at the cost of calibration or normalized** — the primary metric is
  exact, but a change that wins exact while wrecking the other two is not a win.
- **Keep it reproducible and offline.** No network at build/eval time; deterministic splits;
  the benchmark must run from `data/official-isv.csv` + the proto cache alone.
```
