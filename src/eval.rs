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

/// The cumulative ladder of *kept* rules — each one improved measured accuracy —
/// ending exactly at [`ConsensusConfig::production`].
fn kept_ladder() -> Vec<Rung> {
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
    let mut proto = nasal;
    proto.proto_derived_form = true;
    let mut intlpref = proto;
    intlpref.internationalism_preference = true;
    let mut adjfleet = intlpref;
    adjfleet.adj_fleeting_drop = true;

    vec![
        Rung { name: "baseline", description: "Transliterate the first available form; no branch balancing, no repairs (the original prototype behavior).", cfg: base },
        Rung { name: "+branch-consensus", description: "Branch-balanced skeleton vote + South-Slavic representative.", cfg: branch },
        Rung { name: "+six-subgroup", description: "Six dialect-subgroup vote with population tie-break (§4.1).", cfg: six },
        Rung { name: "+lemma-endings", description: "Native POS lemma endings: noun nom.sg, adj -y/-i, verb -ti (§3).", cfg: endings },
        Rung { name: "+internationalism", description: "Internationalism ending table: -izm/-cija/-ičny/-alny/-ovati (§5.2).", cfg: intl },
        Rung { name: "+prefixes", description: "Normalize verbal/nominal prefixes råz-/prěd- (§2).", cfg: prefix },
        Rung { name: "+depleophony", description: "Undo East-Slavic pleophony / liquid metathesis (§2).", cfg: deple },
        Rung { name: "+nasals", description: "Recover ę/ų nasal vowels from Polish (§2 Phase C).", cfg: nasal },
        Rung { name: "+proto-derived", description: "Two-stage §4.4: consensus picks the root, the Proto-Slavic rule engine supplies the flavored form (ě/ć/đ/å/ȯ/y) via a leakage-free descendant+gloss link. Requires the proto cache.", cfg: proto },
        Rung { name: "+intl-preference", description: "Prefer the internationalism cluster over native synonyms (ISV design criteria favor international roots for modern vocabulary): aeroplan over samolot.", cfg: intlpref },
        Rung { name: "+adj-fleeting (production)", description: "Drop a South-Slavic adjective's fleeting vowel before -y, gated on East/West consonant adjacency (dobar→dobry, zelen stays).", cfg: adjfleet },
    ]
}

/// Load the Proto-Slavic cache if it exists (else the proto-derived rung is a
/// no-op that equals the +nasals config).
fn load_proto_index() -> Option<crate::dump::ProtoIndex> {
    let path = Path::new(crate::DEFAULT_PROTO_CACHE);
    if !path.exists() {
        return None;
    }
    crate::dump::ProtoIndex::load(path).ok()
}

/// Rules that were tried and *rejected*: each is the production config plus one
/// experimental rule, so its (negative) delta is measured in isolation.
fn rejected_experiments() -> Vec<Rung> {
    let prod = ConsensusConfig::production();
    let mut palatal = prod;
    palatal.palatal_from_south = true;
    let mut jat = prod;
    jat.jat_reconstruction = true;
    let mut adjrep = prod;
    adjrep.adj_longform_rep = true;
    let mut yrec = prod;
    yrec.y_recovery = true;
    vec![
        Rung { name: "prod+palatals", description: "Recover ć/đ (*tj/*dj) from South Slavic — modern reflexes are too noisy; derive from Proto-Slavic instead.", cfg: palatal },
        Rung { name: "prod+jat", description: "Reconstruct jat ě from the cross-branch reflex — unreliable from modern reflexes.", cfg: jat },
        Rung { name: "prod+adj-longform", description: "Long-form (ru/pl/cs) adjective representative — East/West orthographic quirks outweigh the fleeting-vowel fix.", cfg: adjrep },
        Rung { name: "prod+y-recovery", description: "Recover *y from East/West where South merged *y→i — too aggressive, flips correct i→y.", cfg: yrec },
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
        is_intl_meaning: entry.genesis.trim() == "I",
    }
}

