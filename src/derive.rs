//! Productive Interslavic derivation: generate a lemma's word FAMILY.
//!
//! Interslavic word formation is regular and documented (RULE_SPEC §3.4, the
//! `DERIV` correspondence tables, steen derivation.html). This module derives,
//! from one citation form, its regular derivatives — abstract `-osť`, adverb,
//! verbal noun `-ńje`, agentive `-telj` (+`-teljstvo`/`-teljka`), denominal
//! adjectives `-ny`/`-sky`, diminutive `-ka`/feminine `-ica`, negation `ne-` —
//! applying the morphophonemic seam rules (first palatalization before the
//! suffixes RULE_SPEC §2 lists, iotation before `-jeńje`, O⇒E after softs).
//!
//! The seam conventions are the *official dictionary's own* (measured, not
//! assumed): verbal nouns end `-ńje` (630 vs 12 plain `nje`), iotation writes
//! the etymological `ć`/`đ` (48 `-đeńje` vs 0 `-dženje`), labials take bare `j`
//! (61 `-[pbvm]jeńje`), `-sky` palatalizes (34 `-čsky`, 6 `-žsky`), adverbs
//! take `-o` (430) with `-e` after softs (71).
//!
//! `derive_eval::run_eval` (src/derive_eval.rs) is the leakage-free benchmark (`derive-eval`), built BEFORE the
//! layer was tuned: derivationally related official lemma pairs are mined by
//! inverse suffix lookup, the layer derives the official BASE lemma forward,
//! and the output is scored against the official DERIVATIVE (which the layer
//! never sees). A naive concatenation baseline (same suffix targets, no seam
//! rules, no flavored letters) isolates what the linguistics is worth.

use crate::model::Pos;
use crate::official::OfficialEntry;
use crate::orthography as ortho;
use std::collections::HashMap;

/// One derived family member.
#[derive(Debug, Clone)]
pub struct Derived {
    pub form: String,
    pub pos: Pos,
    /// Stable pattern id (also the eval bucket): "ost", "adv", "vnoun", …
    pub pattern: &'static str,
    /// Human label for the site (Interslavic).
    pub label: &'static str,
}

pub(crate) fn strip_final_vowel(w: &str) -> &str {
    match w.chars().last() {
        Some(c @ ('a' | 'o' | 'e' | 'y' | 'i')) => &w[..w.len() - c.len_utf8()],
        _ => w,
    }
}

