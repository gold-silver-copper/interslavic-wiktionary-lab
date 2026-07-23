//! Whole-output fingerprint + differ (V15 item 8, adopted from the
//! interslavic-rs release workflow).
//!
//! The export tree sha proves two builds are identical; this module
//! EXPLAINS non-identity. `canonical_dump` renders the crate's lexical
//! record surface — every `FormRecord` the checker/API index carries,
//! built from the committed dictionary + novel-words data exactly as
//! `check-text` and the export build it — as sorted, keyed, one-per-line
//! text. `fnv1a64` reduces it to one number pinned in a unit test, and
//! `diff-output` turns "the number moved" into the enumerated record diff.
//!
//! CLI: `dump-output [--out FILE]` prints the fingerprint (and writes the
//! dump); `diff-output BEFORE AFTER` compares two dumps.

use anyhow::Result;
use std::fmt::Write as _;
use std::path::Path;

/// FNV-1a, 64-bit. The 32-bit sibling in forms.rs shards keys; this one
/// fingerprints the whole canonical dump, where 32 bits would collide.
pub fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// One line per record, tab-keyed, sorted — canonical by construction.
/// Probabilities are quantized to 3 decimals (the novel-words TSV cell
/// precision) so the dump never depends on float formatting drift.
pub fn canonical_dump() -> Result<String> {
    let entries = crate::official::load(Path::new(crate::DEFAULT_OFFICIAL))?;
    let novel = crate::novel::load_or_warn(Path::new(crate::novel::DEFAULT_NOVEL_WORDS));
    let index = crate::check::build_index(&entries, &novel, Default::default());
    let mut lines: Vec<String> = Vec::new();
    for recs in index.by_key.values() {
        for r in recs {
            let mut analyses: Vec<&str> = r.analyses.iter().map(String::as_str).collect();
            analyses.sort_unstable();
            let prob = r.probability.map(|p| format!("{p:.3}")).unwrap_or_default();
            lines.push(format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                r.key,
                r.form,
                r.lemma,
                r.lemma_key,
                r.entry_id,
                r.pos,
                r.source,
                r.status,
                prob,
                analyses.join(","),
            ));
        }
    }
    lines.sort_unstable();
    let mut out = String::with_capacity(lines.len() * 48);
    for l in &lines {
        out.push_str(l);
        out.push('\n');
    }
    Ok(out)
}

/// `dump-output`: print the fingerprint, optionally write the full dump.
pub fn run_dump(out: Option<&Path>) -> Result<()> {
    let dump = canonical_dump()?;
    if let Some(path) = out {
        std::fs::write(path, &dump)?;
        println!(
            "wrote {} ({} records)",
            path.display(),
            dump.lines().count()
        );
    }
    println!("output fingerprint: {:016x}", fnv1a64(&dump));
    Ok(())
}

/// `diff-output`: enumerate the record-level differences of two dumps.
pub fn run_diff(before: &Path, after: &Path) -> Result<()> {
    let a = std::fs::read_to_string(before)?;
    let b = std::fs::read_to_string(after)?;
    let sa: std::collections::BTreeSet<&str> = a.lines().collect();
    let sb: std::collections::BTreeSet<&str> = b.lines().collect();
    let removed: Vec<&&str> = sa.difference(&sb).collect();
    let added: Vec<&&str> = sb.difference(&sa).collect();
    let mut s = String::new();
    let _ = writeln!(s, "- removed: {} records", removed.len());
    for l in removed.iter().take(50) {
        let _ = writeln!(s, "  - {l}");
    }
    if removed.len() > 50 {
        let _ = writeln!(s, "  … {} more", removed.len() - 50);
    }
    let _ = writeln!(s, "+ added: {} records", added.len());
    for l in added.iter().take(50) {
        let _ = writeln!(s, "  + {l}");
    }
    if added.len() > 50 {
        let _ = writeln!(s, "  … {} more", added.len() - 50);
    }
    print!("{s}");
    println!("fingerprints: {:016x} -> {:016x}", fnv1a64(&a), fnv1a64(&b));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pinned whole-output fingerprint. If this fails, the record
    /// surface changed: regenerate and ENUMERATE the diff —
    ///   git stash && cargo run --release -- dump-output --out /tmp/before.tsv && git stash pop
    ///   cargo run --release -- dump-output --out /tmp/after.tsv
    ///   cargo run --release -- diff-output /tmp/before.tsv /tmp/after.tsv
    /// then update FINGERPRINT here and record the enumerated diff in the
    /// commit message (a data refresh records it in the refresh changelog).
    /// An UNEXPLAINED movement is a bug, not a pin to bump.
    #[test]
    fn output_fingerprint_is_pinned() {
        if cfg!(debug_assertions) {
            eprintln!(
                "skipped: the pin is enforced in release runs (CI and `cargo test --release`)"
            );
            return;
        }
        const FINGERPRINT: u64 = 0x262d_d798_416d_323f;
        let dump = canonical_dump().unwrap();
        let got = fnv1a64(&dump);
        assert_eq!(
            got,
            FINGERPRINT,
            "output fingerprint moved: {FINGERPRINT:016x} -> {got:016x} \
             ({} records). See this test's doc comment for the regenerate/diff \
             ritual and the changelog obligation.",
            dump.lines().count()
        );
    }

    #[test]
    fn fnv1a64_matches_reference_vectors() {
        // Published FNV-1a 64 test vectors.
        assert_eq!(fnv1a64(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64("a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64("foobar"), 0x85944171f73967e8);
    }
}
