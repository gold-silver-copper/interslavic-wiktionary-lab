//! The lexical verification layer (issue #11): one canonical `FormRecord`
//! pipeline feeding BOTH the website's inflection tables and the agent-facing
//! static API, so the two can never drift apart.
//!
//! - Cell getters (`noun_cell`, `adj_cell`, `verb_cells`) are the single
//!   source: the HTML tables in `site.rs` render from them, and
//!   `paradigm_records` enumerates the same calls into [`FormRecord`]s. A CI
//!   round-trip test asserts every rendered table cell appears in the records.
//! - The API is **sharded**: `api/forms/<n>.json`, `n = fnv1a32(key) % SHARDS`
//!   over the folded key — a full index would be tens of MB (231k+ official
//!   paradigm cells), useless to an agent context window. Shards are compact
//!   JSON arrays, deterministically ordered (BTreeMap), byte-identical across
//!   runs (no timestamps).
//! - `key` is `orthography::to_standard` of the lowercased form — agents send
//!   `pomocnogo`-style folded text; the same fold is mirrored in the site's
//!   client-side JS and documented in `api/agent-guide.md`.

use crate::model::Pos;
use crate::orthography as ortho;
use interslavic::{
    Animacy as IsvAnimacy, Case as IsvCase, Gender as IsvGender, Number as IsvNumber, ISV,
};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// Shard count for the form index. Changing it is a schema break: bump
/// [`SCHEMA_VERSION`] and regenerate `api/agent-guide.md`.
pub const SHARDS: u32 = 1024;
pub const SCHEMA_VERSION: u32 = 1;
pub const LICENSE: &str =
    "CC BY-SA 4.0 (derives from Wiktionary and interslavic-dictionary.com; see /about.html)";

/// 32-bit FNV-1a over the UTF-8 bytes — mirrored in the site's JavaScript
/// (`Math.imul`-based); both sides route `key → shard` identically.
pub fn fnv1a32(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(16_777_619);
    }
    h
}

/// The folded lookup key: standard orthography, lowercase.
pub fn form_key(form: &str) -> String {
    ortho::to_standard(&form.trim().to_lowercase())
}

pub fn shard_of(key: &str) -> u32 {
    fnv1a32(key) % SHARDS
}

/// Run an inflector call, recovering its panics as the blank cell "—".
pub fn catch<F: FnOnce() -> String + std::panic::UnwindSafe>(f: F) -> String {
    std::panic::catch_unwind(f).unwrap_or_else(|_| "—".to_string())
}

// ---------------------------------------------------------------------------
// Cell getters — the single source for tables AND records.
// ---------------------------------------------------------------------------

pub const CASES: [(&str, IsvCase); 6] = [
    ("nom", IsvCase::Nom),
    ("akuz", IsvCase::Acc),
    ("gen", IsvCase::Gen),
    ("dat", IsvCase::Dat),
    ("lok", IsvCase::Loc),
    ("instr", IsvCase::Ins),
];
pub const NUMBERS: [(&str, IsvNumber); 2] =
    [("jd", IsvNumber::Singular), ("mn", IsvNumber::Plural)];
pub const ADJ_COLS: [(&str, IsvGender, IsvAnimacy); 4] = [
    ("m.živ.", IsvGender::Masculine, IsvAnimacy::Animate),
    ("m.než.", IsvGender::Masculine, IsvAnimacy::Inanimate),
    ("ž.", IsvGender::Feminine, IsvAnimacy::Inanimate),
    ("sr.", IsvGender::Neuter, IsvAnimacy::Inanimate),
];

pub fn noun_cell(word: &str, case: IsvCase, number: IsvNumber) -> String {
    catch(|| ISV::noun(word, case, number))
}

pub fn adj_cell(
    word: &str,
    case: IsvCase,
    number: IsvNumber,
    gender: IsvGender,
    animacy: IsvAnimacy,
) -> String {
    catch(|| ISV::adj(word, case, number, gender, animacy))
}

