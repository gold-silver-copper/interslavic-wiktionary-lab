//! Pinnable data releases and the official-dictionary refresh tool
//! (V14 item 4 / deferred #74).
//!
//! Downstream consumers used to pin slovowiki by raw commit hash; live-sheet
//! drift (V10 measured 8 → 17 upstream noun mismatches between two
//! measurements) was a slow skew nobody saw. Two pieces make it a visible,
//! versioned event instead:
//!
//! - `data/MANIFEST.json` — sha256 + size for every committed `data/`
//!   artifact, plus the crate pin, form-index schema version, and the probe
//!   baseline. `data-manifest` verifies it (CI does too, so it cannot rot);
//!   `data-manifest --write` regenerates it. A release is a `data-vN` tag
//!   whose tree passes verification — consumers pin the tag, not a hash.
//! - `refresh-official` — reads a freshly, MANUALLY downloaded
//!   interslavic-dictionary.com export (no build or benchmark path ever
//!   touches the network; house rule 1), refuses no-op refreshes, installs
//!   the new CSV, and prepends an id-keyed row diff to
//!   `data/refresh-changelog.md` with the benchmark checklist the
//!   `docs/DATA-REFRESH.md` ceremony fills in.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// Manifest schema; bump on shape change. Schema 2 (V14.2 item 6) adds
/// `data_release`: the data-vN identity, so a checked-out tree — tarball,
/// vendored copy, no `.git` — can say which release it is, and changelog
/// entries map to pins.
pub const MANIFEST_SCHEMA: u32 = 2;
pub const MANIFEST_PATH: &str = "data/MANIFEST.json";

fn sha256_file(path: &Path) -> Result<(String, u64)> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok((format!("{:x}", h.finalize()), bytes.len() as u64))
}

/// The covered artifact set: the TRACKED files under `data/` per git — the
/// authority the old hand-mirrored `.gitignore` excerpt tried to imitate
/// (V14.1 finding 7). Stray local files can neither break verification nor
/// leak into a committed manifest. git is required; this tool is
/// maintainer/CI-facing and both run in the repository.
fn tracked_data_files() -> Result<Vec<String>> {
    let out = std::process::Command::new("git")
        .args(["ls-files", "-z", "--", "data"])
        .output()
        .context("run `git ls-files` (the manifest covers TRACKED data/ files)")?;
    anyhow::ensure!(
        out.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut names: Vec<String> = String::from_utf8(out.stdout)?
        .split('\0')
        .filter(|s| !s.is_empty() && *s != MANIFEST_PATH)
        .map(str::to_string)
        .collect();
    names.sort();
    anyhow::ensure!(
        !names.is_empty(),
        "git ls-files returned no data/ files — not at the repository root?"
    );
    Ok(names)
}

/// The resolved interslavic version from Cargo.lock — format-stable, and
/// the truth about what actually built (V14.1 finding 6; the old
/// Cargo.toml line-trim broke on legal `{ version = \"…\" }` forms).
fn resolved_pin() -> Result<String> {
    let lock = std::fs::read_to_string("Cargo.lock").context("read Cargo.lock")?;
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() == "name = \"interslavic\"" {
            if let Some(version) = lines
                .next()
                .and_then(|l| l.trim().strip_prefix("version = "))
            {
                return Ok(format!("={}", version.trim_matches('"')));
            }
        }
    }
    anyhow::bail!("interslavic not found in Cargo.lock")
}

/// Render the manifest for an explicit file list — serde_json end to end
/// (V14.1 finding 6): escaping and well-formedness are structural, and the
/// byte-exact verification compares canonical serde renderings.
/// The release number recorded in the committed manifest — carried forward
/// by `--write` so ordinary data changes don't restate it; only a release
/// bump passes `--release N` explicitly.
fn committed_release() -> Result<u32> {
    let text = std::fs::read_to_string(MANIFEST_PATH).with_context(|| {
        format!("{MANIFEST_PATH} missing — first write needs `--write --release N`")
    })?;
    let v: serde_json::Value = serde_json::from_str(&text).context("parse committed manifest")?;
    v["data_release"].as_u64().map(|n| n as u32).context(
        "committed manifest has no data_release (pre-schema-2 tree) — regenerate with \
         `--write --release N`",
    )
}

/// The newest `### data-vN` heading in the refresh changelog — the
/// non-circular witness for `data_release` (V14.3 item 5): verify mode
/// would otherwise read N from the very manifest being verified, so a
/// hand-edited number round-tripped green. Two committed, reviewed files
/// must now agree to lie.
fn newest_changelog_release(changelog: &Path) -> Result<u32> {
    let text = std::fs::read_to_string(changelog)
        .with_context(|| format!("read {}", changelog.display()))?;
    text.lines()
        .find_map(|l| l.trim().strip_prefix("### data-v"))
        .and_then(|n| n.trim().parse::<u32>().ok())
        .with_context(|| {
            format!(
                "{} has no `### data-vN` heading — the release ritual adds one per release",
                changelog.display()
            )
        })
}

