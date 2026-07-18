//! Corpus coverage planning, raw-page deduplication, and coverage reporting.
//!
//! These routines decide which source records reach rendering; they do not
//! depend on page-specific renderers.

use super::layout::truncate;
use super::model::HeadwordIndex;
use crate::consensus::ConsensusConfig;
use crate::official::{self, OfficialEntry};
use anyhow::Result;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
// ---------------------------------------------------------------------------
// Raw-lemma coverage reporting (issue #35)
//
// A transparent, auditable account of the RAW Slavic-Wiktionary datasets: which
// data went in, how many words were included, and how many were excluded and
// why. It stitches together three views:
//   1. EXTRACTION coverage — the drop-reason tally `extract-raw-slavic` wrote to
//      `data/raw-slavic-coverage.json` (Slavic pages seen → kept / dropped-by-reason).
//   2. SITE coverage — of the kept lemmas, how many `export` renders as raw-only
//      pages vs dedups against an official/generated headword. This *replicates*
//      the export dedup (same `build_sets`/`generate_set`, homograph + same-concept
//      suppression, xref + display-headword fold), so the `rendered-raw` number
//      must reconcile with the export's actual `R`-status page count.
//   3. NATIVE JOIN — the fraction of raw lemmas that gain native ru/pl/cs
//      enrichment (an `EnrichIndex` hit), by language.
// The report never reads the benchmark path; it only touches the raw corpus and
// the (display-only) site index, keeping the raw path benchmark-isolated.
// ---------------------------------------------------------------------------

/// One raw lemma's fate under the export dedup (site coverage view). The
/// deduped variants carry WHERE the word's content lives (issue #64), so the
/// raw pre-pass can point word chips at that internal page instead of out to
/// the native Wiktionary; `coverage` only distinguishes rendered vs deduped.
#[derive(Clone, PartialEq, Eq)]
pub(super) enum RawFate {
    /// Gets its own raw page; carries the ě-blind display fold it claimed.
    Rendered { efold: String },
    /// Word or display fold empty — nothing to render or point at.
    Skipped,
    /// Verbatim `(lang, word)` is already a cognate member of an entry — the
    /// cognate `xref` resolves chip links to it, nothing extra to record.
    DedupedXref,
    /// Display fold (or its ě-blind variant) is an official / generated /
    /// official-only headword: `target` is that page's id.
    DedupedFold { target: usize },
    /// An earlier raw lemma claimed the same ě-blind fold; the pre-pass
    /// resolves the fold to that twin's page id.
    DedupedRawTwin { efold: String },
}

/// Minimal replica of `export_corpus`'s per-set state, enough to rebuild the
/// display-headword index (`isv_to_id`) and cognate cross-reference (`xref`)
/// that the raw dedup consults. Kept in lock-step with `export_corpus`.
pub(super) struct CovPrepared {
    pub(super) id: usize,
    pub(super) g: crate::corpus::GeneratedWord,
    pub(super) display: String,
    pub(super) matched: Option<(usize, usize)>,
    pub(super) suppressed: bool,
}

/// Select an official sense only with positive lexical evidence. Exact/folded
/// spelling is a candidate lookup, not enough by itself to establish identity.
pub(super) fn select_official_entry(
    rows: &[usize],
    official_entries: &[OfficialEntry],
    pos: crate::model::Pos,
    set_gloss: &str,
) -> Option<usize> {
    let set_tokens = crate::dump::gloss_tokens(set_gloss);
    let set_compact = set_tokens.join("");
    rows.iter()
        .copied()
        .filter(|&index| official_entries[index].pos == pos)
        .map(|index| {
            let gloss = crate::dump::gloss_tokens(&official_entries[index].english);
            let overlap = set_tokens
                .iter()
                .filter(|token| gloss.contains(token))
                .count();
            let compound_match = !set_compact.is_empty() && set_compact == gloss.join("");
            (index, overlap, compound_match)
        })
        .filter(|(_, overlap, compound_match)| *overlap > 0 || *compound_match)
        .max_by_key(|(_, overlap, compound_match)| (*overlap, *compound_match))
        .map(|(index, _, _)| index)
}

