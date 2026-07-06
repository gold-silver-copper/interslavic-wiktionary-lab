//! Proto-Slavic extraction from the raw Wiktextract dump.
//!
//! The dump is ~23 GB, so we stream it exactly once and write a compact cache of
//! the Proto-Slavic (`sla-pro`) reconstructions — their word, glosses, descendant
//! forms, Balto-Slavic / PIE references, and stem class. Everything downstream
//! (linking, the consensus pipeline, the site) reads the cache, never the dump.

use crate::model::Pos;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// One Proto-Slavic reconstruction, distilled from a `sla-pro` page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtoEntry {
    /// The reconstruction with etymological letters intact (yers, nasals, jat).
    pub word: String,
    pub pos: String,
    pub glosses: Vec<String>,
    /// (lang_code, attested form) pairs flattened from the descendant tree.
    pub descendants: Vec<(String, String)>,
    pub pbs: String,
    pub pie: String,
    pub stem_class: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtoCache {
    pub source: String,
    pub entry_count: usize,
    pub entries: Vec<ProtoEntry>,
}

/// Cheap substring prefilter: only fully parse lines whose top-level language is
/// Proto-Slavic. Descendants of a Proto-Slavic page are modern languages, so the
/// exact field `"lang_code": "sla-pro"` is a good selector; we re-verify after
/// parsing.
const MARKER: &str = "\"lang_code\": \"sla-pro\"";

/// The modern (and near-modern) Slavic languages we collect lemmas for.
pub const SLAVIC_LANGS: &[&str] = &[
    "ru", "uk", "be", "pl", "cs", "sk", "sl", "hr", "sr", "bg", "mk", "bs", "cu", "csb", "szl",
    "dsb", "hsb", "rue",
];

/// One modern-Slavic dictionary lemma tagged with its etymological ancestor, so
/// lemmas sharing an ancestor form a cognate set. A lemma is either **inherited**
/// (`proto` = a Proto-Slavic reconstruction) or a **borrowing / internationalism**
/// (`etymon` = a non-Slavic source such as Latin/Greek/French).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LemmaEntry {
    pub lang: String,
    pub word: String,
    pub pos: String,
    pub gloss: String,
    /// The Proto-Slavic reconstruction (`*orvьnъ`) for inherited lemmas, else "".
    #[serde(default)]
    pub proto: String,
    /// The non-Slavic source (`la computare`, `grc τῆλε`) for borrowings, else "".
    #[serde(default)]
    pub etymon: String,
}

