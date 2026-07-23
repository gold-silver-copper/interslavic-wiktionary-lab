//! English gloss tokenization — every tokenizer, side by side (V15 item 4).
//!
//! The crate grew five parallel gloss tokenizers with deliberately different
//! semantics, three of them reached across module boundaries. This module
//! collects them VERBATIM (pure code motion; the old paths re-export) so the
//! differences are visible in one place. Unifying semantics is V16 material —
//! do not "fix" a divergence here without benchmark evidence.
//!
//! The residents, with their provenance and semantic signature:
//!
//! * [`content_tokens`] (ex `dump::gloss_tokens`, the de-facto shared one):
//!   lowercase alphabetic split, len ≥ 3, small stopword list, NO stemming.
//! * [`stemmed_tokens`] / [`ordered_stemmed_tokens`] (ex
//!   `falsefriends::gloss_tokens`/`ordered_tokens`): parentheticals stripped,
//!   light suffix stemming, large stopword list, len ≥ 2.
//! * [`head_tokens`] (ex `glossxref::head_tokens`): whole head-synonym
//!   phrases (multi-word keys), not single tokens.
//! * [`gloss_keys`] / [`english_gloss_tokens`] (ex `site::english_api`): the
//!   English-index key extraction — segment-aware, parenthetical-aware,
//!   ranked match kinds.

use std::collections::BTreeSet;

// ---------------------------------------------------------------------------
// Ex dump::gloss_tokens — the de-facto shared content tokenizer.
// ---------------------------------------------------------------------------

/// Lowercase content-word gloss tokens (drop stopwords and short tokens).
pub fn content_tokens(gloss: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "the", "a", "an", "to", "of", "and", "or", "in", "on", "for", "with", "be", "is", "as",
        "at", "by", "that", "this", "it", "one", "some", "any", "esp", "e", "g",
    ];
    gloss
        .to_lowercase()
        .split(|c: char| !c.is_alphabetic())
        .filter(|t| t.len() >= 3 && !STOP.contains(t))
        .map(std::string::ToString::to_string)
        .collect()
}

// ---------------------------------------------------------------------------
// Ex falsefriends::{gloss_tokens, ordered_tokens} — stemmed comparison tokens.
// ---------------------------------------------------------------------------

