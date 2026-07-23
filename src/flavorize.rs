//! Display-grade flavorization of source-language words into Interslavic
//! orthography, per `data/FLAVORIZATION_SPEC.md` (issue #62).
//!
//! Two entry points with deliberately different contracts:
//!
//! - [`flavorize_word`] — for *words displayed as words* (raw-lemma display
//!   headwords, cross-lingual chips, cognate word mentions). Adapts the
//!   national spelling into the ISV standard alphabet (+ `ě`): Polish
//!   `winyl→vinyl`, Russian `дело→dělo`, Macedonian `меѓу→medžu`, with a
//!   POS-gated citation-ending layer (`читать→čitati`, `dělat→dělati`).
//! - [`translit_text`] — for *running text* (quoted etymology paragraphs,
//!   glosses). Script-faithful transliteration only: no jat marking, no ending
//!   adaptation — flavorizing a quoted sentence would misquote the source.
//!   Currently transliterates `ru` (byte-identical to the retired
//!   `russian_translit` module, whose tests moved here) and passes every other
//!   language through; extending it per-edition is issue #38.
//!
//! This module is display-only. The consensus vote keeps its own normalizer
//! (`normalize.rs`, benchmark-gated); the voting-machine port experiments all
//! regressed (`data/VOTING_MACHINE_NOTES.md`), so nothing here may leak into
//! voting, generation, or the forms API.

/// True for letters of the ISV standard Latin alphabet (plus `ě`, which is
/// part of it) — the only alphabetic characters flavorized output should
/// contain. Anything else in a flavorized word is counted as residue and
/// reported loudly at export/coverage time, never silently shipped.
pub fn is_isv_letter(c: char) -> bool {
    match c {
        'a'..='z' => !matches!(c, 'q' | 'w' | 'x'),
        'č' | 'š' | 'ž' | 'ě' => true,
        _ => c.is_uppercase() && c.to_lowercase().all(|lc| lc != c && is_isv_letter(lc)),
    }
}

/// Alphabetic characters of `word` that fall outside the ISV standard
/// alphabet — the validation residue of spec §2 stage 5.
pub fn residue_chars(word: &str) -> impl Iterator<Item = char> + '_ {
    word.chars()
        .filter(|&c| c.is_alphabetic() && !is_isv_letter(c))
}

/// Flavorize one attested word (spec §2): strip combining marks, adapt the
/// POS-gated citation ending, run the per-language rewrite, fold foreign
/// letters, and restore the leading capital. Deterministic; a word from an
/// unknown language code gets the common post-pass only.
///
/// `pos` enables the ending layer (`"verb"` / `"adj"`); pass `""` when the
/// part of speech is unknown (word chips) to disable ending adaptation.
pub fn flavorize_word(lang: &str, pos: &str, word: &str) -> String {
    let stripped = strip_marks(word.trim());
    let first_upper = stripped
        .chars()
        .find(|c| c.is_alphabetic())
        .is_some_and(char::is_uppercase);
    let lower = stripped.to_lowercase();
    let lower = adapt_ending(lang, pos, &lower);
    let mapped = match lang {
        "ru" | "uk" | "be" | "rue" | "bg" | "mk" | "cu" | "orv" => cyr_word(lang, &lower),
        // BCMS: Serbian Cyrillic transliterates first, then the shared Latin
        // rules (so an Ijekavian Cyrillic ије still yields ě).
        "sr" | "sh" | "hr" | "bs" => latin_scan(&bcms_cyr_to_latin(&lower), SH_RULES),
        "pl" => latin_scan(&lower, PL_RULES),
        "cs" => latin_scan(&lower, CS_RULES),
        "sk" => latin_scan(&lower, SK_RULES),
        "hsb" | "dsb" => latin_scan(&lower, SORBIAN_RULES),
        // Silesian / Kashubian: fold their extra vowel letters onto the
        // Polish inventory, then reuse the Polish scanner (spec §4.12).
        "szl" => latin_scan(&fold_chars(&lower, SZL_FOLD), PL_RULES),
        "csb" => latin_scan(&fold_chars(&lower, CSB_FOLD), PL_RULES),
        // Slovenian is already ISV-compatible modulo accents/loan letters,
        // which the common post-pass handles (spec §4.7); ditto unknown codes.
        _ => lower.clone(),
    };
    let mapped = post_pass(&mapped);
    let mapped = adj_final_i(lang, pos, mapped);
    restore_first_case(first_upper, &mapped)
}

/// Script-faithful running-text transliteration (spec §0 non-scope). Russian
/// is transliterated exactly as the retired `russian_translit` module did;
/// other languages pass through verbatim (extending them is issue #38).
pub fn translit_text(lang: &str, text: &str) -> String {
    if lang == "ru" {
        ru_text(text)
    } else {
        text.to_string()
    }
}

