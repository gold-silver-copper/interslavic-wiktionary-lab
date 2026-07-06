# Improvement brief V5 — deepen the Interslavic generation engine

You are an expert in comparative Slavic linguistics and Rust. Your job is to read the
engine in this repository, understand its logic end-to-end, and find changes that make
its **measured accuracy** higher — against the official Interslavic dictionary — without
regressing it. This is not a greenfield task: the engine is mature and heavily tested.
Your edge is depth, not breadth.

## 1. What this project is (read before touching anything)

The repo generates **Interslavic (Medžuslovjansky)** words from evidence and scores them
against the official dictionary. There are two generation paths, benchmarked separately:

- **The consensus pipeline** (`pipeline::generate`) — the primary benchmark. For each
  official meaning it sees only the modern-Slavic cognates + the row's POS/gender/`genesis`
  metadata (never the `isv` answer), votes on a branch-balanced consonant-skeleton cluster
  to pick the root, then derives the flavored form from a linked Proto-Slavic reconstruction.
  **Current: 36.98% exact / 44.04% normalized top-1** (baseline prototype was 27.4%).
- **The site path** (`corpus::generate_set`) — builds a cognate-set dictionary from the
  *entire* Wiktionary Slavic-lemma corpus (25k inherited + 22k borrowed lemmas → ~22.4k
  words), deriving each from its **known** Wiktionary ancestor. **Current: 55.3% exact /
  59.4% normalized** on the ~6.9k scorable entries (`corpus-eval`).

The 18-point gap between them is the central fact: **when the ancestor is known, accuracy
is far higher.** The pipeline was recently improved by feeding it Wiktionary's explicit
`(lang → ancestor)` etymology (+2.0 pp exact); there is almost certainly more there.

Every rule has already been through an **adversarial triple-check audit** (a finder + two
independent verifiers), so the low-hanging correctness bugs are gone. Do not expect to win
by finding typos; win by finding *structural* improvements or genuinely subtle rule errors.

## 2. Rules of engagement (non-negotiable — violating these wastes everyone's time)

1. **Measure every change.** `cargo run --release -- evaluate` prints the ablation ladder
   and the `Headline`. A change is kept **only if exact top-1 does not regress** (the
   primary metric; normalized/top-3 are secondary). Run it before and after. Report the delta.
2. **Gate new rules behind a `ConsensusConfig` flag** and add a ladder rung, so the effect
   is attributable in isolation. The ladder MUST end at `ConsensusConfig::production()`
   (a test enforces this; the CI floor reads the last rung, not the best).
3. **No leakage.** Generation may read cognates + POS/gender/`genesis`, never the `isv`
   form. If you add a signal, prove it doesn't peek at the answer.
4. **Ship a regression test with every fix** (`#[test]`), asserting the specific case.
   `cargo test` must stay green (41 tests today).
5. **Revert honestly.** If a linguistically-motivated change regresses the benchmark, revert
   it and record it as a negative result. The benchmark is the arbiter, not intuition.
6. **Prefer depth over volume.** One measured +0.5 pp change with a test and a clear
   linguistic justification beats ten speculative edits.

## 3. The codebase map (where the logic lives)

```
proto.rs        Proto-Slavic → Interslavic ordered rule engine (clean, palatals,
                liquid_metathesis, nasals, prothesis, soft_consonants, syllabic_liquid,
                simplify_clusters, yers incl. Havlík + reflex-guided vocalization,
                endings, finalize). The FORM comes from here. Has tests.
consensus.rs    branch/six-subgroup vote, representative selection, gated repairs
                (nasal/depleophony/jat/palatal), reflexive detection, is_international_form.
                Picks the ROOT.
proto_link.rs   meaning → reconstruction: link_explicit (Wiktionary etymology, precise) then
                link (fuzzy descendant+gloss). Confidence gates the proto override.
pipeline.rs     the two-stage merge + the proto-override gate (flavor_equivalent, demote),
                dedupe, reflexive `sę` append.
corpus.rs       cognate-set building (union-find over Slavic skeleton + Latin etymon),
                generate_set, coverage-based confidence. Drives the site.
dump.rs         extraction from the 22 GB Wiktextract dump; ProtoIndex (+ the explicit
                etymology map); lemma corpus.
normalize.rs    per-language script → phonemic Latin (Cyrillic + Latin). orthography.rs:
                folding, ascii_skeleton (keeps vowels) vs consonant_key (drops them).
morph.rs        POS lemma endings + internationalism ending table (gated au/eu/th).
eval.rs         the benchmark, ablation ladder, audit, proto-eval, corpus-eval.
```
Reference truth: `data/RULE_SPEC.md` (authoritative Proto-Slavic → ISV rules).
Run `explain "<word>"` to see the full trace for any gloss/lemma.

## 4. High-value hypotheses to investigate (ranked by expected payoff, with the evidence)

Test these; keep what measures. The first two are where the real gains are.

