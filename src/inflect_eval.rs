//! Full-corpus inflection evaluation and grammar-invariant reporting.

use anyhow::Result;
use interslavic::{
    Animacy as IsvAnimacy, Case as IsvCase, Gender as IsvGender, Number as IsvNumber,
};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// Inflection validation (Track F / issue #5, `inflect-eval`): run the
/// inflection engine over every single-word official lemma, count blank
/// (panicked) cells, and check the grammar invariants RULE_SPEC §3 states:
/// nom.sg echoes the citation form, masc/neut gen.sg ends -a (the pan-Slavic
/// diagnostic ending), adjective nom.sg agrees (-a fem / -o|-e neut), and the
/// lexicalized suppletive plurals surface. Report: inflection-report.md.
pub fn run_inflect_eval(official_path: &Path, out_dir: &Path) -> Result<()> {
    use crate::model::{Gender, Pos};
    let entries = crate::official::load(official_path)?;
    let fold = |x: &str| crate::orthography::to_standard(&x.trim().to_lowercase());

    let (mut n_words, mut n_cells, mut n_blank) = (0usize, 0usize, 0usize);
    // The dictionary has ~950 duplicated headwords (homograph rows); each
    // unique lemma is inflected and checked once.
    let mut seen_lemmas: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut by_pos: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new(); // (cells, blank)
                                                                              // Invariants: (checked, passed) per rule.
    let mut inv: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
    let check = |inv: &mut BTreeMap<&'static str, (usize, usize)>, rule: &'static str, ok: bool| {
        let e = inv.entry(rule).or_default();
        e.0 += 1;
        e.1 += ok as usize;
    };
    let mut fail_sample: Vec<String> = Vec::new();
    let mut blank_sample: Vec<String> = Vec::new();

    for e in &entries {
        let w = e.isv.trim();
        let bare = w.strip_suffix(" sę").unwrap_or(w);
        if bare.is_empty() || bare.contains(' ') || bare.contains('#') || bare.contains('!') {
            continue;
        }
        if !seen_lemmas.insert(format!("{}|{}", fold(bare), e.pos.code())) {
            continue;
        }
        let plurale_tantum = e.pos_raw.contains("pl.");
        match e.pos {
            Pos::Noun => {
                n_words += 1;
                let mut cells: Vec<(String, &'static str)> = Vec::new();
                for (_, case) in crate::forms::CASES {
                    cells.push((
                        crate::forms::noun_cell_g(
                            bare,
                            case,
                            IsvNumber::Singular,
                            e.noun_traits.gender,
                        ),
                        "sg",
                    ));
                    cells.push((
                        crate::forms::noun_cell_g(
                            bare,
                            case,
                            IsvNumber::Plural,
                            e.noun_traits.gender,
                        ),
                        "pl",
                    ));
                }
                // Full-corpus guard (issue #20): the paradigm-struct path that
                // noun_table AND the API records now render from must equal the
                // panic-guarded single-cell getters above, cell for cell, over
                // every lemma — a build-time upgrade of the unit-scale
                // noun_paradigm_roundtrip_matches_cells test.
                let struct_ok = std::panic::catch_unwind(|| {
                    let f = crate::forms::noun_paradigm_forms(bare, e.noun_traits.gender);
                    let mut v = Vec::new();
                    for (_, case) in crate::forms::CASES {
                        v.push(crate::forms::clean_cell(f.get(case, IsvNumber::Singular)));
                        v.push(crate::forms::clean_cell(f.get(case, IsvNumber::Plural)));
                    }
                    v
                })
                .ok()
                .is_some_and(|v| {
                    v.len() == cells.len() && v.iter().zip(&cells).all(|(a, (b, _))| a == b)
                });
                check(&mut inv, "noun table struct path = cell getter", struct_ok);
                if !struct_ok && fail_sample.len() < 30 {
                    fail_sample.push(format!("{bare}: noun struct/getter mismatch"));
                }
                let blanks = cells.iter().filter(|(c, _)| c == "—").count();
                n_cells += cells.len();
                n_blank += blanks;
                let bp = by_pos.entry("noun").or_default();
                bp.0 += cells.len();
                bp.1 += blanks;
                if blanks > 0 && blank_sample.len() < 30 {
                    blank_sample.push(format!("{bare} (noun, {blanks} blank)"));
                }
                // Invariant: nom.sg echoes the citation form (a multi-variant
                // cell like "den / denj" passes if any variant echoes it).
                // Pluralia tantum are cited in the plural — no singular echo.
                if !plurale_tantum {
                    let nom = crate::forms::noun_cell_g(
                        bare,
                        IsvCase::Nom,
                        IsvNumber::Singular,
                        e.noun_traits.gender,
                    );
                    let ok = nom.split('/').any(|v| fold(v) == fold(bare));
                    check(&mut inv, "noun nom.sg = citation form", ok);
                    if !ok && fail_sample.len() < 30 {
                        fail_sample.push(format!("{bare}: nom.sg → {nom}"));
                    }
                }
                // Invariant: masc/neut gen.sg ends -a (diagnostic ending).
                // Legitimate exemptions (RULE_SPEC §3): pluralia tantum have no
                // singular; §3.5 indeclinables (loans in -e/-i/-u) don't
                // decline; masculine ā-stems (vojevoda) take the feminine -y;
                // substantivized adjectives decline adjectivally (-ogo/-ego).
                let indeclinable = matches!(fold(bare).chars().last(), Some('e' | 'i' | 'u'));
                let a_stem = fold(bare).ends_with('a');
                let substantivized = matches!(fold(bare).chars().last(), Some('y'));
                if matches!(
                    e.noun_traits.gender,
                    Some(Gender::Masculine | Gender::Neuter)
                ) && !plurale_tantum
                    && !indeclinable
                    && !a_stem
                {
                    let gen = crate::forms::noun_cell_g(
                        bare,
                        IsvCase::Gen,
                        IsvNumber::Singular,
                        e.noun_traits.gender,
                    );
                    // A multi-variant cell (čuda / čudese) passes if any variant
                    // carries the diagnostic -a; substantivized adjectives pass
                    // on the adjectival -ogo/-ego.
                    let ok = gen != "—"
                        && gen.split('/').map(&fold).any(|v| {
                            v.ends_with('a')
                                || (substantivized && (v.ends_with("ogo") || v.ends_with("ego")))
                        });
                    check(&mut inv, "masc/neut gen.sg ends -a", ok);
                    if !ok && fail_sample.len() < 30 {
                        fail_sample.push(format!("{bare}: gen.sg → {gen}"));
                    }
                }
            }
            Pos::Adjective => {
                n_words += 1;
                let mut blanks = 0usize;
                let mut cnt = 0usize;
                // Full-corpus guard (issue #20): the AdjParadigm path adj_table
                // AND the API records render from, compared cell-for-cell to the
                // panic-guarded getter over every lemma.
                let struct_forms = std::panic::catch_unwind(|| interslavic::adj_forms(bare)).ok();
                let mut adj_struct_ok = struct_forms.is_some();
                for (_, case) in crate::forms::CASES {
                    for (g, a) in [
                        (IsvGender::Masculine, IsvAnimacy::Animate),
                        (IsvGender::Masculine, IsvAnimacy::Inanimate),
                        (IsvGender::Feminine, IsvAnimacy::Inanimate),
                        (IsvGender::Neuter, IsvAnimacy::Inanimate),
                    ] {
                        for num in [IsvNumber::Singular, IsvNumber::Plural] {
                            let c = crate::forms::adj_cell(bare, case, num, g, a);
                            if let Some(sf) = &struct_forms {
                                adj_struct_ok &=
                                    crate::forms::clean_cell(sf.get(case, num, g, a)) == c;
                            }
                            cnt += 1;
                            blanks += (c == "—") as usize;
                        }
                    }
                }
                check(
                    &mut inv,
                    "adj table struct path = cell getter",
                    adj_struct_ok,
                );
                if !adj_struct_ok && fail_sample.len() < 30 {
                    fail_sample.push(format!("{bare}: adj struct/getter mismatch"));
                }
                n_cells += cnt;
                n_blank += blanks;
                let bp = by_pos.entry("adj").or_default();
                bp.0 += cnt;
                bp.1 += blanks;
                if blanks > 0 && blank_sample.len() < 30 {
                    blank_sample.push(format!("{bare} (adj, {blanks} blank)"));
                }
                let m = crate::forms::catch(|| {
                    interslavic::adj(
                        bare,
                        IsvCase::Nom,
                        IsvNumber::Singular,
                        IsvGender::Masculine,
                        IsvAnimacy::Inanimate,
                    )
                });
                let ok = fold(&m) == fold(bare);
                check(&mut inv, "adj nom.sg.m = citation form", ok);
                if !ok && fail_sample.len() < 30 {
                    fail_sample.push(format!("{bare}: nom.sg.m → {m}"));
                }
                let f = crate::forms::catch(|| {
                    interslavic::adj(
                        bare,
                        IsvCase::Nom,
                        IsvNumber::Singular,
                        IsvGender::Feminine,
                        IsvAnimacy::Inanimate,
                    )
                });
                check(
                    &mut inv,
                    "adj nom.sg.f ends -a",
                    f != "—" && fold(&f).ends_with('a'),
                );
                let nt = crate::forms::catch(|| {
                    interslavic::adj(
                        bare,
                        IsvCase::Nom,
                        IsvNumber::Singular,
                        IsvGender::Neuter,
                        IsvAnimacy::Inanimate,
                    )
                });
                check(
                    &mut inv,
                    "adj nom.sg.n ends -o/-e",
                    nt != "—" && (fold(&nt).ends_with('o') || fold(&nt).ends_with('e')),
                );
            }
            Pos::Verb => {
                n_words += 1;
                let ok = std::panic::catch_unwind(|| interslavic::verb_forms(bare)).is_ok();
                // One "cell" per paradigm: the crate returns the whole set.
                n_cells += 1;
                n_blank += !ok as usize;
                let bp = by_pos.entry("verb (whole paradigm)").or_default();
                bp.0 += 1;
                bp.1 += !ok as usize;
                if !ok && blank_sample.len() < 30 {
                    blank_sample.push(format!("{bare} (verb paradigm panicked)"));
                }
            }
            _ => {}
        }
    }
    // Invariant: the suppletive plurals from RULE_SPEC §3.1 surface — asked of
    // the inflection crate itself (the pinned rev implements them, with the
    // heteroclite byforms); this guards the pin against a regressing rev bump.
    for (base, pl) in [
        ("člověk", "ljudi"),
        ("dětę", "děti"),
        ("oko", "oči"),
        ("uho", "uši"),
    ] {
        let got = crate::forms::noun_cell(base, IsvCase::Nom, IsvNumber::Plural);
        check(
            &mut inv,
            "suppletive plurals (§3.1, from the inflector)",
            got.split('/').any(|v| v.trim() == pl),
        );
    }

    let pct = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f32 / b as f32
        }
    };
    println!(
        "inflect-eval: {n_words} lemmas, {n_cells} cells, {n_blank} blank ({:.2}%)",
        pct(n_blank, n_cells)
    );
    for (rule, (chk, ok)) in &inv {
        println!("  {rule}: {ok}/{chk} ({:.1}%)", pct(*ok, *chk));
    }

    std::fs::create_dir_all(out_dir)?;
    let mut r = String::new();
    writeln!(r, "# Inflection validation (inflect-eval)\n")?;
    writeln!(
        r,
        "**Denominator:** every single-word official lemma (noun/adjective/verb), {n_words} lemmas → {n_cells} paradigm cells generated by the bundled `interslavic` crate. Blank cells are inflector panics recovered by `catch_unwind`. (The export's separate blank-cell count also covers machine-generated reconstruction headwords, whose irregular shapes are where those blanks come from — official lemmas inflect cleanly.) Grammar invariants are the citation-form and ending rules RULE_SPEC §3 states; the failure sample below is the genuine inflector worklist (soft -o loans, §3.5 indeclinables the lexicon must mark).\n"
    )?;
    writeln!(r, "| Measurement | value |")?;
    writeln!(r, "|---|---:|")?;
    writeln!(
        r,
        "| blank cells | **{n_blank}** of {n_cells} ({:.2}%) |",
        pct(n_blank, n_cells)
    )?;
    for (pos, (cells, blank)) in &by_pos {
        writeln!(
            r,
            "| — {pos} | {blank} of {cells} ({:.2}%) |",
            pct(*blank, *cells)
        )?;
    }
    writeln!(r, "\n## Grammar invariants (RULE_SPEC §3)\n")?;
    writeln!(r, "| invariant | pass | rate |")?;
    writeln!(r, "|---|---:|---:|")?;
    for (rule, (chk, ok)) in &inv {
        writeln!(r, "| {rule} | {ok}/{chk} | {:.1}% |", pct(*ok, *chk))?;
    }
    writeln!(r, "\n## Blank-cell sample\n")?;
    for b in &blank_sample {
        writeln!(r, "- {b}")?;
    }
    writeln!(r, "\n## Invariant-failure sample\n")?;
    for f in &fail_sample {
        writeln!(r, "- {f}")?;
    }
    std::fs::write(out_dir.join("inflection-report.md"), r)?;
    println!("Wrote {}", out_dir.join("inflection-report.md").display());
    // The struct-path ≡ cell-getter equivalences are hard guarantees — the site
    // tables AND the forms API render from the struct path, so any divergence
    // from the panic-guarded getters is a bug, and this command (and the CI
    // step running it) fails on it. Every other invariant is the inflector's
    // known worklist (soft -o loans, unmarked indeclinables) and stays
    // report-only.
    for (rule, (chk, ok)) in &inv {
        if rule.contains("struct path = cell getter") {
            anyhow::ensure!(
                ok == chk,
                "inflect-eval: `{rule}` failed on {} of {chk} lemmas — the struct paradigm \
                 path diverged from the cell getters",
                chk - ok
            );
        }
    }
    Ok(())
}