/// Build the identity-safe headword index (`isv_to_id`) and cognate cross-reference
/// (`xref`) exactly as `export_corpus` does, so a raw lemma is judged
/// "already covered" identically. Returns them plus the generated/official
/// headword counts used for the reconciliation lines.
pub(super) fn build_corpus_render_index(
    corpus: &crate::dump::LemmaCorpus,
    official_entries: &[OfficialEntry],
) -> (
    crate::enrich::Xref,
    HeadwordIndex,
    usize, // generated pages (non-suppressed)
    usize, // official-only pages
) {
    let cfg = ConsensusConfig::production();
    let sets = crate::corpus::build_sets(corpus);

    let mut official_by_exact: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    let mut official_by_fold: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, e) in official_entries.iter().enumerate() {
        let isv = e.isv.trim();
        if isv.is_empty() || isv.contains(' ') || isv.contains('#') {
            continue;
        }
        official_by_exact
            .entry(isv.to_lowercase())
            .or_default()
            .push(i);
        official_by_fold
            .entry(crate::orthography::to_standard(&isv.to_lowercase()))
            .or_default()
            .push(i);
    }

    // First pass: generate every set (same as export).
    let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut prepared: Vec<CovPrepared> = Vec::new();
    let mut id = 0usize;
    for set in sets {
        let g = crate::corpus::generate_set(set, &cfg);
        let form = g.form().to_string();
        if form.is_empty() {
            continue;
        }
        id += 1;
        let matched: Option<(usize, usize)> =
            g.candidates
                .iter()
                .take(5)
                .enumerate()
                .find_map(|(rank, c)| {
                    let lower = c.form.trim().to_lowercase();
                    let rows = if let Some(rows) = official_by_exact.get(&lower) {
                        rows.as_slice()
                    } else {
                        let rows = official_by_fold
                            .get(&crate::orthography::to_standard(&lower))?
                            .as_slice();
                        let mut spellings = rows
                            .iter()
                            .map(|&i| official_entries[i].isv.trim().to_lowercase());
                        let first = spellings.next()?;
                        if spellings.any(|s| s != first) {
                            return None;
                        }
                        rows
                    };
                    let i = select_official_entry(rows, official_entries, g.set.pos, &g.set.gloss)?;
                    Some((rank + 1, i))
                });
        let display = matched
            .map(|(_, i)| official_entries[i].isv.trim().to_string())
            .unwrap_or_else(|| form.clone());
        prepared.push(CovPrepared {
            id,
            g,
            display,
            matched,
            suppressed: false,
        });
    }

    // Homograph / duplicate dedup: one representative per official sense.
    {
        let rank = |p: &CovPrepared, en: &str| -> (usize, i32) {
            let a = crate::dump::gloss_tokens(&p.g.set.gloss);
            let b = crate::dump::gloss_tokens(en);
            let overlap = a.iter().filter(|t| b.contains(t)).count();
            (overlap, (p.g.score * 1000.0) as i32)
        };
        let mut best: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for (i, p) in prepared.iter().enumerate() {
            if let Some((_, entry)) = p.matched {
                let e = &official_entries[entry];
                let key = e.id.clone();
                let win = match best.get(&key) {
                    Some(&j) => rank(p, &e.english) > rank(&prepared[j], &e.english),
                    None => true,
                };
                if win {
                    best.insert(key, i);
                }
            }
        }
        for (i, p) in prepared.iter_mut().enumerate() {
            let Some((_, entry)) = p.matched else {
                continue;
            };
            let key = official_entries[entry].id.clone();
            if best.get(&key) != Some(&i) {
                p.matched = None;
                p.display = p.g.form().to_string();
            }
        }
    }

    // Same-concept suppression: collapse duplicate pages sharing a folded form and
    // a gloss token with a stronger set.
    {
        let gloss_of = |p: &CovPrepared| -> Vec<String> {
            match p.matched {
                Some((_, entry)) => crate::dump::gloss_tokens(&official_entries[entry].english),
                None => crate::dump::gloss_tokens(&p.g.set.gloss),
            }
        };
        let rank = |p: &CovPrepared| (p.matched.is_some(), (p.g.score * 1000.0) as i32);
        let mut by_form: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, p) in prepared.iter().enumerate() {
            by_form
                .entry(crate::orthography::to_standard(&p.g.form().to_lowercase()))
                .or_default()
                .push(i);
        }
        for (_f, mut group) in by_form {
            if group.len() < 2 {
                continue;
            }
            group.sort_by(|&a, &b| rank(&prepared[b]).cmp(&rank(&prepared[a])));
            let mut kept: Vec<Vec<String>> = Vec::new();
            for &i in &group {
                let gl = gloss_of(&prepared[i]);
                if !gl.is_empty() && kept.iter().any(|k| gl.iter().any(|t| k.contains(t))) {
                    prepared[i].suppressed = true;
                } else {
                    kept.push(gl);
                }
            }
        }
    }

    // Exact-first, ambiguity-aware display-headword index, and the cognate
    // cross-reference: every member word of every surviving set.
    let mut isv_to_id = HeadwordIndex::default();
    let mut xref = crate::enrich::Xref::new();
    let generated_pages = prepared.iter().filter(|p| !p.suppressed).count();
    for p in &prepared {
        if p.suppressed {
            continue;
        }
        isv_to_id.insert(&p.display, p.id);
        for m in &p.g.set.members {
            xref.insert(&m.lang, &m.word, p.id);
        }
    }

    // Official lemmas no surviving generated page represents: reserve ids and
    // fold them into `isv_to_id`, so raw dedup mirrors the real export.
    covered.clear();
    for p in prepared.iter().filter(|p| !p.suppressed) {
        if let Some((_, entry)) = p.matched {
            covered.insert(official_entries[entry].id.clone());
        }
    }
    let mut official_only = 0usize;
    let mut official_only_records: Vec<(usize, String)> = Vec::new();
    for e in official_entries {
        let isv = e.isv.trim();
        if isv.is_empty() || isv.contains('#') {
            continue;
        }
        if !covered.insert(e.id.clone()) {
            continue;
        }
        id += 1;
        official_only += 1;
        official_only_records.push((id, isv.to_string()));
    }
    for (oid, isv) in &official_only_records {
        isv_to_id.insert(isv, *oid);
    }

    (xref, isv_to_id, generated_pages, official_only)
}

