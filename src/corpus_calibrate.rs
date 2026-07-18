//! Leakage-free corpus coverage calibration.
//!
//! Corpus generation reads only the lemma cache. The official dictionary is
//! consulted afterwards to create an explicit POS-plus-semantic coverage-proxy
//! label. Isotonic fitting uses the immutable train split only; every reported
//! metric and operating point uses the untouched holdout split.

use crate::calibrate::{
    CorpusCalibration, CorpusCounts, CorpusMetrics, CorpusProvenance, OperatingPoint,
    ReliabilityBin, CORPUS_ALGORITHM_VERSION, CORPUS_COVERAGE_SCORE_DOMAIN,
    CORPUS_LABEL_POLICY_VERSION, CORPUS_SCHEMA_VERSION, CORPUS_SPLIT_POLICY, PROPOSE_T, REVIEW_T,
};
use crate::consensus::ConsensusConfig;
use crate::corpus_reference::OfficialIndex;
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::Path;

#[derive(Debug, Clone)]
struct Row {
    id: String,
    holdout: bool,
    positive: bool,
    reason: &'static str,
    sense_id: Option<String>,
    sense_index: Option<usize>,
    candidate_rank: Option<usize>,
    form: String,
    pos: String,
    gloss: String,
    n_langs: usize,
    n_branches: usize,
    raw_score: f32,
    probability: f64,
}

/// Stable concept identity derived from canonical corpus-set fields, not export
/// order. Membership is sorted by `build_sets`; sort again to make the contract
/// independent of that implementation detail.
fn stable_concept_id(set: &crate::corpus::CognateSet) -> String {
    let mut members: Vec<String> = set
        .members
        .iter()
        .map(|m| format!("{}\u{1f}{}\u{1f}{}\u{1f}{}", m.lang, m.word, m.pos, m.gloss))
        .collect();
    members.sort();
    let canonical = format!(
        "corpus-concept-v1\u{1e}{}\u{1e}{}\u{1e}{}\u{1e}{}\u{1e}{}\u{1e}{}",
        set.proto,
        set.etymon,
        set.borrowed,
        set.pos.code(),
        set.gloss,
        members.join("\u{1d}")
    );
    format!("corpus-v1-{:x}", Sha256::digest(canonical.as_bytes()))
}

fn decile(score: f32) -> usize {
    ((score.clamp(0.0, 1.0) * 10.0) as usize).min(9)
}

/// Decile histogram followed by pool-adjacent-violators. Empty deciles inherit
/// the nearest fitted probability (left first, then right), so all raw scores
/// have a monotone mapping while no synthetic observations enter the fit.
fn fit_pava(rows: &[&Row]) -> Result<[f64; 10]> {
    let mut bins = [(0usize, 0usize); 10];
    for row in rows {
        bins[decile(row.raw_score)].0 += 1;
        bins[decile(row.raw_score)].1 += row.positive as usize;
    }
    let occupied: Vec<(usize, f64, f64)> = bins
        .iter()
        .enumerate()
        .filter(|(_, (n, _))| *n > 0)
        .map(|(i, (n, hits))| (i, *n as f64, *hits as f64 / *n as f64))
        .collect();
    anyhow::ensure!(
        !occupied.is_empty(),
        "cannot fit corpus calibration on empty train set"
    );
    let mut pools: Vec<(usize, usize, f64, f64)> = Vec::new(); // first,last,weight,mean
    for (i, weight, mean) in occupied {
        pools.push((i, i, weight, mean));
        while pools.len() >= 2 && pools[pools.len() - 2].3 > pools[pools.len() - 1].3 {
            let (_, last, w2, m2) = pools.pop().unwrap();
            let (first, _, w1, m1) = pools.pop().unwrap();
            pools.push((first, last, w1 + w2, (w1 * m1 + w2 * m2) / (w1 + w2)));
        }
    }
    let mut values = [f64::NAN; 10];
    for (first, last, _, mean) in pools {
        for value in values.iter_mut().take(last + 1).skip(first) {
            *value = mean;
        }
    }
    let first = values.iter().copied().find(|v| v.is_finite()).unwrap();
    let mut previous = first;
    for value in &mut values {
        if value.is_finite() {
            previous = *value;
        } else {
            *value = previous;
        }
    }
    anyhow::ensure!(
        values.windows(2).all(|w| w[0] <= w[1]),
        "PAVA produced a non-monotone map"
    );
    anyhow::ensure!(
        values
            .iter()
            .all(|p| p.is_finite() && (0.0..=1.0).contains(p)),
        "PAVA produced invalid probabilities"
    );
    Ok(values)
}

