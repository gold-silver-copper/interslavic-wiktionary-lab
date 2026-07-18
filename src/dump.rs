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
use std::collections::{BTreeMap, HashMap};
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

// ---------------------------------------------------------------------------
// Cache schema stamps
//
// The committed caches used to load through bare serde with `#[serde(default)]`
// fields, so a cache built by an OLDER extractor deserialized fine and any
// newer field silently came back empty (empty categories, missing quotations,
// …) — nothing failed until a reader noticed data was missing. Each cache now
// carries a `schema` stamp checked at load time against the constant declared
// next to its struct.
//
// The rule: when an extractor change alters WHAT goes into a cache (a new
// field, a changed filter or normalization), bump that cache's
// `*_CACHE_SCHEMA` constant in the same commit. Every loader — local runs, CI
// test builds, the Pages deploy — then refuses the stale committed cache with
// the exact `make` target to re-run, instead of shipping silently degraded
// data. Caches stamped before this scheme deserialize as schema 0, which is
// deliberately the initial expected value: the caches committed when the
// stamps were introduced WERE current, so no restamp-only re-commit of ~50 MB
// of gzip was needed.
// ---------------------------------------------------------------------------

/// Load a cache that is allowed to be absent: `None` when the file doesn't
/// exist (callers degrade with their own notice), but a file that EXISTS and
/// fails to load — corrupt, or refused by its schema stamp — is a hard error.
/// Callers must NOT collapse that error to `None` (`.ok()` /
/// `.unwrap_or_default()`), or a stale cache silently degrades the output,
/// which is exactly what the schema stamps exist to prevent.
pub fn load_optional<T>(path: &Path, load: impl FnOnce(&Path) -> Result<T>) -> Result<Option<T>> {
    if path.exists() {
        load(path).map(Some)
    } else {
        Ok(None)
    }
}

/// Loader-side check for the cache schema stamps above. `regen` is the exact
/// `make` target that rebuilds the cache.
pub(crate) fn check_cache_schema(
    kind: &str,
    path: &Path,
    found: u32,
    expected: u32,
    regen: &str,
) -> Result<()> {
    anyhow::ensure!(
        found == expected,
        "stale {kind} cache {}: built with schema {found} but this binary expects {expected} — \
         regenerate it with `{regen}` and commit the result",
        path.display()
    );
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

/// Bump when `extract-proto` changes what goes into [`ProtoCache`] (fields,
/// filters, normalization); see the cache-schema-stamp note above.
/// Schema 1: `stem_class` also scans sense-level categories (issue #76) —
/// the declension category almost never sits on the page level.
pub const PROTO_CACHE_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtoCache {
    /// Extractor schema stamp ([`PROTO_CACHE_SCHEMA`]); pre-stamp caches
    /// deserialize as 0.
    #[serde(default)]
    pub schema: u32,
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
/// Includes the Serbo-Croatian macro-code `sh` — the code English Wiktionary
/// actually files hr/sr/bs entries under (the individual codes exist but carry
/// almost no entries), so omitting it silences the whole language in the
/// cognate-set corpus.
pub const SLAVIC_LANGS: &[&str] = &[
    "ru", "uk", "be", "pl", "cs", "sk", "sl", "hr", "sr", "bg", "mk", "bs", "sh", "cu", "csb",
    "szl", "dsb", "hsb", "rue",
];

/// The Slavic top-level `lang_code`s the RAW extraction path (issue #33) accepts.
/// Superset of [`SLAVIC_LANGS`], adding Old East Slavic `orv`. This is a
/// SEPARATE, evidence-free path from [`extract_lemmas`]; it must never feed the
/// benchmark, which is why its output uses the distinct [`RawSlavicCorpus`] type
/// and never the benchmark-gated [`LemmaCorpus`].
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

/// Bump when `extract-lemmas` changes what goes into [`LemmaCorpus`] (fields,
/// filters, normalization); see the cache-schema-stamp note above.
/// Schema 1: same-language OLD-STAGE inheritance chains (issue #86) — lemmas
/// whose only ancestry is `inh|pl|zlw-opl|…` (or the newer `ety`
/// etymology-tree template) are no longer dropped; they resolve through the
/// old-stage page's own etymology to a proto/etymon, or stay as
/// evidence-attested chain lemmas with both fields empty. Existing keeps are
/// byte-identical — the gate only ADDS entries (appended after the stream).
pub const LEMMA_CACHE_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LemmaCorpus {
    /// Extractor schema stamp ([`LEMMA_CACHE_SCHEMA`]); pre-stamp caches
    /// deserialize as 0.
    #[serde(default)]
    pub schema: u32,
    pub source: String,
    pub entry_count: usize,
    pub entries: Vec<LemmaEntry>,
}

impl LemmaCorpus {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes =
            read_maybe_gz(path).with_context(|| format!("open lemma corpus {}", path.display()))?;
        let mut corpus: Self = serde_json::from_slice(&bytes).context("parse lemma corpus")?;
        check_cache_schema(
            "lemma",
            path,
            corpus.schema,
            LEMMA_CACHE_SCHEMA,
            "make extract-lemmas",
        )?;
        anyhow::ensure!(
            corpus.entry_count == corpus.entries.len(),
            "corrupt lemma cache {}: entry_count {} but {} entries",
            path.display(),
            corpus.entry_count,
            corpus.entries.len()
        );
        // Load-time hygiene (issues #66/#89): cached protos can carry Cyrillic
        // lookalikes or URL-escaped UTF-8 copied from a Wiktionary template.
        // Clean them at this single ingress so every downstream consumer sees
        // linguistic text rather than transport encoding. The cache stays
        // verbatim, so this correction needs no regeneration/schema bump.
        for e in &mut corpus.entries {
            if !e.proto.is_empty() {
                e.proto = decode_percent_utf8(&e.proto);
                e.proto = crate::normalize::fold_proto_homoglyphs(&e.proto);
            }
        }
        Ok(corpus)
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

/// Bump when `extract-raw-slavic` changes what goes into [`RawSlavicCorpus`]
/// (fields, filters, normalization); see the cache-schema-stamp note above.
pub const RAW_CACHE_SCHEMA: u32 = 0;

/// The RAW-path corpus. Distinct from [`LemmaCorpus`] (which the benchmark reads)
/// so the two can never be confused at a call site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSlavicCorpus {
    /// Extractor schema stamp ([`RAW_CACHE_SCHEMA`]); pre-stamp caches
    /// deserialize as 0.
    #[serde(default)]
    pub schema: u32,
    pub lemmas: Vec<RawSlavicLemma>,
}

impl RawSlavicCorpus {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = read_maybe_gz(path)
            .with_context(|| format!("open raw slavic corpus {}", path.display()))?;
        let corpus: Self = serde_json::from_slice(&bytes).context("parse raw slavic corpus")?;
        check_cache_schema(
            "raw-slavic",
            path,
            corpus.schema,
            RAW_CACHE_SCHEMA,
            "make extract-raw-slavic",
        )?;
        Ok(corpus)
    }

    /// Atomic gzip write (tmp file + rename), mirroring [`extract_lemmas`]'s save.
    pub fn save(&self, path: &Path) -> Result<()> {
        write_gz(path, &serde_json::to_vec(self)?)?;
        Ok(())
    }
}

/// Companion filename written next to the raw cache holding the extraction
/// coverage tally (issue #35). Deterministic; committed for auditability.
pub const RAW_COVERAGE_FILE: &str = "raw-slavic-coverage.json";

/// Transparent, auditable coverage of the RAW extraction pass: over the
/// Slavic-language pages [`extract_raw_slavic`] sees in the English Wiktextract
/// dump, how many were KEPT as raw lemmas and how many were DROPPED, broken down
/// by reason. Written alongside the cache; small, so it stays plain JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RawCoverageStats {
    /// The dump path this tally was produced from.
    pub source: String,
    /// Total lines streamed from the dump.
    pub lines_scanned: u64,
    /// Slavic-language pages examined: marker-matched lines whose top-level
    /// `lang_code` is actually a RAW Slavic code (the coverage denominator).
    pub slavic_pages_seen: u64,
    /// Pages kept as raw lemmas (equals the cache length).
    pub kept: u64,
    /// Dropped: a redirect-like page with no `senses` (nothing to define).
    pub dropped_redirect_no_senses: u64,
    /// Dropped: multi-token headword (contains a space) or empty.
    pub dropped_multiword: u64,
    /// Dropped: not a content part of speech (proper noun, particle, etc.).
    pub dropped_non_content_pos: u64,
    /// Dropped: only form-of / inflection senses — no real gloss.
    pub dropped_no_real_gloss: u64,
    /// Kept lemmas per Slavic language code (BTreeMap → deterministic order).
    pub kept_by_lang: BTreeMap<String, u64>,
}

