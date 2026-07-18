# Implementation prompt V3 — make the Proto-Slavic engine trustworthy, then attack the root problem

Continue improving `interslavic-wiktionary-lab`. The engine, benchmark, website, and the
two-stage Proto-Slavic path all exist and work (see `README.md`, `IMPROVEMENT_PROMPT_V2.md`).
This phase is driven by one measured finding from the V2 work — treat it as the thesis.

## Starting point (measure against these; do not re-derive)

Production config on the leakage-free benchmark (`cargo run --release -- evaluate`,
16,300 official single-word entries):

- exact top-1 **33.83%**, normalized top-1 **40.62%**, normalized top-3 **50.94%**, mean
  normalized edit **0.237**. (Original prototype baseline: 27.38% / 34.96%.)
- Confidence calibration is monotonic (high ≫ medium ≫ low). **Keep it monotonic.**
- Remaining-error mix (`target/eval/candidate-generation-report.md`): **~46% different
  root/derivation**, ~21% extra letter (epenthesis/ending), ~14% single-letter sub, ~12%
  missing letter, ~6.5% y/i, <1% flavored-letter-not-recovered.
- POS exact: noun ~43%, pron ~36%, verb ~28%, adj ~23%, adv ~19%, num ~9%.

## The thesis (why this phase exists)

When the V2 tense-yer + reflex-guided yer rules made the Proto-Slavic engine *more correct*,
the benchmark went slightly **down** (34.15% → 33.83%). The retired "length hack" scored
higher only because it *distrusted the reconstruction more* — it deferred to the reflexes
(consensus) whenever the proto form diverged, dodging the engine's **non-yer** errors
(endings, palatalizations, gender). The current ranking (`pipeline.rs::flavor_equivalent`
reflex-shape-agreement) does the same: **the reconstruction may only supply flavored
spelling; on any segmental disagreement the consensus wins.** That gate is a cap.

> **So the binding constraint is the Proto-Slavic engine's correctness on the segments it
> currently gets wrong. Make it trustworthy, then relax the gate — that is the path to beat
> both 33.83% and the retired 34.15%.** The ~46% divergent-root bucket is the other elephant
> and needs root-level modeling.

## Non-negotiable rules (unchanged)

1. **Keep only if it improves** the benchmark. Every change is an ablation rung in
   `eval.rs` (`kept_ladder` / `rejected_experiments`); regressions stay documented, not
   merged into `ConsensusConfig::production()`.
2. **No leakage.** Nothing on the benchmark path may read `OfficialEntry::isv`.
3. Primary metric: **exact top-1** (flavored target); keep reporting normalized / top-3 /
   edit / POS / calibration / regressions / improvements.
4. Keep the V2 wins: the tense-yer rule, reflex-guided yer vocalization, and the six-subgroup
   consensus. **Do not reintroduce the length hack.**

---

## A. Build a Proto-engine-only benchmark (tight iteration signal) — do this first

Right now the proto engine's accuracy is entangled with linking, ranking, and consensus, so
you can't tell whether a proto rule change helped. Add an isolated benchmark:

- For every benchmarkable official entry that gets a **confident** proto link
  (`proto_link::link`), run `proto::generate_with_reflexes(entry.word, …)` and compare its
  output **directly** to `official.isv` (exact + normalized), broken down **by POS** and,
  ideally, **by which rules fired** (from the `RuleStep` trace ids).
- Emit `target/eval/proto-engine-report.md` + a per-error CSV. Report link **coverage**
  (% of benchmark linked) and this proto-only accuracy.
- This is the signal you iterate §B against — it isn't muddied by consensus/ranking, and it
  tells you exactly which proto rule to fix next.

## B. Make the Proto-Slavic engine trustworthy (unlock the reflex-shape cap)

Fix the engine's segmental errors, ranked by likely impact. Each is a rung on the §A
proto-only benchmark; after a batch, re-run the **full** benchmark and try **relaxing the
`flavor_equivalent` gate** (let proto win some segmental disagreements) — that relaxation is
where the aggregate gain finally lands.

1. **Use the stem class you already extract but ignore.** `dump.rs` captures
   `ProtoEntry.stem_class` ("hard o-stem", "a-stem", "i-stem", "jo-stem", …) and it is
   **currently unused**. Thread it into `proto::endings` to pick the declension class,
   gender, and correct nominal ending deterministically (§3.1): o-stem masc → -Ø consonant,
   o-stem neut → -o, a-stem → -a, i-stem fem → -Ø, soft jo/ja-stem → -e/-a. This should move
   the noun bucket and fix many "extra/missing letter" ending errors.
2. **Palatalizations (§2 Phase H).** Add first palatalization (k/g/x → č/ž/š before a front
   vowel) in the contexts where it applies, and — critically — the **negative rule: no second
   palatalization in inflection** (`Čeh→Čehi`, not `Česi`). Getting this wrong systematically
   corrupts velar-stem paradigms (§6.2).
3. **Soft-stem + labials.** Write `pj/bj/vj/mj`, not East/South `plj/blj/…` (§6.2). Handle the
   soft consonants `ĺ ń ŕ ť ď ś ź` before a consonant vs. `lj/nj` before a vowel.