/// The regular derivational family of a lemma (seam-aware). Only patterns whose
/// preconditions hold fire; the caller filters against attestation if needed.
pub fn derive_family(base: &str, pos: Pos) -> Vec<Derived> {
    // The seam-aware derivation rules moved to the interslavic crate (issue
    // #21); map this project's rich Pos to the crate's four derivation
    // categories and back. Only nouns/adjectives/verbs derive anything.
    let crate_pos = match pos {
        Pos::Noun => interslavic::derivation::Pos::Noun,
        Pos::Adjective => interslavic::derivation::Pos::Adjective,
        Pos::Verb => interslavic::derivation::Pos::Verb,
        _ => return Vec::new(),
    };
    interslavic::derivation::derive(base, crate_pos)
        .into_iter()
        .map(|d| Derived {
            form: d.form,
            pos: match d.pos {
                interslavic::derivation::Pos::Noun => Pos::Noun,
                interslavic::derivation::Pos::Adjective => Pos::Adjective,
                interslavic::derivation::Pos::Verb => Pos::Verb,
                interslavic::derivation::Pos::Adverb => Pos::Adverb,
            },
            pattern: d.pattern,
            label: d.label,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Pair mining (inverse lookup) + the derive-eval benchmark.
// ---------------------------------------------------------------------------

pub(crate) struct Pair {
    pub(crate) base: usize,
    pub(crate) derived: usize,
    pub(crate) pattern: &'static str,
}

/// Mine derivationally related official lemma pairs by inverse suffix lookup.
/// The miner only SELECTS pairs (folded-form lookup); the layer under test must
/// still produce the exact official derivative, flavored letters included.
pub(crate) fn mine_pairs(entries: &[OfficialEntry]) -> Vec<Pair> {
    // Folded form → entry indices, so inverse candidates can be looked up.
    let mut index: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        let w = e.isv.trim();
        if w.is_empty() || w.contains(' ') || w.contains('#') {
            continue;
        }
        index.entry(ortho::fold_key(w)).or_default().push(i);
    }
    let lookup = |cands: &[String], pos: Pos| -> Option<usize> {
        for c in cands {
            let key = ortho::fold_key(c);
            if let Some(v) = index.get(&key) {
                if let Some(&i) = v.iter().find(|&&i| entries[i].pos == pos) {
                    return Some(i);
                }
            }
        }
        None
    };

    let mut pairs: Vec<Pair> = Vec::new();
    let push = |bi: usize, di: usize, pattern: &'static str, pairs: &mut Vec<Pair>| {
        if bi != di {
            pairs.push(Pair {
                base: bi,
                derived: di,
                pattern,
            });
        }
    };

    for (di, d) in entries.iter().enumerate() {
        let w = d.isv.trim();
        if w.is_empty() || w.contains(' ') || w.contains('#') {
            continue;
        }
        let n = w.chars().count();
        // -osť ← adjective
        if d.pos == Pos::Noun && n > 5 {
            if let Some(stem) = w.strip_suffix("osť") {
                let cands: Vec<String> = vec![format!("{stem}y"), format!("{stem}i")];
                if let Some(bi) = lookup(&cands, Pos::Adjective) {
                    push(bi, di, "ost", &mut pairs);
                }
            }
        }
        // adverb ← adjective
        if d.pos == Pos::Adverb && n > 3 && (w.ends_with('o') || w.ends_with('e')) {
            let stem = &w[..w.len() - 1];
            let cands: Vec<String> = vec![format!("{stem}y"), format!("{stem}i")];
            if let Some(bi) = lookup(&cands, Pos::Adjective) {
                push(bi, di, "adv", &mut pairs);
            }
        }
        if d.pos == Pos::Noun && n > 5 {
            // verbal noun ← verb
            if let Some(s) = w.strip_suffix("ńje").or_else(|| w.strip_suffix("nje")) {
                let mut cands: Vec<String> = Vec::new();
                if s.ends_with('a') || s.ends_with('ě') {
                    cands.push(format!("{s}ti"));
                }
                if let Some(t) = s.strip_suffix('e') {
                    for inv in interslavic::phono::inverse_iotation(t) {
                        cands.push(format!("{inv}iti"));
                    }
                }
                if let Some(bi) = lookup(&cands, Pos::Verb) {
                    push(bi, di, "vnoun", &mut pairs);
                }
            }
            // -telj ← verb; -teljstvo / -teljka ← -telj noun
            if let Some(s) = w.strip_suffix("telj") {
                if let Some(bi) = lookup(&[format!("{s}ti")], Pos::Verb) {
                    push(bi, di, "telj", &mut pairs);
                }
            }
            if let Some(s) = w.strip_suffix("stvo") {
                if s.ends_with("telj") {
                    if let Some(bi) = lookup(&[s.to_string()], Pos::Noun) {
                        push(bi, di, "teljstvo", &mut pairs);
                    }
                }
            }
            if let Some(s) = w.strip_suffix("ka") {
                if s.ends_with("telj") {
                    if let Some(bi) = lookup(&[s.to_string()], Pos::Noun) {
                        push(bi, di, "teljka", &mut pairs);
                    }
                } else if n > 4 {
                    // diminutive -ka ← feminine noun
                    let cands: Vec<String> = interslavic::phono::inverse_palatalization(s)
                        .into_iter()
                        .map(|c| format!("{c}a"))
                        .collect();
                    if let Some(bi) = lookup(&cands, Pos::Noun) {
                        push(bi, di, "dimka", &mut pairs);
                    }
                }
            }
            // -ica ← feminine noun
            if let Some(s) = w.strip_suffix("ica") {
                if n > 5 {
                    let cands: Vec<String> = interslavic::phono::inverse_palatalization(s)
                        .into_iter()
                        .map(|c| format!("{c}a"))
                        .collect();
                    if let Some(bi) = lookup(&cands, Pos::Noun) {
                        push(bi, di, "ica", &mut pairs);
                    }
                }
            }
        }
        // -ny / -sky ← noun
        if d.pos == Pos::Adjective && n > 4 {
            for suf in ["ny", "sky"] {
                if let Some(t) = w.strip_suffix(suf) {
                    let mut cands: Vec<String> = Vec::new();
                    for inv in interslavic::phono::inverse_palatalization(t) {
                        cands.push(inv.clone());
                        for v in ["a", "o", "e"] {
                            cands.push(format!("{inv}{v}"));
                        }
                    }
                    if let Some(bi) = lookup(&cands, Pos::Noun) {
                        push(bi, di, if suf == "ny" { "ny" } else { "sky" }, &mut pairs);
                    }
                }
            }
            // ne- ← adjective
            if let Some(t) = w.strip_prefix("ne") {
                if t.chars().count() >= 4 && !t.starts_with('-') {
                    if let Some(bi) = lookup(&[t.to_string()], Pos::Adjective) {
                        push(bi, di, "ne", &mut pairs);
                    }
                }
            }
        }
    }
    // One relation, one pair: duplicate official rows (homograph/duplicate
    // lemma entries) otherwise double-count in numerator and denominator.
    let mut seen: std::collections::HashSet<(String, String, &'static str)> =
        std::collections::HashSet::new();
    pairs.retain(|p| {
        seen.insert((
            ortho::fold_key(entries[p.base].isv.trim()),
            ortho::fold_key(entries[p.derived].isv.trim()),
            p.pattern,
        ))
    });
    pairs
}

#[derive(Default)]
pub(crate) struct PatStat {
    pub(crate) n: usize,
    pub(crate) exact: usize,
    pub(crate) norm: usize,
    pub(crate) naive_exact: usize,
    pub(crate) naive_norm: usize,
}

// ---------------------------------------------------------------------------
// Off-official-base holdout (issue #37): the shipped-derivative probability.
// ---------------------------------------------------------------------------

/// The conservative cap on any shipped generated-derivative probability. Even a
/// morphologically perfect derivation carries an irreducible existence/semantic
/// risk — does the derivative actually EXIST as a word? — that a form-accuracy
/// holdout cannot measure, so no `generated` derivative ever ships above this.
pub const DERIV_PROB_CAP: f64 = 0.90;

/// Off-official-base HOLDOUT stats per pattern (issue #37). Same leakage story
/// as `run_eval`, restricted to the holdout derivatives (`is_holdout_id`, the
/// shared seeded split): the layer derives the official BASE forward and is
/// scored against the held-out official DERIVATIVE it never sees — the derivative
/// is effectively "hidden" because `derive_family` is pure and never consults the
/// dictionary. This is the leakage-free proxy population for the ABSENT
/// derivatives the export ships (measured on attested pairs held out of view).
pub(crate) fn holdout_pattern_stats(
    entries: &[OfficialEntry],
) -> std::collections::BTreeMap<&'static str, PatStat> {
    let pairs = mine_pairs(entries);
    let mut by_pat: std::collections::BTreeMap<&'static str, PatStat> = Default::default();
    for p in &pairs {
        let derived = &entries[p.derived];
        if !crate::eval::is_holdout_id(&derived.id) {
            continue;
        }
        let base = &entries[p.base];
        let got = derive_family(base.isv.trim(), base.pos)
            .into_iter()
            .find(|x| x.pattern == p.pattern);
        let gold = derived.isv.trim();
        let st = by_pat.entry(p.pattern).or_default();
        st.n += 1;
        st.exact += got
            .as_ref()
            .is_some_and(|x| ortho::exact_match(&x.form, gold)) as usize;
        st.norm += got
            .as_ref()
            .is_some_and(|x| ortho::normalized_match(&x.form, gold)) as usize;
    }
    by_pat
}

/// Wilson score-interval lower bound at ~95% (z = 1.96) for `k` successes in
/// `n` trials — a conservative binomial rate estimate that shrinks toward 0 as
/// `n` shrinks, so a thinly-observed pattern ships a lower probability.
pub(crate) fn wilson_lower(k: usize, n: usize) -> f64 {
    if n == 0 {
        return 0.0;
    }
    let z = 1.959_964_f64;
    let (k, nn) = (k as f64, n as f64);
    let p = k / nn;
    let denom = 1.0 + z * z / nn;
    let center = p + z * z / (2.0 * nn);
    let margin = z * ((p * (1.0 - p) + z * z / (4.0 * nn)) / nn).sqrt();
    ((center - margin) / denom).max(0.0)
}

/// Per-pattern shipped probability for `generated` derivatives (issue #37), fit
/// on the leakage-free off-official-base holdout. `probability(pattern)` is the
/// Wilson-95 lower bound of that pattern's holdout EXACT-match rate, capped at
/// [`DERIV_PROB_CAP`]; a pattern with NO leakage-free holdout observation has no
/// measured off-official-base accuracy, so it falls back to a low-confidence
/// suggestion at the review floor ([`crate::calibrate::REVIEW_T`], below
/// `PROPOSE_T`) rather than inheriting the pool's large-n confidence. Pure
/// function of the official dictionary → byte-reproducible.
pub struct DerivationProbabilities {
    per_pattern: std::collections::BTreeMap<&'static str, f64>,
    fallback: f64,
}

impl DerivationProbabilities {
    pub fn probability(&self, pattern: &str) -> f64 {
        self.per_pattern
            .get(pattern)
            .copied()
            .unwrap_or(self.fallback)
    }

    /// A flat table (fallback/test helper): every pattern ships `p`.
    pub fn flat(p: f64) -> Self {
        Self {
            per_pattern: Default::default(),
            fallback: p,
        }
    }
}

/// Fit per-pattern shipped probabilities from the off-official-base holdout.
pub fn pattern_probabilities(entries: &[OfficialEntry]) -> DerivationProbabilities {
    let stats = holdout_pattern_stats(entries);
    let mut per = std::collections::BTreeMap::new();
    for (pat, st) in &stats {
        per.insert(*pat, wilson_lower(st.exact, st.n).min(DERIV_PROB_CAP));
    }
    DerivationProbabilities {
        per_pattern: per,
        // A pattern with NO leakage-free holdout observation has no measured
        // off-official-base accuracy — borrowing the pool's large-n bound would
        // ship an *unobserved* pattern at the cap, contradicting "shrinks with
        // n". Ship its derivatives as a low-confidence suggestion at the review
        // floor (below PROPOSE_T), never verification-grade. (#37 review)
        fallback: crate::calibrate::REVIEW_T,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fam(base: &str, pos: Pos) -> Vec<(String, &'static str)> {
        derive_family(base, pos)
            .into_iter()
            .map(|d| (d.form, d.pattern))
            .collect()
    }

    #[test]
    fn adjective_family() {
        let f = fam("dobry", Pos::Adjective);
        assert!(f.contains(&("dobrosť".into(), "ost")));
        assert!(f.contains(&("dobro".into(), "adv")));
        assert!(f.contains(&("nedobry".into(), "ne")));
        // Soft stem takes the -e adverb (O⇒E).
        let f = fam("svěži", Pos::Adjective);
        assert!(f.contains(&("svěže".into(), "adv")));
    }

    #[test]
    fn verb_family_iotates() {
        let f = fam("prositi", Pos::Verb);
        assert!(f.contains(&("prošeńje".into(), "vnoun")));
        let f = fam("roditi", Pos::Verb);
        assert!(f.contains(&("rođeńje".into(), "vnoun")));
        let f = fam("loviti", Pos::Verb);
        assert!(f.contains(&("lovjeńje".into(), "vnoun")));
        let f = fam("dělati", Pos::Verb);
        assert!(f.contains(&("dělańje".into(), "vnoun")));
        assert!(f.contains(&("dělatelj".into(), "telj")));
        let f = fam("učiti", Pos::Verb);
        assert!(f.contains(&("učeńje".into(), "vnoun")));
        assert!(f.contains(&("učitelj".into(), "telj")));
    }

    #[test]
    fn naive_and_seam_layers_cover_the_same_patterns() {
        // The naive baseline must target the same suffixes as the seam-aware
        // layer, or the derive-eval delta measures a missing baseline pattern
        // instead of the seam morphophonemics. Guard with representative bases.
        for (base, pos) in [
            ("dobry", Pos::Adjective),
            ("prositi", Pos::Verb),
            ("dělati", Pos::Verb),
            ("kniga", Pos::Noun),
            ("učitelj", Pos::Noun),
        ] {
            let mut a: Vec<&str> = derive_family(base, pos).iter().map(|d| d.pattern).collect();
            let mut b: Vec<&str> = crate::derive_eval::naive_family(base, pos)
                .iter()
                .map(|d| d.pattern)
                .collect();
            a.sort();
            b.sort();
            assert_eq!(a, b, "pattern sets differ for {base}");
        }
    }

    #[test]
    fn bare_root_i_verbs_get_no_iotated_gerund() {
        // piti/biti/žiti take -ťje (piťje), not a garbage iotated -jeńje.
        for v in ["piti", "biti", "žiti"] {
            assert!(
                !derive_family(v, Pos::Verb)
                    .iter()
                    .any(|d| d.pattern == "vnoun"),
                "{v} must not derive an iotated gerund"
            );
        }
    }

    #[test]
    fn noun_family_palatalizes() {
        let f = fam("kniga", Pos::Noun);
        assert!(f.contains(&("knižny".into(), "ny")));
        assert!(f.contains(&("knižka".into(), "dimka")));
        let f = fam("ruka", Pos::Noun);
        assert!(f.contains(&("ručny".into(), "ny")));
        assert!(f.contains(&("ručka".into(), "dimka")));
        assert!(f.contains(&("ručica".into(), "ica")));
        let f = fam("učitelj", Pos::Noun);
        assert!(f.contains(&("učiteljstvo".into(), "teljstvo")));
        assert!(f.contains(&("učiteljka".into(), "teljka")));
    }
}