1. **Close the pipeline↔site gap (highest ROI).** The site's 55% vs the pipeline's 37%
   proves that a *known* ancestor is worth ~18 pp. `link_explicit` already helps, but its
   coverage is limited to `(lang, exact-latin)` matches present in the lemma corpus.
   Investigate: (a) fuzzier key matching (skeleton-level, prefix-stripped) so more meanings
   hit an explicit ancestor; (b) aligning each official meaning to a *corpus cognate set* by
   cognate overlap and inheriting that set's ancestor; (c) when the explicit ancestor is
   found, does the proto-override gate actually let its form win? Audit `pipeline.rs`'s
   `demote`/`flavor_equivalent` logic on explicit links specifically.

2. **The `right-cluster-wrong-form` bucket (~35% of misses).** These are meanings where the
   root is right but the reconstructed FORM is wrong — i.e. the proto engine or the
   consensus isvization erred. Run `audit`, read `target/eval/audit-misses.csv`, and
   categorize this bucket: which `proto.rs` rules still misfire? (jat quality, yer
   retention, palatal outcomes, nasal front/back, syllabic liquids). Each recovered
   sub-class is measurable. This is the bucket most under the engine's control.

3. **Correspondence-aware cluster distance.** Cluster selection uses a binary
   `consonant_key`. A cheap **hand-encoded correspondence cost** (g↔h, pleophony o↔oro,
   jat e↔i↔ie, nasal quality, the sibilant classes — essentially ALINE/SCA-lite) as a
   weighted distance could fix `wrong-cluster` near-misses without a learned model. Validate
   cluster purity against the proto oracle before keeping. Beware over-merging.

4. **Verbs / adjectives / numerals lag nouns** (nouns 44%, verbs 29%, adj 30%, num 10%
   exact). Targeted rule work: verb infinitive/aspect derivation, adjective soft-vs-hard
   endings, and the compound numerals (`pęt na desęte`) that are near-broken. Each is a
   contained, measurable win.

5. **Nasal quality (partial).** `nasal_from_polish` now decides front/back from the
   representative's reflex vowel; a nasal-preserving source (OCS `cu`, or the linked proto
   `*ę`/`*ǫ`) would be more reliable. Measure whether consulting `cu`/proto improves it.

6. **Confidence calibration on a held-out split.** Every threshold was tuned on the full
   benchmark. A seeded train/test split with isotonic/Platt calibration would make the
   published confidences honest (does not raise top-1 — a methodology fix; state it as such).

7. **Statistical consensus** — replace the ad-hoc repair toggles with a principled
   per-aligned-column majority (MSA) over a correspondence matrix. Higher-risk; head-to-head
   ablation against the current representative-pick before adopting.

## 5. The honest ceiling — do NOT spend effort here

The audit and repeated experiments have *ruled these out*; re-attempting them will only
burn time:

- **The editorial-root bucket** (a large share of `wrong-cluster`): the ISV committee often
  chose a *minority* root (`krasny` over the more-widespread `lep`). No leakage-free feature
  reproduces an editorial choice. Realistic pipeline ceiling is ~45–48% exact.
- **Reconstructing flavored letters (jat, ć/đ, *y) from *modern reflexes*** — every attempt
  regressed. Flavor must come from the Proto-Slavic derivation, not the reflexes.
- **Blanket rules / whole-string replaces** — they corrupt more than they fix (the audit
  found many). Any new orthographic rule must be gated and boundary-aware.
- **Growing the proto cache to fix coverage** — the linker, not the cache, is the constraint.
- **Ungating the internationalism preference, adj-longform representative, y-recovery,
  re-porting the voting-machine tables** — all measured negative, documented in the report's
  *rejected experiments*.
- **A learned/neural cognate detector or reconstructor** — cognacy is largely GIVEN by the
  gloss + Wiktionary etymology; Slavic correspondences are a known closed set best
  hand-encoded; and 16k entries can't train one without leakage or overfitting.

## 6. What to produce

For each change you propose or make:
- The **hypothesis** and its linguistic/logical justification (cite `RULE_SPEC.md` or a
  Slavic-phonology fact, and `file:line`).
- The **measured delta** (exact top-1 before → after; and normalized/top-3/proto-eval where
  relevant). If it regressed, say so and revert.
- The **regression test** you added.
- Concrete **repro** for any bug (`explain "<word>"` output, actual vs expected).

Finish with a short ranked summary: what you kept (with deltas), what you tried and reverted
(with deltas, so it isn't re-attempted), and the single most promising lever you did NOT get
to. Do not inflate — a small, honest, measured gain is the goal.

## 7. Setup

```
cargo build --release
cargo test --release
cargo run --release -- evaluate         # ablation ladder + Headline (the benchmark)
cargo run --release -- proto-eval       # the proto engine in isolation
cargo run --release -- corpus-eval      # the site path's own accuracy
cargo run --release -- audit            # miss-bucket classification → target/eval/audit-misses.csv
cargo run --release -- explain "<word>" # full trace for one gloss/lemma
```
The Proto-Slavic cache (`data/proto-slavic.cache.json`) and the lemma corpus
(`data/slavic-lemmas.cache.json`) are committed, so you do **not** need the 22 GB dump to
run any benchmark. Work on a branch; keep `cargo fmt` clean.