/// Strip `(...)`/`[...]` disambiguation and the text after a `:` inside them.
pub(crate) fn strip_parens(s: &str) -> String {
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
pub fn stemmed_tokens(gloss: &str) -> BTreeSet<String> {
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

/// Order-preserving variant of [`stemmed_tokens`] for positional rules (the
/// `X or Y` closure needs the token ADJACENT to the `or`, and a BTreeSet
/// iterator would hand back the alphabetical extreme instead — which minted
/// phantom pairs like evil≈event from unrelated glosses).
pub(crate) fn ordered_stemmed_tokens(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in strip_parens(text)
        .to_lowercase()
        .split(|c: char| !c.is_alphabetic())
    {
        if raw.is_empty() || STOPWORDS.contains(&raw) {
            continue;
        }
        let t = light_stem(raw);
        if t.chars().count() >= 2 && !STOPWORDS.contains(&t.as_str()) && !out.contains(&t) {
            out.push(t);
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

// ---------------------------------------------------------------------------
// Ex glossxref::head_tokens — whole head-synonym phrases.
// ---------------------------------------------------------------------------

/// English stop / grammatical words that carry no cross-lingual meaning.
const STOP: &[&str] = &[
    "to",
    "the",
    "a",
    "an",
    "of",
    "or",
    "and",
    "esp",
    "e.g.",
    "i.e.",
    "etc",
    "etc.",
    "vocative",
    "accusative",
    "genitive",
    "dative",
    "locative",
    "instrumental",
    "nominative",
    "singular",
    "plural",
    "imperfective",
    "perfective",
    "diminutive",
    "augmentative",
    "someone",
    "something",
    "one",
    "used",
    "form",
    "variant",
    "alternative",
    "obsolete",
    "archaic",
];

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

// ---------------------------------------------------------------------------
// Ex site::english_api — the English-index key extraction.
// ---------------------------------------------------------------------------

/// Function words: never a lookup key, not even as an exact gloss head.
const HEAD_STOPWORDS: &[&str] = &["a", "an", "and", "etc", "in", "of", "or", "the", "to"];

/// Grammatical-note noise, filtered from token extraction only. A gloss whose
/// entire head is one of these ("one", "form", "plural") is a real English
/// word and must stay findable.
const TOKEN_STOPWORDS: &[&str] = &[
    "archaic",
    "dative",
    "form",
    "genitive",
    "instrumental",
    "locative",
    "nominative",
    "obsolete",
    "one",
    "plural",
    "singular",
    "someone",
    "something",
    "used",
    "variant",
];

fn normalize_english_text(raw: &str) -> String {
    let mut out = String::new();
    let mut last_space = true;
    for ch in raw.to_lowercase().chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

pub fn normalize_english_query(raw: &str) -> String {
    let key = normalize_english_text(raw);
    key.strip_prefix("to ")
        .map(str::trim)
        .filter(|rest| !rest.is_empty())
        .unwrap_or(&key)
        .to_string()
}

fn usable_head_key(key: &str) -> bool {
    let n = key.chars().count();
    (2..=48).contains(&n)
        && !HEAD_STOPWORDS.contains(&key)
        && !key.ends_with(" etc")
        && !key.starts_with("used ")
}

pub(crate) fn usable_token_key(key: &str) -> bool {
    usable_head_key(key) && !TOKEN_STOPWORDS.contains(&key)
}

pub(crate) fn gloss_keys(gloss: &str) -> Vec<(String, String)> {
    // Derivative glosses ("label ← base (…)") truncate the base gloss with a
    // trailing `…`, so their final segment can be a cut word ("substa…") or
    // carry an unbalanced paren. Official glosses use `…` legitimately
    // ("either … or …") and are never ←-shaped.
    let derived = gloss.contains('←');
    let gloss = english_lookup_gloss(gloss);
    let mut out = Vec::new();
    for segment in split_gloss_segments(gloss) {
        if derived && segment.contains('…') {
            continue;
        }
        let (head, parenthetical) = split_parenthetical(segment);
        push_gloss_head_keys(&mut out, head);
        if let Some(note) = parenthetical {
            push_parenthetical_keys(&mut out, note);
        }
    }
    out
}

fn english_lookup_gloss(gloss: &str) -> &str {
    if gloss.contains('←') {
        if let (Some(open), Some(close)) = (gloss.find('('), gloss.rfind(')')) {
            if open < close {
                return &gloss[open + 1..close];
            }
        }
    }
    gloss
}

fn split_gloss_segments(gloss: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    for (i, ch) in gloss.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' | ';' | '/' if depth == 0 => {
                out.push(&gloss[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&gloss[start..]);
    out
}

fn split_parenthetical(segment: &str) -> (&str, Option<&str>) {
    match (segment.find('('), segment.rfind(')')) {
        (Some(open), Some(close)) if open < close => {
            let before = segment[..open].trim();
            let after = segment[close + 1..].trim();
            if before.is_empty() && !after.is_empty() {
                // Leading register label, e.g. "(formal) day": the head follows
                // the note, and the label itself is not a lookup key.
                (after, None)
            } else {
                (before, Some(segment[open + 1..close].trim()))
            }
        }
        _ => (segment.trim(), None),
    }
}

fn push_gloss_head_keys(out: &mut Vec<(String, String)>, raw: &str) {
    let normalized_head = raw.replace(" or ", ",");
    if normalized_head != raw {
        // Idioms like "more or less": query normalization keeps "or", so the
        // full phrase must be a key too, not only the split alternatives. The
        // " or " must survive normalization ("either … or …" folds to
        // "either or" — not a phrase anyone queries).
        let full = normalize_english_query(raw);
        if full.contains(" or ") && usable_head_key(&full) {
            push_key(out, full, "phrase");
        }
    }
    for part in normalized_head.split(',') {
        let key = normalize_english_query(part);
        if !usable_head_key(&key) {
            continue;
        }
        let match_kind = if key.contains(' ') {
            "phrase"
        } else {
            "exact-gloss-head"
        };
        push_key(out, key.clone(), match_kind);
        if key.contains(' ') {
            push_token_keys(out, &key);
        }
    }
}

fn push_parenthetical_keys(out: &mut Vec<(String, String)>, raw: &str) {
    if raw.contains([',', ';', '/']) {
        return;
    }
    let mut key = normalize_english_query(raw);
    for prefix in ["of ", "a ", "an ", "the ", "on ", "in "] {
        while let Some(rest) = key.strip_prefix(prefix) {
            key = rest.trim().to_string();
        }
    }
    let words = key.split_whitespace().count();
    if words <= 3 {
        push_token_keys(out, &key);
    }
}

fn push_token_keys(out: &mut Vec<(String, String)>, key: &str) {
    for token in key.split_whitespace() {
        if usable_token_key(token) {
            push_key(out, token.to_string(), "gloss-token");
        }
    }
}

/// Rank contribution of the match kind. `phrase` and `exact-gloss-head` never
/// compete on one key (a key with a space is always a phrase match, a key
/// without one never is) — the weights only order them against `gloss-token`.
pub(crate) fn match_rank(match_kind: &str) -> i32 {
    match match_kind {
        "phrase" => 120,
        "exact-gloss-head" => 100,
        // Mechanical English derivation of the base's gloss (see
        // `derived_english_forms`): stronger than an incidental token hit,
        // weaker than an exact head.
        "derived-english" => 80,
        "gloss-token" => 40,
        _ => 20,
    }
}

fn push_key(out: &mut Vec<(String, String)>, key: String, match_kind: &str) {
    match out.iter_mut().find(|(seen, _)| seen == &key) {
        // Upgrade, e.g. a token of an earlier phrase segment reappearing as
        // its own exact segment: "up until, before, until".
        Some((_, existing)) if match_rank(match_kind) > match_rank(existing) => {
            *existing = match_kind.to_string();
        }
        Some(_) => {}
        None => out.push((key, match_kind.to_string())),
    }
}

/// Deterministic content-token set of an English gloss — the SAME key
/// extraction and stopword discipline the English index build uses
/// ([`gloss_keys`]), flattened to single tokens. This is the overlap test
/// behind `check-text`'s project-lexicon consistency warning (V13 item 1):
/// two glosses "overlap" iff their token sets intersect.
pub fn english_gloss_tokens(gloss: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for (key, _) in gloss_keys(gloss) {
        for token in key.split_whitespace() {
            if usable_token_key(token) {
                out.insert(token.to_string());
            }
        }
    }
    out
}
