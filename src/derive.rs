//! Productive Interslavic derivation: generate a lemma's word FAMILY.
//!
//! Interslavic word formation is regular and documented (RULE_SPEC §3.4, the
//! [DERIV] correspondence tables, steen derivation.html). This module derives,
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
//! `run_eval` is the leakage-free benchmark (`derive-eval`), built BEFORE the
//! layer was tuned: derivationally related official lemma pairs are mined by
//! inverse suffix lookup, the layer derives the official BASE lemma forward,
//! and the output is scored against the official DERIVATIVE (which the layer
//! never sees). A naive concatenation baseline (same suffix targets, no seam
//! rules, no flavored letters) isolates what the linguistics is worth.

use crate::model::Pos;
use crate::official::{self, OfficialEntry};
use crate::orthography as ortho;
use anyhow::Result;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

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

use interslavic::phono::{iotate_final, palatalize_final};

/// A stem counts as soft for the O⇒E ending alternation (RULE_SPEC §3.4).
/// One definition of softness for the whole crate: morph's.
fn ends_soft(stem: &str) -> bool {
    crate::morph::stem_is_soft(stem) || stem.ends_with("rj")
}

fn strip_final_vowel(w: &str) -> &str {
    match w.chars().last() {
        Some('a' | 'o' | 'e' | 'y' | 'i') => &w[..w.len() - w.chars().last().unwrap().len_utf8()],
        _ => w,
    }
}

/// The regular derivational family of a lemma (seam-aware). Only patterns whose
/// preconditions hold fire; the caller filters against attestation if needed.
pub fn derive_family(base: &str, pos: Pos) -> Vec<Derived> {
    let mut out = Vec::new();
    let b = base.trim();
    if b.is_empty() || b.contains(' ') {
        return out;
    }
    let push = |out: &mut Vec<Derived>, form: String, pos, pattern, label| {
        if !form.is_empty() && form != b {
            out.push(Derived {
                form,
                pos,
                pattern,
                label,
            });
        }
    };

    match pos {
        Pos::Adjective => {
            let stem = strip_final_vowel(b).to_string();
            if stem.chars().count() >= 2 {
                // Abstract noun: dobry → dobrosť.
                push(
                    &mut out,
                    format!("{stem}osť"),
                    Pos::Noun,
                    "ost",
                    "odvlečeny imennik",
                );
                // Adverb: neut.sg -o, -e after a soft stem (svěži → svěže).
                let adv_end = if ends_soft(&stem) { "e" } else { "o" };
                push(
                    &mut out,
                    format!("{stem}{adv_end}"),
                    Pos::Adverb,
                    "adv",
                    "prislovnik",
                );
                // Negation: dobry → nedobry.
                if !b.starts_with("ne") {
                    push(&mut out, format!("ne{b}"), Pos::Adjective, "ne", "negacija");
                }
            }
        }
        Pos::Verb => {
            if let Some(stem) = b.strip_suffix("ti") {
                // Verbal noun (gerund): -ati→-ańje, -ěti→-ěńje, -ovati→-ovańje,
                // -iti → iotated stem + -jeńje (prositi→prošeńje, roditi→rođeńje,
                // loviti→lovjeńje).
                if stem.ends_with('a') || stem.ends_with('ě') {
                    push(
                        &mut out,
                        format!("{stem}ńje"),
                        Pos::Noun,
                        "vnoun",
                        "odglagolny imennik",
                    );
                } else if let Some(istem) = stem.strip_suffix('i') {
                    // Root i-verbs (piti, biti, žiti) take -ťje (piťje), not the
                    // iotated -jeńje; only derive suffixal -iti stems (≥2 chars).
                    if istem.chars().count() >= 2 {
                        push(
                            &mut out,
                            format!("{}eńje", iotate_final(istem)),
                            Pos::Noun,
                            "vnoun",
                            "odglagolny imennik",
                        );
                    }
                }
                // Agentive: učiti → učitelj, izdavati → izdavatelj.
                if stem.chars().count() >= 2 && !stem.ends_with('n') {
                    push(
                        &mut out,
                        format!("{stem}telj"),
                        Pos::Noun,
                        "telj",
                        "dějatelj",
                    );
                }
            }
        }
        Pos::Noun => {
            if b.ends_with("telj") {
                // Agent-noun family: -teljstvo, -teljka.
                push(
                    &mut out,
                    format!("{b}stvo"),
                    Pos::Noun,
                    "teljstvo",
                    "odvlečeny imennik",
                );
                push(
                    &mut out,
                    format!("{b}ka"),
                    Pos::Noun,
                    "teljka",
                    "žensky dějatelj",
                );
            }
            let stem = strip_final_vowel(b).to_string();
            if stem.chars().count() >= 2 {
                // Denominal adjectives with first palatalization at the seam:
                // kniga → knižny, Grek → grečsky. (Rejected experiment: a -j
                // stem → -any allomorph (zemjany) regressed −1.4pp exact — most
                // -j stems take plain -ny; the -any class is lexical.)
                push(
                    &mut out,
                    format!("{}ny", palatalize_final(&stem)),
                    Pos::Adjective,
                    "ny",
                    "pridavnik",
                );
                push(
                    &mut out,
                    format!("{}sky", palatalize_final(&stem)),
                    Pos::Adjective,
                    "sky",
                    "pridavnik",
                );
            }
            if let Some(astem) = b.strip_suffix('a') {
                if astem.chars().count() >= 2 {
                    // Feminine diminutive: kniga → knižka, ruka → ručka.
                    push(
                        &mut out,
                        format!("{}ka", palatalize_final(astem)),
                        Pos::Noun,
                        "dimka",
                        "umenšeny imennik",
                    );
                    // -ica: voda → vodica, ruka → ručica.
                    push(
                        &mut out,
                        format!("{}ica", palatalize_final(astem)),
                        Pos::Noun,
                        "ica",
                        "umenšeny/žensky imennik",
                    );
                }
            }
        }
        _ => {}
    }
    out
}

