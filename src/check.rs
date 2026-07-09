//! Text verification against the lexicon (issue #11 phase 3, `check-text`).
//!
//! Builds the same [`crate::forms::FormRecord`] index the site's API exports
//! (official lemmas + their full paradigms, plus generated lemmas with their
//! calibrated probability from the committed `data/novel-words.tsv`), then
//! tokenizes the input and classifies every token:
//!
//! `known-lemma` / `known-form` / `generated` (carries p) / `unknown` (with
//! nearest-lemma suggestions) — plus curated semantic-trap warnings from
//! `data/semantic-notes.json`. Two-token keys (reflexive `X sę` verbs and
//! two-word official lemmas) are found by a general bigram lookup; only
//! 3+-token phrases are out of tokenized reach (their lemma records still
//! exist in the index for direct key lookup).
//!
//! CLI-first and self-contained: no site build required, deterministic output,
//! `--json` for agents.

use crate::forms::{self, FormRecord, RecordSink};
use crate::model::Pos;
use crate::official::{self, OfficialEntry};
use crate::orthography as ortho;
use anyhow::Result;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

pub const SEMANTIC_NOTES: &str = "data/semantic-notes.json";

/// A curated semantic-trap note (see `data/semantic-notes.json`).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SemanticNote {
    pub warning: String,
    #[serde(default)]
    pub prefer: Vec<String>,
}

pub struct Index {
    /// key → records (lemma citations and inflected forms).
    pub by_key: HashMap<String, Vec<FormRecord>>,
    /// All lemma keys, for nearest-suggestion search.
    pub lemma_keys: Vec<(String, String)>, // (key, display lemma)
    pub notes: BTreeMap<String, SemanticNote>,
    /// Noun lemma key → dictionary gender (m/f/n), for agreement checking.
    pub noun_gender: HashMap<String, char>,
    /// Preposition key (folded) → the cases it governs, sourced from the
    /// interslavic crate's curated `prepositions::PREPOSITIONS` table (which
    /// encodes the community dictionary's `(+N)` government, instrumental = +5).
    pub prep_cases: HashMap<String, Vec<&'static str>>,
}

