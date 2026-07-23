//! Slavic perfective ↔ imperfective pairing and conservative pair repair.
//!
//! Pair discovery uses official aspect/gloss metadata only to define the
//! benchmark slice and site links. Pair generation never reads either gold
//! lemma: it receives the two independently generated candidates and repairs a
//! root disagreement with documented suffix morphology.

use crate::consensus::{ConsensusConfig, MeaningInput};
use crate::model::Candidate;
use crate::official::OfficialEntry;
use crate::orthography as ortho;
use std::collections::HashMap;

// Aspect and aspect() moved verbatim to crate::postag, the one pos_raw
// grammar (V15 item 5); the old paths stay valid.
pub use crate::postag::{aspect, Aspect};

#[derive(Debug, Clone, Copy)]
pub struct AspectPair {
    pub imperfective: usize,
    pub perfective: usize,
}

/// Deterministic 1:1 pairing used by the benchmark and site. Same-gloss
/// candidates are greedily matched by the existing consonant-root criterion;
/// each entry participates at most once, preventing hub lemmas from dominating.
pub fn detect_pairs(entries: &[OfficialEntry]) -> Vec<AspectPair> {
    let mut ipf: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut pf: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        let word = e.isv.trim();
        let gloss = e.english.trim();
        if word.is_empty() || word.contains(' ') || word.contains('#') || gloss.is_empty() {
            continue;
        }
        match aspect(&e.pos_raw) {
            // Preserve the pre-registered 1,440-pair slice: its historical
            // detector admitted the rare `ipf./pf.` row on the imperfective
            // side because `pos_raw.contains("ipf.")` matched first.
            Some(Aspect::Imperfective | Aspect::Biaspectual) => {
                ipf.entry(gloss).or_default().push(i)
            }
            Some(Aspect::Perfective) => pf.entry(gloss).or_default().push(i),
            None => {}
        }
    }
    let mut glosses: Vec<&str> = ipf.keys().copied().collect();
    glosses.sort_unstable();
    let mut out = Vec::new();
    for gloss in glosses {
        let Some(perfectives) = pf.get(gloss) else {
            continue;
        };
        let mut used = vec![false; perfectives.len()];
        for &ii in &ipf[gloss] {
            let ik = ortho::consonant_key(&entries[ii].isv);
            let Some(slot) = perfectives.iter().enumerate().position(|(n, &pi)| {
                !used[n] && roots_related(&ik, &ortho::consonant_key(&entries[pi].isv))
            }) else {
                continue;
            };
            used[slot] = true;
            out.push(AspectPair {
                imperfective: ii,
                perfective: perfectives[slot],
            });
        }
    }
    out
}

fn roots_related(a: &str, b: &str) -> bool {
    // Preserve the pre-registered issue-75 denominator exactly: the legacy
    // slice treated an empty consonant skeleton as suffix-related via
    // `ends_with("")`. Such rare vowel-only rows remain honest misses.
    a.ends_with(b) || b.ends_with(a) || ortho::shares_consonant_root(a, b)
}

/// Consonant-root fingerprint consistency, using the project's deliberately
/// broad cognate heuristic (suffix containment or matching first consonants).
/// This is not a claim of proven etymological identity.
pub fn pairing_correct(imperfective: &str, perfective: &str) -> bool {
    roots_related(
        &ortho::consonant_key(imperfective),
        &ortho::consonant_key(perfective),
    )
}

#[derive(Debug, Clone, Copy)]
pub struct AspectConfig {
    pub suffix_repair: bool,
    pub prefix_perfectivization: bool,
    pub secondary_imperfectives: bool,
}

impl AspectConfig {
    pub const fn baseline() -> Self {
        Self {
            suffix_repair: false,
            prefix_perfectivization: false,
            secondary_imperfectives: false,
        }
    }

