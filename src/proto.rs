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
    generate_with_reflexes(proto_word, pos, gender, &[], None)
}

/// As [`generate`], but with the modern-cognate reflexes (phonemic Latin) as
/// evidence for resolving lexically-ambiguous weak yers (§4.4 at the segment
/// level): a weak yer that most reflexes vocalize is retained, not dropped, so
/// e.g. *pьsati → pisati (which strict Havlík would render *psati, matching only
/// Czech). Pass `&[]` for the pure rule-only derivation.
///
/// `stem_class` is the linked entry's Wiktionary declension category
/// ([`crate::dump::ProtoEntry::stem_class`]), used only for the stem-class-aware
/// citation endings (issue #76). Pass `None` to keep the archaic nominative.
pub fn generate_with_reflexes(
    proto_word: &str,
    pos: Pos,
    gender: Option<Gender>,
    reflexes: &[String],
    stem_class: Option<&str>,
) -> Candidate {
    let mut trace = Vec::new();
    let mut s = clean(proto_word, &mut trace);
    s = x_to_h(&s, &mut trace);
    s = palatals(&s, pos, &mut trace);
    s = liquid_metathesis(&s, &mut trace);
    s = nasals(&s, &mut trace);
    s = prothesis(&s, &mut trace);
    s = soft_consonants(&s, &mut trace);
    s = syllabic_liquid(&s, &mut trace);
    s = simplify_clusters(&s, &mut trace);
    // The Interslavic adjective lemma continues the *definite* form (*-ъjь), not
    // the short nominative the cache cites: append the definite ending BEFORE yer
    // resolution, because it flips the Havlík parity of the stem yers
    // (*bědьnъ → strong ь → *bědeny, but *bědьnъjь → weak ь → bědny; *kortъkъjь
    // → kråtky). Possessives (-inъ/-ovъ) keep the short form. The modern South
    // citations are short forms whose vocalized yer says nothing about the long
    // form, so the reflex-retention vote is suppressed for the definite stem.
    let mut reflexes = reflexes;
    if pos == Pos::Adjective && s.ends_with('ъ') && !s.ends_with("ъjь") {
        let stem = &s[..s.len() - 'ъ'.len_utf8()];
        // A denominal possessive (*materinъ, *bratrovъ) keeps the short form — but
        // the -in/-ov/-ev/-yn string also ends many *qualitative* roots (*novъ,
        // *gotovъ, *zdravъ) that DO continue the long definite -y. Disambiguate by
        // the modern reflexes: if any is cited in a long-form (vowel- or -yj-final)
        // shape, the adjective is qualitative and takes the definite ending.
        let looks_possessive = ["in", "ov", "ev", "yn"].iter().any(|p| stem.ends_with(p));
        let long_reflex = reflexes.iter().any(|r| adj_reflex_long(r));
        let possessive = looks_possessive && !long_reflex;
        if !possessive {
            let before = s.clone();
            s.push_str("jь");
            reflexes = &[];
            step(
                &mut trace,
                "adj-definite",
                &before,
                &s,
                "Pridavnik prodolžaje opreděljenu formu *-ъjь (dȯlga forma), ne kratku.",
                STEEN,
            );
        }
    }
    s = collective_je(&s, &mut trace);
    s = yers(&s, reflexes, &mut trace);
    s = endings(&s, pos, gender, stem_class, &mut trace);
    s = finalize(&s, &mut trace);

    // The rule engine is deterministic; score reflects how much survived intact
    // and whether the source looked well-formed.
    let score = if s.is_empty() { 0.1 } else { 0.66 };
    let mut cand = Candidate::new(s, CandidateSource::ProtoSlavicRule, score);
    cand.trace = trace;
    cand
}