fn evaluate_config(
    entries: &[OfficialEntry],
    rung: &Rung,
    proto: Option<&crate::dump::ProtoIndex>,
) -> RunMetrics {
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
        let (cands, _recon): (Vec<Candidate>, _) =
            crate::pipeline::generate(&input, proto, &rung.cfg);
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

/// Data-quality / ceiling audit (§2/§6 of the V4 plan). For every benchmark miss
/// it asks: is the official root even present in the modern evidence (so better
/// cluster *selection* could fix it), was the right cluster chosen but the
/// surface/form wrong (engine error), or is the official root absent from the
/// evidence entirely (unfixable from the cognates we have)? Also reports the
/// cognate cohesion of each meaning. Uses `isv` only for this offline analysis —
/// never on the benchmark path.
pub fn run_audit(official_path: &Path, out_dir: &Path) -> Result<()> {
    let entries: Vec<OfficialEntry> = official::load(official_path)?
        .into_iter()
        .filter(|e| e.is_benchmarkable())
        .collect();
    let proto = load_proto_index();
    let cfg = ConsensusConfig::production();

    let (mut n, mut miss) = (0usize, 0usize);
    // miss classes
    let (mut wrong_cluster, mut right_cluster_wrong_form, mut root_absent) =
        (0usize, 0usize, 0usize);
    // cohesion: distinct consonant-keys among modern forms
    let mut cohesion_hist: BTreeMap<usize, usize> = BTreeMap::new();
    let mut miss_rows: Vec<String> = Vec::new();

    for entry in &entries {
        let input = build_input(entry);
        let modern: Vec<&crate::consensus::SourceForm> =
            input.forms.iter().filter(|f| f.modern).collect();
        if modern.is_empty() {
            continue;
        }
        n += 1;

        // Distinct cognate clusters (consonant-key) among the modern forms.
        let mut keys: Vec<String> = Vec::new();
        for f in &modern {
            let k = ortho::consonant_key(&f.norm.latin);
            if !k.is_empty() && !keys.contains(&k) {
                keys.push(k);
            }
        }
        *cohesion_hist.entry(keys.len()).or_default() += 1;

        let (cands, _) = crate::pipeline::generate(&input, proto.as_ref(), &cfg);
        let predicted = cands.first().map(|c| c.form.clone()).unwrap_or_default();
        if ortho::normalized_match(&predicted, &entry.isv) {
            continue;
        }
        miss += 1;

        let official_key = ortho::consonant_key(&ortho::to_standard(&entry.isv));
        let predicted_key = ortho::consonant_key(&predicted);
        let root_in_evidence = keys.iter().any(|k| k == &official_key);

        let class = if !root_in_evidence {
            root_absent += 1;
            "root-absent"
        } else if predicted_key == official_key {
            right_cluster_wrong_form += 1;
            "right-cluster-wrong-form"
        } else {
            wrong_cluster += 1;
            "wrong-cluster"
        };
        if miss_rows.len() < 500 {
            miss_rows.push(format!(
                "{},{},{},{},{},{}",
                csv_escape(&entry.english),
                entry.pos.code(),
                csv_escape(&entry.isv),
                csv_escape(&predicted),
                keys.len(),
                class
            ));
        }
    }

    let pct = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f32 / b as f32
        }
    };
    println!("Audit over {} benchmarkable meanings ({} misses):", n, miss);
    println!(
        "  miss classes: wrong-cluster {:.1}% | right-cluster-wrong-form {:.1}% | root-absent {:.1}%",
        pct(wrong_cluster, miss),
        pct(right_cluster_wrong_form, miss),
        pct(root_absent, miss),
    );
    println!("  → cluster-selection ceiling: fixing wrong-cluster misses could recover up to {:.1}% of all misses ({} entries)", pct(wrong_cluster, miss), wrong_cluster);
    let single = *cohesion_hist.get(&1).unwrap_or(&0);
    println!(
        "  cohesion: {:.1}% of meanings are a single cognate cluster; {:.1}% have >=3 clusters",
        pct(single, n),
        pct(
            cohesion_hist
                .iter()
                .filter(|(k, _)| **k >= 3)
                .map(|(_, v)| *v)
                .sum(),
            n
        ),
    );

    std::fs::create_dir_all(out_dir)?;
    let mut s = String::from("gloss,pos,official,predicted,n_clusters,miss_class\n");
    for r in &miss_rows {
        s.push_str(r);
        s.push('\n');
    }
    std::fs::write(out_dir.join("audit-misses.csv"), s)?;
    println!("Wrote {}", out_dir.join("audit-misses.csv").display());
    Ok(())
}