    pub const fn production() -> Self {
        Self {
            suffix_repair: true,
            prefix_perfectivization: true,
            // Implemented and ladder-measured, but disabled: after lexical
            // exceptions are excluded from scoring, the rung is flat on the
            // held-out primary metric (house generalization rule).
            secondary_imperfectives: false,
        }
    }

    pub const fn with_secondary_imperfectives() -> Self {
        Self {
            suffix_repair: true,
            prefix_perfectivization: true,
            secondary_imperfectives: true,
        }
    }
}

const SUPPLETIVE: &[(&str, &str)] = &[
    ("idti", "pojdti"),
    ("jęti", "vzęti"),
    ("tykati", "tknųti"),
    ("pinati", "pnųti"),
    ("jesti", "sjesti"),
];

fn suppletive_pair(imperfective: &str, perfective: &str) -> Option<(&'static str, &'static str)> {
    SUPPLETIVE.iter().copied().find(|(i, p)| {
        ortho::normalized_match(imperfective, i) || ortho::normalized_match(perfective, p)
    })
}

#[derive(Debug, Clone)]
pub struct PairPrediction {
    pub imperfective: String,
    pub perfective: String,
    pub rule: &'static str,
}

/// Production pair-generation entry point shared by the site export and the
/// dedicated benchmark. Both inputs contain only cognate evidence and grammar
/// metadata; neither official answer is accepted. The stronger reconstructed
/// member supplies the shared root for regular partner derivation.
pub fn generate_pair(
    imperfective: &MeaningInput,
    perfective: &MeaningInput,
    proto: Option<&crate::dump::ProtoIndex>,
    consensus_cfg: &ConsensusConfig,
    aspect_cfg: AspectConfig,
) -> Option<PairPrediction> {
    let ipf = crate::pipeline::generate(imperfective, proto, consensus_cfg)
        .0
        .into_iter()
        .next()?;
    let pf = crate::pipeline::generate(perfective, proto, consensus_cfg)
        .0
        .into_iter()
        .next()?;
    Some(reconcile_pair(&ipf, &pf, aspect_cfg))
}

/// Pair-aware generation. Independently reconstructed forms are retained when
/// their roots agree. On disagreement, repair only the lower-scoring member by
/// deriving it from the stronger member with regular ISV aspect suffixes.
/// No official form or dictionary answer is accepted by this function.
pub fn reconcile_pair(ipf: &Candidate, pf: &Candidate, cfg: AspectConfig) -> PairPrediction {
    if !cfg.suffix_repair {
        return PairPrediction {
            imperfective: ipf.form.clone(),
            perfective: pf.form.clone(),
            rule: "independent-baseline",
        };
    }
    // Closed lexical exceptions are declared as grammar data, not discovered
    // from benchmark outcomes. They are intentionally tiny and auditable.
    if let Some((i, p)) = suppletive_pair(&ipf.form, &pf.form) {
        return PairPrediction {
            imperfective: i.to_string(),
            perfective: p.to_string(),
            rule: "closed-suppletive-pair",
        };
    }
    if pairing_correct(&ipf.form, &pf.form) {
        return PairPrediction {
            imperfective: ipf.form.clone(),
            perfective: pf.form.clone(),
            rule: "independent-roots-agree",
        };
    }

    let mut options: Vec<(f32, PairPrediction)> = Vec::new();
    let anchor_is_ipf = ipf.score >= pf.score;
    for (form, rule) in derive_imperfectives(&pf.form, cfg) {
        if !anchor_is_ipf && pairing_correct(&form, &pf.form) {
            let edit = ortho::normalized_edit_distance(&form, &ipf.form);
            // Prefer preserving the stronger anchor; edit distance breaks ties.
            options.push((
                edit + ipf.score.max(0.0),
                PairPrediction {
                    imperfective: form,
                    perfective: pf.form.clone(),
                    rule,
                },
            ));
        }
    }
    for (form, rule) in derive_perfectives(&ipf.form, cfg) {
        if anchor_is_ipf && pairing_correct(&ipf.form, &form) {
            let edit = ortho::normalized_edit_distance(&form, &pf.form);
            options.push((
                edit + pf.score.max(0.0),
                PairPrediction {
                    imperfective: ipf.form.clone(),
                    perfective: form,
                    rule,
                },
            ));
        }
    }
    // Prefix perfectivization: the independently generated PF candidate tells
    // us which productive prefix its cognates support; transfer only that
    // prefix to the shared IPF reconstruction, never an official form.
    let prefixes: &[&str] = if cfg.prefix_perfectivization {
        &[
            "prě", "råz", "pod", "nad", "pri", "pro", "iz", "na", "po", "do", "od", "ob", "za",
            "vy", "o", "s", "v", "u",
        ]
    } else {
        &[]
    };
    for prefix in prefixes {
        let remainder = pf.form.strip_prefix(prefix).unwrap_or("");
        if anchor_is_ipf
            && !ipf.form.starts_with(prefix)
            && prefix_hint_compatible(&ipf.form, remainder)
        {
            let form = format!("{prefix}{}", ipf.form);
            let edit = ortho::normalized_edit_distance(&form, &pf.form);
            options.push((
                edit + pf.score.max(0.0),
                PairPrediction {
                    imperfective: ipf.form.clone(),
                    perfective: form,
                    rule: "prefix-perfectivization",
                },
            ));
        }
    }
    options.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.rule.cmp(b.1.rule)));
    options
        .into_iter()
        .next()
        .map(|(_, p)| p)
        .unwrap_or(PairPrediction {
            imperfective: ipf.form.clone(),
            perfective: pf.form.clone(),
            rule: "unrepaired",
        })
}