impl RawCoverageStats {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = read_maybe_gz(path)
            .with_context(|| format!("open raw coverage stats {}", path.display()))?;
        serde_json::from_slice(&bytes).context("parse raw coverage stats")
    }

    /// The sum of every drop bucket — must reconcile with
    /// `slavic_pages_seen - kept`.
    pub fn dropped_total(&self) -> u64 {
        self.dropped_redirect_no_senses
            + self.dropped_multiword
            + self.dropped_non_content_pos
            + self.dropped_no_real_gloss
    }

    /// Plain pretty JSON written atomically (tmp + rename). Tiny and
    /// human-auditable, so it is committed uncompressed.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Same-language OLD-STAGE inheritance chains (issue #86 item 4)
//
// en.wiktionary often records a modern lemma's ancestry only through the
// language's own historical stage: pl *aloes* says "Inherited from Old Polish
// aloes" and stops; the foreign etymon (frm aloès) lives on the OLD POLISH
// page. The old gate (`proto` or non-Slavic `etymon`, else drop) silently
// deleted this whole evidence class. The extractor now (a) harvests every
// old-stage page's own proto/etymon/parent while streaming, (b) turns
// evidence-less modern lemmas whose etymology names a same-language old stage
// into PENDING chain lemmas, and (c) resolves them after the stream: old-stage
// chain reaches sla-pro → inherited; reaches a foreign etymon → borrowing;
// unresolvable → kept with both fields empty (an attested chain lemma the
// borrowing skeleton layer can still group; see corpus::build_sets).
// Existing keeps are byte-identical — this only ADDS entries.
// ---------------------------------------------------------------------------

/// Modern corpus language → the en.wiktionary codes of ITS OWN historical
/// stages. Measured inventory of the current dump (classic `inh` + `ety`
/// tree templates): cs←zlw-ocs 6,976; ru←orv 2,986 / zle-mru 121;
/// pl←zlw-opl 4,654; uk←zle-ort 1,646 / orv 1,500 / zle-muk 203;
/// be←zle-ort 731 / orv 607 / zle-mbe 186; sk←zlw-osk 387;
/// szl←zlw-opl 734; rue←orv/zle-ort/zle-muk ~183; bg←cu 945; mk←cu 37.
/// The brief's assumed map (pl/cs/ru/uk/be, three codes) was extended to
/// what actually occurs. Order matters: earlier codes are preferred when a
/// page names several stages.
const OLD_STAGE_OF: &[(&str, &[&str])] = &[
    ("pl", &["zlw-opl"]),
    ("szl", &["zlw-opl"]),
    ("cs", &["zlw-ocs"]),
    ("sk", &["zlw-osk"]),
    ("ru", &["zle-mru", "orv"]),
    ("uk", &["zle-muk", "zle-ort", "orv"]),
    ("be", &["zle-mbe", "zle-ort", "orv"]),
    ("rue", &["zle-muk", "zle-ort", "orv"]),
    ("bg", &["cu"]),
    ("mk", &["cu"]),
];

/// Every old-stage code in [`OLD_STAGE_OF`] — the page languages the
/// streaming pass harvests ancestry from. `cu` doubles as a corpus language
/// (SLAVIC_LANGS) and an old stage of bg/mk; both roles apply to its pages.
/// Old stages chain among themselves (zle-ort pages inherit from orv), so
/// the resolver walks parents through this same set.
const OLD_STAGE_LANGS: &[&str] = &[
    "zlw-opl", "zlw-ocs", "zlw-osk", "orv", "zle-ort", "zle-muk", "zle-mbe", "zle-mru", "cu",
];

fn old_codes_for(lang: &str) -> &'static [&'static str] {
    OLD_STAGE_OF
        .iter()
        .find(|(l, _)| *l == lang)
        .map(|(_, codes)| *codes)
        .unwrap_or(&[])
}

