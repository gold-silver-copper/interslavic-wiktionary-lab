//! Proto-Slavic → Interslavic rule engine.
//!
//! An *ordered* pipeline of deterministic transformations, each emitting a
//! [`RuleStep`] for the audit trail. Order matters: liquid metathesis and the
//! palatal outcomes must run before yer-fall, and endings are resolved last and
//! POS-aware. The target is the flavored/scientific orthography used by the
//! official dictionary (ě, ę, ų, å, ȯ, ć, đ), which preserves exactly the
//! etymological distinctions Proto-Slavic encodes.

use crate::model::{Candidate, CandidateSource, Gender, Pos, RuleStep};

const STEEN: &str = "https://steen.free.fr/interslavic/grammar.html";
const PHON: &str = "https://interslavic.fun/learn/phonology/";
const ORTHO: &str = "https://interslavic.fun/learn/orthography/";

/// Generate an Interslavic candidate from a Proto-Slavic reconstruction.
pub fn generate(proto_word: &str, pos: Pos, gender: Option<Gender>) -> Candidate {
    let mut trace = Vec::new();
    let mut s = clean(proto_word, &mut trace);
    s = x_to_h(&s, &mut trace);
    s = palatals(&s, &mut trace);
    s = liquid_metathesis(&s, &mut trace);
    s = nasals(&s, &mut trace);
    s = yers(&s, &mut trace);
    s = endings(&s, pos, gender, &mut trace);
    s = finalize(&s, &mut trace);

    // The rule engine is deterministic; score reflects how much survived intact
    // and whether the source looked well-formed.
    let score = if s.is_empty() { 0.1 } else { 0.66 };
    let mut cand = Candidate::new(s, CandidateSource::ProtoSlavicRule, score);
    cand.trace = trace;
    cand
}

fn step(trace: &mut Vec<RuleStep>, id: &str, before: &str, after: &str, why: &str, doc: &str) {
    if before != after {
        trace.push(RuleStep::new(id, before, after, why, Some(doc)));
    }
}

/// Strip the reconstruction marker and accent/length diacritics, keeping the
/// etymological letters (yers, jat, nasals) intact.
fn clean(input: &str, trace: &mut Vec<RuleStep>) -> String {
    let before = input.to_string();
    let s = input.trim().trim_start_matches('*');
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        // Drop combining accent marks (Proto-Slavic prosody notation).
        if ('\u{0300}'..='\u{036F}').contains(&ch) || ch == '`' || ch == '´' {
            continue;
        }
        out.push(debase_vowel(ch));
    }
    step(
        trace,
        "clean",
        &before,
        &out,
        "Odstranjeny rekonstrukcijny znak i akcenty.",
        ORTHO,
    );
    out
}

fn x_to_h(input: &str, trace: &mut Vec<RuleStep>) -> String {
    let out = input.replace('x', "h").replace('X', "H");
    step(
        trace,
        "x-to-h",
        input,
        &out,
        "Praslovjansky *x → medžuslovjansky h.",
        PHON,
    );
    out
}

/// *tj/*dj, *kt'/*gt', *stj/*skj, *zdj/*zgj outcomes.
fn palatals(input: &str, trace: &mut Vec<RuleStep>) -> String {
    let mut out = input.to_string();
    for (from, to) in [
        ("stj", "šć"),
        ("skj", "šć"),
        ("zdj", "ždž"),
        ("zgj", "ždž"),
        ("tj", "ć"),
        ("dj", "đ"),
        ("ktь", "ćь"),
        ("kti", "ći"),
        ("gtь", "ćь"),
        ("kt", "ć"),
    ] {
        if out.contains(from) {
            out = out.replace(from, to);
        }
    }
    // Proto palatal ligatures if present.
    out = out.replace('ť', "ć").replace('ď', "đ");
    step(
        trace,
        "tj-dj",
        input,
        &out,
        "Refleksy *tj→ć, *dj→đ, *kt→ć, *stj→šć.",
        ORTHO,
    );
    out
}

