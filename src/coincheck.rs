//! `coin-check` — validate a coined word against the lexicon's own evidence
//! (V12 item 6). Real translation work needs a handful of unavoidable
//! coinages (fantasy names with no dictionary answer); this gives the coiner
//! four deterministic verdict axes, entirely from existing machinery:
//!
//! 1. **Phonotactics** — the ISV alphabet plus the character-bigram
//!    inventory attested by the official lemmas themselves (no hand list).
//! 2. **Collision** — folded lookup in the same form index `check-text`
//!    builds: an existing lemma or inflected form is reported.
//! 3. **False-friend risk** — the V10/V11 collision machinery run for this
//!    one surface across the ten languages' caches.
//! 4. **Declinability** — the paradigm the `interslavic` crate produces for
//!    the guessed POS/gender, so the coiner sees how the word will inflect
//!    and can adjust the ending.

use crate::official;
use anyhow::Result;
use std::collections::{BTreeSet, HashSet};
use std::path::Path;

/// Attested character-bigram inventory over the official lemmas' folded
/// surfaces (plus the attested word-initial and word-final character sets).
pub struct Phonotactics {
    bigrams: HashSet<(char, char)>,
    initials: HashSet<char>,
    finals: HashSet<char>,
}

impl Phonotactics {
    pub fn from_official(entries: &[official::OfficialEntry]) -> Self {
        let mut p = Phonotactics {
            bigrams: HashSet::new(),
            initials: HashSet::new(),
            finals: HashSet::new(),
        };
        for e in entries {
            for byform in e.citation_byforms() {
                for token in byform.form.split_whitespace() {
                    let folded = crate::forms::form_key(token);
                    let chars: Vec<char> = folded.chars().collect();
                    if chars.is_empty() {
                        continue;
                    }
                    p.initials.insert(chars[0]);
                    p.finals.insert(*chars.last().unwrap());
                    for w in chars.windows(2) {
                        p.bigrams.insert((w[0], w[1]));
                    }
                }
            }
        }
        p
    }

    /// Un-attested sequences in a folded surface: illegal letters (outside
    /// the ISV alphabet), unattested bigrams, unattested initial/final.
    pub fn violations(&self, folded: &str) -> Vec<String> {
        let mut out = Vec::new();
        let chars: Vec<char> = folded.chars().collect();
        for &c in &chars {
            if !crate::flavorize::is_isv_letter(c) {
                out.push(format!("illegal letter '{c}'"));
            }
        }
        if let Some(&first) = chars.first() {
            if !self.initials.contains(&first) {
                out.push(format!("word-initial '{first}' unattested"));
            }
        }
        if let Some(&last) = chars.last() {
            if !self.finals.contains(&last) {
                out.push(format!("word-final '{last}' unattested"));
            }
        }
        for w in chars.windows(2) {
            if !self.bigrams.contains(&(w[0], w[1])) {
                out.push(format!(
                    "cluster '{}{}' unattested in official lemmas",
                    w[0], w[1]
                ));
            }
        }
        out.dedup();
        out
    }
}

/// Guess the POS a coined citation form will be read as, by its ending —
/// the same convention the dictionary uses (verbs cite -ti, adjectives -y/-i).
fn guess_pos(folded: &str) -> crate::model::Pos {
    if folded.ends_with("ti") {
        crate::model::Pos::Verb
    } else if folded.ends_with('y') || folded.ends_with('i') {
        crate::model::Pos::Adjective
    } else {
        crate::model::Pos::Noun
    }
}

/// Declared metadata overriding the ending-based guess (V13 item 2): a real
/// consumer controls gender/animacy explicitly (`ISV::noun_with`), so the
/// declinability preview must be able to render what the project will
/// actually do — while still printing the guess and flagging divergence.
#[derive(Debug, Default)]
pub struct Overrides {
    pub pos: Option<crate::model::Pos>,
    pub gender: Option<crate::model::Gender>,
    pub animate: Option<bool>,
    pub gloss: Option<String>,
    pub lexicon_row: bool,
}