fn prefix_hint_compatible(anchor: &str, remainder: &str) -> bool {
    !remainder.is_empty() && ortho::normalized_edit_distance(anchor, remainder) <= 0.45
}

fn replace_suffix(word: &str, from: &str, to: &str) -> Option<String> {
    word.strip_suffix(from).map(|stem| format!("{stem}{to}"))
}

/// Documented secondary-imperfective families plus the productive -nųti/-ati
/// pair. Multiple forms are candidates; reconciliation chooses without gold by
/// proximity to the independently reconstructed partner.
pub fn derive_imperfectives(perfective: &str, cfg: AspectConfig) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for (from, to, rule, secondary) in [
        ("nųti", "ati", "pf-nuti-to-ipf-ati", false),
        ("iti", "jati", "pf-iti-to-ipf-jati", false),
        ("ovati", "ovyvati", "ovati-to-secondary-ovyvati", true),
        ("ati", "yvati", "secondary-ipf-yvati", true),
        ("ati", "ivati", "secondary-ipf-ivati", true),
        ("ati", "avati", "secondary-ipf-avati", true),
    ] {
        if secondary && !cfg.secondary_imperfectives {
            continue;
        }
        if let Some(v) = replace_suffix(perfective, from, to) {
            if v != perfective && !out.iter().any(|(x, _)| x == &v) {
                out.push((v, rule));
            }
        }
    }
    out
}

pub fn derive_perfectives(imperfective: &str, cfg: AspectConfig) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    let has_secondary_suffix = ["yvati", "ivati", "avati", "ovati"]
        .iter()
        .any(|suffix| imperfective.ends_with(suffix));
    for (from, to, rule, secondary) in [
        ("jati", "iti", "ipf-jati-to-pf-iti", false),
        ("ati", "nųti", "ipf-ati-to-pf-nuti", false),
        ("ovyvati", "ovati", "secondary-ovyvati-to-ovati", true),
        ("yvati", "ati", "secondary-ipf-yvati-reverse", true),
        ("ivati", "ati", "secondary-ipf-ivati-reverse", true),
        ("avati", "ati", "secondary-ipf-avati-reverse", true),
    ] {
        if secondary && !cfg.secondary_imperfectives {
            continue;
        }
        // Expanded imperfective suffixes all end in `-ati`; allowing the
        // generic productive rule to see them would bypass the secondary-rule
        // flag (or the separate `-ovati/-uje` class) and emit forms such as
        // `sprašivnųti` or `organizovnųti` in production.
        if from == "ati" && !secondary && has_secondary_suffix {
            continue;
        }
        if let Some(v) = replace_suffix(imperfective, from, to) {
            if v != imperfective && !out.iter().any(|(x, _)| x == &v) {
                out.push((v, rule));
            }
        }
    }
    out
}

