//! Interslavic orthography helpers.
//!
//! Interslavic has two written standards. The official dictionary stores lemmas
//! in the *flavored / scientific* alphabet, which preserves etymological
//! distinctions (jat `ě`, nasals `ę`/`ų`, the liquid-diphthong vowel `å`, the
//! yer reflexes `ȯ`/`ė`, the soft consonants `ĺ ń ŕ ť ď ś ź`, and `ć`/`đ`).
//! The *standard* alphabet folds those away. We keep helpers for both, plus the
//! aggressive ASCII skeleton used to align cognates and to compute the
//! "normalized" match metric.

/// The flavored→standard fold (`ě→e`, `ć→č`, `đ→dž`, …) now comes from the
/// interslavic crate (issue #11); re-exported here so the many local call sites
/// keep compiling. The crate's fold is byte-identical to the previous local one
/// on lowercase input (every call here lowercases first). Its stability posture
/// is best-effort, so Slovowiki exact-pins the crate and freezes the fold's
/// wire-format outputs (`router_selftest_samples_are_frozen`, `router_is_stable`,
/// the shipped `router-selftest.json`): a crate-side change to this best-effort
/// fold surfaces as a test failure at pin-bump time, never as a silently
/// repartitioned index.
pub use interslavic::orthography::to_standard;

/// The crate-wide fold-to-key idiom, named ONCE (V15 item 5): lowercase,
/// then the pinned standard-orthography fold. Every keying path must call
/// this instead of hand-composing the two steps.
pub fn fold_key(s: &str) -> String {
    to_standard(&s.to_lowercase())
}

/// Combining diacritical mark (U+0300–U+036F)? The hand-inlined range
/// predicate appeared ten times crate-wide; this is the one copy.
pub fn is_combining_mark(c: char) -> bool {
    ('\u{0300}'..='\u{036F}').contains(&c)
}

/// Aggressive ASCII skeleton: strip *all* diacritics and fold the phonemically
/// close consonant classes together. Used to align cognates across languages
/// and as a looser matching key. Preserves the y/i and hard/soft distinctions
/// only where they survive as separate ASCII letters.
pub fn ascii_skeleton(word: &str) -> String {
    let std = fold_key(word);
    let mut out = String::with_capacity(std.len());
    for ch in std.chars() {
        match ch {
            'č' | 'ć' | 'ç' => out.push('c'),
            'š' | 'ś' | 'ş' => out.push('s'),
            'ž' | 'ź' | 'ż' => out.push('z'),
            'đ' => out.push('d'),
            'ń' | 'ň' => out.push('n'),
            'ľ' | 'ĺ' | 'ł' => out.push('l'),
            'ř' | 'ŕ' => out.push('r'),
            'ť' => out.push('t'),
            'ď' => out.push('d'),
            'á' | 'à' | 'â' | 'ā' | 'ǎ' | 'å' => out.push('a'),
            'é' | 'è' | 'ê' | 'ē' | 'ě' | 'ę' => out.push('e'),
            'í' | 'ì' | 'î' | 'ī' => out.push('i'),
            'ó' | 'ò' | 'ô' | 'ō' | 'ȯ' | 'ǫ' => out.push('o'),
            'ú' | 'ù' | 'û' | 'ū' | 'ů' | 'ų' => out.push('u'),
            'ý' | 'ỳ' | 'ŷ' | 'ȳ' => out.push('y'),
            other => out.push(other),
        }
    }
    out
}

/// Consonant-only alignment key for voting *within one meaning group*, where all
/// forms are cognate by construction. Drops vowels and semivowels, folds the
/// regular consonant correspondences that split cognates across branches
/// (notably *g→h in Czech/Slovak/Ukrainian/Belarusian, and the sibilant
/// classes). The result is a compact fingerprint that collapses pleophony,
/// vowel-quality shifts, and nasal/jat differences so that true cognates across
/// East/West/South land on the same key.
pub fn consonant_key(word: &str) -> String {
    let skel = ascii_skeleton(word);
    let mut out = String::with_capacity(skel.len());
    let mut prev = '\0';
    for ch in skel.chars() {
        let mapped = match ch {
            // vowels and semivowels: dropped
            'a' | 'e' | 'i' | 'o' | 'u' | 'y' | 'j' | '\'' | 'ь' | 'ъ' => '\0',
            // *g and *x both surface as h in several languages; merge to g so
            // cognates align (over-merge is safe inside one meaning group).
            'h' => 'g',
            'w' => 'v',
            'ł' => 'l',
            other => other,
        };
        if mapped != '\0' && mapped != prev {
            out.push(mapped);
            prev = mapped;
        } else if mapped == '\0' {
            prev = '\0';
        }
    }
    out
}

