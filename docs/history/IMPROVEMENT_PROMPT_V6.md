# Improvement brief V6 — accuracy, coverage, and discoverability

You are an expert in comparative Slavic linguistics, Rust, and information retrieval.
V5 closed the loan-adaptation gap (+1.44 pp exact). V6 has **three fronts**: keep raising
measured accuracy, grow honest coverage, and — new — fix **discoverability** at the
architectural level: a correct word that exists in the system but cannot be *found* is a
product failure the benchmark does not see.

## 1. Current state (read before touching anything)

Two generation paths, benchmarked separately:

- **Consensus pipeline** (`pipeline::generate`), the primary benchmark against 16,300
  official meanings: **38.42% exact / 45.50% normalized top-1** (baseline 27.52%).
  Miss classes: 47.0% wrong-cluster, 32.1% right-cluster-wrong-form, 20.8% root-absent.
- **Site path** (`corpus::generate_set`), ~22.4k words from the whole Wiktionary Slavic
  corpus: **56.9% exact / 61.1% normalized** on the 6.9k scorable entries.

The static site (`site/`) is built by `src/site.rs`: one page per generated word, plus
`search.json` and client-side search embedded in `index.html`.

Every prior audit finding stands: the low-hanging correctness bugs are gone; rejected
experiments (V5 report + §6 below) must not be re-attempted.

## 2. The motivating failure: `kråtky` (fix this class, not just this word)

`explain "short"` shows the system at its best *and* its discoverability at its worst:

- Candidate 1: **kråtȯky** (proto-derived from *kortъkъ, conf 0.98) — wrong: official is
  **kråtky**.
- Candidate 2: **kratky** (consensus, conf 0.967) — normalized-equal to the official word.

Three distinct defects, all general:

1. **Engine (yer resolution on adjectives).** The proto engine resolves yers on the
   *short* form `*kortъkъ` (medial `ъ` strong by Havlík → `ȯ`) and only then appends the
   adjectival `-y` (`proto.rs` runs `yers` before `endings`). But the Interslavic
   adjective continues the *definite* form `*kortъkъjь`: the tense `ъ` before `j` becomes
   the ending, the medial `ъ` is then **weak** and drops → `kråtky`. For adjectives the
   yer pass must run on the long form (append `ъjь` first, or re-rank yer strength given
   the ending). Find every adjective in the miss list with a spurious `ȯ`/`e` before the
   final consonant+`y` and measure the class (`slådȯky`-type predictions). This is
   right-cluster-wrong-form territory the engine fully controls.
2. **Search index (site.rs:113).** `search.json` rows carry **only `candidates.first()`**.
   `kratky` is rendered on the entry page under "Alternativne kandidaty" but is
   *unsearchable*. Index every ranked candidate (with its rank), not just the top form.
3. **Search matching (index.html `run()`).** Comparison is exact lowercase: no
   standard-alphabet or ASCII folding. Searching `kratoky` cannot find `kråtȯky`; nor can
   `kratky` even if it were indexed as `kråtky`. `ortho::to_standard` and
   `ortho::ascii_skeleton` already define the folds — precompute them into the index.

## 3. Rules of engagement

1. **Engine changes**: unchanged from V5 — measure with `cargo run --release -- evaluate`;
   keep only if exact top-1 does not regress; gate behind a `ConsensusConfig` flag with a
   ladder rung ending at `production()`; regression test per fix; revert honestly and
   record negatives.
2. **No leakage — clarified.** The *generator* must never read the `isv` answer. The
   *site*, however, is a product, not a benchmark: it MAY (and should, §5) display the
   official lemma as authoritative. Keep the boundary explicit in code: official data
   flows into rendering/search only, never into `pipeline::generate`, `consensus`,
   `proto_link`, or `corpus::generate_set` inputs.
3. **Site/search changes need their own acceptance tests** since the benchmark cannot see
   them. Add `#[test]`s (or a small fixture check in CI) asserting at minimum:
   searching an alternative candidate's form finds its entry; searching the ASCII fold of
   a flavored headword finds it; searching an official lemma that the system knows finds
   the entry that generates it.
4. **Prefer depth over volume.** One measured or testable structural fix beats ten edits.

## 4. Front A — discoverability & linking (architectural; do this first)

Design the index once, correctly. A recommended shape for each site entry:

1. **Index all surface keys per entry**: every ranked candidate form; the
   `to_standard` fold and `ascii_skeleton` fold of each; the official lemma when the
   entry is matched to one (§5); English gloss tokens (already partially there).
   Keep `search.json` compact: `[id, display_form, keys[], gloss, pos, status, conf]`
   with keys deduplicated; measure the payload (22k entries — stay under a few MB, or
   shard by first letter if needed).
2. **Fold the query** in `run()` with the same rules as the index keys (precompute the
   fold table in JS or ship folded keys only). Rank: exact flavored > exact folded >
   candidate-rank > prefix > substring > gloss.
3. **Alternatives become first-class**: an alternative hit must land the user on the
   entry with that candidate highlighted (anchor `#cand-2` or query param), not silently
   on the top form.
4. **Etymology/cognate cross-linking**: entries sharing a Proto-Slavic ancestor should
   link to each other (the ancestor is already in `Reconstruction`); render an ancestor
   page (or section) listing all its derivatives — the corpus already groups by ancestor
   in `corpus.rs` union-find, so this is a rendering join, not new inference. Cognate
   evidence rows already carry `source_url`; make the ancestor itself link to
   `en.wiktionary.org/wiki/Reconstruction:Proto-Slavic/…`.