/// Drop combining stress/length marks (Wiktionary headwords carry them).
fn strip_marks(s: &str) -> String {
    s.chars()
        .filter(|c| !crate::orthography::is_combining_mark(*c))
        .collect()
}

/// POS-gated citation-ending adaptation (spec §2.2), applied to the lowercase
/// *source* spelling before the per-language rewrite so the letter rules see
/// the adapted ending (`читать→читати`, `być→byti`).
fn adapt_ending(lang: &str, pos: &str, w: &str) -> String {
    let rep = |suf: &str, to: &str| -> Option<String> {
        w.strip_suffix(suf).map(|stem| format!("{stem}{to}"))
    };
    let adapted = match (lang, pos) {
        ("ru", "verb") => rep("ть", "ти"),
        ("be", "verb") => rep("ць", "ці"),
        ("ru", "adj") => rep("ый", "ы").or_else(|| rep("ий", "ы")),
        ("uk", "adj") | ("rue", "adj") => rep("ий", "и"),
        // -ct (moct) keeps the transparent cluster, like ISV mogti (spec §2.2).
        ("cs", "verb") if !w.ends_with("ct") => rep("t", "ti"),
        ("sk", "verb") => rep("ť", "ti"),
        ("pl", "verb") | ("csb", "verb") | ("szl", "verb") | ("hsb", "verb") => rep("ć", "ti"),
        ("dsb", "verb") => rep("ś", "ti"),
        _ => None,
    };
    adapted.unwrap_or_else(|| w.to_string())
}

/// sh/hr/bs/sr/mk/bg adjectives are cited in a form ending `-i` (sh definite
/// `novi`); ISV cites `-y` (spec §2.2). Applied post-map, POS-gated.
fn adj_final_i(lang: &str, pos: &str, s: String) -> String {
    if pos == "adj" && matches!(lang, "sh" | "hr" | "bs" | "sr" | "mk" | "bg") && s.ends_with('i') {
        let mut t = s;
        t.pop();
        t.push('y');
        t
    } else {
        s
    }
}

