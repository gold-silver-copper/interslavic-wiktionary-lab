//! Shared generation pipeline used by both the benchmark and the website.
//!
//! Implements the two-stage model (rule spec §4.4): the modern-Slavic consensus
//! chooses the *root*; when a confident Proto-Slavic link exists, the
//! Proto-Slavic rule engine supplies the *form* (with the flavored letters the
//! consensus cannot recover from modern reflexes). This module is leakage-free —
//! it never sees the official lemma — so the benchmark can call it directly.

use crate::consensus::{self, ConsensusConfig, MeaningInput};
use crate::dump::ProtoIndex;
use crate::model::{
    Candidate, CandidateSource, Confidence, Evidence, EvidenceRelation, Reconstruction, RuleStep,
};
use crate::orthography as ortho;
use crate::proto_link;

const DOC_DESIGN: &str = "https://interslavic.fun/learn/misc/design-criteria/";

/// Generate ranked candidates for a meaning, plus the linked reconstruction (if
/// any) for display.
pub fn generate(
    input: &MeaningInput,
    proto: Option<&ProtoIndex>,
    cfg: &ConsensusConfig,
) -> (Vec<Candidate>, Option<Reconstruction>) {
    generate_oracle(input, proto, cfg, None)
}

/// As [`generate`], but with diagnostic oracle hints (V7 §2.4). Never used in
/// production; only the `--diagnostic-oracle` eval path passes a non-`None`
/// oracle, which reads the official answer to measure a stage's headroom.
pub fn generate_oracle(
    input: &MeaningInput,
    proto: Option<&ProtoIndex>,
    cfg: &ConsensusConfig,
    oracle: Option<&consensus::Oracle>,
) -> (Vec<Candidate>, Option<Reconstruction>) {
    let mut candidates = consensus::generate_oracle(input, cfg, oracle);
    let mut reconstruction = None;

    if cfg.proto_derived_form {
        if let Some(index) = proto {
            let linked = if let Some(o) = oracle.filter(|o| o.proto_link) {
                // Oracle proto link (diagnostic): the reconstruction whose derived
                // form is closest to the official lemma — the linker's upper bound.
                proto_link::link_oracle(index, input, o.official)
            } else if cfg.explicit_etymology {
                proto_link::link_explicit(index, input).or_else(|| {
                    proto_link::link(
                        index,
                        input,
                        cfg.proto_prefix_stripping,
                        cfg.proto_link_deep_corroboration,
                    )
                })
            } else {
                proto_link::link(
                    index,
                    input,
                    cfg.proto_prefix_stripping,
                    cfg.proto_link_deep_corroboration,
                )
            };
            if let Some(l) = linked {
                // Feed the modern reflexes to the yer resolver so lexicalized
                // weak-yer retentions (pьsati→pisati) are derived correctly rather
                // than papered over downstream. Only reflexes that are actually
                // cognate with the linked reconstruction feed the vote: a meaning
                // whose cell mixes synonyms of different roots (staruška for
                // *babъka "old woman") would otherwise pollute the yer alignment
                // and inject a spurious vowel (babka→babaka).
                let recon_key = ortho::consonant_key(&ortho::fold_key(&l.entry.word));
                let reflexes: Vec<String> = input
                    .forms
                    .iter()
                    .filter(|f| f.modern && f.primary)
                    .map(|f| f.norm.latin.clone())
                    .filter(|r| ortho::shares_consonant_root(&ortho::consonant_key(r), &recon_key))
                    .collect();
                // Stem-class-aware citation endings (issue #76), flag-gated.
                // The linker's own internal derivations stay stem_class-blind
                // so the rung cannot feed back into link confidences.
                let stem_class = if cfg.proto_stem_class_endings {
                    l.entry.stem_class.as_deref()
                } else {
                    None
                };
                let mut pc = crate::proto::generate_with_reflexes(
                    &l.entry.word,
                    input.pos,
                    input.gender,
                    &reflexes,
                    stem_class,
                );
                // Prefix-stripped link: re-attach the Interslavic prefix onto the
                // derived bare root (*prostirati → prostirati → råzprostirati).
                if let (Some(pref), false) = (&l.prefix, pc.form.is_empty()) {
                    pc.form = format!("{pref}{}", pc.form);
                }
                if !pc.form.is_empty() {
                    // §4.4: the regular derivation is authoritative for the *form*.
                    // A confidently-linked reconstruction therefore outranks the
                    // consensus surface (whose flavored letters are guesses). The
                    // link gate + form-similarity signal keep weak links from
                    // winning; below the confidence bar the proto form is only a
                    // scored alternative.
                    let consensus_top = candidates.iter().map(|c| c.score).fold(0.0_f32, f32::max);
                    // The reconstruction overrides consensus only when it *agrees*
                    // with the reflexes on the shape — then it merely supplies the
                    // flavored letters the consensus guessed (język y/i, blago g/h).
                    // When it diverges, the reflexes (consensus) win; the proto
                    // form stays a scored alternative.
                    let cons_top = candidates.first();
                    let cons_form = cons_top.map(|c| c.form.clone()).unwrap_or_default();
                    let cons_branch_cov = cons_top.map_or(0, |c| c.branch_coverage);
                    let agree = flavor_equivalent(&pc.form, &cons_form);
                    // Reflex-shape agreement, confidence-gated (§F: a trustworthy
                    // engine earns a looser gate). When the reconstruction agrees
                    // with the reflexes on the segments it always supplies the
                    // flavored spelling. On a segmental *disagreement* it now wins
                    // too — the engine is accurate enough (proto-only benchmark) to
                    // be trusted — UNLESS the link itself is weak (confidence <
                    // 0.62), where the living evidence wins instead. Adjectives are
                    // exempt (their consensus citation often has a spurious fleeting
                    // vowel the reconstruction rightly drops: dobry vs dobery).
                    // Rejected experiment: also demoting when consensus is strong
                    // (cons_branch_cov >= 3) regressed −0.47pp exact — a 3-branch
                    // consensus is often the de-flavored form the reconstruction
                    // correctly flavors. The flavor-equivalence gate already
                    // handles the flavor/prothesis-only case.
                    let _ = cons_branch_cov;
                    let demote =
                        !agree && l.confidence < 0.62 && input.pos != crate::model::Pos::Adjective;
                    let base = 0.58 + 0.40 * l.confidence;
                    let score = if demote {
                        base.min(consensus_top - 0.03)
                    } else if l.confidence >= 0.60 {
                        base.max(consensus_top + 0.02)
                    } else {
                        base
                    }
                    .clamp(0.05, 0.98);
                    pc.score = round3(score);
                    pc.confidence = Confidence::from_score(pc.score);
                    pc.branch_coverage = (l.desc_membership * 3.0).round() as u8;
                    // Supporting languages (issue #79 razumlivost): the modern
                    // primary reflexes sharing the reconstruction's consonant
                    // root — the same filter that fed the yer resolver above.
                    pc.langs = input
                        .forms
                        .iter()
                        .filter(|f| f.modern && f.primary)
                        .filter(|f| {
                            ortho::shares_consonant_root(
                                &ortho::consonant_key(&f.norm.latin),
                                &recon_key,
                            )
                        })
                        .map(|f| f.lang_code.clone())
                        .collect();
                    pc.trace.insert(
                        0,
                        RuleStep::new(
                            "proto-link",
                            format!("*{}", l.entry.word),
                            pc.form.clone(),
                            format!(
                                "Praslovjanska rekonstrukcija *{} povezana s dokazom (uvěrjenost {:.2}: {:.0}% naslědnikov, podobnost formy {:.2}, glosa {:.2}).",
                                l.entry.word,
                                l.confidence,
                                100.0 * l.desc_membership,
                                l.form_similarity,
                                l.gloss_overlap
                            ),
                            Some(DOC_DESIGN),
                        ),
                    );
                    // Etymological evidence on the proto candidate.
                    pc.evidence.push(Evidence {
                        lang_code: "sla-pro".to_string(),
                        lang_name: "praslovjansky".to_string(),
                        branch: None,
                        form: format!("*{}", l.entry.word),
                        normalized_form: l.entry.word.clone(),
                        relation: EvidenceRelation::ProtoSlavicAncestor,
                        source_url: crate::enrich::proto_source_url(&l.entry.word),
                    });
                    if !l.entry.pbs.is_empty() {
                        pc.evidence.push(Evidence {
                            lang_code: "ine-bsl-pro".to_string(),
                            lang_name: "prabaltoslavjansky".to_string(),
                            branch: None,
                            form: l.entry.pbs.clone(),
                            normalized_form: l.entry.pbs.clone(),
                            relation: EvidenceRelation::BaltoSlavicAncestor,
                            source_url: String::new(),
                        });
                    }
                    if !l.entry.pie.is_empty() {
                        pc.evidence.push(Evidence {
                            lang_code: "ine-pro".to_string(),
                            lang_name: "praindoevropejsky".to_string(),
                            branch: None,
                            form: l.entry.pie.clone(),
                            normalized_form: l.entry.pie.clone(),
                            relation: EvidenceRelation::IndoEuropeanAncestor,
                            source_url: String::new(),
                        });
                    }
                    reconstruction = Some(Reconstruction {
                        word: l.entry.word.clone(),
                        proto_balto_slavic: l.entry.pbs.clone(),
                        proto_indo_european: l.entry.pie.clone(),
                        confidence: round3(l.confidence),
                    });
                    candidates.push(pc);
                }
            }
        }
    }

    // Reflexive verbs are cited in Interslavic as `<lemma> sę` (§3). The stem was
    // reconstructed from marker-stripped cognates; append the particle here.
    if input.reflexive {
        for c in &mut candidates {
            if c.form.is_empty() {
                continue;
            }
            // Strip any residual reflexive particle first, so a form that already
            // carries one (Slovak " sa", etc.) doesn't become a double marker.
            for p in [" sę", " sa", " se", " sie", " się", " sobě"] {
                if let Some(h) = c.form.strip_suffix(p) {
                    c.form = h.trim_end().to_string();
                    break;
                }
            }
            c.form.push_str(" sę");
        }
    }

    dedupe(&mut candidates);
    candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.form.chars().count().cmp(&b.form.chars().count()))
    });
    (candidates, reconstruction)
}

