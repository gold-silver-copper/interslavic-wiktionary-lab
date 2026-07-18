//! Native-Wiktionary enrichment (RU / PL / CS).
//!
//! The generator's own etymology comes from the English Wiktionary's Proto-Slavic
//! reconstructions (the proto cache). This module adds the *native-language*
//! perspective: for every cognate that appears in our corpus we stream the
//! Russian, Polish and Czech Wiktionary dumps once and keep a compact record of
//! each word's etymology, extra senses, and semantic links (related / derived /
//! synonyms / antonyms). The site shows these per cognate, so each entry carries
//! three independent etymologies, many more meanings, and a web of links back to
//! the source dictionaries — everything downstream reads the cache, never the
//! dumps.

use crate::dump::LemmaCorpus;
use crate::official::OfficialEntry;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// The Wiktionary editions we enrich from; the value is the `lang_code` of that
/// edition's native entries and the subdomain of its site.
pub const ENRICH_LANGS: &[&str] = &["ru", "pl", "cs"];

/// One cognate's native-Wiktionary enrichment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichEntry {
    pub lang: String,
    pub word: String,
    /// Etymology paragraphs, in the native language (Proto-Slavic + PIE + cognates).
    #[serde(default)]
    pub etymology: Vec<String>,
    /// Sense glosses, in the native language (extra meanings beyond the gloss).
    #[serde(default)]
    pub senses: Vec<String>,
    /// Related + derived terms.
    #[serde(default)]
    pub related: Vec<String>,
    #[serde(default)]
    pub synonyms: Vec<String>,
    #[serde(default)]
    pub antonyms: Vec<String>,
    /// Wiktextract category/topic/tag metadata from the native edition. Old
    /// caches deserialize with empty lists; re-run `extract-enrich` to populate.
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Source-language usage quotations from the native edition, each tied to the
    /// sense gloss it illustrates. Old caches deserialize with an empty list;
    /// re-run `extract-enrich` to populate. Skipped when empty so entries without
    /// quotations serialize byte-identically to pre-quotation caches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<Quotation>,
}

/// One native-Wiktionary usage quotation, kept as source-language evidence and
/// tied (by gloss text) to the sense it illustrates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quotation {
    /// The sense gloss text this quotation illustrates — matches an entry in
    /// `EnrichEntry::senses`, so display can render it under the right sense.
    pub sense: String,
    /// The quotation sentence itself, in the native language.
    pub text: String,
    /// A compact citation/source string, when the dump records one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

impl EnrichEntry {
    pub fn is_empty(&self) -> bool {
        self.etymology.is_empty()
            && self.senses.is_empty()
            && self.related.is_empty()
            && self.synonyms.is_empty()
            && self.antonyms.is_empty()
            && self.categories.is_empty()
            && self.topics.is_empty()
            && self.tags.is_empty()
            && self.examples.is_empty()
    }
}

/// Bump when `extract-enrich` changes what goes into [`EnrichCache`] (fields,
/// filters, normalization); see the cache-schema-stamp note in `dump.rs`.
pub const ENRICH_CACHE_SCHEMA: u32 = 0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichCache {
    /// Extractor schema stamp ([`ENRICH_CACHE_SCHEMA`]); pre-stamp caches
    /// deserialize as 0.
    #[serde(default)]
    pub schema: u32,
    pub source: String,
    pub entry_count: usize,
    pub entries: Vec<EnrichEntry>,
}

/// Loaded enrichment cache with an O(1) `(lang, word)` lookup.
pub struct EnrichIndex {
    entries: Vec<EnrichEntry>,
    by_key: HashMap<String, usize>,
}