/// All of a verb's cells, reflexive particle already applied — the shared
/// source for `verb_table` and the records. `None` when the inflector cannot
/// handle the stem.
pub struct VerbCells {
    pub present: Vec<String>,
    pub imperfect: Vec<String>,
    pub future: Vec<String>,
    pub perfect: Vec<String>,
    pub pluperfect: Vec<String>,
    pub conditional: Vec<String>,
    pub imperative: Vec<String>,
    /// (feature label, form): infinitive, participles, gerund.
    pub nonfinite: Vec<(&'static str, String)>,
}

pub const VERB_FINITE_FEATS: [&str; 6] = ["1jd", "2jd", "3jd", "1mn", "2mn", "3mn"];
pub const VERB_COMPOUND_FEATS: [&str; 8] = [
    "1jd", "2jd", "3jd.m", "3jd.ž", "3jd.sr", "1mn", "2mn", "3mn",
];
pub const VERB_IMPERATIVE_FEATS: [&str; 3] = ["2jd", "1mn", "2mn"];

/// Append the reflexive particle to a (possibly multi-variant) cell.
pub fn append_reflexive(form: &str, reflexive: bool) -> String {
    if !reflexive || form == "—" || form.trim().is_empty() {
        form.to_string()
    } else if form.contains(" / ") {
        form.split(" / ")
            .map(|part| format!("{} sę", part.trim()))
            .collect::<Vec<_>>()
            .join(" / ")
    } else {
        format!("{form} sę")
    }
}

pub fn verb_cells(word: &str, reflexive: bool) -> Option<VerbCells> {
    let p = std::panic::catch_unwind(|| ISV::verb_forms(word)).ok()?;
    let fix = |v: Vec<String>| -> Vec<String> {
        v.into_iter()
            .map(|f| append_reflexive(&f, reflexive))
            .collect()
    };
    let prap = p.prap.unwrap_or_else(|| "—".to_string());
    let prpp = p.prpp.unwrap_or_else(|| "—".to_string());
    let pfpp = p.pfpp.unwrap_or_else(|| "—".to_string());
    Some(VerbCells {
        present: fix(p.present),
        imperfect: fix(p.imperfect),
        future: fix(p.future),
        perfect: fix(p.perfect),
        pluperfect: fix(p.pluperfect),
        conditional: fix(p.conditional),
        imperative: fix(p.imperative),
        nonfinite: vec![
            ("inf", append_reflexive(&p.infinitive, reflexive)),
            ("part.akt.tep", append_reflexive(&prap, reflexive)),
            ("part.pas.tep", append_reflexive(&prpp, reflexive)),
            ("part.akt.proš", append_reflexive(&p.pfap, reflexive)),
            ("part.pas.proš", append_reflexive(&pfpp, reflexive)),
            ("gerund", append_reflexive(&p.gerund, reflexive)),
        ],
    })
}

// ---------------------------------------------------------------------------
// FormRecord — the canonical exported analysis.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FormRecord {
    /// Flavored display form (one variant — multi-variant cells are split).
    pub form: String,
    /// Folded lookup key (`form_key`).
    pub key: String,
    pub lemma: String,
    pub entry_id: usize,
    pub pos: &'static str,
    /// Compact analyses, e.g. `["gen.jd.", "akuz.jd. m.živ."]` — one record
    /// per (form, lemma, entry), syncretic cells merged.
    pub analyses: Vec<String>,
    /// "lemma" | "inflection".
    pub source: &'static str,
    /// "official" | "official-only" | "generated".
    pub status: &'static str,
    /// Calibrated P(the lemma matches an official decision); None for official
    /// lemmas (they ARE the official decision).
    pub probability: Option<f64>,
    pub gloss: String,
}

/// Accumulates records, merging analyses of syncretic cells.
#[derive(Default)]
pub struct RecordSink {
    map: BTreeMap<(String, String, usize), FormRecord>,
}