impl LemmaEntry {
    pub fn is_borrowed(&self) -> bool {
        self.proto.is_empty() && !self.etymon.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LemmaCorpus {
    pub source: String,
    pub entry_count: usize,
    pub entries: Vec<LemmaEntry>,
}

impl LemmaCorpus {
    pub fn load(path: &Path) -> Result<Self> {
        use std::io::Read;
        let mut json = String::new();
        File::open(path)
            .with_context(|| format!("open lemma corpus {}", path.display()))?
            .read_to_string(&mut json)?;
        serde_json::from_str(&json).context("parse lemma corpus")
    }
}

/// Stream the dump once and collect every inherited Slavic lemma that Wiktionary
/// links to a Proto-Slavic ancestor. Lemmas grouped by that ancestor become the
/// cognate sets the generator turns into Interslavic words.
pub fn extract_lemmas(dump: &Path, out: &Path) -> Result<()> {
    if !dump.exists() {
        anyhow::bail!("dump not found: {}", dump.display());
    }
    let file = File::open(dump).with_context(|| format!("open {}", dump.display()))?;
    let reader = BufReader::with_capacity(8 * 1024 * 1024, file);

    let mut entries: Vec<LemmaEntry> = Vec::new();
    let mut line_count: u64 = 0;
    for line in reader.lines() {
        let line = line?;
        line_count += 1;
        // Inherited lemmas mention `sla-pro`; borrowings carry a bor/der/lbor
        // template. Cheap prefilter before the full JSON parse.
        if !(line.contains("sla-pro")
            || line.contains("\"bor")
            || line.contains("\"lbor")
            || line.contains("\"der+"))
        {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(entry) = lemma_from_value(&value) {
            entries.push(entry);
            if entries.len() % 5000 == 0 {
                eprintln!(
                    "  collected {} Slavic lemmas after {} lines",
                    entries.len(),
                    line_count
                );
            }
        }
    }

    let corpus = LemmaCorpus {
        source: dump.display().to_string(),
        entry_count: entries.len(),
        entries,
    };
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = out.with_extension("json.tmp");
    let mut f = File::create(&tmp)?;
    serde_json::to_writer(&mut f, &corpus)?;
    f.flush()?;
    std::fs::rename(&tmp, out)?;
    println!(
        "wrote {} ({} Slavic lemmas from {} lines)",
        out.display(),
        corpus.entry_count,
        line_count
    );
    Ok(())
}

fn lemma_from_value(value: &Value) -> Option<LemmaEntry> {
    let lang = value.get("lang_code").and_then(Value::as_str)?;
    if !SLAVIC_LANGS.contains(&lang) {
        return None;
    }
    let word = value
        .get("word")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    // Lemmas only: single token, not a reconstruction, not a phrase.
    if word.is_empty() || word.contains(' ') || word.starts_with('*') || word.starts_with('-') {
        return None;
    }
    let pos = Pos::parse(value.get("pos").and_then(Value::as_str).unwrap_or("")).code();
    // Prefer the inherited (Proto-Slavic) ancestor; else fall back to a non-Slavic
    // borrowing source (internationalisms, Graeco-Latin and other loans).
    let proto = proto_ancestor(value).unwrap_or_default();
    let etymon = if proto.is_empty() {
        borrowed_etymon(value).unwrap_or_default()
    } else {
        String::new()
    };
    if proto.is_empty() && etymon.is_empty() {
        return None;
    }
    let gloss = lemma_gloss(value)?;
    Some(LemmaEntry {
        lang: lang.to_string(),
        word,
        pos: pos.to_string(),
        gloss,
        proto,
        etymon,
    })
}

/// True for a Slavic (or Proto-Slavic) source language — a borrowing *within*
/// Slavic is not an internationalism.
fn is_slavic_src(code: &str) -> bool {
    code == "sla-pro" || code == "sla" || SLAVIC_LANGS.contains(&code)
}

/// The non-Slavic source of a borrowing/derivation, as a display string
/// (`la computare`). Prefers the classical etymon (Latin/Greek) when present, as
/// it groups internationalisms across their varied immediate sources.
fn borrowed_etymon(value: &Value) -> Option<String> {
    let templates = value.get("etymology_templates").and_then(Value::as_array)?;
    let mut best: Option<(u8, String)> = None;
    for t in templates {
        let name = t.get("name").and_then(Value::as_str).unwrap_or("");
        if !matches!(
            name,
            "bor" | "bor+" | "borrowed" | "lbor" | "lbor+" | "der" | "der+" | "derived"
        ) {
            continue;
        }
        let Some(args) = t.get("args") else { continue };
        let src = args.get("2").and_then(Value::as_str).unwrap_or("").trim();
        let word = args.get("3").and_then(Value::as_str).unwrap_or("").trim();
        if src.is_empty() || is_slavic_src(src) {
            continue;
        }
        let word = word.split('<').next().unwrap_or(word).trim();
        if word.is_empty() || word == "-" {
            continue;
        }
        let rank = match src {
            "la" | "ML." | "LL." | "la-med" | "la-lat" => 6,
            "grc" | "el" => 5,
            "it" => 3,
            "fr" | "frm" | "fro" => 3,
            "de" | "gmh" | "nl" => 2,
            "en" => 2,
            _ => 1,
        };
        if best.as_ref().map(|(r, _)| rank > *r).unwrap_or(true) {
            best = Some((rank, format!("{src} {word}")));
        }
    }
    best.map(|(_, s)| s)
}

/// The Proto-Slavic ancestor from an `inh`/`der` etymology template, normalized
/// to a bare reconstruction (`*orvьnъ`).
fn proto_ancestor(value: &Value) -> Option<String> {
    let templates = value.get("etymology_templates").and_then(Value::as_array)?;
    for t in templates {
        let name = t.get("name").and_then(Value::as_str).unwrap_or("");
        if !matches!(
            name,
            "inh" | "inh+" | "inherited" | "der" | "der+" | "derived"
        ) {
            continue;
        }
        let args = t.get("args")?;
        if args.get("2").and_then(Value::as_str) != Some("sla-pro") {
            continue;
        }
        let form = args.get("3").and_then(Value::as_str)?.trim();
        // Wiktextract sometimes carries `<id:...>` qualifiers; drop them.
        let form = form.split('<').next().unwrap_or(form).trim();
        if form.is_empty() || form == "*" {
            continue;
        }
        let form = form.strip_prefix('*').unwrap_or(form);
        // Reject placeholders ("-") and BOUND morphemes (prefixes/suffixes like
        // *per-, *orz-, *-ъkъ): they are not standalone roots, so clustering by
        // them fuses dozens of unrelated lemmas (B9). Require a real word form.
        if form.is_empty()
            || form.starts_with('-')
            || form.ends_with('-')
            || !form.chars().any(|c| c.is_alphabetic())
        {
            continue;
        }
        return Some(format!("*{form}"));
    }
    None
}

/// First real (non-form-of) English gloss.
fn lemma_gloss(value: &Value) -> Option<String> {
    let senses = value.get("senses").and_then(Value::as_array)?;
    for sense in senses {
        if sense.get("form_of").is_some() {
            continue;
        }
        let is_form = sense
            .get("tags")
            .and_then(Value::as_array)
            .map(|tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .any(|t| t == "form-of" || t == "inflection-of")
            })
            .unwrap_or(false);
        if is_form {
            continue;
        }
        if let Some(gs) = sense.get("glosses").and_then(Value::as_array) {
            if let Some(g) = gs.iter().filter_map(Value::as_str).next() {
                let g = g.trim();
                if !g.is_empty() {
                    return Some(g.chars().take(80).collect());
                }
            }
        }
    }
    None
}

pub fn extract(dump: &Path, out: &Path) -> Result<()> {
    if !dump.exists() {
        anyhow::bail!("dump not found: {}", dump.display());
    }
    let file = File::open(dump).with_context(|| format!("open {}", dump.display()))?;
    let reader = BufReader::with_capacity(8 * 1024 * 1024, file);

    let mut entries: Vec<ProtoEntry> = Vec::new();
    let mut line_count: u64 = 0;
    for line in reader.lines() {
        let line = line?;
        line_count += 1;
        if !line.contains(MARKER) {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if value.get("lang_code").and_then(Value::as_str) != Some("sla-pro") {
            continue;
        }
        if let Some(entry) = proto_from_value(&value) {
            entries.push(entry);
            if entries.len() % 2000 == 0 {
                eprintln!(
                    "  extracted {} Proto-Slavic entries after {} lines",
                    entries.len(),
                    line_count
                );
            }
        }
    }

    let cache = ProtoCache {
        source: dump.display().to_string(),
        entry_count: entries.len(),
        entries,
    };
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = out.with_extension("json.tmp");
    let mut f = File::create(&tmp)?;
    serde_json::to_writer(&mut f, &cache)?;
    f.flush()?;
    std::fs::rename(&tmp, out)?;
    println!(
        "wrote {} ({} Proto-Slavic entries from {} lines)",
        out.display(),
        cache.entry_count,
        line_count
    );
    Ok(())
}

fn proto_from_value(value: &Value) -> Option<ProtoEntry> {
    let word = value.get("word").and_then(Value::as_str)?.to_string();
    if word.is_empty() {
        return None;
    }
    let pos = Pos::parse(value.get("pos").and_then(Value::as_str).unwrap_or("")).code();

    let mut glosses: Vec<String> = Vec::new();
    if let Some(senses) = value.get("senses").and_then(Value::as_array) {
        for sense in senses {
            if let Some(gs) = sense.get("glosses").and_then(Value::as_array) {
                for g in gs.iter().filter_map(Value::as_str) {
                    let g = g.trim().to_string();
                    if !g.is_empty() && !glosses.contains(&g) {
                        glosses.push(g);
                    }
                }
            }
        }
    }
    glosses.truncate(8);

    let mut descendants: Vec<(String, String)> = Vec::new();
    collect_descendants(
        value.get("descendants").and_then(Value::as_array),
        &mut descendants,
    );
    // Prefer short (lemma-like) forms; cap to keep the cache compact.
    descendants.sort_by_key(|(_, w)| w.split_whitespace().count());
    descendants.truncate(80);

    let (pbs, pie) = proto_refs(value);
    let stem_class = stem_class(value);

    Some(ProtoEntry {
        word,
        pos: pos.to_string(),
        glosses,
        descendants,
        pbs,
        pie,
        stem_class,
    })
}

fn collect_descendants(nodes: Option<&Vec<Value>>, out: &mut Vec<(String, String)>) {
    let Some(nodes) = nodes else { return };
    for node in nodes {
        let code = node.get("lang_code").and_then(Value::as_str).unwrap_or("");
        let word = node.get("word").and_then(Value::as_str).unwrap_or("");
        if !code.is_empty() && !word.is_empty() {
            out.push((code.to_string(), word.to_string()));
        }
        collect_descendants(node.get("descendants").and_then(Value::as_array), out);
    }
}

fn proto_refs(value: &Value) -> (String, String) {
    let mut text = value
        .get("etymology_text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Some(ts) = value.get("etymology_templates").and_then(Value::as_array) {
        for t in ts {
            text.push('\n');
            text.push_str(t.get("expansion").and_then(Value::as_str).unwrap_or(""));
        }
    }
    (
        after_needle(&text, "Proto-Balto-Slavic"),
        after_needle(&text, "Proto-Indo-European"),
    )
}

fn after_needle(text: &str, needle: &str) -> String {
    let Some(idx) = text.find(needle) else {
        return String::new();
    };
    let rest = &text[idx + needle.len()..];
    let Some(star) = rest.find('*') else {
        return String::new();
    };
    rest[star..]
        .split(|c: char| c.is_whitespace() || [',', ';', ']', ')'].contains(&c))
        .next()
        .unwrap_or("")
        .to_string()
}

fn stem_class(value: &Value) -> Option<String> {
    let cats = value.get("categories").and_then(Value::as_array)?;
    for c in cats.iter().filter_map(Value::as_str) {
        let lc = c.to_lowercase();
        for key in [
            "o-stem",
            "a-stem",
            "ā-stem",
            "i-stem",
            "u-stem",
            "n-stem",
            "s-stem",
            "r-stem",
            "jo-stem",
            "ja-stem",
            "consonant stem",
        ] {
            if lc.contains(key) {
                return Some(c.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Cache loading + indexes
// ---------------------------------------------------------------------------

/// In-memory Proto-Slavic index used by the linker.
pub struct ProtoIndex {
    pub entries: Vec<ProtoEntry>,
    /// gloss word token -> entry indices.
    by_gloss_token: HashMap<String, Vec<usize>>,
    /// descendant form skeleton -> entry indices (whole-word tokens).
    by_desc_skeleton: HashMap<String, Vec<usize>>,
    /// reconstruction word -> entry index (for exact ancestor lookup).
    by_word: HashMap<String, usize>,
    /// Wiktionary's *explicit* etymology: "lang\u{1}phonemic-latin" -> ancestor
    /// reconstruction (`*voda`). Built from the lemma corpus when present. Lets the
    /// linker use the attested ancestor directly instead of guessing.
    etym: HashMap<String, String>,
}

impl ProtoIndex {
    pub fn load(path: &Path) -> Result<Self> {
        let mut json = String::new();
        use std::io::Read;
        File::open(path)
            .with_context(|| format!("open proto cache {}", path.display()))?
            .read_to_string(&mut json)?;
        let cache: ProtoCache = serde_json::from_str(&json).context("parse proto cache")?;
        let mut idx = Self::build(cache.entries);
        // Attach Wiktionary's explicit (lang, lemma) -> ancestor etymology if the
        // lemma corpus is available next to the proto cache.
        let lemma_path = Path::new(crate::DEFAULT_LEMMA_CACHE);
        if lemma_path.exists() {
            if let Ok(corpus) = LemmaCorpus::load(lemma_path) {
                idx.attach_etymology(&corpus);
            }
        }
        Ok(idx)
    }

    fn attach_etymology(&mut self, corpus: &LemmaCorpus) {
        for e in &corpus.entries {
            if e.proto.is_empty() || !self.by_word.contains_key(e.proto.trim_start_matches('*')) {
                continue; // only ancestors we actually have a reconstruction for
            }
            let latin = crate::normalize::to_phonemic_latin(&e.lang, &e.word);
            if latin.is_empty() {
                continue;
            }
            self.etym
                .entry(format!("{}\u{1}{latin}", e.lang))
                .or_insert_with(|| e.proto.clone());
        }
    }

    /// The explicitly-attested Proto-Slavic ancestor of a modern lemma, if any.
    pub fn etym_ancestor(&self, lang: &str, latin: &str) -> Option<&str> {
        self.etym
            .get(&format!("{lang}\u{1}{latin}"))
            .map(|s| s.as_str())
    }

    /// The entry index for a reconstruction word (`voda`, no `*`).
    pub fn entry_by_word(&self, word: &str) -> Option<usize> {
        self.by_word.get(word).copied()
    }

    pub fn build(entries: Vec<ProtoEntry>) -> Self {
        let mut by_gloss_token: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_desc_skeleton: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_word: HashMap<String, usize> = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            by_word.entry(e.word.clone()).or_insert(i);
            for g in &e.glosses {
                for tok in gloss_tokens(g) {
                    by_gloss_token.entry(tok).or_default().push(i);
                }
            }
            for (_, form) in &e.descendants {
                for word in form.split_whitespace() {
                    let sk = crate::orthography::ascii_skeleton(word);
                    if sk.len() >= 2 {
                        by_desc_skeleton.entry(sk).or_default().push(i);
                    }
                }
            }
        }
        ProtoIndex {
            entries,
            by_gloss_token,
            by_desc_skeleton,
            by_word,
            etym: HashMap::new(),
        }
    }

    pub fn gloss_candidates(&self, gloss: &str) -> Vec<usize> {
        let mut seen = Vec::new();
        for tok in gloss_tokens(gloss) {
            if let Some(v) = self.by_gloss_token.get(&tok) {
                for &i in v {
                    if !seen.contains(&i) {
                        seen.push(i);
                    }
                }
            }
        }
        seen
    }

    pub fn desc_candidates(&self, skeleton: &str) -> Option<&Vec<usize>> {
        self.by_desc_skeleton.get(skeleton)
    }
}

/// Lowercase content-word gloss tokens (drop stopwords and short tokens).
pub fn gloss_tokens(gloss: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "the", "a", "an", "to", "of", "and", "or", "in", "on", "for", "with", "be", "is", "as",
        "at", "by", "that", "this", "it", "one", "some", "any", "esp", "e", "g",
    ];
    gloss
        .to_lowercase()
        .split(|c: char| !c.is_alphabetic())
        .filter(|t| t.len() >= 3 && !STOP.contains(t))
        .map(|t| t.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_etymology_lookup() {
        let e = ProtoEntry {
            word: "voda".into(),
            pos: "noun".into(),
            glosses: vec!["water".into()],
            descendants: vec![],
            pbs: String::new(),
            pie: String::new(),
            stem_class: None,
        };
        let mut idx = ProtoIndex::build(vec![e]);
        let corpus = LemmaCorpus {
            source: String::new(),
            entry_count: 1,
            entries: vec![LemmaEntry {
                lang: "ru".into(),
                word: "вода".into(),
                pos: "noun".into(),
                gloss: "water".into(),
                proto: "*voda".into(),
                etymon: String::new(),
            }],
        };
        idx.attach_etymology(&corpus);
        assert_eq!(idx.etym_ancestor("ru", "voda"), Some("*voda"));
        assert!(idx.entry_by_word("voda").is_some());
        assert_eq!(idx.etym_ancestor("ru", "нет"), None);
    }

    #[test]
    fn proto_ancestor_rejects_bound_morphemes() {
        use serde_json::json;
        let v = json!({"etymology_templates":[{"name":"inh","args":{"2":"sla-pro","3":"*orz-"}}]});
        assert_eq!(proto_ancestor(&v), None);
        let v2 = json!({"etymology_templates":[{"name":"inh","args":{"2":"sla-pro","3":"*voda"}}]});
        assert_eq!(proto_ancestor(&v2).as_deref(), Some("*voda"));
    }
}
