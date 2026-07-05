//! Interslavic lemma-ending normalization.
//!
//! Two rule families, both from the authoritative rule spec (`data/RULE_SPEC.md`):
//!  * §5.2 internationalism ending adaptations (`-ism→-izm`, `-tion→-cija`,
//!    `-ic/-ical→-ičny`, `-al→-alny`, verbs→`-ovati`, …). International and
//!    Graeco-Latin vocabulary is a large, self-contained slice where the modern
//!    Slavic forms agree and only the Interslavic ending convention differs.
//!  * §3 POS lemma endings (noun nom.sg, adjective `-y`/`-i`, verb infinitive
//!    `-ti`), plus the abstract-noun suffix `-osť`.
//!
//! Output is the flavored/etymological spelling (so `-osť`, `-aľnosť`); the
//! standard reduction (§1.3) folds it for the normalized metric.

use crate::model::{Pos, RuleStep};

const DERIV: &str = "https://interslavic.fun/learn/vocabulary/derivation/";
const STEEN: &str = "https://steen.free.fr/interslavic/grammar.html";

/// Normalize the lemma ending. `intl` gates the internationalism table; `endings`
/// gates the native POS endings; `prefixes` gates verbal/nominal prefix
/// normalization.
pub fn normalize_lemma(
    word: &str,
    pos: Pos,
    intl: bool,
    endings: bool,
    prefixes: bool,
) -> (String, Vec<RuleStep>) {
    let mut trace = Vec::new();
    let mut w = word.to_string();

    if prefixes {
        if let Some((next, id, why)) = normalize_prefix(&w) {
            if next != w {
                trace.push(RuleStep::new(id, &w, &next, why, Some(STEEN)));
                w = next;
            }
        }
    }

    if intl {
        // §5.1: the Latin/Greek diphthongs au/eu are adapted to av/ev. Native
        // Slavic vocabulary has no /au/ or /eu/, so this only touches loanwords.
        let loan = w.replace("eu", "ev").replace("au", "av").replace("th", "t");
        if loan != w {
            trace.push(RuleStep::new(
                "intl-diphthong",
                &w,
                &loan,
                "Grečsko-latinske au→av, eu→ev, th→t.",
                Some(DERIV),
            ));
            w = loan;
        }
        if let Some((next, id, why)) = international_ending(&w, pos) {
            if next != w {
                trace.push(RuleStep::new(id, &w, &next, why, Some(DERIV)));
                w = next;
            }
        }
    }

    if endings {
        if let Some((next, id, why)) = pos_ending(&w, pos) {
            if next != w {
                trace.push(RuleStep::new(id, &w, &next, why, Some(STEEN)));
                w = next;
            }
        }
    }

    (w, trace)
}

/// Replace a matched suffix. Returns the new string.
fn swap(word: &str, suffix: &str, rep: &str) -> String {
    let stem = &word[..word.len() - suffix.len()];
    format!("{stem}{rep}")
}