/// True when a modern adjective reflex is cited in its long (definite/attributive)
/// form — vowel-final, or ending in a long-form vowel + `j` (Russian -yj/-ij/-oj) —
/// rather than the short predicative/possessive form.
fn adj_reflex_long(r: &str) -> bool {
    let mut it = r.chars().rev();
    match it.next() {
        Some(c) if is_full_vowel(c) => true,
        Some('j') => it.next().map(is_full_vowel).unwrap_or(false),
        _ => false,
    }
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
        // Drop combining accent marks (Proto-Slavic prosody notation) and the
        // parentheses Wiktionary wraps optional letters in (*(j)azъ → jazъ): keep
        // the letter, drop the brackets. Also drop syllable dots / hyphens.
        if ('\u{0300}'..='\u{036F}').contains(&ch)
            || matches!(ch, '`' | '´' | '(' | ')' | '·' | '.' | '‑')
        {
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
fn palatals(input: &str, pos: Pos, trace: &mut Vec<RuleStep>) -> String {
    let mut out = input.to_string();
    // Verb infinitives keep a word-final velar+t cluster transparent
    // (*pekti→pekti, *mogti→mogti); don't palatalize -kti/-gti there.
    let verb_final_kt = pos == Pos::Verb && (out.ends_with("kti") || out.ends_with("gti"));
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
        if verb_final_kt && (from == "kti" || from == "kt") {
            continue;
        }
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
        // Word-initial *orC/*olC (no leading consonant) metathesizes to raC/laC
        // (rising accent → a): *orbota→rabota, *orzumъ→razumъ, *olkъtь→lakȯtь.
        if i == 0
            && n >= 3
            && matches!(chars[0], 'o' | 'e')
            && matches!(chars[1], 'r' | 'l')
            && is_cons(chars[2])
        {
            out.push(chars[1]);
            out.push(if chars[0] == 'o' { 'a' } else { 'ě' });
            i += 2;
            continue;
        }
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

/// Word-initial prothesis: Interslavic prepends j- before a front nasal and v-
/// before a back nasal/rounded vowel (*ęzykъ → język, *ǫtroba → vųtroba), and
/// resolves an initial tense yer (*jьgra → igra, *jъ- → y-).
fn prothesis(input: &str, trace: &mut Vec<RuleStep>) -> String {
    let out = if let Some(rest) = input.strip_prefix("jь") {
        format!("i{rest}")
    } else if let Some(rest) = input.strip_prefix("jъ") {
        format!("y{rest}")
    } else if let Some(rest) = input.strip_prefix('ę') {
        format!("ję{rest}")
    } else if let Some(rest) = input.strip_prefix('ų') {
        format!("vų{rest}")
    } else if let Some(rest) = input.strip_prefix('ě') {
        // Word-initial jat takes a prothetic j and de-flavors: *ěsti → jesti,
        // *ěxati → jehati.
        format!("je{rest}")
    } else if let Some(rest) = input.strip_prefix('e') {
        // Word-initial *e- takes a prothetic j: *edinъ → jedin, *ezero →
        // jezero, *elenь → jelenj.
        format!("je{rest}")
    } else if let Some(rest) = input.strip_prefix('a') {
        // Word-initial *a- takes a prothetic j: *avorъ → javor, *agoda →
        // jagoda, *arьmo → jaŕmo. Interslavic has ~80 native ja- lemmas and no
        // native bare a- lemma (Slavic avoided initial a-), so this is safe on
        // the reconstructions that reach the engine (later loans are not stored
        // as *a- Proto-Slavic entries).
        format!("ja{rest}")
    } else {
        input.to_string()
    };
    step(
        trace,
        "prothesis",
        input,
        &out,
        "Protetični soglasnik: počętny e/ě→je, ę→ję, ų→vų.",
        PHON,
    );
    out
}

/// Soft consonants: the etymological soft sonorants surface as digraphs before a
/// vowel or word-finally (ľ→lj, ň→nj, ř→rj: moře→morje, poľe→polje, koňь→konj)
/// but as a plain consonant before another consonant. After a labial, *lj gives
/// just labial+j (zemľa→zemja, not zemlja) — the East/South epenthetic l is
/// dropped.
fn soft_consonants(input: &str, trace: &mut Vec<RuleStep>) -> String {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let trigger = |c: Option<char>| match c {
        None => true, // word-final
        Some(x) => is_full_vowel(x) || x == 'ь' || x == 'ъ',
    };
    let mut out = String::new();
    for i in 0..n {
        let next = chars.get(i + 1).copied();
        // Before /i/ the softness is redundant (i already palatalizes), so the
        // sonorant stays plain: *gňida → gnida, not gnjida. Elsewhere before a
        // vowel or word-finally it surfaces as a digraph.
        let soft_pos = trigger(next) && next != Some('i');
        let prev = out.chars().last().unwrap_or(' ');
        match chars[i] {
            'ľ' | 'ĺ' => {
                if matches!(prev, 'p' | 'b' | 'v' | 'm') {
                    out.push('j'); // labial + *lj -> labial + j (zemja)
                } else if soft_pos {
                    out.push_str("lj");
                } else {
                    out.push('l');
                }
            }
            'ň' => out.push_str(if soft_pos { "nj" } else { "n" }),
            'ř' | 'ŕ' => out.push_str(if soft_pos { "rj" } else { "r" }),
            other => out.push(other),
        }
    }
    step(
        trace,
        "soft-consonants",
        input,
        &out,
        "Mękke soglasniky: ľ→lj, ň→nj, ř→rj prěd glasnikom; labial+lj→labial+j (zemja).",
        PHON,
    );
    out
}

/// South-Slavic cluster simplification adopted by Interslavic: medial *dl/*tl → l
/// (*modlitva → molitva, *motovidlo → motovilo). Never word-initial (dlanj).
fn simplify_clusters(input: &str, trace: &mut Vec<RuleStep>) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        // Medial *dl/*tl → l. The preceding nucleus may be a full vowel (*mydlo)
        // or a syllabic liquid produced upstream (*gъrdlo→gŕdlo→gŕlo), so only the
        // anti-initial guard (i>0) is required (B13).
        if i > 0 && matches!(c, 'd' | 't') && chars.get(i + 1) == Some(&'l') {
            continue; // drop the d/t before l
        }
        out.push(c);
    }
    step(
        trace,
        "cluster-dl",
        input,
        &out,
        "Uproščenje: medialne *dl/*tl → l.",
        STEEN,
    );
    out
}

/// Syllabic liquids: a yer + r/l wedged before another consonant becomes a
/// syllabic liquid (*sьrpъ → sŕp, *sъmьrtь → smŕť, *vьrba → vŕba). Runs after
/// soft-consonants (so the new ŕ/ĺ survive) and before yer resolution.
fn syllabic_liquid(input: &str, trace: &mut Vec<RuleStep>) -> String {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut out = String::new();
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if (c == 'ь' || c == 'ъ')
            && i + 1 < n
            && matches!(chars[i + 1], 'r' | 'l')
            && (i + 2 >= n || is_cons(chars[i + 2]))
        {
            if chars[i + 1] == 'r' {
                out.push('ŕ'); // syllabic r stays: *sьrpъ→sŕp, *vьrxъ→vŕh
            } else {
                // *ъl/*ьl vocalizes to ȯl, it does NOT become a syllabic ĺ:
                // *vьlkъ→vȯlk, *dъlgъ→dȯlg, *pьlnъ→pȯlny (RULE_SPEC §2 liquids).
                out.push('ȯ');
                out.push('l');
            }
            i += 2; // consume the yer and the liquid
            continue;
        }
        out.push(c);
        i += 1;
    }
    step(
        trace,
        "syllabic-liquid",
        input,
        &out,
        "Slogotvorne plavne: *ьr/*ъr→ŕ (sŕp), a *ьl/*ъl→ȯl (vȯlk, dȯlg).",
        STEEN,
    );
    out
}

