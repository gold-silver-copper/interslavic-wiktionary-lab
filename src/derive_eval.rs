//! The derivation benchmark writer (V15 item 6).
//!
//! Moved verbatim out of src/derive.rs: this is answer-reading diagnostic
//! machinery (the report reaches into eval's split and publishes dev-only
//! misses), and production modules must not host it. The production
//! surface — derive_family, pattern_probabilities — stays in derive.rs.

use crate::derive::{
    derive_family, holdout_pattern_stats, mine_pairs, pattern_probabilities, strip_final_vowel,
    Derived, PatStat, DERIV_PROB_CAP,
};
use crate::model::Pos;
use crate::official;
use crate::orthography as ortho;
use anyhow::Result;
use std::fmt::Write as _;
use std::path::Path;

/// The naive baseline: same suffix targets, ZERO seam rules and no flavored
/// letters (plain concatenation in the standard alphabet). The derive-eval
/// delta over this is what the morphophonemics is worth.
pub(crate) fn naive_family(base: &str, pos: Pos) -> Vec<Derived> {
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

/// The `derive-eval` benchmark. Leakage story: input = the official BASE lemma
/// + its POS; gold = the official DERIVATIVE, which the layer never sees. Pairs
///   are mined by inverse folded-form lookup, so pair SELECTION shares alternation
///   knowledge with the layer (a selection bias, disclosed here), but the layer
///   must still produce the full official string — flavored letters, suffix
///   allomorph and seam included — forward, without the answer.
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
        let ex = got.is_some_and(|x| ortho::exact_match(&x.form, gold));
        let nm = got.is_some_and(|x| ortho::normalized_match(&x.form, gold));
        let nex = got_naive.is_some_and(|x| ortho::exact_match(&x.form, gold));
        let nnm = got_naive.is_some_and(|x| ortho::normalized_match(&x.form, gold));

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
                got.map_or("", |x| x.form.as_str()),
                got_naive.map_or("", |x| x.form.as_str()),
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
    // --- Off-official-base holdout (issue #37): the leakage-free proxy for the
    // ABSENT derivatives the export ships, and the source of their probability.
    let hstats = holdout_pattern_stats(&entries);
    let probs = pattern_probabilities(&entries);
    let (mut hk, mut hn) = (0usize, 0usize);
    for st in hstats.values() {
        hk += st.exact;
        hn += st.n;
    }
    println!(
        "  off-official-base holdout (issue #37): {} pairs, exact {:.2}%  (shipped p = per-pattern Wilson-95 lower bound of exact, cap {:.2})",
        hn,
        pct(hk, hn),
        DERIV_PROB_CAP
    );
    writeln!(
        s,
        "\n## Off-official-base holdout (issue #37) — shipped derivative probability\n"
    )?;
    writeln!(
        s,
        "The `generated` derivatives the export ships off attested official bases are ABSENT from the dictionary, so they have no gold and cannot be scored directly. This is the leakage-free proxy: hold out a slice of official derivatives by `is_holdout_id` (the shared seeded split), hide them from view, derive them off their still-visible official base, and score the derivation. Because `derive_family` never consults the dictionary, the hidden derivative is genuinely unseen. The shipped `probability` for a pattern is the **Wilson 95% lower bound of its holdout EXACT-match rate** (conservative: it shrinks toward 0 as the sample thins), capped at {:.2} — an irreducible existence/semantics margin the form-accuracy proxy cannot measure (the holdout asks *did we spell the derivative right*, not *is the derivative a real word*). Overall holdout exact **{:.2}%** over **{}** held-out pairs. This is NOT the {:.2}% derive-eval headline above, which scores a different, both-attested population.\n",
        DERIV_PROB_CAP,
        pct(hk, hn),
        hn,
        pct(ex, n)
    )?;
    writeln!(
        s,
        "| pattern | holdout pairs | exact | normalized | shipped probability |"
    )?;
    writeln!(s, "|---|---:|---:|---:|---:|")?;
    for (pat, st) in &hstats {
        writeln!(
            s,
            "| {} | {} | {:.2}% | {:.2}% | {:.3} |",
            pat,
            st.n,
            pct(st.exact, st.n),
            pct(st.norm, st.n),
            probs.probability(pat)
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
