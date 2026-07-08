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
}

/// Build the verification index from the official dictionary (lemmas + full
/// paradigms) and, when present, the committed novel-word proposals (lemma
/// records with their calibrated probability).
pub fn build_index(entries: &[OfficialEntry], novel_words_tsv: Option<&Path>) -> Index {
    let mut sink = RecordSink::default();
    let mut seen: HashSet<String> = HashSet::new();
    for e in entries {
        let isv = e.isv.trim();
        if isv.is_empty() || isv.contains('#') || isv.contains('!') {
            continue;
        }
        // Two-token lemmas (reflexive verbs AND collocations) are reachable
        // via the general bigram lookup; only 3+-token phrases stay lemma-only.
        let single = !isv.contains(' ') || isv.ends_with(" sę");
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
            forms::paradigm_records(&mut sink, isv, e.pos, 0, "official", None, &e.english);
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

pub fn check_tokens(index: &Index, tokens: &[String]) -> Vec<TokenReport> {
    let mut reports = Vec::new();
    let mut i = 0usize;
    while i < tokens.len() {
        let tok = &tokens[i];
        let key = forms::form_key(tok);
        if key.is_empty() {
            i += 1;
            continue;
        }
        // Two-token lookup first: reflexive verbs (`myti sę` → key `myti se`)
        // and multi-word official lemmas (`adamovo jablȯko`) are indexed under
        // a single space-joined key.
        let mut consumed = 1;
        let mut matched_key = key.clone();
        let mut recs: Option<&Vec<FormRecord>> = None;
        if let Some(next) = tokens.get(i + 1) {
            let bigram = format!("{key} {}", forms::form_key(next));
            if let Some(r) = index.by_key.get(&bigram) {
                recs = Some(r);
                matched_key = bigram;
                consumed = 2;
            }
        }
        let recs = recs.or_else(|| index.by_key.get(&key));
        // Display echoes the SOURCE spelling so JSON consumers can locate the
        // original text span.
        let display = if consumed == 2 {
            format!("{tok} {}", tokens[i + 1])
        } else {
            tok.clone()
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
            },
        };
        reports.push(report);
        i += consumed;
    }
    reports
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
    let tokens = tokenize(&text);
    let reports = check_tokens(&index, &tokens);

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