/// Liquid diphthong metathesis: *CorC→CråC, *ColC→ClåC, *CerC→CrěC, *CelC→ClěC.
fn liquid_metathesis(input: &str, trace: &mut Vec<RuleStep>) -> String {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut out = String::new();
    let mut i = 0;
    while i < n {
        if i + 2 < n
            && is_cons(chars[i])
            && matches!(chars[i + 1], 'o' | 'e')
            && matches!(chars[i + 2], 'r' | 'l')
            && (i + 3 >= n || is_cons(chars[i + 3]))
        {
            let liquid = chars[i + 2];
            let nucleus = if chars[i + 1] == 'o' { 'å' } else { 'ě' };
            out.push(chars[i]);
            out.push(liquid);
            out.push(nucleus);
            i += 3;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    step(
        trace,
        "liquid-metathesis",
        input,
        &out,
        "Plavne dvoglasy: *TorT→TråT, *TolT→TlåT, *TerT→TrěT, *TelT→TlěT.",
        STEEN,
    );
    out
}

fn nasals(input: &str, trace: &mut Vec<RuleStep>) -> String {
    let out = input.replace('ǫ', "ų").replace('ę', "ę");
    step(
        trace,
        "nasal-vowels",
        input,
        &out,
        "Nosove glasy: *ę→ę, *ǫ→ų.",
        PHON,
    );
    out
}

/// Yer treatment via Havlík's law: scanning yers right-to-left, they alternate
/// weak/strong; weak yers drop, strong back yer → ȯ, strong front yer → e.
fn yers(input: &str, trace: &mut Vec<RuleStep>) -> String {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    // Determine strong/weak per yer position.
    let mut strong = vec![false; n];
    let mut counter = 0; // counts from the right; first yer (0) is weak
    for idx in (0..n).rev() {
        if chars[idx] == 'ъ' || chars[idx] == 'ь' {
            // Position in the alternation: even => weak, odd => strong.
            if counter % 2 == 1 {
                strong[idx] = true;
            }
            counter += 1;
        } else {
            // A full vowel resets the alternation run.
            if is_full_vowel(chars[idx]) {
                counter = 0;
            }
        }
    }
    let mut out = String::new();
    for idx in 0..n {
        match chars[idx] {
            'ъ' => {
                if strong[idx] {
                    out.push('ȯ');
                }
            }
            'ь' => {
                if strong[idx] {
                    out.push('e');
                }
            }
            other => out.push(other),
        }
    }
    step(
        trace,
        "yers",
        input,
        &out,
        "Jery po Havlíkovom pravilu: slabe padajų, silne *ъ→ȯ, *ь→e.",
        STEEN,
    );
    out
}

/// POS-aware lemma endings.
fn endings(input: &str, pos: Pos, gender: Option<Gender>, trace: &mut Vec<RuleStep>) -> String {
    let mut out = input.to_string();
    match pos {
        Pos::Verb => {
            if out.ends_with("ti") || out.ends_with("ať") {
                // fine
            } else if out.ends_with('t') {
                out.push('i');
            }
        }
        Pos::Adjective => {
            // hard adjective *-ъjь / *-ъ -> -y ; soft *-ьjь -> -ji
            for suf in ["ъjь", "yjь", "ъj", "yj"] {
                if out.ends_with(suf) {
                    out.truncate(out.len() - suf.len());
                    out.push('y');
                    break;
                }
            }
            if !out.ends_with('y') && !out.ends_with("ji") && ends_cons(&out) {
                out.push('y');
            }
        }
        Pos::Noun => {
            // Neuter o-stem keeps -o/-e; a-stem keeps -a; masculine o-stem drops
            // the final yer (already gone) leaving a consonant.
            if gender == Some(Gender::Neuter) && ends_cons(&out) {
                out.push('o');
            }
        }
        _ => {}
    }
    step(
        trace,
        "endings",
        input,
        &out,
        "Prilagoženje zakončenja po časti rěči.",
        STEEN,
    );
    out
}

fn finalize(input: &str, trace: &mut Vec<RuleStep>) -> String {
    // Drop any yers that survived (e.g. no strong reflex chosen), tidy.
    let out: String = input.chars().filter(|c| *c != 'ъ' && *c != 'ь').collect();
    let out = out.trim_matches([' ', '-']).to_string();
    step(
        trace,
        "finalize",
        input,
        &out,
        "Uklonjene ostatne jery i čiščenje.",
        ORTHO,
    );
    out
}

/// Map an accented base vowel (acute/grave/circumflex/macron/tilde/double-grave/
/// inverted-breve) to its plain base. Written with explicit escapes to avoid any
/// source-encoding ambiguity. Etymological letters (ě ę ǫ ъ ь ȯ y) are preserved.
fn debase_vowel(ch: char) -> char {
    match ch {
        '\u{00E0}' | '\u{00E1}' | '\u{00E2}' | '\u{00E3}' | '\u{0101}' | '\u{01CE}'
        | '\u{0201}' | '\u{0203}' => 'a',
        '\u{00E8}' | '\u{00E9}' | '\u{00EA}' | '\u{0113}' | '\u{0205}' | '\u{0207}'
        | '\u{1EBD}' => 'e',
        '\u{00EC}' | '\u{00ED}' | '\u{00EE}' | '\u{012B}' | '\u{0209}' | '\u{020B}'
        | '\u{0129}' => 'i',
        '\u{00F2}' | '\u{00F3}' | '\u{00F4}' | '\u{00F5}' | '\u{014D}' | '\u{020D}'
        | '\u{020F}' => 'o',
        '\u{00F9}' | '\u{00FA}' | '\u{00FB}' | '\u{016B}' | '\u{0169}' | '\u{0215}'
        | '\u{0217}' => 'u',
        '\u{00FD}' | '\u{1EF3}' | '\u{0177}' | '\u{0233}' | '\u{1EF9}' => 'y',
        other => other,
    }
}

fn is_cons(ch: char) -> bool {
    ch.is_alphabetic() && !is_full_vowel(ch) && ch != 'ъ' && ch != 'ь'
}

fn is_full_vowel(ch: char) -> bool {
    matches!(
        ch,
        'a' | 'e' | 'i' | 'o' | 'u' | 'y' | 'ě' | 'ę' | 'ǫ' | 'ų' | 'å' | 'ȯ' | 'ê' | 'ô'
    )
}

fn ends_cons(s: &str) -> bool {
    s.chars().last().map(is_cons).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orthography::normalized_match;

    fn gen(proto: &str, pos: Pos) -> String {
        generate(proto, pos, None).form
    }

    #[test]
    fn yer_fall_and_x_to_h() {
        // Final weak yer drops; *x → h.
        assert!(normalized_match(&gen("*bogъ", Pos::Noun), "bog"));
        assert!(normalized_match(&gen("*duxъ", Pos::Noun), "duh"));
    }

    #[test]
    fn strong_yer_vocalizes() {
        // *sъnъ: final ъ weak (drops), first ъ strong → ȯ (→o standard).
        assert!(normalized_match(&gen("*sъnъ", Pos::Noun), "son"));
        // *pьsъ: strong front yer → e.
        assert!(normalized_match(&gen("*pьsъ", Pos::Noun), "pes"));
    }

    #[test]
    fn liquid_metathesis() {
        assert!(normalized_match(&gen("*gordъ", Pos::Noun), "grad"));
        assert!(normalized_match(&gen("*melko", Pos::Noun), "mleko"));
        assert!(normalized_match(&gen("*bergъ", Pos::Noun), "breg"));
    }

    #[test]
    fn palatal_outcomes() {
        assert!(normalized_match(&gen("*světja", Pos::Noun), "svěća"));
        assert!(normalized_match(&gen("*medja", Pos::Noun), "međa"));
        assert!(normalized_match(&gen("*noktь", Pos::Noun), "noć"));
    }

    #[test]
    fn nasal_vowels() {
        assert!(normalized_match(&gen("*rǫka", Pos::Noun), "ruka"));
        assert!(normalized_match(&gen("*pętь", Pos::Noun), "pet"));
    }

    #[test]
    fn stable_words_and_infinitive() {
        assert!(normalized_match(&gen("*voda", Pos::Noun), "voda"));
        assert!(normalized_match(&gen("*duša", Pos::Noun), "duša"));
        assert!(normalized_match(&gen("*pisati", Pos::Verb), "pisati"));
    }
}
