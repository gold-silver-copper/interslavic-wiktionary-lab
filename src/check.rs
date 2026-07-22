//! Text verification against the lexicon (issue #11 phase 3, `check-text`).
//!
//! Builds the same [`crate::forms::FormRecord`] index the site's API exports
//! (official lemmas + their full paradigms, plus generated proposal lemmas when
//! `data/novel-words.tsv` contains rows from a compatible calibrator), then
//! tokenizes the input and classifies every token:
//!
//! `known-lemma` / `known-form` / `generated` (carries p when proposals are enabled) / `unknown` (with
//! nearest-lemma suggestions) — plus computed false-friend warnings from
//! [`crate::falsefriends`]. Two-token keys (reflexive `X sę` verbs and
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

pub const SUGGEST_SHARDS: u32 = 64;
pub const SUGGEST_SELFTEST_INPUTS: &[&str] = &["domm", "pomocnyy", "rěkaa", "xyzq"];
const SUGGEST_SELFTEST_ROWS: &[(&str, &str)] = &[
    ("dom", "dom"),
    ("doma", "doma"),
    ("don", "Don"),
    ("pomočny", "pomoćny"),
    ("rěka", "rěka"),
    ("reklama", "reklama"),
    ("rešta", "rešta"),
];

pub struct Index {
    /// key → records (lemma citations and inflected forms).
    pub by_key: HashMap<String, Vec<FormRecord>>,
    /// All lemma keys, for nearest-suggestion search.
    pub lemma_keys: Vec<(String, String)>, // (key, display lemma)
    /// Computed false-friend notes, keyed by folded form (see
    /// [`crate::falsefriends::compute`]).
    pub notes: BTreeMap<String, crate::falsefriends::Note>,
    /// Noun lemma key → dictionary gender (m/f/n), for agreement checking.
    pub noun_gender: HashMap<String, char>,
    /// Preposition key (folded) → the cases it governs, sourced from the
    /// interslavic crate's curated `prepositions::PREPOSITIONS` table (which
    /// encodes the community dictionary's `(+N)` government, instrumental = +5).
    pub prep_cases: HashMap<String, Vec<&'static str>>,
    /// Project-lexicon rows (V13 item 1), in file order; empty when no
    /// `--lexicon` was supplied. Drives the consistency check.
    pub lexicon: Vec<LexiconRow>,
}