/// The single raw-lemma dedup rule: BOTH `export_corpus`'s raw render loop and
/// the `coverage` command classify through this function, so the rendered/deduped
/// split always reconciles. `raw_covered` carries the running raw-vs-raw dedup
/// set (mutated).
///
/// A lemma is deduped when: its word or display fold is empty (`Skipped`); it
/// is already a cognate member of a generated page (verbatim `(lang, word)`
/// xref match — `DedupedXref`); its display fold is already an official /
/// generated / official-only entry (`DedupedFold`; catches internationalisms
/// like konflikt whose source spelling isn't a cognate member but whose ISV
/// form has a page); or another raw lemma already claimed the same display
/// fold (`DedupedRawTwin`; same word under several POS, cross-language twins)
/// — so each attested ISV spelling gets exactly one page, while distinct words
/// the phonemic fold conflated (vođa / voda) stay separate.
pub(super) fn raw_lemma_fate(
    lemma: &crate::dump::RawSlavicLemma,
    xref: &crate::enrich::Xref,
    isv_to_id: &HeadwordIndex,
    raw_covered: &mut std::collections::HashSet<String>,
) -> RawFate {
    let word = lemma.word.trim();
    if word.is_empty() {
        return RawFate::Skipped;
    }
    // Any generated-page membership (including an ambiguous homograph/sense
    // key) means this attestation is already represented. Link ambiguity must
    // not create a duplicate raw page.
    if xref.contains(&lemma.lang, word) {
        return RawFate::DedupedXref;
    }
    // Same call as the render loop's display headword, by construction —
    // dedup and display must never diverge (issue #62).
    let display = crate::flavorize::flavorize_word(&lemma.lang, &lemma.pos, word);
    let disp_fold = crate::orthography::to_standard(&display.to_lowercase());
    if disp_fold.is_empty() {
        return RawFate::Skipped;
    }
    // ě-tolerant dedup (spec §6): flavorization can over-mark ě relative to
    // the official jat (ru день→děnj vs official denj), so the official
    // collision check tries both the fold and its ě→e variant, and raw-vs-raw
    // dedup keys on the ě-blind fold (cs město and sr mesto = one page).
    let efold = disp_fold.replace('ě', "e");
    if let Some(target) = isv_to_id
        .resolve(&display)
        .or_else(|| isv_to_id.resolve_fold(&efold))
    {
        return RawFate::DedupedFold { target };
    }
    if !raw_covered.insert(efold.clone()) {
        return RawFate::DedupedRawTwin { efold };
    }
    RawFate::Rendered { efold }
}