/// Build the verification index from the official dictionary (lemmas + full
/// paradigms) and, when present, the committed novel-word proposals (lemma
/// records with their calibrated probability).
pub fn build_index(entries: &[OfficialEntry], novel_words_tsv: Option<&Path>) -> Index {
    let mut sink = RecordSink::default();
    forms::closed_class_records(&mut sink);
    let mut seen: HashSet<String> = HashSet::new();
    let mut noun_gender: HashMap<String, char> = HashMap::new();
    let mut prep_cases: HashMap<String, Vec<&'static str>> = HashMap::new();
    // Preposition government now comes from the interslavic crate's curated
    // table (issue #12), which is built from the same official dictionary and
    // encodes the (+5)=instrumental convention centrally. Fold each flavored
    // key to the index's standard-orthography key convention and translate the
    // crate's Case to the local labels.
    let case_label = |c: interslavic::Case| -> &'static str {
        match c {
            interslavic::Case::Gen => "gen",
            interslavic::Case::Dat => "dat",
            interslavic::Case::Acc => "akuz",
            interslavic::Case::Ins => "instr",
            interslavic::Case::Loc => "lok",
            interslavic::Case::Nom => "nom",
        }
    };
    for (prep, cases) in interslavic::prepositions::PREPOSITIONS {
        let entry = prep_cases.entry(forms::form_key(prep)).or_default();
        for c in cases.iter().map(|c| case_label(*c)) {
            if !entry.contains(&c) {
                entry.push(c);
            }
        }
    }
    for e in entries {
        // ~230 rows list byform variants in one cell ("iměti, imati",
        // "srědnji, srědny") — each variant is its own lemma.
        for isv in e.isv.split(',').map(str::trim) {
            if isv.is_empty() || isv.contains('#') || isv.contains('!') {
                continue;
            }
            // Strip government hints ("pozirati (na)") and reject raw
            // notation, same as the site API.
            let Some(clean) = forms::citation(isv) else {
                continue;
            };
            let isv = clean.as_str();
            // Two-token lemmas (reflexive verbs AND collocations) are reachable
            // via the general bigram lookup; only 3+-token phrases stay
            // lemma-only.
            let single = !isv.contains(' ') || isv.ends_with(" sę");
            if e.pos == Pos::Noun {
                if let Some(g) = e.noun_traits.gender {
                    let c = match g {
                        crate::model::Gender::Masculine => 'm',
                        crate::model::Gender::Feminine => 'f',
                        crate::model::Gender::Neuter => 'n',
                        _ => ' ',
                    };
                    if c != ' ' {
                        noun_gender.entry(forms::form_key(isv)).or_insert(c);
                    }
                }
            }
            sink.add(
                isv,
                "",
                isv,
                0,
                e.pos.code(),
                "lemma",
                "official",
                None,
                &e.english,
            );
            if single
                && matches!(
                    e.pos,
                    Pos::Noun | Pos::ProperNoun | Pos::Adjective | Pos::Verb
                )
                && seen.insert(format!("{isv}|{}", e.pos.code()))
            {
                forms::paradigm_records(
                    &mut sink,
                    isv,
                    e.pos,
                    e.noun_traits.gender,
                    0,
                    "official",
                    None,
                    &e.english,
                );
            }
            if single
                && matches!(e.pos, Pos::Pronoun | Pos::Numeral)
                && seen.insert(format!("{isv}|{}", e.pos.code()))
            {
                forms::pronoun_numeral_records(&mut sink, isv, e.pos, 0, "official", &e.english);
            }
        }
    }
    if let Some(path) = novel_words_tsv {
        if let Ok(tsv) = std::fs::read_to_string(path) {
            for line in tsv.lines().skip(1) {
                let cols: Vec<&str> = line.split('\t').collect();
                if cols.len() < 8 {
                    continue;
                }
                let (form, pos, prob, gloss) = (cols[0], cols[1], cols[2], cols[7]);
                let pos: &'static str = match pos {
                    "noun" => "noun",
                    "verb" => "verb",
                    "adj" => "adj",
                    "adv" => "adv",
                    "proper_noun" => "proper_noun",
                    _ => "other",
                };
                sink.add(
                    form,
                    "",
                    form,
                    0,
                    pos,
                    "lemma",
                    "generated",
                    prob.parse::<f64>().ok(),
                    gloss,
                );
            }
        }
    }
    let records = sink.into_records();
    let mut by_key: HashMap<String, Vec<FormRecord>> = HashMap::new();
    let mut lemma_keys: Vec<(String, String)> = Vec::new();
    let mut lemma_seen: HashSet<String> = HashSet::new();
    for r in records {
        if r.source == "lemma" && lemma_seen.insert(r.key.clone()) {
            lemma_keys.push((r.key.clone(), r.lemma.clone()));
        }
        by_key.entry(r.key.clone()).or_default().push(r);
    }
    lemma_keys.sort();
    let notes: BTreeMap<String, SemanticNote> = std::fs::read_to_string(SEMANTIC_NOTES)
        .ok()
        .and_then(|raw| serde_json::from_str::<BTreeMap<String, SemanticNote>>(&raw).ok())
        .map(|m| {
            m.into_iter()
                .map(|(k, v)| (forms::form_key(&k), v))
                .collect()
        })
        .unwrap_or_default();
    Index {
        by_key,
        lemma_keys,
        notes,
        noun_gender,
        prep_cases,
    }
}

#[derive(Debug, serde::Serialize)]
pub struct TokenReport {
    pub token: String,
    /// known-lemma | known-form | generated | unknown
    pub status: &'static str,
    /// Distinct lemmas this surface can belong to.
    pub lemmas: Vec<String>,
    /// Analyses of the matching records (feature strings).
    pub analyses: Vec<String>,
    pub ambiguous: bool,
    /// Calibrated probability, for generated lemmas.
    pub probability: Option<f64>,
    /// Nearest known lemmas, for unknown tokens.
    pub suggestions: Vec<String>,
    /// Curated semantic-trap warning, if any.
    pub warning: Option<String>,
    pub prefer: Vec<String>,
    /// Grammar-agreement warning (issue #13 §3): set when NO combination of
    /// this token's analyses is compatible with its neighbour.
    pub agreement: Option<String>,
}

