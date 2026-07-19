//! Algorithmic false-friend (semantic-trap) detection.
//!
//! Replaces the retired curated `data/semantic-notes.json` with a deterministic
//! computation over the committed evidence caches: an official Interslavic
//! lemma is a *false friend* for language ℓ when ℓ has a word a speaker will
//! read as that lemma (same folded surface after the standard flavorization /
//! citation-ending adaptation) whose English Wiktionary glosses share **no
//! content token** with the official gloss. The old 12 hand-written notes are
//! encoded as test expectations below — never as runtime data.
//!
//! Surfaces are matched at two levels:
//! 1. **exact** — `form_key(flavorize_word(lang, pos, word)) == form_key(isv)`;
//! 2. **loose** — equal ASCII skeletons after additionally folding `y→i`
//!    (ru корыстный → korystny vs koristny), stems ≥ [`LOOSE_MIN_CHARS`] only.
//!
//! Gloss overlap is a plain token comparison: lowercase, parentheticals
//! stripped, stopwords removed, light plural/participle suffix strip. Zero
//! shared tokens ⇒ divergent ⇒ a computed warning record. Official homographs
//! pool their glosses first, so a sense the dictionary itself attests (žena
//! 'woman, wife'; vonjati 'smell, stink') is *not* flagged — by design, unlike
//! two of the old curated "ambiguous by attestation" notes.

use crate::dump::{LemmaCorpus, RawSlavicCorpus};
use crate::official::OfficialEntry;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Minimum folded-key length for the loose (y→i-folded skeleton) match level.
const LOOSE_MIN_CHARS: usize = 5;
/// Minimum folded-key length for any collision at all (short function words
/// collide across the whole family for phonology, not semantics).
const MIN_KEY_CHARS: usize = 3;
/// At most this many languages are rendered into the warning sentence (the
/// `collisions` list stays complete).
const MAX_WARNED_LANGS: usize = 4;
/// At most this many divergent senses are quoted per colliding word.
const MAX_QUOTED_SENSES: usize = 2;

/// Modern languages whose speakers the warnings address, in render order.
/// English Wiktionary's `sh` macro-code covers Serbian/Croatian/Bosnian.
const LANGS: &[(&str, &str)] = &[
    ("ru", "Russian"),
    ("uk", "Ukrainian"),
    ("be", "Belarusian"),
    ("pl", "Polish"),
    ("cs", "Czech"),
    ("sk", "Slovak"),
    ("sl", "Slovene"),
    ("sh", "Serbo-Croatian"),
    ("mk", "Macedonian"),
    ("bg", "Bulgarian"),
];

/// One colliding source word with divergent meaning.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Collision {
    /// Language code (`sh` = Serbo-Croatian macro-code).
    pub lang: String,
    /// The colliding word in its native spelling.
    pub word: String,
    /// The divergent English Wiktionary glosses (deduped, parentheticals kept
    /// for display).
    pub glosses: Vec<String>,
}

/// A computed semantic-trap note for one folded Interslavic key. The
/// `warning`/`prefer` shape is kept from the curated era so consumers
/// (check-text JSON, `api/notes.json`, English-API candidates) don't break;
/// `collisions` carries the full machine-readable evidence.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Note {
    pub warning: String,
    /// Official lemmas whose gloss best covers the divergent sense (computed,
    /// e.g. urok trap 'lesson' → lekcija), possibly empty.
    pub prefer: Vec<String>,
    pub collisions: Vec<Collision>,
}

/// Strip `(...)`/`[...]` disambiguation and the text after a `:` inside them.
fn strip_parens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

const STOPWORDS: &[&str] = &[
    "a",
    "an",
    "the",
    "to",
    "of",
    "in",
    "on",
    "or",
    "and",
    "for",
    "be",
    "by",
    "with",
    "at",
    "as",
    "from",
    "is",
    "are",
    "was",
    "it",
    "its",
    "one",
    "ones",
    "etc",
    "who",
    "whom",
    "whose",
    "which",
    "that",
    "this",
    "these",
    "those",
    "sth",
    "smth",
    "smb",
    "so",
    "not",
    "do",
    "does",
    "up",
    "out",
    "e",
    "g",
    "i",
    "also",
    "any",
    "some",
    "very",
    "into",
    "over",
    "s",
    "someone",
    "something",
    "oneself",
    "esp",
    "especially",
    "usually",
    "often",
    "person",
    "thing",
];

