//! Reproducible benchmark of the candidate generator against the official
//! Interslavic dictionary.
//!
//! For every benchmarkable official entry we hand the generator only the modern
//! Slavic cognates (never the answer), ask it to reconstruct the Interslavic
//! lemma, and compare against the official `isv`. We run an *ablation ladder* —
//! baseline, then each linguistic rule switched on cumulatively — so the
//! measured effect of every change is attributable. All metrics and the
//! regression/improvement diffs are written under `target/eval/`.

use crate::consensus::{self, ConsensusConfig, MeaningInput, SourceForm};
use crate::model::{Candidate, Confidence, Pos};
use crate::official::{self, OfficialEntry};
use crate::orthography as ortho;
use anyhow::Result;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// One rung of the ablation ladder.
struct Rung {
    name: &'static str,
    description: &'static str,
    cfg: ConsensusConfig,
}

fn ladder() -> Vec<Rung> {
    // Each rung adds exactly one linguistic capability on top of the previous, so
    // the benchmark attributes the accuracy delta of every rule. Ordered by the
    // spec's expected value (§6.1).
    let base = ConsensusConfig::baseline();
    let mut branch = base;
    branch.branch_balanced = true;
    branch.prefer_south_representative = true;
    let mut six = branch;
    six.six_subgroup_vote = true;
    let mut endings = six;
    endings.lemma_endings = true;
    let mut intl = endings;
    intl.internationalism = true;
    let mut prefix = intl;
    prefix.prefix_normalization = true;
    let mut deple = prefix;
    deple.depleophony = true;
    let mut nasal = deple;
    nasal.nasal_from_polish = true;
    let mut palatal = nasal;
    palatal.palatal_from_south = true;
    let mut jat = palatal;
    jat.jat_reconstruction = true;

    vec![
        Rung { name: "baseline", description: "Transliterate the first available form; no branch balancing, no repairs (the original prototype behavior).", cfg: base },
        Rung { name: "+branch-consensus", description: "Branch-balanced skeleton vote + South-Slavic representative.", cfg: branch },
        Rung { name: "+six-subgroup", description: "Six dialect-subgroup vote with population tie-break (§4.1).", cfg: six },
        Rung { name: "+lemma-endings", description: "Native POS lemma endings: noun nom.sg, adj -y/-i, verb -ti (§3).", cfg: endings },
        Rung { name: "+internationalism", description: "Internationalism ending table: -izm/-cija/-ičny/-alny/-ovati (§5.2).", cfg: intl },
        Rung { name: "+prefixes", description: "Normalize verbal/nominal prefixes råz-/prěd- (§2).", cfg: prefix },
        Rung { name: "+depleophony", description: "Undo East-Slavic pleophony / liquid metathesis (§2).", cfg: deple },
        Rung { name: "+nasals", description: "Recover ę/ų nasal vowels from Polish (§2 Phase C).", cfg: nasal },
        Rung { name: "+palatals", description: "Recover ć/đ (*tj/*dj) from South Slavic (§2 Phase B).", cfg: palatal },
        Rung { name: "+jat (full)", description: "Reconstruct jat ě from the cross-branch reflex (§2 Phase D).", cfg: jat },
    ]
}

#[derive(Clone)]
struct EntryResult {
    id: String,
    isv: String,
    gloss: String,
    pos: Pos,
    predicted: String,
    exact: bool,
    normalized: bool,
    skeleton: bool,
    top3: bool,
    top5: bool,
    norm_edit: f32,
    branch_cov: usize,
    confidence: Option<Confidence>,
    n_langs: usize,
}

#[derive(Default, Clone)]
struct Bucket {
    n: usize,
    exact: usize,
    normalized: usize,
}
impl Bucket {
    fn add(&mut self, r: &EntryResult) {
        self.n += 1;
        self.exact += r.exact as usize;
        self.normalized += r.normalized as usize;
    }
    fn rate(hits: usize, n: usize) -> f32 {
        if n == 0 {
            0.0
        } else {
            hits as f32 / n as f32
        }
    }
}

