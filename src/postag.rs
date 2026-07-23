//! The ONE grammar for the official dictionary's `partOfSpeech` column
//! (V15 item 5).
//!
//! `pos_raw` strings like `m.anim.`, `v.tr. ipf.`, `adj.` used to be parsed
//! by four independent substring parsers (`Pos::parse`, `parse_noun_traits`,
//! `aspect::aspect`, `check::valence_char`) plus one inline
//! re-implementation in eval.rs. Every reading of the column now lives
//! here; the old public entry points remain as shims. Bodies are verbatim
//! from their original homes — this is motion, not redesign.

use crate::model::{Gender, NounTraits, Pos};

/// Part of speech. Wiktextract-style names first, then the official
/// dictionary's leading abbreviations, then bare gender markers → noun.
pub fn pos(raw: &str) -> Pos {
    let s = raw.trim().to_lowercase();
    if s.is_empty() {
        return Pos::Other;
    }
    // Wiktextract style first.
    match s.as_str() {
        "noun" => return Pos::Noun,
        "proper noun" | "proper_noun" | "name" => return Pos::ProperNoun,
        "verb" => return Pos::Verb,
        "adj" | "adjective" => return Pos::Adjective,
        "adv" | "adverb" => return Pos::Adverb,
        "num" | "numeral" | "number" => return Pos::Numeral,
        "pron" | "pronoun" => return Pos::Pronoun,
        "prep" | "preposition" | "postp" => return Pos::Preposition,
        "conj" | "conjunction" => return Pos::Conjunction,
        "intj" | "interjection" => return Pos::Interjection,
        "particle" | "prtcl" => return Pos::Particle,
        "prefix" => return Pos::Prefix,
        "suffix" | "affix" => return Pos::Suffix,
        "phrase" | "proverb" | "idiom" => return Pos::Phrase,
        _ => {}
    }
    // Official dictionary style (leading abbreviation).
    if s.starts_with("v.") || s.starts_with("v ") || s == "v" {
        return Pos::Verb;
    }
    if s.starts_with("adj") {
        return Pos::Adjective;
    }
    if s.starts_with("adv") {
        return Pos::Adverb;
    }
    if s.starts_with("num") {
        return Pos::Numeral;
    }
    if s.starts_with("pron") {
        return Pos::Pronoun;
    }
    if s.starts_with("prep") || s.starts_with("postp") {
        return Pos::Preposition;
    }
    if s.starts_with("conj") {
        return Pos::Conjunction;
    }
    if s.starts_with("intj") {
        return Pos::Interjection;
    }
    if s.starts_with("prefix") {
        return Pos::Prefix;
    }
    if s.starts_with("suffix") {
        return Pos::Suffix;
    }
    if s.starts_with("phrase") {
        return Pos::Phrase;
    }
    // Bare gender markers -> noun. `m.`, `f.`, `n.`, `m.anim.`, `f.sg.` ...
    if s.starts_with("m.")
        || s.starts_with("f.")
        || s.starts_with("n.")
        || s == "m"
        || s == "f"
        || s == "n"
        || s.starts_with("m/")
    {
        return Pos::Noun;
    }
    Pos::Other
}

/// Nominal metadata (`m.anim.`, `f.pl.`, `indecl.`).
pub fn noun_traits(raw: &str) -> NounTraits {
    let s = raw.to_lowercase();
    let mut t = NounTraits::default();
    if s.starts_with("m.") || s == "m" || s.starts_with("m/") || s.starts_with("m ") {
        t.gender = Some(Gender::Masculine);
    } else if s.starts_with("f.") || s == "f" {
        t.gender = Some(Gender::Feminine);
    } else if s.starts_with("n.") || s == "n" {
        t.gender = Some(Gender::Neuter);
    }
    t.animate = s.contains("anim");
    t.plural_only = s.contains(".pl") || s.contains("pl.");
    t.singular_only = s.contains(".sg") || s.contains("sg.");
    t.indeclinable = s.contains("indecl");
    t
}

/// Verb aspect marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aspect {
    Imperfective,
    Perfective,
    Biaspectual,
}

impl Aspect {
    pub fn code(self) -> &'static str {
        match self {
            Self::Imperfective => "ipf",
            Self::Perfective => "pf",
            Self::Biaspectual => "ipf/pf",
        }
    }
}

/// Verb aspect from the tag. Order matters: `ipf./pf.` (biaspectual) must
/// be tested before its two substrings.
pub fn aspect(pos_raw: &str) -> Option<Aspect> {
    if pos_raw.contains("ipf./pf.") {
        Some(Aspect::Biaspectual)
    } else if pos_raw.contains("ipf.") {
        Some(Aspect::Imperfective)
    } else if pos_raw.contains("pf.") {
        Some(Aspect::Perfective)
    } else {
        None
    }
}

/// Verb valence: `'t'` transitive, `'i'` intransitive, `'r'` reflexive,
/// `' '` untagged/ambiguous. `.intr` must be tested before `.tr` (substring).
pub fn valence(pos_raw: &str) -> char {
    let s = pos_raw.to_lowercase();
    if s.contains(".intr") {
        'i'
    } else if s.contains(".refl") {
        'r'
    } else if s.contains(".tr") {
        't'
    } else {
        ' '
    }
}
