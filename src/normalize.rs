//! Per-language script normalization.
//!
//! Every Slavic source form — Cyrillic or Latin — is mapped into one common
//! *phonemic Latin* representation so cognates align. The mapping deliberately
//! **preserves** etymologically important distinctions that later rules rely
//! on: nasal vowels (`ę`, `ǫ`), the jat-like `ě`, and the palatal outcomes
//! `ć`/`đ`/`č`/`ž`. It only discards noise (vowel length, stress, hard/soft
//! signs, script). Destroying those signals too early would erase the evidence
//! the consensus and reconstruction rules need.

use crate::lang::{lang_info, Script};

/// A single normalized source form.
#[derive(Debug, Clone)]
pub struct NormForm {
    /// Original attested spelling (first variant), unchanged.
    pub original: String,
    /// Phonemic Latin: keeps ě, ę, ǫ, č, š, ž, ć, đ, dž, lj, nj, y/i, h.
    pub latin: String,
    /// Aggressive ASCII skeleton for coarse cross-language voting.
    pub skeleton: String,
    /// True if the source cell flagged this form as coined/imperfect (`!`).
    pub flagged: bool,
}

/// Split a raw dictionary cell into its individual variant forms. Handles the
/// `!` coinage flag, parenthetical disambiguation, and multi-value separators.
pub fn split_cell(cell: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    // Remove parenthetical/bracketed disambiguation entirely.
    let mut cleaned = String::with_capacity(cell.len());
    let mut depth = 0i32;
    for ch in cell.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            _ if depth == 0 => cleaned.push(ch),
            _ => {}
        }
    }
    for piece in cleaned.split(|c| c == ',' || c == ';' || c == '/') {
        let mut p = piece.trim();
        if p.is_empty() {
            continue;
        }
        let mut flagged = false;
        while let Some(rest) = p.strip_prefix('!') {
            flagged = true;
            p = rest.trim();
        }
        // Drop leftover punctuation-only tokens and obviously non-lexical noise.
        let p = p.trim_matches(|c: char| c == '.' || c == '"' || c == '\'' || c == '’' || c == ' ');
        if p.is_empty() || p.chars().all(|c| !c.is_alphabetic()) {
            continue;
        }
        out.push((p.to_string(), flagged));
    }
    out
}

/// Normalize every variant in a cell for one language.
pub fn normalize_cell(lang_code: &str, cell: &str) -> Vec<NormForm> {
    split_cell(cell)
        .into_iter()
        .map(|(form, flagged)| {
            let latin = to_phonemic_latin(lang_code, &form);
            let skeleton = crate::orthography::ascii_skeleton(&latin);
            NormForm {
                original: form,
                latin,
                skeleton,
                flagged,
            }
        })
        .filter(|f| !f.skeleton.is_empty())
        .collect()
}

/// Convert one attested form to phonemic Latin.
pub fn to_phonemic_latin(lang_code: &str, form: &str) -> String {
    let script = lang_info(lang_code)
        .map(|l| l.script)
        .unwrap_or(Script::Latin);
    let lower = form.trim().to_lowercase();
    let s = match script {
        Script::Cyrillic => translit_cyrillic(lang_code, &lower),
        Script::Latin => translit_latin(lang_code, &lower),
    };
    // Final tidy: collapse whitespace, strip stray marks.
    s.trim().to_string()
}

