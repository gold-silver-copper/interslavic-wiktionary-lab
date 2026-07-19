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

const STOPWORDS: &[&str] = &[
    "a",
    "an",
    "and",
    "archaic",
    "dative",
    "etc",
    "form",
    "genitive",
    "in",
    "instrumental",
    "locative",
    "nominative",
    "obsolete",
    "of",
    "one",
    "or",
    "plural",
    "singular",
    "someone",
    "something",
    "the",
    "to",
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

pub(super) fn normalize_english_query(raw: &str) -> String {
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

pub(super) fn english_shard_of(key: &str) -> u32 {
    forms::fnv1a32(key) % EN_SHARDS
}

fn is_stopword(key: &str) -> bool {
    STOPWORDS.contains(&key)
}

fn usable_key(key: &str) -> bool {
    let n = key.chars().count();
    (2..=48).contains(&n)
        && !is_stopword(key)
        && !key.contains(" of ")
        && !key.ends_with(" etc")
        && !key.starts_with("used ")
}

pub(super) fn gloss_keys(gloss: &str) -> Vec<(String, String)> {
    let gloss = english_lookup_gloss(gloss);
    let mut out = Vec::new();
    for segment in split_gloss_segments(gloss) {
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
        (Some(open), Some(close)) if open < close => (
            segment[..open].trim(),
            Some(segment[open + 1..close].trim()),
        ),
        _ => (segment.trim(), None),
    }
}

fn push_gloss_head_keys(out: &mut Vec<(String, String)>, raw: &str) {
    let normalized_head = raw.replace(" or ", ",");
    for part in normalized_head.split(',') {
        let mut key = normalize_english_query(part);
        if let Some(rest) = key.strip_prefix("to ") {
            key = rest.trim().to_string();
        }
        if !usable_key(&key) {
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
        if usable_key(token) {
            push_key(out, token.to_string(), "gloss-token");
        }
    }
}

fn push_key(out: &mut Vec<(String, String)>, key: String, match_kind: &str) {
    if !out.iter().any(|(seen, _)| seen == &key) {
        out.push((key, match_kind.to_string()));
    }
}

fn key_matches(gloss: &str) -> Vec<KeyMatch> {
    gloss_keys(gloss)
        .into_iter()
        .map(|(key, match_kind)| {
            let match_rank = match match_kind.as_str() {
                "phrase" => 120,
                "exact-gloss-head" => 100,
                "gloss-token" => 40,
                _ => 20,
            };
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

fn semantic_notes() -> BTreeMap<String, crate::check::SemanticNote> {
    std::fs::read_to_string(crate::check::SEMANTIC_NOTES)
        .ok()
        .and_then(|raw| {
            serde_json::from_str::<BTreeMap<String, crate::check::SemanticNote>>(&raw).ok()
        })
        .map(|notes| {
            notes
                .into_iter()
                .map(|(key, note)| (forms::form_key(&key), note))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn build_english_index(
    lemmas: &[FormRecord],
    metas: &[SiteEntryMeta],
    aspect_meta: &AspectMeta,
) -> BTreeMap<String, Vec<EnglishCandidate>> {
    let meta_by_id: HashMap<usize, &SiteEntryMeta> = metas.iter().map(|m| (m.id, m)).collect();
    let notes = semantic_notes();
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
    git: &str,
) -> anyhow::Result<EnglishApiCounts> {
    let api = out_dir.join("api");
    let en_dir = api.join("en");
    let _ = std::fs::remove_dir_all(&en_dir);
    std::fs::create_dir_all(&en_dir)?;

    let index = build_english_index(lemmas, metas, aspect_meta);
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

    let meta = serde_json::json!({
        "schema_version": EN_SCHEMA_VERSION,
        "git": git,
        "license": forms::LICENSE,
        "shards": EN_SHARDS,
        "router": "fnv1a32(utf8(normalized_query)) % shards",
        "normalization": "lowercase; replace punctuation with spaces; collapse whitespace; strip leading verb marker `to ` for gloss heads",
        "english_keys": key_count,
        "candidate_records": candidate_count,
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
            "warnings": "semantic-trap warnings from api/notes.json",
            "prefer": "preferred alternatives from semantic notes",
            "form_lookup": "folded lemma key and api/forms shard for inflection lookup",
            "probability": "model-specific generated probability when available"
        },
        "files": {
            "shards": "api/en/<n>.json",
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
        assert_eq!(english_shard_of("save"), forms::fnv1a32("save") % EN_SHARDS);
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
    fn orders_verified_candidates_before_generated_candidates() {
        let records = vec![
            record("spasati", 1, "official", "to save, rescue", None),
            record("save-machine", 2, "generated", "save", Some(0.9)),
        ];
        let metas = vec![meta(1, Some("official-1")), meta(2, None)];
        let index = build_english_index(&records, &metas, &AspectMeta::new());
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
        let index = build_english_index(&records, &metas, &AspectMeta::new());
        let game = index.get("game").expect("game key");
        assert_eq!(game[0].lemma, "divina");
        assert_eq!(game[0].match_kind, "exact-gloss-head");
        assert_eq!(game[1].lemma, "bridž");
        assert_eq!(game[1].match_kind, "gloss-token");
    }

    #[test]
    fn official_byforms_are_each_indexed_for_the_english_key() {
        let records = vec![
            record("iměti", 10, "official", "to have", None),
            record("imati", 10, "official", "to have", None),
        ];
        let metas = vec![meta(10, Some("have-10"))];
        let index = build_english_index(&records, &metas, &AspectMeta::new());
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
        let counts = write_en_api(&tmp, &records, &metas, &AspectMeta::new(), "test")
            .expect("write english api");
        assert!(tmp.join("api/en/meta.json").exists());
        assert!(tmp
            .join(format!("api/en/{}.json", english_shard_of("game")))
            .exists());
        assert!(counts.keys >= 2);
        assert!(counts.candidates >= 2);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