/// Deterministic content tokens of a gloss: lowercase, parentheticals removed,
/// split on non-letters, stopwords dropped, light suffix strip (plural
/// `-s`/`-es`, participle `-ing`/`-ed`) so `asking`/`asks` meet `ask`.
pub fn gloss_tokens(gloss: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for raw in strip_parens(gloss)
        .to_lowercase()
        .split(|c: char| !c.is_alphabetic())
    {
        if raw.is_empty() || STOPWORDS.contains(&raw) {
            continue;
        }
        let t = light_stem(raw);
        if t.chars().count() >= 2 && !STOPWORDS.contains(&t.as_str()) {
            out.insert(t);
        }
    }
    out
}

fn light_stem(t: &str) -> String {
    let n = t.chars().count();
    for (suf, min_len) in [("ing", 6), ("ed", 5), ("es", 5), ("s", 4)] {
        if n >= min_len {
            if let Some(stem) = t.strip_suffix(suf) {
                return stem.to_string();
            }
        }
    }
    t.to_string()
}

/// The loose comparison key: ASCII skeleton with `y` folded onto `i`.
fn loose_key(folded: &str) -> String {
    crate::orthography::ascii_skeleton(folded).replace('y', "i")
}

/// How language ℓ's dictionary word surfaces to a reader of Interslavic:
/// citation-ending adaptation + per-language flavorization, folded to the
/// standard lookup key (the same transform the raw-attestation pages use).
fn read_as_key(lang: &str, pos: &str, word: &str) -> String {
    crate::forms::form_key(&crate::flavorize::flavorize_word(lang, pos, word))
}

/// Only open-class cache rows participate; closed-class/name rows collide for
/// grammatical, not semantic, reasons.
fn eligible_pos(pos: &str) -> bool {
    matches!(pos, "noun" | "verb" | "adj" | "adv")
}

struct CacheWord {
    lang_idx: usize,
    word: String,
    glosses: Vec<String>,
    tokens: BTreeSet<String>,
}