/// §5.2 internationalism ending adaptations, matched on the Slavicized surface.
/// Longest / most specific suffixes first.
fn international_ending(word: &str, pos: Pos) -> Option<(String, &'static str, &'static str)> {
    // Adjectival internationalisms (must run before generic -ny handling).
    if pos == Pos::Adjective {
        for suf in [
            "ický", "icki", "ički", "ičky", "ičeskij", " yczny", "yczny", "ičen",
        ] {
            if word.ends_with(suf) {
                return Some((
                    swap(word, suf, "ičny"),
                    "intl-ic-ical",
                    "Latinsky -ic/-ical → medžuslovjansky -ičny.",
                ));
            }
        }
        for suf in ["ično", "ično", "ičny", "ičné", "ične"] {
            if word.ends_with(suf) {
                return Some((
                    swap(word, suf, "ičny"),
                    "intl-ic-ical",
                    "Prilagoženje pridavnika -ičny.",
                ));
            }
        }
        // -al(is): includes the South-Slavic predicative short forms with the
        // fleeting vowel (-alen/-alan) that must collapse, not gain a -y.
        for suf in [
            "alny", "alni", "alné", "alno", "aľny", "alnyj", "alen", "alan", "aľen", "alën",
        ] {
            if word.ends_with(suf) {
                return Some((
                    swap(word, suf, "alny"),
                    "intl-al",
                    "Latinsky -al(is) → -alny.",
                ));
            }
        }
        for suf in [
            "ativny", "ativni", "ativen", "ativan", "ativna", "ativno", "atȯvny",
        ] {
            if word.ends_with(suf) {
                return Some((
                    swap(word, suf, "ativny"),
                    "intl-ative",
                    "Latinsky -ative → -ativny.",
                ));
            }
        }
        for suf in ["ivny", "ivni", "ivnyj", "ivné", "iven", "ivan"] {
            if word.ends_with(suf) {
                return Some((
                    swap(word, suf, "ivny"),
                    "intl-ive",
                    "Latinsky -ive → -ivny.",
                ));
            }
        }
        for suf in ["ozny", "ózny", "osny", "ozni"] {
            if word.ends_with(suf) {
                return Some((
                    swap(word, suf, "ozny"),
                    "intl-ous",
                    "Latinsky -ous → -ozny.",
                ));
            }
        }
        for suf in ["ijny", "ijni", "yjny", "ijné"] {
            if word.ends_with(suf) {
                return Some((
                    swap(word, suf, "ijny"),
                    "intl-ijny",
                    "Pridavnik od internacionalizma → -ijny.",
                ));
            }
        }
    }

    // Nominal internationalisms.
    if matches!(pos, Pos::Noun) {
        for suf in ["izem", "izam", "izmus", "ismus", "izmu", "ism"] {
            if word.ends_with(suf) {
                return Some((swap(word, suf, "izm"), "intl-ism", "Anglijsky -ism → -izm."));
            }
        }
        for suf in ["ista", "isti", "istu", "iste"] {
            if word.ends_with(suf) {
                return Some((swap(word, suf, "ist"), "intl-ist", "Anglijsky -ist → -ist."));
            }
        }
        for suf in ["cija", "cije", "cja", "cyja", "ciju", "cijo", "ция"] {
            if word.ends_with(suf) {
                return Some((
                    swap(word, suf, "cija"),
                    "intl-tion",
                    "Anglijsky -tion → -cija.",
                ));
            }
        }
        for suf in ["zija", "zije", "zja", "ziju"] {
            if word.ends_with(suf) {
                return Some((
                    swap(word, suf, "zija"),
                    "intl-sion",
                    "Anglijsky -sion → -zija.",
                ));
            }
        }
        for suf in ["sija", "sije", "sju"] {
            if word.ends_with(suf) {
                return Some((
                    swap(word, suf, "sija"),
                    "intl-ssion",
                    "Anglijsky -ssion → -sija.",
                ));
            }
        }
    }

    // Verb internationalisms: Latin -ate/-ise/-ize / German -ieren → -ovati.
    if pos == Pos::Verb {
        for suf in [
            "ovati", "ować", "ovať", "ovat", " irovati", "irovať", "izovati", "izovať", "izirati",
            "izovat", "ovac",
        ] {
            if word.ends_with(suf) && !word.ends_with("ovati") {
                // Normalize any -ova(ć/ť/t/c) tail to -ovati.
                if let Some(pos_ova) = word.rfind("ova") {
                    let stem = &word[..pos_ova];
                    return Some((
                        format!("{stem}ovati"),
                        "intl-verb",
                        "Internacionalny glagol → -ovati.",
                    ));
                }
                return Some((
                    swap(word, suf, "ovati"),
                    "intl-verb",
                    "Internacionalny glagol → -ovati.",
                ));
            }
        }
    }

    None
}

/// §3 POS lemma endings, applied to the native/consensus surface.
fn pos_ending(word: &str, pos: Pos) -> Option<(String, &'static str, &'static str)> {
    match pos {
        Pos::Verb => verb_infinitive(word),
        Pos::Adjective => Some(adjective_lemma(word)),
        Pos::Adverb => adverb_lemma(word),
        Pos::Noun => noun_lemma(word),
        _ => None,
    }
}

fn verb_infinitive(word: &str) -> Option<(String, &'static str, &'static str)> {
    if word.ends_with("ti") {
        return None;
    }
    // Map the common cited infinitive tails onto Interslavic -ti / -ati / -iti.
    for (suf, rep) in [
        ("ать", "ati"),
        ("ити", "iti"),
        ("нути", "nuti"),
        ("овати", "ovati"),
        ("ować", "ovati"),
        ("ovať", "ovati"),
        ("ovat", "ovati"),
        ("ať", "ati"),
        ("at", "ati"),
        ("iť", "iti"),
        ("it", "iti"),
        ("ěť", "ěti"),
        ("ět", "ěti"),
        ("nuť", "nuti"),
        ("nut", "nuti"),
        (" nąć", "nuti"),
        ("nąć", "nuti"),
        ("ć", "ti"),
        ("сти", "sti"),
        ("ть", "ti"),
    ] {
        if word.ends_with(suf) {
            return Some((swap(word, suf, rep), "verb-inf-ti", "Infinitiv na -ti."));
        }
    }
    if word.ends_with('t') {
        return Some((format!("{word}i"), "verb-inf-ti", "Infinitiv na -ti."));
    }
    None
}

