//! Static English -> Interslavic lookup API for translation agents.
//!
//! This is deliberately generated from the same lemma [`crate::forms::FormRecord`]
//! stream as the form API. The English API discovers candidate lemmas; the form
//! API remains the source for validating and inflecting those lemmas.

use crate::forms::{self, AspectMeta, FormRecord};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use super::model::SiteEntryMeta;

pub(super) const EN_SCHEMA_VERSION: u32 = 1;
pub(super) const EN_SHARDS: u32 = 256;

/// Canonical raw queries shipped in `api/en/selftest.json`. Chosen to exercise
/// every normalization rule: case + whitespace, punctuation folding (hyphen,
/// apostrophe), the leading `to ` strip, and non-ASCII UTF-8 hashing.
const EN_SELFTEST_SAMPLES: &[&str] = &[
    " To   Save! ",
    "Coat-of-Arms",
    "to be",
    "don't",
    "naïve café",
    "game",
];

/// Function words: never a lookup key, not even as an exact gloss head.
const HEAD_STOPWORDS: &[&str] = &["a", "an", "and", "etc", "in", "of", "or", "the", "to"];

/// Grammatical-note noise, filtered from token extraction only. A gloss whose
/// entire head is one of these ("one", "form", "plural") is a real English
/// word and must stay findable.
const TOKEN_STOPWORDS: &[&str] = &[
    "archaic",
    "dative",
    "form",
    "genitive",
    "instrumental",
    "locative",
    "nominative",
    "obsolete",
    "one",
    "plural",
    "singular",
    "someone",
    "something",
    "used",
    "variant",
];