/// Build the verification index from the official dictionary (lemmas + full
/// paradigms) and, when present, the committed novel-word proposals (lemma
/// records with their calibrated probability). `notes` carries the computed
/// false-friend warnings (empty map to disable them, e.g. in unit tests).
pub fn build_index(
    entries: &[OfficialEntry],
    novel_words_tsv: Option<&Path>,
    notes: BTreeMap<String, crate::falsefriends::Note>,
) -> Index {
    let mut sink = RecordSink::default();
    forms::closed_class_records(&mut sink);
    let mut seen: HashSet<String> = HashSet::new();
    let mut noun_gender: HashMap<String, char> = HashMap::new();
    // Preposition government comes from the same shared table used by entry
    // rendering; no site-only copy may drift from checker behavior.
    let prep_cases = preposition_government();
    for e in entries {
        // ~230 rows list byform variants in one cell ("iměti, imati",
        // "srědnji, srědny") — each variant is its own lemma.
        for byform in e.citation_byforms() {
            let e = byform.entry;
            let isv = byform.form.as_str();
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
                        // A spelling can name nouns of different genders
                        // (`družba` friendship f. / best man m.). Agreement must
                        // abstain instead of letting CSV order choose a gender
                        // for every homograph (issue #89 B04/G12).
                        noun_gender
                            .entry(forms::form_key(isv))
                            .and_modify(|known| {
                                // Once conflicting senses make the key
                                // ambiguous, later rows must not restore a
                                // concrete gender.
                                if *known != ' ' && *known != c {
                                    *known = ' ';
                                }
                            })
                            .or_insert(c);
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
        // The proposals file is a committed export artifact (`export` refreshes
        // it). Rows carry the corpus-coverage calibrator's probability (V11
        // item 5); a missing file is a separate reproducibility warning.
        let tsv = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!(
                "warning: generated-word proposal artifact unavailable ({}: {e}); \
                 generated corpus words will be classified as unknown",
                path.display()
            );
            String::new()
        });
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
    Index {
        by_key,
        lemma_keys,
        notes,
        noun_gender,
        prep_cases,
        lexicon: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Project lexicon (V13 item 1): a translation project's sanctioned coinages.
// ---------------------------------------------------------------------------

/// The documented column order of a project-lexicon TSV row.
pub const LEXICON_COLUMNS: &str = "lemma\tpos\tgender\tanimacy\tgloss";

/// One sanctioned project word: a coinage that passed `coin-check`, or an
/// official word the project pins for a source concept. `coin-check
/// --lexicon-row` emits exactly this row shape, so the coinage workflow
/// chains mechanically: `coin-check → append row → check-text --lexicon`.
#[derive(Debug, Clone)]
pub struct LexiconRow {
    pub lemma: String,
    /// Folded lookup key of `lemma` ([`forms::form_key`]).
    pub lemma_key: String,
    /// noun | adj | verb only — the POS classes the paradigm machinery
    /// declines in full.
    pub pos: Pos,
    /// Required for nouns (explicit `ISV::noun_with` control), forbidden
    /// otherwise.
    pub gender: Option<crate::model::Gender>,
    pub animate: bool,
    /// English gloss of the source concept — drives the consistency check.
    pub gloss: String,
    /// Content tokens of `gloss`, normalized exactly as the English API
    /// normalizes gloss keys ([`crate::site::english_gloss_tokens`]).
    pub gloss_tokens: std::collections::BTreeSet<String>,
}

/// Parse a project-lexicon TSV (`lemma  pos  gender  animacy  gloss`; blank
/// lines and `#` comments skipped). Syntax errors are hard errors — a broken
/// lexicon must not silently weaken the `check-text` gate.
pub fn parse_lexicon(text: &str) -> Result<Vec<LexiconRow>> {
    let mut rows: Vec<LexiconRow> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (idx, raw) in text.lines().enumerate() {
        let n = idx + 1;
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        anyhow::ensure!(
            cols.len() == 5,
            "lexicon line {n}: expected 5 tab-separated columns ({}), got {}",
            LEXICON_COLUMNS.replace('\t', " "),
            cols.len()
        );
        let (lemma, pos_raw, gender_raw, animacy_raw, gloss) = (
            cols[0].trim(),
            cols[1].trim(),
            cols[2].trim(),
            cols[3].trim(),
            cols[4].trim(),
        );
        anyhow::ensure!(
            !lemma.is_empty() && !lemma.contains(' '),
            "lexicon line {n}: lemma must be one non-empty token, got '{lemma}'"
        );
        let pos = match pos_raw {
            "noun" => Pos::Noun,
            "adj" => Pos::Adjective,
            "verb" => Pos::Verb,
            other => anyhow::bail!("lexicon line {n}: pos must be noun|adj|verb, got '{other}'"),
        };
        let gender = match gender_raw {
            "" => None,
            "m" => Some(crate::model::Gender::Masculine),
            "f" => Some(crate::model::Gender::Feminine),
            "n" => Some(crate::model::Gender::Neuter),
            other => {
                anyhow::bail!("lexicon line {n}: gender must be m|f|n or blank, got '{other}'")
            }
        };
        let animate = match animacy_raw {
            "" => None,
            "anim" => Some(true),
            "inanim" => Some(false),
            other => {
                anyhow::bail!(
                    "lexicon line {n}: animacy must be anim|inanim or blank, got '{other}'"
                )
            }
        };
        if pos == Pos::Noun {
            // A project lexicon exists to control the paradigm explicitly
            // (`ISV::noun_with`); a guessed gender would silently weaken it.
            anyhow::ensure!(
                gender.is_some(),
                "lexicon line {n}: nouns must declare gender (m|f|n)"
            );
            anyhow::ensure!(
                animate.is_some(),
                "lexicon line {n}: nouns must declare animacy (anim|inanim)"
            );
        } else {
            anyhow::ensure!(
                gender.is_none() && animate.is_none(),
                "lexicon line {n}: gender/animacy apply to nouns only"
            );
        }
        anyhow::ensure!(
            !gloss.is_empty(),
            "lexicon line {n}: gloss must be non-empty (it drives the consistency check)"
        );
        let lemma_key = forms::form_key(lemma);
        anyhow::ensure!(
            !lemma_key.is_empty(),
            "lexicon line {n}: lemma folds to nothing"
        );
        anyhow::ensure!(
            seen.insert(lemma_key.clone()),
            "lexicon line {n}: duplicate lemma '{lemma}' (folded '{lemma_key}')"
        );
        rows.push(LexiconRow {
            lemma: lemma.to_string(),
            lemma_key,
            pos,
            gender,
            animate: animate.unwrap_or(false),
            gloss: gloss.to_string(),
            gloss_tokens: crate::site::english_gloss_tokens(gloss),
        });
    }
    Ok(rows)
}

/// Validate one lexicon row against the built index: the row must pass
/// coin-check's collision axis (no existing lemma or inflected form under its
/// key) OR pin an official lemma whose POS/gender agree with the declaration.
/// The crate's citation-form requirements (verbs cite `-ti`, adjectives
/// `-y`/`-i`, nouns must be declinable) are enforced too. Returns `true` when
/// the row pins an official word (its paradigm is already indexed).
pub fn validate_lexicon_row(index: &Index, row: &LexiconRow) -> Result<bool> {
    match row.pos {
        Pos::Verb => {
            anyhow::ensure!(
                row.lemma_key.ends_with("ti"),
                "lexicon lemma '{}': verbs must cite the -ti infinitive",
                row.lemma
            );
            anyhow::ensure!(
                forms::verb_cells(&row.lemma, false).is_some(),
                "lexicon lemma '{}': the inflector cannot conjugate this stem",
                row.lemma
            );
        }
        Pos::Adjective => {
            anyhow::ensure!(
                row.lemma_key.ends_with(['y', 'i']),
                "lexicon lemma '{}': adjectives must cite the -y/-i masculine form",
                row.lemma
            );
        }
        Pos::Noun => {
            let declinable = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                forms::noun_paradigm_forms_with_animacy(&row.lemma, row.gender, row.animate)
            }))
            .is_ok();
            anyhow::ensure!(
                declinable,
                "lexicon lemma '{}': the inflector cannot decline this noun",
                row.lemma
            );
        }
        _ => anyhow::bail!("lexicon lemma '{}': unsupported POS", row.lemma),
    }
    let Some(recs) = index.by_key.get(&row.lemma_key) else {
        return Ok(false); // clean coinage: no collision
    };
    let pins: Vec<&FormRecord> = recs
        .iter()
        .filter(|r| {
            matches!(r.status, "official" | "official-only" | "grammar")
                && r.source == "lemma"
                && forms::form_key(&r.lemma) == row.lemma_key
        })
        .collect();
    if pins.is_empty() {
        let c = &recs[0];
        anyhow::bail!(
            "lexicon lemma '{}' collides with existing {} {} of '{}' ({}) — run coin-check and choose another surface",
            row.lemma,
            c.status,
            if c.source == "lemma" { "lemma" } else { "inflected form" },
            c.lemma,
            c.pos
        );
    }
    // Official pin: declared metadata must not contradict the dictionary.
    anyhow::ensure!(
        pins.iter().any(|r| r.pos == row.pos.code()),
        "lexicon lemma '{}' pins official '{}' but declares pos '{}' while the official word is '{}'",
        row.lemma,
        pins[0].lemma,
        row.pos.code(),
        pins[0].pos
    );
    if row.pos == Pos::Noun {
        if let Some(&dict) = index.noun_gender.get(&row.lemma_key) {
            let declared = gender_char(row.gender);
            anyhow::ensure!(
                dict == ' ' || declared == dict,
                "lexicon lemma '{}' declares gender '{declared}' but the dictionary says '{dict}'",
                row.lemma
            );
        }
    }
    Ok(true)
}

