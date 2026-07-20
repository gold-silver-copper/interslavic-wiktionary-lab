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

/// The `coin-check` CLI entry point.
pub fn run(official_path: &Path, word: &str, json: bool) -> Result<()> {
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

    // Axis 4: declinability — render the paradigm for the guessed POS.
    let pos = guess_pos(&folded);
    let mut sink = crate::forms::RecordSink::default();
    crate::forms::paradigm_records(&mut sink, word, pos, None, 0, "generated", None, "");
    let mut paradigm: Vec<(String, String)> = sink
        .into_records()
        .into_iter()
        .filter(|r| r.source != "lemma")
        .map(|r| (r.analyses.join("/"), r.form))
        .collect();
    paradigm.sort();
    paradigm.dedup();

    let pass_phono = violations.is_empty();
    let pass_collision = collisions.is_empty();
    let pass_falsefriends = readings.is_empty();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "word": word,
                "folded_key": folded,
                "phonotactics": { "pass": pass_phono, "violations": violations },
                "collision": { "pass": pass_collision, "collides_with": collisions },
                "false_friends": {
                    "pass": pass_falsefriends,
                    "readings": readings,
                },
                "declinability": {
                    "guessed_pos": pos.code(),
                    "paradigm": paradigm
                        .iter()
                        .map(|(a, f)| serde_json::json!([a, f]))
                        .collect::<Vec<_>>(),
                },
            }))?
        );
        return Ok(());
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
    println!(
        "  declinability: as {} — {} paradigm cells, e.g.:",
        pos.code(),
        paradigm.len()
    );
    for (analyses, form) in paradigm.iter().take(6) {
        println!("                   {form:<20} {analyses}");
    }
    Ok(())
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
}