impl Overrides {
    pub fn parse(
        pos: Option<&str>,
        gender: Option<&str>,
        animacy: Option<&str>,
        gloss: Option<String>,
        lexicon_row: bool,
    ) -> Result<Self> {
        let pos = match pos {
            None => None,
            Some("noun") => Some(crate::model::Pos::Noun),
            Some("adj") => Some(crate::model::Pos::Adjective),
            Some("verb") => Some(crate::model::Pos::Verb),
            Some(other) => anyhow::bail!("--pos must be noun|adj|verb, got '{other}'"),
        };
        let gender = match gender {
            None => None,
            Some("m") => Some(crate::model::Gender::Masculine),
            Some("f") => Some(crate::model::Gender::Feminine),
            Some("n") => Some(crate::model::Gender::Neuter),
            Some(other) => anyhow::bail!("--gender must be m|f|n, got '{other}'"),
        };
        let animate = match animacy {
            None => None,
            Some("anim") => Some(true),
            Some("inanim") => Some(false),
            Some(other) => anyhow::bail!("--animacy must be anim|inanim, got '{other}'"),
        };
        anyhow::ensure!(
            !lexicon_row || gloss.as_deref().is_some_and(|g| !g.trim().is_empty()),
            "--lexicon-row needs --gloss <english concept> (the lexicon's consistency check reads it)"
        );
        Ok(Overrides {
            pos,
            gender,
            animate,
            gloss,
            lexicon_row,
        })
    }
}

fn gender_label(g: interslavic::Gender) -> &'static str {
    match g {
        interslavic::Gender::Masculine => "m",
        interslavic::Gender::Feminine => "f",
        interslavic::Gender::Neuter => "n",
    }
}

fn model_gender_label(g: crate::model::Gender) -> &'static str {
    match g {
        crate::model::Gender::Masculine => "m",
        crate::model::Gender::Feminine => "f",
        crate::model::Gender::Neuter => "n",
        crate::model::Gender::Unknown => "?",
    }
}

/// Validate one constructed lexicon-row line through the SAME parse +
/// semantic rules `check-text --lexicon` applies, returning the row on
/// success. A `#`-initial word makes the line parse as a lexicon COMMENT
/// (empty result) — that must be a rejection, never an index panic.
fn validated_lexicon_row(index: &crate::check::Index, row: String, word: &str) -> Result<String> {
    crate::check::parse_lexicon(&row)
        .and_then(|rows| {
            let parsed = rows.into_iter().next().ok_or_else(|| {
                anyhow::anyhow!("the word makes the row parse as a lexicon comment, not a row")
            })?;
            crate::check::validate_lexicon_row(index, &parsed)
        })
        .map(|_pinned| row)
        .map_err(|e| anyhow::anyhow!("--lexicon-row rejected for '{word}': {e:#}"))
}