struct RunMetrics {
    name: String,
    description: String,
    n: usize,
    exact: usize,
    normalized: usize,
    skeleton: usize,
    top3: usize,
    top5: usize,
    sum_norm_edit: f32,
    by_pos: BTreeMap<&'static str, Bucket>,
    by_branch: [Bucket; 4],
    by_conf: BTreeMap<&'static str, Bucket>,
    results: Vec<EntryResult>,
}

fn branch_cov_of(input: &MeaningInput) -> usize {
    let mut branches = Vec::new();
    for f in &input.forms {
        if f.modern && !branches.contains(&f.branch) {
            branches.push(f.branch);
        }
    }
    branches.len()
}

fn build_input(entry: &OfficialEntry) -> MeaningInput {
    let forms: Vec<SourceForm> = consensus::source_forms_from_cells(&entry.cells, |code, form| {
        format!(
            "https://en.wiktionary.org/wiki/{}#{}",
            form.replace(' ', "_"),
            code
        )
    });
    MeaningInput {
        pos: entry.pos,
        gender: entry.noun_traits.gender,
        gloss: entry.english.clone(),
        forms,
    }
}

fn evaluate_config(entries: &[OfficialEntry], rung: &Rung) -> RunMetrics {
    let mut m = RunMetrics {
        name: rung.name.to_string(),
        description: rung.description.to_string(),
        n: 0,
        exact: 0,
        normalized: 0,
        skeleton: 0,
        top3: 0,
        top5: 0,
        sum_norm_edit: 0.0,
        by_pos: BTreeMap::new(),
        by_branch: Default::default(),
        by_conf: BTreeMap::new(),
        results: Vec::new(),
    };

    for entry in entries {
        let input = build_input(entry);
        // Need at least one modern cognate to have anything to reconstruct from.
        if !input.forms.iter().any(|f| f.modern) {
            continue;
        }
        let cands: Vec<Candidate> = consensus::generate(&input, &rung.cfg);
        let top = cands.first();
        let predicted = top.map(|c| c.form.clone()).unwrap_or_default();
        let confidence = top.map(|c| c.confidence);
        let top_branch_cov = top.map(|c| c.branch_coverage as usize).unwrap_or(0);

        let exact = ortho::exact_match(&predicted, &entry.isv);
        let normalized = ortho::normalized_match(&predicted, &entry.isv);
        let skeleton = ortho::skeleton_match(&predicted, &entry.isv);
        let top3 = cands
            .iter()
            .take(3)
            .any(|c| ortho::normalized_match(&c.form, &entry.isv));
        let top5 = cands
            .iter()
            .take(5)
            .any(|c| ortho::normalized_match(&c.form, &entry.isv));
        let norm_edit = ortho::normalized_edit_distance(&predicted, &entry.isv);
        let branch_cov = top_branch_cov;
        let n_langs = input.forms.iter().filter(|f| f.modern).count();

        let r = EntryResult {
            id: entry.id.clone(),
            isv: entry.isv.clone(),
            gloss: entry.english.clone(),
            pos: entry.pos,
            predicted,
            exact,
            normalized,
            skeleton,
            top3,
            top5,
            norm_edit,
            branch_cov,
            confidence,
            n_langs,
        };

        m.n += 1;
        m.exact += exact as usize;
        m.normalized += normalized as usize;
        m.skeleton += skeleton as usize;
        m.top3 += top3 as usize;
        m.top5 += top5 as usize;
        m.sum_norm_edit += norm_edit;
        m.by_pos.entry(r.pos.code()).or_default().add(&r);
        m.by_branch[branch_cov.min(3)].add(&r);
        if let Some(c) = confidence {
            m.by_conf.entry(conf_label(c)).or_default().add(&r);
        }
        m.results.push(r);
    }
    m
}