fn gender_char(g: Option<crate::model::Gender>) -> char {
    match g {
        Some(crate::model::Gender::Masculine) => 'm',
        Some(crate::model::Gender::Feminine) => 'f',
        Some(crate::model::Gender::Neuter) => 'n',
        _ => ' ',
    }
}

/// Validate every row and index the coinages' full paradigms (status
/// `project`), exactly as the official paradigms are indexed — so inflected
/// sanctioned coinages (`žabervoka`, `žabervokom`) classify instead of
/// drowning the `--max-unknown` gate. Any invalid row is a hard error.
pub fn apply_lexicon(index: &mut Index, rows: Vec<LexiconRow>) -> Result<()> {
    for row in &rows {
        let pinned = validate_lexicon_row(index, row)?;
        if pinned {
            continue; // the official paradigm is already indexed
        }
        let mut sink = RecordSink::default();
        sink.add(
            &row.lemma,
            "",
            &row.lemma,
            0,
            row.pos.code(),
            "lemma",
            "project",
            None,
            &row.gloss,
        );
        forms::project_paradigm_records(
            &mut sink,
            &row.lemma,
            row.pos,
            row.gender,
            row.animate,
            "project",
            &row.gloss,
        );
        for r in sink.into_records() {
            if r.source == "lemma" && r.key == row.lemma_key {
                index.lemma_keys.push((r.key.clone(), r.lemma.clone()));
            }
            index.by_key.entry(r.key.clone()).or_default().push(r);
        }
        // Declared gender feeds the agreement checker like a dictionary
        // gender does (same absorbing homograph logic).
        let c = gender_char(row.gender);
        if row.pos == Pos::Noun && c != ' ' {
            index
                .noun_gender
                .entry(row.lemma_key.clone())
                .and_modify(|known| {
                    if *known != ' ' && *known != c {
                        *known = ' ';
                    }
                })
                .or_insert(c);
        }
    }
    index.lemma_keys.sort();
    index.lexicon = rows;
    Ok(())
}

