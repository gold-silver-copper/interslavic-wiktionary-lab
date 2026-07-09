//! Proto-Slavic extraction from the raw Wiktextract dump.
//!
//! The dump is ~23 GB, so we stream it exactly once and write a compact cache of
//! the Proto-Slavic (`sla-pro`) reconstructions — their word, glosses, descendant
//! forms, Balto-Slavic / PIE references, and stem class. Everything downstream
//! (linking, the consensus pipeline, the site) reads the cache, never the dump.

use crate::model::Pos;
use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;

// ---------------------------------------------------------------------------
// Gzip-on-disk cache format (issue #34)
//
// The committed caches keep their `.json` filenames but hold a gzip stream, so
// the 147 MB enrich cache fits under GitHub's 100 MB blob limit without Git LFS.
// Loading is format-agnostic: a pre-existing PLAIN cache still parses, and a
// shell-produced `gzip -c` stream decodes identically to what `write_gz` emits
// (both are ordinary gzip). Decompressed bytes equal the exact serde JSON, so
// serialization is byte-for-byte preserved.
// ---------------------------------------------------------------------------

/// Read a cache file, transparently gunzipping it when it begins with the gzip
/// magic bytes (`0x1f 0x8b`). A plain (uncompressed) file is returned as-is, so
/// older non-gzip caches still load. Plain JSON never starts with `0x1f`, so the
/// magic-byte sniff cannot misfire.
pub fn read_maybe_gz(path: &Path) -> std::io::Result<Vec<u8>> {
    let raw = std::fs::read(path)?;
    if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
        let mut out = Vec::new();
        GzDecoder::new(&raw[..]).read_to_end(&mut out)?;
        Ok(out)
    } else {
        Ok(raw)
    }
}

/// Write `bytes` gzip-compressed to `path`, atomically (tmp file + rename),
/// mirroring the plain-JSON atomic-save pattern. The file keeps its `.json`
/// name but holds a gzip stream; [`read_maybe_gz`] auto-detects it on load.
pub fn write_gz(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut enc = GzEncoder::new(File::create(&tmp)?, Compression::default());
    enc.write_all(bytes)?;
    enc.finish()?.flush()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

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

/// The Slavic top-level `lang_code`s the RAW extraction path (issue #33) accepts.
/// Superset of [`SLAVIC_LANGS`], adding the Serbo-Croatian macro-code `sh` (where
/// English Wiktionary actually files hr/sr/bs entries) and Old East Slavic `orv`.
/// This is a SEPARATE, evidence-free path from [`extract_lemmas`]; it must never
/// feed the benchmark, which is why its output uses the distinct
/// [`RawSlavicCorpus`] type and never the benchmark-gated [`LemmaCorpus`].
pub const RAW_SLAVIC_LANGS: &[&str] = &[
    "ru", "uk", "be", "pl", "cs", "sk", "sl", "hr", "sr", "bs", "bg", "mk", "sh", "dsb", "hsb",
    "szl", "csb", "rue", "cu", "orv",
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
    /// The non-Slavic source (`la computare`, `grc τῆле`) for borrowings, else "".
    #[serde(default)]
    pub etymon: String,
    /// English Wiktionary etymology paragraphs for this lemma. These are source
    /// evidence, so the generated site may display them in English.
    #[serde(default)]
    pub etymology: Vec<String>,
    /// Wiktextract top-level and sense-level category names from English Wiktionary.
    /// Old caches deserialize with empty lists; re-run `extract-lemmas` to populate.
    #[serde(default)]
    pub categories: Vec<String>,
    /// Wiktextract topical labels, often the cleanest source for a Wiktionary-like
    /// category tree (e.g. technology/tools/weapons/hunting).
    #[serde(default)]
    pub topics: Vec<String>,
    /// Grammatical/register tags retained as secondary metadata categories.
    #[serde(default)]
    pub tags: Vec<String>,
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
        let bytes =
            read_maybe_gz(path).with_context(|| format!("open lemma corpus {}", path.display()))?;
        serde_json::from_slice(&bytes).context("parse lemma corpus")
    }

    /// Atomic gzip write (tmp file + rename). Decompressed bytes are the exact
    /// serde JSON, so the on-disk data round-trips byte-for-byte.
    pub fn save(&self, path: &Path) -> Result<()> {
        write_gz(path, &serde_json::to_vec(self)?)?;
        Ok(())
    }
}

