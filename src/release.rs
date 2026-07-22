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

/// Manifest schema; bump on shape change.
pub const MANIFEST_SCHEMA: u32 = 1;
pub const MANIFEST_PATH: &str = "data/MANIFEST.json";

/// data/ entries NOT covered by the manifest — mirrors `.gitignore` (local
/// multi-gigabyte source datasets and scratch), plus the manifest itself.
/// Keep in sync with `.gitignore`.
const MANIFEST_EXCLUDE: &[&str] = &[
    "MANIFEST.json",
    "raw-wiktextract-data.jsonl",
    "wiktionary",
    "wiktionary-lab.json",
    "__pycache__",
];

fn sha256_file(path: &Path) -> Result<(String, u64)> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok((format!("{:x}", h.finalize()), bytes.len() as u64))
}

/// The covered artifact set: every regular file directly under `data/`,
/// minus the gitignored exclusions, sorted by name — deterministic.
fn covered_files(data_dir: &Path) -> Result<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(data_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !entry.file_type()?.is_file()
            || name.ends_with(".tmp")
            || MANIFEST_EXCLUDE.contains(&name.as_str())
        {
            continue;
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}

/// The exact-pin line from Cargo.toml — the manifest records which crate
/// version produced the release's paradigms.
fn crate_pin() -> Result<String> {
    let toml = std::fs::read_to_string("Cargo.toml")?;
    toml.lines()
        .find_map(|l| {
            let l = l.trim();
            l.strip_prefix("interslavic = ")
                .map(|v| v.trim_matches('"').to_string())
        })
        .context("no interslavic pin in Cargo.toml")
}

fn render_manifest(data_dir: &Path) -> Result<String> {
    let (b0, b1, b2) = crate::site::PROBE_BASELINE;
    let mut s = format!(
        "{{\n  \"schema_version\": {MANIFEST_SCHEMA},\n  \"crate_pin\": \"{}\",\n  \"forms_schema\": {},\n  \"probe_baseline\": [{b0}, {b1}, {b2}],\n  \"files\": [\n",
        crate_pin()?,
        crate::forms::SCHEMA_VERSION,
    );
    let names = covered_files(data_dir)?;
    for (i, name) in names.iter().enumerate() {
        let (hash, bytes) = sha256_file(&data_dir.join(name))?;
        let _ = writeln!(
            s,
            "    {{\"path\": \"data/{name}\", \"sha256\": \"{hash}\", \"bytes\": {bytes}}}{}",
            if i + 1 < names.len() { "," } else { "" }
        );
    }
    s.push_str("  ]\n}\n");
    Ok(s)
}

/// `data-manifest [--write]`: verify (default) or regenerate the manifest.
/// Verification is byte-exact against a re-render, so ANY covered change —
/// content, file added, file removed, pin bump, baseline move — fails until
/// the manifest is regenerated, which is the visible event.
pub fn run_manifest(write: bool) -> Result<()> {
    let rendered = render_manifest(Path::new("data"))?;
    if write {
        std::fs::write(MANIFEST_PATH, &rendered)?;
        println!(
            "Wrote {MANIFEST_PATH} ({} artifacts)",
            rendered.matches("\"path\"").count()
        );
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
    println!(
        "data-manifest: OK — {} artifacts match",
        rendered.matches("\"path\"").count()
    );
    Ok(())
}

/// Verify a manifest rendered for an arbitrary data dir (unit tests).
pub fn verify_manifest_str(data_dir: &Path, manifest: &str) -> Result<bool> {
    render_manifest(data_dir).map(|r| r == manifest)
}

pub fn render_manifest_for(data_dir: &Path) -> Result<String> {
    render_manifest(data_dir)
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
        "- aspect both/either/fingerprint: __ → __ (re-bless target/eval/aspect-pairs.*)",
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
    use super::*;

    /// The committed manifest matches the working tree — the release
    /// contract itself, CI-enforced so it cannot rot. On failure: regenerate
    /// with `cargo run --release -- data-manifest --write` and commit.
    #[test]
    fn committed_manifest_matches_tree() {
        let on_disk = std::fs::read_to_string(MANIFEST_PATH).expect(
            "data/MANIFEST.json missing — run `cargo run --release -- data-manifest --write`",
        );
        assert!(
            verify_manifest_str(Path::new("data"), &on_disk).expect("render"),
            "data/MANIFEST.json is stale — regenerate with `data-manifest --write` and commit \
             the diff (that diff is the visible event)"
        );
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
        std::fs::write(dir.join("a.tsv"), "x\t1\n").unwrap();
        let m1 = render_manifest_for(&dir).expect("render");
        assert!(verify_manifest_str(&dir, &m1).unwrap());
        std::fs::write(dir.join("a.tsv"), "x\t2\n").unwrap();
        assert!(
            !verify_manifest_str(&dir, &m1).unwrap(),
            "content change must invalidate"
        );
        std::fs::write(dir.join("b.tsv"), "y\n").unwrap();
        let m2 = render_manifest_for(&dir).expect("render");
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