/// The collective/abstract suffix *-ьje: the weak front yer drops before *j,
/// giving a word-final `-je` (*kopьje→kopje, *znanьje→znanje, *zdorvьje→zdravje),
/// not the tense `-ije` the generic yer-before-*j rule would produce. Targets
/// ONLY the word-final `ьje` suffix, so *čьjь (→čij) and other tense yers are
/// untouched. The dictionary has no native `-ije` lemma, so this is near-lossless.
fn collective_je(input: &str, trace: &mut Vec<RuleStep>) -> String {
    if let Some(stem) = input.strip_suffix("ьje") {
        if !stem.is_empty() {
            let out = format!("{stem}je");
            step(
                trace,
                "collective-je",
                input,
                &out,
                "Zbirny/odvlečeny sufiks *-ьje → -je (kopje, znanje), ne -ije.",
                STEEN,
            );
            return out;
        }
    }
    input.to_string()
}

/// Yer resolution. Three fates:
///   * **tense** (a yer before *j) always vocalizes: *ь→i, *ъ→y (novъjь→novy,
///     pьjǫ→pij-);
///   * **strong** (Havlík: alternating from the right, odd positions) vocalizes:
///     *ъ→ȯ, *ь→e (sъnъ→sȯn, pьsъ→pes);
///   * **weak** normally drops, unless the modern reflexes vote to keep a vowel
///     at that position — a lexicalized retention the reflexes alone can resolve
///     (pьsati→pisati vs bьrati→brati).
fn yers(input: &str, reflexes: &[String], trace: &mut Vec<RuleStep>) -> String {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let is_yer = |c: char| c == 'ъ' || c == 'ь';

    // Tense yers: immediately before *j.
    let mut tense = vec![false; n];
    for idx in 0..n {
        if is_yer(chars[idx]) && idx + 1 < n && chars[idx + 1] == 'j' {
            tense[idx] = true;
        }
    }

    // Havlík strong/weak for the non-tense yers. A full vowel or a tense yer
    // (which surfaces as a vowel) resets the alternation run.
    let mut strong = vec![false; n];
    let mut counter = 0;
    for idx in (0..n).rev() {
        let c = chars[idx];
        if is_yer(c) && !tense[idx] {
            if counter % 2 == 1 {
                strong[idx] = true;
            }
            counter += 1;
        } else if is_full_vowel(c) || matches!(c, 'ŕ' | 'ĺ') || (is_yer(c) && tense[idx]) {
            // A full vowel, a tense yer, or a syllabic liquid is a syllable
            // nucleus and resets the Havlík alternation (*sъmьrtь → smŕť, not
            // sȯmŕt: the ъ is weak because the following ŕ carries the syllable).
            counter = 0;
        }
    }

    let mut out = String::new();
    let mut cons_before = 0usize; // consonants seen so far, for reflex alignment
    for idx in 0..n {
        let c = chars[idx];
        if is_yer(c) {
            let back = c == 'ъ';
            if tense[idx] {
                out.push(if back { 'y' } else { 'i' });
            } else if strong[idx] {
                out.push(if back { 'ȯ' } else { 'e' });
            } else if idx + 1 == n {
                // Word-final weak yer: drops. If it is a soft (front) yer after l
                // or n it palatalizes them: *solь->solj, *dьnь->denj. A final soft
                // *ŕ, however, reduces to plain r (*carь->car, *zvěrь->zvěr), so r
                // is excluded here. (Final yers are not reflex-retained.)
                if !back && matches!(out.chars().last(), Some('l' | 'n')) {
                    out.push('j');
                }
            } else if let Some(v) = reflex_vowel_vote(reflexes, cons_before) {
                // Internal weak yer retained: adopt the reflexes' vowel (o -> ȯ for
                // a back yer: *dъska -> dȯska; *pьsati keeps i).
                out.push(map_retained_vowel(v, back));
            }
            // otherwise the weak yer drops with no trace
        } else {
            out.push(c);
            if is_cons(c) {
                cons_before += 1;
            }
        }
    }
    step(
        trace,
        "yers",
        input,
        &out,
        "Jery: napręžene (prěd j) *ь→i/*ъ→y; silne *ъ→ȯ/*ь→e; slabe padajų (ale ostajų, ako naslědniky drže glasnik).",
        STEEN,
    );
    out
}