fn metrics(rows: &[&Row], calibrated: bool) -> CorpusMetrics {
    let mut bins = [(0usize, 0.0f64, 0usize); 10];
    let mut brier = 0.0;
    for row in rows {
        let p = if calibrated {
            row.probability
        } else {
            row.raw_score as f64
        };
        let y = row.positive as u8 as f64;
        brier += (p - y).powi(2);
        let b = ((p.clamp(0.0, 1.0) * 10.0) as usize).min(9);
        bins[b].0 += 1;
        bins[b].1 += p;
        bins[b].2 += row.positive as usize;
    }
    let n = rows.len() as f64;
    let ece = bins
        .iter()
        .filter(|(count, _, _)| *count > 0)
        .map(|(count, sum, hits)| {
            *count as f64 / n * (sum / *count as f64 - *hits as f64 / *count as f64).abs()
        })
        .sum();
    CorpusMetrics {
        ece,
        brier: brier / n,
    }
}

fn reliability(rows: &[&Row]) -> Vec<ReliabilityBin> {
    let mut bins = [(0usize, 0usize, 0.0f64); 10];
    for row in rows {
        let b = ((row.probability.clamp(0.0, 1.0) * 10.0) as usize).min(9);
        bins[b].0 += 1;
        bins[b].1 += row.positive as usize;
        bins[b].2 += row.probability;
    }
    bins.iter()
        .enumerate()
        .map(|(i, (count, hits, sum))| ReliabilityBin {
            lower: i as f64 / 10.0,
            upper: (i + 1) as f64 / 10.0,
            count: *count,
            hits: *hits,
            mean_probability: if *count == 0 {
                0.0
            } else {
                sum / *count as f64
            },
        })
        .collect()
}

fn operating_point(rows: &[&Row], positives: usize, threshold: f64) -> OperatingPoint {
    let selected = rows.iter().filter(|r| r.probability >= threshold).count();
    let hits = rows
        .iter()
        .filter(|r| r.probability >= threshold && r.positive)
        .count();
    OperatingPoint {
        threshold,
        selected,
        hits,
        precision: if selected == 0 {
            0.0
        } else {
            hits as f64 / selected as f64
        },
        coverage: hits as f64 / positives as f64,
    }
}