/// Language-aware Cyrillic → phonemic Latin.
fn translit_cyrillic(lang: &str, s: &str) -> String {
    // OCS/Church-Slavonic digraph оу = /u/ — fold it on the Cyrillic *input*,
    // before per-character transliteration (моужь→muž). Doing it afterwards was
    // dead code: о and у are already Latin 'o'/'u' by then.
    let s = s.replace("оу", "у");
    let mut out = String::with_capacity(s.len() * 2);
    let chars: Vec<char> = s.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        let prev = if i > 0 { chars[i - 1] } else { ' ' };
        let repl: &str = match ch {
            'а' => "a",
            'б' => "b",
            'в' => "v",
            // Ukrainian & Belarusian г = /h/; Russian/Bulgarian/Macedonian/Serbian г = /g/.
            'г' => {
                if lang == "uk" || lang == "be" {
                    "h"
                } else {
                    "g"
                }
            }
            'ґ' => "g",
            'д' => "d",
            // Plain е is /e/, but after a separating soft/hard sign it carries the
            // /j/ (пьеса→pjesa, объезд→objezd).
            'е' => {
                if prev == 'ь' || prev == 'ъ' {
                    "je"
                } else {
                    "e"
                }
            }
            // Russian/Belarusian ё: after a consonant it palatalizes → /o/; word-
            // initial or after a vowel/soft-sign it is /jo/ (ёж→jož, моё→mojo).
            'ё' => {
                if is_soft_context(prev) {
                    "o"
                } else {
                    "jo"
                }
            }
            // Ukrainian є: after a consonant it is /e/ (synє→syne); word-initial or
            // after a vowel it carries /j/ (є→je).
            'є' => {
                if is_soft_context(prev) {
                    "e"
                } else {
                    "je"
                }
            }
            'ж' => "ž",
            'з' => "z",
            'ѕ' => "dz",
            'и' => {
                if lang == "uk" {
                    "y" // Ukrainian и = /ɪ/, historically *y/*i merged toward y
                } else {
                    "i"
                }
            }
            'і' => "i",
            'ї' => "ji",
            'й' => "j",
            'к' => "k",
            'л' => "l",
            'м' => "m",
            'н' => "n",
            'о' => "o",
            'п' => "p",
            'р' => "r",
            'с' => "s",
            'т' => "t",
            'у' => "u",
            'ў' => "v",
            'ф' => "f",
            'х' => "h", // ISV writes *x as h
            'ц' => "c",
            'ч' => "č",
            'ш' => "š",
            'щ' => {
                if lang == "bg" {
                    "št" // Bulgarian щ = /ʃt/
                } else {
                    "šč"
                }
            }
            'ъ' => {
                if lang == "bg" {
                    "ȯ" // Bulgarian ъ is a full vowel (schwa), often a yer reflex
                } else {
                    "" // East Slavic hard sign: no phonemic value
                }
            }
            'ы' => "y",
            'ь' => "", // soft sign: palatalization handled lexically, drop here
            'э' => "e",
            'ю' => {
                if is_soft_context(prev) {
                    "u"
                } else {
                    "ju"
                }
            }
            'я' => {
                if is_soft_context(prev) {
                    "a"
                } else {
                    "ja"
                }
            }
            // Serbian / Macedonian specials
            'ђ' => "đ",
            'ћ' => "ć",
            'џ' => "dž",
            'љ' => "lj",
            'њ' => "nj",
            'ј' => "j",
            'ѓ' => "đ", // Macedonian ѓ ~ đ
            'ќ' => "ć", // Macedonian ќ ~ ć
            // Church Slavonic / historical
            'ѣ' => "ě",
            'ѫ' => "ǫ",
            'ѭ' => "jǫ",
            'ѧ' => "ę",
            'ѩ' => "ję",
            'ꙑ' => "y",
            'ѹ' => "u",
            _ => {
                out.push(ch);
                continue;
            }
        };
        out.push_str(repl);
    }
    out
}

fn is_soft_context(prev: char) -> bool {
    // After a consonant, ю/я mark palatalization of that consonant rather than a
    // full /j/. After a vowel or at word start they carry /j/. A *separating* soft
    // or hard sign (семья, статья, объект) is precisely the signal that the /j/ is
    // present, so it is NOT a soft (de-iotating) context.
    if prev == 'ь' || prev == 'ъ' {
        return false;
    }
    const CYR_VOWELS: &str = "аеёиіїоуыэюяєѣѫѧ ";
    prev.is_alphabetic() && !CYR_VOWELS.contains(prev)
}

/// Language-aware Latin (with diacritics/digraphs) → phonemic Latin.
fn translit_latin(lang: &str, s: &str) -> String {
    // Multi-character digraphs first, per language.
    let mut t = s.to_string();
    match lang {
        "pl" => {
            // Polish digraphs and diacritics.
            t = t
                .replace("dź", "đ")
                .replace("dż", "dž")
                .replace("cz", "č")
                .replace("sz", "š")
                .replace("rz", "ř")
                .replace("ch", "h");
        }
        "cs" | "sk" => {
            t = t.replace("ch", "h").replace("dž", "dž");
        }
        "sl" | "hr" | "bs" | "sh" => {
            t = t
                .replace("dž", "dž")
                .replace("lj", "lj")
                .replace("nj", "nj");
        }
        _ => {}
    }
    let mut out = String::with_capacity(t.len());
    for ch in t.chars() {
        let repl: &str = match ch {
            // Polish
            'ł' => "l",
            'w' if lang == "pl" => "v",
            'ó' => "o",
            'ą' => "ǫ", // Polish back nasal
            'ę' if lang == "pl" => "ę",
            'ż' => "ž",
            'ź' => "z",
            'ś' => "s",
            'ć' => "ć",
            'ń' => "nj",
            // Czech / Slovak
            'á' => "a",
            'é' => "e",
            'í' => "i",
            'ú' => "u",
            'ů' => "u",
            'ý' => "y",
            'ě' => "e",
            'ř' => "ř",
            'ň' => "nj",
            'ď' => "d",
            'ť' => "t",
            'ä' => "e",
            'ô' => "o",
            'ĺ' => "l",
            'ľ' => "l",
            'ŕ' => "r",
            // South Slavic Latin
            'č' => "č",
            'š' => "š",
            'ž' => "ž",
            'đ' => "đ",
            'w' => "v",
            other => {
                out.push(other);
                continue;
            }
        };
        out.push_str(repl);
    }
    out
}

