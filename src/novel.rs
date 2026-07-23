//! Single owner of `data/novel-words.tsv` (V15 item 3).
//!
//! One row type, one writer, one parser. The site export builds
//! [`NovelWordRow`]s in memory, serializes them once with [`write_tsv`], and
//! passes the same rows straight to `check::build_index` — no same-run disk
//! round-trip. The CLI paths (check-text, coin-check) read the committed
//! file back through [`load_or_warn`], so every consumer shares one column
//! layout instead of hand-rolled `split('\t')` copies.

use std::fmt::Write as _;
use std::path::Path;

/// The committed proposals artifact (refreshed by `export`).
pub const DEFAULT_NOVEL_WORDS: &str = "data/novel-words.tsv";

const HEADER: &str =
    "form\tpos\tprobability\tbucket\tancestor\tn_langs\tn_branches\tgloss\tclassification\tofficial\n";

/// One novel-word proposal row, exactly as serialized to the TSV.
pub struct NovelWordRow {
    pub form: String,
    pub pos: String,
    /// Calibrated probability. `None` mirrors the historical lenient parse
    /// (`.parse().ok()`): a malformed committed cell still yields a row,
    /// just without a probability. The export always writes `Some`.
    pub prob: Option<f64>,
    pub ancestor: String,
    pub n_langs: usize,
    pub n_branches: usize,
    /// Display gloss; the writer sanitizes tabs/newlines into the cell.
    pub gloss: String,
    /// `novel` or `near-official` (V12 item 3 reconciliation).
    pub classification: String,
    /// The official byform a near-official proposal reconstructs (empty for
    /// truly novel rows).
    pub official: String,
}

/// Serialize rows in their given order (the export sorts before calling).
/// The bucket column is derived here — buckets are only meaningful in
/// calibrated-probability space.
pub fn write_tsv(rows: &[NovelWordRow]) -> String {
    let mut tsv = String::from(HEADER);
    for r in rows {
        let bucket = match r.prob {
            Some(p) if p >= crate::calibrate::PROPOSE_T => "predlog",
            _ => "pregled",
        };
        let _ = writeln!(
            tsv,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            r.form,
            r.pos,
            r.prob.map(|p| format!("{p:.3}")).unwrap_or_default(),
            bucket,
            r.ancestor,
            r.n_langs,
            r.n_branches,
            r.gloss.replace(['\t', '\n'], " "),
            r.classification,
            r.official,
        );
    }
    tsv
}

/// Parse a proposals TSV. Lenient by contract: rows with fewer than 8
/// columns are dropped, numeric cells that fail to parse degrade to
/// `None`/0 — the file is machine-written, so damage means a bad commit,
/// and the checker's job is to keep working on whatever rows remain.
pub fn parse(tsv: &str) -> Vec<NovelWordRow> {
    tsv.lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 8 {
                return None;
            }
            Some(NovelWordRow {
                form: cols[0].to_string(),
                pos: cols[1].to_string(),
                prob: cols[2].parse::<f64>().ok(),
                ancestor: cols[4].to_string(),
                n_langs: cols[5].parse().unwrap_or(0),
                n_branches: cols[6].parse().unwrap_or(0),
                gloss: cols[7].to_string(),
                classification: cols.get(8).unwrap_or(&"").to_string(),
                official: cols.get(9).unwrap_or(&"").to_string(),
            })
        })
        .collect()
}

/// Read and parse the committed proposals file. A missing file is a
/// reproducibility warning, not an error: generated corpus words then
/// classify as unknown (the historical `build_index` behavior).
pub fn load_or_warn(path: &Path) -> Vec<NovelWordRow> {
    let tsv = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!(
            "warning: generated-word proposal artifact unavailable ({}: {e}); \
             generated corpus words will be classified as unknown",
            path.display()
        );
        String::new()
    });
    parse(&tsv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_parse_round_trip() {
        let rows = vec![NovelWordRow {
            form: "žabervok".to_string(),
            pos: "noun".to_string(),
            prob: Some(0.40625),
            ancestor: "*žabrъ".to_string(),
            n_langs: 5,
            n_branches: 2,
            gloss: "jabberwock,\tmonster".to_string(),
            classification: "novel".to_string(),
            official: String::new(),
        }];
        let tsv = write_tsv(&rows);
        let back = parse(&tsv);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].form, "žabervok");
        // The writer quantizes to 3 decimals and sanitizes the gloss.
        assert_eq!(back[0].prob, Some(0.406));
        assert_eq!(back[0].gloss, "jabberwock, monster");
        assert_eq!(back[0].n_langs, 5);
    }

    #[test]
    fn short_rows_drop_and_malformed_cells_degrade() {
        let parsed = parse(
            "form\tpos\tprobability\tbucket\tancestor\tn_langs\tn_branches\tgloss\n\
             short\trow\n\
             ok\tnoun\tnot-a-number\tpregled\t-\tx\ty\tgloss text\n",
        );
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].form, "ok");
        assert_eq!(parsed[0].prob, None);
        assert_eq!((parsed[0].n_langs, parsed[0].n_branches), (0, 0));
    }
}