fn render_manifest(files: &[String], data_release: u32) -> Result<String> {
    let mut entries: Vec<serde_json::Value> = Vec::new();
    for path in files {
        let (sha256, bytes) = sha256_file(Path::new(path))?;
        entries.push(serde_json::json!({
            "path": path,
            "sha256": sha256,
            "bytes": bytes,
        }));
    }
    let (b0, b1, b2) = crate::site::PROBE_BASELINE;
    let manifest = serde_json::json!({
        "schema_version": MANIFEST_SCHEMA,
        "data_release": data_release,
        "crate_pin": resolved_pin()?,
        "forms_schema": crate::forms::SCHEMA_VERSION,
        "probe_baseline": [b0, b1, b2],
        "files": entries,
    });
    Ok(serde_json::to_string_pretty(&manifest)? + "\n")
}

/// `data-manifest [--write]`: verify (default) or regenerate the manifest.
/// Verification is byte-exact against a re-render, so ANY covered change —
/// content, file added, file removed, pin bump, baseline move — fails until
/// the manifest is regenerated, which is the visible event.
pub fn run_manifest(write: bool, release: Option<u32>) -> Result<()> {
    // --release is the release-bump act and only makes sense while
    // writing; in verify mode it would render the comparison with the
    // caller's N and misreport the mismatch as data drift (V14.3 item 5).
    anyhow::ensure!(
        write || release.is_none(),
        "--release only makes sense with --write (verify mode always checks the committed identity)"
    );
    let files = tracked_data_files()?;
    let n = match release {
        Some(n) => n,
        None => committed_release()?,
    };
    // Non-circular identity witness: the changelog's newest release
    // heading must agree, in BOTH modes — writing enforces the ritual
    // order (heading first), verifying catches hand-edits and bad merges.
    let witnessed = newest_changelog_release(Path::new("data/refresh-changelog.md"))?;
    anyhow::ensure!(
        witnessed == n,
        "data_release {n} disagrees with data/refresh-changelog.md's newest heading \
         `### data-v{witnessed}` — the two committed files must agree (see docs/DATA-REFRESH.md)"
    );
    let rendered = render_manifest(&files, n)?;
    if write {
        std::fs::write(MANIFEST_PATH, &rendered)?;
        println!("Wrote {MANIFEST_PATH} ({} artifacts)", files.len());
        return Ok(());
    }
    let on_disk = std::fs::read_to_string(MANIFEST_PATH)
        .with_context(|| format!("{MANIFEST_PATH} missing — run `data-manifest --write`"))?;
    anyhow::ensure!(
        on_disk == rendered,
        "{MANIFEST_PATH} does not match the working tree — a covered data artifact, the crate \
         pin, or the probe baseline changed. Regenerate with `cargo run --release -- \
         data-manifest --write` and commit the diff (that diff IS the visible event)."
    );
    println!("data-manifest: OK — {} artifacts match", files.len());
    Ok(())
}

