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
//!
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
    /// An Interslavic prefix to prepend to the derived bare-root form, when the
    /// meaning was linked by stripping a shared prefix off the cognates.
    pub prefix: Option<String>,
}

/// Minimum combined confidence to accept a proto link. Tuned on the benchmark.
pub const DEFAULT_THRESHOLD: f32 = 0.42;

/// Deep-corroboration rescue band (issue #76, pre-registered — do not tune):
/// a best candidate whose confidence lands in `[RESCUE_FLOOR, DEFAULT_THRESHOLD)`
/// is accepted anyway when at least [`RESCUE_SHARE`] of the meaning's primary
/// cognates' own Wiktionary etymologies name the candidate's Proto-Balto-Slavic
/// or PIE ancestor. The 0.42/0.36/0.22 weight mix and the gate itself stay put.
const RESCUE_FLOOR: f32 = 0.34;
const RESCUE_SHARE: f32 = 0.5;

/// Link by Wiktionary's **explicit** etymology: if ≥2 primary cognates are
/// attested (via `inh`/`der` templates) as continuing the same Proto-Slavic
/// ancestor, use that ancestor directly instead of the fuzzy descendant+gloss
/// match. This is what the corpus site does, and it is far more precise — the
/// ancestor is Wiktionary's stated etymology, not a guess.
pub fn link_explicit<'a>(index: &'a ProtoIndex, input: &MeaningInput) -> Option<ProtoLink<'a>> {
    let mut by_proto: BTreeMap<&str, std::collections::BTreeSet<&str>> = BTreeMap::new();
    for f in &input.forms {
        if !f.modern || !f.primary {
            continue;
        }
        if let Some(proto) = index.etym_ancestor(&f.lang_code, &f.norm.latin) {
            by_proto
                .entry(proto)
                .or_default()
                .insert(f.lang_code.as_str());
        }
    }
    let (proto, langs) = by_proto
        .iter()
        .map(|(p, s)| (*p, s.len()))
        .max_by_key(|(_, n)| *n)?;
    if langs < 2 {
        return None; // need corroboration across ≥2 languages
    }
    let idx = index.entry_by_word(proto.trim_start_matches('*'))?;
    Some(ProtoLink {
        entry: &index.entries[idx],
        confidence: (0.75 + 0.05 * langs as f32).min(0.95),
        desc_membership: 1.0,
        form_similarity: 1.0,
        gloss_overlap: 0.0,
        prefix: None,
    })
}

/// Diagnostic-only oracle linker (V7 §2.4b): among the reconstructions the real
/// linker would consider (the gloss + descendant candidate set), return the one
/// whose *derived form* is closest to the official lemma. This READS THE ANSWER
/// and must never feed production — it measures the linking stage's upper bound.
pub fn link_oracle<'a>(
    index: &'a ProtoIndex,
    input: &MeaningInput,
    official: &str,
) -> Option<ProtoLink<'a>> {
    let target = ortho::fold_key(official.trim());
    let mut candidates: Vec<usize> = index.gloss_candidates(&input.gloss);
    for f in &input.forms {
        if !f.modern || !f.primary || f.norm.skeleton.is_empty() {
            continue;
        }
        let sk = ortho::ascii_skeleton(&f.norm.latin);
        if let Some(v) = index.desc_candidates(&sk) {
            for &i in v {
                if !candidates.contains(&i) {
                    candidates.push(i);
                }
            }
        }
    }
    let mut best: Option<(&ProtoEntry, f32)> = None;
    for idx in candidates {
        let e = &index.entries[idx];
        let e_pos = Pos::parse(&e.pos);
        if e_pos != Pos::Other && input.pos != Pos::Other && !pos_compatible(e_pos, input.pos) {
            continue;
        }
        let derived = crate::proto::generate(&e.word, input.pos, input.gender).form;
        if derived.is_empty() {
            continue;
        }
        let d = ortho::normalized_edit_distance(&ortho::fold_key(&derived), &target);
        if best.as_ref().is_none_or(|(_, bd)| d < *bd) {
            best = Some((e, d));
        }
    }
    // A *perfect* linker also knows when NOT to link: only take the reconstruction
    // when its derived form is genuinely close to the official lemma (else the
    // consensus surface is the better source and forcing a proto override hurts).
    // This makes the oracle measure the true linking headroom, not "always trust
    // the closest of a bad set".
    best.filter(|(_, d)| *d < 0.20).map(|(e, d)| ProtoLink {
        entry: e,
        confidence: 0.95,
        desc_membership: 1.0,
        form_similarity: 1.0 - d,
        gloss_overlap: 0.0,
        prefix: None,
    })
}