impl EnrichIndex {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = crate::dump::read_maybe_gz(path)
            .with_context(|| format!("open enrich cache {}", path.display()))?;
        let mut cache: EnrichCache =
            serde_json::from_slice(&bytes).context("parse enrich cache")?;
        crate::dump::check_cache_schema(
            "enrich",
            path,
            cache.schema,
            ENRICH_CACHE_SCHEMA,
            "make extract-enrich",
        )?;
        // Checked before the markup filter below mutates `entries`.
        anyhow::ensure!(
            cache.entry_count == cache.entries.len(),
            "corrupt enrich cache {}: entry_count {} but {} entries",
            path.display(),
            cache.entry_count,
            cache.entries.len()
        );
        // Drop the handful of strings where wiktextract leaked unparsed wiki markup
        // (`[[">*melko< / [[span>#…|span>]]]]`, `''…''`, stray tags) so no page shows
        // garbage. A bare `<` is kept — it is legit descent notation ("*ognь < …").
        for e in &mut cache.entries {
            e.etymology.retain(|s| !looks_like_markup(s));
            e.senses.retain(|s| !looks_like_markup(s));
            e.related.retain(|s| !looks_like_markup(s));
            e.synonyms.retain(|s| !looks_like_markup(s));
            e.antonyms.retain(|s| !looks_like_markup(s));
            // Quotations carry natural-language sentences, but the same leaked
            // wiki/HTML markup can slip into a quote or its citation; drop those.
            e.examples
                .retain(|q| !looks_like_markup(&q.text) && !looks_like_markup(&q.source));
        }
        let mut by_key = HashMap::new();
        for (i, e) in cache.entries.iter().enumerate() {
            by_key.entry(key(&e.lang, &e.word)).or_insert(i);
        }
        Ok(EnrichIndex {
            entries: cache.entries,
            by_key,
        })
    }

    pub fn get(&self, lang: &str, word: &str) -> Option<&EnrichEntry> {
        self.by_key.get(&key(lang, word)).map(|&i| &self.entries[i])
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// True when a string carries leaked wiki/HTML markup (as opposed to a bare `<`
/// used for etymological descent, which is legitimate).
fn looks_like_markup(s: &str) -> bool {
    const M: &[&str] = &[
        "[[", "]]", "{{", "}}", "<span", "</", "span>#", "|span", "<ref", "''",
    ];
    M.iter().any(|m| s.contains(m))
}

/// Accent-stripped, lowercased word used as the lookup key (Russian corpus forms
/// carry stress marks the headword does not: вода́ vs вода).
fn norm_word(word: &str) -> String {
    word.trim()
        .to_lowercase()
        .chars()
        .filter(|c| !('\u{0300}'..='\u{036F}').contains(c))
        .collect()
}

fn key(lang: &str, word: &str) -> String {
    format!("{lang}:{}", norm_word(word))
}

/// Reverse index: `(lang, word)` → the site entry id whose cognate set contains
/// that word. Lets an enrichment chip link to an *internal* dictionary page when
/// the related/synonym term is itself a headword (else it links out to the native
/// Wiktionary), turning the per-entry enrichment into a site-wide semantic graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XrefMatch {
    Missing,
    Unique(usize),
    Ambiguous,
}

#[derive(Default)]
pub struct Xref {
    by_key: HashMap<String, Vec<usize>>,
}

impl Xref {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register that `word` (in `lang`) is a cognate of entry `id`.
    ///
    /// A spelling can belong to several homographs/senses. Preserve every
    /// claimant rather than routing semantic chips according to insertion
    /// order; [`get`](Self::get) returns an internal target only when identity
    /// is unambiguous.
    pub fn insert(&mut self, lang: &str, word: &str, id: usize) {
        let ids = self.by_key.entry(key(lang, word)).or_default();
        if !ids.contains(&id) {
            ids.push(id);
        }
    }

    pub fn lookup(&self, lang: &str, word: &str) -> XrefMatch {
        match self.by_key.get(&key(lang, word)).map(Vec::as_slice) {
            None | Some([]) => XrefMatch::Missing,
            Some([id]) => XrefMatch::Unique(*id),
            Some(_) => XrefMatch::Ambiguous,
        }
    }

    pub fn get(&self, lang: &str, word: &str) -> Option<usize> {
        match self.lookup(lang, word) {
            XrefMatch::Unique(id) => Some(id),
            XrefMatch::Missing | XrefMatch::Ambiguous => None,
        }
    }

