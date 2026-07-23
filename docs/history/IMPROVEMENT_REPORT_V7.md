# V7 report — full-pipeline review and the measured close of the gap

Response to [`docs/history/IMPROVEMENT_PROMPT_V7.md`](docs/history/IMPROVEMENT_PROMPT_V7.md). The V7 brief asked for a *systematic,
per-stage* review with two new diagnostic instruments built first, then fixes
spent where the attribution says the headroom actually is — not where a pattern
happens to be visible in a CSV.

## 0. Reproduced baseline (V7 §1)

All numbers reproduced before any change:

| Benchmark | Baseline | Brief's stated |
|---|---|---|
| `evaluate` exact / norm top-1 | **39.23% / 46.44%** | 39.23 / 46.44 ✓ |
| `audit` miss classes | wrong-cluster 47.6% / rcwf 31.2% / root-absent 21.2% | ✓ |
| `proto-eval` coverage / on-linked exact | 18.0% / 46.26% | 18.0% / 46.26% ✓ |
| `corpus-eval` | 57.9% / 62.2% | 57.92 / 62.23 ✓ |

## 1. The two diagnostic instruments (V7 §2.3, §2.4) — *the review*

### 1a. Stage-attribution harness (`audit`, new `stage`/`stage_detail` columns + histogram)

Every candidate already carries a `RuleStep` trace. For each normalized miss the
harness replays the winning candidate's trace, folding each intermediate form to
the standard alphabet, and names the **last stage whose output still folded to
the official form** (or the first that destroyed it). It writes a `stage` +
`stage_detail` column into `audit-misses.csv` and the full histogram to
`target/eval/stage-attribution.md`. Baseline histogram (8,730 misses):

| Stage | misses | share | dominant cause |
|---|---:|---:|---|
| 3-cluster/vote (wrong root chosen) | 2,701 | 30.9% | editorial minority-root |
| **8-merge-rank** (correct primary demoted) | 1,808 | 20.7% | **1,584 diff-root editorial / 224 same-root surface** |
| 0-root-absent (evidence gap) | 1,779 | 20.4% | unfixable |
| 1-normalize/representative | 1,563 | 17.9% | residual length 696 / y-i 473 / subst 383 / flavor 35 |
| 7-endings | 636 | 7.3% | ending residual |
| 6-proto-rule | 141 | 1.6% | yers 85 / residual 23 / endings 15 |
| 4-repair | 17 | 0.2% | loan-epenthesis, loan-ok |

**What this changed vs. the old three-way audit.** The old classifier called 47.6%
of misses "wrong-cluster." The harness shows that only ~31% is genuine
cluster-choice; another ~21% is *merge/rank* (a correct candidate was generated
but demoted), and of *that* only **224 (2.6% of all misses)** are a genuine
same-cluster ranking bug — the other 1,584 are the official picking a synonym we
rank as a valid but non-top primary (editorial word choice). The proto **rule
engine** — the place five briefs kept hunting — owns only **1.6%** of misses.
The blame does **not** live where the CSV's "1-letter substitution" bucket
implied.

### 1b. Oracle ladder (`oracle` subcommand — diagnostic only, reads the answer, can never feed production)

Each row makes ONE stage perfect while everything downstream stays the real
engine. Structurally isolated behind `consensus::Oracle` + `*_oracle` wrappers;
production call sites pass `None`.

| Stage oracle | exact | Δ exact | interpretation |
|---|---:|---:|---|
| baseline (production) | 39.92% | — | (post-fix baseline) |
| oracle-cluster | 43.83% | **+3.91pp** | mostly editorial — unrecoverable without the answer |
| oracle-representative | 43.59% | **+3.67pp** | within-cluster surface choice — the real lever |
| oracle-proto-link | 42.47% | **+2.55pp** | coverage headroom, *only when the link declines to override bad derivations* |
| oracle-all | 50.60% | **+10.68pp** | approximate ceiling below word-selection |

