//! Cross-lingual "same meaning" reverse index (reverse gloss links).
//!
//! Maps each English gloss head-synonym token to the `(lang, word)` lemmas that
//! carry it, so a word's page can show the words for its meaning(s) in OTHER
//! Slavic languages — e.g. the Russian raw entry `пластинка` ("record, disc")
//! links to Czech `deska`, Macedonian `плоча`, Polish `dysk`, … The bridge is a
//! shared English gloss, so this is an approximate *meaning* link, never an
//! etymological cognate claim. Built once at export time from the lemma caches;
//! display-only, never feeds generation, evidence, or ranking.

use std::collections::{HashMap, HashSet};

/// A token carried by more lemmas than this is too generic to be a useful
/// "same meaning" link (e.g. "move", "hit", "a surname"), so it is skipped.
const FREQ_CAP: usize = 150;
/// Max distinct languages shown per gloss token.
pub const MAX_LANGS: usize = 12;
/// Max words shown per language within one gloss token.
pub const MAX_PER_LANG: usize = 5;
/// Max gloss tokens rendered per entry.
pub const MAX_TOKENS: usize = 6;

/// English stop / grammatical words that carry no cross-lingual meaning.
const STOP: &[&str] = &[
    "to", "the", "a", "an", "of", "or", "and", "esp", "e.g.", "i.e.", "etc", "etc.", "vocative",
    "accusative", "genitive", "dative", "locative", "instrumental", "nominative", "singular",
    "plural", "imperfective", "perfective", "diminutive", "augmentative", "someone", "something",
    "one", "used", "form", "variant", "alternative", "obsolete", "archaic",
];

/// Reverse index: English gloss head-token → the `(lang, word)` lemmas glossed
/// with it.
#[derive(Default)]
pub struct GlossXref {
    by_token: HashMap<String, Vec<(String, String)>>,
}

impl GlossXref {
    pub fn new() -> Self {
        Self::default()
    }

    /// Index one lemma's glosses (idempotent per `(token, lang, word)`).
    pub fn add(&mut self, lang: &str, word: &str, glosses: &[String]) {
        let word = word.trim();
        if word.is_empty() || lang.is_empty() {
            return;
        }
        for tok in head_tokens(glosses) {
            let v = self.by_token.entry(tok).or_default();
            if !v.iter().any(|(l, w)| l == lang && w == word) {
                v.push((lang.to_string(), word.to_string()));
            }
        }
    }

    /// Sort each token's list so rendered output is deterministic.
    pub fn finalize(&mut self) {
        for v in self.by_token.values_mut() {
            v.sort();
        }
    }

    /// The cross-lingual matches for a word's glosses, grouped by gloss token in
    /// gloss order (deduped), excluding the source word's own language (which
    /// also excludes the word itself). Over-generic tokens (in > `FREQ_CAP`
    /// lemmas) are skipped. Each group's `(lang, word)` list is already
    /// language-sorted (from [`finalize`]).
    pub fn matches(
        &self,
        lang: &str,
        glosses: &[String],
    ) -> Vec<(String, Vec<(String, String)>)> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for tok in head_tokens(glosses) {
            if !seen.insert(tok.clone()) {
                continue;
            }
            let Some(all) = self.by_token.get(&tok) else {
                continue;
            };
            if all.len() > FREQ_CAP {
                continue;
            }
            let others: Vec<(String, String)> = all
                .iter()
                .filter(|(l, _)| l != lang)
                .cloned()
                .collect();
            if !others.is_empty() {
                out.push((tok, others));
            }
            if out.len() >= MAX_TOKENS {
                break;
            }
        }
        out
    }

    pub fn token_count(&self) -> usize {
        self.by_token.len()
    }
}

/// Extract the head-synonym tokens of a gloss list: for each gloss element take
/// the text before the first `(` (the synonym list, dropping parenthetical
/// explanations), split on `, ; /` and " or ", strip a leading "to " verb
/// marker, lowercase, and keep 2..=32-char content words that are not stopwords
/// or `"... of ..."` phrases. Deduplicated, order-preserving.
pub fn head_tokens(glosses: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for g in glosses {
        let head = g.split('(').next().unwrap_or("");
        for part in head.split([',', ';', '/']).flat_map(|p| p.split(" or ")) {
            let mut t = part.trim().trim_matches('.').trim().to_lowercase();
            if let Some(rest) = t.strip_prefix("to ") {
                t = rest.trim().to_string();
            }
            let n = t.chars().count();
            if (2..=32).contains(&n)
                && !t.ends_with(" etc")
                && !t.contains(" of ")
                && !STOP.contains(&t.as_str())
                && !out.contains(&t)
            {
                out.push(t);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_tokens_drop_parentheticals_and_verb_marker() {
        let g = vec![
            "record, disc".to_string(),
            "to cut (divide with a blade)".to_string(),
        ];
        assert_eq!(head_tokens(&g), vec!["record", "disc", "cut"]);
    }

    #[test]
    fn head_tokens_drop_stopwords_and_of_phrases() {
        let g = vec!["alternative form of foo".to_string(), "the, a, of".to_string()];
        // "alternative form of foo" contains " of " -> dropped; stopwords dropped.
        assert!(head_tokens(&g).is_empty());
    }

    #[test]
    fn matches_group_by_token_other_languages_only() {
        let mut gx = GlossXref::new();
        gx.add("ru", "пластинка", &["record, disc".to_string()]);
        gx.add("cs", "deska", &["record, platter".to_string()]);
        gx.add("cs", "disk", &["disc, disk".to_string()]);
        gx.add("ru", "диск", &["disc".to_string()]); // same lang as source -> excluded
        gx.finalize();
        let m = gx.matches("ru", &["record, disc".to_string()]);
        // token "record" -> cs deska ; token "disc" -> cs disk (ru диск excluded)
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].0, "record");
        assert_eq!(m[0].1, vec![("cs".to_string(), "deska".to_string())]);
        assert_eq!(m[1].0, "disc");
        assert_eq!(m[1].1, vec![("cs".to_string(), "disk".to_string())]);
    }
}