/// Collapse candidates that reduce to the same standard spelling. On a tie, keep
/// the Proto-Slavic-derived one — it carries the correct flavored letters.
fn dedupe(candidates: &mut Vec<Candidate>) {
    candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(proto_rank(b).cmp(&proto_rank(a)))
    });
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<Candidate> = Vec::new();
    for c in candidates.drain(..) {
        let key = ortho::fold_key(&c.form);
        if seen.contains(&key) {
            // Already have an equal-or-better representative; but if this one is
            // Proto-Slavic-derived and the kept one is not, upgrade to flavored.
            if c.source == CandidateSource::ProtoSlavicRule {
                if let Some(existing) = out.iter_mut().find(|e| {
                    ortho::fold_key(&e.form) == key && e.source != CandidateSource::ProtoSlavicRule
                }) {
                    // Upgrade to the flavored Proto-Slavic-derived spelling and
                    // adopt its provenance so the trace/source stay honest.
                    existing.form = c.form.clone();
                    existing.trace = c.trace.clone();
                    existing.source = CandidateSource::ProtoSlavicRule;
                    existing.score = existing.score.max(c.score);
                    for ev in c.evidence {
                        existing.evidence.push(ev);
                    }
                }
            }
            continue;
        }
        seen.push(key);
        out.push(c);
    }
    *candidates = out;
}

fn proto_rank(c: &Candidate) -> u8 {
    (c.source == CandidateSource::ProtoSlavicRule) as u8
}

/// True when two forms differ only in etymological *flavor* (ě/ę/ų/å/ȯ/ć/đ, which
/// fold away in the standard alphabet) and/or the y↔i distinction. This is the
/// safe condition under which the Proto-Slavic reconstruction may override the
/// consensus: it refines the spelling of the *same* form rather than changing a
/// segmental choice the reflexes made.
fn flavor_equivalent(a: &str, b: &str) -> bool {
    let fold = |s: &str| {
        ortho::fold_key(s)
            .replace('y', "i")
            // soft-sonorant palatalization is a flavored refinement, too: the
            // reconstruction adds the softness (solj, konj, morje) the consensus
            // citation drops.
            .replace("lj", "l")
            .replace("nj", "n")
            .replace("rj", "r")
    };
    fold(a) == fold(b)
}

fn round3(x: f32) -> f32 {
    (x * 1000.0).round() / 1000.0
}