**Methodological finding.** A *naive* oracle-proto-link (force the closest-deriving
reconstruction and always trust it) measured **−7.28pp** — maximizing coverage
regresses, because the proto engine's derivation is imperfect and forcing it to
override a good consensus form hurts. Gating the oracle to link only when the
derived form is genuinely close to the answer flips it to **+2.55pp**. This is
the honest ceiling of the linking *decision*, and it explains why the previously
rejected coverage experiments were negative: coverage is not free headroom.

## 2. Fixes shipped, ranked by measured Δ exact (each on the full ladder + regression test + `explain` repro)

Cumulative **39.23 → 39.92% exact (+0.69pp)**, **46.44 → 47.09% norm (+0.65pp)**;
proto-eval coverage **18.0 → 20.1%**, on-linked **46.26 → 46.68% exact / 49.36 →
52.74% norm** (norm now clears the §1 target of 50%).

| # | Fix | Stage | Δ exact | Justification / repro |
|---|---|---|---:|---|
| G | **Descendant transliteration** — index & score `desc_membership` on `to_phonemic_latin`-transliterated descendants (54% of the cache is Cyrillic and never matched the Latin cognate skeletons) | 0/5 | **+0.25** | `dump.rs` build + `proto_link.rs` score; coverage 18→20.1%. Found by the Stage-0 reviewer. |
| — | **Reflex cognate-filter + yer corroboration** — feed the yer resolver only reflexes that share the reconstruction's consonant root, and require ≥2 reflexes to retain a weak yer | 6 | **+0.12** | `explain babka` → `babaka`→`babka`; a `star-` synonym in the "old woman" cell no longer injects a vowel. |
| B+C | **Proto `*-ьje→-je`** (kopje, not kopije) and **`*a-→ja-` prothesis** (javor, jagoda) | 6 | **+0.10** | 0 native `-ije` / 0 native bare `a-` lemmas → near-lossless. |
| E | **Feminine `-ij[oei]→-ija`, `-ike→-ika`** nom.sg fold (fizioterapijo→fizioterapija) | 7 | **+0.09** | 668 `-ija` lemmas vs ~0 singular `-ijo`; Slovene/CS oblique reps. |
| A | **Agentive `-tel→-telj`** with a closed 4-word skip list (hotel/kotel/kostel/dětel) | 7 | **+0.06** | 122 `-telj` vs 4 hard `-tel` lemmas. izbiratel→izbiratelj. |
| — | **Qualitative-adjective definite `-y`** — a root ending `-ov/-in` is only possessive when the reflexes are short (novъ→novy, materinъ stays short) | 6 | **+0.05** | The possessive string-check false-fired on nov/gotov/zdrav. |
| F | **Soft-stem adjective `-y→-i`** for hushing stems (staršy→starši) | 7 | **+0.02** | 60 `-ši`/72 `-ji` vs 0 `-šy/-žy/-čy/-jy`. |

## 3. Tried and rejected (measured or review-blocked, recorded honestly)