/// True when two consonant-key fingerprints plausibly belong to the same root —
/// one is a prefix of the other, or they share their first two consonants. Used
/// to keep only genuine reflexes of a linked reconstruction in the yer vote, so a
/// meaning cell that mixes synonyms of different roots doesn't pollute alignment.
pub fn shares_consonant_root(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return true;
    }
    if a.starts_with(b) || b.starts_with(a) {
        return true;
    }
    let a2: String = a.chars().take(2).collect();
    let b2: String = b.chars().take(2).collect();
    a2.chars().count() == 2 && a2 == b2
}

/// Case-insensitive exact equality on the flavored spelling.
pub fn exact_match(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// Match after folding both sides to the standard alphabet.
pub fn normalized_match(a: &str, b: &str) -> bool {
    fold_key(a.trim()) == fold_key(b.trim())
}

/// Match on the aggressive ASCII skeleton (loosest).
pub fn skeleton_match(a: &str, b: &str) -> bool {
    ascii_skeleton(a) == ascii_skeleton(b)
}

/// Levenshtein edit distance over Unicode scalar values.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Normalized edit distance in `[0, 1]`: edit distance over the standard spelling
/// divided by the longer length.
pub fn normalized_edit_distance(a: &str, b: &str) -> f32 {
    let sa = fold_key(a);
    let sb = fold_key(b);
    let d = levenshtein(&sa, &sb);
    let len = sa.chars().count().max(sb.chars().count()).max(1);
    d as f32 / len as f32
}

pub fn is_vowel(ch: char) -> bool {
    matches!(
        ch,
        'a' | 'e'
            | 'i'
            | 'o'
            | 'u'
            | 'y'
            | 'ě'
            | 'ę'
            | 'ų'
            | 'å'
            | 'ȯ'
            | 'ė'
            | 'á'
            | 'é'
            | 'í'
            | 'ó'
            | 'ú'
            | 'ý'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_standard_folds_flavored_letters() {
        assert_eq!(to_standard("běly"), "bely");
        assert_eq!(to_standard("rųka"), "ruka");
        assert_eq!(to_standard("mȯre"), "more");
        assert_eq!(to_standard("moŕe"), "more");
        assert_eq!(to_standard("måly"), "maly");
        assert_eq!(to_standard("međa"), "medža"); // đ → dž (B20)
        assert_eq!(to_standard("noćь"), "nočь"); // ć → č
    }

    #[test]
    fn normalized_match_ignores_flavor_only() {
        assert!(normalized_match("věra", "vera"));
        assert!(normalized_match("rųka", "ruka"));
        assert!(!normalized_match("bog", "rog"));
        assert!(!normalized_match("voda", "vodka"));
    }

    #[test]
    fn exact_match_keeps_flavor() {
        assert!(exact_match("bog", "bog"));
        assert!(!exact_match("běly", "bely"));
    }

    #[test]
    fn ascii_skeleton_folds_diacritics_keeps_vowels() {
        // ascii_skeleton folds diacritics but KEEPS vowels (unlike consonant_key).
        assert_eq!(ascii_skeleton("vodå"), "voda");
        assert_eq!(ascii_skeleton("běly"), "bely");
        assert_eq!(ascii_skeleton("žaba"), "zaba");
        assert_eq!(ascii_skeleton("moře"), "more");
    }

    #[test]
    fn shares_consonant_root_detects_cognates() {
        assert!(shares_consonant_root("bbk", "bbk")); // equal
        assert!(shares_consonant_root("bb", "bbk")); // prefix (baba ⊂ babka)
        assert!(shares_consonant_root("prstr", "prst")); // prefix (prefixed form)
        assert!(!shares_consonant_root("strc", "bbk")); // different root (starica vs babka)
    }

    #[test]
    fn consonant_key_drops_vowels() {
        // consonant_key IS the vowel-dropping key (folds *g→h too).
        let k = consonant_key("automobil");
        assert!(!k.contains('a') && !k.contains('o') && !k.contains('i'));
        // pleophony collapses onto one key across branches:
        assert_eq!(consonant_key("golova"), consonant_key("glava"));
    }
}