/// Present-tense stem allomorph for `-ovati` verbs (`kupovati` → `kupuje`).
/// Pair artifacts export infinitives; this helper pins the requested
/// `-ovati/-ujE-` morphology for downstream conjugation.
pub fn ovati_present_stem(lemma: &str) -> Option<String> {
    lemma.strip_suffix("ovati").map(|stem| format!("{stem}uje"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_tags_are_unambiguous() {
        assert_eq!(aspect("v.tr. ipf."), Some(Aspect::Imperfective));
        assert_eq!(aspect("v.tr. pf."), Some(Aspect::Perfective));
        assert_eq!(aspect("v.tr. ipf./pf."), Some(Aspect::Biaspectual));
    }

    #[test]
    fn prefix_transfer_requires_the_hint_remainder_to_share_the_anchor_root() {
        use crate::model::CandidateSource;
        let ipf = Candidate::new("kalkulavac".into(), CandidateSource::BranchConsensus, 0.8);
        let pf = Candidate::new("viličic".into(), CandidateSource::BranchConsensus, 0.2);
        let bad = reconcile_pair(&ipf, &pf, AspectConfig::production());
        assert_ne!(bad.perfective, "vkalkulavac");

        let ipf = Candidate::new("pisati".into(), CandidateSource::BranchConsensus, 0.8);
        let pf = Candidate::new("zapisati".into(), CandidateSource::BranchConsensus, 0.2);
        let good = reconcile_pair(&ipf, &pf, AspectConfig::production());
        assert_eq!(good.perfective, "zapisati");
    }

    #[test]
    fn regular_partner_derivations_cover_secondary_imperfectives() {
        let ipf = derive_imperfectives("dobaviti", AspectConfig::with_secondary_imperfectives());
        assert!(ipf.iter().any(|(f, _)| f == "dobavjati"));
        let ipf = derive_imperfectives("bryzgnųti", AspectConfig::with_secondary_imperfectives());
        assert!(ipf.iter().any(|(f, _)| f == "bryzgati"));
        let secondary =
            derive_perfectives("pokazyvati", AspectConfig::with_secondary_imperfectives());
        assert!(secondary.iter().any(|(f, _)| f == "pokazati"));
        assert!(!secondary.iter().any(|(f, _)| f == "pokazyvnųti"));
        assert!(
            derive_imperfectives("organizovati", AspectConfig::with_secondary_imperfectives())
                .iter()
                .any(|(f, _)| f == "organizovyvati")
        );
        assert_eq!(ovati_present_stem("kupovati").as_deref(), Some("kupuje"));
        assert_eq!(suppletive_pair("idti", "hoditi"), Some(("idti", "pojdti")));
    }

    #[test]
    fn production_does_not_bypass_disabled_secondary_imperfectives() {
        for word in [
            "pokazyvati",
            "sprašivati",
            "prědavati",
            "organizovati",
            "organizovyvati",
        ] {
            assert!(
                derive_perfectives(word, AspectConfig::production()).is_empty(),
                "production derived a perfective from secondary-family {word}"
            );
        }
        assert!(derive_perfectives("pisati", AspectConfig::production())
            .iter()
            .any(|(form, rule)| form == "pisnųti" && *rule == "ipf-ati-to-pf-nuti"));
    }
}