/// Proto-engine-only benchmark (§A of the V3 plan). Isolates the Proto-Slavic
/// rule engine's accuracy from linking/ranking/consensus: for every meaning that
/// gets a confident proto link, derive the form straight from the reconstruction
/// and compare to the official lemma. Reports link coverage and proto-only
/// accuracy by POS so the engine rules can be iterated against a tight signal.
pub fn run_proto_engine(official_path: &Path, out_dir: &Path) -> Result<()> {
    let entries: Vec<OfficialEntry> = official::load(official_path)?
        .into_iter()
        .filter(|e| e.is_benchmarkable())
        .collect();
    let Some(proto) = load_proto_index() else {
        anyhow::bail!(
            "no Proto-Slavic cache ({}); run `extract-proto` first.",
            crate::DEFAULT_PROTO_CACHE
        );
    };

    let (mut n, mut linked, mut exact, mut norm) = (0usize, 0usize, 0usize, 0usize);
    let mut by_pos: BTreeMap<&'static str, (usize, usize, usize)> = BTreeMap::new(); // (linked, exact, norm)
    let mut errors: Vec<(String, String, String, String, f32)> = Vec::new(); // gloss, official, proto_form, proto_word, conf

    for entry in &entries {
        let input = build_input(entry);
        if !input.forms.iter().any(|f| f.modern) {
            continue;
        }
        n += 1;
        let Some(l) = crate::proto_link::link(&proto, &input) else {
            continue;
        };
        linked += 1;
        let reflexes: Vec<String> = input
            .forms
            .iter()
            .filter(|f| f.modern)
            .map(|f| f.norm.latin.clone())
            .collect();
        let form =
            crate::proto::generate_with_reflexes(&l.entry.word, input.pos, input.gender, &reflexes)
                .form;
        let e = ortho::exact_match(&form, &entry.isv);
        let nm = ortho::normalized_match(&form, &entry.isv);
        exact += e as usize;
        norm += nm as usize;
        let bp = by_pos.entry(entry.pos.code()).or_default();
        bp.0 += 1;
        bp.1 += e as usize;
        bp.2 += nm as usize;
        if !nm {
            errors.push((
                entry.english.clone(),
                entry.isv.clone(),
                form,
                l.entry.word.clone(),
                l.confidence,
            ));
        }
    }

    let rate = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f32 / b as f32
        }
    };
    println!(
        "Proto-engine benchmark: {} linked / {} ({:.1}% coverage); on linked: exact {:.2}%, normalized {:.2}%",
        linked,
        n,
        rate(linked, n),
        rate(exact, linked),
        rate(norm, linked),
    );

    std::fs::create_dir_all(out_dir)?;
    let mut s = String::new();
    writeln!(s, "# Proto-Slavic engine benchmark\n")?;
    writeln!(
        s,
        "Isolates `proto::generate_with_reflexes` from linking/ranking/consensus: derive the form straight from the linked reconstruction and compare to the official lemma.\n"
    )?;
    writeln!(
        s,
        "- Benchmark entries with modern evidence: **{}**\n- Confidently linked to a Proto-Slavic entry: **{}** ({:.1}% coverage)\n- On the linked subset: **exact {:.2}%**, **normalized {:.2}%**\n",
        n,
        linked,
        rate(linked, n),
        rate(exact, linked),
        rate(norm, linked),
    )?;
    writeln!(s, "## Proto-engine accuracy by POS (linked subset)\n")?;
    writeln!(s, "| POS | linked | exact | normalized |")?;
    writeln!(s, "|---|---:|---:|---:|")?;
    for (pos, (ln, ex, nm)) in &by_pos {
        writeln!(
            s,
            "| {} | {} | {:.2}% | {:.2}% |",
            pos,
            ln,
            rate(*ex, *ln),
            rate(*nm, *ln)
        )?;
    }
    errors.sort_by(|a, b| b.4.total_cmp(&a.4)); // most-confident errors first (most actionable)
    writeln!(s, "\n## Confident proto-engine errors (sample)\n")?;
    writeln!(
        s,
        "| gloss | official | proto form | *reconstruction | link conf |"
    )?;
    writeln!(s, "|---|---|---|---|---:|")?;
    for (g, off, form, word, conf) in errors.iter().take(60) {
        writeln!(
            s,
            "| {} | {} | {} | *{} | {:.2} |",
            g.replace('|', "/"),
            off,
            form,
            word,
            conf
        )?;
    }
    std::fs::write(out_dir.join("proto-engine-report.md"), s)?;
    println!("Wrote {}", out_dir.join("proto-engine-report.md").display());
    Ok(())
}