pub fn link<'a>(
    index: &'a ProtoIndex,
    input: &MeaningInput,
    try_prefix: bool,
    deep_corroboration: bool,
) -> Option<ProtoLink<'a>> {
    // Primary-cognate phonemic-Latin forms (secondary synonyms would blur it).
    let latins: Vec<String> = input
        .forms
        .iter()
        .filter(|f| f.modern && f.primary && !f.norm.skeleton.is_empty())
        .map(|f| f.norm.latin.clone())
        .collect();
    if latins.is_empty() {
        return None;
    }
    let gloss_toks = gloss_tokens(&input.gloss);

    // Direct attempt: match the full cognate skeletons.
    let full_skeletons: Vec<String> = latins.iter().map(|l| ortho::ascii_skeleton(l)).collect();
    if let Some(l) = link_core(
        index,
        &full_skeletons,
        &gloss_toks,
        input,
        None,
        deep_corroboration,
    ) {
        return Some(l);
    }

    // Fallback: strip a shared verbal/nominal prefix off the cognates and link
    // the bare root (råzprostirati → the *prostirati reconstruction), then the
    // pipeline re-attaches the Interslavic prefix. The deep-corroboration
    // rescue does not apply here: a rescued-then-discounted confidence would
    // land below the threshold again anyway.
    if try_prefix {
        if let Some((isv_prefix, bare)) = strip_shared_prefix(&latins) {
            let bare_sk: Vec<String> = bare.iter().map(|l| ortho::ascii_skeleton(l)).collect();
            if let Some(mut l) = link_core(
                index,
                &bare_sk,
                &gloss_toks,
                input,
                Some(&isv_prefix),
                false,
            ) {
                // Stripped links are slightly less certain.
                l.confidence *= 0.94;
                if l.confidence >= DEFAULT_THRESHOLD {
                    return Some(l);
                }
            }
        }
    }
    None
}

/// The scoring core: find the best proto entry for a set of cognate skeletons.
fn link_core<'a>(
    index: &'a ProtoIndex,
    skeletons: &[String],
    gloss_toks: &[String],
    input: &MeaningInput,
    prefix: Option<&str>,
    deep_corroboration: bool,
) -> Option<ProtoLink<'a>> {
    if skeletons.is_empty() {
        return None;
    }
    let mut mode: BTreeMap<&str, usize> = BTreeMap::new();
    for sk in skeletons {
        *mode.entry(sk.as_str()).or_default() += 1;
    }
    let mode_skeleton = mode
        .iter()
        .max_by_key(|(_, n)| **n)
        .map(|(s, _)| s.to_string())
        .unwrap_or_default();

    let mut candidates: Vec<usize> = index.gloss_candidates(&input.gloss);
    for sk in skeletons {
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

    let mut best: Option<ProtoLink> = None;
    for idx in candidates {
        let e = &index.entries[idx];
        let e_pos = Pos::parse(&e.pos);
        if e_pos != Pos::Other && input.pos != Pos::Other && !pos_compatible(e_pos, input.pos) {
            continue;
        }
        let desc_sks: Vec<String> = e
            .descendants
            .iter()
            .flat_map(|(lang, w)| {
                w.split_whitespace()
                    .map(|x| crate::normalize::desc_skeleton(lang, x))
                    .collect::<Vec<_>>()
            })
            .collect();
        let hits = skeletons
            .iter()
            .filter(|sk| desc_sks.iter().any(|d| d == *sk))
            .count();
        let desc_membership = hits as f32 / skeletons.len() as f32;

        let derived = crate::proto::generate(&e.word, input.pos, input.gender).form;
        let form_similarity = if derived.is_empty() {
            0.0
        } else {
            1.0 - ortho::normalized_edit_distance(&ortho::ascii_skeleton(&derived), &mode_skeleton)
        };

        let e_toks: Vec<String> = e.glosses.iter().flat_map(|g| gloss_tokens(g)).collect();
        let overlap = gloss_toks.iter().filter(|t| e_toks.contains(t)).count();
        let gloss_overlap = if gloss_toks.is_empty() {
            0.0
        } else {
            overlap as f32 / gloss_toks.len() as f32
        };

        let confidence = 0.42 * desc_membership + 0.36 * form_similarity + 0.22 * gloss_overlap;
        if best.as_ref().is_none_or(|b| confidence > b.confidence) {
            best = Some(ProtoLink {
                entry: e,
                confidence,
                desc_membership,
                form_similarity,
                gloss_overlap,
                prefix: prefix.map(str::to_string),
            });
        }
    }
    let mut best = best?;
    if best.confidence >= DEFAULT_THRESHOLD {
        return Some(best);
    }
    // Deep-ancestor rescue (issue #76): a near-threshold candidate is accepted
    // when the cognates' own etymologies independently name its PBS/PIE
    // ancestor. The confidence is floored to the gate value, not raised above
    // it — corroboration grows coverage, it never outranks a real link.
    if deep_corroboration
        && best.confidence >= RESCUE_FLOOR
        && deep_share(index, input, best.entry) >= RESCUE_SHARE
    {
        best.confidence = DEFAULT_THRESHOLD;
        return Some(best);
    }
    None
}