/// The naive baseline: same suffix targets, ZERO seam rules and no flavored
/// letters (plain concatenation in the standard alphabet). The derive-eval
/// delta over this is what the morphophonemics is worth.
fn naive_family(base: &str, pos: Pos) -> Vec<Derived> {
    let mut out = Vec::new();
    let b = base.trim();
    let push = |out: &mut Vec<Derived>, form: String, pos, pattern| {
        out.push(Derived {
            form,
            pos,
            pattern,
            label: "",
        });
    };
    match pos {
        Pos::Adjective => {
            let stem = strip_final_vowel(b);
            push(&mut out, format!("{stem}ost"), Pos::Noun, "ost");
            push(&mut out, format!("{stem}o"), Pos::Adverb, "adv");
            push(&mut out, format!("ne{b}"), Pos::Adjective, "ne");
        }
        Pos::Verb => {
            if let Some(stem) = b.strip_suffix("ti") {
                let vstem = stem.strip_suffix('i').unwrap_or(stem);
                let vn = if stem.ends_with('a') || stem.ends_with('ě') {
                    format!("{stem}nje")
                } else {
                    format!("{vstem}enje")
                };
                push(&mut out, vn, Pos::Noun, "vnoun");
                push(&mut out, format!("{stem}telj"), Pos::Noun, "telj");
            }
        }
        Pos::Noun => {
            if b.ends_with("telj") {
                push(&mut out, format!("{b}stvo"), Pos::Noun, "teljstvo");
                push(&mut out, format!("{b}ka"), Pos::Noun, "teljka");
            }
            let stem = strip_final_vowel(b);
            push(&mut out, format!("{stem}ny"), Pos::Adjective, "ny");
            push(&mut out, format!("{stem}sky"), Pos::Adjective, "sky");
            if let Some(astem) = b.strip_suffix('a') {
                push(&mut out, format!("{astem}ka"), Pos::Noun, "dimka");
                push(&mut out, format!("{astem}ica"), Pos::Noun, "ica");
            }
        }
        _ => {}
    }
    out
}

// ---------------------------------------------------------------------------
// Pair mining (inverse lookup) + the derive-eval benchmark.
// ---------------------------------------------------------------------------

