# Integration contract for downstream consumers

What a consumer of slovowiki's artifacts may rely on, in one place. Every
rule here was learned the hard way by a real downstream (mrzavec,
interslavic-rs). The pipeline that produces these artifacts is documented in
[docs/PIPELINE.md](docs/PIPELINE.md); the refresh/release ceremony in
[docs/DATA-REFRESH.md](docs/DATA-REFRESH.md).

## 1. check-text JSON: schema 1

`check-text <file> --json` emits **one versioned envelope** per invocation
(`src/check.rs`, `CHECKTEXT_JSON_SCHEMA = 1`):

```json
{"schema_version": 1, "tokens": [...], "summary": {...}, "lexicon": {...}}
```

- Parse `schema_version` first; a bump is the migration point. The pre-V14.3
  bare token array is retired — do not accept it.
- `tokens` is always present: one object per token with `token`,
  `status` (`known-lemma` | `known-form` | `project` | `generated` |
  `unknown`), `lemmas` (distinct lemma spellings the surface can belong to),
  `analyses`, `ambiguous` (true for multiple lexical readings, including
  same-spelling homographs), `probability` (calibrated, generated lemmas
  only — any non-null value is a suggestion, never verification),
  `suggestions` (unknown tokens), `warning`/`severity`/`prefer`
  (false-friend), `agreement`, `consistency`.
- `summary` appears only with `--summary` (the CI-gate mode): counts per
  status plus `agreement_errors`, `false_friend_warnings`, `severe_warnings`,
  `consistency_warnings`, and the boolean `passed`. The process exit code is
  the gate; `passed` mirrors it.
- `lexicon` appears only with `--lexicon`: `{rows, coinages, official_pins,
  adoptions: [{lemma, adopted_gloss}]}` — row dispositions are part of the
  contract because an adoption appearing or vanishing across data refreshes
  must never be silent.
- `coin-check --json` carries its own `schema_version` (currently 1) and,
  with `--lexicon-row`, names its disposition as `lexicon_row_disposition`.

## 2. Pinning data releases

- Pin **`data-vN` tags**, never commit hashes. A tag is a release claim:
  `data-release.yml` verifies every pushed tag against its own manifest.
- Verify what you extracted against `data/MANIFEST.json`: **plain sha256**
  (plus byte size) of every covered file — the covered set is exactly git's
  tracked `data/` files, minus the manifest itself. The manifest also records
  the `interslavic` crate pin (`crate_pin`), the form-index schema
  (`forms_schema`), and the translation-probe baseline (`probe_baseline`).
- Read `data_release` from the manifest to identify a tree that arrived
  without `.git` — the tree knows its own N.
- Read `data/refresh-changelog.md`'s `### data-vN` sections to see exactly
  what moved between your old pin and your new one: id-keyed row diffs and
  the benchmark before/after table for every official-dictionary refresh.

## 3. The `api/forms` positional row schema

`site/api/` is a static, deterministic API; `api/agent-guide.md` in the
export is the full manual and `api/meta.json` the counts/router spec. The
load-bearing shape (form-index schema 4):

- Route a folded key with `fnv1a32(utf8(key)) % 2048`, fetch
  `api/forms/<n>.json`, read `records[key]`. Verify your fold + router
  against `api/router-selftest.json` before trusting any lookup.
- Each record is a **positional array** — fields by index, not by name:

  ```
  [form, lemma, entry_id, pos, [analyses], source, status, probability, gloss]
  ```

- `api/lemmas.json` rows are positional too (schema 4, twelve fields):

  ```
  [lemma, pos, status, probability, entry_id, gloss, aspect, aspect_partners,
   frequency, langs, branch_pattern, borrowed]
  ```

  Schema evolution is **append-only within a schema's life**: v3 grew two
  trailing fields over v2, v4 four more — consumers must accept trailing
  fields they do not know, and a reshape that isn't append-only bumps
  `schema_version` in `api/meta.json`. The English API (`api/en/`, schema 2)
  and its retry ladder are versioned separately in `api/en/meta.json`.

## 4. Byform order is API

The official dictionary cites byforms in one row (`iměti, imati`), and
paradigm cells can hold variants (`den / denj`). **Their order is
load-bearing and preserved end to end**: `primary_citation_byform` is the
*first* byform, variant splitting keeps cell order, and downstream projects
bless first-variant outputs into their expectations. Reordering byforms is a
breaking change on slovowiki's side and will be declared; consumers must
never re-sort variants, and must not assume any ordering other than
"as shipped".

## 5. Build provenance

Every deployed tree carries `build-info.json` at its root: git revision,
crate name/version, the RESOLVED `interslavic` version from Cargo.lock
(field `interslavic`, plain version with no constraint operator — truthful
even under a `[patch]` override), the official dictionary input path/hash,
the optional pinned `data_release`, and the sha256 of each input cache (the
same digests `data/MANIFEST.json` publishes). `data_release` is non-null only
when the default inputs are used and the full manifest contract verifies
against the current checkout; custom inputs, edited data, or an incomplete
checkout leave it null rather than falsely claiming a data-vN identity. Use
it to identify what produced the artifacts you are consuming; it is
deterministic for a given checkout and exact set of input bytes.

## 6. Never post-process

Treat artifact bytes as canonical. Consumers must **never re-fold, re-sort,
re-case, or re-serialize** artifact contents before comparing or storing
them:

- Exports are deterministic by construction (BTreeMap ordering, no
  timestamps; CI proves byte-identity by double-exporting and diffing
  sha256s). If your bytes differ from a pinned artifact, that is a signal,
  not noise — do not "normalize" it away.
- `data/MANIFEST.json` hashes are over exact bytes; verification after any
  local rewrite (line endings, JSON pretty-printing, Unicode normalization)
  is meaningless.
- Folding is slovowiki's job, done once, with the exact table published in
  `api/agent-guide.md` and self-tested via `api/router-selftest.json` /
  `api/en/selftest.json`. Reimplement the fold only to route lookups, verify
  it against the selftests, and never apply your own casing or Unicode
  normalization on top of shipped keys or forms.
- Candidate lists (`api/en/`) are shipped best-first; `rank` is comparable
  only within one English key. Do not re-rank across keys or stable-sort by
  your own criteria and call the result slovowiki's.