#[derive(Debug, serde::Serialize)]
pub struct TokenReport {
    pub token: String,
    /// known-lemma | known-form | project | generated | unknown
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
    /// Computed false-friend warning, if any.
    pub warning: Option<String>,
    /// Warning severity (`high`/`medium` = primary-sense trap, `low` =
    /// colloquial-only), present iff `warning` is.
    pub severity: Option<String>,
    pub prefer: Vec<String>,
    /// Grammar-agreement warning (issue #13 §3): set when NO combination of
    /// this token's analyses is compatible with its neighbour.
    pub agreement: Option<String>,
    /// Project-lexicon consistency warning (V13 item 1): this token is a
    /// verification-grade official word whose gloss overlaps a lexicon row
    /// that maps the concept to a DIFFERENT lemma — register drift.
    pub consistency: Option<String>,
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
                let mut project = false;
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
                    match r.status {
                        "generated" => {
                            if probability.is_none() {
                                probability = r.probability;
                            }
                        }
                        "project" => project = true,
                        _ => official = true,
                    }
                }
                let status = if official {
                    if is_lemma {
                        "known-lemma"
                    } else {
                        "known-form"
                    }
                } else if project {
                    "project"
                } else {
                    "generated"
                };
                let note = index.notes.get(&matched_key);
                TokenReport {
                    consistency: consistency_warning(index, rs, &display),
                    token: display,
                    status,
                    ambiguous: lemmas.len() > 1,
                    lemmas,
                    analyses,
                    probability: if official || project {
                        None
                    } else {
                        probability
                    },
                    suggestions: Vec::new(),
                    warning: note.map(|n| n.warning.clone()),
                    severity: note.map(|n| n.severity.to_string()),
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
                severity: None,
                prefer: Vec::new(),
                agreement: None,
                consistency: None,
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

/// The project-lexicon consistency check (V13 item 1): when a token resolves
/// to a verification-grade OFFICIAL word whose English gloss overlaps the
/// gloss of some lexicon row, but the token's lemma is not that row's lemma,
/// the same source concept is being rendered by two different target words —
/// register drift across a large text. Deterministic gloss-token overlap,
/// same normalization as the English API; fires on the first matching row in
/// file order.
fn consistency_warning(index: &Index, rs: &[FormRecord], display: &str) -> Option<String> {
    if index.lexicon.is_empty() {
        return None;
    }
    let official: Vec<&FormRecord> = rs
        .iter()
        .filter(|r| matches!(r.status, "official" | "official-only"))
        .collect();
    if official.is_empty() {
        return None;
    }
    let lemma_keys: HashSet<String> = official.iter().map(|r| forms::form_key(&r.lemma)).collect();
    // Tokenize each official gloss ONCE per token — the sets are row-loop
    // invariant, and a large text × large lexicon multiplies this cost.
    let record_tokens: Vec<std::collections::BTreeSet<String>> = official
        .iter()
        .map(|r| crate::site::english_gloss_tokens(&r.gloss))
        .collect();
    for row in &index.lexicon {
        if lemma_keys.contains(&row.lemma_key) {
            continue; // the token IS the project's choice for this concept
        }
        for (r, tokens) in official.iter().zip(&record_tokens) {
            let shared: Vec<&str> = tokens
                .intersection(&row.gloss_tokens)
                .map(String::as_str)
                .collect();
            if !shared.is_empty() {
                return Some(format!(
                    "text uses '{display}' (official '{}'), but the project lexicon maps '{}' to '{}'",
                    r.lemma,
                    shared.join("', '"),
                    row.lemma
                ));
            }
        }
    }
    None
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
        // DELIBERATE (V13): `project` records count as verification-grade
        // here, so sanctioned coinages participate in the conservative
        // agreement checks like official words do — their paradigms are
        // explicit project decisions, and apply_lexicon feeds their declared
        // genders into `noun_gender`. Only `generated` stays excluded.
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

/// Normalized preposition government from the interslavic crate's curated
/// `(+N)` table. This is the sole adapter used by checking and site rendering.
pub fn preposition_government() -> HashMap<String, Vec<&'static str>> {
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
    let mut out: HashMap<String, Vec<&'static str>> = HashMap::new();
    for (prep, cases) in interslavic::prepositions::PREPOSITIONS {
        let entry = out.entry(forms::form_key(prep)).or_default();
        for case in cases.iter().map(|case| case_label(*case)) {
            if !entry.contains(&case) {
                entry.push(case);
            }
        }
    }
    out
}

/// Nearest known lemmas for an unknown token: same first letter, folded edit
/// distance ≤ 2, closest first, at most 3 (deterministic tie-break by lemma).
pub fn suggest(index: &Index, key: &str) -> Vec<String> {
    suggest_rows(&index.lemma_keys, key)
}

fn suggest_rows(rows: &[(String, String)], key: &str) -> Vec<String> {
    let first = key.chars().next();
    let mut cands: Vec<(usize, &str)> = rows
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

/// Write the browser's compact lemma-suggestion shards and a Rust-produced
/// fixture that the JavaScript algorithm checks before showing suggestions.
pub fn write_web_suggestions(out_dir: &Path, index: &Index) -> Result<usize> {
    let dir = out_dir.join("api/suggest");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let mut shards: BTreeMap<u32, Vec<&(String, String)>> = BTreeMap::new();
    for row in &index.lemma_keys {
        let first = row
            .0
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_default();
        let shard = forms::fnv1a32(&first) % SUGGEST_SHARDS;
        shards.entry(shard).or_default().push(row);
    }
    let mut bytes = 0;
    for shard in 0..SUGGEST_SHARDS {
        let rows = shards.get(&shard).map(Vec::as_slice).unwrap_or(&[]);
        let body = format!(
            "{{\"shard\":{shard},\"rows\":[{}]}}\n",
            rows.iter()
                .map(|(key, lemma)| format!("[{},{}]", json_escape(key), json_escape(lemma)))
                .collect::<Vec<_>>()
                .join(",")
        );
        bytes += body.len();
        std::fs::write(dir.join(format!("{shard}.json")), body)?;
    }
    let fixture_rows: Vec<(String, String)> = SUGGEST_SELFTEST_ROWS
        .iter()
        .map(|(key, lemma)| (forms::form_key(key), lemma.to_string()))
        .collect();
    let fixture = format!(
        "{{\"shards\":{SUGGEST_SHARDS},\"rows\":[{}],\"samples\":[{}]}}\n",
        fixture_rows
            .iter()
            .map(|(key, lemma)| format!("[{},{}]", json_escape(key), json_escape(lemma)))
            .collect::<Vec<_>>()
            .join(","),
        SUGGEST_SELFTEST_INPUTS
            .iter()
            .map(|input| {
                let key = forms::form_key(input);
                format!(
                    "[{},[{}]]",
                    json_escape(input),
                    suggest_rows(&fixture_rows, &key)
                        .iter()
                        .map(|value| json_escape(value))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    );
    bytes += fixture.len();
    std::fs::write(out_dir.join("api/suggest-selftest.json"), fixture)?;
    Ok(bytes)
}

fn json_escape(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Gate thresholds for `check-text --summary` (issue: downstream projects
/// want "all rendered messages verify clean" as a CI test).
#[derive(Debug, Clone, Copy)]
pub struct SummaryGate {
    /// Maximum allowed `unknown` tokens before a nonzero exit.
    pub max_unknown: usize,
    /// Maximum allowed agreement warnings before a nonzero exit.
    pub max_agreement: usize,
    /// When set, maximum allowed SEVERE false-friend warnings (severity
    /// `high` or `medium` — the word's primary sense diverges) before a
    /// nonzero exit. None = false-friend warnings never gate.
    pub max_severe_warnings: Option<usize>,
    /// When set, maximum allowed project-lexicon consistency warnings
    /// (V13 item 1) before a nonzero exit. None = never gate.
    pub max_consistency: Option<usize>,
}

/// Deterministic summary of one check-text run.
#[derive(Debug, serde::Serialize)]
pub struct Summary {
    pub tokens: usize,
    pub known_lemma: usize,
    pub known_form: usize,
    pub generated: usize,
    /// Sanctioned project-lexicon words and their inflections (V13 item 1) —
    /// counted separately, never against `--max-unknown`.
    pub project: usize,
    pub unknown: usize,
    pub agreement_errors: usize,
    pub false_friend_warnings: usize,
    /// Warnings with severity `high`/`medium` (primary-sense traps).
    pub severe_warnings: usize,
    /// Project-lexicon consistency warnings (register drift).
    pub consistency_warnings: usize,
    pub passed: bool,
}

pub fn summarize(reports: &[TokenReport], gate: SummaryGate) -> Summary {
    let count = |st: &str| reports.iter().filter(|r| r.status == st).count();
    let unknown = count("unknown");
    let agreement_errors = reports.iter().filter(|r| r.agreement.is_some()).count();
    let severe_warnings = reports
        .iter()
        .filter(|r| matches!(r.severity.as_deref(), Some("high" | "medium")))
        .count();
    let consistency_warnings = reports.iter().filter(|r| r.consistency.is_some()).count();
    Summary {
        tokens: reports.len(),
        known_lemma: count("known-lemma"),
        known_form: count("known-form"),
        generated: count("generated"),
        project: count("project"),
        unknown,
        agreement_errors,
        false_friend_warnings: reports.iter().filter(|r| r.warning.is_some()).count(),
        severe_warnings,
        consistency_warnings,
        passed: unknown <= gate.max_unknown
            && agreement_errors <= gate.max_agreement
            && gate
                .max_severe_warnings
                .is_none_or(|max| severe_warnings <= max)
            && gate
                .max_consistency
                .is_none_or(|max| consistency_warnings <= max),
    }
}

/// The `check-text` CLI entry point. With `gate` set (`--summary`), a summary
/// is emitted and the process exits nonzero when the text fails the gate.
/// `warnings: false` skips the false-friend computation entirely (it loads
/// both evidence caches, ~2-3s — pure classification gates don't need it).
pub fn run(
    official_path: &Path,
    text_path: &Path,
    lexicon: Option<&Path>,
    json: bool,
    gate: Option<SummaryGate>,
    warnings: bool,
) -> Result<()> {
    let entries = official::load(official_path)?;
    let notes = if warnings {
        crate::falsefriends::compute_from_default_caches(&entries)
    } else {
        BTreeMap::new()
    };
    let mut index = build_index(&entries, Some(Path::new("data/novel-words.tsv")), notes);
    if let Some(path) = lexicon {
        let rows = parse_lexicon(&std::fs::read_to_string(path)?)?;
        apply_lexicon(&mut index, rows)?;
    }
    let index = index;
    let text = std::fs::read_to_string(text_path)?;
    let reports = check_text(&index, &text);
    let summary = gate.map(|g| summarize(&reports, g));

    if json {
        let mut s = String::from("[\n");
        for (i, r) in reports.iter().enumerate() {
            if i > 0 {
                s.push_str(",\n");
            }
            let _ = write!(s, "{}", serde_json::to_string(r)?);
        }
        s.push_str("\n]\n");
        match &summary {
            // --json --summary: an object with the token array AND the
            // summary, so agents get both in one parse.
            Some(summary) => println!(
                "{{\"tokens\":{},\"summary\":{}}}",
                s.trim_end(),
                serde_json::to_string(summary)?
            ),
            None => println!("{s}"),
        }
        return fail_gate_if_needed(summary);
    }

    let n = reports.len();
    let count = |st: &str| reports.iter().filter(|r| r.status == st).count();
    println!(
        "check-text: {n} tokens — {} known-lemma, {} known-form, {} project, {} generated, {} unknown",
        count("known-lemma"),
        count("known-form"),
        count("project"),
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
            "project" => {
                println!(
                    "  + {:<20} project lexicon ({})",
                    r.token,
                    r.lemmas.join(", ")
                );
            }
            _ => {}
        }
        if let Some(w) = &r.agreement {
            println!("  ⚠ {:<20} {}", r.token, w);
        }
        if let Some(w) = &r.consistency {
            println!("  ⇄ {:<20} {}", r.token, w);
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
    if let Some(s) = &summary {
        let severe = match gate.unwrap().max_severe_warnings {
            Some(max) => format!(", {} severe warnings (max {max})", s.severe_warnings),
            None => format!(", {} severe warnings (not gated)", s.severe_warnings),
        };
        let consistency = match gate.unwrap().max_consistency {
            Some(max) => format!(
                ", {} consistency warnings (max {max})",
                s.consistency_warnings
            ),
            None if !index.lexicon.is_empty() => format!(
                ", {} consistency warnings (not gated)",
                s.consistency_warnings
            ),
            None => String::new(),
        };
        println!(
            "summary: {} — {} unknown (max {}), {} agreement errors (max {}), {} project{severe}{consistency}",
            if s.passed { "PASS" } else { "FAIL" },
            s.unknown,
            gate.unwrap().max_unknown,
            s.agreement_errors,
            gate.unwrap().max_agreement,
            s.project,
        );
    }
    let _ = json_escape("");
    fail_gate_if_needed(summary)
}

/// Nonzero exit for CI gating: a failed gate is an error AFTER all output has
/// been printed, so agents still receive the full report.
fn fail_gate_if_needed(summary: Option<Summary>) -> Result<()> {
    match summary {
        Some(s) if !s.passed => anyhow::bail!(
            "check-text gate failed: {} unknown token(s), {} agreement error(s), {} severe warning(s), {} consistency warning(s)",
            s.unknown,
            s.agreement_errors,
            s.severe_warnings,
            s.consistency_warnings
        ),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod summary_tests {
    use super::*;

    fn report(status: &'static str, agreement: Option<&str>) -> TokenReport {
        TokenReport {
            token: "x".into(),
            status,
            lemmas: Vec::new(),
            analyses: Vec::new(),
            ambiguous: false,
            probability: None,
            suggestions: Vec::new(),
            warning: None,
            severity: None,
            prefer: Vec::new(),
            agreement: agreement.map(str::to_string),
            consistency: None,
        }
    }

    #[test]
    fn summary_gate_counts_and_thresholds() {
        let reports = vec![
            report("known-lemma", None),
            report("unknown", None),
            report("known-form", Some("case mismatch")),
        ];
        let strict = summarize(
            &reports,
            SummaryGate {
                max_unknown: 0,
                max_agreement: 0,
                max_severe_warnings: None,
                max_consistency: None,
            },
        );
        assert_eq!((strict.unknown, strict.agreement_errors), (1, 1));
        assert!(!strict.passed);
        let lenient = summarize(
            &reports,
            SummaryGate {
                max_unknown: 1,
                max_agreement: 1,
                max_severe_warnings: None,
                max_consistency: None,
            },
        );
        assert!(lenient.passed);
    }

    /// V11 item 6: severe (primary-sense) warnings gate only when asked.
    #[test]
    fn severe_warning_gate_counts_high_and_medium_only() {
        let mut warned = report("known-lemma", None);
        warned.warning = Some("trap".into());
        warned.severity = Some("high".into());
        let mut colloquial = report("known-lemma", None);
        colloquial.warning = Some("slangy".into());
        colloquial.severity = Some("low".into());
        let reports = vec![warned, colloquial];
        let ungated = summarize(
            &reports,
            SummaryGate {
                max_unknown: 0,
                max_agreement: 0,
                max_severe_warnings: None,
                max_consistency: None,
            },
        );
        assert_eq!(ungated.severe_warnings, 1);
        assert!(ungated.passed, "severity must not gate unless requested");
        let gated = summarize(
            &reports,
            SummaryGate {
                max_unknown: 0,
                max_agreement: 0,
                max_severe_warnings: Some(0),
                max_consistency: None,
            },
        );
        assert!(!gated.passed);
    }
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
    let notes = crate::falsefriends::compute_from_default_caches(&entries);
    let index = build_index(&entries, Some(Path::new("data/novel-words.tsv")), notes);

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
        build_index(&entries, None, Default::default())
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
        let index = build_index(&entries, None, Default::default());
        let mut checked = 0usize;
        for e in &entries {
            for byform in e.citation_byforms() {
                let isv = byform.form;
                let toks = tokenize(&isv);
                if toks.len() > 2 || isv.contains("...") {
                    continue;
                }
                let reps = check_tokens(&index, &toks);
                assert!(
                    reps.iter()
                        .all(|r| r.status == "known-lemma" || r.status == "known-form"),
                    "official lemma '{isv}' not recognized: {:?}",
                    reps.iter().map(|r| r.status).collect::<Vec<_>>()
                );
                checked += 1;
            }
        }
        assert!(checked > 300, "sample too small: {checked}");

        // Sampled paradigm cells resolve as known forms.
        for e in entries.iter().filter(|e| e.pos == Pos::Noun).take(30) {
            let Some(isv) = e
                .citation_byforms()
                .into_iter()
                .map(|byform| byform.form)
                .find(|isv| !isv.contains(' '))
            else {
                continue;
            };
            let gen = crate::forms::noun_cell(
                &isv,
                interslavic::Case::Gen,
                interslavic::Number::Singular,
            );
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
        let index = build_index(&entries, None, Default::default());
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
    fn homographic_noun_genders_abstain_from_agreement() {
        let entries = official::load(Path::new(crate::DEFAULT_OFFICIAL)).expect("official csv");
        let index = build_index(&entries, None, Default::default());
        // Cover exact/case-folded homographs and genuine standard-orthography
        // collisions (Bělorus/Běloruś, plȯť/plot, spust/spusť).
        for word in ["Bělorus", "dodatȯk", "družba", "led", "plȯť", "spust"] {
            assert_eq!(
                index.noun_gender.get(&forms::form_key(word)),
                Some(&' '),
                "mixed-gender homograph {word} must not inherit the first CSV row's gender"
            );
        }
        for phrase in ["dobry družba", "dobry Bělorus", "dobra Běloruś"] {
            assert!(
                check_text(&index, phrase)
                    .iter()
                    .all(|r| r.agreement.is_none()),
                "valid homograph reading must not be rejected: {phrase}"
            );
        }

        // A synthetic m/f/m sequence proves ambiguity is absorbing without
        // coupling the order-independence test to CSV row count or ordering.
        let template = entries
            .iter()
            .find(|e| e.isv == "družba")
            .expect("noun fixture");
        let synthetic: Vec<OfficialEntry> = [
            crate::model::Gender::Masculine,
            crate::model::Gender::Feminine,
            crate::model::Gender::Masculine,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, gender)| {
            let mut entry = template.clone();
            entry.id = format!("synthetic-{i}");
            entry.isv = "testova".to_string();
            entry.noun_traits.gender = Some(gender);
            entry
        })
        .collect();
        let synthetic_index = build_index(&synthetic, None, Default::default());
        assert_eq!(
            synthetic_index.noun_gender.get(&forms::form_key("testova")),
            Some(&' ')
        );
    }

    /// V14 (interslavic 0.10.0): the pronoun series and the fixed
    /// l-participles resolve in running text — the exact forms a real
    /// translation exercises ("Ona šla k njemu", "Strěla tę ubila").
    #[test]
    fn pronoun_series_and_l_participles_resolve() {
        let entries = official::load(Path::new(crate::DEFAULT_OFFICIAL)).expect("official csv");
        let index = build_index(&entries, None, Default::default());
        for tok in [
            "njego", "njej", "njim", "njih", "njejų", "njemu", // n- forms
            "mę", "tę", "mi", "ti", "sę", "si", // clitics
            "mnojų", "tobojų", "sobojų", "sobě", "jejų", "je", // full obliques
            "pisala", "pisalo", "pisali", "šla", "šli", "viděla", "směli", // l-participles
        ] {
            let reps = check_tokens(&index, &tokenize(tok));
            assert!(
                reps.iter().all(|r| r.status != "unknown"),
                "'{tok}' must resolve: {:?}",
                reps.iter().map(|r| r.status).collect::<Vec<_>>()
            );
        }
        for sentence in ["Ona šla k njemu.", "Strěla tę ubila."] {
            let reps = check_text(&index, sentence);
            assert!(
                reps.iter()
                    .all(|r| r.status != "unknown" && r.agreement.is_none()),
                "'{sentence}' must verify clean: {reps:?}"
            );
        }
    }

    #[test]
    fn official_comma_byforms_are_known_lemmas() {
        let entries = official::load(Path::new(crate::DEFAULT_OFFICIAL)).expect("official csv");
        let index = build_index(&entries, None, Default::default());
        for isv in ["iměti", "imati", "poslědnji", "poslědny"] {
            let reps = check_tokens(&index, &tokenize(isv));
            assert!(
                reps.iter().any(|r| r.status == "known-lemma"),
                "official byform '{isv}' not recognized: {:?}",
                reps.iter().map(|r| r.status).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn shared_government_and_browser_suggestion_artifacts_are_deterministic() {
        let government = preposition_government();
        assert_eq!(government.get(&forms::form_key("bez")), Some(&vec!["gen"]));
        assert_eq!(
            government.get(&forms::form_key("v")),
            Some(&vec!["akuz", "lok"])
        );

        let index = Index {
            by_key: HashMap::new(),
            lemma_keys: vec![
                ("dom".to_string(), "dom".to_string()),
                ("doma".to_string(), "doma".to_string()),
                ("pomočny".to_string(), "pomoćny".to_string()),
            ],
            notes: BTreeMap::new(),
            noun_gender: HashMap::new(),
            prep_cases: government,
            lexicon: Vec::new(),
        };
        assert_eq!(suggest(&index, "domm"), vec!["dom", "doma"]);
        let dir = std::env::temp_dir().join(format!(
            "slovowiki-suggest-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let bytes = write_web_suggestions(&dir, &index).expect("suggest artifacts");
        assert!(bytes > 0);
        let fixture: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("api/suggest-selftest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(fixture["shards"], SUGGEST_SHARDS);
        assert_eq!(
            fixture["samples"].as_array().unwrap().len(),
            SUGGEST_SELFTEST_INPUTS.len()
        );
        assert_eq!(
            fixture["rows"].as_array().unwrap().len(),
            SUGGEST_SELFTEST_ROWS.len()
        );
        assert_eq!(
            std::fs::read_dir(dir.join("api/suggest")).unwrap().count(),
            SUGGEST_SHARDS as usize
        );
        let _ = std::fs::remove_dir_all(dir);
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

    /// V13 item 1 acceptance: the committed game-text/lexicon fixture pair.
    /// Without the lexicon the sanctioned coinage's inflections drown the
    /// gate as `unknown`; with it they classify `project`, the pinned
    /// official synonym raises exactly one consistency warning, and the
    /// `--max-unknown 0` gate passes (consistency gating stays opt-in).
    #[test]
    fn project_lexicon_end_to_end() {
        let entries = official::load(Path::new(crate::DEFAULT_OFFICIAL)).expect("official csv");
        let mut index = build_index(&entries, None, Default::default());
        let text = std::fs::read_to_string("data/game-text-fixture.txt").expect("fixture");

        let without: Vec<String> = check_text(&index, &text)
            .into_iter()
            .filter(|r| r.status == "unknown")
            .map(|r| r.token)
            .collect();
        assert_eq!(
            without,
            ["žabervoka", "žabervoka", "Žabervok", "žabervokom"],
            "coinage inflections must be unknown without the lexicon"
        );

        let rows = parse_lexicon(
            &std::fs::read_to_string("data/project-lexicon-fixture.tsv").expect("lexicon"),
        )
        .expect("lexicon parses");
        assert_eq!(rows.len(), 4);
        apply_lexicon(&mut index, rows).expect("lexicon validates");

        let reps = check_text(&index, &text);
        for tok in ["žabervoka", "Žabervok", "žabervokom"] {
            let rep = reps.iter().find(|r| r.token == tok).unwrap();
            assert_eq!(rep.status, "project", "{tok}");
            assert_eq!(rep.lemmas, ["žabervok"]);
            assert!(!rep.analyses.is_empty(), "{tok} carries analyses");
        }
        // Masc-ANIMATE accusative: the declared animacy must reach the
        // inflector (žabervoka = gen-shaped accusative).
        let acc = reps.iter().find(|r| r.token == "žabervoka").unwrap();
        assert!(
            acc.analyses.iter().any(|a| a.contains("akuz.jd.")),
            "animate accusative missing: {:?}",
            acc.analyses
        );
        assert!(reps.iter().all(|r| r.status != "unknown"));

        let consistency: Vec<&TokenReport> =
            reps.iter().filter(|r| r.consistency.is_some()).collect();
        assert_eq!(consistency.len(), 1, "exactly the praksa token drifts");
        assert_eq!(consistency[0].token, "praksa");
        let msg = consistency[0].consistency.as_deref().unwrap();
        assert!(
            msg.contains("praktika") && msg.contains("practice"),
            "{msg}"
        );

        let gate = SummaryGate {
            max_unknown: 0,
            max_agreement: 0,
            max_severe_warnings: None,
            max_consistency: None,
        };
        let s = summarize(&reps, gate);
        assert!(s.passed, "{s:?}");
        assert_eq!((s.project, s.unknown, s.consistency_warnings), (4, 0, 1));
        let gated = summarize(
            &reps,
            SummaryGate {
                max_consistency: Some(0),
                ..gate
            },
        );
        assert!(!gated.passed, "consistency gate must fire when requested");

        // The skip-own-lemma path: the project's OWN choice for a concept
        // (`praktika`, pinned official) must never warn against itself.
        let own = check_text(&index, "Tvoja praktika pomagaje.");
        let praktika = own.iter().find(|r| r.token == "praktika").unwrap();
        assert_eq!(praktika.status, "known-lemma");
        assert!(
            praktika.consistency.is_none(),
            "the row's own lemma must not drift against its own row: {:?}",
            praktika.consistency
        );
    }

    /// interslavic 0.12.0: the full numeral inventory declines. The two
    /// cells the release CHANGED are pinned with their lemma and analysis
    /// (a rerouted homograph must not pass silently); the additive classes
    /// (hundreds, tens, collectives, sedm/osm) are pinned by existence;
    /// and the dual-archaic dvěma stays unknown (the crate's instrumental
    /// is dvoma).
    #[test]
    fn numeral_inventory_resolves() {
        let entries = official::load(Path::new(crate::DEFAULT_OFFICIAL)).expect("official csv");
        let index = build_index(&entries, None, Default::default());
        // Changed cells: masculine-animate accusative and the neuter
        // nom/acc now shared with the feminine — both must belong to dva.
        for (tok, analysis) in [("dvoh", "akuz. m.živ."), ("dvě", "akuz. sr.")] {
            let reps = check_tokens(&index, &tokenize(tok));
            let rep = &reps[0];
            assert!(
                rep.lemmas.iter().any(|l| l == "dva"),
                "'{tok}' must belong to dva: {:?}",
                rep.lemmas
            );
            assert!(
                rep.analyses.iter().any(|a| a == analysis),
                "'{tok}' must carry '{analysis}': {:?}",
                rep.analyses
            );
        }
        for tok in [
            "trěh",
            "dvoma",
            "sedm",
            "osm",
            "dvěstě",
            "devęťsȯt",
            "pęťdesęt",
            "dvojih",
        ] {
            let reps = check_tokens(&index, &tokenize(tok));
            assert!(
                reps.iter().all(|r| r.status != "unknown"),
                "'{tok}' must resolve: {:?}",
                reps.iter().map(|r| r.status).collect::<Vec<_>>()
            );
        }
        let dvema = check_tokens(&index, &tokenize("dvěma"));
        assert!(
            dvema.iter().all(|r| r.status == "unknown"),
            "the dual-archaic dvěma must stay unknown (crate instrumental is dvoma): {:?}",
            dvema.iter().map(|r| r.status).collect::<Vec<_>>()
        );
    }

    /// V13 item 1: a broken lexicon is a hard error, never a silent
    /// weakening of the gate — syntax, crate-requirement, collision, and
    /// official-pin contradictions all reject.
    #[test]
    fn lexicon_validation_rejects_broken_rows() {
        // Syntax layer.
        for (row, why) in [
            ("žabervok\tnoun\tm\tanim", "4 columns"),
            ("žabervok\tpron\tm\tanim\tjabberwock", "unsupported pos"),
            ("žabervok\tnoun\t\tanim\tjabberwock", "noun without gender"),
            ("žabervok\tnoun\tm\t\tjabberwock", "noun without animacy"),
            ("žabervočiti\tverb\tm\t\tto jabberwock", "gender on a verb"),
            ("žabervok\tnoun\tm\tanim\t", "empty gloss"),
            (
                "žabervok\tnoun\tm\tanim\tjabberwock\nžabervok\tnoun\tm\tanim\tjabberwock",
                "duplicate lemma",
            ),
        ] {
            assert!(parse_lexicon(row).is_err(), "must reject: {why}");
        }
        // Valid adj/verb coinage rows parse and validate (no gender columns).
        let entries = official::load(Path::new(crate::DEFAULT_OFFICIAL)).expect("official csv");
        let index = build_index(&entries, None, Default::default());
        let ok = parse_lexicon(
            "žabervočny\tadj\t\t\tjabberwockian\nžabervočiti\tverb\t\t\tto jabberwock",
        )
        .expect("valid rows");
        for row in &ok {
            assert!(!validate_lexicon_row(&index, row).expect("validates"));
        }
        // Semantic layer, against the real index.
        for semantic in [
            (
                "vodami\tnoun\tf\tinanim\twater",
                "collides with an official inflected form",
            ),
            (
                "pravy\tnoun\tm\tinanim\tright",
                "pins official 'pravy' with a contradictory POS",
            ),
            (
                "voda\tnoun\tm\tinanim\twater",
                "pins official 'voda' with a contradictory gender",
            ),
            (
                "krva\tverb\t\t\tto bleed",
                "verb must cite the -ti infinitive",
            ),
            ("žabervok\tadj\t\t\tjabberwocky", "adjective must end -y/-i"),
        ] {
            let rows = parse_lexicon(semantic.0).expect("syntax ok");
            assert!(
                validate_lexicon_row(&index, &rows[0]).is_err(),
                "must reject: {}",
                semantic.1
            );
        }
        // The official PIN path: voda with the dictionary's own gender is
        // accepted and marked pinned (no project paradigm re-indexed).
        let pin = parse_lexicon("voda\tnoun\tf\tinanim\twater").unwrap();
        assert!(validate_lexicon_row(&index, &pin[0]).expect("pin validates"));

        // Rows validate SEQUENTIALLY against the growing index: a later row
        // colliding with an earlier coinage's inflected form is rejected,
        // not silently double-indexed. (Empty official corpus — the
        // collision is purely between the two project rows.)
        let mut coinage_index = build_index(&[], None, Default::default());
        let ordered = parse_lexicon(
            "žabervok\tnoun\tm\tanim\tjabberwock\nžabervoka\tnoun\tf\tinanim\tjabberwock hen",
        )
        .expect("both rows parse");
        let err = apply_lexicon(&mut coinage_index, ordered).unwrap_err();
        assert!(
            err.to_string().contains("žabervoka") && err.to_string().contains("collides"),
            "{err}"
        );
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