pub fn run(
    lemmas_path: &Path,
    official_path: &Path,
    out_dir: &Path,
    artifact_path: &Path,
) -> Result<()> {
    let lemmas_sha256 = crate::calibrate::sha256_file(lemmas_path)?;
    let official_sha256 = crate::calibrate::sha256_file(official_path)?;
    let corpus = crate::dump::LemmaCorpus::load(lemmas_path)?;
    let official = crate::official::load(official_path)?;
    let index = OfficialIndex::new(&official);
    let cfg = ConsensusConfig::production();
    let mut rows = Vec::new();
    for set in crate::corpus::build_sets(&corpus) {
        let id = stable_concept_id(&set);
        let generated = crate::corpus::generate_set(set, &cfg);
        if generated.form().is_empty() {
            continue;
        }
        let matched = index.match_candidates(
            &generated.candidates,
            &official,
            generated.set.pos,
            &generated.set.gloss,
        );
        rows.push(Row {
            holdout: crate::eval::is_holdout_id(&id),
            positive: matched.is_some(),
            reason: if matched.is_some() {
                "official-pos-semantic-match"
            } else {
                "no-compatible-official-sense"
            },
            sense_id: matched.as_ref().map(|m| m.sense_id.clone()),
            sense_index: matched.as_ref().map(|m| m.sense_index),
            candidate_rank: matched.as_ref().map(|m| m.candidate_rank),
            id,
            form: generated.form().to_string(),
            pos: generated.set.pos.code().to_string(),
            gloss: generated.set.gloss.clone(),
            n_langs: generated.n_langs,
            n_branches: generated.n_branches,
            raw_score: generated.score,
            probability: 0.0,
        });
    }
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    anyhow::ensure!(
        rows.windows(2).all(|w| w[0].id != w[1].id),
        "stable corpus concept ID collision"
    );
    let train: Vec<&Row> = rows.iter().filter(|r| !r.holdout).collect();
    let train_len = train.len();
    let train_positive = train.iter().filter(|r| r.positive).count();
    let holdout_len = rows.iter().filter(|r| r.holdout).count();
    let holdout_positive = rows.iter().filter(|r| r.holdout && r.positive).count();
    anyhow::ensure!(
        train_len > 0 && holdout_len > 0,
        "empty corpus train/holdout split"
    );
    anyhow::ensure!(
        train_positive > 0 && train_positive < train_len,
        "corpus train split lacks positive or negative class"
    );
    anyhow::ensure!(
        holdout_positive > 0 && holdout_positive < holdout_len,
        "corpus holdout split lacks positive or negative class"
    );
    let deciles = fit_pava(&train)?;
    drop(train);
    for row in &mut rows {
        row.probability = deciles[decile(row.raw_score)];
    }
    let holdout: Vec<&Row> = rows.iter().filter(|r| r.holdout).collect();
    let raw_holdout = metrics(&holdout, false);
    let calibrated_holdout = metrics(&holdout, true);
    anyhow::ensure!(
        raw_holdout.ece.is_finite()
            && raw_holdout.brier.is_finite()
            && calibrated_holdout.ece.is_finite()
            && calibrated_holdout.brier.is_finite(),
        "degenerate corpus holdout metrics"
    );
    let propose = operating_point(&holdout, holdout_positive, PROPOSE_T);
    let review = operating_point(&holdout, holdout_positive, REVIEW_T);
    anyhow::ensure!(
        propose.selected > 0 && review.selected > 0,
        "calibrated corpus probabilities do not reach existing proposal/review thresholds"
    );
    let artifact = CorpusCalibration {
        schema_version: CORPUS_SCHEMA_VERSION,
        score_domain: CORPUS_COVERAGE_SCORE_DOMAIN.into(),
        score_model_version: crate::corpus::COVERAGE_SCORE_MODEL_VERSION.into(),
        label_policy_version: CORPUS_LABEL_POLICY_VERSION.into(),
        split_policy: CORPUS_SPLIT_POLICY.into(),
        algorithm_version: CORPUS_ALGORITHM_VERSION.into(),
        provenance: CorpusProvenance {
            lemmas_path: lemmas_path.display().to_string(),
            official_path: official_path.display().to_string(),
            lemmas_sha256: lemmas_sha256.clone(),
            official_sha256: official_sha256.clone(),
            fitted_on: "corpus semantic-proxy train split only".into(),
        },
        counts: CorpusCounts {
            train: train_len,
            train_positive,
            holdout: holdout_len,
            holdout_positive,
        },
        raw_holdout,
        calibrated_holdout,
        reliability_bins: reliability(&holdout),
        propose,
        review,
        deciles,
    };
    artifact.validate(&lemmas_sha256, &official_sha256)?;

    let mut inventory = String::from("concept_id\tsplit\tlabel\treason\tofficial_sense_id\tofficial_sense_index\tcandidate_rank\tform\tpos\tgloss\tn_langs\tn_branches\traw_score\tprobability\n");
    for row in &rows {
        writeln!(
            inventory,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.6}\t{:.6}",
            row.id,
            if row.holdout { "holdout" } else { "train" },
            row.positive as u8,
            row.reason,
            row.sense_id.as_deref().unwrap_or(""),
            row.sense_index.map(|v| v.to_string()).unwrap_or_default(),
            row.candidate_rank
                .map(|v| v.to_string())
                .unwrap_or_default(),
            sanitize(&row.form),
            row.pos,
            sanitize(&row.gloss),
            row.n_langs,
            row.n_branches,
            row.raw_score,
            row.probability
        )?;
    }
    let report = format!("# Corpus coverage calibration\n\n- Score domain: `{}` (`{}`)\n- Labels: `{}`. A negative means only that no compatible official sense was found; it is not proof that a reconstruction is linguistically wrong.\n- Split: `{}`; isotonic/PAVA fit uses train rows only.\n- Coverage means recall over holdout semantic positives.\n- Inputs: `{}` `{}`; `{}` `{}`.\n\n| split | rows | semantic positives |\n|---|---:|---:|\n| train | {} | {} |\n| holdout | {} | {} |\n\n| holdout metric | raw | calibrated |\n|---|---:|---:|\n| ECE | {:.6} | {:.6} |\n| Brier | {:.6} | {:.6} |\n\n| unfiltered holdout operating point (not proposal-list quality) | selected | hits | precision | coverage |\n|---|---:|---:|---:|---:|\n| proposal p≥{:.1} | {} | {} | {:.6} | {:.6} |\n| review p≥{:.1} | {} | {} | {:.6} | {:.6} |\n",
        artifact.score_domain, artifact.score_model_version, artifact.label_policy_version, artifact.split_policy,
        lemmas_path.display(), lemmas_sha256, official_path.display(), official_sha256,
        artifact.counts.train, artifact.counts.train_positive, artifact.counts.holdout, artifact.counts.holdout_positive,
        artifact.raw_holdout.ece, artifact.calibrated_holdout.ece, artifact.raw_holdout.brier, artifact.calibrated_holdout.brier,
        artifact.propose.threshold, artifact.propose.selected, artifact.propose.hits, artifact.propose.precision, artifact.propose.coverage,
        artifact.review.threshold, artifact.review.selected, artifact.review.hits, artifact.review.precision, artifact.review.coverage);

    std::fs::create_dir_all(out_dir)?;
    if let Some(parent) = artifact_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut artifact_json = serde_json::to_string_pretty(&artifact)?;
    artifact_json.push('\n');
    std::fs::write(artifact_path, artifact_json)?;
    std::fs::write(out_dir.join("corpus-coverage-calibration.md"), report)?;
    std::fs::write(out_dir.join("corpus-coverage-evaluation.tsv"), inventory)?;
    println!("corpus calibration: {} train ({} positive), {} holdout ({} positive); holdout ECE {:.4}, Brier {:.4}", artifact.counts.train, artifact.counts.train_positive, artifact.counts.holdout, artifact.counts.holdout_positive, artifact.calibrated_holdout.ece, artifact.calibrated_holdout.brier);
    Ok(())
}