/// The share of the meaning's primary modern cognates whose own Wiktionary
/// etymology names the candidate reconstruction's deep (Proto-Balto-Slavic or
/// PIE) ancestor, fold-compared ([`crate::normalize::fold_deep_token`]).
fn deep_share(index: &ProtoIndex, input: &MeaningInput, entry: &ProtoEntry) -> f32 {
    let pbs = crate::normalize::fold_deep_token(&entry.pbs);
    let pie = crate::normalize::fold_deep_token(&entry.pie);
    if pbs.is_empty() && pie.is_empty() {
        return 0.0;
    }
    let (mut hits, mut n) = (0usize, 0usize);
    for f in input.forms.iter().filter(|f| f.modern && f.primary) {
        n += 1;
        if let Some(toks) = index.deep_ancestors(&f.lang_code, &f.norm.latin) {
            if toks
                .iter()
                .any(|t| (!pbs.is_empty() && *t == pbs) || (!pie.is_empty() && *t == pie))
            {
                hits += 1;
            }
        }
    }
    if n == 0 {
        0.0
    } else {
        hits as f32 / n as f32
    }
}

/// Interslavic prefixes and the surface variants that mark them across the
/// modern languages. Longest variants first so `pred`/`raz` win over `pre`/`ra`.
const PREFIX_VARIANTS: &[(&str, &str)] = &[
    ("prěd", "prěd"),
    ("pred", "prěd"),
    ("perě", "prě"),
    ("pere", "prě"),
    ("prěs", "prě"),
    ("raz", "råz"),
    ("ras", "råz"),
    ("roz", "råz"),
    ("ros", "råz"),
    ("bez", "bez"),
    ("bes", "bez"),
    ("voz", "vȯz"),
    ("pod", "pod"),
    ("nad", "nad"),
    ("pri", "pri"),
    ("pro", "pro"),
    ("prě", "prě"),
    ("pre", "prě"),
    ("iz", "iz"),
    ("od", "od"),
    ("ot", "od"),
    ("ob", "ob"),
    ("na", "na"),
    ("po", "po"),
    ("za", "za"),
    ("do", "do"),
    ("vy", "vy"),
];

/// If a shared Interslavic prefix is stripped from a majority of the cognates,
/// return it and the bare stems (of the forms that carried it).
fn strip_shared_prefix(latins: &[String]) -> Option<(String, Vec<String>)> {
    let mut by_prefix: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for l in latins {
        if let Some((isv, bare)) = strip_one(l) {
            by_prefix.entry(isv).or_default().push(bare);
        }
    }
    // The prefix carried by the most cognates, needing corroboration (>=2 forms
    // and at least half of them) to avoid stripping a root-initial syllable.
    let total = latins.len();
    by_prefix
        .into_iter()
        .filter(|(_, v)| v.len() >= 2 && v.len() * 2 >= total)
        .max_by_key(|(_, v)| v.len())
        .map(|(isv, bare)| (isv.to_string(), bare))
}

fn strip_one(latin: &str) -> Option<(&'static str, String)> {
    for (variant, isv) in PREFIX_VARIANTS {
        if let Some(rest) = latin.strip_prefix(variant) {
            let n = rest.chars().count();
            let vowel_initial = rest
                .chars()
                .next()
                .is_none_or(|c| "aeiouyěęǫųåȯ".contains(c));
            // Consonant-initial stems are the safe common case. Vowel-initial
            // stems (raz+um→umeti) are allowed only for the longer, unambiguous
            // prefixes and a longer stem — the short prefixes (na/po/za/do/u)
            // start too many roots to strip before a vowel. The gloss-overlap
            // gate rejects any wrong bare root either way.
            let ok = if vowel_initial {
                variant.chars().count() >= 3 && n >= 4
            } else {
                n >= 3
            };
            if ok {
                return Some((isv, rest.to_string()));
            }
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_a_corroborated_prefix_onto_a_consonant_stem() {
        let latins = ["napisati".into(), "napisati".into(), "napisat".into()];
        let (isv, bare) = strip_shared_prefix(&latins).unwrap();
        assert_eq!(isv, "na");
        assert!(bare.iter().all(|b| b.starts_with("pisat")), "{bare:?}");
    }

    #[test]
    fn requires_corroboration() {
        // A single form carrying a prefix isn't enough to strip it.
        assert!(strip_shared_prefix(&["napisati".into()]).is_none());
    }

    #[test]
    fn strips_vowel_initial_stem_only_for_long_prefixes() {
        // A long, unambiguous prefix may strip before a vowel: raz+umeti.
        assert_eq!(strip_one("razumeti"), Some(("råz", "umeti".to_string())));
        // But a short prefix before a vowel is left intact (na+uka is not a
        // safe strip — too many roots start that way).
        assert!(strip_one("nauka").is_none());
        // And a too-short stem is left intact (iz+ba).
        assert!(strip_one("izba").is_none());
    }
}