/// Tokenize: maximal runs of letters (flavored letters are alphabetic).
/// Internal hyphens stay part of the token (čij-nebųď is one lemma); the
/// reflexive bigram is handled by the caller.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphabetic() {
            cur.push(c);
        } else if c == '-'
            && !cur.is_empty()
            && chars.peek().map(|n| n.is_alphabetic()).unwrap_or(false)
        {
            cur.push('-');
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Tokenize AND record punctuation breaks: `breaks[i]` is true when token i
/// is separated from its predecessor by anything beyond whitespace (comma,
/// period, table cell, …) — agreement never crosses such a break.
pub fn tokenize_with_breaks(text: &str) -> (Vec<String>, Vec<bool>) {
    let mut tokens: Vec<String> = Vec::new();
    let mut breaks: Vec<bool> = Vec::new();
    let mut cur = String::new();
    let mut pending_break = true; // text start counts as a break
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphabetic() {
            cur.push(c);
        } else if c == '-'
            && !cur.is_empty()
            && chars.peek().map(|n| n.is_alphabetic()).unwrap_or(false)
        {
            cur.push('-');
        } else {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
                breaks.push(pending_break);
                pending_break = false;
            }
            if !c.is_whitespace() {
                pending_break = true;
            }
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
        breaks.push(pending_break);
    }
    (tokens, breaks)
}

/// Verify a full text: tokenization with punctuation breaks, so agreement
/// checks only apply within a phrase.
pub fn check_text(index: &Index, text: &str) -> Vec<TokenReport> {
    let (tokens, breaks) = tokenize_with_breaks(text);
    check_tokens_impl(index, &tokens, Some(&breaks))
}

pub fn check_tokens(index: &Index, tokens: &[String]) -> Vec<TokenReport> {
    check_tokens_impl(index, tokens, None)
}

fn check_tokens_impl(
    index: &Index,
    tokens: &[String],
    breaks: Option<&[bool]>,
) -> Vec<TokenReport> {
    let mut reports = Vec::new();
    let mut grammar: Vec<TokenGrammar> = Vec::new();
    let mut report_breaks: Vec<bool> = Vec::new();
    let mut i = 0usize;
    while i < tokens.len() {
        let tok = &tokens[i];
        let key = forms::form_key(tok);
        if key.is_empty() {
            i += 1;
            continue;
        }
        // Longest match first: trigram (3-token official lemmas), then bigram
        // (reflexive verbs, two-word lemmas), then the single token.
        let mut consumed = 1;
        let mut matched_key = key.clone();
        let mut recs: Option<&Vec<FormRecord>> = None;
        if let (Some(n1), Some(n2)) = (tokens.get(i + 1), tokens.get(i + 2)) {
            let trigram = format!("{key} {} {}", forms::form_key(n1), forms::form_key(n2));
            if let Some(r) = index.by_key.get(&trigram) {
                recs = Some(r);
                matched_key = trigram;
                consumed = 3;
            }
        }
        if recs.is_none() {
            if let Some(next) = tokens.get(i + 1) {
                let bigram = format!("{key} {}", forms::form_key(next));
                if let Some(r) = index.by_key.get(&bigram) {
                    recs = Some(r);
                    matched_key = bigram;
                    consumed = 2;
                }
            }
        }
        let recs = recs.or_else(|| index.by_key.get(&key));
        // Display echoes the SOURCE spelling so JSON consumers can locate the
        // original text span.
        let display = match consumed {
            3 => format!("{tok} {} {}", tokens[i + 1], tokens[i + 2]),
            2 => format!("{tok} {}", tokens[i + 1]),
            _ => tok.clone(),
        };

        let report = match recs {
            Some(rs) => {
                let mut lemmas: Vec<String> = Vec::new();
                let mut analyses: Vec<String> = Vec::new();
                let mut is_lemma = false;
                let mut official = false;
                let mut probability: Option<f64> = None;
                for r in rs {
                    if !lemmas.contains(&r.lemma) {
                        lemmas.push(r.lemma.clone());
                    }
                    for a in &r.analyses {
                        if !analyses.contains(a) {
                            analyses.push(a.clone());
                        }
                    }
                    if r.source == "lemma" {
                        is_lemma = true;
                    }
                    if r.status != "generated" {
                        official = true;
                    } else if probability.is_none() {
                        probability = r.probability;
                    }
                }
                let status = if !official {
                    "generated"
                } else if is_lemma {
                    "known-lemma"
                } else {
                    "known-form"
                };
                let note = index.notes.get(&matched_key);
                TokenReport {
                    token: display,
                    status,
                    ambiguous: lemmas.len() > 1,
                    lemmas,
                    analyses,
                    probability: if official { None } else { probability },
                    suggestions: Vec::new(),
                    warning: note.map(|n| n.warning.clone()),
                    prefer: note.map(|n| n.prefer.clone()).unwrap_or_default(),
                    agreement: None,
                }
            }
            None => TokenReport {
                token: display,
                status: "unknown",
                lemmas: Vec::new(),
                analyses: Vec::new(),
                ambiguous: false,
                probability: None,
                suggestions: suggest(index, &key),
                warning: None,
                prefer: Vec::new(),
                agreement: None,
            },
        };
        grammar.push(match recs {
            Some(rs) => token_grammar(index, rs, &matched_key),
            None => token_grammar(index, &[], &matched_key),
        });
        report_breaks.push(breaks.map(|b| b[i]).unwrap_or(false));
        reports.push(report);
        i += consumed;
    }
    agreement_pass(index, &grammar, &report_breaks, &mut reports);
    reports
}

