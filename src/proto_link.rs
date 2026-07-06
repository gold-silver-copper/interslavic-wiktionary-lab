//! Leakage-free linking of a meaning to its Proto-Slavic reconstruction.
//!
//! Given only the modern Slavic cognates, the English gloss and the POS (never
//! the official Interslavic lemma), find the `sla-pro` entry that reconstructs
//! this root. Three independent signals are combined so no single noisy one
//! dominates:
//!   1. **descendant membership** — how many of the meaning's modern cognates
//!      appear in the proto entry's descendant tree;
//!   2. **derived-form similarity** — how close the proto engine's output is to
//!      the modern consensus shape;
//!   3. **gloss overlap** — English gloss tokens shared with the proto senses.
//! POS agreement gates the match. The combined confidence is thresholded so the
//! proto path is only taken when the link is trustworthy.

use crate::consensus::MeaningInput;
use crate::dump::{gloss_tokens, ProtoEntry, ProtoIndex};
use crate::model::Pos;
use crate::orthography as ortho;
use std::collections::BTreeMap;

pub struct ProtoLink<'a> {
    pub entry: &'a ProtoEntry,
    pub confidence: f32,
    pub desc_membership: f32,
    pub form_similarity: f32,
    pub gloss_overlap: f32,
}

/// Minimum combined confidence to accept a proto link. Tuned on the benchmark.
pub const DEFAULT_THRESHOLD: f32 = 0.42;

pub fn link<'a>(index: &'a ProtoIndex, input: &MeaningInput) -> Option<ProtoLink<'a>> {
    // Modern-cognate skeletons and the modal (consensus) shape.
    let mut skeletons: Vec<String> = Vec::new();
    let mut mode: BTreeMap<String, usize> = BTreeMap::new();
    for f in &input.forms {
        // Link on primary translations only; secondary synonyms would blur the
        // descendant/skeleton match.
        if !f.modern || !f.primary || f.norm.skeleton.is_empty() {
            continue;
        }
        skeletons.push(f.norm.skeleton.clone());
        *mode.entry(f.norm.skeleton.clone()).or_default() += 1;
    }
    if skeletons.is_empty() {
        return None;
    }
    let mode_skeleton = mode
        .iter()
        .max_by_key(|(_, n)| **n)
        .map(|(s, _)| s.clone())
        .unwrap_or_default();

    // Candidate proto entries: those sharing a gloss token or a descendant form.
    let mut candidates: Vec<usize> = index.gloss_candidates(&input.gloss);
    for sk in &skeletons {
        if let Some(v) = index.desc_candidates(sk) {
            for &i in v {
                if !candidates.contains(&i) {
                    candidates.push(i);
                }
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }

    let gloss_toks: Vec<String> = gloss_tokens(&input.gloss);
    let mut best: Option<ProtoLink> = None;

    for idx in candidates {
        let e = &index.entries[idx];
        let e_pos = Pos::parse(&e.pos);
        // POS gate: reject a clear mismatch (both known and different).
        if e_pos != Pos::Other && input.pos != Pos::Other && !pos_compatible(e_pos, input.pos) {
            continue;
        }

        // Signal 1: descendant membership.
        let desc_sks: Vec<String> = e
            .descendants
            .iter()
            .flat_map(|(_, w)| w.split_whitespace().map(ortho::ascii_skeleton))
            .collect();
        let hits = skeletons
            .iter()
            .filter(|sk| desc_sks.iter().any(|d| d == *sk))
            .count();
        let desc_membership = hits as f32 / skeletons.len() as f32;

        // Signal 2: derived-form similarity to the consensus shape.
        let derived = crate::proto::generate(&e.word, input.pos, input.gender).form;
        let form_similarity = if derived.is_empty() {
            0.0
        } else {
            1.0 - ortho::normalized_edit_distance(&ortho::ascii_skeleton(&derived), &mode_skeleton)
        };

        // Signal 3: gloss overlap.
        let e_toks: Vec<String> = e.glosses.iter().flat_map(|g| gloss_tokens(g)).collect();
        let overlap = gloss_toks.iter().filter(|t| e_toks.contains(t)).count();
        let gloss_overlap = if gloss_toks.is_empty() {
            0.0
        } else {
            overlap as f32 / gloss_toks.len() as f32
        };

        let confidence = 0.42 * desc_membership + 0.36 * form_similarity + 0.22 * gloss_overlap;

        if best
            .as_ref()
            .map(|b| confidence > b.confidence)
            .unwrap_or(true)
        {
            best = Some(ProtoLink {
                entry: e,
                confidence,
                desc_membership,
                form_similarity,
                gloss_overlap,
            });
        }
    }

    best.filter(|b| b.confidence >= DEFAULT_THRESHOLD)
}

/// POS compatibility, treating noun/proper-noun and the numeral/pronoun fuzz as
/// compatible, and allowing `Other` to match anything.
fn pos_compatible(a: Pos, b: Pos) -> bool {
    if a == b {
        return true;
    }
    matches!(
        (a, b),
        (Pos::Noun, Pos::ProperNoun)
            | (Pos::ProperNoun, Pos::Noun)
            | (Pos::Adjective, Pos::Adverb)
            | (Pos::Adverb, Pos::Adjective)
    )
}