/// One single-token Slavic dictionary lemma pulled by the RAW extraction path
/// (issue #33), **without** the etymological-evidence filter that
/// [`LemmaEntry`]/[`extract_lemmas`] apply. This deliberately captures
/// low-evidence dictionary words the benchmark corpus drops (e.g. Russian
/// пластинка, whose only ancestry is a native `-ка` suffixation).
///
/// It is a DISTINCT type from [`LemmaEntry`] on purpose: nothing that consumes
/// the benchmark corpus accepts a `RawSlavicLemma`, so accidentally wiring this
/// evidence-free data into the accuracy benchmark is a compile error, not a
/// silent regression. `proto`/`etymon` are informational only here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSlavicLemma {
    pub word: String,
    pub lang: String,
    pub pos: String,
    pub glosses: Vec<String>,
    pub etymology_text: String,
    pub proto: String,
    pub etymon: String,
}

/// The RAW-path corpus. Distinct from [`LemmaCorpus`] (which the benchmark reads)
/// so the two can never be confused at a call site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSlavicCorpus {
    pub lemmas: Vec<RawSlavicLemma>,
}

impl RawSlavicCorpus {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = read_maybe_gz(path)
            .with_context(|| format!("open raw slavic corpus {}", path.display()))?;
        serde_json::from_slice(&bytes).context("parse raw slavic corpus")
    }

    /// Atomic gzip write (tmp file + rename), mirroring [`extract_lemmas`]'s save.
    pub fn save(&self, path: &Path) -> Result<()> {
        write_gz(path, &serde_json::to_vec(self)?)?;
        Ok(())
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
    corpus.save(out)?;
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
    let etymology = lemma_etymology(value);
    let (categories, topics, tags) = wiki_metadata(value);
    Some(LemmaEntry {
        lang: lang.to_string(),
        word,
        pos: pos.to_string(),
        gloss,
        proto,
        etymon,
        etymology,
        categories,
        topics,
        tags,
    })
}

fn lemma_etymology(value: &Value) -> Vec<String> {
    let mut out: Vec<String> = value
        .get("etymology_texts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|s| truncate_chars(s.trim(), 1800))
        .filter(|s| !s.is_empty())
        .take(4)
        .collect();
    if out.is_empty() {
        if let Some(s) = value.get("etymology_text").and_then(Value::as_str) {
            let s = truncate_chars(s.trim(), 2600);
            if !s.is_empty() {
                out.push(s);
            }
        }
    }
    out
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max_chars).collect::<String>())
    }
}

fn wiki_metadata(value: &Value) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut categories = string_values(value.get("categories"), 24);
    let mut topics = string_values(value.get("topics"), 16);
    let mut tags = string_values(value.get("tags"), 16);
    if let Some(senses) = value.get("senses").and_then(Value::as_array) {
        for sense in senses {
            push_limited(
                &mut categories,
                string_values(sense.get("categories"), 12),
                32,
            );
            push_limited(&mut topics, string_values(sense.get("topics"), 12), 24);
            push_limited(&mut tags, string_values(sense.get("tags"), 12), 24);
            push_limited(&mut tags, string_values(sense.get("raw_tags"), 8), 28);
        }
    }
    (categories, topics, tags)
}

fn string_values(v: Option<&Value>, cap: usize) -> Vec<String> {
    let mut out = Vec::new();
    let Some(arr) = v.and_then(Value::as_array) else {
        return out;
    };
    for item in arr {
        if out.len() >= cap {
            break;
        }
        let s = item
            .as_str()
            .or_else(|| item.get("name").and_then(Value::as_str))
            .or_else(|| item.get("category").and_then(Value::as_str))
            .or_else(|| item.get("topic").and_then(Value::as_str))
            .or_else(|| item.get("tag").and_then(Value::as_str));
        let Some(s) = s else { continue };
        let s = s.trim();
        if s.chars().count() >= 2 && !out.iter().any(|x| x == s) {
            out.push(s.to_string());
        }
    }
    out
}