/// One parsed analysis: case, number ('j'=sg 'm'=pl), gender ('m','f','n',
/// ' '=unspecified), verb person+number ("3mn"), and degree/participle
/// prefixes are ignored for agreement.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Feat {
    case: &'static str,
    number: char,
    gender: char,
}

const CASES6: [&str; 6] = ["nom", "akuz", "gen", "dat", "lok", "instr"];

/// Parse the compact analysis strings ("gen.jd.", "komp. akuz.mn. m.živ.",
/// "gen.jd. ž. / dat.jd. ž.") into feature tuples.
fn parse_feats(analyses: &[String]) -> Vec<Feat> {
    let mut out = Vec::new();
    for a in analyses {
        // The pronoun tables write masc-or-neuter as "m./sr." — protect it
        // from the '/'-alternative split.
        let a = a.replace("m./sr.", "msr.");
        for part in a.split('/') {
            let mut case: Option<&'static str> = None;
            let mut number = ' ';
            let mut gender = ' ';
            for tok in part.split_whitespace() {
                let t = tok.trim_end_matches('.');
                if let Some((c, n)) = t.split_once('.') {
                    if let Some(cc) = CASES6.iter().find(|x| **x == c) {
                        case = Some(cc);
                        number = match n {
                            "jd" => 'j',
                            "mn" => 'm',
                            _ => ' ',
                        };
                        continue;
                    }
                }
                // Bare case (numeral tables: "gen.")
                if let Some(cc) = CASES6.iter().find(|x| **x == t) {
                    case = Some(cc);
                    continue;
                }
                match t {
                    "m" | "m.živ" | "m.než" => gender = 'm',
                    "ž" => gender = 'f',
                    "sr" => gender = 'n',
                    "msr" => gender = 'b', // masc-or-neuter (pronoun tables)
                    _ => {}
                }
            }
            if let Some(c) = case {
                out.push(Feat {
                    case: c,
                    number,
                    gender,
                });
            }
        }
    }
    out
}

/// Verb present-tense person+number analyses ("prez.3mn." → ('3','m')).
fn parse_prez(analyses: &[String]) -> Vec<(char, char)> {
    let mut out = Vec::new();
    for a in analyses {
        if let Some(rest) = a.strip_prefix("prez.") {
            let mut ch = rest.chars();
            if let (Some(p), Some(n)) = (ch.next(), ch.next()) {
                let n = match n {
                    'j' => 'j',
                    'm' => 'm',
                    _ => ' ',
                };
                out.push((p, n));
            }
        }
    }
    out
}

/// Token-level grammar info extracted for the agreement pass.
struct TokenGrammar {
    /// Adjectival analyses (carry a gender column).
    adj: Vec<Feat>,
    /// Nominal analyses (no gender column) + the lemma's dictionary gender.
    noun: Vec<Feat>,
    noun_gender: char,
    prez: Vec<(char, char)>,
    /// Preposition government (cases), when the token is a known preposition.
    prep: Option<Vec<&'static str>>,
    /// Personal-pronoun subject (person, number), e.g. ja → ('1','j').
    subject: Option<(char, char)>,
    official: bool,
    /// Every record is the given POS — the ambiguity gates for the
    /// conservative agreement checks ('malo' is adj AND adverb: skipped).
    pure_adj: bool,
    pure_noun: bool,
    pure_verb: bool,
}