/// Compute the notes map, keyed by the folded Interslavic lookup key
/// (`forms::form_key`). Deterministic: BTree collections throughout, fixed
/// language order, alphabetical tie-breaks.
pub fn compute(
    official: &[OfficialEntry],
    evidence: Option<&LemmaCorpus>,
    raw: Option<&RawSlavicCorpus>,
) -> BTreeMap<String, Note> {
    let lang_idx: HashMap<&str, usize> = LANGS.iter().enumerate().map(|(i, l)| (l.0, i)).collect();

    // ---- 1/2. Index cache records by read-as key (exact and loose). ----
    // Divergence is judged per RECORD (one POS/etymology entry of one word),
    // not per pooled word: pl jutro-the-adverb 'tomorrow' must fire even
    // though pl jutro-the-noun also lists an archaic 'morning' sense. Records
    // for the same (lang, word) later merge into one collision for display.
    let mut words: Vec<CacheWord> = Vec::new();
    let mut by_exact: HashMap<String, Vec<usize>> = HashMap::new();
    let mut by_loose: HashMap<String, Vec<usize>> = HashMap::new();
    {
        let mut add = |lang: &str, word: &str, pos: &str, glosses: Vec<String>| {
            let Some(&li) = lang_idx.get(lang) else {
                return;
            };
            if !eligible_pos(pos) || word.contains(' ') {
                return;
            }
            let tokens: BTreeSet<String> = glosses.iter().flat_map(|g| gloss_tokens(g)).collect();
            if tokens.is_empty() {
                return;
            }
            let key = read_as_key(lang, pos, word);
            if key.chars().count() < MIN_KEY_CHARS {
                return;
            }
            let idx = words.len();
            by_exact.entry(key.clone()).or_default().push(idx);
            if key.chars().count() >= LOOSE_MIN_CHARS {
                by_loose.entry(loose_key(&key)).or_default().push(idx);
            }
            words.push(CacheWord {
                lang_idx: li,
                word: word.to_string(),
                glosses,
                tokens,
            });
        };
        if let Some(corpus) = evidence {
            for e in &corpus.entries {
                add(&e.lang, &e.word, &e.pos, vec![e.gloss.clone()]);
            }
        }
        if let Some(corpus) = raw {
            for e in &corpus.lemmas {
                add(&e.lang, &e.word, &e.pos, e.glosses.clone());
            }
        }
    }

    // ---- 3. Pool official glosses per folded key (homographs vote jointly),
    // and build the token → official-lemma index that computes `prefer`. ----
    let mut official_tokens: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut official_display: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    let mut by_token: HashMap<String, Vec<usize>> = HashMap::new();
    for (ei, e) in official.iter().enumerate() {
        let isv = e.isv.trim();
        if isv.is_empty() || isv.contains(' ') || isv.contains('#') || e.english.trim().is_empty() {
            continue;
        }
        let key = crate::forms::form_key(isv);
        if key.chars().count() < MIN_KEY_CHARS {
            continue;
        }
        let toks = gloss_tokens(&e.english);
        for t in &toks {
            by_token.entry(t.clone()).or_default().push(ei);
        }
        official_tokens.entry(key.clone()).or_default().extend(toks);
        let display = official_display
            .entry(key)
            .or_insert_with(|| (isv.to_string(), Vec::new()));
        let gloss = e.english.trim().to_string();
        if !display.1.iter().any(|have| have == &gloss) {
            display.1.push(gloss);
        }
    }

    // ---- 4. Detect divergent collisions per official key. ----
    let mut notes: BTreeMap<String, Note> = BTreeMap::new();
    for (key, isv_tokens) in &official_tokens {
        if isv_tokens.is_empty() {
            continue;
        }
        let mut candidate_idxs: BTreeSet<usize> = BTreeSet::new();
        if let Some(v) = by_exact.get(key) {
            candidate_idxs.extend(v.iter().copied());
        }
        if key.chars().count() >= LOOSE_MIN_CHARS {
            if let Some(v) = by_loose.get(&loose_key(key)) {
                candidate_idxs.extend(v.iter().copied());
            }
        }
        // Merge divergent records of the same (lang, word) into one collision.
        let mut merged: BTreeMap<(usize, String), CacheWord> = BTreeMap::new();
        for w in candidate_idxs
            .into_iter()
            .map(|i| &words[i])
            .filter(|w| w.tokens.is_disjoint(isv_tokens))
        {
            let slot = merged
                .entry((w.lang_idx, w.word.clone()))
                .or_insert_with(|| CacheWord {
                    lang_idx: w.lang_idx,
                    word: w.word.clone(),
                    glosses: Vec::new(),
                    tokens: BTreeSet::new(),
                });
            for g in &w.glosses {
                if !slot.glosses.iter().any(|have| have == g) {
                    slot.glosses.push(g.clone());
                }
            }
            slot.tokens.extend(w.tokens.iter().cloned());
        }
        if merged.is_empty() {
            continue;
        }
        let collisions: Vec<CacheWord> = merged.into_values().collect();

        let (isv_display, gloss_display) = &official_display[key];
        let mut warning = format!("Official meaning: '{}'.", gloss_display.join("' / '"));
        let mut warned_langs: Vec<usize> = Vec::new();
        for w in &collisions {
            if warned_langs.contains(&w.lang_idx) || warned_langs.len() >= MAX_WARNED_LANGS {
                continue;
            }
            warned_langs.push(w.lang_idx);
            let senses: Vec<&str> = w
                .glosses
                .iter()
                .map(|g| g.as_str())
                .take(MAX_QUOTED_SENSES)
                .collect();
            let _ = std::fmt::Write::write_fmt(
                &mut warning,
                format_args!(
                    " {} speakers may read it as '{}' ({} {}).",
                    LANGS[w.lang_idx].1,
                    senses.join("; "),
                    LANGS[w.lang_idx].0,
                    w.word
                ),
            );
        }

        // `prefer`: the official lemma whose gloss best covers the divergent
        // sense — the algorithmic replacement for the curated field. Score by
        // *collision coverage* (the fraction of each colliding word's sense
        // tokens the lemma's gloss covers, summed in integer ppm so the result
        // is deterministic): urok → lekcija fully covers bg урок 'lesson',
        // while oči 'eyes' only grazes the multi-token 'evil eye' senses.
        let mut scores: BTreeMap<usize, usize> = BTreeMap::new();
        for c in &collisions {
            let mut per_entry: BTreeMap<usize, usize> = BTreeMap::new();
            for t in &c.tokens {
                if let Some(eis) = by_token.get(t) {
                    for &ei in eis {
                        *per_entry.entry(ei).or_default() += 1;
                    }
                }
            }
            for (ei, o) in per_entry {
                *scores.entry(ei).or_default() += o * 1_000_000 / c.tokens.len();
            }
        }
        let mut best: Option<(usize, f32, String)> = None; // (coverage, freq, lemma)
        for (ei, overlap) in scores {
            let e = &official[ei];
            let lemma = e.isv.trim().to_string();
            if &crate::forms::form_key(&lemma) == key {
                continue;
            }
            let freq = e.frequency.unwrap_or(0.0);
            let better = match &best {
                None => true,
                Some((bo, bf, bl)) => {
                    (overlap, freq, std::cmp::Reverse(lemma.clone()))
                        > (*bo, *bf, std::cmp::Reverse(bl.clone()))
                }
            };
            if better {
                best = Some((overlap, freq, lemma));
            }
        }
        let prefer: Vec<String> = best.map(|(_, _, lemma)| vec![lemma]).unwrap_or_default();

        let _ = isv_display; // display lemma is implicit in the key for consumers
        notes.insert(
            key.clone(),
            Note {
                warning,
                prefer,
                collisions: collisions
                    .into_iter()
                    .map(|w| Collision {
                        lang: LANGS[w.lang_idx].0.to_string(),
                        word: w.word.clone(),
                        glosses: w.glosses.clone(),
                    })
                    .collect(),
            },
        );
    }
    notes
}