/// The raw pre-pass result (issue #64): every rendered raw lemma with its
/// pre-assigned entry id, plus a cross-reference from EVERY raw `(lang, word)`
/// to the internal page that shows it — its own raw page, the official /
/// generated page its display fold collided with, or the earlier raw twin
/// that claimed the same fold. Built before any page renders so word chips on
/// every page (including raw pages rendered early in the loop) can link
/// internally.
#[derive(Default)]
pub(super) struct RawPlan {
    /// (index into the raw corpus's lemma list, assigned entry id).
    pub(super) pages: Vec<(usize, usize)>,
    /// (lang, verbatim attested word) → internal entry id. Consulted by word
    /// chips AFTER the cognate `xref` (which resolves generated membership).
    pub(super) xref: crate::enrich::Xref,
    pub(super) deduped: usize,
    /// Raw-collision display credit (issue #86 item 6): target entry id →
    /// the raw `(lang, word)` attestations whose display fold deduped onto
    /// that page (RawFate::DedupedFold — the site knew them but showed them
    /// nowhere). Sorted lang-then-word, deduped. DISPLAY ONLY: never counted
    /// in n_langs / Dokaz / razumlivost / the vote — raw evidence stays
    /// benchmark-forbidden by type. (DedupedXref attestations are already
    /// visible as cognate members on their page and are NOT repeated here.)
    pub(super) credit: std::collections::BTreeMap<usize, Vec<(String, String)>>,
}

/// Classify every raw lemma once (via [`raw_lemma_fate`] — still the single
/// dedup rule shared with `coverage`), assigning sequential ids from
/// `next_id + 1` to the rendered ones in corpus order — the same ids the old
/// in-loop allocation produced.
pub(super) fn plan_raw_pages(
    lemmas: &[crate::dump::RawSlavicLemma],
    xref: &crate::enrich::Xref,
    isv_to_id: &HeadwordIndex,
    mut next_id: usize,
) -> RawPlan {
    let mut plan = RawPlan::default();
    let mut raw_covered: std::collections::HashSet<String> = std::collections::HashSet::new();
    // ě-blind fold → the raw page id that claimed it (for twin resolution).
    let mut fold_owner: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, lemma) in lemmas.iter().enumerate() {
        match raw_lemma_fate(lemma, xref, isv_to_id, &mut raw_covered) {
            RawFate::Rendered { efold } => {
                next_id += 1;
                plan.pages.push((i, next_id));
                plan.xref.insert(&lemma.lang, lemma.word.trim(), next_id);
                fold_owner.insert(efold, next_id);
            }
            RawFate::DedupedFold { target } => {
                plan.deduped += 1;
                plan.xref.insert(&lemma.lang, lemma.word.trim(), target);
                plan.credit
                    .entry(target)
                    .or_default()
                    .push((lemma.lang.clone(), lemma.word.trim().to_string()));
            }
            RawFate::DedupedRawTwin { efold } => {
                plan.deduped += 1;
                if let Some(&owner) = fold_owner.get(&efold) {
                    plan.xref.insert(&lemma.lang, lemma.word.trim(), owner);
                }
            }
            RawFate::DedupedXref | RawFate::Skipped => plan.deduped += 1,
        }
    }
    // Deterministic credit rows: lang then word, duplicates collapsed (a raw
    // corpus can carry the same (lang, word) under several POS sections).
    for rows in plan.credit.values_mut() {
        rows.sort();
        rows.dedup();
    }
    plan
}