/// Undo the first palatalization (for inverse base lookup). Returns the
/// alternates to try INCLUDING the unchanged stem.
fn inverse_palatalization(stem: &str) -> Vec<String> {
    interslavic::phono::inverse_palatalization(stem)
}

/// Undo iotation (for inverse -jeńje lookup). Includes the unchanged stem so
/// hushing-final stems (učiti → uč-) resolve too.
fn inverse_iotation(t: &str) -> Vec<String> {
    interslavic::phono::inverse_iotation(t)
}

struct Pair {
    base: usize,
    derived: usize,
    pattern: &'static str,
}

/// Mine derivationally related official lemma pairs by inverse suffix lookup.
/// The miner only SELECTS pairs (folded-form lookup); the layer under test must
/// still produce the exact official derivative, flavored letters included.
fn mine_pairs(entries: &[OfficialEntry]) -> Vec<Pair> {
    // Folded form → entry indices, so inverse candidates can be looked up.
    let mut index: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        let w = e.isv.trim();
        if w.is_empty() || w.contains(' ') || w.contains('#') {
            continue;
        }
        index
            .entry(ortho::to_standard(&w.to_lowercase()))
            .or_default()
            .push(i);
    }
    let lookup = |cands: &[String], pos: Pos| -> Option<usize> {
        for c in cands {
            let key = ortho::to_standard(&c.to_lowercase());
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
                    for inv in inverse_iotation(t) {
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
                    let cands: Vec<String> = inverse_palatalization(s)
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
                    let cands: Vec<String> = inverse_palatalization(s)
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
                    for inv in inverse_palatalization(t) {
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
            ortho::to_standard(&entries[p.base].isv.trim().to_lowercase()),
            ortho::to_standard(&entries[p.derived].isv.trim().to_lowercase()),
            p.pattern,
        ))
    });
    pairs
}

#[derive(Default)]
struct PatStat {
    n: usize,
    exact: usize,
    norm: usize,
    naive_exact: usize,
    naive_norm: usize,
}