/// Join key for chain resolution: etymology templates cite old-stage words
/// with combining accents (orv дѣ́дъ) that page titles lack — strip them and
/// case-fold so the citation matches the harvested page.
fn chain_key(word: &str) -> String {
    word.trim()
        .chars()
        .filter(|c| !('\u{0300}'..='\u{036F}').contains(c))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Parse one `ety` template — the newer {{etymon}} "Etymology tree" format
/// en.wiktionary is migrating to (most Polish chains already use it):
/// `{"name":"ety","args":{"1":"pl","2":":inh","3":"zlw-opl:aloes<ref:…>"}}`
/// → `(":inh", "zlw-opl", "aloes")`. Returns None for other templates.
fn ety_parts(t: &Value) -> Option<(&str, &str, &str)> {
    if t.get("name").and_then(Value::as_str) != Some("ety") {
        return None;
    }
    let args = t.get("args")?;
    let op = args.get("2").and_then(Value::as_str)?.trim();
    let target = args.get("3").and_then(Value::as_str)?.trim();
    let (src, word) = target.split_once(':')?;
    let word = word.split('<').next().unwrap_or(word).trim();
    if src.is_empty() || word.is_empty() || word == "-" {
        return None;
    }
    Some((op, src.trim(), word))
}

/// The same-language old-stage parent a modern lemma's etymology names —
/// classic `{{inh|pl|zlw-opl|aloes}}` or `ety` `:inh zlw-opl:aloes`. Only
/// consulted when the direct proto/etymon parsers found nothing, so the
/// existing keeps cannot change.
fn old_stage_parent(value: &Value, lang: &str) -> Option<(String, String)> {
    let codes = old_codes_for(lang);
    if codes.is_empty() {
        return None;
    }
    let templates = value.get("etymology_templates").and_then(Value::as_array)?;
    for t in templates {
        let name = t.get("name").and_then(Value::as_str).unwrap_or("");
        if matches!(name, "inh" | "inh+" | "inherited") {
            let Some(args) = t.get("args") else { continue };
            let src = args.get("2").and_then(Value::as_str).unwrap_or("").trim();
            if !codes.contains(&src) {
                continue;
            }
            let word = args.get("3").and_then(Value::as_str).unwrap_or("").trim();
            let word = word.split('<').next().unwrap_or(word).trim();
            if word.is_empty() || word == "-" {
                continue;
            }
            return Some((src.to_string(), word.to_string()));
        }
        if let Some((op, src, word)) = ety_parts(t) {
            if op == ":inh" && codes.contains(&src) {
                return Some((src.to_string(), word.to_string()));
            }
        }
    }
    None
}

/// What one old-stage page knows about its own ancestry.
#[derive(Debug, Clone, Default)]
struct OldStageInfo {
    /// Proto-Slavic reconstruction (`*aloe` shape guard as in
    /// [`proto_ancestor`]) — resolves the chain as INHERITED.
    proto: String,
    /// Non-Slavic source (`frm aloès`) — resolves the chain as a BORROWING.
    etymon: String,
    /// Another old stage this page inherits from (zle-ort ← orv); the
    /// resolver keeps walking.
    parent: Option<(String, String)>,
}

/// Harvest one old-stage page's ancestry: the classic template parsers plus
/// the `ety` tree format. A Slavic/old-stage source is never an etymon here
/// (a `der` from another Slavic stage is not a foreign borrowing source).
fn old_stage_info(value: &Value) -> OldStageInfo {
    let mut info = OldStageInfo {
        proto: proto_ancestor(value).unwrap_or_default(),
        etymon: borrowed_etymon(value).unwrap_or_default(),
        parent: None,
    };
    // borrowed_etymon treats old-stage codes as foreign (they are not in
    // SLAVIC_LANGS); on an old-stage page that would misclass an intra-Slavic
    // link as a borrowing source — drop it.
    let etymon_src = info.etymon.split_whitespace().next().unwrap_or("");
    if OLD_STAGE_LANGS.contains(&etymon_src) || is_slavic_src(etymon_src) {
        info.etymon.clear();
    }
    let Some(templates) = value.get("etymology_templates").and_then(Value::as_array) else {
        return info;
    };
    for t in templates {
        // Classic inheritance from another old stage → parent.
        let name = t.get("name").and_then(Value::as_str).unwrap_or("");
        if matches!(name, "inh" | "inh+" | "inherited") && info.parent.is_none() {
            if let Some(args) = t.get("args") {
                let src = args.get("2").and_then(Value::as_str).unwrap_or("").trim();
                if OLD_STAGE_LANGS.contains(&src) {
                    let word = args.get("3").and_then(Value::as_str).unwrap_or("").trim();
                    let word = word.split('<').next().unwrap_or(word).trim();
                    if !word.is_empty() && word != "-" {
                        info.parent = Some((src.to_string(), word.to_string()));
                    }
                }
            }
        }
        let Some((op, src, word)) = ety_parts(t) else {
            continue;
        };
        match op {
            ":inh" if src == "sla-pro" && info.proto.is_empty() => {
                // Same shape guard as proto_ancestor: reject placeholders and
                // bound morphemes.
                let form = word.strip_prefix('*').unwrap_or(word);
                if !form.is_empty()
                    && !form.starts_with('-')
                    && !form.ends_with('-')
                    && form.chars().any(|c| c.is_alphabetic())
                {
                    info.proto = format!("*{form}");
                }
            }
            ":inh" if OLD_STAGE_LANGS.contains(&src) && info.parent.is_none() => {
                info.parent = Some((src.to_string(), word.to_string()));
            }
            ":bor" | ":lbor" | ":ubor" | ":obor" | ":slbor" | ":der"
                if info.etymon.is_empty()
                    && !is_slavic_src(src)
                    && !OLD_STAGE_LANGS.contains(&src) =>
            {
                info.etymon = format!("{src} {word}");
            }
            _ => {}
        }
    }
    info
}

/// Walk a pending lemma's parent chain through the old-stage map (bounded —
/// zle-ort pages inherit from orv, which inherits from sla-pro). Returns
/// `(proto, etymon, class)` where class ∈ inherited/borrowing/unresolved.
fn resolve_chain(
    map: &HashMap<(String, String), OldStageInfo>,
    parent: &(String, String),
) -> (String, String, &'static str) {
    let mut cur = Some((parent.0.clone(), chain_key(&parent.1)));
    for _ in 0..4 {
        let Some(key) = cur.take() else { break };
        let Some(info) = map.get(&key) else { break };
        if !info.proto.is_empty() {
            return (info.proto.clone(), String::new(), "inherited");
        }
        if !info.etymon.is_empty() {
            return (String::new(), info.etymon.clone(), "borrowing");
        }
        cur = info.parent.as_ref().map(|(l, w)| (l.clone(), chain_key(w)));
    }
    (String::new(), String::new(), "unresolved")
}

/// How the lemma gate classifies one page (issue #86): kept with direct
/// evidence, pending on an old-stage chain, or dropped.
enum LemmaGate {
    Keep(LemmaEntry),
    /// Same record shape as a keep but with empty proto/etymon; carries the
    /// `(old_lang, old_word)` parent to resolve after the stream.
    Pending(LemmaEntry, (String, String)),
    Drop,
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
    // Old-stage chain state (issue #86): pages harvested while streaming and
    // the pending modern lemmas resolved afterwards, appended AFTER every
    // directly-kept entry so the previous cache is a byte-identical prefix.
    let mut old_stage: HashMap<(String, String), OldStageInfo> = HashMap::new();
    let mut pending: Vec<(LemmaEntry, (String, String))> = Vec::new();
    let mut line_count: u64 = 0;
    for line in reader.lines() {
        let line = line?;
        line_count += 1;
        // Inherited lemmas mention `sla-pro`; borrowings carry a bor/der/lbor
        // template; old-stage chains (issue #86) mention an old-stage code
        // either quoted (classic template args, page lang_code) or with a
        // colon (`ety` tree targets like "zlw-opl:aloes"). `orv`/`cu` are too
        // short to match bare (torva, "cut"), so only their quoted/colon
        // forms are markers. Cheap prefilter before the full JSON parse.
        if !(line.contains("sla-pro")
            || line.contains("\"bor")
            || line.contains("\"lbor")
            || line.contains("\"der+")
            || line.contains("zlw-opl")
            || line.contains("zlw-ocs")
            || line.contains("zlw-osk")
            || line.contains("zle-ort")
            || line.contains("zle-muk")
            || line.contains("zle-mbe")
            || line.contains("zle-mru")
            || line.contains("\"orv\"")
            || line.contains("orv:")
            || line.contains("\"cu\"")
            || line.contains("cu:"))
        {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Old-stage page: harvest its own ancestry (independent of — and in
        // cu's case in addition to — the modern-lemma gate below).
        if let Some(lang) = value.get("lang_code").and_then(Value::as_str) {
            if OLD_STAGE_LANGS.contains(&lang) {
                if let Some(word) = value.get("word").and_then(Value::as_str) {
                    let word = word.trim();
                    if !word.is_empty() {
                        let harvested = old_stage_info(&value);
                        let slot = old_stage
                            .entry((lang.to_string(), chain_key(word)))
                            .or_default();
                        // A page can appear as several POS/etymology sections;
                        // first non-empty answer per field wins.
                        if slot.proto.is_empty() {
                            slot.proto = harvested.proto;
                        }
                        if slot.etymon.is_empty() {
                            slot.etymon = harvested.etymon;
                        }
                        if slot.parent.is_none() {
                            slot.parent = harvested.parent;
                        }
                    }
                }
            }
        }
        match classify_lemma(&value) {
            LemmaGate::Keep(entry) => {
                entries.push(entry);
                if entries.len().is_multiple_of(5000) {
                    eprintln!(
                        "  collected {} Slavic lemmas after {} lines",
                        entries.len(),
                        line_count
                    );
                }
            }
            LemmaGate::Pending(entry, parent) => pending.push((entry, parent)),
            LemmaGate::Drop => {}
        }
    }

    // Post-stream chain resolution (issue #86): every pending lemma is kept —
    // resolved to a proto (inherited) or a foreign etymon (borrowing) through
    // its old-stage page(s), or appended with both fields empty (attested
    // chain lemma; the borrowing skeleton layer groups those). Deterministic:
    // stream order, appended after all direct keeps.
    let direct_keeps = entries.len();
    let mut by_class: BTreeMap<(&'static str, String), u64> = BTreeMap::new();
    let mut samples: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    for (mut entry, parent) in pending {
        let (proto, etymon, class) = resolve_chain(&old_stage, &parent);
        entry.proto = proto;
        entry.etymon = etymon;
        *by_class
            .entry((class, format!("{}←{}", entry.lang, parent.0)))
            .or_default() += 1;
        let bucket = samples.entry(class).or_default();
        if bucket.len() < 5 {
            bucket.push(format!(
                "{} {} ← {} {} → {}",
                entry.lang,
                entry.word,
                parent.0,
                parent.1,
                if !entry.proto.is_empty() {
                    &entry.proto
                } else if !entry.etymon.is_empty() {
                    &entry.etymon
                } else {
                    "(unresolved)"
                }
            ));
        }
        entries.push(entry);
    }
    let mut per_code: BTreeMap<&String, u64> = BTreeMap::new();
    for (lang, _) in old_stage.keys() {
        *per_code.entry(lang).or_default() += 1;
    }
    println!(
        "old-stage chains (issue #86): harvested {} old-stage pages ({}); {} pending lemmas resolved, {} direct keeps unchanged",
        old_stage.len(),
        per_code
            .iter()
            .map(|(l, n)| format!("{l} {n}"))
            .collect::<Vec<_>>()
            .join(", "),
        entries.len() - direct_keeps,
        direct_keeps,
    );
    for ((class, pair), n) in &by_class {
        println!("  chain {class}: {pair} × {n}");
    }
    for (class, rows) in &samples {
        for s in rows {
            println!("  sample {class}: {s}");
        }
    }

    let corpus = LemmaCorpus {
        schema: LEMMA_CACHE_SCHEMA,
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

/// The lemma gate. The Keep branch is unchanged from the pre-#86 extractor
/// (byte-identical records); the Pending branch fires only where the old gate
/// returned None — an evidence-less page whose etymology names a
/// same-language old stage.
fn classify_lemma(value: &Value) -> LemmaGate {
    let Some(lang) = value.get("lang_code").and_then(Value::as_str) else {
        return LemmaGate::Drop;
    };
    if !SLAVIC_LANGS.contains(&lang) {
        return LemmaGate::Drop;
    }
    let Some(word) = value.get("word").and_then(Value::as_str) else {
        return LemmaGate::Drop;
    };
    let word = word.trim().to_string();
    // Lemmas only: single token, not a reconstruction, not a phrase.
    if word.is_empty() || word.contains(' ') || word.starts_with('*') || word.starts_with('-') {
        return LemmaGate::Drop;
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
    // No direct evidence: a same-language old-stage chain (issue #86) makes
    // the lemma PENDING (resolved after the stream); otherwise drop, as ever.
    let parent = if proto.is_empty() && etymon.is_empty() {
        match old_stage_parent(value, lang) {
            Some(p) => Some(p),
            None => return LemmaGate::Drop,
        }
    } else {
        None
    };
    let Some(gloss) = lemma_gloss(value) else {
        return LemmaGate::Drop;
    };
    let etymology = lemma_etymology(value);
    let (categories, topics, tags) = wiki_metadata(value);
    let entry = LemmaEntry {
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
    };
    match parent {
        Some(p) => LemmaGate::Pending(entry, p),
        None => LemmaGate::Keep(entry),
    }
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

/// Decode URL-escaped UTF-8 that occasionally leaks from Wiktionary template
/// arguments. Invalid/incomplete encoding is preserved rather than guessed.
fn decode_percent_utf8(value: &str) -> String {
    fn hex(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut changed = false;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                changed = true;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    if !changed {
        return value.to_string();
    }
    String::from_utf8(out).unwrap_or_else(|_| value.to_string())
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
        let form = decode_percent_utf8(form);
        if form.is_empty() || form == "*" {
            continue;
        }
        let form = form.strip_prefix('*').unwrap_or(&form);
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
    // Auditable drop-reason tally over the Slavic pages seen (issue #35). The
    // KEPT set (and thus `lemmas`) is unchanged, so the cache stays byte-identical.
    let mut stats = RawCoverageStats {
        source: dump.display().to_string(),
        ..Default::default()
    };
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
        match classify_raw_slavic(&value) {
            // Prefilter over-match (nested Slavic mention): not a Slavic page.
            RawOutcome::NotSlavic => {}
            RawOutcome::Kept(lemma) => {
                stats.slavic_pages_seen += 1;
                stats.kept += 1;
                *stats.kept_by_lang.entry(lemma.lang.clone()).or_default() += 1;
                lemmas.push(lemma);
                if lemmas.len().is_multiple_of(20000) {
                    eprintln!(
                        "  collected {} raw Slavic lemmas after {} lines",
                        lemmas.len(),
                        line_count
                    );
                }
            }
            RawOutcome::DropRedirect => {
                stats.slavic_pages_seen += 1;
                stats.dropped_redirect_no_senses += 1;
            }
            RawOutcome::DropMultiword => {
                stats.slavic_pages_seen += 1;
                stats.dropped_multiword += 1;
            }
            RawOutcome::DropNonContentPos => {
                stats.slavic_pages_seen += 1;
                stats.dropped_non_content_pos += 1;
            }
            RawOutcome::DropNoRealGloss => {
                stats.slavic_pages_seen += 1;
                stats.dropped_no_real_gloss += 1;
            }
        }
    }
    stats.lines_scanned = line_count;

    let corpus = RawSlavicCorpus {
        schema: RAW_CACHE_SCHEMA,
        lemmas,
    };
    corpus.save(out)?;
    let cov_path = out.with_file_name(RAW_COVERAGE_FILE);
    stats.save(&cov_path)?;
    println!(
        "wrote {} ({} raw Slavic lemmas from {} lines)",
        out.display(),
        corpus.lemmas.len(),
        line_count
    );
    println!(
        "wrote {} (coverage: {} slavic pages seen; {} kept; dropped {} redirect / {} multiword / {} non-content-pos / {} no-gloss)",
        cov_path.display(),
        stats.slavic_pages_seen,
        stats.kept,
        stats.dropped_redirect_no_senses,
        stats.dropped_multiword,
        stats.dropped_non_content_pos,
        stats.dropped_no_real_gloss,
    );
    Ok(())
}

/// How the RAW gate classifies one Wiktextract page, for coverage reporting
/// (issue #35). Every classification but [`RawOutcome::NotSlavic`] counts toward
/// the coverage denominator; [`RawOutcome::Kept`] carries the produced lemma.
enum RawOutcome {
    /// A single-token Slavic content lemma with ≥1 real gloss — kept.
    Kept(RawSlavicLemma),
    /// Top-level language is not a RAW Slavic code (the substring prefilter can
    /// match a nested mention) — not a Slavic page, excluded from the denominator.
    NotSlavic,
    /// Dropped: redirect-like page with no `senses`.
    DropRedirect,
    /// Dropped: multi-token or empty headword.
    DropMultiword,
    /// Dropped: not a content part of speech.
    DropNonContentPos,
    /// Dropped: only form-of senses; no real gloss.
    DropNoRealGloss,
}

/// Apply the RAW gate to one Wiktextract page, classifying it for the coverage
/// tally. The KEPT branch is byte-for-byte the same lemma the old gate produced
/// (the accept condition is an unordered conjunction, so the cache is unchanged);
/// the drop branches only add auditable reason buckets.
fn classify_raw_slavic(value: &Value) -> RawOutcome {
    // Top-level language must be one of the Slavic codes.
    let lang = match value.get("lang_code").and_then(Value::as_str) {
        Some(l) if RAW_SLAVIC_LANGS.contains(&l) => l,
        _ => return RawOutcome::NotSlavic,
    };
    let word = value
        .get("word")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    // Real lemma page: non-empty `senses` (skips redirect rows / word-less pages).
    let senses = match value.get("senses").and_then(Value::as_array) {
        Some(s) if !s.is_empty() => s,
        _ => return RawOutcome::DropRedirect,
    };
    // Single token: non-empty and no space.
    if word.is_empty() || word.contains(' ') {
        return RawOutcome::DropMultiword;
    }
    // Quality gate: content POS only (drop proper nouns/particles/etc.).
    let raw_pos = value.get("pos").and_then(Value::as_str).unwrap_or("");
    if !matches!(raw_pos, "noun" | "verb" | "adj" | "adv") {
        return RawOutcome::DropNonContentPos;
    }
    // Quality gate: at least one real (non-form-of) gloss.
    let glosses = real_glosses(senses, 4);
    if glosses.is_empty() {
        return RawOutcome::DropNoRealGloss;
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
    RawOutcome::Kept(RawSlavicLemma {
        word,
        lang: lang.to_string(),
        pos: raw_pos.to_string(),
        glosses,
        etymology_text,
        proto,
        etymon,
    })
}

/// Apply the RAW gate to one page, returning the kept [`RawSlavicLemma`] when it
/// is a single-token Slavic content lemma with at least one real gloss.
#[cfg(test)]
fn raw_slavic_from_value(value: &Value) -> Option<RawSlavicLemma> {
    match classify_raw_slavic(value) {
        RawOutcome::Kept(lemma) => Some(lemma),
        _ => None,
    }
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
            if entries.len().is_multiple_of(2000) {
                eprintln!(
                    "  extracted {} Proto-Slavic entries after {} lines",
                    entries.len(),
                    line_count
                );
            }
        }
    }

    let cache = ProtoCache {
        schema: PROTO_CACHE_SCHEMA,
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
    // The declension category ("Proto-Slavic masculine n-stem nouns") usually
    // sits on the SENSE level in wiktextract, not the page level — *kamy* and
    // *bratrъ* have no top-level `categories` at all. Scan both.
    let top = value.get("categories").and_then(Value::as_array);
    let sense_cats = value
        .get("senses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.get("categories").and_then(Value::as_array));
    for cats in top.into_iter().chain(sense_cats) {
        for c in cats.iter().filter_map(Value::as_str) {
            let lc = c.to_lowercase();
            for key in [
                "o-stem",
                "a-stem",
                "ā-stem",
                "i-stem",
                "u-stem",
                // Wiktionary files the feminine ū-stems (*kry, *svekry, *buky)
                // as "v-stem" after their oblique extension — there is no
                // "ū-stem" category in the dump.
                "v-stem",
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
    /// Deep (pre-Slavic) ancestors named by a modern lemma's English etymology:
    /// "lang\u{1}phonemic-latin" -> folded Proto-Balto-Slavic / PIE tokens
    /// (issue #76). Built alongside `etym`; consumed by the linker's
    /// deep-corroboration rescue.
    deep_etym: HashMap<String, Vec<String>>,
}

impl ProtoIndex {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes =
            read_maybe_gz(path).with_context(|| format!("open proto cache {}", path.display()))?;
        let mut cache: ProtoCache = serde_json::from_slice(&bytes).context("parse proto cache")?;
        check_cache_schema(
            "proto",
            path,
            cache.schema,
            PROTO_CACHE_SCHEMA,
            "make extract-proto",
        )?;
        anyhow::ensure!(
            cache.entry_count == cache.entries.len(),
            "corrupt proto cache {}: entry_count {} but {} entries",
            path.display(),
            cache.entry_count,
            cache.entries.len()
        );
        // Same load-time homoglyph hygiene as LemmaCorpus::load (issue #66),
        // applied to reconstruction names so `by_word` and the lemma corpus's
        // folded `proto` fields keep matching. Descendants stay verbatim —
        // those are attested modern words, legitimately Cyrillic.
        for e in &mut cache.entries {
            e.word = crate::normalize::fold_proto_homoglyphs(&e.word);
        }
        let mut idx = Self::build(cache.entries);
        // Attach Wiktionary's explicit (lang, lemma) -> ancestor etymology if the
        // lemma corpus is available next to the proto cache. Absent → skip; a
        // corpus that exists but fails to load is a hard error (a silently
        // missing etymology map would degrade the linker with no visible cause).
        if let Some(corpus) =
            load_optional(Path::new(crate::DEFAULT_LEMMA_CACHE), LemmaCorpus::load)?
        {
            idx.attach_etymology(&corpus);
        }
        Ok(idx)
    }

    fn attach_etymology(&mut self, corpus: &LemmaCorpus) {
        for e in &corpus.entries {
            let latin = crate::normalize::to_phonemic_latin(&e.lang, &e.word);
            if latin.is_empty() {
                continue;
            }
            let key = format!("{}\u{1}{latin}", e.lang);
            // Deep (pre-Slavic) ancestors the lemma's own etymology names,
            // scraped with the same needle logic as the proto cache's pbs/pie
            // fields so both sides of the corroboration match fold-equal.
            let mut deep: Vec<String> = Vec::new();
            for text in &e.etymology {
                for needle in ["Proto-Balto-Slavic", "Proto-Indo-European"] {
                    let tok = crate::normalize::fold_deep_token(&after_needle(text, needle));
                    if !tok.is_empty() && !deep.contains(&tok) {
                        deep.push(tok);
                    }
                }
            }
            if !deep.is_empty() {
                self.deep_etym.entry(key.clone()).or_insert(deep);
            }
            if e.proto.is_empty() || !self.by_word.contains_key(e.proto.trim_start_matches('*')) {
                continue; // only ancestors we actually have a reconstruction for
            }
            self.etym.entry(key).or_insert_with(|| e.proto.clone());
        }
    }

    /// The explicitly-attested Proto-Slavic ancestor of a modern lemma, if any.
    pub fn etym_ancestor(&self, lang: &str, latin: &str) -> Option<&str> {
        self.etym
            .get(&format!("{lang}\u{1}{latin}"))
            .map(|s| s.as_str())
    }

    /// The folded deep (Proto-Balto-Slavic / PIE) ancestor tokens a modern
    /// lemma's English etymology names, if any (issue #76).
    pub fn deep_ancestors(&self, lang: &str, latin: &str) -> Option<&[String]> {
        self.deep_etym
            .get(&format!("{lang}\u{1}{latin}"))
            .map(|v| v.as_slice())
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
            deep_etym: HashMap::new(),
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

    /// Pins the cache-schema-stamp contract: a pre-stamp cache (no `schema`
    /// field) deserializes as 0 and — now that the lemma schema moved past 0
    /// (issue #86 bumped it to 1) — is REFUSED with the exact `make` target to
    /// re-run, exactly like any other stale stamp. Only the current stamp
    /// loads.
    #[test]
    fn cache_schema_guard_accepts_current_and_rejects_stale() {
        let legacy: LemmaCorpus =
            serde_json::from_str(r#"{"source":"","entry_count":0,"entries":[]}"#).unwrap();
        assert_eq!(legacy.schema, 0);
        // The unwrap_err below only holds while LEMMA_CACHE_SCHEMA > 0 — it
        // IS the pin that the issue-#86 bump is in effect.
        let err = check_cache_schema(
            "lemma",
            Path::new("data/x.json"),
            legacy.schema,
            LEMMA_CACHE_SCHEMA,
            "make extract-lemmas",
        )
        .unwrap_err();
        assert!(err.to_string().contains("make extract-lemmas"), "{err}");
        check_cache_schema(
            "lemma",
            Path::new("data/x.json"),
            LEMMA_CACHE_SCHEMA,
            LEMMA_CACHE_SCHEMA,
            "make extract-lemmas",
        )
        .unwrap();
        let err = check_cache_schema(
            "lemma",
            Path::new("data/x.json"),
            LEMMA_CACHE_SCHEMA + 1,
            LEMMA_CACHE_SCHEMA,
            "make extract-lemmas",
        )
        .unwrap_err();
        assert!(err.to_string().contains("make extract-lemmas"), "{err}");
    }

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
            schema: LEMMA_CACHE_SCHEMA,
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
    fn proto_ancestor_rejects_bound_morphemes_and_decodes_transport_escapes() {
        use serde_json::json;
        let v = json!({"etymology_templates":[{"name":"inh","args":{"2":"sla-pro","3":"*orz-"}}]});
        assert_eq!(proto_ancestor(&v), None);
        let v2 = json!({"etymology_templates":[{"name":"inh","args":{"2":"sla-pro","3":"*voda"}}]});
        assert_eq!(proto_ancestor(&v2).as_deref(), Some("*voda"));
        let escaped =
            json!({"etymology_templates":[{"name":"inh","args":{"2":"sla-pro","3":"*%C4%BEuby"}}]});
        assert_eq!(proto_ancestor(&escaped).as_deref(), Some("*ľuby"));
        assert_eq!(decode_percent_utf8("*%ZZuby"), "*%ZZuby");
        assert_eq!(decode_percent_utf8("*%FFuby"), "*%FFuby");
    }

    /// Issue #86: an evidence-less modern lemma whose etymology names its own
    /// language's OLD STAGE becomes a PENDING chain lemma — in both the
    /// classic template format and the newer `ety` tree format — while pages
    /// with direct evidence stay Keep (byte-identical to the old gate) and
    /// pages with neither stay dropped.
    #[test]
    fn old_stage_chains_classify_pending_keep_and_drop() {
        use serde_json::json;
        // pl aloes, real dump shape: only an `ety` :inh zlw-opl link.
        let ety_chain = json!({
            "word": "aloes", "lang_code": "pl", "pos": "noun",
            "etymology_templates": [
                {"name": "ety", "args": {"1": "pl", "2": ":inh", "3": "zlw-opl:aloes", "text": "+", "tree": "1"}}
            ],
            "senses": [{"glosses": ["aloe"]}]
        });
        match classify_lemma(&ety_chain) {
            LemmaGate::Pending(e, parent) => {
                assert_eq!((e.lang.as_str(), e.word.as_str()), ("pl", "aloes"));
                assert!(e.proto.is_empty() && e.etymon.is_empty());
                assert_eq!(parent, ("zlw-opl".to_string(), "aloes".to_string()));
            }
            _ => panic!("ety chain lemma should be pending"),
        }
        // Classic inh|ru|orv chain.
        let classic_chain = json!({
            "word": "дом", "lang_code": "ru", "pos": "noun",
            "etymology_templates": [
                {"name": "inh", "args": {"1": "ru", "2": "orv", "3": "домъ"}}
            ],
            "senses": [{"glosses": ["house"]}]
        });
        assert!(matches!(
            classify_lemma(&classic_chain),
            LemmaGate::Pending(_, _)
        ));
        // Direct sla-pro evidence wins: stays Keep even with an old-stage
        // template on the same page (the old gate's records are untouched).
        let direct = json!({
            "word": "дом", "lang_code": "ru", "pos": "noun",
            "etymology_templates": [
                {"name": "inh", "args": {"1": "ru", "2": "orv", "3": "домъ"}},
                {"name": "inh", "args": {"1": "ru", "2": "sla-pro", "3": "*domъ"}}
            ],
            "senses": [{"glosses": ["house"]}]
        });
        match classify_lemma(&direct) {
            LemmaGate::Keep(e) => assert_eq!(e.proto, "*domъ"),
            _ => panic!("direct evidence must stay Keep"),
        }
        // A foreign old stage is NOT a chain for this language (sl has no
        // old-stage codes) — still dropped.
        let foreign = json!({
            "word": "x", "lang_code": "sl", "pos": "noun",
            "etymology_templates": [
                {"name": "inh", "args": {"1": "sl", "2": "orv", "3": "y"}}
            ],
            "senses": [{"glosses": ["z"]}]
        });
        assert!(matches!(classify_lemma(&foreign), LemmaGate::Drop));
    }

    /// Issue #86: the old-stage harvest reads classic templates AND the `ety`
    /// tree format, never takes a Slavic/old-stage source as an etymon, and
    /// records old→old inheritance as a parent hop.
    #[test]
    fn old_stage_info_harvests_proto_etymon_and_parent() {
        use serde_json::json;
        // Real zlw-opl aloes shape: ety :bor frm:aloès with a <ref:> tail.
        let opl = json!({
            "word": "aloes", "lang_code": "zlw-opl", "pos": "noun",
            "etymology_templates": [
                {"name": "ety", "args": {"1": "zlw-opl", "2": ":bor", "3": "frm:aloès<ref:…>"}}
            ]
        });
        let info = old_stage_info(&opl);
        assert_eq!(info.etymon, "frm aloès");
        assert!(info.proto.is_empty() && info.parent.is_none());
        // Classic inh sla-pro on an orv page → proto.
        let orv = json!({
            "word": "домъ", "lang_code": "orv",
            "etymology_templates": [
                {"name": "inh", "args": {"1": "orv", "2": "sla-pro", "3": "*domъ"}}
            ]
        });
        assert_eq!(old_stage_info(&orv).proto, "*domъ");
        // ety :inh sla-pro also yields the proto; bound morphemes rejected.
        let ety_proto = json!({
            "word": "вода", "lang_code": "orv",
            "etymology_templates": [
                {"name": "ety", "args": {"1": "orv", "2": ":inh", "3": "sla-pro:*voda"}}
            ]
        });
        assert_eq!(old_stage_info(&ety_proto).proto, "*voda");
        // Old→old inheritance (zle-ort ← orv) is a parent hop, NOT an etymon.
        let ort = json!({
            "word": "мѣсто", "lang_code": "zle-ort",
            "etymology_templates": [
                {"name": "inh", "args": {"1": "zle-ort", "2": "orv", "3": "мѣсто"}}
            ]
        });
        let info = old_stage_info(&ort);
        assert_eq!(info.parent, Some(("orv".to_string(), "мѣсто".to_string())));
        assert!(info.etymon.is_empty());
        // A der from another Slavic language is not a foreign etymon.
        let slavic_der = json!({
            "word": "x", "lang_code": "zlw-opl",
            "etymology_templates": [
                {"name": "der", "args": {"1": "zlw-opl", "2": "zlw-ocs", "3": "y"}}
            ]
        });
        assert!(old_stage_info(&slavic_der).etymon.is_empty());
    }

    /// Issue #86: chain resolution walks bounded old→old hops, strips the
    /// combining accents template citations carry, and classifies inherited /
    /// borrowing / unresolved.
    #[test]
    fn resolve_chain_walks_hops_and_strips_accents() {
        let mut map: HashMap<(String, String), OldStageInfo> = HashMap::new();
        map.insert(
            ("zle-ort".into(), chain_key("мѣсто")),
            OldStageInfo {
                proto: String::new(),
                etymon: String::new(),
                parent: Some(("orv".into(), "мѣ́сто".into())), // accented citation
            },
        );
        map.insert(
            ("orv".into(), chain_key("мѣсто")),
            OldStageInfo {
                proto: "*město".into(),
                etymon: String::new(),
                parent: None,
            },
        );
        map.insert(
            ("zlw-opl".into(), chain_key("aloes")),
            OldStageInfo {
                proto: String::new(),
                etymon: "frm aloès".into(),
                parent: None,
            },
        );
        // Two hops, accent-insensitive: uk word ← zle-ort мѣсто ← orv → proto.
        let (proto, etymon, class) = resolve_chain(&map, &("zle-ort".into(), "мѣ́сто".into()));
        assert_eq!(
            (proto.as_str(), etymon.as_str(), class),
            ("*město", "", "inherited")
        );
        // One hop to a foreign etymon → borrowing.
        let (proto, etymon, class) = resolve_chain(&map, &("zlw-opl".into(), "aloes".into()));
        assert_eq!(
            (proto.as_str(), etymon.as_str(), class),
            ("", "frm aloès", "borrowing")
        );
        // Missing old-stage page → unresolved, both fields empty.
        let (proto, etymon, class) = resolve_chain(&map, &("orv".into(), "нѣтъ".into()));
        assert_eq!(
            (proto.as_str(), etymon.as_str(), class),
            ("", "", "unresolved")
        );
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

    #[test]
    fn raw_slavic_classify_reasons() {
        use serde_json::json;
        // Kept.
        let kept = json!({"word": "вода", "lang_code": "ru", "pos": "noun",
                          "senses": [{"glosses": ["water"]}]});
        assert!(matches!(classify_raw_slavic(&kept), RawOutcome::Kept(_)));
        // Non-Slavic language -> excluded from the coverage denominator.
        let en = json!({"word": "water", "lang_code": "en", "pos": "noun",
                        "senses": [{"glosses": ["water"]}]});
        assert!(matches!(classify_raw_slavic(&en), RawOutcome::NotSlavic));
        // Redirect: no senses.
        let redirect = json!({"word": "foo", "lang_code": "ru", "pos": "noun"});
        assert!(matches!(
            classify_raw_slavic(&redirect),
            RawOutcome::DropRedirect
        ));
        // Multiword.
        let phrase = json!({"word": "по мере", "lang_code": "ru", "pos": "adv",
                            "senses": [{"glosses": ["gradually"]}]});
        assert!(matches!(
            classify_raw_slavic(&phrase),
            RawOutcome::DropMultiword
        ));
        // Non-content POS (proper noun).
        let name = json!({"word": "Москва", "lang_code": "ru", "pos": "name",
                          "senses": [{"glosses": ["Moscow"]}]});
        assert!(matches!(
            classify_raw_slavic(&name),
            RawOutcome::DropNonContentPos
        ));
        // No real gloss (only a form-of sense).
        let only_form = json!({"word": "води", "lang_code": "ru", "pos": "noun",
                               "senses": [{"glosses": ["genitive singular of вода"],
                                           "tags": ["form-of"],
                                           "form_of": [{"word": "вода"}]}]});
        assert!(matches!(
            classify_raw_slavic(&only_form),
            RawOutcome::DropNoRealGloss
        ));
    }

    #[test]
    fn raw_coverage_stats_reconcile() {
        let mut s = RawCoverageStats {
            slavic_pages_seen: 10,
            kept: 6,
            dropped_redirect_no_senses: 1,
            dropped_multiword: 1,
            dropped_non_content_pos: 1,
            dropped_no_real_gloss: 1,
            ..Default::default()
        };
        assert_eq!(s.dropped_total(), 4);
        assert_eq!(s.kept + s.dropped_total(), s.slavic_pages_seen);
        s.kept_by_lang.insert("ru".into(), 6);
        assert_eq!(s.kept_by_lang.values().sum::<u64>(), s.kept);
    }

    #[test]
    fn stem_class_reads_sense_level_categories() {
        use serde_json::json;
        // Issue #76: the declension category almost always sits on the SENSE
        // level in wiktextract (*kamy has no page-level `categories` at all),
        // so the extractor must scan both levels.
        let sense_only = json!({
            "word": "kamy",
            "senses": [{
                "glosses": ["stone"],
                "categories": ["Proto-Slavic lemmas", "Proto-Slavic masculine n-stem nouns"]
            }]
        });
        assert_eq!(
            stem_class(&sense_only).as_deref(),
            Some("Proto-Slavic masculine n-stem nouns")
        );
        // Page-level categories keep working.
        let page_level = json!({
            "word": "kry",
            "categories": ["Proto-Slavic hard v-stem nouns"]
        });
        assert_eq!(
            stem_class(&page_level).as_deref(),
            Some("Proto-Slavic hard v-stem nouns")
        );
        // No declension category anywhere → None.
        let none = json!({
            "word": "x",
            "senses": [{ "categories": ["Proto-Slavic lemmas"] }]
        });
        assert_eq!(stem_class(&none), None);
    }
}