/// POS-aware lemma endings.
fn endings(
    input: &str,
    pos: Pos,
    gender: Option<Gender>,
    stem_class: Option<&str>,
    trace: &mut Vec<RuleStep>,
) -> String {
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
            // Possessive adjectives (*-inъ, *-ovъ, *-jь) stay in the short form:
            // mamin, ottsov — no -y.
            let possessive = out.ends_with("in")
                || out.ends_with("ov")
                || out.ends_with("ev")
                || out.ends_with("yn");
            if !possessive && !out.ends_with('y') && !out.ends_with("ji") && ends_cons(&out) {
                // Soft-stem adjectives take -i, hard stems -y: *siňь->sinji,
                // *svěžь->svěži, but *dobrъ->dobry (RULE_SPEC §3.2).
                out.push(if ends_soft(&out) { 'i' } else { 'y' });
            }
        }
        Pos::Noun => {
            // Neuter o-stem keeps -o/-e; a-stem keeps -a; masculine o-stem drops
            // the final yer (already gone) leaving a consonant.
            if gender == Some(Gender::Neuter) && ends_cons(&out) {
                out.push('o');
            }
            // Masculine n-stem: the archaic nominative *-y survives the sound
            // rules (*kamy → kamy), but the dictionary cites the extended
            // oblique stem (kamenj) — categorical in the official CSV (issue
            // #76 pre-check: every cache n-stem in -y is cited in -enj, none
            // in -y). Wiktionary's declension category supplies the class;
            // neuter n-stems (*jьmę → imę) end in -ę and are untouched.
            if stem_class.is_some_and(|sc| sc.contains("n-stem")) && out.ends_with('y') {
                out.truncate(out.len() - 1);
                out.push_str("enj");
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
/// `pub(crate)`: the site's proto-reflex join (issue #73b) folds ancestor and
/// cache words with this same table, so the two sides can never drift.
pub(crate) fn debase_vowel(ch: char) -> char {
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

/// Majority vote across reflexes: which vowel (if any) is kept right after
/// `cons_before` consonants — the aligned yer position? Returns the most common
/// retained vowel, or `None` when the reflexes drop it. Reflexes that can't be
/// aligned abstain.
fn reflex_vowel_vote(reflexes: &[String], cons_before: usize) -> Option<char> {
    use std::collections::BTreeMap;
    let mut votes: BTreeMap<char, usize> = BTreeMap::new();
    let mut drop_votes = 0usize;
    for r in reflexes {
        match reflex_vowel_at(r, cons_before) {
            Some(Some(v)) => *votes.entry(v).or_default() += 1,
            Some(None) => drop_votes += 1,
            None => {}
        }
    }
    let keep: usize = votes.values().sum();
    // Require CORROBORATION (>=2 reflexes agree on a vowel) before retaining a
    // weak yer. A single reflex showing a vowel at the aligned slot is usually a
    // misalignment — a cognate with an epenthetic/pleophonic/different segment
    // shifts the consonant index and injects a spurious vowel (*babъka→babka, not
    // babaka; *čajьka→čajka, not čajeka). Genuine lexicalized retentions are
    // corroborated across reflexes (*pьsati→pisati has three, *dъska→dȯska two).
    if keep >= 2 && keep > drop_votes {
        votes.into_iter().max_by_key(|(_, n)| *n).map(|(v, _)| v)
    } else {
        None
    }
}

/// In one reflex: `None` if it can't be aligned; `Some(None)` if the aligned slot
/// is a consonant (dropped); `Some(Some(v))` if it is the vowel `v`.
fn reflex_vowel_at(r: &str, cons_before: usize) -> Option<Option<char>> {
    let cs: Vec<char> = r.chars().collect();
    if cons_before == 0 {
        return cs
            .first()
            .map(|&c| if is_reflex_vowel(c) { Some(c) } else { None });
    }
    let mut cnt = 0;
    for i in 0..cs.len() {
        if is_reflex_cons(cs[i]) {
            cnt += 1;
            if cnt == cons_before {
                return Some(match cs.get(i + 1) {
                    Some(&c) if is_reflex_vowel(c) => Some(c),
                    _ => None,
                });
            }
        }
    }
    None
}

/// Map a reflex vowel onto the retained-yer spelling: a back yer whose reflex is
/// `o` takes the strong-back letter `ȯ` (dъska→dȯska); otherwise keep the vowel.
fn map_retained_vowel(v: char, back_yer: bool) -> char {
    match (back_yer, v) {
        (true, 'o') => 'ȯ',
        _ => v,
    }
}

fn is_reflex_vowel(c: char) -> bool {
    matches!(
        c,
        'a' | 'e'
            | 'i'
            | 'o'
            | 'u'
            | 'y'
            | 'ě'
            | 'ę'
            | 'ǫ'
            | 'ų'
            | 'å'
            | 'ȯ'
            | 'á'
            | 'é'
            | 'í'
            | 'ó'
            | 'ú'
            | 'ý'
            | 'à'
            | 'è'
    )
}

fn is_reflex_cons(c: char) -> bool {
    // Count `j` as a consonant so reflex alignment matches the proto-side
    // consonant count (`is_cons` counts it too): *vojьna → vojna, not vojana.
    c.is_alphabetic() && !is_reflex_vowel(c) && c != 'ъ' && c != 'ь'
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

/// True when the stem ends in a soft (palatal/palatalized) consonant, which takes
/// the soft adjective ending -i rather than hard -y.
fn ends_soft(s: &str) -> bool {
    if s.ends_with("lj") || s.ends_with("nj") || s.ends_with("rj") {
        return true;
    }
    matches!(
        s.chars().last(),
        Some('š' | 'ž' | 'č' | 'j' | 'ć' | 'đ' | 'c' | 'ś' | 'ź' | 'ť' | 'ď' | 'ŕ' | 'ĺ' | 'ń')
    )
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

    #[test]
    fn tense_yer_before_j() {
        // A yer before *j is tense and vocalizes: the hard definite adjective
        // ending *-ъjь surfaces with y (novъjь → novy). Without the tense rule
        // strict Havlík would misassign the ъ as strong (→ novȯj-).
        assert!(normalized_match(&gen("*novъjь", Pos::Adjective), "novy"));
    }

    #[test]
    fn adjective_lemma_continues_the_definite_form() {
        // The lemma is the definite *-ъjь form, which flips the Havlík parity of
        // the stem yers: the short form would give *kråtȯky/*bědeny, the definite
        // form correctly drops the now-weak yer (kråtky, bědny, nizky).
        assert_eq!(gen("*kortъkъ", Pos::Adjective), "kråtky");
        assert_eq!(gen("*bědьnъ", Pos::Adjective), "bědny");
        assert_eq!(gen("*nizъkъ", Pos::Adjective), "nizky");
        // Possessives keep the short form (no -y).
        assert!(!gen("*materinъ", Pos::Adjective).ends_with('y'));
        // A reconstruction already cited in the long form is not doubled.
        assert_eq!(gen("*kortъkъjь", Pos::Adjective), "kråtky");
    }

    #[test]
    fn syllabic_l_vocalizes_to_ol() {
        // *ъl/*ьl → ȯl (not a syllabic ĺ): vȯlk, dȯlg, pȯlny.
        assert!(
            normalized_match(&gen("*vьlkъ", Pos::Noun), "vȯlk"),
            "{}",
            gen("*vьlkъ", Pos::Noun)
        );
        assert!(normalized_match(&gen("*dъlgъ", Pos::Noun), "dȯlg"));
        assert!(normalized_match(&gen("*pьlnъ", Pos::Adjective), "pȯlny"));
    }

    #[test]
    fn verb_infinitive_keeps_velar_t_cluster() {
        // *pekti/*mogti stay transparent (official pekti/mogti), not peći/moći.
        assert!(
            normalized_match(&gen("*pekti", Pos::Verb), "pekti"),
            "{}",
            gen("*pekti", Pos::Verb)
        );
        assert!(normalized_match(&gen("*mogti", Pos::Verb), "mogti"));
    }

    #[test]
    fn final_soft_r_reduces_to_plain_r() {
        // *carь→car, *zvěrь→zvěr (soft ŕ → r), but *solь→solj keeps the soft l.
        assert!(
            normalized_match(&gen("*carь", Pos::Noun), "car"),
            "{}",
            gen("*carь", Pos::Noun)
        );
        assert!(normalized_match(&gen("*zvěrь", Pos::Noun), "zvěr"));
        assert!(gen("*solь", Pos::Noun).contains("lj"));
    }

    #[test]
    fn soft_adjective_takes_i() {
        // Soft-stem adjectives take -i not -y: *siňь→sinji, *svěžь→svěži.
        assert!(
            normalized_match(&gen("*siňь", Pos::Adjective), "sinji"),
            "{}",
            gen("*siňь", Pos::Adjective)
        );
        assert!(normalized_match(&gen("*svěžь", Pos::Adjective), "svěži"));
        assert!(normalized_match(&gen("*dobrъ", Pos::Adjective), "dobry")); // hard stays -y
    }

    #[test]
    fn dl_simplifies_after_syllabic_liquid() {
        // *gъrdlo → gŕlo (normalizes to grlo): the dl-drop fires after ŕ too.
        assert!(
            normalized_match(&gen("*gъrdlo", Pos::Noun), "grlo"),
            "{}",
            gen("*gъrdlo", Pos::Noun)
        );
    }

    #[test]
    fn word_initial_liquid_metathesis() {
        // Word-initial *orC → raC: *orbota→rabota, *orzumъ→razum.
        assert!(
            normalized_match(&gen("*orbota", Pos::Noun), "rabota"),
            "{}",
            gen("*orbota", Pos::Noun)
        );
        assert!(normalized_match(&gen("*orzumъ", Pos::Noun), "razum"));
    }

    #[test]
    fn word_initial_e_jat_prothesis() {
        // Word-initial *e-/*ě- take a prothetic j: *ěsti → jesti, *ezero → jezero.
        assert!(normalized_match(
            &generate("*ěsti", Pos::Verb, None).form,
            "jesti"
        ));
        assert!(normalized_match(
            &generate("*ezero", Pos::Noun, None).form,
            "jezero"
        ));
    }

    #[test]
    fn reflex_alignment_counts_j() {
        // Regression for the is_reflex_cons `j` bug: the yer in *vojьna aligns
        // past the j, so the reflexes (vojna, no vowel there) drop it.
        let out = generate_with_reflexes(
            "*vojьna",
            Pos::Noun,
            None,
            &["vojna".into(), "vojna".into(), "vojna".into()],
            None,
        )
        .form;
        assert!(normalized_match(&out, "vojna"), "got {out}");
    }

    #[test]
    fn collective_je_suffix_drops_yer() {
        // The collective/abstract *-ьje suffix: the weak front yer drops before j
        // → -je (kopje, znanje), not the tense -ije.
        assert_eq!(gen("*kopьje", Pos::Noun), "kopje");
        assert!(normalized_match(&gen("*znanьje", Pos::Noun), "znanje"));
    }

    #[test]
    fn word_initial_a_takes_prothetic_j() {
        // *a- → ja-: *avorъ→javor, *agoda→jagoda (Slavic avoided initial a-).
        assert!(normalized_match(&gen("*avorъ", Pos::Noun), "javor"));
        assert!(normalized_match(&gen("*agoda", Pos::Noun), "jagoda"));
    }

    #[test]
    fn qualitative_adjective_takes_definite_y() {
        // A qualitative root ending -ov/-in is NOT a possessive when the reflexes
        // cite the long form: *novъ→novy, *gotovъ→gotovy (not short nov/gotov).
        let novy = generate_with_reflexes(
            "*novъ",
            Pos::Adjective,
            None,
            &["novyj".into(), "novy".into(), "novy".into()],
            None,
        )
        .form;
        assert!(normalized_match(&novy, "novy"), "got {novy}");
        let gotovy = generate_with_reflexes(
            "*gotovъ",
            Pos::Adjective,
            None,
            &["gotovyj".into(), "gotovy".into()],
            None,
        )
        .form;
        assert!(normalized_match(&gotovy, "gotovy"), "got {gotovy}");
        // A true possessive (short reflexes) keeps the short form.
        let materin =
            generate_with_reflexes("*materinъ", Pos::Adjective, None, &["materin".into()], None)
                .form;
        assert!(!materin.ends_with('y'), "got {materin}");
    }

    #[test]
    fn weak_yer_retention_requires_corroboration() {
        // Two agreeing reflexes retain the weak yer (*dъska→dȯska)...
        let two = generate_with_reflexes(
            "*dъska",
            Pos::Noun,
            None,
            &["doska".into(), "deska".into()],
            None,
        )
        .form;
        assert!(
            !normalized_match(&two, "dska"),
            "two-reflex retention: {two}"
        );
        // ...but a single (possibly misaligned) reflex is not enough: the yer
        // drops rather than injecting a spurious vowel (babka, not babaka).
        let one = generate_with_reflexes("*dъska", Pos::Noun, None, &["doska".into()], None).form;
        assert!(normalized_match(&one, "dska"), "single-reflex guard: {one}");
    }

    #[test]
    fn reflex_guided_weak_yer() {
        // Strict Havlík drops the weak yer of *pьsati → *psati (matching only
        // Czech psát); the reflexes vocalize it, so with reflex evidence the
        // engine derives pisati — no length hack needed.
        let pure = generate("*pьsati", Pos::Verb, None).form;
        assert!(normalized_match(&pure, "psati"), "pure Havlík was {pure}");
        let guided = generate_with_reflexes(
            "*pьsati",
            Pos::Verb,
            None,
            &["pisati".into(), "pisat".into(), "pisac".into()],
            None,
        )
        .form;
        assert!(
            normalized_match(&guided, "pisati"),
            "reflex-guided was {guided}"
        );
        // *bьrati: the reflexes also drop it (brati, brać), so the weak yer drops.
        let brati = generate_with_reflexes(
            "*bьrati",
            Pos::Verb,
            None,
            &["brati".into(), "brat".into(), "brac".into()],
            None,
        )
        .form;
        assert!(normalized_match(&brati, "brati"), "brati was {brati}");
    }

    #[test]
    fn n_stem_stem_class_cites_oblique_stem() {
        // Issue #76: a masculine n-stem's archaic nominative *-y survives the
        // sound rules, but the dictionary cites the extended oblique stem.
        // Pinned exactly — the flavored letters are the point.
        let n_stem = Some("Proto-Slavic masculine n-stem nouns");
        let cite = |proto: &str| generate_with_reflexes(proto, Pos::Noun, None, &[], n_stem).form;
        assert_eq!(cite("*kamy"), "kamenj");
        // The override composes with the earlier sound rules: prothetic j-
        // (*ely → jely) and liquid-metathesis å (*polmy → plåmy).
        assert_eq!(cite("*ely"), "jelenj");
        assert_eq!(cite("*polmy"), "plåmenj");
        // Without the declension category the archaic nominative stays.
        assert_eq!(
            generate_with_reflexes("*kamy", Pos::Noun, None, &[], None).form,
            "kamy"
        );
        // Neuter n-stems end in -ę, not -y: untouched.
        assert_eq!(
            generate_with_reflexes(
                "*jьmę",
                Pos::Noun,
                None,
                &[],
                Some("Proto-Slavic n-stem nouns")
            )
            .form,
            "imę"
        );
        // The override lives in the Noun arm only: other POS are untouched.
        assert_eq!(
            generate_with_reflexes("*kamy", Pos::Adjective, None, &[], n_stem).form,
            "kamy"
        );
    }
}