/// The `coin-check` CLI entry point.
pub fn run(official_path: &Path, word: &str, json: bool, overrides: &Overrides) -> Result<()> {
    let word = word.trim();
    anyhow::ensure!(
        !word.is_empty() && !word.contains(' '),
        "coin-check takes exactly one single-token word"
    );
    let entries = official::load(official_path)?;
    let folded = crate::forms::form_key(word);

    // Axis 1: phonotactics.
    let phono = Phonotactics::from_official(&entries);
    let violations = phono.violations(&folded);

    // Axis 2: collision with the existing lexicon (same index as check-text;
    // novel-word proposals included so a coinage can't shadow one).
    let index = crate::check::build_index(
        &entries,
        Some(Path::new("data/novel-words.tsv")),
        Default::default(),
    );
    let collisions: Vec<serde_json::Value> = index
        .by_key
        .get(&folded)
        .map(|recs| {
            let mut seen: BTreeSet<(String, &str, &str)> = BTreeSet::new();
            recs.iter()
                .filter(|r| seen.insert((r.lemma.clone(), r.pos, r.status)))
                .map(|r| {
                    serde_json::json!({
                        "lemma": r.lemma,
                        "pos": r.pos,
                        "status": r.status,
                        "as": if r.source == "lemma" { "lemma" } else { "inflected form" },
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Axis 3: false-friend readings across the ten languages' caches.
    let evidence = crate::dump::LemmaCorpus::load(Path::new(crate::DEFAULT_LEMMA_CACHE)).ok();
    let raw = crate::dump::RawSlavicCorpus::load(Path::new(crate::DEFAULT_RAW_LEMMA_CACHE)).ok();
    let readings = crate::falsefriends::surface_readings(word, evidence.as_ref(), raw.as_ref());

    // Axis 4: declinability. The GUESS comes from the ending (POS) and the
    // crate's own inference (gender/animacy); declared metadata overrides
    // the rendered paradigm, and divergence from the guess is flagged.
    let guessed_pos = guess_pos(&folded);
    let pos = overrides.pos.unwrap_or(guessed_pos);
    anyhow::ensure!(
        pos == crate::model::Pos::Noun
            || (overrides.gender.is_none() && overrides.animate.is_none()),
        "--gender/--animacy apply to nouns; '{word}' is being checked as {}",
        pos.code()
    );
    // The crate's guessed gender/animacy for the noun reading (what
    // `interslavic::noun_forms` would silently do).
    let guess = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        interslavic::noun_forms(word)
    }))
    .ok()
    .map(|p| {
        (
            gender_label(p.gender),
            p.animacy == interslavic::Animacy::Animate,
        )
    });
    let (guessed_gender, guessed_animate) = match &guess {
        Some((g, a)) => (Some(*g), Some(*a)),
        None => (None, None),
    };

    let overridden = pos == crate::model::Pos::Noun
        && (overrides.gender.is_some() || overrides.animate.is_some());
    let mut sink = crate::forms::RecordSink::default();
    if overridden {
        // Fall back to the crate's guess for whichever axis was NOT declared,
        // so the paradigm always reflects one concrete (gender, animacy).
        let gender = overrides.gender.or(match guessed_gender {
            Some("m") => Some(crate::model::Gender::Masculine),
            Some("f") => Some(crate::model::Gender::Feminine),
            Some("n") => Some(crate::model::Gender::Neuter),
            _ => None,
        });
        let animate = overrides.animate.or(guessed_animate).unwrap_or(false);
        crate::forms::project_paradigm_records(
            &mut sink,
            word,
            pos,
            gender,
            animate,
            "generated",
            "",
        );
    } else {
        crate::forms::paradigm_records(&mut sink, word, pos, None, 0, "generated", None, "");
    }
    let mut paradigm: Vec<(String, String)> = sink
        .into_records()
        .into_iter()
        .filter(|r| r.source != "lemma")
        .map(|r| (r.analyses.join("/"), r.form))
        .collect();
    paradigm.sort();
    paradigm.dedup();

    // Divergence between declaration and guess, spelled out for the coiner.
    let mut divergences: Vec<String> = Vec::new();
    if let Some(declared) = overrides.pos {
        if declared != guessed_pos {
            divergences.push(format!(
                "ending suggests {}; you declared {}",
                guessed_pos.code(),
                declared.code()
            ));
        }
    }
    if pos == crate::model::Pos::Noun {
        if let (Some(declared), Some(guessed)) = (overrides.gender, guessed_gender) {
            if model_gender_label(declared) != guessed {
                divergences.push(format!(
                    "ending suggests gender {guessed}; you declared {}",
                    model_gender_label(declared)
                ));
            }
        }
        if let (Some(declared), Some(guessed)) = (overrides.animate, guessed_animate) {
            if declared != guessed {
                let label = |a: bool| if a { "anim" } else { "inanim" };
                divergences.push(format!(
                    "crate guesses {}; you declared {}",
                    label(guessed),
                    label(declared)
                ));
            }
        }
    }

    // The project-lexicon hand-off (V13 item 2): emit the exact item-1 TSV
    // row, validated by the SAME rules `check-text --lexicon` applies — an
    // invalid row must fail here, not later in CI. The failure surfaces
    // AFTER the full four-axis report (like check-text's summary gate): the
    // report is precisely the diagnostic that explains a rejection.
    let lexicon_row: Option<Result<String>> = if overrides.lexicon_row {
        let (g, a) = if pos == crate::model::Pos::Noun {
            (
                overrides
                    .gender
                    .map(model_gender_label)
                    .or(guessed_gender)
                    .unwrap_or(""),
                match overrides.animate.or(guessed_animate) {
                    Some(true) => "anim",
                    Some(false) => "inanim",
                    None => "",
                },
            )
        } else {
            ("", "")
        };
        let row = format!(
            "{word}\t{}\t{g}\t{a}\t{}",
            pos.code(),
            overrides.gloss.as_deref().unwrap_or("").trim()
        );
        Some(validated_lexicon_row(&index, row, word))
    } else {
        None
    };

    let pass_phono = violations.is_empty();
    let pass_collision = collisions.is_empty();
    let pass_falsefriends = readings.is_empty();

    if json {
        let mut out = serde_json::json!({
            "word": word,
            "folded_key": folded,
            "phonotactics": { "pass": pass_phono, "violations": violations },
            "collision": { "pass": pass_collision, "collides_with": collisions },
            "false_friends": {
                "pass": pass_falsefriends,
                "readings": readings,
            },
            "declinability": {
                "guessed_pos": guessed_pos.code(),
                "effective_pos": pos.code(),
                "guessed_gender": guessed_gender,
                "guessed_animacy": guessed_animate.map(|a| if a { "anim" } else { "inanim" }),
                "declared_pos": overrides.pos.map(|p| p.code()),
                "declared_gender": overrides.gender.map(model_gender_label),
                "declared_animacy": overrides.animate.map(|a| if a { "anim" } else { "inanim" }),
                "divergences": divergences,
                "paradigm": paradigm
                    .iter()
                    .map(|(a, f)| serde_json::json!([a, f]))
                    .collect::<Vec<_>>(),
            },
        });
        match &lexicon_row {
            Some(Ok(row)) => out["lexicon_row"] = serde_json::json!(row),
            // Agents get the rejection in-band too; the nonzero exit below
            // still fires after the full report is printed.
            Some(Err(e)) => out["lexicon_row_error"] = serde_json::json!(e.to_string()),
            None => {}
        }
        println!("{}", serde_json::to_string_pretty(&out)?);
        return match lexicon_row {
            Some(Err(e)) => Err(e),
            _ => Ok(()),
        };
    }

    println!("coin-check '{word}' (folded key '{folded}')");
    println!(
        "  phonotactics : {}",
        if pass_phono {
            "PASS".to_string()
        } else {
            format!("WARN — {}", violations.join("; "))
        }
    );
    match collisions.len() {
        0 => println!("  collision    : PASS — no existing lemma or form"),
        n => {
            println!("  collision    : WARN — {n} existing record(s):");
            for c in collisions.iter().take(5) {
                println!(
                    "                   {} ({} {}, as {})",
                    c["lemma"].as_str().unwrap_or(""),
                    c["status"].as_str().unwrap_or(""),
                    c["pos"].as_str().unwrap_or(""),
                    c["as"].as_str().unwrap_or(""),
                );
            }
        }
    }
    match readings.len() {
        0 => println!("  false friends: PASS — no language reads it as an existing word"),
        n => {
            println!("  false friends: WARN — {n} language reading(s):");
            for r in readings.iter().take(6) {
                println!(
                    "                   {} {} ({}): {}",
                    r.lang,
                    r.word,
                    r.level,
                    r.glosses.join("; ").chars().take(70).collect::<String>(),
                );
            }
        }
    }
    let describe_noun = |gender: Option<&str>, animate: Option<bool>| -> String {
        match (gender, animate) {
            (Some(g), Some(a)) => format!(" {g}.{}", if a { "anim" } else { "inanim" }),
            (Some(g), None) => format!(" {g}."),
            _ => String::new(),
        }
    };
    let declared = if overridden || overrides.pos.is_some() {
        format!(
            ", declared {}{}",
            pos.code(),
            if pos == crate::model::Pos::Noun {
                describe_noun(overrides.gender.map(model_gender_label), overrides.animate)
            } else {
                String::new()
            }
        )
    } else {
        String::new()
    };
    let guessed = if pos == crate::model::Pos::Noun || guessed_pos == crate::model::Pos::Noun {
        format!(
            "guess: {}{}",
            guessed_pos.code(),
            describe_noun(guessed_gender, guessed_animate)
        )
    } else {
        format!("guess: {}", guessed_pos.code())
    };
    println!(
        "  declinability: as {}{declared} ({guessed}) — {} paradigm cells, e.g.:",
        pos.code(),
        paradigm.len()
    );
    for (analyses, form) in paradigm.iter().take(6) {
        println!("                   {form:<20} {analyses}");
    }
    for d in &divergences {
        println!("  ⚠ divergence : {d}");
    }
    match &lexicon_row {
        Some(Ok(row)) => {
            println!("  lexicon row  : {}", row.replace('\t', "\\t"));
            println!("                 (append the raw TSV line to your project lexicon; --json carries it in 'lexicon_row')");
        }
        Some(Err(e)) => println!("  lexicon row  : REJECTED — {e}"),
        None => {}
    }
    match lexicon_row {
        Some(Err(e)) => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phono() -> (Vec<official::OfficialEntry>, Phonotactics) {
        let entries = official::load(Path::new(crate::DEFAULT_OFFICIAL)).unwrap();
        let p = Phonotactics::from_official(&entries);
        (entries, p)
    }

    /// Selftest per the brief: a known-good official lemma passes
    /// phonotactics; an illegal cluster fails; a deliberate collision is
    /// detected by the same index check-text uses.
    #[test]
    fn coin_check_selftests() {
        let (entries, p) = phono();
        // Known-good official surface: clean phonotactics.
        assert!(p.violations("voda").is_empty());
        assert!(p.violations(&crate::forms::form_key("葡萄")).len() + 1 > 1); // non-ISV letters flagged
                                                                              // Illegal cluster / letter.
        let v = p.violations("xqzt");
        assert!(!v.is_empty(), "{v:?}");
        assert!(v.iter().any(|m| m.contains("illegal letter 'x'")), "{v:?}");
        assert!(v.iter().any(|m| m.contains("unattested")), "{v:?}");
        // Deliberate collision: 'voda' exists as an official lemma.
        let index = crate::check::build_index(&entries, None, Default::default());
        assert!(index
            .by_key
            .get("voda")
            .is_some_and(|recs| recs.iter().any(|r| r.status == "official")));
    }

    #[test]
    fn pos_guess_follows_citation_conventions() {
        assert_eq!(guess_pos("teleportovati"), crate::model::Pos::Verb);
        assert_eq!(guess_pos("zeleny"), crate::model::Pos::Adjective);
        assert_eq!(guess_pos("jabberwok"), crate::model::Pos::Noun);
    }

    /// V13 item 2: flag parsing fails closed, and the declared animacy
    /// actually reaches the inflector (animate masculines take
    /// genitive-shaped accusatives, the whole point of the override).
    #[test]
    fn overrides_parse_and_change_the_paradigm() {
        assert!(Overrides::parse(Some("pron"), None, None, None, false).is_err());
        assert!(Overrides::parse(None, Some("x"), None, None, false).is_err());
        assert!(Overrides::parse(None, None, Some("dead"), None, false).is_err());
        assert!(
            Overrides::parse(None, None, None, None, true).is_err(),
            "--lexicon-row without --gloss must fail (the row would be rejected downstream)"
        );
        let o = Overrides::parse(
            Some("noun"),
            Some("m"),
            Some("anim"),
            Some("jabberwock".into()),
            true,
        )
        .expect("valid flags");
        assert_eq!(o.pos, Some(crate::model::Pos::Noun));
        assert_eq!(o.animate, Some(true));

        let cell = |animate: bool| -> Vec<String> {
            let mut sink = crate::forms::RecordSink::default();
            crate::forms::project_paradigm_records(
                &mut sink,
                "žabervok",
                crate::model::Pos::Noun,
                Some(crate::model::Gender::Masculine),
                animate,
                "generated",
                "",
            );
            sink.into_records()
                .into_iter()
                .filter(|r| r.analyses.iter().any(|a| a.contains("akuz.jd.")))
                .map(|r| r.form)
                .collect()
        };
        assert_eq!(cell(true), ["žabervoka"], "animate acc.sg = gen.sg");
        assert_eq!(cell(false), ["žabervok"], "inanimate acc.sg = nom.sg");
    }

    /// Regression: a `#`-initial word makes the constructed TSV line parse
    /// as a lexicon COMMENT (empty row set) — --lexicon-row must reject it
    /// with an error, not panic on `rows[0]`.
    #[test]
    fn lexicon_row_rejects_comment_shaped_word() {
        let index = crate::check::build_index(&[], None, Default::default());
        let err = validated_lexicon_row(&index, "#foo\tnoun\tm\tanim\ttest".to_string(), "#foo")
            .unwrap_err();
        assert!(
            err.to_string().contains("rejected for '#foo'"),
            "must reject, not panic: {err}"
        );
        // The happy path through the same helper still returns the row.
        let ok = validated_lexicon_row(
            &index,
            "žabervok\tnoun\tm\tanim\tjabberwock".to_string(),
            "žabervok",
        )
        .expect("clean coinage validates");
        assert_eq!(ok, "žabervok\tnoun\tm\tanim\tjabberwock");
    }

    /// The crate's guess is exposed for the divergence report: 'žabervok'
    /// reads as a masculine inanimate noun by ending.
    #[test]
    fn noun_guess_is_reported() {
        let p = interslavic::noun_forms("žabervok");
        assert_eq!(gender_label(p.gender), "m");
        assert_eq!(p.animacy, interslavic::Animacy::Inanimate);
    }
}