    pub fn contains(&self, lang: &str, word: &str) -> bool {
        !matches!(self.lookup(lang, word), XrefMatch::Missing)
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    pub fn ambiguous_len(&self) -> usize {
        self.by_key.values().filter(|ids| ids.len() > 1).count()
    }
}

/// Percent-encode one Wiktionary path/fragment component as UTF-8. Spaces in
/// page titles use Wiktionary's conventional underscore spelling. Encoding is
/// deliberately local (rather than applied to a complete URL) so delimiters
/// such as the language-section `#` retain their structural meaning.
fn wiki_component(value: &str, spaces_as_underscores: bool) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let value = value.trim();
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else if byte == b' ' && spaces_as_underscores {
            out.push('_');
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

/// The native-Wiktionary URL for a word.
pub fn source_url(lang: &str, word: &str) -> String {
    format!(
        "https://{lang}.wiktionary.org/wiki/{}",
        wiki_component(word, true)
    )
}

/// The English-Wiktionary page for an attested word, optionally anchored to a
/// language section. Page and fragment identities are encoded independently.
pub fn english_source_url(word: &str, section: Option<&str>) -> String {
    let mut url = format!(
        "https://en.wiktionary.org/wiki/{}",
        wiki_component(word, true)
    );
    if let Some(section) = section.filter(|s| !s.trim().is_empty()) {
        url.push('#');
        url.push_str(&wiki_component(section, false));
    }
    url
}

/// The English-Wiktionary reconstruction page for a Proto-Slavic form.
pub fn proto_source_url(proto: &str) -> String {
    format!(
        "https://en.wiktionary.org/wiki/Reconstruction:Proto-Slavic/{}",
        wiki_component(proto.trim_start_matches('*'), true)
    )
}

/// The set of cognate words we actually show on the site, per enrich language —
/// the union of the corpus lemma members, the official dictionary's cells, and
/// the RAW low-evidence Slavic lemmas (issue #33). The raw lemmas are unioned so
/// `extract-enrich` also pulls native RU/PL/CS entries for raw words like
/// пластинка, which the raw entry page then merges with the English-dump data.
/// Only ru/pl/cs raw lemmas can match a `wanted` bucket (the only editions with a
/// dump); raw lemmas of any other language are silently ignored, as intended.
pub fn build_wanted(
    lemmas: &LemmaCorpus,
    official: &[OfficialEntry],
    raw: &[crate::dump::RawSlavicLemma],
) -> HashMap<String, HashSet<String>> {
    let mut wanted: HashMap<String, HashSet<String>> = HashMap::new();
    for &l in ENRICH_LANGS {
        wanted.insert(l.to_string(), HashSet::new());
    }
    for e in &lemmas.entries {
        if let Some(set) = wanted.get_mut(e.lang.as_str()) {
            set.insert(norm_word(&e.word));
        }
    }
    for e in official {
        for &l in ENRICH_LANGS {
            if let Some(cell) = e.cells.get(l) {
                for (form, _) in crate::normalize::split_cell(cell) {
                    if let Some(set) = wanted.get_mut(l) {
                        set.insert(norm_word(&form));
                    }
                }
            }
        }
    }
    for e in raw {
        if let Some(set) = wanted.get_mut(e.lang.as_str()) {
            set.insert(norm_word(&e.word));
        }
    }
    wanted
}

/// Stream the per-edition wiktextract dumps in `dir` and cache enrichment for
/// every wanted cognate word.
pub fn extract(dir: &Path, wanted: &HashMap<String, HashSet<String>>, out: &Path) -> Result<()> {
    // Merge multiple POS entries of the same word (noun + verb) into one record.
    let mut merged: HashMap<String, EnrichEntry> = HashMap::new();

    for &lang in ENRICH_LANGS {
        let path = dir.join(format!("{lang}-extract.jsonl"));
        if !path.exists() {
            eprintln!("  (skip {lang}: {} not found)", path.display());
            continue;
        }
        let Some(want) = wanted.get(lang) else {
            continue;
        };
        let file = File::open(&path).with_context(|| format!("open {}", path.display()))?;
        let reader = BufReader::with_capacity(8 * 1024 * 1024, file);
        let marker = format!("\"lang_code\": \"{lang}\"");
        let (mut kept, mut lines) = (0usize, 0u64);
        for line in reader.lines() {
            let line = line?;
            lines += 1;
            // Cheap prefilter: the native entries of this edition carry its
            // lang_code (foreign entries and pure redirects are skipped).
            if !line.contains(&marker) {
                continue;
            }
            let v: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("lang_code").and_then(Value::as_str) != Some(lang) {
                continue; // the marker matched a nested translation, not the headword
            }
            let Some(word) = v.get("word").and_then(Value::as_str) else {
                continue;
            };
            let nk = norm_word(word);
            if !want.contains(&nk) {
                continue;
            }
            let entry = entry_from_value(&v, lang, word);
            if entry.is_empty() {
                continue;
            }
            merged
                .entry(format!("{lang}:{nk}"))
                .and_modify(|e| merge(e, &entry))
                .or_insert(entry);
            kept += 1;
            if lines % 1_000_000 == 0 {
                eprintln!("  {lang}: {} unique / {lines} lines scanned", merged.len());
            }
        }
        eprintln!(
            "  {lang}: {kept} entries scanned, {} unique so far",
            merged.len()
        );
    }

    let mut entries: Vec<EnrichEntry> = merged.into_values().collect();
    entries.sort_by(|a, b| {
        (a.lang.as_str(), a.word.as_str()).cmp(&(b.lang.as_str(), b.word.as_str()))
    });
    let cache = EnrichCache {
        schema: ENRICH_CACHE_SCHEMA,
        source: "per-edition wiktextract (ru/pl/cs Wiktionary)".to_string(),
        entry_count: entries.len(),
        entries,
    };
    crate::dump::write_gz(out, &serde_json::to_vec(&cache)?)?;
    eprintln!(
        "Wrote {} enrichment entries to {}",
        cache.entry_count,
        out.display()
    );
    Ok(())
}

/// Merge a second record of the same word (different POS) into the first.
fn merge(into: &mut EnrichEntry, other: &EnrichEntry) {
    let push_new = |dst: &mut Vec<String>, src: &[String], cap: usize| {
        for s in src {
            if dst.len() >= cap {
                break;
            }
            if !dst.iter().any(|x| x == s) {
                dst.push(s.clone());
            }
        }
    };
    push_new(&mut into.etymology, &other.etymology, 4);
    push_new(&mut into.senses, &other.senses, 10);
    push_new(&mut into.related, &other.related, 48);
    push_new(&mut into.synonyms, &other.synonyms, 16);
    push_new(&mut into.antonyms, &other.antonyms, 10);
    push_new(&mut into.categories, &other.categories, 32);
    push_new(&mut into.topics, &other.topics, 24);
    push_new(&mut into.tags, &other.tags, 24);
    // Quotations: dedup by quote text, cap the merged list.
    for q in &other.examples {
        if into.examples.len() >= EXAMPLES_PER_ENTRY {
            break;
        }
        if !into.examples.iter().any(|x| x.text == q.text) {
            into.examples.push(q.clone());
        }
    }
}

/// Distil one wiktextract entry into a compact enrichment record.
fn entry_from_value(v: &Value, lang: &str, word: &str) -> EnrichEntry {
    let etymology = str_list(v.get("etymology_texts"))
        .into_iter()
        .map(|s| truncate(&s, 600))
        .take(3)
        .collect();

    let mut senses = Vec::new();
    let mut examples: Vec<Quotation> = Vec::new();
    if let Some(arr) = v.get("senses").and_then(Value::as_array) {
        for s in arr {
            if senses.len() >= 8 {
                break;
            }
            if let Some(g) = s.get("glosses").and_then(Value::as_array) {
                let joined: Vec<String> = g
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|x| x.to_string())
                    .collect();
                let text = truncate(&joined.join("; "), 220);
                if !text.trim().is_empty() && !senses.contains(&text) {
                    senses.push(text.clone());
                }
                // Usage quotations attached to this sense, tied by gloss text so
                // display can render them under the matching numbered sense.
                if !text.trim().is_empty() {
                    collect_examples(s, lang, &text, &mut examples);
                }
            }
        }
    }

    // related + derived collapse into one "related" list; keep them distinct words.
    let mut related = word_list(v.get("related"), 48);
    for w in word_list(v.get("derived"), 48) {
        if related.len() >= 48 {
            break;
        }
        if !related.contains(&w) {
            related.push(w);
        }
    }

    let (categories, topics, tags) = wiki_metadata(v);
    EnrichEntry {
        lang: lang.to_string(),
        word: word.to_string(),
        etymology,
        senses,
        related,
        synonyms: word_list(v.get("synonyms"), 16),
        antonyms: word_list(v.get("antonyms"), 10),
        categories,
        topics,
        tags,
        examples,
    }
}

/// Max quotations kept per source entry, and per individual sense.
const EXAMPLES_PER_ENTRY: usize = 6;
const EXAMPLES_PER_SENSE: usize = 2;

/// Read the `examples[]` of one wiktextract sense object and append the usable
/// usage quotations, each tied (by gloss text) to `sense_text`.
fn collect_examples(sense: &Value, lang: &str, sense_text: &str, out: &mut Vec<Quotation>) {
    if out.len() >= EXAMPLES_PER_ENTRY {
        return;
    }
    let Some(arr) = sense.get("examples").and_then(Value::as_array) else {
        return;
    };
    let mut per_sense = 0usize;
    for ex in arr {
        if out.len() >= EXAMPLES_PER_ENTRY || per_sense >= EXAMPLES_PER_SENSE {
            break;
        }
        let Some(text) = ex.get("text").and_then(Value::as_str) else {
            continue;
        };
        let text = text.trim();
        // Skip fragments and — for RU — foreign-script text (letter entries carry
        // translation examples in other Cyrillic-using languages).
        if text.chars().count() < 6 {
            continue;
        }
        if lang == "ru" && !text.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)) {
            continue;
        }
        let quote = truncate(text, 240);
        if out.iter().any(|q| q.text == quote) {
            continue;
        }
        // A compact citation, when present and meaningful ("table" is a template
        // artifact, not a real source).
        let source = ex
            .get("ref")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|r| r.chars().count() >= 3 && *r != "table")
            .map(|r| truncate(r, 160))
            .unwrap_or_default();
        out.push(Quotation {
            sense: sense_text.to_string(),
            text: quote,
            source,
        });
        per_sense += 1;
    }
}