- **Stage-1 Cyrillic iotation/softness** (the brief's ⚠ highest-suspicion item):
  **not shipped**. The Stage-1 reviewer proved it is architecturally blocked —
  `REP_PRIORITY` is South/West-first so an East-Slavic soft form is never the
  representative, and the vote's `consonant_key` already drops `j`. The
  soft-sonorant words that *do* fail (konj/solj/polje/morje) already pass via the
  proto path; the ones that don't (akvarelj internationalisms) fail in
  representative-selection, not normalization. A blanket `ь→j` after `l/n/r` also
  regresses `caŕ` (soft `r`=`ŕ` not `rj`) and the 17 bare `-l` loans
  (festival/model). Net ≈ 0 flips, slightly negative — the flagged bug was a red
  herring, and the review redirected the effort to the agentive `-telj`
  morphology (Fix A) where the real losses live.
- **labial `+ľ→+j` proto rule** (bljudo/sablja): the split is lexical, not
  mechanical — 355 official `labial+j` (epenthetic dobavjati/izbavjeńje) vs 33
  `labial+lj`, and East/South reflexes keep the epenthetic `l` in *both* classes,
  so no proto+reflex signal distinguishes `sablja` (keep) from `zemja` (drop).
  Among clean matches it is 5-keep vs 4-drop → ≈ net-zero. Not shipped; only a
  hand-curated exception list could help.
- **Stage-8 override guard for the sablja class**: same wall — a guard keyed on
  "a reflex retains the cluster" fires on `zemja` too (East/South keep the `l`),
  so it would regress as much as it fixes. Not shipped.
- **Proto-link recall via etym-ancestor as a scored candidate** (`link_core`):
  measured **+0.00pp** — `link_explicit` already catches the ≥2-language cases and
  the added single-language candidates never out-scored the existing pool.
  Reverted.
- **`-tr/-br` final-cluster reduction** (bratr→brat): lexical (~3-6 entries), and
  any general rule regresses větr/ostry/sestra. Not attempted.

## 4. Did the blame move? (V7 §6.3)

Post-fix histogram (8,624 misses, −106): the buckets I targeted shrank —
`6-proto-rule` 141→139, `same-root-surface` merge-rank 224→211, and the endings
residual absorbed the `-telj`/`-ija`/soft-adj wins (misses that were `7-endings`
now match). The editorial buckets are unchanged by construction:
`3-cluster/vote` (2,698), `diff-root-editorial` merge-rank (1,581), and
`0-root-absent` (1,779) together are **70% of all misses** and are the ceiling —
they require reading the answer (the oracle-cluster's +3.91pp is almost entirely
this editorial slice).

## 5. Parity with the official dictionary (V7 §4, display-only)

Generation and the benchmark are byte-identical before/after (all changes are in
the `export` path, which `evaluate`/`audit`/`proto-eval` never call).

- **Multi-word & reflexive official lemmas now get pages** (`site.rs` official-only
  pass): the `isv.contains(' ')` guard was dropping **1,657** lemmas incl. all
  reflexive `… sę` — they now render (`a takože`, `adamovo jablȯko`, …) and are in
  `search.json`. 12,304 official-only pages (was ~10.6k).
- **Official gloss searchable on matched entries**: matched official-headword
  entries now add their official English gloss tokens to the search keys (already
  searchable on official-only pages) — no entry-HTML change.
- **Family cross-links to official-only pages**: *not* done — the Stage-4 reviewer
  showed the naive skeleton-stem key fuses unrelated lemmas and, because
  `family_block` renders into every member page, it would mutate already-emitted
  generated HTML (a parity regression). It needs a real proto join; left out.

## 6. The single biggest remaining lever

**Representative / surface selection within the right cluster** — oracle-measured
headroom **+3.67pp exact**, and unlike the +3.91pp cluster oracle it is *not*
editorial: it is choosing which attested surface (which language's form, before
repairs) best matches Interslavic, a decision made without reading the answer.
The `1-normalize/representative` bucket (1,563 misses, residual length 696 + y/i
473) is the visible tail of this. The concrete next step the diagnostics point to
is a smarter representative than the fixed `REP_PRIORITY` list — e.g. preferring a
`nom.sg`, `-tel(j)`-agentive-preserving, or long-form-adjective member per POS —
measured against the oracle-representative ceiling on a seeded split.

## 7. Honest note on the parity target

The §1 target "land measurably above 41% exact" was **not reached** (39.92%).
The two instruments explain why and make the claim falsifiable: ~70% of the
remaining error is editorial word-choice or an evidence gap (oracle-cluster's
+3.91pp is almost all editorial), and the recoverable non-editorial headroom
(representative +3.67, proto-link +2.55) is real but not reachable by the cheap
targeted edits that carried V1–V6. The +0.69pp shipped here is the clean,
zero-regression slice; closing to 41% requires the representative-selection work
in §6, which is a model change, not a rule. The proto-engine on-linked target
(>50% exact) was met on *normalized* (52.74%) but not exact (46.68%).