impl RecordSink {
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        cell: &str,
        feats: &str,
        lemma: &str,
        entry_id: usize,
        pos: &'static str,
        source: &'static str,
        status: &'static str,
        probability: Option<f64>,
        gloss: &str,
    ) {
        // A cell may hold byform variants ("den / denj"): each variant is its
        // own record (its own key), sharing the analysis.
        for variant in cell.split('/') {
            let form = variant.trim();
            if form.is_empty() || form == "—" {
                continue;
            }
            let key = form_key(form);
            if key.is_empty() {
                continue;
            }
            let entry = self
                .map
                .entry((key.clone(), form_key(lemma), entry_id))
                .or_insert_with(|| FormRecord {
                    form: form.to_string(),
                    key,
                    lemma: lemma.to_string(),
                    entry_id,
                    pos,
                    analyses: Vec::new(),
                    source,
                    status,
                    probability,
                    gloss: gloss.to_string(),
                });
            // The lemma record outranks an inflection analysis for provenance
            // (nom.sg is also the citation form).
            if source == "lemma" {
                entry.source = "lemma";
            }
            let feats = feats.to_string();
            if !feats.is_empty() && !entry.analyses.contains(&feats) {
                entry.analyses.push(feats);
            }
        }
    }

    pub fn into_records(self) -> Vec<FormRecord> {
        self.map.into_values().collect()
    }
}