fn restore_first_case(upper: bool, s: &str) -> String {
    if !upper {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut done = false;
    for c in s.chars() {
        if !done && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            done = true;
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Cyrillic word mode (spec §4.1–4.3, §4.9–4.10, §4.13)
// ---------------------------------------------------------------------------

const CYR_VOWELS: &str = "аеёиіїоуыэюяєѣѫѧѩѭꙑѹꙗ";

/// After a consonant (for the soft-e rule): the previous character is an
/// alphabetic Cyrillic non-vowel that is not a soft/hard sign or separator.
fn cyr_after_cons(prev: Option<char>) -> bool {
    match prev {
        Some(p) => {
            p.is_alphabetic()
                && !CYR_VOWELS.contains(p)
                && !matches!(p, 'ь' | 'ъ' | 'ʼ' | '’' | '\'')
        }
        None => false,
    }
}

fn is_cyr_front(c: char) -> bool {
    // Belarusian dzekanne/cekanne context: front vowels + soft sign.
    matches!(c, 'е' | 'ё' | 'і' | 'ь' | 'ю' | 'я')
}

fn cyr_word(lang: &str, s: &str) -> String {
    // OCS digraph оу = /u/ folds on the Cyrillic input (as in normalize.rs);
    // yers resolve by Havlík's law before the per-character pass (spec §4.13).
    let owned;
    let s = if lang == "cu" || lang == "orv" {
        owned = resolve_yers(&s.replace("оу", "у"));
        owned.as_str()
    } else {
        s
    };
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() * 2);
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        let next = chars.get(i + 1).copied();
        // Belarusian dzekanne: дз + front vowel is soft *d (дзень→děnj).
        if lang == "be"
            && ch == 'д'
            && next == Some('з')
            && chars.get(i + 2).copied().is_some_and(is_cyr_front)
        {
            out.push('d');
            i += 2;
            continue;
        }
        let east = matches!(lang, "ru" | "be" | "rue");
        let repl: &str = match ch {
            'а' => "a",
            'б' => "b",
            'в' => "v",
            'г' => {
                if matches!(lang, "uk" | "be" | "rue") {
                    "h"
                } else {
                    "g"
                }
            }
            'ґ' => "g",
            'д' => "d",
            // The soft-e principle (spec §3): ru/be е marks palatalization →
            // ě after a consonant, je elsewhere. Other languages' е is hard.
            'е' => {
                if east {
                    if cyr_after_cons(prev) {
                        "ě"
                    } else {
                        "je"
                    }
                } else {
                    "e"
                }
            }
            // ё < *e (spec §4.1): самолёт→samolet, ёж→jež.
            'ё' => {
                if cyr_after_cons(prev) {
                    "e"
                } else {
                    "je"
                }
            }
            // Ukrainian є is the palatalization-marked e.
            'є' => {
                if cyr_after_cons(prev) {
                    "ě"
                } else {
                    "je"
                }
            }
            'ж' => "ž",
            'з' => "z",
            'ѕ' => "dz",
            'и' => {
                if matches!(lang, "uk" | "rue") {
                    "y"
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
            'х' => "h",
            // Belarusian cekanne: ц + front vowel is soft *t (цень→těnj).
            'ц' => {
                if lang == "be" && next.is_some_and(is_cyr_front) {
                    "t"
                } else {
                    "c"
                }
            }
            'ч' => "č",
            'ш' => "š",
            'щ' => {
                if matches!(lang, "bg" | "cu" | "orv") {
                    "št"
                } else {
                    "šč"
                }
            }
            'ъ' => {
                if lang == "bg" {
                    "o"
                } else {
                    ""
                }
            }
            'ы' => "y",
            // Soft sign: soft l/n survive as ISV lj/nj word-finally and before
            // a consonant (соль→solj, конь→konj); Bulgarian ьо carries /j/;
            // otherwise palatalization folds away (кровать→krovat, ISV t́→t).
            'ь' => {
                if next == Some('о')
                    || (matches!(prev, Some('л') | Some('н'))
                        && next.is_none_or(|n| !CYR_VOWELS.contains(n)))
                {
                    "j"
                } else {
                    ""
                }
            }
            'э' => "e",
            // Word mode keeps the soft signal as j everywhere (буря→burja,
            // земля→zemlja — matching ISV lj/nj/rj); text mode drops it.
            'ю' => "ju",
            'я' => "ja",
            // uk/be separating apostrophe: the following я/ю/ї already carry
            // the /j/ under the uniform ja/ju rule, so the mark itself drops.
            'ʼ' | '’' | '\'' => "",
            // Serbian/Macedonian specials (word mode: standard-alphabet folds).
            'ђ' => "dž",
            'ћ' => "č",
            'џ' => "dž",
            'љ' => "lj",
            'њ' => "nj",
            'ј' => "j",
            'ѓ' => "dž",
            'ќ' => "č",
            // Church Slavonic / historical (spec §4.13; nasals fold to the
            // standard alphabet — a raw attestation must not fake precision).
            'ѣ' => "ě",
            'ѫ' => "u",
            'ѭ' => "ju",
            'ѧ' => "e",
            'ѩ' => "je",
            'ꙑ' => "y",
            'ѹ' => "u",
            'ѳ' => "f",
            'ѵ' => "i",
            'ꙗ' => "ja",
            'ѥ' => "je",
            'ѡ' => "o",
            'ѐ' => "e",
            'ѝ' => "i",
            'ѿ' => "ot",
            'ꙋ' => "u",
            'ꙃ' => "dz",
            'ꙉ' => "dž",
            _ => {
                out.push(ch);
                i += 1;
                continue;
            }
        };
        out.push_str(repl);
        i += 1;
    }
    out
}

/// Havlík's law over OCS/Old-East-Slavic yers, deterministic from spelling
/// (spec §4.13): counting nuclei right-to-left, a yer is weak when the nucleus
/// to its right is a strong yer or a normal vowel (or nothing), strong when it
/// is a weak yer. Weak yers drop; strong ь→е, ъ→о (сънъ→son, дьнь→den).
fn resolve_yers(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut strong = vec![false; chars.len()];
    // 0 = word end / normal vowel to the right, 1 = weak yer, 2 = strong yer.
    let mut right_state = 0u8;
    for i in (0..chars.len()).rev() {
        let c = chars[i];
        if c == 'ь' || c == 'ъ' {
            if right_state == 1 {
                strong[i] = true;
                right_state = 2;
            } else {
                right_state = 1;
            }
        } else if CYR_VOWELS.contains(c) {
            right_state = 0;
        }
    }
    chars
        .iter()
        .enumerate()
        .filter_map(|(i, &c)| match c {
            'ь' => strong[i].then_some('е'),
            'ъ' => strong[i].then_some('о'),
            _ => Some(c),
        })
        .collect()
}

/// Serbian Cyrillic → BCMS Latin (identity on Latin input), so sr/sh words in
/// either script reach the shared Latin rules.
fn bcms_cyr_to_latin(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let repl: &str = match ch {
            'а' => "a",
            'б' => "b",
            'в' => "v",
            'г' => "g",
            'д' => "d",
            'ђ' => "đ",
            'е' => "e",
            'ж' => "ž",
            'з' => "z",
            'и' => "i",
            'ј' => "j",
            'к' => "k",
            'л' => "l",
            'љ' => "lj",
            'м' => "m",
            'н' => "n",
            'њ' => "nj",
            'о' => "o",
            'п' => "p",
            'р' => "r",
            'с' => "s",
            'т' => "t",
            'ћ' => "ć",
            'у' => "u",
            'ф' => "f",
            'х' => "h",
            'ц' => "c",
            'ч' => "č",
            'џ' => "dž",
            'ш' => "š",
            // Pre-reform / quoted-archaic spellings inside sh/sr entries.
            'ь' | 'ъ' => "",
            'ѣ' => "ě",
            'ѕ' => "dz",
            _ => {
                out.push(ch);
                continue;
            }
        };
        out.push_str(repl);
    }
    out
}

// ---------------------------------------------------------------------------
// Latin word mode: a small ordered-rule scanner (spec §4.4–4.8, §4.11–4.12)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum When {
    Always,
    /// Previous source character is a consonant letter.
    AfterCons,
    /// Pattern ends the word (or is followed by a non-letter).
    WordFinal,
    /// Pattern is followed by a vowel letter.
    BeforeVowel,
}

struct LRule(&'static str, &'static str, When);

const LATIN_VOWELS: &str = "aeiouyáàâāäãåéèêëēěęąíìîïóòôöõōŏȯúùûüūůýÿ";

fn is_latin_vowel(c: char) -> bool {
    LATIN_VOWELS.contains(c)
}

fn is_latin_cons(c: char) -> bool {
    c.is_alphabetic() && !is_latin_vowel(c)
}

/// Apply the first matching rule at each position (rules are listed longest /
/// most specific first); unmatched characters pass through.
fn latin_scan(s: &str, rules: &[LRule]) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() * 2);
    let mut i = 0;
    'outer: while i < chars.len() {
        for LRule(pat, to, when) in rules {
            let plen = pat.chars().count();
            if i + plen > chars.len() || !chars[i..i + plen].iter().copied().eq(pat.chars()) {
                continue;
            }
            let ok = match when {
                When::Always => true,
                When::AfterCons => i > 0 && is_latin_cons(chars[i - 1]),
                When::WordFinal => chars.get(i + plen).is_none_or(|c| !c.is_alphabetic()),
                When::BeforeVowel => chars.get(i + plen).copied().is_some_and(is_latin_vowel),
            };
            if ok {
                out.push_str(to);
                i += plen;
                continue 'outer;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

use When::{AfterCons, Always, BeforeVowel, WordFinal};

/// Polish (spec §4.4). The soft-marking `i` de-palatalizes dentals to the
/// etymological stop (nici→niti, ciało→talo), keeps ISV's own soft n as nj
/// (niania→njanja-class), marks soft e as ě (niebo→něbo, wiek→věk), and folds
/// the nasals (ę/ią/ię < *ę → e; ą < *ǫ → u). rz < *ŕ → r, with rě before e.
const PL_RULES: &[LRule] = &[
    // Dental soft clusters + vowel (i is the palatalization mark).
    LRule("dzie", "dě", Always),
    LRule("dzię", "de", Always),
    LRule("dzią", "de", Always),
    LRule("dzia", "da", Always),
    LRule("dzio", "do", Always),
    LRule("dziu", "du", Always),
    LRule("dzi", "di", Always),
    LRule("cie", "tě", Always),
    LRule("cię", "te", Always),
    LRule("cią", "te", Always),
    LRule("cia", "ta", Always),
    LRule("cio", "to", Always),
    LRule("ciu", "tu", Always),
    LRule("ci", "ti", Always),
    LRule("sie", "sě", Always),
    LRule("się", "se", Always),
    LRule("sią", "se", Always),
    LRule("sia", "sa", Always),
    LRule("sio", "so", Always),
    LRule("siu", "su", Always),
    LRule("zie", "zě", Always),
    LRule("zię", "ze", Always),
    LRule("zią", "ze", Always),
    LRule("zia", "za", Always),
    LRule("zio", "zo", Always),
    LRule("ziu", "zu", Always),
    LRule("nie", "ně", Always),
    LRule("nię", "ne", Always),
    LRule("nią", "ne", Always),
    LRule("nia", "nja", Always),
    LRule("nio", "njo", Always),
    LRule("niu", "nju", Always),
    // Soft-marking i after the remaining consonants (labials, velars).
    LRule("ie", "ě", AfterCons),
    LRule("ię", "e", AfterCons),
    LRule("ią", "e", AfterCons),
    LRule("ia", "ja", AfterCons),
    LRule("io", "jo", AfterCons),
    LRule("iu", "ju", AfterCons),
    // After a husher, Polish y spells etymological *i (czysty < *čistъ,
    // przy < *pri, szyja < *šija, żyto < *žito).
    LRule("rzy", "ri", Always),
    LRule("czy", "či", Always),
    LRule("szy", "ši", Always),
    LRule("ży", "ži", Always),
    // Digraphs.
    LRule("rze", "rě", Always),
    LRule("rz", "r", Always),
    LRule("sz", "š", Always),
    LRule("cz", "č", Always),
    LRule("dż", "dž", Always),
    LRule("dź", "d", Always),
    LRule("ch", "h", Always),
    // Single letters.
    LRule("ć", "t", Always),
    LRule("ś", "s", Always),
    LRule("ź", "z", Always),
    LRule("ń", "nj", WordFinal),
    LRule("ń", "n", Always),
    LRule("ż", "ž", Always),
    LRule("ó", "o", Always),
    LRule("ł", "l", Always),
    LRule("ą", "u", Always),
    LRule("ę", "e", Always),
];

/// Czech (spec §4.5): vowel length is noise, ů < *o, ou < *u/*ǫ, ě kept.
const CS_RULES: &[LRule] = &[
    LRule("ch", "h", Always),
    LRule("ou", "u", Always),
    LRule("ře", "rě", Always),
    LRule("ř", "r", Always),
    LRule("ď", "dj", BeforeVowel),
    LRule("ď", "d", Always),
    LRule("ť", "tj", BeforeVowel),
    LRule("ť", "t", Always),
    LRule("ň", "nj", BeforeVowel),
    LRule("ň", "nj", WordFinal),
    LRule("ň", "n", Always),
    LRule("á", "a", Always),
    LRule("é", "e", Always),
    LRule("í", "i", Always),
    LRule("ó", "o", Always),
    LRule("ú", "u", Always),
    LRule("ů", "o", Always),
    LRule("ý", "y", Always),
];

/// Slovak (spec §4.6): Czech plus the diphthongs (ie < *ě), ä < *ę, ô < *o.
const SK_RULES: &[LRule] = &[
    LRule("ie", "ě", Always),
    LRule("ia", "ja", Always),
    LRule("iu", "ju", Always),
    LRule("ä", "e", Always),
    LRule("ô", "o", Always),
    LRule("ĺ", "l", Always),
    LRule("ŕ", "r", Always),
    LRule("ľ", "lj", BeforeVowel),
    LRule("ľ", "lj", WordFinal),
    LRule("ľ", "l", Always),
    LRule("ch", "h", Always),
    LRule("ou", "u", Always),
    LRule("ď", "dj", BeforeVowel),
    LRule("ď", "d", Always),
    LRule("ť", "tj", BeforeVowel),
    LRule("ť", "t", Always),
    LRule("ň", "nj", BeforeVowel),
    LRule("ň", "nj", WordFinal),
    LRule("ň", "n", Always),
    LRule("á", "a", Always),
    LRule("é", "e", Always),
    LRule("í", "i", Always),
    LRule("ó", "o", Always),
    LRule("ú", "u", Always),
    LRule("ý", "y", Always),
];

/// BCMS Latin (spec §4.8): the §1.3 standard folds (đ→dž, ć→č) plus Ijekavian
/// jat recovery (ije→ě; consonant+je→ě; word-initial je stays).
const SH_RULES: &[LRule] = &[
    LRule("ije", "ě", Always),
    LRule("je", "ě", AfterCons),
    LRule("đ", "dž", Always),
    LRule("ć", "č", Always),
];

/// Upper + Lower Sorbian union table (spec §4.11): Polish-family orthography
/// with ě kept (dźěło→dělo) and hsb ř as in Czech (přez→prěz).
const SORBIAN_RULES: &[LRule] = &[
    LRule("ch", "h", Always),
    LRule("ře", "rě", Always),
    LRule("ř", "r", Always),
    LRule("dź", "d", Always),
    LRule("ć", "t", Always),
    LRule("ś", "s", Always),
    LRule("ź", "z", Always),
    LRule("ń", "nj", WordFinal),
    LRule("ń", "n", Always),
    LRule("ó", "o", Always),
    LRule("ł", "l", Always),
    LRule("é", "e", Always),
    LRule("ŕ", "r", Always),
];

/// Silesian extra vowels → Polish inventory (spec §4.12).
const SZL_FOLD: &[(char, char)] = &[('ō', 'o'), ('ô', 'o'), ('ŏ', 'o'), ('ů', 'o')];

/// Kashubian extra vowels → Polish inventory (spec §4.12; ã < *ę → e).
const CSB_FOLD: &[(char, char)] = &[
    ('ë', 'e'),
    ('ò', 'o'),
    ('ô', 'o'),
    ('ù', 'u'),
    ('é', 'e'),
    ('ã', 'e'),
];

fn fold_chars(s: &str, table: &[(char, char)]) -> String {
    s.chars()
        .map(|c| table.iter().find(|(f, _)| *f == c).map_or(c, |(_, t)| *t))
        .collect()
}

/// Common post-pass (spec §2 stage 4): loan letters into the ISV inventory and
/// a base-letter safety net for stray accented Latin the per-language rules
/// did not claim. Target letters (č š ž ě) pass untouched.
fn post_pass(s: &str) -> String {
    let s = s.replace("qu", "kv");
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let repl: &str = match c {
            'w' => "v",
            'q' => "k",
            'x' => "ks",
            'á' | 'à' | 'â' | 'ā' | 'ã' | 'å' | 'ä' | 'ą' | 'ă' => "a",
            'é' | 'è' | 'ê' | 'ë' | 'ē' | 'ę' | 'ė' => "e",
            'í' | 'ì' | 'î' | 'ï' | 'ī' => "i",
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'ō' | 'ŏ' | 'ȯ' | 'ő' => "o",
            'ú' | 'ù' | 'û' | 'ü' | 'ū' | 'ů' | 'ų' | 'ű' => "u",
            'ý' | 'ÿ' => "y",
            'ß' => "ss",
            // Stray flavored/national letters fold per RULE_SPEC §1.3.
            'ć' => "č",
            'đ' => "dž",
            'ś' => "s",
            'ź' => "z",
            'ż' => "ž",
            'ł' => "l",
            'ń' | 'ň' | 'ñ' => "n",
            'ľ' | 'ĺ' => "l",
            'ř' | 'ŕ' => "r",
            'ď' => "d",
            'ť' => "t",
            _ => {
                out.push(c);
                continue;
            }
        };
        out.push_str(repl);
    }
    out
}

// ---------------------------------------------------------------------------
// Running-text mode: the retired russian_translit, verbatim (byte-identical).
// ---------------------------------------------------------------------------

/// Transliterate Russian text to the same broad Latin/Interslavic conventions
/// used elsewhere on the site: ж→ž, ч→č, ш→š, щ→šč, х→h, ц→c, ю/я→ju/ja or
/// u/a after consonants, and hard/soft signs omitted.
fn ru_text(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    for (i, &ch) in chars.iter().enumerate() {
        // Drop stress/combining marks common in Wiktionary Russian headwords.
        if crate::orthography::is_combining_mark(ch) {
            continue;
        }
        let prev = previous_base(&chars, i);
        let lower = ch.to_lowercase().next().unwrap_or(ch);
        let repl = match lower {
            'а' => "a",
            'б' => "b",
            'в' => "v",
            'г' => "g",
            'д' => "d",
            'е' => {
                if prev == Some('ь') || prev == Some('ъ') || !is_after_consonant(prev) {
                    "je"
                } else {
                    "e"
                }
            }
            'ё' => {
                if prev == Some('ь') || prev == Some('ъ') || !is_after_consonant(prev) {
                    "jo"
                } else {
                    "o"
                }
            }
            'ж' => "ž",
            'з' => "z",
            'и' => "i",
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
            'ф' => "f",
            'х' => "h",
            'ц' => "c",
            'ч' => "č",
            'ш' => "š",
            'щ' => "šč",
            'ъ' | 'ь' => "",
            'ы' => "y",
            'э' => "e",
            'ю' => {
                if prev == Some('ь') || prev == Some('ъ') || !is_after_consonant(prev) {
                    "ju"
                } else {
                    "u"
                }
            }
            'я' => {
                if prev == Some('ь') || prev == Some('ъ') || !is_after_consonant(prev) {
                    "ja"
                } else {
                    "a"
                }
            }
            _ => {
                out.push(ch);
                continue;
            }
        };
        if ch.is_uppercase() {
            push_capitalized(&mut out, repl);
        } else {
            out.push_str(repl);
        }
    }
    out
}

fn previous_base(chars: &[char], i: usize) -> Option<char> {
    chars[..i]
        .iter()
        .rev()
        .copied()
        .find(|c| !(crate::orthography::is_combining_mark(*c)))
        .map(|c| c.to_lowercase().next().unwrap_or(c))
}

fn is_after_consonant(prev: Option<char>) -> bool {
    let Some(prev) = prev else { return false };
    if !prev.is_alphabetic() || prev == 'ь' || prev == 'ъ' {
        return false;
    }
    !matches!(
        prev,
        'а' | 'е' | 'ё' | 'и' | 'о' | 'у' | 'ы' | 'э' | 'ю' | 'я'
    )
}

fn push_capitalized(out: &mut String, repl: &str) {
    let mut chars = repl.chars();
    if let Some(first) = chars.next() {
        for up in first.to_uppercase() {
            out.push(up);
        }
    }
    out.extend(chars);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fw(lang: &str, pos: &str, w: &str) -> String {
        flavorize_word(lang, pos, w)
    }

    /// The spec §7 golden table, verbatim — the seed acceptance test.
    #[test]
    fn spec_golden_table() {
        let cases: &[(&str, &str, &str, &str)] = &[
            ("pl", "noun", "winyl", "vinyl"),
            ("pl", "noun", "rzeka", "rěka"),
            ("pl", "noun", "radość", "radost"),
            ("pl", "verb", "być", "byti"),
            ("cs", "verb", "dělat", "dělati"),
            ("cs", "noun", "kůň", "konj"),
            ("cs", "noun", "mouka", "muka"),
            ("sk", "noun", "mäso", "meso"),
            ("sl", "verb", "delati", "delati"),
            ("sh", "noun", "rijeka", "rěka"),
            ("sh", "noun", "noć", "noč"),
            ("sr", "noun", "међа", "medža"),
            ("mk", "noun", "меѓу", "medžu"),
            ("mk", "noun", "ноќ", "noč"),
            ("bg", "noun", "дъжд", "dožd"),
            ("ru", "noun", "пластинка", "plastinka"),
            ("ru", "noun", "дело", "dělo"),
            ("ru", "noun", "самолёт", "samolet"),
            ("ru", "verb", "читать", "čitati"),
            ("ru", "noun", "конь", "konj"),
            ("uk", "noun", "голова", "holova"),
            ("be", "noun", "цень", "těnj"),
            ("hsb", "noun", "dźěło", "dělo"),
            ("cu", "noun", "дьнь", "den"),
        ];
        for (lang, pos, word, want) in cases {
            assert_eq!(&fw(lang, pos, word), want, "{lang} {word}");
        }
    }

    #[test]
    fn class_b_rules_per_language() {
        // ru: soft-e principle, ё<*e, lj/nj finals, uniform ja/ju, adj ending.
        assert_eq!(fw("ru", "noun", "река"), "rěka");
        assert_eq!(fw("ru", "noun", "ель"), "jelj");
        assert_eq!(fw("ru", "noun", "объезд"), "objezd");
        assert_eq!(fw("ru", "noun", "земля"), "zěmlja"); // ě-overmark accepted (spec §3)
        assert_eq!(fw("ru", "noun", "семья"), "sěmja");
        assert_eq!(fw("ru", "adj", "русский"), "russky");
        assert_eq!(fw("ru", "noun", "ёж"), "jež");
        // uk: h, y, apostrophe, adj ending, є.
        assert_eq!(fw("uk", "noun", "риба"), "ryba");
        assert_eq!(fw("uk", "noun", "м'ясо"), "mjaso");
        assert_eq!(fw("uk", "adj", "добрий"), "dobry");
        // be: dzekanne/cekanne, ў, infinitive.
        assert_eq!(fw("be", "noun", "дзень"), "děnj");
        assert_eq!(fw("be", "verb", "чытаць"), "čytati");
        assert_eq!(fw("be", "noun", "воўк"), "vovk");
        // pl: nasals, soft clusters, rz, ń.
        assert_eq!(fw("pl", "noun", "dąb"), "dub");
        assert_eq!(fw("pl", "noun", "imię"), "ime");
        assert_eq!(fw("pl", "num", "pięć"), "pet");
        assert_eq!(fw("pl", "noun", "koń"), "konj");
        assert_eq!(fw("pl", "noun", "niebo"), "něbo");
        assert_eq!(fw("pl", "noun", "wiek"), "věk");
        assert_eq!(fw("pl", "noun", "nici"), "niti");
        assert_eq!(fw("pl", "adv", "dobrze"), "dobrě");
        assert_eq!(fw("pl", "adv", "przy"), "pri");
        assert_eq!(fw("pl", "adj", "czysty"), "čisty");
        assert_eq!(fw("pl", "noun", "żyto"), "žito");
        assert_eq!(fw("pl", "noun", "góra"), "gora");
        // cs/sk.
        assert_eq!(fw("cs", "noun", "řeka"), "rěka");
        assert_eq!(fw("cs", "noun", "soud"), "sud");
        assert_eq!(fw("cs", "adj", "nový"), "novy");
        assert_eq!(fw("cs", "noun", "loď"), "lod");
        assert_eq!(fw("sk", "verb", "robiť"), "robiti");
        assert_eq!(fw("sk", "noun", "viera"), "věra");
        assert_eq!(fw("sk", "noun", "kôň"), "konj");
        assert_eq!(fw("sk", "noun", "ľud"), "ljud");
        // sh: jat recovery gated to post-consonant je; adj ending.
        assert_eq!(fw("sh", "noun", "mjesto"), "město");
        assert_eq!(fw("sh", "noun", "jezik"), "jezik");
        assert_eq!(fw("sh", "adj", "novi"), "novy");
        assert_eq!(fw("sh", "noun", "vođa"), "vodža");
        // bg ьо; mk specials.
        assert_eq!(fw("bg", "adj", "синьо"), "sinjo");
        assert_eq!(fw("mk", "noun", "ѕвезда"), "dzvezda");
        // cu Havlík.
        assert_eq!(fw("cu", "noun", "сънъ"), "son");
        assert_eq!(fw("cu", "verb", "оучити"), "učiti");
    }

    #[test]
    fn case_and_loan_letters() {
        assert_eq!(fw("pl", "noun", "Warszawa"), "Varšava");
        assert_eq!(fw("ru", "noun", "Юрий"), "Jurij");
        assert_eq!(fw("ru", "noun", "Россия"), "Rossija");
        assert_eq!(fw("sl", "noun", "taxi"), "taksi");
        assert_eq!(fw("sh", "noun", "quiz"), "kviz");
        // Unknown language codes get the post-pass only.
        assert_eq!(fw("xx", "noun", "wagon"), "vagon");
    }

    #[test]
    fn output_is_isv_closed_on_goldens() {
        for w in [
            fw("pl", "noun", "źdźbło"),
            fw("ru", "noun", "съезд"),
            fw("cs", "noun", "čtvrtek"),
            fw("bg", "noun", "въздух"),
            fw("mk", "noun", "раѓање"),
            fw("cu", "noun", "мѫжь"),
        ] {
            assert!(residue_chars(&w).next().is_none(), "residue in {w:?}");
        }
    }

    /// Every rendered raw headword must land in the ISV alphabet; a tiny
    /// residue tail (foreign scripts inside loanword spellings) is tolerated
    /// but capped, and export/coverage report it loudly. Skips gracefully on a
    /// checkout without the raw cache (same posture as export).
    #[test]
    fn raw_corpus_headwords_are_isv_closed() {
        let path = std::path::Path::new(crate::DEFAULT_RAW_LEMMA_CACHE);
        if !path.exists() {
            eprintln!("skip: {} absent", path.display());
            return;
        }
        let corpus = crate::dump::RawSlavicCorpus::load(path).expect("raw cache loads");
        let (mut with_residue, mut total) = (0usize, 0usize);
        let mut top: std::collections::BTreeMap<char, usize> = std::collections::BTreeMap::new();
        for l in &corpus.lemmas {
            let d = flavorize_word(&l.lang, &l.pos, &l.word);
            total += 1;
            let mut any = false;
            for c in residue_chars(&d) {
                *top.entry(c).or_default() += 1;
                any = true;
            }
            if any {
                with_residue += 1;
            }
        }
        let rate = with_residue as f64 / total.max(1) as f64;
        eprintln!(
            "flavorize closure: {with_residue}/{total} with residue ({rate:.4}); top: {top:?}"
        );
        assert!(
            rate < 0.005,
            "flavorize residue rate {rate:.4} over 0.5% — new unmapped letters? top: {top:?}"
        );
    }

    // ---- text mode: the retired russian_translit tests, verbatim ----

    #[test]
    fn transliterates_russian_examples() {
        let tr = |s: &str| translit_text("ru", s);
        assert_eq!(tr("вода́"), "voda");
        assert_eq!(tr("русский язык"), "russkij jazyk");
        assert_eq!(tr("семья, объект"), "semja, objekt");
        assert_eq!(tr("ёлка и щука"), "jolka i ščuka");
    }

    #[test]
    fn preserves_basic_capitalization() {
        let tr = |s: &str| translit_text("ru", s);
        assert_eq!(tr("Россия"), "Rossija");
        assert_eq!(tr("Юрий"), "Jurij");
    }

    #[test]
    fn text_mode_passes_other_languages_verbatim() {
        assert_eq!(translit_text("pl", "rzeka płynie"), "rzeka płynie");
        assert_eq!(translit_text("uk", "голова"), "голова");
    }
}