4. **Verb classes (§3.3).** Infinitive is always `-ti`; derive the present-stem class only for
   display: `-ovati→-uj-`, `-nǫti→-nųti`, `-iti/-ěti→-i-`, athematic. Verbs are the weakest
   inflected POS (~28%).
5. **Morphophonemic alternations (§3.4).** O⇒E after soft stems; Y⇒E in **nouns** but Y⇒I in
   **adjectives/pronouns**; Ě⇒I in fem dat/loc — three soft-stem rules with *divergent*
   targets; keying them off the wrong POS silently corrupts endings (§6.2).

## C. Root / semantic-family modeling — the ~46% bucket

Most far-misses are cases where Interslavic chose a **different root** than the plurality
skeleton. Implement §4.2 step 3:

- Cluster the meaning's modern cognates into **root families** using the Proto-Slavic
  **descendant graph** already in the cache (`ProtoEntry.descendants`): forms sharing a
  reconstructed ancestor are one root, so near-synonyms stop fragmenting the vote.
- Vote on **roots** (six subgroups, ½-votes on splits) rather than surface forms; pick the
  root present in the most subgroups with the most "average" meaning.
- This is exploratory and hard — gate it, expect modest gains, and keep the error analysis
  honest (many are editorial choices no algorithm reproduces).

## D. Expand link coverage and precision

- The cache has **5,424** reconstructions; report and grow the fraction of benchmark entries
  that link. Use the §A benchmark as ground truth to tune `proto_link::DEFAULT_THRESHOLD` and
  the signal weights.
- **Derived/prefixed words**: many misses are prefixed verbs / derived nouns whose *bare
  root* has a reconstruction but the derived form does not. Strip the prefix (`råz-`, `pri-`,
  `na-`, …) / suffix, link the root, derive it, and re-attach — reusing `morph::normalize_prefix`.
- Handle multi-sense proto pages (a `word` appears several times with different glosses/POS):
  pick per (root, POS) and let one reconstruction serve several meanings.

## E. Derivational morphology (§ word-formation / derivation)

Apply to **both** the consensus and proto paths, gated:

- Secondary-imperfective verb stems `-yva-/-iva-/-ava-` (seen throughout the verb tail).
- Agentive `-telj`/`-nik`, abstract `-ostь`/`-ьstvo`, diminutives.
- Compound connector `-o-` (`-e-` after a soft consonant): `voda+padati → vodopad`.

## F. Recalibrate and re-gate

- As the candidate mix shifts, re-fit `Confidence::from_score` thresholds so **calibration
  stays monotonic** (the report's calibration table must keep high > medium > low).
- After §B lands, sweep the `flavor_equivalent` relaxation and the proto score formula in
  `pipeline.rs` as explicit rungs — the whole point is that a *trustworthy* engine earns a
  looser gate.

## Acceptance criteria

- New `proto-engine` benchmark (command + `target/eval/proto-engine-report.md`) reports
  proto-only accuracy by POS and link coverage.
- Full-benchmark **exact top-1 beats 33.83%**, and the explicit stretch target is to **pass
  the retired 34.15%** by relaxing the reflex-shape gate once the engine is trustworthy —
  with **normalized not regressing** and **calibration still monotonic**.
- `regressions.csv` inspected; any new regressions attributed and bounded.
- Ablation ladder shows each §B/§C/§D/§E rule's measured delta; only accuracy-improving rungs
  enter `production()`.
- Website still renders the reconstruction + derivation; `serve` works, local CSS only.
- `cargo fmt`, `cargo check` (0 warnings), `cargo test` (existing 8 proto tests + new ones)
  all green.
- Spot-checks stay correct: `bog · duša · glåva · blågo · měsęc · język · pisati · brati ·
  dobry · noć · grad · voda · oko`, plus new ones you add for palatalization / stem-class
  (`Bog→Bože` vocative, `kniga`/`ruka` velar plurals, an i-stem like `kostь→kost`, an a-stem).

## Pitfalls (carried forward + new)

- **Leakage** is still the cardinal sin — link and derive from evidence, never from `isv`.
- **No second palatalization in inflection.** Only the vocative palatalizes velars in
  paradigms; over-applying it corrupts `-i`/`-e` plurals (§6.2).
- **Y⇒E (nouns) vs Y⇒I (adjectives) vs Ě⇒I (fem dat/loc)** — three soft-stem vowel rules with
  opposite targets; wrong POS keying silently breaks endings (§6.2).
- **Labials + j** → `pj/bj/vj/mj`, never `plj/blj/…` (§6.2).
- **`dž` provenance**: etymological `đ` (*dj) and loan `dž` both reduce to standard `dž`;
  keep them distinct on the flavored layer (§6.2).
- **Don't let proto win on a weak link.** The confidence gate + reflex-shape agreement exist
  because the engine is imperfect; loosen them *only* as far as the §A benchmark proves safe.
- **Keep the reflex-guided yer rule and the tense-yer rule; do not reintroduce the length
  hack.** If a change regresses, revert it and record why — a negative result is a result.
```
