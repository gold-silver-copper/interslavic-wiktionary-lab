# Improvement brief V7 — full-pipeline review, then close the gap to the official dictionary

You are an expert in comparative Slavic linguistics, Rust, and evaluation methodology.
Six briefs of targeted work have taken the engine from 27.5% to **39.23% exact / 46.44%
normalized top-1** against the official Interslavic dictionary. The cheap targeted wins
are getting smaller (V6b netted +0.11 pp across three rules). V7 is different in kind:
**review every stage of the pipeline systematically**, attribute the remaining error to
stages with evidence, and spend effort where the attribution says the headroom is —
not where a pattern happens to be visible in a CSV.

## 1. Current state (verify these numbers first — they must reproduce)

- Pipeline (`evaluate`): **39.23% exact / 46.44% normalized** top-1, 16,300 meanings.
- Miss classes (`audit`): wrong-cluster 47.6% | right-cluster-wrong-form 31.2% |
  root-absent 21.2% (~8.7k normalized misses).
- Proto engine (`proto-eval`): 18.0% link coverage; 46.26% exact on linked.
- Site path (`corpus-eval`): 57.92% exact / 62.23% normalized on 6.9k scorable.
- Honest ceiling estimate (from the editorial-root audit): **~45–48% exact** for the
  pipeline. Parity goal for this brief: land measurably above 41% without a single
  regression, and raise the proto engine's on-linked accuracy above 50% exact.
- 49 tests green; every kept rule sits on an ablation-ladder rung ending at
  `ConsensusConfig::production()`.

## 2. Rules of engagement (unchanged core + two new instruments)

1. **Measure everything** (`evaluate` before/after; exact top-1 is the gate; report
   deltas). Gate consensus rules behind `ConsensusConfig` flags with ladder rungs;
   proto.rs changes are measured by `proto-eval` AND the full ladder. Ship a regression
   test per fix. Revert honestly and record the negative delta.
2. **No leakage in generation** — cognates + POS/gender/genesis only, never `isv`.
3. **NEW — stage-attribution harness (build this first).** Every candidate carries a
   `RuleStep` trace. For each miss, find the **last pipeline stage whose output still
   folds to the official form** (or the first stage that destroyed it) and bucket misses
   by stage: normalization → clustering/vote → representative pick → repair X → proto
   link → proto rule Y → endings → merge/rank. This converts "1-letter substitution"
   buckets into *per-stage blame* and is the map for everything below. Write it into
   `audit` (a `stage` column in audit-misses.csv) and report the histogram.
4. **NEW — oracle ladder (diagnostic only, never shipped).** For each stage, measure the
   upper bound from making it perfect while everything downstream stays real:
   (a) oracle cluster choice — force the cluster whose key matches the official key;
   (b) oracle proto link — force the reconstruction whose derived form is closest;
   (c) oracle representative — pick the group member whose folded form is closest.
   These *require reading the answer*, so they live strictly behind a
   `--diagnostic-oracle` eval path that can never feed production, clearly labeled.
   The result is a table: stage → headroom in pp. Spend effort top-down by headroom.

## 3. The pipeline, stage by stage — review checklist

Walk each stage in order. For each: read the code completely, list the assumptions it
makes, test the assumptions against 20+ real traces (`explain`), and only then patch.

### Stage 0 — extraction (`dump.rs`, committed caches)
- Are etymology templates parsed completely (`inh`/`der`/`bor`/`lbor` variants, nested
  templates)? Every parse miss is lost link coverage downstream.
- Multi-word lemmas and reflexives are dropped (`word.contains(' ')`) — quantify what
  that costs the site path before deciding to support them.

### Stage 1 — normalization (`normalize.rs`) ⚠ highest-suspicion stage
- **Known bug-class, diagnosed in V6b and unfixed**: Cyrillic iotation/softness is lost —
  ru `блюдо` → `bludo` (should carry the palatalization ISV writes as `lj`: `bljudo`),
  soft-sign softness `ль` → plain `l`. The V5 `-lj` miss bucket (`akvarelj`, `aprilj`)
  and the `bljudo/sablja` family are all this one defect. Fixing it changes skeletons
  and therefore *clustering everywhere* — do it early, rerun the full ladder AND the
  audit diff, and expect to re-tune downstream repairs.