/// Collect the full paradigm of one lemma into the sink. `reflexive` verbs
/// (`X sę`) are inflected on the bare stem with the particle re-applied, so
/// their keys are two-token (`myti se`) and `check-text`'s bigram lookup finds
/// them.
#[allow(clippy::too_many_arguments)]
pub fn paradigm_records(
    sink: &mut RecordSink,
    lemma: &str,
    pos: Pos,
    entry_id: usize,
    status: &'static str,
    probability: Option<f64>,
    gloss: &str,
) {
    let reflexive = lemma.ends_with(" sę");
    let bare = lemma.strip_suffix(" sę").unwrap_or(lemma).trim();
    if bare.is_empty() || bare.contains(' ') {
        return;
    }
    match pos {
        Pos::Noun | Pos::ProperNoun => {
            for (nf, num) in NUMBERS {
                for (cf, case) in CASES {
                    sink.add(
                        &noun_cell(bare, case, num),
                        &format!("{cf}.{nf}."),
                        lemma,
                        entry_id,
                        pos.code(),
                        "inflection",
                        status,
                        probability,
                        gloss,
                    );
                }
            }
        }
        Pos::Adjective => {
            for (nf, num) in NUMBERS {
                for (cf, case) in CASES {
                    for (gf, g, a) in ADJ_COLS {
                        sink.add(
                            &adj_cell(bare, case, num, g, a),
                            &format!("{cf}.{nf}. {gf}"),
                            lemma,
                            entry_id,
                            "adj",
                            "inflection",
                            status,
                            probability,
                            gloss,
                        );
                    }
                }
            }
        }
        Pos::Verb => {
            let Some(cells) = verb_cells(bare, reflexive) else {
                return;
            };
            let mut add = |form: &str, feats: String| {
                sink.add(
                    form,
                    &feats,
                    lemma,
                    entry_id,
                    "verb",
                    "inflection",
                    status,
                    probability,
                    gloss,
                );
            };
            for (tense, forms) in [
                ("prez", &cells.present),
                ("impf", &cells.imperfect),
                ("fut", &cells.future),
            ] {
                for (i, f) in forms.iter().enumerate() {
                    if let Some(p) = VERB_FINITE_FEATS.get(i) {
                        add(f, format!("{tense}.{p}."));
                    }
                }
            }
            for (tense, forms) in [
                ("perf", &cells.perfect),
                ("plusk", &cells.pluperfect),
                ("kond", &cells.conditional),
            ] {
                for (i, f) in forms.iter().enumerate() {
                    if let Some(p) = VERB_COMPOUND_FEATS.get(i) {
                        add(f, format!("{tense}.{p}."));
                    }
                }
            }
            for (i, f) in cells.imperative.iter().enumerate() {
                if let Some(p) = VERB_IMPERATIVE_FEATS.get(i) {
                    add(f, format!("imper.{p}."));
                }
            }
            for (feat, f) in &cells.nonfinite {
                add(f, format!("{feat}."));
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// The static API writer.
// ---------------------------------------------------------------------------

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn record_json(r: &FormRecord) -> String {
    let analyses = r
        .analyses
        .iter()
        .map(|a| json_str(a))
        .collect::<Vec<_>>()
        .join(",");
    let prob = r
        .probability
        .map(|p| format!("{:.3}", p))
        .unwrap_or_else(|| "null".into());
    format!(
        "[{},{},{},{},[{}],{},{},{},{}]",
        json_str(&r.form),
        json_str(&r.lemma),
        r.entry_id,
        json_str(r.pos),
        analyses,
        json_str(r.source),
        json_str(r.status),
        prob,
        json_str(&r.gloss),
    )
}

pub struct ApiCounts {
    pub records: usize,
    pub keys: usize,
    pub lemmas: usize,
    pub bytes: usize,
    pub largest_shard: usize,
}

/// Write `api/meta.json`, `api/lemmas.json` and the `api/forms/<n>.json`
/// shards. Deterministic: BTreeMap ordering everywhere, no timestamps.
pub fn write_api(
    out_dir: &Path,
    records: &[FormRecord],
    lemmas: &[FormRecord],
    git: &str,
    agent_guide: &str,
) -> anyhow::Result<ApiCounts> {
    let api = out_dir.join("api");
    let forms_dir = api.join("forms");
    let _ = std::fs::remove_dir_all(&forms_dir);
    std::fs::create_dir_all(&forms_dir)?;

    // Group records by shard, then by key.
    let mut shards: BTreeMap<u32, BTreeMap<&str, Vec<&FormRecord>>> = BTreeMap::new();
    let mut keyset: std::collections::BTreeSet<&str> = Default::default();
    for r in records {
        keyset.insert(&r.key);
        shards
            .entry(shard_of(&r.key))
            .or_default()
            .entry(&r.key)
            .or_default()
            .push(r);
    }
    let (mut bytes, mut largest) = (0usize, 0usize);
    for n in 0..SHARDS {
        let mut s = format!(
            "{{\"schema_version\":{SCHEMA_VERSION},\"shard\":{n},\"license\":{},\"records\":{{",
            json_str(LICENSE)
        );
        if let Some(keys) = shards.get(&n) {
            let mut first = true;
            for (key, rs) in keys {
                if !first {
                    s.push(',');
                }
                first = false;
                let _ = write!(
                    s,
                    "{}:[{}]",
                    json_str(key),
                    rs.iter()
                        .map(|r| record_json(r))
                        .collect::<Vec<_>>()
                        .join(",")
                );
            }
        }
        s.push_str("}}\n");
        bytes += s.len();
        largest = largest.max(s.len());
        std::fs::write(forms_dir.join(format!("{n}.json")), s)?;
    }

    // lemmas.json: compact array [lemma, pos, status, probability, entry_id, gloss].
    let mut ls = format!(
        "{{\"schema_version\":{SCHEMA_VERSION},\"license\":{},\"lemmas\":[\n",
        json_str(LICENSE)
    );
    for (i, r) in lemmas.iter().enumerate() {
        if i > 0 {
            ls.push_str(",\n");
        }
        let prob = r
            .probability
            .map(|p| format!("{:.3}", p))
            .unwrap_or_else(|| "null".into());
        let _ = write!(
            ls,
            "[{},{},{},{},{},{}]",
            json_str(&r.form),
            json_str(r.pos),
            json_str(r.status),
            prob,
            r.entry_id,
            json_str(&r.gloss),
        );
    }
    ls.push_str("\n]}\n");
    bytes += ls.len();
    std::fs::write(api.join("lemmas.json"), &ls)?;

    std::fs::write(api.join("agent-guide.md"), agent_guide)?;
    bytes += agent_guide.len();

    let meta = format!(
        "{{\n  \"schema_version\": {SCHEMA_VERSION},\n  \"git\": {},\n  \"license\": {},\n  \"shards\": {SHARDS},\n  \"router\": \"fnv1a32(utf8(key)) % shards; key = to_standard(lowercase(form)) — see agent-guide.md for the fold table\",\n  \"form_records\": {},\n  \"distinct_keys\": {},\n  \"lemmas\": {},\n  \"total_bytes\": {},\n  \"largest_shard_bytes\": {},\n  \"files\": {{\n    \"forms\": \"api/forms/<n>.json\",\n    \"lemmas\": \"api/lemmas.json\",\n    \"guide\": \"api/agent-guide.md\"\n  }}\n}}\n",
        json_str(git),
        json_str(LICENSE),
        records.len(),
        keyset.len(),
        lemmas.len(),
        bytes,
        largest,
    );
    std::fs::write(api.join("meta.json"), meta)?;

    Ok(ApiCounts {
        records: records.len(),
        keys: keyset.len(),
        lemmas: lemmas.len(),
        bytes,
        largest_shard: largest,
    })
}

/// The agent-facing usage guide, written into `api/agent-guide.md`. Static
/// content (counts live in `api/meta.json`); regenerated with every export.
pub fn agent_guide() -> String {
    format!(
        r#"# Slovowiki lexical API — agent guide

Static, deterministic JSON artifacts for verifying Interslavic (Medžuslovjansky)
text. No server, no rate limits: every file is a plain static asset. Schema
version: {SCHEMA_VERSION} (see `api/meta.json`; a bump means breaking change).
License: {LICENSE}.

## Lookup protocol

1. **Fold the token** to its key: lowercase, then apply the standard-orthography
   fold (same as the site's search): `ě→e ę→e ų→u å→a ȯ→o ė→e ĺ/ľ→l ń→n ŕ→r
   ť→t ď→d ś→s ź→z ć→č đ→dž`. ASCII input like `pomocnogo` will NOT match keys
   that contain the phonemic letters (`č ž š dž`) — if your text is fully
   ASCII, also try `c→č`-style broadenings or use the site search.
2. **Route to a shard**: `n = fnv1a32(utf8(key)) % {SHARDS}` (FNV-1a, 32-bit,
   offset 0x811c9dc5, prime 16777619). Fetch `api/forms/<n>.json`.
3. **Read the analyses** under `records[key]`. Each record is a compact array:
   `[form, lemma, entry_id, pos, [analyses], source, status, probability, gloss]`
   - `form` — the flavored (etymological) spelling;
   - `lemma` → its page is `entry/<entry_id>.html`;
   - `analyses` — e.g. `"gen.jd."` (genitive singular), `"prez.3mn."`
     (present, 3rd plural), `"akuz.jd. m.živ."` (adjective, masc animate);
   - `source` — `lemma` (citation form) or `inflection`;
   - `status` — `official` / `official-only` (both verified against the
     official dictionary) or `generated` (machine reconstruction).

## Trust rules

- `status: official`/`official-only` records are verification-grade.
- `status: generated` records carry `probability` — the isotonic-calibrated
  P(this lemma matches an official decision), holdout-validated. **Treat
  p < 0.6 as a suggestion, never as verification.** Generated lemmas have NO
  inflection records on purpose: an inflected form of a wrong reconstruction
  is confidently wrong.
- Machine-proposed derivatives (the site's "Slovotvorstvo" chips) are NOT in
  this index — a missing key means "unknown to Slovowiki", not "wrong".

## Writing workflow

1. Prefer official lemmas (`api/lemmas.json`, filter by `status`).
2. Verify every token of your draft against the form index. Two-token keys
   exist for reflexive verbs (`myti se`) AND two-word official lemmas
   (`adamovo jablȯko`): try the space-joined bigram of adjacent tokens
   before falling back to unigrams.
3. Check the `gloss` — do not assume a cognate's meaning from your own Slavic
   language (see the semantic notes the check-text tool applies).
4. For unknown tokens, `cargo run -- check-text` suggests nearest known forms.
5. Cite `entry/<entry_id>.html` when you need a human-checkable source.
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_is_stable() {
        // The shard router is a wire format: these values are frozen (the JS
        // side mirrors them). Changing fnv1a32 or SHARDS is a schema break.
        assert_eq!(fnv1a32(""), 0x811c_9dc5);
        assert_eq!(fnv1a32("voda"), fnv1a32("voda"));
        assert!(shard_of("voda") < SHARDS);
        assert_eq!(form_key("Pomoćnogo"), "pomoćnogo".replace('ć', "č"));
        assert_eq!(form_key("råzumě"), "razume");
    }

    #[test]
    fn sink_merges_syncretism_and_splits_variants() {
        let mut sink = RecordSink::default();
        sink.add(
            "den / denj",
            "nom.jd.",
            "denj",
            7,
            "noun",
            "inflection",
            "official",
            None,
            "day",
        );
        sink.add(
            "den / denj",
            "akuz.jd.",
            "denj",
            7,
            "noun",
            "inflection",
            "official",
            None,
            "day",
        );
        let recs = sink.into_records();
        // Two variants, each with both (syncretic) analyses merged.
        assert_eq!(recs.len(), 2);
        for r in &recs {
            assert_eq!(
                r.analyses,
                vec!["nom.jd.".to_string(), "akuz.jd.".to_string()]
            );
        }
    }

    #[test]
    fn noun_paradigm_roundtrip_matches_cells() {
        // The round-trip guarantee at unit scale: every rendered table cell
        // variant appears among the records.
        let mut sink = RecordSink::default();
        paradigm_records(&mut sink, "voda", Pos::Noun, 1, "official", None, "water");
        let recs = sink.into_records();
        for (_, num) in NUMBERS {
            for (_, case) in CASES {
                let cell = noun_cell("voda", case, num);
                for v in cell.split('/') {
                    let v = v.trim();
                    if v.is_empty() || v == "—" {
                        continue;
                    }
                    assert!(
                        recs.iter().any(|r| r.form == v),
                        "cell variant {v} missing from records"
                    );
                }
            }
        }
    }

    #[test]
    fn verb_paradigm_roundtrip_matches_cells() {
        // The verb table and the records read the same VerbCells: every cell
        // variant the table would render appears among the records.
        let mut sink = RecordSink::default();
        paradigm_records(&mut sink, "dělati", Pos::Verb, 3, "official", None, "do");
        let recs = sink.into_records();
        let cells = verb_cells("dělati", false).expect("paradigm");
        let all = cells
            .present
            .iter()
            .chain(&cells.imperfect)
            .chain(&cells.future)
            .chain(&cells.perfect)
            .chain(&cells.pluperfect)
            .chain(&cells.conditional)
            .chain(&cells.imperative)
            .cloned()
            .chain(cells.nonfinite.iter().map(|(_, f)| f.clone()));
        for cell in all {
            for v in cell.split('/') {
                let v = v.trim();
                if v.is_empty() || v == "—" {
                    continue;
                }
                assert!(
                    recs.iter().any(|r| r.form == v),
                    "verb cell variant {v} missing from records"
                );
            }
        }
    }

    #[test]
    fn adj_paradigm_roundtrip_matches_cells() {
        let mut sink = RecordSink::default();
        paradigm_records(
            &mut sink,
            "dobry",
            Pos::Adjective,
            4,
            "official",
            None,
            "good",
        );
        let recs = sink.into_records();
        for (_, num) in NUMBERS {
            for (_, case) in CASES {
                for (_, g, a) in ADJ_COLS {
                    let cell = adj_cell("dobry", case, num, g, a);
                    for v in cell.split('/') {
                        let v = v.trim();
                        if v.is_empty() || v == "—" {
                            continue;
                        }
                        assert!(
                            recs.iter().any(|r| r.form == v),
                            "adj cell variant {v} missing from records"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn reflexive_verbs_get_two_token_keys() {
        let mut sink = RecordSink::default();
        paradigm_records(
            &mut sink,
            "myti sę",
            Pos::Verb,
            2,
            "official",
            None,
            "wash oneself",
        );
        let recs = sink.into_records();
        assert!(!recs.is_empty());
        let inf = recs
            .iter()
            .find(|r| r.analyses.iter().any(|a| a == "inf."))
            .expect("infinitive record");
        assert!(inf.form.ends_with(" sę"), "{}", inf.form);
        assert!(inf.key.ends_with(" se"), "{}", inf.key);
    }
}
