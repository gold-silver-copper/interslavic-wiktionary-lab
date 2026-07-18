//! An Interslavic synonym thesaurus built from the official dictionary.
//!
//! The dictionary lists one lemma per concept, but Interslavic has several valid
//! words per meaning. Two official lemmas are treated as synonyms when they share
//! a **modern-Slavic translation** (a strong meaning signal) AND an **English
//! gloss content token** (which filters the polysemy/homograph noise that shared
//! translations alone introduce — e.g. `dom`↔`suka`), with the **same POS**. The
//! result is a compact, high-precision `lemma → synonyms` resource, rebuilt
//! in-memory from the official dictionary wherever it's needed (the site export
//! and the synonym-aware accuracy scoring) — it is never persisted, so it can
//! never go stale.

use crate::model::Pos;
use crate::official::OfficialEntry;
use crate::orthography as ortho;
use std::collections::{BTreeSet, HashMap, HashSet};

/// The modern Slavic columns whose shared translations signal a shared meaning.
const SLAV: &[&str] = &[
    "ru", "be", "uk", "pl", "cs", "sk", "sl", "hr", "sr", "bg", "mk",
];

#[derive(Debug, Clone)]
pub struct ThesaurusEntry {
    pub isv: String,
    pub synonyms: Vec<String>,
}

/// Loaded thesaurus with an O(1) lookup by normalized lemma.
pub struct Thesaurus {
    entries: Vec<ThesaurusEntry>,
    by_key: HashMap<String, usize>,
}

/// Coarse part-of-speech class for synonym compatibility (a verb synonym must be
/// a verb). `Other` never matches, so untagged entries don't cross-link.
fn pos_class(p: Pos) -> &'static str {
    match p {
        Pos::Noun | Pos::ProperNoun => "n",
        Pos::Verb => "v",
        Pos::Adjective => "adj",
        Pos::Adverb => "adv",
        Pos::Numeral => "num",
        Pos::Pronoun => "pron",
        Pos::Preposition => "prep",
        Pos::Conjunction => "conj",
        Pos::Interjection => "intj",
        _ => "x",
    }
}

/// The lookup key for a lemma: its standard-alphabet folded spelling.
fn key(isv: &str) -> String {
    ortho::to_standard(&isv.trim().to_lowercase())
}

/// A modern translation form reduced for cross-lemma matching: lowercased,
/// non-letters dropped (keeps the native script, so ru `блюдо` matches `блюдо`).
fn trans_key(form: &str) -> String {
    form.trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphabetic())
        .collect()
}

impl Thesaurus {
    /// Build the synonym graph from the official dictionary's single-word lemmas.
    pub fn build(official: &[OfficialEntry]) -> Self {
        struct Info {
            orig: String,
            pos: &'static str,
            gloss: HashSet<String>,
        }
        let mut info: HashMap<String, Info> = HashMap::new();
        // (lang, translation-key) -> the set of lemma keys that translate to it.
        let mut by_trans: HashMap<(String, String), BTreeSet<String>> = HashMap::new();

        for e in official {
            let isv = e.isv.trim();
            if isv.is_empty() || isv.contains(' ') || isv.contains('#') {
                continue;
            }
            let k = key(isv);
            let pos = pos_class(e.pos);
            let gloss: HashSet<String> =
                crate::dump::gloss_tokens(&e.english).into_iter().collect();
            info.entry(k.clone()).or_insert(Info {
                orig: isv.to_string(),
                pos,
                gloss,
            });
            for &lang in SLAV {
                if let Some(cell) = e.cells.get(lang) {
                    for (form, _) in crate::normalize::split_cell(cell) {
                        let tk = trans_key(&form);
                        if tk.chars().count() >= 3 {
                            by_trans
                                .entry((lang.to_string(), tk))
                                .or_default()
                                .insert(k.clone());
                        }
                    }
                }
            }
        }

        // A synonym edge: two lemmas share a translation AND a gloss token AND POS.
        let mut syn: HashMap<String, BTreeSet<String>> = HashMap::new();
        for members in by_trans.values() {
            if members.len() < 2 {
                continue;
            }
            let v: Vec<&String> = members.iter().collect();
            for i in 0..v.len() {
                for j in (i + 1)..v.len() {
                    let (ia, ib) = (&info[v[i]], &info[v[j]]);
                    if ia.pos == ib.pos && ia.pos != "x" && !ia.gloss.is_disjoint(&ib.gloss) {
                        syn.entry(v[i].clone()).or_default().insert(ib.orig.clone());
                        syn.entry(v[j].clone()).or_default().insert(ia.orig.clone());
                    }
                }
            }
        }

        let mut entries: Vec<ThesaurusEntry> = syn
            .into_iter()
            .map(|(k, set)| ThesaurusEntry {
                isv: info[&k].orig.clone(),
                synonyms: set.into_iter().collect(),
            })
            .collect();
        entries.sort_by(|a, b| a.isv.cmp(&b.isv));
        Self::from_entries(entries)
    }

    fn from_entries(entries: Vec<ThesaurusEntry>) -> Self {
        let mut by_key = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            by_key.entry(key(&e.isv)).or_insert(i);
        }
        Thesaurus { entries, by_key }
    }

    /// Synonyms of a lemma (empty if none / not in the thesaurus).
    pub fn get(&self, isv: &str) -> &[String] {
        self.by_key
            .get(&key(isv))
            .map(|&i| self.entries[i].synonyms.as_slice())
            .unwrap_or(&[])
    }

    /// True when `a` and `b` are synonyms (either direction).
    pub fn are_synonyms(&self, a: &str, b: &str) -> bool {
        let bk = key(b);
        self.get(a).iter().any(|s| key(s) == bk)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thesaurus_roundtrip_and_lookup() {
        let t = Thesaurus::from_entries(vec![
            ThesaurusEntry {
                isv: "govoriti".into(),
                synonyms: vec!["mȯlviti".into(), "rěkti".into()],
            },
            ThesaurusEntry {
                isv: "krasny".into(),
                synonyms: vec!["krasivy".into()],
            },
        ]);
        assert_eq!(t.get("govoriti").len(), 2);
        assert!(t.are_synonyms("govoriti", "rěkti"));
        // Folded lookup: the flavored key matches.
        assert!(t.are_synonyms("krasny", "krasivy"));
        assert!(!t.are_synonyms("govoriti", "krasny"));
        assert!(t.get("nonesuch").is_empty());
    }
}