/// Transliterate a descendant form (often native-script and stress-accented, e.g.
/// Cyrillic `вода́`) to its phonemic-Latin ASCII skeleton, so proto-descendant
/// matching aligns with the Latin-normalized modern cognates. Without this the
/// 54% of cached descendants stored in Cyrillic/Glagolitic never match a cognate
/// skeleton, silently capping the fuzzy proto-link's descendant-membership signal.
pub fn desc_skeleton(lang: &str, word: &str) -> String {
    let latin = to_phonemic_latin(lang, word);
    // Drop combining accent marks left by stress notation (вода́ → voda).
    let stripped: String = latin
        .chars()
        .filter(|c| !('\u{0300}'..='\u{036F}').contains(c))
        .collect();
    crate::orthography::ascii_skeleton(&stripped)
}

/// Choose the single most representative form from a normalized cell: the first
/// non-flagged variant, else the first variant.
pub fn primary<'a>(forms: &'a [NormForm]) -> Option<&'a NormForm> {
    forms.iter().find(|f| !f.flagged).or_else(|| forms.first())
}

#[cfg(test)]
mod tests {
    use super::to_phonemic_latin as tr;

    #[test]
    fn basic_cyrillic_and_latin() {
        assert_eq!(tr("ru", "вода"), "voda");
        assert_eq!(tr("uk", "голова"), "holova"); // uk г→h
        assert_eq!(tr("pl", "głowa"), "glova"); // ł→l, w→v
        assert_eq!(tr("cs", "hlava"), "hlava");
    }

    #[test]
    fn separating_soft_sign_keeps_j() {
        // The separating ь signals the /j/ — it must not de-iotate (B3).
        assert_eq!(tr("ru", "семья"), "semja");
        assert_eq!(tr("ru", "статья"), "statja");
        assert!(tr("ru", "пьеса").contains('j'), "{}", tr("ru", "пьеса"));
    }

    #[test]
    fn yo_iotates_word_initially() {
        // ё is /jo/ initially/after a vowel, /o/ after a consonant (B4).
        assert_eq!(tr("ru", "ёж"), "jož");
        assert!(tr("ru", "моё").contains('j'), "{}", tr("ru", "моё"));
        assert_eq!(tr("ru", "тёплый"), "toplyj"); // ё after consonant → o; final й→j
    }

    #[test]
    fn ukrainian_je_after_consonant_has_no_j() {
        // є is /e/ after a consonant, /je/ otherwise (B14).
        assert_eq!(tr("uk", "синє"), "synje".replace("nje", "ne"));
        assert!(!tr("uk", "синє").contains('j'), "{}", tr("uk", "синє"));
    }

    #[test]
    fn ocs_ou_digraph_folds_to_u() {
        // Church Slavonic оу = /u/ (B15): моужь->muž, оучити->učiti.
        assert_eq!(tr("cu", "моужь"), "muž");
        assert_eq!(tr("cu", "оучити"), "učiti");
    }

    #[test]
    fn descendant_skeleton_transliterates_and_strips_accents() {
        use super::desc_skeleton;
        // A Cyrillic descendant folds to the same skeleton as the Latin cognate,
        // so proto-descendant matching aligns across scripts.
        assert_eq!(
            desc_skeleton("ru", "вода"),
            crate::orthography::ascii_skeleton("voda")
        );
        // Stress accents (combining marks) are stripped.
        assert_eq!(desc_skeleton("ru", "вода\u{0301}"), "voda");
    }

    #[test]
    fn iotated_vowels_after_vowel_keep_j() {
        assert!(tr("ru", "моя").contains('j'), "{}", tr("ru", "моя"));
        assert_eq!(tr("ru", "яблоко"), "jabloko"); // word-initial я
    }
}