fn token_grammar(index: &Index, recs: &[FormRecord], matched_key: &str) -> TokenGrammar {
    let mut adj: Vec<Feat> = Vec::new();
    let mut noun: Vec<Feat> = Vec::new();
    let mut prez: Vec<(char, char)> = Vec::new();
    let mut noun_gender = ' ';
    let mut official = false;
    let pure = |p: &str| !recs.is_empty() && recs.iter().all(|r| r.pos == p);
    let (pure_adj, pure_noun, pure_verb) = (pure("adj"), pure("noun"), pure("verb"));
    for r in recs {
        if r.status != "generated" {
            official = true;
        }
        let feats = parse_feats(&r.analyses);
        match r.pos {
            "adj" => adj.extend(feats),
            "noun" => {
                noun.extend(feats);
                if noun_gender == ' ' {
                    if let Some(g) = index.noun_gender.get(&forms::form_key(&r.lemma)) {
                        noun_gender = *g;
                    }
                }
            }
            "verb" => prez.extend(parse_prez(&r.analyses)),
            _ => {}
        }
    }
    let prep = index.prep_cases.get(matched_key).cloned();
    let subject = match matched_key {
        "ja" => Some(('1', 'j')),
        "ty" => Some(('2', 'j')),
        "my" => Some(('1', 'm')),
        "vy" => Some(('2', 'm')),
        "on" | "ona" | "ono" => Some(('3', 'j')),
        "oni" | "one" => Some(('3', 'm')),
        _ => None,
    };
    TokenGrammar {
        adj,
        noun,
        noun_gender,
        prez,
        prep,
        subject,
        official,
        pure_adj,
        pure_noun,
        pure_verb,
    }
}

/// The conservative agreement pass (issue #13 §3): a warning fires ONLY when
/// no combination of the two tokens' analyses is compatible, both tokens are
/// verification-grade, and each is unambiguously the expected part of speech.
fn agreement_pass(
    index: &Index,
    grammar: &[TokenGrammar],
    breaks: &[bool],
    reports: &mut [TokenReport],
) {
    let _ = index;
    for i in 0..reports.len().saturating_sub(1) {
        let (a, b) = (&grammar[i], &grammar[i + 1]);
        if !a.official || !b.official {
            continue;
        }
        // Agreement never crosses punctuation (lists, table cells, clause
        // boundaries): "pomoćnogo, ljudi" is an enumeration, not a phrase.
        if breaks.get(i + 1).copied().unwrap_or(false) {
            continue;
        }
        // Preposition government: prep + unambiguous noun.
        if let Some(cases) = &a.prep {
            if b.pure_noun && !b.noun.is_empty() {
                let ok = b.noun.iter().any(|f| cases.contains(&f.case));
                if !ok {
                    reports[i + 1].agreement = Some(format!(
                        "predlog '{}' vlada padežami [{}], ale '{}' ne imaje ni jednogo takogo padeža",
                        reports[i].token,
                        cases.join(", "),
                        reports[i + 1].token
                    ));
                }
                continue;
            }
        }
        // Adjective + noun agreement (adjacent, both POS-unambiguous). Gender
        // is only distinctive in the SINGULAR — ISV plural adjectives mark
        // only nom-animacy, so plural pairs check case+number alone (and the
        // dictionary genders pluralia like `ljudi` idiosyncratically anyway).
        if a.pure_adj && !a.adj.is_empty() && b.pure_noun && !b.noun.is_empty() {
            let ok = a.adj.iter().any(|fa| {
                b.noun.iter().any(|fn_| {
                    fa.case == fn_.case
                        && (fa.number == fn_.number || fa.number == ' ' || fn_.number == ' ')
                        && (fn_.number == 'm'
                            || b.noun_gender == ' '
                            || fa.gender == ' '
                            || fa.gender == b.noun_gender
                            || (fa.gender == 'b' && matches!(b.noun_gender, 'm' | 'n')))
                })
            });
            if !ok {
                reports[i].agreement = Some(format!(
                    "'{}' ne sųglašaje sę s '{}' v padežu/čislu/rodu (ni jedna kombinacija analiz ne je sųměstna)",
                    reports[i].token,
                    reports[i + 1].token
                ));
            }
            continue;
        }
        // Personal pronoun + present-tense verb: person/number.
        if let Some((p, n)) = a.subject {
            if b.pure_verb && !b.prez.is_empty() {
                let ok = b.prez.iter().any(|(vp, vn)| *vp == p && *vn == n);
                if !ok {
                    reports[i + 1].agreement = Some(format!(
                        "glagol '{}' ne sųglašaje sę s podmetom '{}' v osobě/čislu",
                        reports[i + 1].token,
                        reports[i].token
                    ));
                }
            }
        }
    }
}