fn conf_label(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

pub fn run(official_path: &Path, _dump: Option<&Path>, out_dir: &Path) -> Result<()> {
    let mut entries_all = official::load(official_path)?;
    // The metadata TSV has no per-language translations, so the consensus
    // benchmark is impossible from it. Fall back to the bundled full export.
    let with_cells = entries_all.iter().filter(|e| !e.cells.is_empty()).count();
    if with_cells < 100 {
        let fallback = Path::new(crate::DEFAULT_OFFICIAL);
        if fallback != official_path && fallback.exists() {
            eprintln!(
                "note: {} has no per-language translations; using {} for the consensus benchmark.",
                official_path.display(),
                fallback.display()
            );
            entries_all = official::load(fallback)?;
        }
    }
    let entries: Vec<OfficialEntry> = entries_all
        .into_iter()
        .filter(|e| e.is_benchmarkable())
        .collect();
    println!(
        "Loaded {} benchmarkable official entries from {}",
        entries.len(),
        official_path.display()
    );

    let rungs = ladder();
    let runs: Vec<RunMetrics> = rungs.iter().map(|r| evaluate_config(&entries, r)).collect();

    for r in &runs {
        println!(
            "  {:<18} exact {:>6.2}%  norm {:>6.2}%  top3 {:>6.2}%  skel {:>6.2}%  edit {:.3}",
            r.name,
            100.0 * Bucket::rate(r.exact, r.n),
            100.0 * Bucket::rate(r.normalized, r.n),
            100.0 * Bucket::rate(r.top3, r.n),
            100.0 * Bucket::rate(r.skeleton, r.n),
            r.sum_norm_edit / r.n.max(1) as f32,
        );
    }

    std::fs::create_dir_all(out_dir)?;
    let baseline = &runs[0];
    // "Keep only if it improves": the final kept config is the empirically best
    // cumulative point (by exact top-1, then normalized), NOT simply the last
    // rung — so rules that regress accuracy are shown but never chosen.
    let best_idx = (0..runs.len())
        .max_by(|&a, &b| {
            let ra = &runs[a];
            let rb = &runs[b];
            Bucket::rate(ra.exact, ra.n)
                .total_cmp(&Bucket::rate(rb.exact, rb.n))
                .then(Bucket::rate(ra.normalized, ra.n).total_cmp(&Bucket::rate(rb.normalized, rb.n)))
        })
        .unwrap();
    let best = &runs[best_idx];
    println!("Kept config (best of ladder): {}", best.name);

    write_summary_json(out_dir, &runs)?;
    write_report_md(out_dir, &runs, best)?;
    write_diffs(out_dir, baseline, best)?;
    write_errors_sample(out_dir, best)?;

    println!("Wrote benchmark report to {}", out_dir.display());
    println!(
        "Headline: normalized top-1 {:.2}% (baseline {:.2}%), exact top-1 {:.2}% (baseline {:.2}%)",
        100.0 * Bucket::rate(best.normalized, best.n),
        100.0 * Bucket::rate(baseline.normalized, baseline.n),
        100.0 * Bucket::rate(best.exact, best.n),
        100.0 * Bucket::rate(baseline.exact, baseline.n),
    );
    Ok(())
}

fn write_summary_json(out_dir: &Path, runs: &[RunMetrics]) -> Result<()> {
    let mut arr = Vec::new();
    for r in runs {
        let by_pos: BTreeMap<String, serde_json::Value> = r
            .by_pos
            .iter()
            .map(|(k, b)| {
                (
                    k.to_string(),
                    serde_json::json!({
                        "n": b.n,
                        "exact": Bucket::rate(b.exact, b.n),
                        "normalized": Bucket::rate(b.normalized, b.n),
                    }),
                )
            })
            .collect();
        let by_branch: Vec<serde_json::Value> = r
            .by_branch
            .iter()
            .enumerate()
            .map(|(i, b)| {
                serde_json::json!({
                    "branch_coverage": i,
                    "n": b.n,
                    "exact": Bucket::rate(b.exact, b.n),
                    "normalized": Bucket::rate(b.normalized, b.n),
                })
            })
            .collect();
        let by_conf: BTreeMap<String, serde_json::Value> = r
            .by_conf
            .iter()
            .map(|(k, b)| {
                (
                    k.to_string(),
                    serde_json::json!({
                        "n": b.n,
                        "normalized": Bucket::rate(b.normalized, b.n),
                    }),
                )
            })
            .collect();
        arr.push(serde_json::json!({
            "name": r.name,
            "description": r.description,
            "n": r.n,
            "exact_top1": Bucket::rate(r.exact, r.n),
            "normalized_top1": Bucket::rate(r.normalized, r.n),
            "skeleton_top1": Bucket::rate(r.skeleton, r.n),
            "normalized_top3": Bucket::rate(r.top3, r.n),
            "normalized_top5": Bucket::rate(r.top5, r.n),
            "mean_normalized_edit_distance": r.sum_norm_edit / r.n.max(1) as f32,
            "by_pos": by_pos,
            "by_branch_coverage": by_branch,
            "by_confidence": by_conf,
        }));
    }
    let doc = serde_json::json!({ "runs": arr });
    std::fs::write(
        out_dir.join("candidate-generation-summary.json"),
        serde_json::to_string_pretty(&doc)?,
    )?;
    Ok(())
}

fn write_report_md(out_dir: &Path, runs: &[RunMetrics], best: &RunMetrics) -> Result<()> {
    let baseline = &runs[0];
    let mut s = String::new();
    writeln!(s, "# Candidate-generation benchmark\n")?;
    writeln!(
        s,
        "Benchmark: reconstruct the official Interslavic lemma from the modern Slavic cognates in the official dictionary, without showing the generator the answer. Evaluated on **{}** benchmarkable single-word entries.\n",
        baseline.n
    )?;
    writeln!(s, "## Ablation ladder (each rule added cumulatively)\n")?;
    writeln!(
        s,
        "| Rung | exact top-1 | norm top-1 | Δ norm | top-3 | skeleton | mean edit |"
    )?;
    writeln!(s, "|---|---:|---:|---:|---:|---:|---:|")?;
    let mut prev_norm = Bucket::rate(baseline.normalized, baseline.n);
    for r in runs {
        let norm = Bucket::rate(r.normalized, r.n);
        let delta = norm - prev_norm;
        writeln!(
            s,
            "| {} | {:.2}% | {:.2}% | {:+.2} pp | {:.2}% | {:.2}% | {:.3} |",
            r.name,
            100.0 * Bucket::rate(r.exact, r.n),
            100.0 * norm,
            100.0 * delta,
            100.0 * Bucket::rate(r.top3, r.n),
            100.0 * Bucket::rate(r.skeleton, r.n),
            r.sum_norm_edit / r.n.max(1) as f32,
        )?;
        prev_norm = norm;
    }
    writeln!(s)?;
    for r in runs {
        writeln!(s, "- **{}** — {}", r.name, r.description)?;
    }

    writeln!(s, "\n## POS-specific accuracy (final config)\n")?;
    writeln!(s, "| POS | n | exact | normalized |")?;
    writeln!(s, "|---|---:|---:|---:|")?;
    for (pos, b) in &best.by_pos {
        writeln!(
            s,
            "| {} | {} | {:.2}% | {:.2}% |",
            pos,
            b.n,
            100.0 * Bucket::rate(b.exact, b.n),
            100.0 * Bucket::rate(b.normalized, b.n)
        )?;
    }

    writeln!(s, "\n## Branch coverage vs accuracy (final config)\n")?;
    writeln!(s, "| branches with the consensus form | n | normalized |")?;
    writeln!(s, "|---:|---:|---:|")?;
    for (i, b) in best.by_branch.iter().enumerate() {
        writeln!(
            s,
            "| {} | {} | {:.2}% |",
            i,
            b.n,
            100.0 * Bucket::rate(b.normalized, b.n)
        )?;
    }

    writeln!(s, "\n## Confidence calibration (final config)\n")?;
    writeln!(
        s,
        "High-confidence candidates should match the official dictionary more often than low-confidence ones.\n"
    )?;
    writeln!(s, "| confidence | n | normalized match |")?;
    writeln!(s, "|---|---:|---:|")?;
    for label in ["high", "medium", "low"] {
        if let Some(b) = best.by_conf.get(label) {
            writeln!(
                s,
                "| {} | {} | {:.2}% |",
                label,
                b.n,
                100.0 * Bucket::rate(b.normalized, b.n)
            )?;
        }
    }

    writeln!(s, "\n## Before / after\n")?;
    writeln!(
        s,
        "- Baseline normalized top-1: **{:.2}%**\n- Final normalized top-1: **{:.2}%** ({:+.2} pp)\n- Baseline exact top-1: **{:.2}%**\n- Final exact top-1: **{:.2}%** ({:+.2} pp)",
        100.0 * Bucket::rate(baseline.normalized, baseline.n),
        100.0 * Bucket::rate(best.normalized, best.n),
        100.0 * (Bucket::rate(best.normalized, best.n) - Bucket::rate(baseline.normalized, baseline.n)),
        100.0 * Bucket::rate(baseline.exact, baseline.n),
        100.0 * Bucket::rate(best.exact, best.n),
        100.0 * (Bucket::rate(best.exact, best.n) - Bucket::rate(baseline.exact, baseline.n)),
    )?;

    std::fs::write(out_dir.join("candidate-generation-report.md"), s)?;
    Ok(())
}

fn write_diffs(out_dir: &Path, baseline: &RunMetrics, best: &RunMetrics) -> Result<()> {
    let base_map: BTreeMap<&str, &EntryResult> =
        baseline.results.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut regressions = String::from("id,gloss,pos,official,baseline_pred,final_pred\n");
    let mut improvements = String::from("id,gloss,pos,official,baseline_pred,final_pred\n");
    for r in &best.results {
        let Some(b) = base_map.get(r.id.as_str()) else {
            continue;
        };
        if b.normalized && !r.normalized {
            writeln!(
                regressions,
                "{},{},{},{},{},{}",
                r.id,
                csv_escape(&r.gloss),
                r.pos.code(),
                csv_escape(&r.isv),
                csv_escape(&b.predicted),
                csv_escape(&r.predicted)
            )?;
        }
        if !b.normalized && r.normalized {
            writeln!(
                improvements,
                "{},{},{},{},{},{}",
                r.id,
                csv_escape(&r.gloss),
                r.pos.code(),
                csv_escape(&r.isv),
                csv_escape(&b.predicted),
                csv_escape(&r.predicted)
            )?;
        }
    }
    std::fs::write(out_dir.join("regressions.csv"), regressions)?;
    std::fs::write(out_dir.join("improvements.csv"), improvements)?;
    Ok(())
}

fn write_errors_sample(out_dir: &Path, best: &RunMetrics) -> Result<()> {
    let mut errors: Vec<&EntryResult> = best.results.iter().filter(|r| !r.normalized).collect();
    // Sort by closeness (largest edit distance last) so the sample surfaces the
    // near-misses first, which are the most actionable.
    errors.sort_by(|a, b| a.norm_edit.total_cmp(&b.norm_edit));
    let mut s = String::from("id,gloss,pos,official,predicted,norm_edit,branch_cov,n_langs\n");
    for r in errors.iter().take(400) {
        writeln!(
            s,
            "{},{},{},{},{},{:.3},{},{}",
            r.id,
            csv_escape(&r.gloss),
            r.pos.code(),
            csv_escape(&r.isv),
            csv_escape(&r.predicted),
            r.norm_edit,
            r.branch_cov,
            r.n_langs
        )?;
    }
    std::fs::write(out_dir.join("errors-sample.csv"), s)?;
    Ok(())
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