fn wiki_metadata(value: &Value) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut categories = string_values(value.get("categories"), 24);
    let mut topics = string_values(value.get("topics"), 16);
    let mut tags = string_values(value.get("tags"), 16);
    push_limited(&mut tags, string_values(value.get("raw_tags"), 8), 24);
    if let Some(senses) = value.get("senses").and_then(Value::as_array) {
        for sense in senses {
            push_limited(
                &mut categories,
                string_values(sense.get("categories"), 12),
                32,
            );
            push_limited(&mut topics, string_values(sense.get("topics"), 12), 24);
            push_limited(&mut tags, string_values(sense.get("tags"), 12), 28);
            push_limited(&mut tags, string_values(sense.get("raw_tags"), 8), 32);
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

/// Extract the `word` field of each object in a list field (related/synonyms/…).
fn word_list(v: Option<&Value>, cap: usize) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(arr) = v.and_then(Value::as_array) {
        for item in arr {
            if out.len() >= cap {
                break;
            }
            if let Some(w) = item.get("word").and_then(Value::as_str) {
                let w = w.trim();
                if w.chars().count() >= 2 && !out.iter().any(|x| x == w) {
                    out.push(w.to_string());
                }
            }
        }
    }
    out
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_word_strips_accents_and_case() {
        assert_eq!(norm_word("Вода\u{0301}"), "вода");
        assert_eq!(norm_word("  Woda "), "woda");
    }

    #[test]
    fn source_urls_encode_page_and_fragment_identity() {
        assert_eq!(
            source_url("ru", "вода"),
            "https://ru.wiktionary.org/wiki/%D0%B2%D0%BE%D0%B4%D0%B0"
        );
        assert_eq!(
            source_url("cs", "za slova"),
            "https://cs.wiktionary.org/wiki/za_slova"
        );
        assert_eq!(
            english_source_url("#cząsteczka", Some("pl")),
            "https://en.wiktionary.org/wiki/%23cz%C4%85steczka#pl"
        );
        assert_eq!(
            english_source_url("kdo?", Some("cs")),
            "https://en.wiktionary.org/wiki/kdo%3F#cs"
        );
        assert_eq!(
            proto_source_url("*ľuby"),
            "https://en.wiktionary.org/wiki/Reconstruction:Proto-Slavic/%C4%BEuby"
        );
    }

    #[test]
    fn xref_abstains_when_a_word_has_multiple_entry_claimants() {
        let mut xref = Xref::new();
        xref.insert("ru", "мир", 10);
        xref.insert("ru", "мир", 10); // duplicate membership is still unique
        assert_eq!(xref.lookup("ru", "мир"), XrefMatch::Unique(10));
        assert!(xref.contains("ru", "мир"));
        assert_eq!(xref.ambiguous_len(), 0);

        xref.insert("ru", "мир", 20); // peace/world homographs
        assert_eq!(xref.lookup("ru", "мир"), XrefMatch::Ambiguous);
        assert!(xref.contains("ru", "мир"));
        assert_eq!(xref.get("ru", "мир"), None);
        assert_eq!(xref.ambiguous_len(), 1);
        assert_eq!(xref.len(), 1);
    }

    #[test]
    fn markup_detector_drops_junk_keeps_descent() {
        assert!(looks_like_markup(
            "[[\">*melko< / [[span>#Праславянский|span>]]]]"
        ));
        assert!(looks_like_markup("по + ''том'', аналогично"));
        assert!(looks_like_markup("motykou<span>x</span>"));
        // A bare `<` for etymological descent is legitimate and kept.
        assert!(!looks_like_markup("prasł. *ognь < praindoeur. *ngnis"));
        assert!(!looks_like_markup("От праслав. *vodā, от которого"));
    }

    #[test]
    fn entry_from_value_distils_fields() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"word":"voda","lang_code":"cs",
                "etymology_texts":["Z praslovanského *voda."],
                "senses":[{"glosses":["tekutina"]},{"glosses":["vodstvo"]}],
                "related":[{"word":"vodní"},{"word":"vodník"}],
                "synonyms":[{"word":"H2O"}]}"#,
        )
        .unwrap();
        let e = entry_from_value(&v, "cs", "voda");
        assert_eq!(e.etymology.len(), 1);
        assert_eq!(e.senses, vec!["tekutina", "vodstvo"]);
        assert_eq!(e.related, vec!["vodní", "vodník"]);
        assert_eq!(e.synonyms, vec!["H2O"]);
        assert!(e.examples.is_empty());
        assert!(!e.is_empty());
    }

    #[test]
    fn entry_from_value_captures_quotations_tied_to_sense() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"word":"hlavní","lang_code":"cs",
                "senses":[
                  {"glosses":["nejdůležitější"],
                   "examples":[{"text":"Hlavní příčinou porážky byly chyby.","ref":"Zdroj X"},
                               {"text":"To je hlavní bod."}]},
                  {"glosses":["centrální"],
                   "examples":[{"text":"Hlavní město Španělska je Madrid."}]}
                ]}"#,
        )
        .unwrap();
        let e = entry_from_value(&v, "cs", "hlavní");
        assert_eq!(e.senses, vec!["nejdůležitější", "centrální"]);
        assert_eq!(e.examples.len(), 3);
        // Each quotation is tied to the gloss it illustrates.
        assert_eq!(e.examples[0].sense, "nejdůležitější");
        assert_eq!(e.examples[0].text, "Hlavní příčinou porážky byly chyby.");
        assert_eq!(e.examples[0].source, "Zdroj X");
        assert_eq!(e.examples[1].source, "");
        assert_eq!(e.examples[2].sense, "centrální");
        assert!(!e.is_empty());
    }

    #[test]
    fn quotation_caps_and_filters_apply() {
        // >2 examples on one sense are capped to EXAMPLES_PER_SENSE; the RU
        // Cyrillic guard drops foreign-script text and the "table" ref is dropped.
        let v: serde_json::Value = serde_json::from_str(
            r#"{"word":"дом","lang_code":"ru",
                "senses":[{"glosses":["жилище"],
                   "examples":[
                     {"text":"Мой дом большой и светлый.","ref":"table"},
                     {"text":"Этот дом стоит у реки давно.","ref":"Автор, книга"},
                     {"text":"Третий русский пример про дом здесь."},
                     {"text":"Latin only sentence, no Cyrillic."}
                   ]}]}"#,
        )
        .unwrap();
        let e = entry_from_value(&v, "ru", "дом");
        assert_eq!(e.examples.len(), 2, "per-sense cap");
        assert_eq!(e.examples[0].source, "", "\"table\" ref dropped");
        assert_eq!(e.examples[1].source, "Автор, книга");
        assert!(
            e.examples.iter().all(|q| q.sense == "жилище"),
            "all tied to the gloss"
        );
        assert!(
            !e.examples.iter().any(|q| q.text.contains("Latin")),
            "non-Cyrillic RU text filtered"
        );
    }
}