5. **CLI parity**: `explain "kratky"` currently fails with "No official entry found" —
   `explain` should also match generated candidate forms (folded), not only official
   lemma / English gloss.

## 5. Front B — make the official dictionary authoritative on the site

The official dictionary (18,459 rows, `data/official-isv.csv`) is today used *only* to
score. The site should present it as truth and the generator as the transparent
derivation engine underneath:

1. **Match site entries to official meanings** (by the same meaning-alignment used in
   eval where applicable, else by gloss + folded-form agreement). Where matched:
   - the **official lemma is the headword**; the generated top-1 becomes the
     "reconstructed" line with its full trace;
   - status badge: *exact match* / *normalized match* / *differs (official shown,
     candidate N agrees)* / *differs (no candidate agrees)*. The `kråtky` page would read:
     headword **kråtky** (official), reconstruction kråtȯky, note that candidate 2
     matches normalized.
2. **Official-only entries**: official lemmas the corpus never generates should still get
   (searchable) pages, marked "official, not yet derivable from evidence" — this is the
   root-absent 20.8% and makes the site a complete dictionary rather than a sample.
3. **Novel words** (generated, no official counterpart — the current `novel-words` list)
   keep their existing "unofficial reconstruction" framing, clearly badged.
4. All of the above is **display-layer**: assert with a test that benchmark numbers are
   byte-identical before/after (no generation code path may observe the merge).

## 6. Front C — engine accuracy (ranked hypotheses)

1. **Adjective definite-form yers** (§2.1) — the one concrete new engine bug class this
   brief hands you. Measure the bucket first (grep the audit CSV for adjectives whose
   prediction has `ȯ`/`e` immediately before `C+y` where official lacks it).
2. **Correspondence-aware cluster distance** (V5 leftover, still the biggest pool:
   wrong-cluster = 4,178 misses). Hand-encoded correspondence costs (g↔h, pleophony
   o↔oro, jat e↔i↔ije, nasal a↔ę/u↔ą, sibilant classes) as a weighted distance for
   cluster merging/voting instead of binary `consonant_key` equality. Validate cluster
   purity against the proto oracle before keeping; beware over-merging. Expect most of
   wrong-cluster to remain (editorial choices — ceiling ~45–48% exact), but the
   *near-miss* slice (same root, split by one correspondence) is real.
3. **Verb tail**: 592 right-cluster verb misses, mostly conjugation-class endings
   (`-iti/-ěti/-ati` choice) and aspect morphology. The `-ěti` statives (`kameněti`
   mispredicted `kameniti`) look rule-recoverable from ru `-еть`/cs `-ět`.
4. **Compound numerals** are near-broken (10% exact) — `pęt na desęte` vs official
   `pętnadsęť`; a contained rewrite of the numeral assembly.
5. **Remaining form tail** is documented in the V5 session: each pattern ≤20 entries.
   Only chase these opportunistically.

## 7. Front D — coverage (more words, honestly)

1. `dump.rs` drops multi-word lemmas (`word.contains(' ')`) and prefixed reconstructions.
   Reflexives and fixed verb+particle idioms are a measurable coverage slice.
2. **Derived-family expansion**: for each corpus cognate set, Wiktionary carries derived
   terms (diminutives, agent nouns, abstracts). Deriving `-ka/-nik/-ost/-stvo/-teljstvo`
   family members from an already-derived base (with the existing suffix tables) can grow
   the site's vocabulary at high plausibility — badge them as derived, not attested.
3. Do NOT grow the proto cache expecting pipeline gains (V4/V5: the linker, not the
   cache, is the constraint).

## 8. Do NOT re-attempt (measured negative, cumulative across V4/V5)

- Skeleton-level fallback key for explicit etymology (−0.03 pp); single-language explicit
  ancestor even with similarity gating (−0.16 pp / neutral at 0.9).
- Per-aligned-column majority vote for internationalisms (+0.02 exact, −0.30 normalized).
- Final `-l`→`-lj` from Russian soft sign (official usage inconsistent: model/festival).
- Czech-y as a Greek-upsilon signal (official writes `analitičny`).
- Flavor recovery from modern reflexes (jat/palatals/y-recovery), adj-longform
  representative, internationalism-preference ungating, blanket string replaces,
  learned/neural anything.

## 9. What to produce

Per engine change: hypothesis + linguistic justification (`RULE_SPEC.md` cite,
`file:line`), measured delta, regression test, repro. Per site/search change: the
acceptance tests of §3.3 plus a before/after demonstration (e.g. the `kratky` search).
Finish with a ranked summary: kept (deltas), reverted (deltas), and the most promising
lever not reached.

## 10. Setup

```
cargo build --release && cargo test --release
cargo run --release -- evaluate      # ablation ladder + Headline (primary benchmark)
cargo run --release -- corpus-eval   # site path accuracy
cargo run --release -- audit         # miss buckets → target/eval/audit-misses.csv
cargo run --release -- explain "<word|gloss>"
cargo run --release -- export        # build the static site into site/ (corpus-based
                                     # when the lemma cache exists, else official-seeded)
```
Caches are committed; the 22 GB dump is not needed. Work on a branch; keep `cargo fmt`
clean; commit `target/eval` artifacts as the repo does.