- Audit every per-language table against a checklist of ISV-relevant contrasts:
  softness (ль/нь → lj/nj), iotated vowels (ю/я after consonant vs word-initial),
  Polish ó/rz/ł, Czech ě/ů, Bulgarian щ, Ukrainian и/і, Belarusian ў. For each contrast:
  does the phonemic Latin preserve exactly the information ISV spelling needs?
- Every normalization change needs a table-driven test (`tr("ru","блюдо") == "bljudo"`).

### Stage 2 — folds (`orthography.rs`)
- `ascii_skeleton` (keeps vowels) vs `consonant_key` (drops them): verify each fold
  decision against pairs that must merge (pleophony, *g→h) and pairs that must NOT
  (s/z — proven contrastive in V6b). If Stage 1 starts emitting `lj/nj`, decide how the
  folds treat them (probably fold to `l/n` for voting, keep for surface).

### Stage 3 — clustering & vote (`consensus.rs` grouping, six-subgroup, intl preference)
- Wrong-cluster is 47.6% of misses but most is editorial (minority-root choices — the
  known ceiling). The oracle-cluster run tells you the true recoverable slice; V6b
  measured that *merging* clusters by correspondence always loses (four variants, all
  negative — see §5). The unexplored lever is **root-level, not surface-level, voting**:
  score candidate roots across the proto descendant graph (meanings whose clusters are
  linked to the same reconstruction vote together even when their surface keys differ).
- Check the vote's tie-breaks and the intl-preference bonus against traces; check that
  secondary translations never outvote primaries.

### Stage 4 — representative pick + repairs (`reconstruct` + repair family)
- Repair order matters and has never been reviewed as a whole: depleophony → palatal
  (off) → nasal → adj-fleeting → endings/intl tables → loan-stem → verb-class →
  voicing. Look for order bugs with traces (a repair whose precondition another repair
  destroyed) and for repairs that should be corroborated but aren't.
- The V6b lesson generalizes: **when a cluster wins with a bad national surface, repair
  the surface; don't re-vote.** Inventory what the representative still leaks
  (per-stage-attribution will show it): candidates include SC ijekavian `ije`,
  Bulgarian-only vocalism, Polish nasal spelling in non-nasal contexts.

### Stage 5 — proto link (`proto_link.rs`)
- Coverage is 18%; the site path proves a known ancestor is worth ~18 pp. Rejected
  already: skeleton fallback key, single-language link (both measured negative).
  Remaining ideas with a real chance: (a) link via the *corpus cognate set* (align the
  meaning's cognates to a set by member overlap, inherit its ancestor — the set was
  built from explicit etymology, so precision is high); (b) POS-aware gloss expansion
  (the gloss-token overlap is naive — synonyms/inflected glosses miss).
- Confidence calibration: thresholds (0.42, 0.62) were tuned once; re-check them after
  any coverage change with a sweep, on a seeded split to avoid re-tuning on the test.

### Stage 6 — proto rule engine (`proto.rs`) — proven to still hide structural bugs
- V6's definite-form fix was worth +0.70 pp and was found by *reading one trace
  carefully*. The engine is an ordered rule cascade — review each rule's ordering
  assumptions the same way: palatals before metathesis? nasals before prothesis?
  yers assume endings run after — verify with the nominal paradigms too (neuter -o/-e
  choice, feminine -a vs -ь stems, masculine animacy).
- proto-eval's per-rule error table (`proto-engine-report.md`) is the worklist:
  read the top 30 wrong derivations, classify by rule, fix the biggest class, repeat.
- The reflex-retention vote (`reflex_vowel_vote`) uses naive consonant-index alignment —
  check it against pleophony and prefixed words where the index drifts.

### Stage 7 — endings & lemma morphology (`morph.rs`)
- The intl ending table and POS endings are mature; the review question is *coverage of
  citation-form variance*: Bulgarian/Macedonian no-infinitive handling, Slovene dual
  citation quirks, aspect-pair selection (ISV cites the imperfective more often —
  quantify against `pf./ipf.` metadata, which is legal input).