/// Print the generator's full reasoning for one word/gloss (manual spot-check).
pub fn explain(official_path: &Path, query: &str) -> Result<()> {
    let entries = official::load(official_path)?;
    let ql = query.trim().to_lowercase();
    let entry = entries
        .iter()
        .find(|e| e.isv.to_lowercase() == ql)
        .or_else(|| {
            entries.iter().find(|e| {
                e.english
                    .to_lowercase()
                    .split(&[',', ';'][..])
                    .any(|g| g.trim() == ql)
            })
        })
        .or_else(|| {
            entries
                .iter()
                .find(|e| e.english.to_lowercase().contains(&ql))
        });

    let Some(entry) = entry else {
        println!("No official entry found matching '{query}'.");
        return Ok(());
    };

    let input = build_input(entry);
    let overrides = crate::overrides::Overrides::load(Path::new(crate::DEFAULT_OVERRIDES));
    let cfg = crate::consensus::ConsensusConfig::production();
    let proto = load_proto_index();
    let gen =
        crate::generator::generate(&input, Some(&entry.isv), proto.as_ref(), &cfg, &overrides);
    if let Some(r) = &gen.reconstruction {
        println!(
            "Reconstruction: *{} (link conf {:.2})",
            r.word, r.confidence
        );
    }

    println!("Gloss:    {}", entry.english);
    println!("POS:      {} ({})", entry.pos.code(), entry.pos_raw);
    println!("Official: {}", entry.isv);
    println!(
        "Status:   {:?} ({})",
        gen.match_status,
        gen.match_status.label()
    );
    println!("\nEvidence by branch:");
    for f in &input.forms {
        println!(
            "  [{}] {:<3} {:<18} -> {}",
            f.branch.code().chars().next().unwrap().to_uppercase(),
            f.lang_code,
            f.norm.original,
            f.norm.latin
        );
    }
    println!("\nRanked candidates:");
    for (i, c) in gen.candidates.iter().enumerate().take(5) {
        println!(
            "  {}. {:<20} score {:.3}  conf {:<7} branches {}  [{}]",
            i + 1,
            c.form,
            c.score,
            c.confidence.label(),
            c.branch_coverage,
            c.source.label()
        );
        for step in &c.trace {
            println!(
                "       · {}: {} -> {} ({})",
                step.id, step.before, step.after, step.explanation
            );
        }
        for w in &c.warnings {
            println!("       ! {w}");
        }
    }
    Ok(())
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

    // Load the Proto-Slavic cache if present; the +proto-derived rung needs it.
    let proto_index = load_proto_index();
    if proto_index.is_some() {
        println!("Loaded Proto-Slavic cache for the proto-derived rung.");
    } else {
        println!(
            "note: no Proto-Slavic cache ({}); run `extract-proto` to enable the proto-derived rung.",
            crate::DEFAULT_PROTO_CACHE
        );
    }
    let proto = proto_index.as_ref();

    let kept = kept_ladder();
    let runs: Vec<RunMetrics> = kept
        .iter()
        .map(|r| evaluate_config(&entries, r, proto))
        .collect();
    let rejected: Vec<RunMetrics> = rejected_experiments()
        .iter()
        .map(|r| evaluate_config(&entries, r, proto))
        .collect();

    println!("Kept ladder (cumulative):");
    for r in &runs {
        println!(
            "  {:<22} exact {:>6.2}%  norm {:>6.2}%  top3 {:>6.2}%  edit {:.3}",
            r.name,
            100.0 * Bucket::rate(r.exact, r.n),
            100.0 * Bucket::rate(r.normalized, r.n),
            100.0 * Bucket::rate(r.top3, r.n),
            r.sum_norm_edit / r.n.max(1) as f32,
        );
    }
    println!("Rejected experiments (production + one rule, deltas negative):");
    for r in &rejected {
        println!(
            "  {:<22} exact {:>6.2}%  norm {:>6.2}%",
            r.name,
            100.0 * Bucket::rate(r.exact, r.n),
            100.0 * Bucket::rate(r.normalized, r.n),
        );
    }

    std::fs::create_dir_all(out_dir)?;
    let baseline = &runs[0];
    // The kept ladder is monotone-improving by construction; the production
    // config is its last rung. Confirm empirically (by exact, then normalized).
    let best_idx = (0..runs.len())
        .max_by(|&a, &b| {
            let ra = &runs[a];
            let rb = &runs[b];
            Bucket::rate(ra.exact, ra.n)
                .total_cmp(&Bucket::rate(rb.exact, rb.n))
                .then(
                    Bucket::rate(ra.normalized, ra.n).total_cmp(&Bucket::rate(rb.normalized, rb.n)),
                )
        })
        .unwrap();
    let best = &runs[best_idx];
    println!("Kept production config: {}", best.name);

    write_summary_json(out_dir, &runs)?;
    write_report_md(out_dir, &runs, &rejected, best)?;
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

fn write_report_md(
    out_dir: &Path,
    runs: &[RunMetrics],
    rejected: &[RunMetrics],
    best: &RunMetrics,
) -> Result<()> {
    let baseline = &runs[0];
    let mut s = String::new();
    writeln!(s, "# Candidate-generation benchmark\n")?;
    writeln!(
        s,
        "Benchmark: reconstruct the official Interslavic lemma from the modern Slavic cognates in the official dictionary, **without showing the generator the answer**. Evaluated on **{}** benchmarkable single-word entries. Every rule is kept only if it improved measured accuracy.\n",
        baseline.n
    )?;
    writeln!(
        s,
        "- **Metrics.** *exact*: identical to the official flavored lemma; *normalized*: identical after reducing both to the standard alphabet (§1.3); *skeleton*: identical after an ASCII fold; *top-3/5*: any of the first N candidates matches (normalized); *mean edit*: mean normalized Levenshtein distance to the official lemma.\n"
    )?;
    writeln!(s, "## Kept rules — cumulative ablation ladder\n")?;
    writeln!(
        s,
        "Each rung adds exactly one rule to the previous, so its accuracy delta is attributable. The last rung is the kept **production** configuration.\n"
    )?;
    writeln!(
        s,
        "| Rung | exact top-1 | norm top-1 | Δ norm | top-3 | mean edit |"
    )?;
    writeln!(s, "|---|---:|---:|---:|---:|---:|")?;
    let mut prev_norm = Bucket::rate(baseline.normalized, baseline.n);
    for r in runs {
        let norm = Bucket::rate(r.normalized, r.n);
        let delta = norm - prev_norm;
        writeln!(
            s,
            "| {} | {:.2}% | {:.2}% | {:+.2} pp | {:.2}% | {:.3} |",
            r.name,
            100.0 * Bucket::rate(r.exact, r.n),
            100.0 * norm,
            100.0 * delta,
            100.0 * Bucket::rate(r.top3, r.n),
            r.sum_norm_edit / r.n.max(1) as f32,
        )?;
        prev_norm = norm;
    }
    writeln!(s)?;
    for r in runs {
        writeln!(s, "- **{}** — {}", r.name, r.description)?;
    }

    writeln!(s, "\n## Rejected rules — tested and reverted\n")?;
    writeln!(
        s,
        "Each is the production config plus one experimental rule. All regress accuracy on the benchmark and are therefore **not** in the production config, per the keep-only-if-it-improves rule.\n"
    )?;
    let prod_norm = Bucket::rate(best.normalized, best.n);
    let prod_exact = Bucket::rate(best.exact, best.n);
    writeln!(
        s,
        "| Experiment | exact top-1 | Δ exact | norm top-1 | Δ norm |"
    )?;
    writeln!(s, "|---|---:|---:|---:|---:|")?;
    for r in rejected {
        writeln!(
            s,
            "| {} | {:.2}% | {:+.2} pp | {:.2}% | {:+.2} pp |",
            r.name,
            100.0 * Bucket::rate(r.exact, r.n),
            100.0 * (Bucket::rate(r.exact, r.n) - prod_exact),
            100.0 * Bucket::rate(r.normalized, r.n),
            100.0 * (Bucket::rate(r.normalized, r.n) - prod_norm),
        )?;
    }
    writeln!(s)?;
    for r in rejected {
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

    // Remaining systematic errors: classify the misses by a cheap heuristic so
    // the largest remaining buckets are visible.
    writeln!(s, "\n## Remaining systematic errors (final config)\n")?;
    let misses: Vec<&EntryResult> = best.results.iter().filter(|r| !r.normalized).collect();
    let total_miss = misses.len();
    let near = misses.iter().filter(|r| r.norm_edit < 0.20).count();
    let far = total_miss - near;
    let mut by_cause: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &misses {
        *by_cause.entry(classify_error(r)).or_default() += 1;
    }
    writeln!(
        s,
        "Of **{}** misses, **{}** ({:.0}%) are near-misses (normalized edit < 0.20 — an ending/one-letter fix) and **{}** are farther (usually a different root chosen by Interslavic).\n",
        total_miss,
        near,
        100.0 * near as f32 / total_miss.max(1) as f32,
        far
    )?;
    let mut causes: Vec<(&&str, &usize)> = by_cause.iter().collect();
    causes.sort_by(|a, b| b.1.cmp(a.1));
    writeln!(s, "| Error class | count | share of misses |")?;
    writeln!(s, "|---|---:|---:|")?;
    for (cause, n) in causes {
        writeln!(
            s,
            "| {} | {} | {:.1}% |",
            cause,
            n,
            100.0 * (*n as f32) / total_miss.max(1) as f32
        )?;
    }

    writeln!(s, "\n## Next recommended linguistic rules\n")?;
    writeln!(
        s,
        "The Proto-Slavic-derived-form path (§4.4) is implemented — consensus picks the root and the Proto-Slavic rule engine supplies the flavored form via a leakage-free descendant+gloss link. Yer resolution now uses a genuine **tense-yer rule** (yer before *j → i/y) plus **reflex-guided vocalization** (a lexically-ambiguous weak yer is retained when the reflexes vote to keep it: `*pьsati`→`pisati` vs `*bьrati`→`brati`), and a length-free **reflex-shape-agreement** ranking rule replaced the earlier length heuristic. Ranked next steps, from the remaining-error analysis:\n\n1. **Expand Proto-Slavic link coverage.** Only meanings with a matched `sla-pro` reconstruction get the flavored derivation; raising cache coverage and loosening the link gate (without admitting bad links) directly grows the proto-derived slice.\n2. **Reduce the reconstruction's non-yer errors** (endings, palatalizations) so the proto form can be trusted even when it disagrees with the reflexes — currently such disagreements defer to the reflexes, capping the proto gain.\n3. **Divergent-root modeling (semantic families, §4.2 step 3).** The ~{far} far-misses are mostly cases where Interslavic picked a different root than the plurality skeleton; scoring candidate *roots* (not surface forms) over the six subgroups, clustered by the proto descendant graph, would recover many.\n4. **Secondary-imperfective verb stems** (`-yva-/-iva-/-ava-`) and the agentive `-telj`/abstract `-teljstvo` suffixes, seen repeatedly in the verb/noun error tail.\n5. **POS-specific gender/animacy inference** to pick the right nominal ending where the modern citation forms disagree.",
        far = far
    )?;

    std::fs::write(out_dir.join("candidate-generation-report.md"), s)?;
    Ok(())
}

/// Cheap heuristic bucketing of a miss into a systematic-error class.
fn classify_error(r: &EntryResult) -> &'static str {
    let off = &r.isv;
    let pred = &r.predicted;
    if pred.is_empty() {
        return "no candidate produced";
    }
    if r.norm_edit >= 0.34 {
        return "different root / derivation";
    }
    let so = ortho::to_standard(&off.to_lowercase());
    let sp = ortho::to_standard(&pred.to_lowercase());
    // Same skeleton but different flavored letters => a flavor-recovery miss.
    if ortho::ascii_skeleton(off) == ortho::ascii_skeleton(pred) {
        return "flavored letter (ě/ę/ų/å/ć/đ) not recovered";
    }
    if so.len() != sp.len() {
        if so.chars().count() > sp.chars().count() {
            return "missing letter (fleeting vowel / cluster)";
        }
        return "extra letter (epenthesis / ending)";
    }
    if off.contains('y') != pred.contains('y') || off.contains('i') != pred.contains('i') {
        return "y / i distinction";
    }
    "single-letter substitution"
}

fn write_diffs(out_dir: &Path, baseline: &RunMetrics, best: &RunMetrics) -> Result<()> {
    let base_map: BTreeMap<&str, &EntryResult> = baseline
        .results
        .iter()
        .map(|r| (r.id.as_str(), r))
        .collect();
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