fn sanitize(value: &str) -> String {
    value.replace(['\t', '\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(score: f32, positive: bool) -> Row {
        Row {
            id: String::new(),
            holdout: false,
            positive,
            reason: "",
            sense_id: None,
            sense_index: None,
            candidate_rank: None,
            form: String::new(),
            pos: String::new(),
            gloss: String::new(),
            n_langs: 0,
            n_branches: 0,
            raw_score: score,
            probability: 0.0,
        }
    }

    #[test]
    fn concept_ids_ignore_member_order_and_have_stable_split() {
        use crate::dump::LemmaEntry;
        let member = |lang: &str, word: &str| LemmaEntry {
            lang: lang.into(),
            word: word.into(),
            pos: "noun".into(),
            gloss: "water".into(),
            proto: "*voda".into(),
            etymon: "*voda".into(),
            etymology: Vec::new(),
            categories: Vec::new(),
            topics: Vec::new(),
            tags: Vec::new(),
        };
        let make = |members| crate::corpus::CognateSet {
            proto: "voda".into(),
            etymon: "*voda".into(),
            borrowed: false,
            pos: crate::model::Pos::Noun,
            gloss: "water".into(),
            members,
        };
        let a = stable_concept_id(&make(vec![member("ru", "voda"), member("pl", "woda")]));
        let b = stable_concept_id(&make(vec![member("pl", "woda"), member("ru", "voda")]));
        assert_eq!(a, b);
        assert_eq!(
            crate::eval::is_holdout_id(&a),
            crate::eval::is_holdout_id(&b)
        );
    }

    #[test]
    fn pava_is_monotone_and_uses_labels() {
        let rows = [
            row(0.15, true),
            row(0.15, true),
            row(0.55, false),
            row(0.55, false),
            row(0.85, true),
        ];
        let refs: Vec<&Row> = rows.iter().collect();
        let fitted = fit_pava(&refs).unwrap();
        assert!(fitted.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(fitted[1], fitted[5]);
        assert!(fitted[8] > fitted[5]);
    }

    #[test]
    fn holdout_metrics_count_only_supplied_rows() {
        let mut rows = [row(0.2, false), row(0.8, true), row(0.9, true)];
        rows[0].probability = 0.1;
        rows[1].probability = 0.7;
        rows[2].probability = 0.8;
        let held = vec![&rows[0], &rows[1]];
        let bins = reliability(&held);
        assert_eq!(bins.iter().map(|b| b.count).sum::<usize>(), 2);
        assert_eq!(bins.iter().map(|b| b.hits).sum::<usize>(), 1);
        let op = operating_point(&held, 1, 0.3);
        assert_eq!((op.selected, op.hits, op.coverage), (1, 1, 1.0));
    }
}