/// The `derive-eval` benchmark. Leakage story: input = the official BASE lemma
/// + its POS; gold = the official DERIVATIVE, which the layer never sees. Pairs
/// are mined by inverse folded-form lookup, so pair SELECTION shares alternation
/// knowledge with the layer (a selection bias, disclosed here), but the layer
/// must still produce the full official string — flavored letters, suffix
/// allomorph and seam included — forward, without the answer.
pub fn run_eval(official_path: &Path, out_dir: &Path) -> Result<()> {
    let entries = official::load(official_path)?;
    let pairs = mine_pairs(&entries);

    let mut by_pat: std::collections::BTreeMap<&'static str, PatStat> = Default::default();
    let (mut dev, mut held) = (PatStat::default(), PatStat::default());
    let mut miss_rows: Vec<String> = Vec::new();

    for p in &pairs {
        let (base, derived) = (&entries[p.base], &entries[p.derived]);
        let fam = derive_family(base.isv.trim(), base.pos);
        let naive = naive_family(base.isv.trim(), base.pos);
        let got = fam.iter().find(|x| x.pattern == p.pattern);
        let got_naive = naive.iter().find(|x| x.pattern == p.pattern);
        let gold = derived.isv.trim();
        let ex = got
            .map(|x| ortho::exact_match(&x.form, gold))
            .unwrap_or(false);
        let nm = got
            .map(|x| ortho::normalized_match(&x.form, gold))
            .unwrap_or(false);
        let nex = got_naive
            .map(|x| ortho::exact_match(&x.form, gold))
            .unwrap_or(false);
        let nnm = got_naive
            .map(|x| ortho::normalized_match(&x.form, gold))
            .unwrap_or(false);

        let st = by_pat.entry(p.pattern).or_default();
        for s in [
            st,
            if crate::eval::is_holdout_id(&derived.id) {
                &mut held
            } else {
                &mut dev
            },
        ] {
            s.n += 1;
            s.exact += ex as usize;
            s.norm += nm as usize;
            s.naive_exact += nex as usize;
            s.naive_norm += nnm as usize;
        }
        // The miss sample is the tuning artifact — publish DEV misses only,
        // so nobody tunes seam rules against holdout gold forms.
        if !nm && miss_rows.len() < 400 && !crate::eval::is_holdout_id(&derived.id) {
            miss_rows.push(format!(
                "{},{},{},{},{}",
                p.pattern,
                base.isv.trim(),
                gold,
                got.map(|x| x.form.as_str()).unwrap_or(""),
                got_naive.map(|x| x.form.as_str()).unwrap_or(""),
            ));
        }
    }

    let tot = |f: fn(&PatStat) -> usize| by_pat.values().map(f).sum::<usize>();
    let (n, ex, nm, nex, nnm) = (
        tot(|s| s.n),
        tot(|s| s.exact),
        tot(|s| s.norm),
        tot(|s| s.naive_exact),
        tot(|s| s.naive_norm),
    );
    let pct = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f32 / b as f32
        }
    };
    println!(
        "derive-eval: {n} mined official base→derivative pairs across {} patterns",
        by_pat.len()
    );
    println!(
        "  seam-aware layer: exact {:.2}%  normalized {:.2}%",
        pct(ex, n),
        pct(nm, n)
    );
    println!(
        "  naive concat    : exact {:.2}%  normalized {:.2}%",
        pct(nex, n),
        pct(nnm, n)
    );
    println!(
        "  dev/holdout (normalized): {:.2}% / {:.2}%  ({} held out)",
        pct(dev.norm, dev.n),
        pct(held.norm, held.n),
        held.n
    );

    std::fs::create_dir_all(out_dir)?;
    let mut s = String::new();
    writeln!(s, "# Derivation benchmark (derive-eval)\n")?;
    writeln!(
        s,
        "**Denominator:** {n} derivationally related official lemma pairs, mined by inverse suffix lookup over the official dictionary ({} entries). **Leakage story:** the layer receives the official *base* lemma + POS and must produce the official *derivative* forward; it never sees the derivative. Pair *selection* shares alternation knowledge with the layer (a disclosed bias — pairs the miner cannot align are excluded), but forward generation must still choose the right suffix allomorph, seam alternation and flavored spelling. A small share of mined pairs are string coincidences rather than true derivations (e.g. vino→vinny 'wine→guilty'); they inflate both layers symmetrically and are counted in the disclosed selection bias. **Dev/holdout (seeded id split):** normalized {:.2}% / {:.2}% ({} held out).\n",
        entries.len(),
        pct(dev.norm, dev.n),
        pct(held.norm, held.n),
        held.n
    )?;
    writeln!(
        s,
        "| Metric | seam-aware layer | naive concat baseline | Δ |"
    )?;
    writeln!(s, "|---|---:|---:|---:|")?;
    writeln!(
        s,
        "| exact | **{:.2}%** | {:.2}% | {:+.2}pp |",
        pct(ex, n),
        pct(nex, n),
        pct(ex, n) - pct(nex, n)
    )?;
    writeln!(
        s,
        "| normalized | **{:.2}%** | {:.2}% | {:+.2}pp |",
        pct(nm, n),
        pct(nnm, n),
        pct(nm, n) - pct(nnm, n)
    )?;
    writeln!(s, "\n## Per pattern\n")?;
    writeln!(
        s,
        "| pattern | pairs | exact | normalized | naive exact | naive normalized |"
    )?;
    writeln!(s, "|---|---:|---:|---:|---:|---:|")?;
    for (pat, st) in &by_pat {
        writeln!(
            s,
            "| {} | {} | {:.2}% | {:.2}% | {:.2}% | {:.2}% |",
            pat,
            st.n,
            pct(st.exact, st.n),
            pct(st.norm, st.n),
            pct(st.naive_exact, st.n),
            pct(st.naive_norm, st.n)
        )?;
    }
    writeln!(
        s,
        "\n## Nearest misses (dev split only — holdout misses are never published)\n"
    )?;
    writeln!(
        s,
        "```\npattern,base,official,derived,naive\n{}\n```",
        miss_rows.join("\n")
    )?;
    std::fs::write(out_dir.join("derivation-report.md"), s)?;
    println!("Wrote {}", out_dir.join("derivation-report.md").display());
    Ok(())
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
            let mut b: Vec<&str> = naive_family(base, pos).iter().map(|d| d.pattern).collect();
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