/// Load both caches and compute notes; either cache may be absent (degrades to
/// fewer/no notes, never an error) so `check-text` stays usable without them.
pub fn compute_from_default_caches(official: &[OfficialEntry]) -> BTreeMap<String, Note> {
    let evidence = LemmaCorpus::load(std::path::Path::new(crate::DEFAULT_LEMMA_CACHE)).ok();
    let raw = RawSlavicCorpus::load(std::path::Path::new(crate::DEFAULT_RAW_LEMMA_CACHE)).ok();
    compute(official, evidence.as_ref(), raw.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes() -> BTreeMap<String, Note> {
        let official =
            crate::official::load(std::path::Path::new(crate::DEFAULT_OFFICIAL)).unwrap();
        compute_from_default_caches(&official)
    }

    /// The retired curated notes (data/semantic-notes.json, removed) as a
    /// held-out sanity set: the detector must independently rediscover the
    /// genuine surface-collision traps among them.
    #[test]
    fn rediscovers_curated_false_friends() {
        let notes = notes();
        let hit = |key: &str, lang: &str| {
            notes
                .get(key)
                .map(|n| n.collisions.iter().any(|c| c.lang == lang))
                .unwrap_or(false)
        };
        assert!(hit("pytati", "ru"), "pytati / ru пытать 'torture'");
        assert!(hit("jutro", "pl"), "jutro / pl jutro 'tomorrow'");
        assert!(hit("čas", "sh"), "čas / sh час 'hour, moment'");
        assert!(hit("urok", "ru"), "urok / ru урок 'lesson'");
        assert!(hit("rok", "sh"), "rok / sh rok 'deadline'");
        assert!(hit("slovo", "sh"), "slovo / sh slovo 'letter'");
        assert!(hit("koristny", "ru"), "koristny / ru корыстный (loose y/i)");
        assert!(hit("trg", "sh"), "trg / sh trg 'square'");
    }

    /// The two curated "ambiguous by attestation" notes must NOT fire: the
    /// dictionary itself attests both senses, so the colliding word's gloss
    /// overlaps the pooled official gloss. Documented behavior change.
    #[test]
    fn attested_ambiguity_is_not_a_false_friend() {
        let notes = notes();
        assert!(!notes.contains_key("žena"), "žena 'woman/wife' is attested");
        for key in ["vonjati", "vonjeti"] {
            if let Some(n) = notes.get(key) {
                assert!(
                    !n.collisions
                        .iter()
                        .any(|c| c.lang == "cs" || c.lang == "ru"),
                    "vonjati polarity is attested ambiguity, not a false friend"
                );
            }
        }
    }

    /// urok's divergent sense is 'lesson'; the computed prefer must point at
    /// the official word for it.
    #[test]
    fn prefer_recovers_lekcija_for_urok() {
        let notes = notes();
        let urok = notes.get("urok").expect("urok note");
        assert_eq!(urok.prefer, vec!["lekcija".to_string()]);
    }

    /// Volume + determinism guard: the detector's yield should stay in a sane
    /// band (a threshold regression that floods or empties the notes artifact
    /// must fail loudly, not ship silently).
    #[test]
    fn note_volume_stays_sane() {
        let notes = notes();
        let collisions: usize = notes.values().map(|n| n.collisions.len()).sum();
        eprintln!(
            "false-friend notes: {} keys, {collisions} collisions",
            notes.len()
        );
        assert!(
            notes.len() >= 100,
            "suspiciously few notes: {}",
            notes.len()
        );
        assert!(
            notes.len() <= 20_000,
            "suspiciously many notes: {}",
            notes.len()
        );
    }

    #[test]
    fn gloss_tokens_strip_parentheticals_and_stopwords() {
        let t = gloss_tokens("hour (unit of time: one twenty-fourth of a day)");
        assert!(t.contains("hour"));
        assert!(!t.contains("time"));
        let t = gloss_tokens("to ask");
        assert_eq!(t.into_iter().collect::<Vec<_>>(), vec!["ask"]);
    }
}
