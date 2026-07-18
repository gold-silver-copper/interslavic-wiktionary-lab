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

use crate::model::{Gender, Pos, RuleStep};

const DERIV: &str = "https://interslavic.fun/learn/vocabulary/derivation/";
const STEEN: &str = "https://steen.free.fr/interslavic/grammar.html";

/// Which lemma-normalization rule families to run (each is a benchmark-gated
/// `ConsensusConfig` flag, so the ablation ladder can attribute its effect).
#[derive(Debug, Clone, Copy, Default)]
pub struct LemmaRules {
    /// §5.2 internationalism ending table.
    pub intl: bool,
    /// §3 native POS lemma endings.
    pub endings: bool,
    /// Verbal/nominal prefix normalization (råz-, prěd-).
    pub prefixes: bool,
    /// Derivational-suffix normalization (root-consistency invariant `DERIV`):
    /// -telj- kept before suffixes, feminine i-stem -sť, -livy.
    pub deriv: bool,
    /// Graeco-Latin hiatus in loans: ISV keeps -ia- (social-, entuziazm), the
    /// Slavic cognates' glide (-ija-) is a national adaptation.
    pub loan_hiatus: bool,
}

/// Normalize the lemma ending. `intl` gates the internationalism table; `endings`
/// gates the native POS endings; `prefixes` gates verbal/nominal prefix
/// normalization.
pub fn normalize_lemma(
    word: &str,
    pos: Pos,
    gender: Option<Gender>,
    rules: LemmaRules,
) -> (String, Vec<RuleStep>) {
    let LemmaRules {
        intl,
        endings,
        prefixes,
        deriv,
        loan_hiatus,
    } = rules;
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
        // §5.1: the Latin/Greek diphthongs au/eu adapt to av/ev and th→t — but
        // ONLY inside a recognized internationalism. Blanket-replacing corrupts
        // native words whose au/eu/th spans a morpheme boundary (naučiti, neuspěh,
        // vethy) and loans that keep the digraph (sauna, pauza). Gate on the
        // Graeco-Latin shape and never touch a native prefix boundary.
        if crate::consensus::is_international_form(&w) && !starts_native_prefix_vowel(&w) {
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

    if loan_hiatus {
        if let Some((next, id, why)) = loan_hiatus_rule(&w, pos) {
            if next != w {
                trace.push(RuleStep::new(id, &w, &next, why, Some(DERIV)));
                w = next;
            }
        }
    }

    if deriv {
        // Unlike the ending stages, several derivational rules can apply to one
        // word (dějatelnost → -telj- AND -sť), so run them all in sequence.
        for (next, id, why) in derivational_suffixes(&w, pos, gender) {
            if next != w {
                trace.push(RuleStep::new(id, &w, &next, why, Some(DERIV)));
                w = next;
            }
        }
    }

    (w, trace)
}

/// Derivational-suffix normalization (the root-consistency invariant `[DERIV]`:
/// the same suffix must surface identically in every derivative). Each rule is
/// categorical in the official dictionary:
///  * `-telj-` is kept before derivational suffixes — 53 lemmas in
///    -teljstvo/-teljny/-teljsky (+6 -teljka) vs **zero** with hard -tel- there;
///    the word-final `-telj` rule alone missed the derived family.
///  * feminine i-stems end soft `-sť` — 516 feminine lemmas in -sť vs **zero**
///    in plain -st (kost́, radosť, zabolěvajemosť); masculines (most, tekst) are
///    untouched. The general noun `-ost` → `-osť` covers the abstract suffix
///    when gender is unknown, behind a closed skip list (most/post/tost/hvost).
///  * the deverbal adjective suffix is `-livy` — 152 lemmas in -liv- vs **zero**
///    in -ljiv- (South-Slavic cognates write -ljiv).
fn derivational_suffixes(
    word: &str,
    pos: Pos,
    gender: Option<Gender>,
) -> Vec<(String, &'static str, &'static str)> {
    let mut out = Vec::new();
    let mut w = word.to_string();

    // -ljiv- → -liv- (suffix positions only, so šljiva-like roots are safe).
    for suf in ["ljivy", "ljivi", "ljivo", "ljivosť", "ljivost"] {
        if w.ends_with(suf) {
            let next = format!("{}l{}", &w[..w.len() - suf.len()], &suf[2..]);
            out.push((
                next.clone(),
                "deriv-liv",
                "Sufiks -liv(y): medžuslovjansky dŕži tvŕde l (South -ljiv je narodna adaptacija).",
            ));
            w = next;
            break;
        }
    }

    // -telj- kept before derivational suffixes (učiteljstvo, bditeljny), unless
    // the root is one of the closed set of genuine hard -tel- words.
    let hard_tel = ["hotel", "kotel", "kostel", "dětel"];
    if !hard_tel.iter().any(|r| w.contains(r)) {
        for (suf, rep) in [
            ("telstvo", "teljstvo"),
            ("telnosť", "teljnosť"),
            ("telnost", "teljnosť"),
            ("telny", "teljny"),
            ("telno", "teljno"),
            ("telsky", "teljsky"),
            ("telka", "teljka"),
        ] {
            if w.ends_with(suf) && w.chars().count() > suf.chars().count() + 1 {
                let next = format!("{}{}", &w[..w.len() - suf.len()], rep);
                out.push((
                    next.clone(),
                    "deriv-telj",
                    "Sufiks dějatelja *-teljь dŕži mękke lj i prěd sufiksami (-teljstvo, -teljny).",
                ));
                w = next;
                break;
            }
        }
    }

    // Feminine i-stem: soft -sť (kosť, radosť). Categorical for feminine nouns;
    // when gender is unknown, only the abstract -osť suffix is safe (skip the
    // closed masculine set most/post/tost/hvost).
    if pos == Pos::Noun && w.ends_with("st") && w.chars().count() > 3 {
        let feminine = gender == Some(Gender::Feminine);
        let osty = w.ends_with("ost")
            && !["most", "post", "tost", "hvost", "avanpost", "kompost"].contains(&w.as_str());
        if feminine || osty {
            let next = format!("{}ť", &w[..w.len() - 1]);
            out.push((
                next,
                "deriv-ost",
                "Žensky i-kmen: mękke -sť (kosť, radosť, novosť).",
            ));
        }
    }

    out
}

/// Graeco-Latin hiatus in internationalisms: Interslavic keeps the Latin -ia-
/// hiatus (socialny, entuziazm, sociolog) where the Slavic cognates insert a
/// glide (-ija-). Categorical in the dictionary: 24 -ial- vs 0 -ijal-, 2 -iaz(m)
/// vs 0 -ijaz-, 3 -iast- vs 0 -ijast-, 139 midword -io- vs 1 -ijo- (kopijovati,
/// a verb — hence the noun/adjective gate for -ijo-).
fn loan_hiatus_rule(word: &str, pos: Pos) -> Option<(String, &'static str, &'static str)> {
    let mut w = word.to_string();
    for pat in ["ijal", "ijazm", "ijast"] {
        if w.contains(pat) {
            w = w.replace(pat, &pat.replacen('j', "", 1));
        }
    }
    // Midword -ijo- → -io- (socijolog → sociolog); nouns/adjectives only, the
    // word-final feminine -ijo is the nom.sg -ija (handled by noun-ija) and
    // verbs like kopijovati genuinely keep the glide.
    if matches!(pos, Pos::Noun | Pos::Adjective) {
        if let Some(i) = w.find("ijo") {
            if i + 3 < w.len() {
                w = format!("{}{}{}", &w[..i], "io", &w[i + 3..]);
            }
        }
    }
    if w != word {
        return Some((
            w,
            "loan-hiatus",
            "Grečsko-latinsky zěv -ia-/-io- sę dŕži v internacionalizmah (socialny, sociolog); -ija- je narodna adaptacija.",
        ));
    }
    None
}

/// Replace a matched suffix. Returns the new string.
fn swap(word: &str, suffix: &str, rep: &str) -> String {
    let stem = &word[..word.len() - suffix.len()];
    format!("{stem}{rep}")
}

/// True when a word begins with a native prefix directly followed by a vowel that
/// would form a spurious au/eu across the boundary (na+u→"au", ne+u→"eu"), so the
/// internationalism diphthong rule must not fire: naučiti, neuspěh, zaučiti.
fn starts_native_prefix_vowel(w: &str) -> bool {
    for p in [
        "na", "ne", "za", "po", "pre", "prě", "do", "vy", "raz", "roz", "iz",
    ] {
        if let Some(rest) = w.strip_prefix(p) {
            if rest.starts_with(['a', 'e', 'u', 'i', 'o']) {
                return true;
            }
        }
    }
    false
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
        // -ózny/-ozny (Latin -ous). NOT bare -osny: that voicing wrongly hits
        // native adjectives (snosny, nosny, opasny) — leave -osny alone (B18).
        for suf in ["ozny", "ózny", "ozni"] {
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
        // Slovene/Serbo-Croatian -enca/-anca for Latin -entia/-antia.
        for (suf, rep) in [("enca", "encija"), ("anca", "ancija")] {
            if word.ends_with(suf) {
                return Some((
                    swap(word, suf, rep),
                    "intl-tion",
                    "Latinsky -entia/-antia → -encija/-ancija.",
                ));
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
        // A hushing/soft stem before the final -y takes the soft ending -i, not
        // hard -y (staršy→starši, božy→boži): the dictionary has 60 -ši / 72 -ji /
        // 7 -či lemmas and zero in -šy/-žy/-čy/-jy. `c` is excluded (proper noun
        // Jangcy), matching stem_is_soft's own hard-`c` treatment.
        if matches!(word.chars().rev().nth(1), Some('š' | 'ž' | 'č' | 'j')) {
            let mut s = word.to_string();
            s.pop();
            s.push('i');
            return (s, "adj-soft-i", "Mękky pridavnik -i po šumnom sųglasniku.");
        }
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
    // Agentive/instrumental *-teljь nouns: East/Bulgarian cite the bare -tel
    // (ru -тель, bg -тел), but Interslavic keeps the soft l as -telj (učitelj,
    // izbiratelj, proizvoditelj). The dictionary has 122 lemmas in -telj against
    // only four native nouns that genuinely end in a hard -tel (hotel, kotel,
    // kostel, dětel), so the rule is near-lossless behind that closed skip list.
    if word.ends_with("tel")
        && word.chars().count() > 4
        && !["hotel", "kotel", "kostel", "dětel"]
            .iter()
            .any(|x| word.ends_with(x))
    {
        return Some((
            format!("{word}j"),
            "noun-telj",
            "Sufiks dějatelja *-teljь: mękke -telj (učitelj), ne tvŕde -tel.",
        ));
    }
    // Feminine ja-stem citation: Slovene/Czech oblique or vowel-shifted
    // representatives end -ijo/-ije/-iji where Interslavic cites the nominative
    // -ija (fizioterapijo→fizioterapija, podkategorije→podkategorija); likewise
    // -ike→-ika. The dictionary has 668 -ija / 90 -ika lemmas and effectively no
    // singular lemma in -ij[oei]/-ike, so this is near-lossless.
    for suf in ["ijo", "ije", "iji"] {
        if word.ends_with(suf) && word.chars().count() > 4 {
            return Some((
                swap(word, suf, "ija"),
                "noun-ija",
                "Žensky imennik: nom.sg -ija (ne -ijo/-ije/-iji).",
            ));
        }
    }
    if word.ends_with("ike") && word.chars().count() > 4 {
        return Some((
            swap(word, "ike", "ika"),
            "noun-ika",
            "Žensky imennik: nom.sg -ika (ne -ike).",
        ));
    }
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
    for suf in ["nost", "nast", "nosc", "ность"] {
        if word.ends_with(suf) {
            return Some((
                swap(word, suf, "nosť"),
                "noun-ost",
                "Odvlečeny imennik -osť.",
            ));
        }
    }
    // Deverbal (verbal) nouns: -nie → -nje (dviženie → dviženje, znanie → znanje).
    if word.ends_with("nie") && word.chars().count() > 4 {
        return Some((
            swap(word, "nie", "nje"),
            "noun-verbal",
            "Odglagolny imennik -ńje/-nje.",
        ));
    }
    None
}

/// Normalize the common verbal/nominal prefixes to their Interslavic shape.
/// Fires only word-initially and only when a plausible root follows.
fn normalize_prefix(word: &str) -> Option<(String, &'static str, &'static str)> {
    // *orz- : Russian raz-/ras-, Polish/Ukrainian roz-/ros- → ISV råz-. Require a
    // CONSONANT-initial stem after the prefix: a real prefixed verb is raz+CV…
    // (rasprostirati), whereas a root that merely begins with ros/ras has a vowel
    // there (rositi, rosa) — stripping it would wrongly yield råziti (B17).
    for pre in ["raz", "ras", "roz", "ros", "rȯz", "råz"] {
        if let Some(rest) = word.strip_prefix(pre) {
            if rest.chars().count() >= 3 && rest.chars().next().map(is_consonant).unwrap_or(false) {
                if pre == "råz" {
                    return None;
                }
                return Some((format!("råz{rest}"), "prefix-orz", "Predpona *orz- → råz-."));
            }
        }
    }
    // *perd- : pred-/pred → prěd- (jat). Same consonant-initial-stem guard, so
    // predator/predikat (Latin roots) are not mis-analyzed as prěd-.
    if let Some(rest) = word.strip_prefix("pred") {
        if rest.chars().count() >= 3 && rest.chars().next().map(is_consonant).unwrap_or(false) {
            return Some((
                format!("prěd{rest}"),
                "prefix-perd",
                "Predpona *perd- → prěd-.",
            ));
        }
    }
    None
}

/// A consonant (not a vowel or semivowel start of a fresh syllable).
fn is_consonant(c: char) -> bool {
    is_letter(c) && !"aeiouyěęųǫåȯėAEIOUY".contains(c)
}

fn is_letter(c: char) -> bool {
    c.is_alphabetic()
}

/// A stem is grammatically soft when it ends in a hushing/soft consonant.
/// The crate's single definition of softness (derive.rs reuses it).
pub(crate) fn stem_is_soft(stem: &str) -> bool {
    interslavic::phono::is_soft(stem)
}

#[cfg(test)]
mod tests {
    #[test]
    fn prefix_normalized_only_before_a_consonant_stem() {
        // Real prefixed verbs normalize; roots that merely start ros-/pred- don't (B17).
        assert!(super::normalize_prefix("rasprostirati")
            .unwrap()
            .0
            .starts_with("råz"));
        assert!(super::normalize_prefix("predstaviti")
            .unwrap()
            .0
            .starts_with("prěd"));
        assert!(super::normalize_prefix("rositi").is_none());
        assert!(super::normalize_prefix("predator").is_none());
        assert!(super::normalize_prefix("rosa").is_none());
    }

    fn rules(intl: bool, endings: bool) -> super::LemmaRules {
        super::LemmaRules {
            intl,
            endings,
            ..Default::default()
        }
    }

    #[test]
    fn latin_entia_antia_endings_normalize() {
        // Slovene/SC -enca/-anca (Latin -entia/-antia) → -encija/-ancija.
        let (w, _) =
            super::normalize_lemma("licenca", crate::model::Pos::Noun, None, rules(true, true));
        assert_eq!(w, "licencija");
        let (w, _) =
            super::normalize_lemma("aroganca", crate::model::Pos::Noun, None, rules(true, true));
        assert_eq!(w, "arogancija");
    }

    #[test]
    fn agent_noun_and_feminine_nominative_endings() {
        use crate::model::Pos;
        let n = |w: &str| super::normalize_lemma(w, Pos::Noun, None, rules(true, true)).0;
        // Agentive *-teljь: bare -tel gains the soft l.
        assert_eq!(n("izbiratel"), "izbiratelj");
        assert_eq!(n("proizvoditel"), "proizvoditelj");
        // The closed set of native hard -tel nouns is protected.
        assert_eq!(n("hotel"), "hotel");
        assert_eq!(n("kostel"), "kostel");
        // Feminine ja-stem: oblique/vowel-shifted reps fold to nom.sg -ija/-ika.
        assert_eq!(n("fizioterapijo"), "fizioterapija");
        assert_eq!(n("podkategorije"), "podkategorija");
        assert_eq!(n("gimnastike"), "gimnastika");
    }

    #[test]
    fn soft_stem_adjective_takes_i() {
        use crate::model::Pos;
        let a = |w: &str| super::normalize_lemma(w, Pos::Adjective, None, rules(false, true)).0;
        // A hushing stem before -y takes the soft ending -i.
        assert_eq!(a("staršy"), "starši");
        assert_eq!(a("božy"), "boži");
        // A hard stem keeps -y.
        assert_eq!(a("dobry"), "dobry");
    }

    #[test]
    fn derivational_suffixes_normalize() {
        use crate::model::{Gender, Pos};
        let deriv = super::LemmaRules {
            endings: true,
            deriv: true,
            ..Default::default()
        };
        // -telj- kept before derivational suffixes.
        let n = |w: &str, p: Pos, g: Option<Gender>| super::normalize_lemma(w, p, g, deriv).0;
        assert_eq!(n("izdatelstvo", Pos::Noun, None), "izdateljstvo");
        assert_eq!(n("bditelny", Pos::Adjective, None), "bditeljny");
        assert_eq!(n("neprijatelsky", Pos::Adjective, None), "neprijateljsky");
        assert_eq!(n("izključitelno", Pos::Adverb, None), "izključiteljno");
        // ... via the endings-stage -nost first: dějatelnost → dějateljnosť.
        assert_eq!(n("dějatelnost", Pos::Noun, None), "dějateljnosť");
        // The closed hard -tel- set is protected.
        assert_eq!(n("kotelny", Pos::Adjective, None), "kotelny");
        // Feminine i-stem takes the soft -sť; masculines don't.
        assert_eq!(
            n("kost", Pos::Noun, Some(Gender::Feminine)),
            "kosť".to_string()
        );
        assert_eq!(n("zabolevajemost", Pos::Noun, None), "zabolevajemosť");
        assert_eq!(n("most", Pos::Noun, None), "most");
        assert_eq!(n("kompost", Pos::Noun, None), "kompost");
        // Deverbal -livy, suffix position only.
        assert_eq!(n("razdražljivy", Pos::Adjective, None), "razdražlivy");
        assert_eq!(n("šljiva", Pos::Noun, Some(Gender::Feminine)), "šljiva");
    }

    #[test]
    fn loan_hiatus_kept_in_internationalisms() {
        use crate::model::Pos;
        let hia = super::LemmaRules {
            loan_hiatus: true,
            ..Default::default()
        };
        let n = |w: &str, p: Pos| super::normalize_lemma(w, p, None, hia).0;
        assert_eq!(n("socijalny", Pos::Adjective), "socialny");
        assert_eq!(n("nacionalsocijalizm", Pos::Noun), "nacionalsocializm");
        assert_eq!(n("entuzijazm", Pos::Noun), "entuziazm");
        assert_eq!(n("entuzijastičny", Pos::Adjective), "entuziastičny");
        assert_eq!(n("socijologija", Pos::Noun), "sociologija");
        // Word-final -ijo (feminine nom.sg citation) is not the loan hiatus.
        assert_eq!(n("fizioterapijo", Pos::Noun), "fizioterapijo");
        // Verbs keep the glide (kopijovati is official).
        assert_eq!(n("kopijovati", Pos::Verb), "kopijovati");
    }

    #[test]
    fn native_prefix_vowel_boundary_detected() {
        // au/eu across a native prefix boundary must be protected (B1).
        assert!(super::starts_native_prefix_vowel("naučiti"));
        assert!(super::starts_native_prefix_vowel("neuspěh"));
        assert!(super::starts_native_prefix_vowel("zaučiti"));
        assert!(!super::starts_native_prefix_vowel("avtomobil"));
        assert!(!super::starts_native_prefix_vowel("telefon"));
    }
}