/// Nearest known lemmas for an unknown token: same first letter, folded edit
/// distance ≤ 2, closest first, at most 3 (deterministic tie-break by key).
fn suggest(index: &Index, key: &str) -> Vec<String> {
    let first = key.chars().next();
    let mut cands: Vec<(usize, &str)> = index
        .lemma_keys
        .iter()
        .filter(|(k, _)| k.chars().next() == first)
        .filter_map(|(k, lemma)| {
            let d = ortho::levenshtein(k, key);
            (d <= 2).then_some((d, lemma.as_str()))
        })
        .collect();
    cands.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(b.1)));
    cands
        .into_iter()
        .take(3)
        .map(|(_, l)| l.to_string())
        .collect()
}

fn json_escape(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// The `check-text` CLI entry point.
pub fn run(official_path: &Path, text_path: &Path, json: bool) -> Result<()> {
    let entries = official::load(official_path)?;
    let index = build_index(&entries, Some(Path::new("data/novel-words.tsv")));
    let text = std::fs::read_to_string(text_path)?;
    let reports = check_text(&index, &text);

    if json {
        let mut s = String::from("[\n");
        for (i, r) in reports.iter().enumerate() {
            if i > 0 {
                s.push_str(",\n");
            }
            let _ = write!(s, "{}", serde_json::to_string(r)?);
        }
        s.push_str("\n]\n");
        println!("{s}");
        return Ok(());
    }

    let n = reports.len();
    let count = |st: &str| reports.iter().filter(|r| r.status == st).count();
    println!(
        "check-text: {n} tokens — {} known-lemma, {} known-form, {} generated, {} unknown",
        count("known-lemma"),
        count("known-form"),
        count("generated"),
        count("unknown")
    );
    for r in &reports {
        match r.status {
            "unknown" => {
                println!(
                    "  ? {:<20} unknown{}",
                    r.token,
                    if r.suggestions.is_empty() {
                        String::new()
                    } else {
                        format!("  → nearest: {}", r.suggestions.join(", "))
                    }
                );
            }
            "generated" => {
                println!(
                    "  ~ {:<20} generated (p={}) — machine reconstruction, not official",
                    r.token,
                    r.probability
                        .map(|p| format!("{p:.2}"))
                        .unwrap_or_else(|| "?".into())
                );
            }
            _ => {}
        }
        if let Some(w) = &r.agreement {
            println!("  ⚠ {:<20} {}", r.token, w);
        }
        if let Some(w) = &r.warning {
            println!(
                "  ! {:<20} {}{}",
                r.token,
                w,
                if r.prefer.is_empty() {
                    String::new()
                } else {
                    format!(" Prefer: {}.", r.prefer.join(", "))
                }
            );
        }
    }
    let _ = json_escape("");
    Ok(())
}

/// The check-text benchmark (issue #13, `checktext-eval`): classification
/// counts on the committed fixture (all-correct Interslavic → must be fully
/// known with zero agreement flags), plus the agreement gold/error sets.
/// Report: target/eval/checktext-report.md.
pub const FIXTURE: &str = "data/checktext-fixture.txt";

/// Grammatically correct sentences: zero agreement warnings expected.
pub const AGREEMENT_GOLD: &[&str] = &[
    "Toj dobry člověk čitaje najlěpšu knigu.",
    "Ja vidžų velikų rěku bez mosta.",
    "My pijemo čistu vodu s prijateljami.",
    "Ona pisala oba pisma za pęť minut.",
    "Dobri ljudi pomagajųt vsim dětam.",
];

/// Each sentence seeds exactly one agreement error that MUST be flagged.
pub const AGREEMENT_ERRORS: &[&str] = &[
    "Ja vidiš rěku.",           // person: ja + 2sg verb
    "Bez voda ne možemo žiti.", // government: bez + nominative
    "Vidimo velikogo ženu.",    // gender: masc-anim acc adj + fem noun
    "K vodu idemo.",            // government: k + accusative
];

/// A nonsense verb form: must stay `unknown` (tests unknown-handling, never
/// an agreement flag).
pub const UNKNOWN_PROBE: &str = "On vidita rěku.";

pub fn run_eval(official_path: &Path, out_dir: &Path) -> Result<()> {
    use std::fmt::Write as _;
    let entries = official::load(official_path)?;
    let index = build_index(&entries, Some(Path::new("data/novel-words.tsv")));

    let text = std::fs::read_to_string(FIXTURE)?;
    let reps = check_text(&index, &text);
    let count = |st: &str| reps.iter().filter(|r| r.status == st).count();
    let unknown = count("unknown");
    let agree_flags = reps.iter().filter(|r| r.agreement.is_some()).count();

    let gold_flags: usize = AGREEMENT_GOLD
        .iter()
        .map(|s| {
            check_tokens(&index, &tokenize(s))
                .iter()
                .filter(|r| r.agreement.is_some())
                .count()
        })
        .sum();
    let error_hits: Vec<bool> = AGREEMENT_ERRORS
        .iter()
        .map(|s| check_text(&index, s).iter().any(|r| r.agreement.is_some()))
        .collect();
    let errors_flagged = error_hits.iter().filter(|x| **x).count();
    let probe_unknown = check_text(&index, UNKNOWN_PROBE)
        .iter()
        .any(|r| r.status == "unknown");

    println!(
        "checktext-eval: fixture {} tokens — {} known-lemma, {} known-form, {} generated, {} unknown, {} agreement flags",
        reps.len(),
        count("known-lemma"),
        count("known-form"),
        count("generated"),
        unknown,
        agree_flags,
    );
    println!(
        "  agreement: gold sentences {} false flags / seeded errors {}/{} flagged / unknown-probe {}",
        gold_flags,
        errors_flagged,
        error_hits.len(),
        if probe_unknown { "ok" } else { "FAILED" }
    );

    std::fs::create_dir_all(out_dir)?;
    let mut s = String::new();
    writeln!(s, "# check-text benchmark (checktext-eval)\n")?;
    writeln!(
        s,
        "**Denominators:** the committed all-correct fixture `{FIXTURE}` ({} tokens), {} gold sentences (agreement false-alarm set) and {} seeded-error sentences. **Leakage story:** the fixture and sentence sets are hand-written against the official vocabulary; the checker never sees expected labels.\n",
        reps.len(),
        AGREEMENT_GOLD.len(),
        error_hits.len(),
    )?;
    writeln!(s, "| Measurement | value |")?;
    writeln!(s, "|---|---:|")?;
    writeln!(
        s,
        "| fixture classification | {} known-lemma / {} known-form / {} generated / **{} unknown** |",
        count("known-lemma"),
        count("known-form"),
        count("generated"),
        unknown
    )?;
    writeln!(s, "| fixture agreement false alarms | **{agree_flags}** |")?;
    writeln!(s, "| gold-sentence false alarms | **{gold_flags}** |")?;
    writeln!(
        s,
        "| seeded errors flagged | **{errors_flagged} / {}** |",
        error_hits.len()
    )?;
    writeln!(
        s,
        "| nonsense probe stays unknown | **{}** |",
        if probe_unknown { "yes" } else { "NO" }
    )?;
    writeln!(
        s,
        "\nAgreement checks are deliberately conservative: they fire only when NO combination of the neighbouring tokens' analyses is compatible, both tokens are verification-grade, and each token is POS-unambiguous. Gender is enforced in the singular only (ISV plural adjectives mark nom-animacy only)."
    )?;
    std::fs::write(out_dir.join("checktext-report.md"), s)?;
    println!("Wrote {}", out_dir.join("checktext-report.md").display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_index() -> Index {
        // Every 20th benchmarkable entry keeps the test fast while still
        // exercising thousands of paradigm cells.
        let entries: Vec<OfficialEntry> = official::load(Path::new(crate::DEFAULT_OFFICIAL))
            .expect("official csv")
            .into_iter()
            .step_by(20)
            .collect();
        build_index(&entries, None)
    }

    #[test]
    fn self_verification_official_lemmas_are_known() {
        // Acceptance criterion (issue #11): every official lemma in the index
        // resolves as known; every sampled paradigm cell resolves as a known
        // form; garbage resolves as unknown.
        let entries: Vec<OfficialEntry> = official::load(Path::new(crate::DEFAULT_OFFICIAL))
            .expect("official csv")
            .into_iter()
            .step_by(20)
            .collect();
        let index = build_index(&entries, None);
        let mut checked = 0usize;
        for e in &entries {
            let isv = e.isv.trim();
            if isv.is_empty() || isv.contains('#') || isv.contains('!') || isv.contains(' ') {
                continue;
            }
            let toks = tokenize(isv);
            let reps = check_tokens(&index, &toks);
            assert!(
                reps.iter()
                    .all(|r| r.status == "known-lemma" || r.status == "known-form"),
                "official lemma '{isv}' not recognized: {:?}",
                reps.iter().map(|r| r.status).collect::<Vec<_>>()
            );
            checked += 1;
        }
        assert!(checked > 300, "sample too small: {checked}");

        // Sampled paradigm cells resolve as known forms.
        for e in entries.iter().filter(|e| e.pos == Pos::Noun).take(30) {
            let isv = e.isv.trim();
            if isv.is_empty() || isv.contains(' ') || isv.contains('#') {
                continue;
            }
            let gen =
                crate::forms::noun_cell(isv, interslavic::Case::Gen, interslavic::Number::Singular);
            for v in gen.split('/') {
                let v = v.trim();
                if v.is_empty() || v == "—" {
                    continue;
                }
                let reps = check_tokens(&index, &tokenize(v));
                assert!(
                    reps.iter().all(|r| r.status != "unknown"),
                    "paradigm cell '{v}' of '{isv}' unknown"
                );
            }
        }

        // Negative control.
        let reps = check_tokens(&index, &tokenize("xqzvw grblfk"));
        assert!(reps.iter().all(|r| r.status == "unknown"));
        assert_eq!(reps.len(), 2);
    }

    #[test]
    fn fixture_and_agreement_benchmark_hold() {
        // The acceptance criteria of issue #13, enforced in CI: the committed
        // all-correct fixture stays fully known with zero agreement false
        // alarms, the gold sentences stay clean, and every seeded error is
        // flagged. If a change legitimately shifts these numbers, update the
        // fixture/report — that is the change-detector working.
        let entries = official::load(Path::new(crate::DEFAULT_OFFICIAL)).expect("official csv");
        let index = build_index(&entries, None);
        let text = std::fs::read_to_string(FIXTURE).expect("fixture");
        let reps = check_text(&index, &text);
        let unknown: Vec<&str> = reps
            .iter()
            .filter(|r| r.status == "unknown")
            .map(|r| r.token.as_str())
            .collect();
        assert!(unknown.is_empty(), "fixture unknowns: {unknown:?}");
        let flags: Vec<&TokenReport> = reps.iter().filter(|r| r.agreement.is_some()).collect();
        assert!(
            flags.is_empty(),
            "fixture agreement false alarms: {:?}",
            flags.iter().map(|r| &r.token).collect::<Vec<_>>()
        );
        for s in AGREEMENT_GOLD {
            let reps = check_text(&index, s);
            assert!(
                reps.iter().all(|r| r.agreement.is_none()),
                "gold sentence falsely flagged: {s}"
            );
        }
        for s in AGREEMENT_ERRORS {
            let reps = check_text(&index, s);
            assert!(
                reps.iter().any(|r| r.agreement.is_some()),
                "seeded error NOT flagged: {s}"
            );
        }
        // The nonsense probe stays unknown and never trips agreement.
        let reps = check_text(&index, UNKNOWN_PROBE);
        assert!(reps.iter().any(|r| r.status == "unknown"));
        assert!(reps.iter().all(|r| r.agreement.is_none()));
    }

    #[test]
    fn feature_parsing_and_compatibility() {
        let adj = parse_feats(&["nom.mn. m.živ.".to_string()]);
        assert_eq!(adj.len(), 1, "{adj:?}");
        assert_eq!(
            (adj[0].case, adj[0].number, adj[0].gender),
            ("nom", 'm', 'm')
        );
        let noun = parse_feats(&["nom.mn.".to_string(), "gen.jd.".to_string()]);
        assert_eq!(noun.len(), 2, "{noun:?}");
        assert_eq!(
            (noun[0].case, noun[0].number, noun[0].gender),
            ("nom", 'm', ' ')
        );
        let multi = parse_feats(&["gen.jd. ž. / dat.jd. ž.".to_string()]);
        assert_eq!(multi.len(), 2, "{multi:?}");
        let prez = parse_prez(&["prez.3mn.".to_string()]);
        assert_eq!(prez, vec![('3', 'm')]);
    }

    #[test]
    fn reflexive_bigram_lookup() {
        let index = sample_index();
        // Find a reflexive lemma in the sampled index to test against.
        let Some((key, _)) = index.lemma_keys.iter().find(|(k, _)| k.ends_with(" se")) else {
            return; // sample contained no reflexive lemma; covered by full runs
        };
        let stem = key.strip_suffix(" se").unwrap();
        let toks = vec![stem.to_string(), "sę".to_string()];
        let reps = check_tokens(&index, &toks);
        assert_eq!(reps.len(), 1, "bigram must consume both tokens");
        assert_ne!(reps[0].status, "unknown");
    }
}