#[derive(Debug, Clone, Serialize)]
pub(super) struct EnglishCandidate {
    lemma: String,
    entry_id: usize,
    official_id: Option<String>,
    pos: String,
    gloss: String,
    status: String,
    trust: String,
    rank: i32,
    #[serde(rename = "match")]
    match_kind: String,
    aspect: Option<String>,
    aspect_partners: Vec<AspectPartner>,
    warnings: Vec<String>,
    prefer: Vec<String>,
    form_lookup: FormLookup,
    probability: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct AspectPartner {
    entry_id: usize,
    lemma: String,
}

#[derive(Debug, Clone, Serialize)]
struct FormLookup {
    key: String,
    shard: u32,
    path: String,
}

#[derive(Debug, Clone)]
struct KeyMatch {
    key: String,
    match_kind: String,
    match_rank: i32,
}

#[derive(Debug, Clone)]
pub(super) struct EnglishApiCounts {
    pub(super) keys: usize,
    pub(super) candidates: usize,
    pub(super) bytes: usize,
    pub(super) largest_shard: usize,
}

#[derive(Debug, Clone, Serialize)]
struct ShardFile {
    schema_version: u32,
    shard: u32,
    license: &'static str,
    records: BTreeMap<String, Vec<EnglishCandidate>>,
}

fn normalize_english_text(raw: &str) -> String {
    let mut out = String::new();
    let mut last_space = true;
    for ch in raw.to_lowercase().chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

pub(super) fn normalize_english_query(raw: &str) -> String {
    let key = normalize_english_text(raw);
    key.strip_prefix("to ")
        .map(str::trim)
        .filter(|rest| !rest.is_empty())
        .unwrap_or(&key)
        .to_string()
}

pub(super) fn english_shard_of(key: &str) -> u32 {
    forms::fnv1a32(key) % EN_SHARDS
}

fn usable_head_key(key: &str) -> bool {
    let n = key.chars().count();
    (2..=48).contains(&n)
        && !HEAD_STOPWORDS.contains(&key)
        && !key.ends_with(" etc")
        && !key.starts_with("used ")
}

fn usable_token_key(key: &str) -> bool {
    usable_head_key(key) && !TOKEN_STOPWORDS.contains(&key)
}

pub(super) fn gloss_keys(gloss: &str) -> Vec<(String, String)> {
    // Derivative glosses ("label ← base (…)") truncate the base gloss with a
    // trailing `…`, so their final segment can be a cut word ("substa…") or
    // carry an unbalanced paren. Official glosses use `…` legitimately
    // ("either … or …") and are never ←-shaped.
    let derived = gloss.contains('←');
    let gloss = english_lookup_gloss(gloss);
    let mut out = Vec::new();
    for segment in split_gloss_segments(gloss) {
        if derived && segment.contains('…') {
            continue;
        }
        let (head, parenthetical) = split_parenthetical(segment);
        push_gloss_head_keys(&mut out, head);
        if let Some(note) = parenthetical {
            push_parenthetical_keys(&mut out, note);
        }
    }
    out
}

fn english_lookup_gloss(gloss: &str) -> &str {
    if gloss.contains('←') {
        if let (Some(open), Some(close)) = (gloss.find('('), gloss.rfind(')')) {
            if open < close {
                return &gloss[open + 1..close];
            }
        }
    }
    gloss
}

fn split_gloss_segments(gloss: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    for (i, ch) in gloss.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' | ';' | '/' if depth == 0 => {
                out.push(&gloss[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&gloss[start..]);
    out
}

fn split_parenthetical(segment: &str) -> (&str, Option<&str>) {
    match (segment.find('('), segment.rfind(')')) {
        (Some(open), Some(close)) if open < close => {
            let before = segment[..open].trim();
            let after = segment[close + 1..].trim();
            if before.is_empty() && !after.is_empty() {
                // Leading register label, e.g. "(formal) day": the head follows
                // the note, and the label itself is not a lookup key.
                (after, None)
            } else {
                (before, Some(segment[open + 1..close].trim()))
            }
        }
        _ => (segment.trim(), None),
    }
}

fn push_gloss_head_keys(out: &mut Vec<(String, String)>, raw: &str) {
    let normalized_head = raw.replace(" or ", ",");
    if normalized_head != raw {
        // Idioms like "more or less": query normalization keeps "or", so the
        // full phrase must be a key too, not only the split alternatives. The
        // " or " must survive normalization ("either … or …" folds to
        // "either or" — not a phrase anyone queries).
        let full = normalize_english_query(raw);
        if full.contains(" or ") && usable_head_key(&full) {
            push_key(out, full, "phrase");
        }
    }
    for part in normalized_head.split(',') {
        let key = normalize_english_query(part);
        if !usable_head_key(&key) {
            continue;
        }
        let match_kind = if key.contains(' ') {
            "phrase"
        } else {
            "exact-gloss-head"
        };
        push_key(out, key.clone(), match_kind);
        if key.contains(' ') {
            push_token_keys(out, &key);
        }
    }
}

fn push_parenthetical_keys(out: &mut Vec<(String, String)>, raw: &str) {
    if raw.contains([',', ';', '/']) {
        return;
    }
    let mut key = normalize_english_query(raw);
    for prefix in ["of ", "a ", "an ", "the ", "on ", "in "] {
        while let Some(rest) = key.strip_prefix(prefix) {
            key = rest.trim().to_string();
        }
    }
    let words = key.split_whitespace().count();
    if words <= 3 {
        push_token_keys(out, &key);
    }
}

fn push_token_keys(out: &mut Vec<(String, String)>, key: &str) {
    for token in key.split_whitespace() {
        if usable_token_key(token) {
            push_key(out, token.to_string(), "gloss-token");
        }
    }
}

/// Rank contribution of the match kind. `phrase` and `exact-gloss-head` never
/// compete on one key (a key with a space is always a phrase match, a key
/// without one never is) — the weights only order them against `gloss-token`.
fn match_rank(match_kind: &str) -> i32 {
    match match_kind {
        "phrase" => 120,
        "exact-gloss-head" => 100,
        "gloss-token" => 40,
        _ => 20,
    }
}

fn push_key(out: &mut Vec<(String, String)>, key: String, match_kind: &str) {
    match out.iter_mut().find(|(seen, _)| seen == &key) {
        // Upgrade, e.g. a token of an earlier phrase segment reappearing as
        // its own exact segment: "up until, before, until".
        Some((_, existing)) if match_rank(match_kind) > match_rank(existing) => {
            *existing = match_kind.to_string();
        }
        Some(_) => {}
        None => out.push((key, match_kind.to_string())),
    }
}

fn key_matches(gloss: &str) -> Vec<KeyMatch> {
    gloss_keys(gloss)
        .into_iter()
        .map(|(key, match_kind)| {
            let match_rank = match_rank(&match_kind);
            KeyMatch {
                key,
                match_kind,
                match_rank,
            }
        })
        .collect()
}

fn trust(status: &str) -> &'static str {
    match status {
        "official" => "verified-official",
        "official-only" => "verified-official-only",
        _ => "generated-review",
    }
}

fn status_rank(status: &str) -> i32 {
    match status {
        "official" => 300,
        "official-only" => 280,
        "generated" => 100,
        _ => 0,
    }
}

pub(super) fn build_english_index(
    lemmas: &[FormRecord],
    metas: &[SiteEntryMeta],
    aspect_meta: &AspectMeta,
    notes: &BTreeMap<String, crate::falsefriends::Note>,
) -> BTreeMap<String, Vec<EnglishCandidate>> {
    let meta_by_id: HashMap<usize, &SiteEntryMeta> = metas.iter().map(|m| (m.id, m)).collect();
    let mut candidates: BTreeMap<(String, usize, String), EnglishCandidate> = BTreeMap::new();

    for record in lemmas {
        if record.source != "lemma"
            || !matches!(record.status, "official" | "official-only" | "generated")
        {
            continue;
        }
        let keys = key_matches(&record.gloss);
        if keys.is_empty() {
            continue;
        }

        let form_key = forms::form_key(&record.lemma);
        let form_shard = forms::shard_of(&form_key);
        let note = notes.get(&form_key);
        let warnings = note.map(|n| vec![n.warning.clone()]).unwrap_or_default();
        let prefer = note.map(|n| n.prefer.clone()).unwrap_or_default();

        let official_id = if matches!(record.status, "official" | "official-only") {
            meta_by_id
                .get(&record.entry_id)
                .and_then(|m| m.official_sense_id.clone())
        } else {
            None
        };
        let (aspect, aspect_partners) =
            if record.pos == "verb" && matches!(record.status, "official" | "official-only") {
                aspect_meta
                    .get(&record.entry_id)
                    .map(|(aspect, partners)| {
                        (
                            Some(aspect.clone()),
                            partners
                                .iter()
                                .map(|(entry_id, lemma)| AspectPartner {
                                    entry_id: *entry_id,
                                    lemma: lemma.clone(),
                                })
                                .collect(),
                        )
                    })
                    .unwrap_or_default()
            } else {
                (None, Vec::new())
            };

        for key_match in keys {
            let rank = status_rank(record.status)
                + key_match.match_rank
                + record
                    .probability
                    .map(|p| (p * 10.0).round() as i32)
                    .unwrap_or(0);
            let candidate = EnglishCandidate {
                lemma: record.lemma.clone(),
                entry_id: record.entry_id,
                official_id: official_id.clone(),
                pos: record.pos.to_string(),
                gloss: record.gloss.clone(),
                status: record.status.to_string(),
                trust: trust(record.status).to_string(),
                rank,
                match_kind: key_match.match_kind.to_string(),
                aspect: aspect.clone(),
                aspect_partners: aspect_partners.clone(),
                warnings: warnings.clone(),
                prefer: prefer.clone(),
                form_lookup: FormLookup {
                    key: form_key.clone(),
                    shard: form_shard,
                    path: format!("api/forms/{form_shard}.json"),
                },
                probability: record.probability,
            };
            let dedup_key = (key_match.key, record.entry_id, form_key.clone());
            match candidates.get_mut(&dedup_key) {
                Some(existing) if candidate.rank > existing.rank => *existing = candidate,
                Some(_) => {}
                None => {
                    candidates.insert(dedup_key, candidate);
                }
            }
        }
    }

    let mut by_key: BTreeMap<String, Vec<EnglishCandidate>> = BTreeMap::new();
    for ((key, _, _), candidate) in candidates {
        by_key.entry(key).or_default().push(candidate);
    }
    for values in by_key.values_mut() {
        values.sort_by(|a, b| {
            b.rank
                .cmp(&a.rank)
                .then_with(|| a.lemma.cmp(&b.lemma))
                .then_with(|| a.pos.cmp(&b.pos))
                .then_with(|| a.entry_id.cmp(&b.entry_id))
        });
    }
    by_key
}

pub(super) fn write_en_api(
    out_dir: &Path,
    lemmas: &[FormRecord],
    metas: &[SiteEntryMeta],
    aspect_meta: &AspectMeta,
    notes: &BTreeMap<String, crate::falsefriends::Note>,
    git: &str,
) -> anyhow::Result<EnglishApiCounts> {
    let api = out_dir.join("api");
    let en_dir = api.join("en");
    let _ = std::fs::remove_dir_all(&en_dir);
    std::fs::create_dir_all(&en_dir)?;

    let index = build_english_index(lemmas, metas, aspect_meta, notes);
    let key_count = index.len();
    let mut shards: BTreeMap<u32, BTreeMap<String, Vec<EnglishCandidate>>> = BTreeMap::new();
    let mut candidate_count = 0usize;
    for (key, records) in index {
        candidate_count += records.len();
        shards
            .entry(english_shard_of(&key))
            .or_default()
            .insert(key, records);
    }

    let mut bytes = 0usize;
    let mut largest_shard = 0usize;
    for shard in 0..EN_SHARDS {
        let file = ShardFile {
            schema_version: EN_SCHEMA_VERSION,
            shard,
            license: forms::LICENSE,
            records: shards.remove(&shard).unwrap_or_default(),
        };
        let json = serde_json::to_string(&file)? + "\n";
        bytes += json.len();
        largest_shard = largest_shard.max(json.len());
        std::fs::write(en_dir.join(format!("{shard}.json")), json)?;
    }
    let total_shard_bytes = bytes;

    // Selftest mirror of api/router-selftest.json: canonical
    // (raw query → normalized key → shard) samples so a client's independent
    // normalization + router implementation fails loudly instead of silently
    // fetching the wrong shard.
    let samples: Vec<serde_json::Value> = EN_SELFTEST_SAMPLES
        .iter()
        .map(|raw| {
            let key = normalize_english_query(raw);
            let shard = english_shard_of(&key);
            serde_json::json!([raw, key, shard])
        })
        .collect();
    let selftest = serde_json::json!({
        "schema_version": EN_SCHEMA_VERSION,
        "shards": EN_SHARDS,
        "samples": samples,
    });
    let selftest_json = serde_json::to_string(&selftest)? + "\n";
    bytes += selftest_json.len();
    std::fs::write(en_dir.join("selftest.json"), selftest_json)?;

    let meta = serde_json::json!({
        "schema_version": EN_SCHEMA_VERSION,
        "git": git,
        "license": forms::LICENSE,
        "shards": EN_SHARDS,
        "router": "fnv1a32(utf8(normalized_query)) % shards",
        "normalization": "lowercase; replace punctuation with spaces; collapse whitespace; trim; strip leading verb marker `to `",
        "selftest": "api/en/selftest.json samples are [raw_query, normalized_key, shard]; verify your normalization + router reproduces them before first use",
        "english_keys": key_count,
        "candidate_records": candidate_count,
        "total_shard_bytes": total_shard_bytes,
        "largest_shard_bytes": largest_shard,
        "fields": {
            "lemma": "Interslavic citation form",
            "entry_id": "static site entry id; page is entry/<entry_id>.html",
            "official_id": "source official dictionary id when status is official or official-only",
            "pos": "compact part-of-speech code",
            "gloss": "English source gloss",
            "status": "official, official-only, or generated",
            "trust": "verified-official, verified-official-only, or generated-review",
            "rank": "deterministic ranking score within one English key",
            "match": "why this candidate is indexed for this English key",
            "aspect": "ipf, pf, ipf/pf, or null",
            "aspect_partners": "known aspect partner entry ids and lemmas",
            "warnings": "computed false-friend warnings (same records as api/notes.json)",
            "prefer": "official lemma(s) covering the divergent sense, computed from gloss overlap",
            "form_lookup": "folded lemma key and api/forms shard for inflection lookup",
            "probability": "model-specific generated probability when available"
        },
        "files": {
            "shards": "api/en/<n>.json",
            "selftest": "api/en/selftest.json",
            "forms": "api/forms/<n>.json",
            "guide": "api/agent-guide.md"
        }
    });
    let meta_json = serde_json::to_string_pretty(&meta)? + "\n";
    bytes += meta_json.len();
    std::fs::write(en_dir.join("meta.json"), meta_json)?;

    Ok(EnglishApiCounts {
        keys: key_count,
        candidates: candidate_count,
        bytes,
        largest_shard,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Confidence;

    fn record(
        lemma: &str,
        entry_id: usize,
        status: &'static str,
        gloss: &str,
        probability: Option<f64>,
    ) -> FormRecord {
        FormRecord {
            form: lemma.to_string(),
            key: forms::form_key(lemma),
            lemma: lemma.to_string(),
            entry_id,
            pos: "verb",
            analyses: Vec::new(),
            source: "lemma",
            status,
            probability,
            gloss: gloss.to_string(),
        }
    }

    fn meta(id: usize, official_id: Option<&str>) -> SiteEntryMeta {
        SiteEntryMeta {
            id,
            title: format!("entry-{id}"),
            gloss: String::new(),
            pos: "verb".to_string(),
            conf: Confidence::High,
            score: 1.0,
            prob: None,
            prior: None,
            n_langs: 0,
            n_branches: 0,
            borrowed: false,
            official_only: false,
            raw: false,
            official_lemma: Some("x".to_string()),
            official_sense_id: official_id.map(str::to_string),
            aspect: None,
            aspect_partners: Vec::new(),
            ancestor: String::new(),
            languages: Vec::new(),
            first: String::new(),
            categories: Vec::new(),
        }
    }

    #[test]
    fn normalizes_english_query_and_routes_deterministically() {
        assert_eq!(normalize_english_query(" Save   Game! "), "save game");
        assert_eq!(normalize_english_query(" To   Save! "), "save");
        assert_eq!(
            english_shard_of(&normalize_english_query("to save")),
            english_shard_of("save")
        );
        assert_eq!(english_shard_of("save"), forms::fnv1a32("save") % EN_SHARDS);
    }

    #[test]
    fn en_selftest_samples_are_frozen() {
        // These exact values ship in api/en/selftest.json; clients verify
        // their normalization + router against them. Changing either the fold
        // or the router breaks the published contract — bump EN_SCHEMA_VERSION.
        let expected: &[(&str, &str, u32)] = &[
            (" To   Save! ", "save", 72),
            ("Coat-of-Arms", "coat of arms", 18),
            ("to be", "be", 128),
            ("don't", "don t", 58),
            ("naïve café", "naïve café", 21),
            ("game", "game", 7),
        ];
        assert_eq!(EN_SELFTEST_SAMPLES.len(), expected.len());
        for (raw, key, shard) in expected {
            assert_eq!(normalize_english_query(raw), *key, "fold of {raw:?}");
            assert_eq!(english_shard_of(key), *shard, "shard of {key:?}");
        }
    }

    #[test]
    fn extracts_gloss_keys_without_parenthetical_noise_or_stopwords() {
        assert_eq!(
            gloss_keys("to save, rescue"),
            vec![
                ("save".to_string(), "exact-gloss-head".to_string()),
                ("rescue".to_string(), "exact-gloss-head".to_string())
            ]
        );
        assert_eq!(
            gloss_keys("save game"),
            vec![
                ("save game".to_string(), "phrase".to_string()),
                ("save".to_string(), "gloss-token".to_string()),
                ("game".to_string(), "gloss-token".to_string())
            ]
        );
        assert_eq!(
            gloss_keys("coat of arms"),
            vec![
                ("coat of arms".to_string(), "phrase".to_string()),
                ("coat".to_string(), "gloss-token".to_string()),
                ("arms".to_string(), "gloss-token".to_string())
            ]
        );
        assert_eq!(
            gloss_keys("to open (make accessible, unseal)"),
            vec![("open".to_string(), "exact-gloss-head".to_string())]
        );
        assert_eq!(
            gloss_keys("dish (food), course (of a meal)"),
            vec![
                ("dish".to_string(), "exact-gloss-head".to_string()),
                ("food".to_string(), "gloss-token".to_string()),
                ("course".to_string(), "exact-gloss-head".to_string()),
                ("meal".to_string(), "gloss-token".to_string())
            ]
        );
        assert_eq!(
            gloss_keys("pridavnik ← Abhazija (Abkhazia)"),
            vec![("abkhazia".to_string(), "exact-gloss-head".to_string())]
        );
        assert!(gloss_keys("the, a, of").is_empty());
    }

    #[test]
    fn token_stopwords_stay_findable_as_exact_gloss_heads() {
        // "one" (jedin), "form" (forma), "plural" (množina) are real English
        // headwords in the official data; the token stoplist must not hide them.
        assert_eq!(
            gloss_keys("one"),
            vec![("one".to_string(), "exact-gloss-head".to_string())]
        );
        assert_eq!(
            gloss_keys("form, shape"),
            vec![
                ("form".to_string(), "exact-gloss-head".to_string()),
                ("shape".to_string(), "exact-gloss-head".to_string())
            ]
        );
        // ... while grammatical-note tokens are still filtered.
        assert_eq!(
            gloss_keys("plural form of oko"),
            vec![
                ("plural form of oko".to_string(), "phrase".to_string()),
                ("oko".to_string(), "gloss-token".to_string())
            ]
        );
    }

    #[test]
    fn later_exact_segment_upgrades_earlier_token_match() {
        // dopŕva-style gloss: "until" first appears as a token of "up until",
        // then as its own exact segment — the exact match must win.
        assert_eq!(
            gloss_keys("up until, before, until"),
            vec![
                ("up until".to_string(), "phrase".to_string()),
                ("up".to_string(), "gloss-token".to_string()),
                ("until".to_string(), "exact-gloss-head".to_string()),
                ("before".to_string(), "exact-gloss-head".to_string())
            ]
        );
    }

    #[test]
    fn truncated_derivative_segments_are_not_indexed() {
        // coverage.rs truncates the base gloss at 50 chars with a trailing
        // `…`; the cut fragment must not become an English key.
        assert_eq!(
            gloss_keys("dějatelj ← potvŕditi (confirm, attest, substa…)"),
            vec![
                ("confirm".to_string(), "exact-gloss-head".to_string()),
                ("attest".to_string(), "exact-gloss-head".to_string())
            ]
        );
        // ... while official glosses keep their legitimate `…` segments.
        assert_eq!(
            gloss_keys("either … or …"),
            vec![("either".to_string(), "exact-gloss-head".to_string())]
        );
    }

    #[test]
    fn or_idioms_keep_their_full_phrase_key() {
        // Query normalization keeps "or", so "more or less" must be findable
        // as a phrase, not only via its split alternatives.
        assert_eq!(
            gloss_keys("more or less"),
            vec![
                ("more or less".to_string(), "phrase".to_string()),
                ("more".to_string(), "exact-gloss-head".to_string()),
                ("less".to_string(), "exact-gloss-head".to_string())
            ]
        );
    }

    #[test]
    fn leading_parenthetical_is_a_label_not_a_key() {
        // denonočje-style segment "(formal) day": head follows the label and
        // the label itself is not indexed.
        assert_eq!(
            gloss_keys("(formal) day"),
            vec![("day".to_string(), "exact-gloss-head".to_string())]
        );
    }

    #[test]
    fn orders_verified_candidates_before_generated_candidates() {
        let records = vec![
            record("spasati", 1, "official", "to save, rescue", None),
            record("save-machine", 2, "generated", "save", Some(0.9)),
        ];
        let metas = vec![meta(1, Some("official-1")), meta(2, None)];
        let index = build_english_index(&records, &metas, &AspectMeta::new(), &BTreeMap::new());
        let save = index.get("save").expect("save key");
        assert_eq!(save[0].lemma, "spasati");
        assert_eq!(save[0].status, "official");
        assert_eq!(save[0].official_id.as_deref(), Some("official-1"));
        assert_eq!(save[1].status, "generated");
    }

    #[test]
    fn exact_official_only_match_outranks_official_token_match() {
        let records = vec![
            record("bridž", 1, "official", "bridge (game)", None),
            record("divina", 2, "official-only", "game, wildfowl", None),
        ];
        let metas = vec![meta(1, Some("bridge-1")), meta(2, Some("game-2"))];
        let index = build_english_index(&records, &metas, &AspectMeta::new(), &BTreeMap::new());
        let game = index.get("game").expect("game key");
        assert_eq!(game[0].lemma, "divina");
        assert_eq!(game[0].match_kind, "exact-gloss-head");
        assert_eq!(game[1].lemma, "bridž");
        assert_eq!(game[1].match_kind, "gloss-token");
    }

    #[test]
    fn indexes_of_phrases_and_tokens() {
        let records = vec![record("gerb", 1, "official", "coat of arms", None)];
        let metas = vec![meta(1, Some("arms-1"))];
        let index = build_english_index(&records, &metas, &AspectMeta::new(), &BTreeMap::new());
        assert_eq!(
            index.get("coat of arms").expect("phrase key")[0].lemma,
            "gerb"
        );
        assert_eq!(index.get("coat").expect("coat token")[0].lemma, "gerb");
        assert_eq!(index.get("arms").expect("arms token")[0].lemma, "gerb");
        assert!(!index.contains_key("of"));
    }

    #[test]
    fn official_byforms_are_each_indexed_for_the_english_key() {
        let records = vec![
            record("iměti", 10, "official", "to have", None),
            record("imati", 10, "official", "to have", None),
        ];
        let metas = vec![meta(10, Some("have-10"))];
        let index = build_english_index(&records, &metas, &AspectMeta::new(), &BTreeMap::new());
        let have = index.get("have").expect("have key");
        let lemmas: Vec<&str> = have.iter().map(|c| c.lemma.as_str()).collect();
        assert!(lemmas.contains(&"iměti"));
        assert!(lemmas.contains(&"imati"));
    }

    #[test]
    fn writes_meta_and_shards() {
        let tmp =
            std::env::temp_dir().join(format!("slovowiki-en-api-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let records = vec![record("igra", 42, "official-only", "game, play", None)];
        let metas = vec![SiteEntryMeta {
            official_only: true,
            official_lemma: Some("igra".to_string()),
            ..meta(42, Some("game-42"))
        }];
        let counts = write_en_api(
            &tmp,
            &records,
            &metas,
            &AspectMeta::new(),
            &BTreeMap::new(),
            "test",
        )
        .expect("write english api");
        assert!(tmp.join("api/en/meta.json").exists());
        assert!(tmp
            .join(format!("api/en/{}.json", english_shard_of("game")))
            .exists());
        assert!(counts.keys >= 2);
        assert!(counts.candidates >= 2);
        let selftest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.join("api/en/selftest.json")).expect("selftest file"),
        )
        .expect("selftest json");
        let samples = selftest["samples"].as_array().expect("samples array");
        assert_eq!(samples.len(), EN_SELFTEST_SAMPLES.len());
        for sample in samples {
            let raw = sample[0].as_str().expect("raw query");
            assert_eq!(
                sample[1].as_str(),
                Some(normalize_english_query(raw).as_str())
            );
            assert_eq!(
                sample[2].as_u64(),
                Some(u64::from(english_shard_of(sample[1].as_str().unwrap())))
            );
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