### Stage 8 — merge, dedupe, ranking (`pipeline.rs`)
- `flavor_equivalent` and the `demote` gate decide who wins when proto and consensus
  disagree. The 0.62 confidence gate and adjective exemption were tuned before V6
  changed the adjective engine — re-sweep them.
- Check `dedupe`'s flavored-upgrade path with traces: does a correct consensus form
  ever get *replaced* by a wrong proto spelling of the same standard form?

### Stage 9 — the eval itself (`eval.rs`)
- Verify the audit classifier against 30 hand-checked misses (is "wrong-cluster" ever
  actually a form error whose key drifted?). The stage-attribution harness (§2.3)
  subsumes and replaces the current three-way classification.
- Per-POS and per-genesis breakdowns should be first-class in the report — verbs,
  numerals and `genesis=I` rows have distinct failure profiles and distinct fixes.

## 4. Parity with the official dictionary (product goal, display layer)

Generation aside, "parity" means: every official lemma findable and correctly related
to the generated evidence. The site already shows official headwords, official-only
pages, and family cross-links. Remaining parity gaps to close (display-only, benchmark
byte-identical before/after):
- Multi-word official lemmas (`pęt na desęte`, reflexive `… sę`) have no pages.
- The official gloss should be searchable on official-headword entries (it already is
  on official-only pages).
- Family cross-links don't yet include official-only pages (an official lemma whose
  stem matches a generated family should appear in that family's list).

## 5. Do NOT re-attempt (every item measured negative; cumulative V4→V6b)

- **Cluster merging by correspondence-folded keys** — four variants, all negative:
  global s→z (−0.20), long-keys-only (−0.15), assimilation-position-only (−0.08),
  corroborated mid-word s→z repair (−0.09). Repair the winning surface instead.
- Skeleton-level fallback for explicit etymology (−0.03); single-language explicit
  ancestor even at sim ≥ 0.75/0.9 (−0.16/neutral).
- Per-aligned-column majority vote for internationalisms (+0.02 exact, −0.30 norm).
- West-corroborated stative `-ěti` (Czech iterative `-ět` false-positives; East-only kept).
- Final `-l`→`-lj` from Russian soft sign at the *repair* level (official usage split
  model/festival vs aprilj — but note: the Stage-1 iotation fix may resolve this class
  correctly at the source, which is allowed).
- Czech `y` as Greek-upsilon signal; flavor recovery from modern reflexes (jat/palatal/
  y-recovery); adj-longform representative; intl-preference ungating; blanket
  whole-string replaces; growing the proto cache for coverage; learned/neural anything.

## 6. Deliverables

1. The **stage-attribution histogram** (before any fixes) and the **oracle-ladder
   table** — these two artifacts are the review; commit them to the report.
2. Fixes in priority order of measured headroom, each with: hypothesis + linguistic
   justification (`RULE_SPEC.md` cite, `file:line`), measured delta (ladder + proto-eval
   + corpus-eval where relevant), regression test, `explain` repro.
3. A ranked closing summary: kept (deltas), tried-and-reverted (deltas), the
   stage-attribution histogram after your changes (did the blame actually move?), and
   the single biggest remaining lever with its oracle-measured headroom.
4. Updated README numbers; `cargo fmt` clean; ladder ends at `production()`.

## 7. Setup

```
cargo build --release && cargo test --release
cargo run --release -- evaluate      # ablation ladder + Headline (primary gate)
cargo run --release -- proto-eval    # proto engine in isolation (+ per-rule report)
cargo run --release -- corpus-eval   # site path accuracy
cargo run --release -- audit         # miss buckets → target/eval/audit-misses.csv
cargo run --release -- explain "<word|gloss>"   # full trace (folded matching works)
cargo run --release -- export        # static site into site/
```
Caches are committed; the 22 GB dump is not needed. Work on a branch. The audit CSV cap
is 500 rows — you may raise it locally for analysis, but restore it before committing.