/// Compute the raw-lemma coverage report and write it to `out` as both
/// `raw-coverage.md` (human) and `raw-coverage.json` (machine). Reconciles the
/// extraction tally, the site render/dedup split, and the native-join rate.
pub fn run_coverage(out: &Path) -> Result<()> {
    let raw_path = Path::new(crate::DEFAULT_RAW_LEMMA_CACHE);
    let raw_corpus = crate::dump::RawSlavicCorpus::load(raw_path).map_err(|e| {
        anyhow::anyhow!(
            "coverage needs the raw cache {} — run `extract-raw-slavic` first ({e})",
            raw_path.display()
        )
    })?;
    let cov_stats_path = raw_path.with_file_name(crate::dump::RAW_COVERAGE_FILE);
    let cov_stats =
        crate::dump::load_optional(&cov_stats_path, crate::dump::RawCoverageStats::load)?;
    if cov_stats.is_none() {
        println!(
            "(no {} — re-run `extract-raw-slavic` to regenerate the extraction tally)",
            cov_stats_path.display()
        );
    }

    let corpus = crate::dump::LemmaCorpus::load(Path::new(crate::DEFAULT_LEMMA_CACHE))?;
    let official_entries = official::load(Path::new(crate::DEFAULT_OFFICIAL))?;
    let enrich = crate::dump::load_optional(
        Path::new(crate::DEFAULT_ENRICH_CACHE),
        crate::enrich::EnrichIndex::load,
    )?;

    // --- View 1: totals by language and POS over the kept raw lemmas ---
    let total = raw_corpus.lemmas.len();
    let mut by_lang: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_pos: BTreeMap<String, usize> = BTreeMap::new();
    for l in &raw_corpus.lemmas {
        *by_lang.entry(l.lang.clone()).or_default() += 1;
        *by_pos.entry(l.pos.clone()).or_default() += 1;
    }

    // --- View 2: replicate the export dedup to split kept → rendered vs deduped ---
    let (xref, isv_to_id, generated_pages, official_only_pages) =
        build_corpus_render_index(&corpus, &official_entries);
    let mut raw_covered: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut rendered = 0usize;
    let mut deduped = 0usize;
    let mut rendered_by_lang: BTreeMap<String, usize> = BTreeMap::new();
    let mut deduped_by_lang: BTreeMap<String, usize> = BTreeMap::new();
    // Flavorization residue over the rendered set (spec §2 stage 5 / #62):
    // rendered headwords whose letters fall outside the ISV alphabet.
    let mut flavor_residue_words = 0usize;
    let mut flavor_residue: BTreeMap<char, usize> = BTreeMap::new();
    for lemma in &raw_corpus.lemmas {
        match raw_lemma_fate(lemma, &xref, &isv_to_id, &mut raw_covered) {
            RawFate::Rendered { .. } => {
                rendered += 1;
                *rendered_by_lang.entry(lemma.lang.clone()).or_default() += 1;
                let display =
                    crate::flavorize::flavorize_word(&lemma.lang, &lemma.pos, lemma.word.trim());
                let mut had_residue = false;
                for c in crate::flavorize::residue_chars(&display) {
                    *flavor_residue.entry(c).or_default() += 1;
                    had_residue = true;
                }
                if had_residue {
                    flavor_residue_words += 1;
                }
            }
            _ => {
                deduped += 1;
                *deduped_by_lang.entry(lemma.lang.clone()).or_default() += 1;
            }
        }
    }

    // --- View 3: native ru/pl/cs enrichment hit rate, by language ---
    let mut native_hits: BTreeMap<String, usize> = BTreeMap::new();
    if let Some(en) = &enrich {
        for l in &raw_corpus.lemmas {
            if en.get(&l.lang, &l.word).is_some() {
                *native_hits.entry(l.lang.clone()).or_default() += 1;
            }
        }
    }
    let native_total: usize = native_hits.values().sum();

    // --- Provenance: the datasets that fed the raw path, with paths + sizes ---
    let file_line = |label: &str, path: &Path| -> String {
        match std::fs::metadata(path) {
            Ok(m) => format!("- {label}: `{}` ({})", path.display(), fmt_bytes(m.len())),
            Err(_) => format!("- {label}: `{}` (not present)", path.display()),
        }
    };
    let dump_path = cov_stats
        .as_ref()
        .map(|s| s.source.clone())
        .unwrap_or_else(|| crate::DEFAULT_DUMP.to_string());
    let mut provenance = vec![
        format!(
            "- English Wiktextract raw dump (single-token content-word gate): `{}`{}",
            dump_path,
            match std::fs::metadata(&dump_path) {
                Ok(m) => format!(" ({})", fmt_bytes(m.len())),
                Err(_) => " (not present here — the 22 GB source is streamed once)".to_string(),
            }
        ),
        file_line(
            "Derived raw lemma cache",
            Path::new(crate::DEFAULT_RAW_LEMMA_CACHE),
        ),
        file_line("Extraction coverage tally", &cov_stats_path),
        file_line(
            "Native ru/pl/cs Wiktionary enrichment cache",
            Path::new(crate::DEFAULT_ENRICH_CACHE),
        ),
    ];
    provenance.push(format!(
        "- Native editions merged: {} (ru = Russian, pl = Polish, cs = Czech Wiktionary)",
        crate::enrich::ENRICH_LANGS.join(", ")
    ));

    // --- Reconciliation checks ---
    let kept = total; // the cache is exactly the kept set
    let render_reconciles = rendered + deduped == kept;
    let extract_reconciles = cov_stats
        .as_ref()
        .map(|s| s.kept as usize == kept && s.kept + s.dropped_total() == s.slavic_pages_seen)
        .unwrap_or(false);

    std::fs::create_dir_all(out)?;

    // ---- Machine-readable JSON ----
    let report_json = coverage_report_json(CoverageReportInput {
        raw_corpus: &raw_corpus,
        coverage_stats: cov_stats.as_ref(),
        by_language: &by_lang,
        by_pos: &by_pos,
        rendered,
        deduped,
        rendered_by_language: &rendered_by_lang,
        generated_pages,
        official_only_pages,
        native_hits: &native_hits,
        native_total,
        flavor_residue_words,
        flavor_residue: &flavor_residue,
    });
    std::fs::write(out.join("raw-coverage.json"), report_json)?;

    // ---- Human-readable Markdown ----
    let mut md = String::new();
    let _ = writeln!(md, "# Raw Slavic-lemma coverage (issue #35)\n");
    let _ = writeln!(
        md,
        "Auditable account of the raw Slavic-Wiktionary datasets: what went in, how \
         many words were included, and how many were excluded and why. Deterministic; \
         regenerate with `coverage` after `extract-raw-slavic` + `export`.\n"
    );

    let _ = writeln!(md, "## Datasets used (provenance)\n");
    for line in &provenance {
        let _ = writeln!(md, "{line}");
    }

    let _ = writeln!(md, "\n## 1. Extraction coverage (English dump)\n");
    if let Some(s) = &cov_stats {
        let _ = writeln!(md, "Streamed {} dump lines.\n", s.lines_scanned);
        let _ = writeln!(md, "| Outcome | Pages | Share of Slavic pages |");
        let _ = writeln!(md, "|---|--:|--:|");
        let seen = s.slavic_pages_seen.max(1);
        let pct = |x: u64| format!("{:.2}%", 100.0 * x as f64 / seen as f64);
        let _ = writeln!(
            md,
            "| Slavic pages seen | {} | 100.00% |",
            s.slavic_pages_seen
        );
        let _ = writeln!(md, "| **KEPT** | {} | {} |", s.kept, pct(s.kept));
        let _ = writeln!(
            md,
            "| dropped — redirect (no senses) | {} | {} |",
            s.dropped_redirect_no_senses,
            pct(s.dropped_redirect_no_senses)
        );
        let _ = writeln!(
            md,
            "| dropped — multiword / empty | {} | {} |",
            s.dropped_multiword,
            pct(s.dropped_multiword)
        );
        let _ = writeln!(
            md,
            "| dropped — non-content POS | {} | {} |",
            s.dropped_non_content_pos,
            pct(s.dropped_non_content_pos)
        );
        let _ = writeln!(
            md,
            "| dropped — no real gloss | {} | {} |",
            s.dropped_no_real_gloss,
            pct(s.dropped_no_real_gloss)
        );
        let _ = writeln!(
            md,
            "\nReconciles: kept ({}) + dropped ({}) = slavic pages seen ({}) → **{}**.",
            s.kept,
            s.dropped_total(),
            s.slavic_pages_seen,
            if extract_reconciles { "OK" } else { "MISMATCH" }
        );
    } else {
        let _ = writeln!(
            md,
            "_Extraction tally unavailable ({} missing); re-run `extract-raw-slavic`._",
            cov_stats_path.display()
        );
    }

    let _ = writeln!(md, "\n## 2. Kept raw lemmas by language\n");
    let _ = writeln!(md, "| Lang | Kept | Rendered raw | Deduped | Native join |");
    let _ = writeln!(md, "|---|--:|--:|--:|--:|");
    for (lang, n_lang) in &by_lang {
        let r = rendered_by_lang.get(lang).copied().unwrap_or(0);
        let d = deduped_by_lang.get(lang).copied().unwrap_or(0);
        let h = native_hits.get(lang).copied().unwrap_or(0);
        let hp = if *n_lang > 0 {
            format!("{:.1}%", 100.0 * h as f64 / *n_lang as f64)
        } else {
            "0.0%".to_string()
        };
        let _ = writeln!(md, "| {lang} | {n_lang} | {r} | {d} | {h} ({hp}) |");
    }
    let _ = writeln!(
        md,
        "| **total** | **{total}** | **{rendered}** | **{deduped}** | **{native_total}** |"
    );

    let _ = writeln!(md, "\n## 3. Kept raw lemmas by part of speech\n");
    let _ = writeln!(md, "| POS | Kept |");
    let _ = writeln!(md, "|---|--:|");
    for (pos, n_pos) in &by_pos {
        let _ = writeln!(md, "| {pos} | {n_pos} |");
    }

    let _ = writeln!(md, "\n## 4. Site rendering (replicated export dedup)\n");
    let _ = writeln!(
        md,
        "Of the {total} kept raw lemmas, the site renders **{rendered}** as raw-only \
         attestation pages and dedups **{deduped}** against an existing official / \
         generated / official-only headword (verbatim `(lang, word)` cognate match, or \
         the display-headword fold already claimed)."
    );
    let _ = writeln!(
        md,
        "\nFor context, the same export renders {generated_pages} generated cognate \
         pages and {official_only_pages} official-only pages.\n"
    );
    let _ = writeln!(
        md,
        "Reconciles: rendered ({rendered}) + deduped ({deduped}) = kept ({kept}) → **{}**.",
        if render_reconciles { "OK" } else { "MISMATCH" }
    );
    let _ = writeln!(
        md,
        "\n> Correctness check: this `rendered-raw` count must equal the number of \
         distinct `R`-status rows across a fresh `export`'s `search/` shards \
         (rows are deliberately listed in several shards — dedup by id)."
    );
    let _ = writeln!(
        md,
        "\nDisplay headwords are flavorized into ISV orthography \
         (`data/FLAVORIZATION_SPEC.md`, issue #62). Validation residue: \
         **{flavor_residue_words}** of {rendered} rendered headwords carry a letter \
         outside the ISV standard alphabet{}.",
        if flavor_residue.is_empty() {
            String::new()
        } else {
            format!(
                " (by letter: {})",
                flavor_residue
                    .iter()
                    .map(|(c, n)| format!("{c}×{n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );

    let _ = writeln!(md, "\n## 5. Native-join (ru/pl/cs enrichment) hit rate\n");
    if enrich.is_some() {
        let _ = writeln!(
            md,
            "{native_total} of {total} raw lemmas ({:.1}%) gain a native ru/pl/cs \
             Wiktionary enrichment match. By language:\n",
            if total > 0 {
                100.0 * native_total as f64 / total as f64
            } else {
                0.0
            }
        );
        let _ = writeln!(md, "| Lang | Kept | Native hits | Rate |");
        let _ = writeln!(md, "|---|--:|--:|--:|");
        for (lang, n_lang) in &by_lang {
            let h = native_hits.get(lang).copied().unwrap_or(0);
            let hp = if *n_lang > 0 {
                format!("{:.1}%", 100.0 * h as f64 / *n_lang as f64)
            } else {
                "0.0%".to_string()
            };
            let _ = writeln!(md, "| {lang} | {n_lang} | {h} | {hp} |");
        }
    } else {
        let _ = writeln!(
            md,
            "_Enrichment cache unavailable ({}); run `extract-enrich`._",
            crate::DEFAULT_ENRICH_CACHE
        );
    }

    std::fs::write(out.join("raw-coverage.md"), &md)?;

    println!(
        "coverage: {total} kept raw lemmas across {} languages → {rendered} rendered raw / {deduped} deduped (reconcile: {}).",
        by_lang.len(),
        if render_reconciles { "OK" } else { "MISMATCH" }
    );
    if let Some(s) = &cov_stats {
        println!(
            "extraction: {} slavic pages seen → {} kept + {} dropped (redirect {} / multiword {} / non-content-pos {} / no-gloss {}); reconcile: {}.",
            s.slavic_pages_seen,
            s.kept,
            s.dropped_total(),
            s.dropped_redirect_no_senses,
            s.dropped_multiword,
            s.dropped_non_content_pos,
            s.dropped_no_real_gloss,
            if extract_reconciles { "OK" } else { "MISMATCH" }
        );
    }
    println!("native-join: {native_total}/{total} raw lemmas matched ru/pl/cs enrichment.");
    println!(
        "wrote {} and {}",
        out.join("raw-coverage.md").display(),
        out.join("raw-coverage.json").display()
    );
    if !render_reconciles || (cov_stats.is_some() && !extract_reconciles) {
        anyhow::bail!("coverage reconciliation FAILED — see report");
    }
    Ok(())
}

/// Human byte size for provenance lines.
pub(super) fn fmt_bytes(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

pub(super) struct CoverageReportInput<'a> {
    pub(super) raw_corpus: &'a crate::dump::RawSlavicCorpus,
    pub(super) coverage_stats: Option<&'a crate::dump::RawCoverageStats>,
    pub(super) by_language: &'a BTreeMap<String, usize>,
    pub(super) by_pos: &'a BTreeMap<String, usize>,
    pub(super) rendered: usize,
    pub(super) deduped: usize,
    pub(super) rendered_by_language: &'a BTreeMap<String, usize>,
    pub(super) generated_pages: usize,
    pub(super) official_only_pages: usize,
    pub(super) native_hits: &'a BTreeMap<String, usize>,
    pub(super) native_total: usize,
    pub(super) flavor_residue_words: usize,
    pub(super) flavor_residue: &'a BTreeMap<char, usize>,
}

/// Machine-readable coverage report (stable key order via serde_json::json!).
pub(super) fn coverage_report_json(input: CoverageReportInput<'_>) -> Vec<u8> {
    let CoverageReportInput {
        raw_corpus,
        coverage_stats: cov_stats,
        by_language: by_lang,
        by_pos,
        rendered,
        deduped,
        rendered_by_language: rendered_by_lang,
        generated_pages,
        official_only_pages,
        native_hits,
        native_total,
        flavor_residue_words,
        flavor_residue,
    } = input;
    let total = raw_corpus.lemmas.len();
    let extraction = cov_stats.map(|s| {
        serde_json::json!({
            "source": s.source,
            "lines_scanned": s.lines_scanned,
            "slavic_pages_seen": s.slavic_pages_seen,
            "kept": s.kept,
            "dropped": {
                "redirect_no_senses": s.dropped_redirect_no_senses,
                "multiword": s.dropped_multiword,
                "non_content_pos": s.dropped_non_content_pos,
                "no_real_gloss": s.dropped_no_real_gloss,
                "total": s.dropped_total(),
            },
            "kept_by_lang": s.kept_by_lang,
            "reconciles": s.kept as usize == total
                && s.kept + s.dropped_total() == s.slavic_pages_seen,
        })
    });
    let report = serde_json::json!({
        "kept_total": total,
        "kept_by_lang": by_lang,
        "kept_by_pos": by_pos,
        "site": {
            "rendered_raw": rendered,
            "deduped": deduped,
            "rendered_by_lang": rendered_by_lang,
            "generated_pages": generated_pages,
            "official_only_pages": official_only_pages,
            "reconciles": rendered + deduped == total,
            "flavorize_residue": {
                "words": flavor_residue_words,
                "by_letter": flavor_residue
                    .iter()
                    .map(|(c, n)| (c.to_string(), *n))
                    .collect::<BTreeMap<String, usize>>(),
            },
        },
        "native_join": {
            "total_hits": native_total,
            "by_lang": native_hits,
            "langs": crate::enrich::ENRICH_LANGS,
        },
        "extraction": extraction,
    });
    let mut v = serde_json::to_vec_pretty(&report).unwrap_or_default();
    v.push(b'\n');
    v
}

/// Inject `generated` derivative FormRecords off attested bases (issue #37) into
/// both sinks. Each base's regular family is derived on the attested-base path;
/// a derivative is kept ONLY if its folded key is absent from `taken` — the set
/// of every key already in the form index (official / official-only lemmas AND
/// their inflected forms, generated reconstructions, and derivatives added
/// earlier in this pass). That guarantees no derivative collides with an
/// attested `form_key`. Survivors ship as `status = "generated"`,
/// `source = "lemma"`, probability from the pattern's leakage-free holdout
/// precision, provenance = `deriv:<pattern>` (in analyses) + the base entry id —
/// and deliberately NO inflected paradigm (an inflected form of a proposed
/// derivative would be confidently wrong). Returns the count added; pure and
/// order-deterministic, so the export stays byte-reproducible.
pub(super) fn inject_generated_derivatives(
    form_sink: &mut crate::forms::RecordSink,
    lemma_sink: &mut crate::forms::RecordSink,
    taken: &mut std::collections::HashSet<String>,
    bases: &[(String, crate::model::Pos, usize, String)],
    probs: &crate::derive::DerivationProbabilities,
) -> usize {
    let mut added = 0usize;
    for (base, pos, base_id, base_gloss) in bases {
        for d in crate::derive::derive_family(base, *pos) {
            let form = d.form.trim();
            if form.is_empty() || form.contains(' ') || form.contains(['!', '#']) {
                continue;
            }
            let key = crate::forms::form_key(form);
            // Absent-only + dedup in one step: skip when the key is already
            // taken (attested form, prior lemma, or an earlier derivative).
            if key.is_empty() || !taken.insert(key) {
                continue;
            }
            let prob = probs.probability(d.pattern);
            let gloss = format!("{} ← {} ({})", d.label, base, truncate(base_gloss, 50));
            let feats = format!("deriv:{}", d.pattern);
            for sink in [&mut *lemma_sink, &mut *form_sink] {
                sink.add(
                    form,
                    &feats,
                    form,
                    *base_id,
                    d.pos.code(),
                    "lemma",
                    "generated",
                    Some(prob),
                    &gloss,
                );
            }
            added += 1;
        }
    }
    added
}
