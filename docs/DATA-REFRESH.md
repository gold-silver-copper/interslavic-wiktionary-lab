# Data refresh & release ceremony (V14 item 4 / #74)

Slovowiki's committed data drifts only through DELIBERATE, logged events.
Two rituals: a **refresh** re-pulls the official dictionary; a **release**
tags the current tree so consumers pin `data-vN` instead of a commit hash.
No build or benchmark path ever touches the network — the download in step
1 is a manual maintainer action.

## Refresh ritual (ordered; every step's output lands in the changelog entry)

1. Download the current interslavic-dictionary.com full export locally
   (outside the repo).
2. `cargo run --release -- refresh-official <downloaded.csv>` — parses the
   file with the production loader, refuses a no-op, installs it as
   `data/official-isv.csv`, and prepends the id-keyed row diff plus an
   EMPTY benchmark table to `data/refresh-changelog.md`.
3. Record the BEFORE numbers (from the last release's changelog entry or a
   pre-refresh run), then:
   - `cargo run --release -- corpus-eval --fit` — refit the corpus-coverage
     calibrator on the new rows;
   - `cargo run --release -- evaluate` — must stay ≥ the CI floor or the
     floor is re-argued IN THE CHANGELOG, never silently;
   - `cargo run --release -- export --out site` — regenerates
     `data/novel-words.tsv`;
   - `cargo run --release -- aspect-eval` — re-bless
     `reports/aspect-pairs.{md,tsv}` (the frozen-manifest guard exists
     for exactly this moment);
   - `cargo run --release -- translation-probe` — if the counts moved,
     update `PROBE_BASELINE` in `src/site/english_api.rs` (a moved baseline
     is legal ONLY here, with the movement explained in the entry).
4. Fill every `__` in the changelog entry's benchmark table. A refresh PR
   with a blank table or without a changelog entry fails review/CI.
5. `cargo run --release -- data-manifest --write` — the manifest diff is
   the visible event.
6. Full battery: `cargo test`, clippy, the three site validators,
   byte-identical double export.

## Release ritual (after the refresh PR — or any consumer-visible artifact
change — merges to master)

```sh
# 1. Bump the release identity IN the tree (schema 2: the tree knows its N):
cargo run --release -- data-manifest --write --release N
#    add a "### data-vN" heading atop the changelog section this release
#    covers, commit both, merge.
# 2. Tag the merge commit and push:
cargo run --release -- data-manifest          # must pass on the merge commit
git tag data-vN <merge-commit> && git push origin data-vN
```

Tagging is a human release decision — CI never tags, but the
`data-release.yml` workflow verifies every pushed `data-v*` tag: the tagged
tree must pass `data-manifest` and its manifest's `data_release` must match
the tag name, so a moved or stale release tag is a red X, not a silent lie.
Ordinary `--write` runs (data changes between releases) carry the committed
`data_release` forward; only this ritual passes `--release`, and only with
`--write` (verify mode always checks the committed identity). `data-manifest`
cross-checks `data_release` against the changelog's newest `### data-vN`
heading in both modes — add the heading FIRST, then `--write --release N`.
First run on a pre-schema-2 tree: `--write --release N` (the old manifest
carries no identity to inherit).

Consumers pin `data-vN`, verify artifacts against `data/MANIFEST.json`
(plain sha256), read `data_release` to identify an extracted tree with no
`.git`, and read `data/refresh-changelog.md`'s `### data-vN` sections to
see exactly what moved between their pins.