fn push_limited(dst: &mut Vec<String>, src: Vec<String>, cap: usize) {
    for s in src {
        if dst.len() >= cap {
            break;
        }
        if !dst.iter().any(|x| x == &s) {
            dst.push(s);
        }
    }
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

// ---------------------------------------------------------------------------
// RAW single-token Slavic lemma extraction (issue #33, PR-1)
//
// A SEPARATE streaming pass from `extract_lemmas`. It drops the etymological-
// evidence gate (no `proto.is_empty() && etymon.is_empty()` early-return) so it
// keeps low-evidence dictionary words, and instead applies a QUALITY gate
// (content POS + at least one real, non-form-of gloss). Output is the distinct
// `RawSlavicCorpus`, which no benchmark/consensus/proto function accepts.
// ---------------------------------------------------------------------------

/// Stream the dump once and cache every single-token Slavic dictionary lemma that
/// passes the quality gate, WITHOUT the evidence filter. See [`RawSlavicLemma`].
pub fn extract_raw_slavic(dump: &Path, out: &Path) -> Result<()> {
    if !dump.exists() {
        anyhow::bail!("dump not found: {}", dump.display());
    }
    let file = File::open(dump).with_context(|| format!("open {}", dump.display()))?;
    let reader = BufReader::with_capacity(8 * 1024 * 1024, file);

    // Cheap substring prefilter: only fully parse lines whose top-level language
    // is one of the Slavic codes, before the expensive JSON parse. The dump emits
    // `"lang_code": "xx"` with a space after the colon (verified).
    let markers: Vec<String> = RAW_SLAVIC_LANGS
        .iter()
        .map(|c| format!("\"lang_code\": \"{c}\""))
        .collect();

    let mut lemmas: Vec<RawSlavicLemma> = Vec::new();
    let mut line_count: u64 = 0;
    for line in reader.lines() {
        let line = line?;
        line_count += 1;
        if !markers.iter().any(|m| line.contains(m.as_str())) {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(lemma) = raw_slavic_from_value(&value) {
            lemmas.push(lemma);
            if lemmas.len().is_multiple_of(20000) {
                eprintln!(
                    "  collected {} raw Slavic lemmas after {} lines",
                    lemmas.len(),
                    line_count
                );
            }
        }
    }

    let corpus = RawSlavicCorpus { lemmas };
    corpus.save(out)?;
    println!(
        "wrote {} ({} raw Slavic lemmas from {} lines)",
        out.display(),
        corpus.lemmas.len(),
        line_count
    );
    Ok(())
}

/// Apply the RAW gate to one Wiktextract page, returning a [`RawSlavicLemma`] when
/// it is a single-token Slavic content lemma with at least one real gloss.
fn raw_slavic_from_value(value: &Value) -> Option<RawSlavicLemma> {
    // Top-level language must be one of the Slavic codes.
    let lang = value.get("lang_code").and_then(Value::as_str)?;
    if !RAW_SLAVIC_LANGS.contains(&lang) {
        return None;
    }
    // Real lemma page: a `word` plus non-empty `senses` (skips redirect rows).
    let word = value.get("word").and_then(Value::as_str)?.trim().to_string();
    let senses = value.get("senses").and_then(Value::as_array)?;
    if senses.is_empty() {
        return None;
    }
    // Single token: non-empty and no space.
    if word.is_empty() || word.contains(' ') {
        return None;
    }
    // Quality gate: content POS only (drop proper nouns/particles/etc.).
    let raw_pos = value.get("pos").and_then(Value::as_str).unwrap_or("");
    if !matches!(raw_pos, "noun" | "verb" | "adj" | "adv") {
        return None;
    }
    // Quality gate: at least one real (non-form-of) gloss.
    let glosses = real_glosses(senses, 4);
    if glosses.is_empty() {
        return None;
    }
    // Etymology: the English dump uses the SINGULAR `etymology_text`. Informational.
    let etymology_text = value
        .get("etymology_text")
        .and_then(Value::as_str)
        .map(|s| truncate_chars(s.trim(), 2600))
        .unwrap_or_default();
    // Cheaply reuse the evidence parsers; informational only (NOT benchmark-gated).
    let proto = proto_ancestor(value).unwrap_or_default();
    let etymon = if proto.is_empty() {
        borrowed_etymon(value).unwrap_or_default()
    } else {
        String::new()
    };
    Some(RawSlavicLemma {
        word,
        lang: lang.to_string(),
        pos: raw_pos.to_string(),
        glosses,
        etymology_text,
        proto,
        etymon,
    })
}

/// Collect up to `cap` distinct real English glosses, skipping form-of /
/// inflection / alternative-form / abbreviation senses. Returns empty if every
/// sense is merely a pointer to another lemma.
fn real_glosses(senses: &[Value], cap: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for sense in senses {
        if is_form_of_sense(sense) {
            continue;
        }
        let Some(gs) = sense.get("glosses").and_then(Value::as_array) else {
            continue;
        };
        for g in gs.iter().filter_map(Value::as_str) {
            let g = g.trim();
            if g.is_empty() || gloss_marks_form_of(g) {
                continue;
            }
            let g = truncate_chars(g, 200);
            if !out.contains(&g) {
                out.push(g);
                if out.len() >= cap {
                    return out;
                }
            }
        }
    }
    out
}

/// True when a sense is a form-of / inflection / alt-form / abbreviation pointer,
/// per how wiktextract marks them: a populated `form_of`/`alt_of` field, or a
/// form-of tag.
fn is_form_of_sense(sense: &Value) -> bool {
    let has_pointer = |field: &str| {
        sense
            .get(field)
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    };
    if has_pointer("form_of") || has_pointer("alt_of") {
        return true;
    }
    sense
        .get("tags")
        .and_then(Value::as_array)
        .is_some_and(|tags| {
            tags.iter().filter_map(Value::as_str).any(|t| {
                matches!(
                    t,
                    "form-of" | "inflection-of" | "alt-of" | "alternative" | "abbreviation"
                )
            })
        })
}

/// True when a gloss *text* announces itself as a form-of pointer (some
/// wiktextract senses carry the marker only in the gloss, not the tags).
fn gloss_marks_form_of(gloss: &str) -> bool {
    let lc = gloss.to_lowercase();
    lc.starts_with("form of ")
        || lc.starts_with("inflection of ")
        || lc.starts_with("alternative form of ")
        || lc.starts_with("alternative spelling of ")
        || lc.starts_with("abbreviation of ")
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
    write_gz(out, &serde_json::to_vec(&cache)?)?;
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
        let bytes =
            read_maybe_gz(path).with_context(|| format!("open proto cache {}", path.display()))?;
        let cache: ProtoCache = serde_json::from_slice(&bytes).context("parse proto cache")?;
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
            for (lang, form) in &e.descendants {
                for word in form.split_whitespace() {
                    // Transliterate native-script (mostly Cyrillic) descendants so
                    // they share a skeleton space with the Latin-normalized cognates.
                    let sk = crate::normalize::desc_skeleton(lang, word);
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
                etymology: Vec::new(),
                categories: Vec::new(),
                topics: Vec::new(),
                tags: Vec::new(),
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

    #[test]
    fn raw_slavic_keeps_low_evidence_lemma() {
        use serde_json::json;
        // Russian пластинка: no sla-pro/bor evidence (native `-ка` suffixation),
        // so `extract_lemmas` drops it. The RAW path must keep it, skipping the
        // leading form-of sense and taking the real "record, disc" gloss.
        let v = json!({
            "word": "пластинка",
            "lang_code": "ru",
            "pos": "noun",
            "etymology_text": "пласти́на (plastína) + -ка (-ka)",
            "senses": [
                {"glosses": ["diminutive of пласти́на (plastína): plate"],
                 "tags": ["diminutive", "form-of"],
                 "form_of": [{"word": "пласти́на"}]},
                {"glosses": ["record, disc"]},
                {"glosses": ["blade, lamina"]}
            ]
        });
        let lemma = raw_slavic_from_value(&v).expect("пластинка should be captured");
        assert_eq!(lemma.word, "пластинка");
        assert_eq!(lemma.lang, "ru");
        assert_eq!(lemma.pos, "noun");
        assert_eq!(lemma.glosses, vec!["record, disc", "blade, lamina"]);
        assert_eq!(lemma.etymology_text, "пласти́на (plastína) + -ка (-ka)");
    }

    #[test]
    fn raw_slavic_gate_rejects() {
        use serde_json::json;
        // Proper noun (pos "name") is dropped.
        let name = json!({"word": "Москва", "lang_code": "ru", "pos": "name",
                          "senses": [{"glosses": ["Moscow"]}]});
        assert!(raw_slavic_from_value(&name).is_none());
        // Non-Slavic language is dropped.
        let en = json!({"word": "record", "lang_code": "en", "pos": "noun",
                        "senses": [{"glosses": ["a record"]}]});
        assert!(raw_slavic_from_value(&en).is_none());
        // Multi-token is dropped.
        let phrase = json!({"word": "по мере", "lang_code": "ru", "pos": "adv",
                            "senses": [{"glosses": ["gradually"]}]});
        assert!(raw_slavic_from_value(&phrase).is_none());
        // Only form-of senses -> no real gloss -> dropped.
        let only_form = json!({"word": "плаcтинки", "lang_code": "ru", "pos": "noun",
                               "senses": [{"glosses": ["genitive singular of пластинка"],
                                           "tags": ["form-of"],
                                           "form_of": [{"word": "пластинка"}]}]});
        assert!(raw_slavic_from_value(&only_form).is_none());
    }
}