fn adjective_lemma(word: &str) -> (String, &'static str, &'static str) {
    // Already a hard/soft adjective ending.
    if word.ends_with('y') {
        return (word.to_string(), "adj-hard-y", "Tvŕdy pridavnik -y.");
    }
    // Strip a final adjectival vowel (from various languages: -i, -ý, -í, -o, -e,
    // -a) and re-attach the correct hard/soft ending.
    let stem: String = if word
        .chars()
        .last()
        .map(|c| "iíýoeaà".contains(c))
        .unwrap_or(false)
    {
        let mut s = word.to_string();
        s.pop();
        s
    } else {
        word.to_string()
    };
    let soft = stem_is_soft(&stem);
    let ending = if soft { "i" } else { "y" };
    (
        format!("{stem}{ending}"),
        if soft { "adj-soft-i" } else { "adj-hard-y" },
        "Pridavnik: tvŕdy -y / mękky -i.",
    )
}

fn adverb_lemma(word: &str) -> Option<(String, &'static str, &'static str)> {
    // Abstract/international adverbs: -alno, keep -o; convert -e→-o only after
    // hard stems is risky, so only normalize the clear -aľno case.
    if word.ends_with("alno") || word.ends_with("aľno") {
        return Some((
            swap(
                word,
                if word.ends_with("aľno") {
                    "aľno"
                } else {
                    "alno"
                },
                "alno",
            ),
            "adv-alno",
            "Prislov -alno.",
        ));
    }
    None
}

fn noun_lemma(word: &str) -> Option<(String, &'static str, &'static str)> {
    // Abstract nouns in -ost take the soft ť; -alnost → -aľnosť.
    for suf in ["alnost", "aljnost", "alnosť", "aľnost"] {
        if word.ends_with(suf) {
            return Some((
                swap(word, suf, "aľnosť"),
                "noun-alnost",
                "Odvlečeny imennik -aľnosť.",
            ));
        }
    }
    for suf in ["nost", "nast", "nosc", "ność", "ность"] {
        if word.ends_with(suf) {
            return Some((
                swap(word, suf, "nosť"),
                "noun-ost",
                "Odvlečeny imennik -osť.",
            ));
        }
    }
    None
}

/// Normalize the common verbal/nominal prefixes to their Interslavic shape.
/// Fires only word-initially and only when a plausible root follows.
fn normalize_prefix(word: &str) -> Option<(String, &'static str, &'static str)> {
    // *orz- : Russian raz-/ras-, Polish/Ukrainian roz-/ros- → ISV råz-.
    for pre in ["raz", "ras", "roz", "ros", "rȯz", "råz"] {
        if let Some(rest) = word.strip_prefix(pre) {
            if rest.chars().count() >= 3 && rest.chars().next().map(is_letter).unwrap_or(false) {
                if pre == "råz" {
                    return None;
                }
                return Some((format!("råz{rest}"), "prefix-orz", "Predpona *orz- → råz-."));
            }
        }
    }
    // *perd- : pred-/pred → prěd- (jat).
    if let Some(rest) = word.strip_prefix("pred") {
        if rest.chars().count() >= 3 {
            return Some((
                format!("prěd{rest}"),
                "prefix-perd",
                "Predpona *perd- → prěd-.",
            ));
        }
    }
    None
}

fn is_letter(c: char) -> bool {
    c.is_alphabetic()
}

/// A stem is grammatically soft when it ends in a hushing/soft consonant.
fn stem_is_soft(stem: &str) -> bool {
    let last = stem.chars().last().unwrap_or(' ');
    matches!(
        last,
        'š' | 'ž' | 'č' | 'c' | 'j' | 'ć' | 'đ' | 'ń' | 'ľ' | 'ŕ'
    ) || stem.ends_with("lj")
        || stem.ends_with("nj")
        || stem.ends_with("dž")
}