/// `refresh-official <input>`: install a freshly downloaded official export
/// and prepend the id-keyed row diff to the refresh changelog. Refuses
/// no-op refreshes. The benchmark before/after table is filled by the
/// `docs/DATA-REFRESH.md` ceremony — this tool records the DATA facts.
pub fn run_refresh(input: &Path, official: &Path, changelog: &Path) -> Result<()> {
    // Parse BOTH files with the production loader first: a refresh that the
    // pipeline cannot read must fail before touching anything.
    let new_entries = crate::official::load(input)?;
    let old_entries = crate::official::load(official)?;

    // Row maps come from the SAME RFC-4180 reader the production loader
    // uses (V14.1 finding 3) — the previous line-based heuristic glued
    // multiline quoted cells to the wrong row (BTreeMap max-key, not the
    // previous row), dropped comma-less continuation lines entirely, and
    // let a digit-comma continuation clobber an unrelated id. Cells are
    // re-joined with a non-CSV separator so comparison is content-exact
    // and quoting-insensitive; duplicate ids mean a corrupt export.
    let raw_rows = |path: &Path| -> Result<BTreeMap<String, String>> {
        let text = std::fs::read_to_string(path)?;
        let mut out = BTreeMap::new();
        for rec in crate::official::read_csv_records(&text).into_iter().skip(1) {
            let Some((id, rest)) = rec.split_first() else {
                continue;
            };
            if id.is_empty() {
                continue;
            }
            anyhow::ensure!(
                out.insert(id.clone(), rest.join("\u{1f}")).is_none(),
                "{}: duplicate row id '{id}' — corrupt export, refusing",
                path.display()
            );
        }
        Ok(out)
    };
    let old = raw_rows(official)?;
    let new = raw_rows(input)?;

    let added: Vec<&String> = new.keys().filter(|id| !old.contains_key(*id)).collect();
    let removed: Vec<&String> = old.keys().filter(|id| !new.contains_key(*id)).collect();
    let changed: Vec<&String> = new
        .iter()
        .filter(|(id, row)| old.get(*id).is_some_and(|o| o != *row))
        .map(|(id, _)| id)
        .collect();
    anyhow::ensure!(
        !(added.is_empty() && removed.is_empty() && changed.is_empty()),
        "refresh-official: the input is identical to {} — refusing a no-op refresh",
        official.display()
    );

    let head = |ids: &[&String]| -> String {
        let shown: Vec<&str> = ids.iter().take(50).map(|s| s.as_str()).collect();
        let suffix = if ids.len() > 50 {
            format!(" … (+{} more)", ids.len() - 50)
        } else {
            String::new()
        };
        format!("{}{suffix}", shown.join(", "))
    };
    let mut entry = String::new();
    writeln!(
        entry,
        "## Refresh — {} rows → {} rows\n",
        old_entries.len(),
        new_entries.len()
    )?;
    writeln!(
        entry,
        "Row diff (by id): **{} added, {} removed, {} changed**.\n",
        added.len(),
        removed.len(),
        changed.len()
    )?;
    if !added.is_empty() {
        writeln!(entry, "- added: {}", head(&added))?;
    }
    if !removed.is_empty() {
        writeln!(entry, "- removed: {}", head(&removed))?;
    }
    if !changed.is_empty() {
        writeln!(entry, "- changed: {}", head(&changed))?;
    }
    writeln!(
        entry,
        "\n### Benchmarks (before → after; fill via docs/DATA-REFRESH.md, every line REQUIRED)\n"
    )?;
    for line in [
        "- evaluate exact top-1: __% → __%",
        "- evaluate normalized top-1: __% → __%",
        "- corpus-eval exact/normalized: __ → __",
        "- probe verified/generated-only/miss: __/__/__ → __/__/__ (update PROBE_BASELINE)",
        "- aspect both/either/fingerprint: __ → __ (re-bless reports/aspect-pairs.*)",
        "- form index records/keys/lemmas: __ → __",
    ] {
        writeln!(entry, "{line}")?;
    }
    writeln!(entry)?;

    // Prepend under the header so the newest refresh reads first.
    let existing = std::fs::read_to_string(changelog).unwrap_or_else(|_| {
        "# Official-dictionary refresh changelog\n\nNewest first. Every entry is written by \
         `refresh-official` and completed by the docs/DATA-REFRESH.md ceremony.\n\n"
            .to_string()
    });
    let (header, rest) = match existing.find("\n## ") {
        Some(i) => existing.split_at(i + 1),
        None => (existing.as_str(), ""),
    };
    std::fs::write(changelog, format!("{header}{entry}{rest}"))?;
    std::fs::copy(input, official)?;
    println!(
        "refresh-official: installed {} ({} added / {} removed / {} changed rows); \
         changelog entry prepended to {} — now run the docs/DATA-REFRESH.md ceremony",
        official.display(),
        added.len(),
        removed.len(),
        changed.len(),
        changelog.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::panic,
        clippy::unwrap_in_result,
        clippy::indexing_slicing,
        clippy::too_many_lines,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::match_same_arms,
        clippy::map_unwrap_or,
        clippy::redundant_closure_for_method_calls,
        clippy::uninlined_format_args,
        clippy::needless_pass_by_value
    )]
    use super::*;

    /// The committed manifest matches the working tree — the release
    /// contract itself, CI-enforced so it cannot rot. On failure: regenerate
    /// with `cargo run --release -- data-manifest --write` and commit. Also
    /// pins that the manifest is machine-parseable JSON (finding 6) and
    /// covers exactly git's tracked data/ files (finding 7).
    #[test]
    fn committed_manifest_matches_tree() {
        let on_disk = std::fs::read_to_string(MANIFEST_PATH).expect(
            "data/MANIFEST.json missing — run `cargo run --release -- data-manifest --write`",
        );
        let files = tracked_data_files().expect("git ls-files");
        let n = committed_release().expect("committed data_release");
        assert_eq!(
            on_disk,
            render_manifest(&files, n).expect("render"),
            "data/MANIFEST.json is stale — regenerate with `data-manifest --write` and commit \
             the diff (that diff is the visible event)"
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&on_disk).expect("manifest must always parse as JSON");
        assert_eq!(parsed["files"].as_array().unwrap().len(), files.len());
        assert!(parsed["crate_pin"].as_str().unwrap().starts_with('='));
        assert_eq!(parsed["schema_version"], MANIFEST_SCHEMA as u64);
        assert!(
            parsed["data_release"].as_u64().is_some(),
            "schema 2: a tree must know which data-vN it is"
        );
    }

    /// V14.3 item 5: --release is write-only, and data_release has a
    /// non-circular witness — the changelog heading.
    #[test]
    fn manifest_release_flag_and_witness_discipline() {
        let err = run_manifest(false, Some(5)).unwrap_err();
        assert!(err.to_string().contains("--write"), "{err}");
        let dir = std::env::temp_dir().join(format!(
            "slovowiki-witness-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("t")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cl = dir.join("cl.md");
        std::fs::write(&cl, "# log\n\n### data-v4\n\n## entry\n").unwrap();
        assert_eq!(newest_changelog_release(&cl).unwrap(), 4);
        std::fs::write(&cl, "# log\n\nno heading\n").unwrap();
        assert!(newest_changelog_release(&cl).is_err());
        // The committed pair agrees (the real cross-check runs in
        // committed_manifest_matches_tree via run_manifest's own path).
        assert_eq!(
            newest_changelog_release(Path::new("data/refresh-changelog.md")).unwrap(),
            committed_release().unwrap()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn manifest_detects_tampering_and_refresh_refuses_noop() {
        let dir = std::env::temp_dir().join(format!(
            "slovowiki-release-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("t")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file_a = dir.join("a.tsv").to_string_lossy().to_string();
        std::fs::write(&file_a, "x\t1\n").unwrap();
        let m1 = render_manifest(std::slice::from_ref(&file_a), 1).expect("render");
        assert_eq!(
            m1,
            render_manifest(std::slice::from_ref(&file_a), 1).unwrap()
        );
        std::fs::write(&file_a, "x\t2\n").unwrap();
        assert_ne!(
            m1,
            render_manifest(std::slice::from_ref(&file_a), 1).unwrap(),
            "content change must invalidate"
        );
        let file_b = dir.join("b.tsv").to_string_lossy().to_string();
        std::fs::write(&file_b, "y\n").unwrap();
        let m2 = render_manifest(&[file_a.clone(), file_b], 1).expect("render");
        assert_ne!(m1, m2, "added file must change the manifest");

        // refresh-official refuses a no-op (identical input).
        let a = dir.join("official-a.csv");
        std::fs::copy("data/official-isv.csv", &a).unwrap();
        let err =
            run_refresh(&a, Path::new("data/official-isv.csv"), &dir.join("cl.md")).unwrap_err();
        assert!(err.to_string().contains("no-op"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// V14.1 finding 3: the refresh diff survives the three multiline-CSV
    /// traps the old heuristic fell into — a quoted multiline cell, a
    /// comma-less continuation line, and a continuation line that LOOKS
    /// like a new row (digits + comma). Only the truly-changed id reports.
    #[test]
    fn refresh_diff_handles_multiline_cells() {
        let dir = std::env::temp_dir().join(format!(
            "slovowiki-refresh-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("t")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let header = "id,isv,addition,partOfSpeech,type,en\n";
        // Row 5101 has a quoted multiline cell whose continuations include a
        // comma-less line AND a '1985,'-shaped line; row 24020 is ordinary.
        let base = format!(
            "{header}5101,slovo,,n.,1,\"word\nplain continuation\n1985, pěsnja goda\"\n24020,dom,,m.,1,house\n"
        );
        let changed = base.replace("1985, pěsnja goda", "1985, pěsnja lěta");
        let old_p = dir.join("old.csv");
        let new_p = dir.join("new.csv");
        std::fs::write(&old_p, &base).unwrap();
        std::fs::write(&new_p, &changed).unwrap();
        let changelog = dir.join("cl.md");
        run_refresh(&new_p, &old_p, &changelog).expect("refresh applies");
        let entry = std::fs::read_to_string(&changelog).unwrap();
        assert!(
            entry.contains("0 added, 0 removed, 1 changed") && entry.contains("- changed: 5101"),
            "only the multiline row may report as changed:\n{entry}"
        );
        assert!(
            !entry.contains("1985") && !entry.contains("24020,"),
            "phantom rows must not appear:\n{entry}"
        );
        // Duplicate ids are a corrupt export, not last-wins.
        let dup = format!("{header}1,a,,n.,1,x\n1,b,,n.,1,y\n");
        std::fs::write(&new_p, dup).unwrap();
        let err = run_refresh(&new_p, &old_p, &changelog).unwrap_err();
        assert!(err.to_string().contains("duplicate row id"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
