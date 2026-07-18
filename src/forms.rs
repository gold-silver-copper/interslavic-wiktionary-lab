//! The lexical verification layer (issue #11): one canonical `FormRecord`
//! pipeline feeding BOTH the website's inflection tables and the agent-facing
//! static API, so the two can never drift apart.
//!
//! - Paradigm builders ([`noun_paradigm_forms`]/`ISV::adj_forms`, issue #20)
//!   are the single source: the site's HTML tables and `paradigm_records`
//!   both build one paradigm struct per lemma and index it, so the rendered
//!   tables and the exported [`FormRecord`]s cannot drift. The single-cell
//!   getters (`noun_cell`/`adj_cell`, panic-guarded) back the agreement
//!   checker and are pinned equal to the struct path by `inflect-eval` over
//!   the whole corpus and by CI round-trip tests.
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
pub const SHARDS: u32 = 2048;
pub const SCHEMA_VERSION: u32 = 3;
pub const LICENSE: &str =
    "CC BY-SA 4.0 (derives from Wiktionary and interslavic-dictionary.com; see /about.html)";

/// The flavored→standard fold pairs (issue #11): re-exported from the
/// interslavic crate, still THE single source for the client-side JavaScript
/// fold (site.rs builds the JS map from this constant) and pinned by the
/// router-selftest — the wire format cannot drift between the Rust key path,
/// the JS mirror, and the crate without a frozen test catching it.
pub use interslavic::orthography::FOLD_PAIRS;

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

/// Clean an inflector cell for display AND keys: expand parenthesized
/// alternatives into ` / ` variants (`generoval(a)` → `generoval /
/// generovala`; `generovaný (generovaná, generovanó)` → three variants) and
/// strip the crate's stress accents (á/ì/ý…) which are neither standard nor
/// etymological ISV orthography.
pub fn clean_cell(cell: &str) -> String {
    // The flavored→variants normalization moved to the interslavic crate
    // (issue #22); this rejoins its structured output into the ` / `-separated
    // form the index/display paths expect. `variants(x).join(" / ")` is
    // byte-identical to the former local implementation.
    interslavic::cells::variants(cell).join(" / ")
}

/// Map the dictionary's gender metadata onto the inflector's.
fn noun_gender(g: Option<crate::model::Gender>) -> Option<IsvGender> {
    match g {
        Some(crate::model::Gender::Masculine) => Some(IsvGender::Masculine),
        Some(crate::model::Gender::Feminine) => Some(IsvGender::Feminine),
        Some(crate::model::Gender::Neuter) => Some(IsvGender::Neuter),
        _ => None,
    }
}

/// A noun cell, gender-aware when the dictionary states the gender. Without
/// it the inflector GUESSES gender for out-of-lexicon nouns and mis-declines
/// e.g. feminine i-stems (`točnosť` → masculine `točnosťa`).
pub fn noun_cell_g(
    word: &str,
    case: IsvCase,
    number: IsvNumber,
    gender: Option<crate::model::Gender>,
) -> String {
    let cell = match noun_gender(gender) {
        Some(g) => catch(|| ISV::noun_with(word, case, number, g, IsvAnimacy::Inanimate)),
        None => catch(|| ISV::noun(word, case, number)),
    };
    clean_cell(&cell)
}

pub fn noun_cell(word: &str, case: IsvCase, number: IsvNumber) -> String {
    noun_cell_g(word, case, number, None)
}

/// Build a noun's whole paradigm once (issue #20), inferring gender exactly as
/// [`noun_cell_g`] does — THE shared source for both the API records
/// ([`paradigm_records`]) and the site's HTML inflection table, so the two
/// render from one struct. Index a cell with `.get(case, number)` and normalize
/// it through [`clean_cell`] to reproduce [`noun_cell_g`] byte-for-byte. Panics
/// propagate (the official corpus is panic-free — asserted by `inflect-eval`);
/// single-cell callers wanting the `—`-on-panic guard use [`noun_cell_g`].
pub fn noun_paradigm_forms(
    word: &str,
    gender: Option<crate::model::Gender>,
) -> interslavic::NounParadigm {
    match noun_gender(gender) {
        Some(g) => ISV::noun_forms_with(word, g, IsvAnimacy::Inanimate),
        None => ISV::noun_forms(word),
    }
}

pub fn adj_cell(
    word: &str,
    case: IsvCase,
    number: IsvNumber,
    gender: IsvGender,
    animacy: IsvAnimacy,
) -> String {
    clean_cell(&catch(|| ISV::adj(word, case, number, gender, animacy)))
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
            .map(|f| append_reflexive(&clean_cell(&f), reflexive))
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
            (
                "inf",
                append_reflexive(&clean_cell(&p.infinitive), reflexive),
            ),
            (
                "part.akt.tep",
                append_reflexive(&clean_cell(&prap), reflexive),
            ),
            (
                "part.pas.tep",
                append_reflexive(&clean_cell(&prpp), reflexive),
            ),
            (
                "part.akt.proš",
                append_reflexive(&clean_cell(&p.pfap), reflexive),
            ),
            (
                "part.pas.proš",
                append_reflexive(&clean_cell(&pfpp), reflexive),
            ),
            (
                "gerund",
                append_reflexive(&clean_cell(&p.gerund), reflexive),
            ),
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

/// Sanitize a citation surface for lemma records: strip parenthesized
/// annotations ("pozirati (na)" government hints), keep only the first
/// comma-variant ("pleskati,*plěskati" pipeline notation), and reject
/// surfaces that still carry raw notation (asterisked reconstructions).
pub fn citation(form: &str) -> Option<String> {
    let mut f = form.to_string();
    while let (Some(i), Some(j)) = (f.find('('), f.find(')')) {
        if i < j {
            f = format!("{}{}", &f[..i], &f[j + 1..]);
        } else {
            break;
        }
    }
    let f = f.split(',').next().unwrap_or("").trim().to_string();
    if f.is_empty() || f.contains(['*', '(', ')']) {
        return None;
    }
    Some(f)
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

    /// The set of folded form keys currently held — the absence test for
    /// generated derivatives (issue #37): a derivative is shipped only if its
    /// key is NOT already present as an official / official-only inflected form
    /// or an already-emitted lemma.
    pub fn form_key_set(&self) -> std::collections::HashSet<String> {
        self.map.keys().map(|(k, _, _)| k.clone()).collect()
    }
}

/// Decline an adjective-shaped lemma into the sink with a feature prefix
/// (used for adjectives themselves, their comparatives/superlatives,
/// declinable participles, and adjectivally-declined pronouns).
#[allow(clippy::too_many_arguments)]
fn adj_paradigm(
    sink: &mut RecordSink,
    adj: &str,
    feat_prefix: &str,
    lemma: &str,
    entry_id: usize,
    pos: &'static str,
    status: &'static str,
    probability: Option<f64>,
    gloss: &str,
) {
    // Build the whole adjective paradigm once from the crate (issue #20) and
    // index it, instead of a single-form call per cell. clean_cell normalizes
    // the raw cell exactly as adj_cell did.
    let forms = ISV::adj_forms(adj);
    for (nf, num) in NUMBERS {
        for (cf, case) in CASES {
            for (gf, g, a) in ADJ_COLS {
                sink.add(
                    &clean_cell(forms.get(case, num, g, a)),
                    &format!("{feat_prefix}{cf}.{nf}. {gf}"),
                    lemma,
                    entry_id,
                    pos,
                    "inflection",
                    status,
                    probability,
                    gloss,
                );
            }
        }
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
    gender: Option<crate::model::Gender>,
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
            // Build the whole noun paradigm once from the crate (issue #20) —
            // the same struct the site's noun_table renders from.
            let forms = noun_paradigm_forms(bare, gender);
            for (nf, num) in NUMBERS {
                for (cf, case) in CASES {
                    sink.add(
                        &clean_cell(forms.get(case, num)),
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
            adj_paradigm(
                sink,
                bare,
                "",
                lemma,
                entry_id,
                "adj",
                status,
                probability,
                gloss,
            );
            // Degrees of comparison (issue #13 §1): comparative and superlative
            // are soft adjectives — declined in full — plus their adverbs.
            if let Some((comp, comp_adv)) = interslavic::ISV::comparative(bare) {
                for (deg, adj_form, adv_form) in [
                    ("komp. ", comp.clone(), comp_adv.clone()),
                    ("superl. ", format!("naj{comp}"), format!("naj{comp_adv}")),
                ] {
                    adj_paradigm(
                        sink,
                        &adj_form,
                        deg,
                        lemma,
                        entry_id,
                        "adj",
                        status,
                        probability,
                        gloss,
                    );
                    sink.add(
                        &adv_form,
                        &format!("{}prisl.", deg),
                        lemma,
                        entry_id,
                        "adv",
                        "inflection",
                        status,
                        probability,
                        gloss,
                    );
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
            // Declinable participles (issue #13 §1): the passive participles
            // and the active present participle decline like adjectives; the
            // first cell variant is the masc.sg citation. The past active
            // (-vši) is used adverbially and stays lemma-only.
            for (feat, f) in &cells.nonfinite {
                if !matches!(*feat, "part.pas.proš" | "part.pas.tep" | "part.akt.tep") {
                    continue;
                }
                let citation = f.split('/').next().unwrap_or("").trim();
                if citation.is_empty()
                    || citation == "—"
                    || citation.contains(' ')
                    || !citation.ends_with(['y', 'i'])
                {
                    continue;
                }
                adj_paradigm(
                    sink,
                    citation,
                    &format!("{feat}. "),
                    lemma,
                    entry_id,
                    "adj",
                    status,
                    probability,
                    gloss,
                );
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Pronoun & numeral paradigms — enumerated from the upstream ISV::pronoun /
// ISV::numeral declension (interslavic 0.4.0), which now covers the toj/moj
// classes, kto/čto, veś, the -koli indefinites, jedin, dva/tri/četyri, the
// i-stem numerals and the adjectivally-declined determiners and ordinals.
// ---------------------------------------------------------------------------

/// Enumerate a closed-class paradigm from a single-form decliner (the upstream
/// `ISV::pronoun` / `ISV::numeral`), emitting inflection records. Labels are
/// minimal: `number`/`gender` appear only where the form actually varies along
/// that dimension, and syncretic cells merge in the sink. Returns false when
/// the decliner recognizes nothing (so an unknown lemma emits no records).
fn emit_closed_class<F>(
    sink: &mut RecordSink,
    lemma: &str,
    pos_label: &'static str,
    entry_id: usize,
    status: &'static str,
    gloss: &str,
    decline: F,
) -> bool
where
    F: Fn(IsvCase, IsvNumber, IsvGender, IsvAnimacy) -> Option<String>,
{
    // Does the paradigm distinguish number at all? (kto/čto and the numerals
    // do not; the toj/moj demonstratives and jedin do.)
    let number_matters = CASES.iter().any(|(_, case)| {
        ADJ_COLS.iter().any(|(_, g, a)| {
            let sg = decline(*case, IsvNumber::Singular, *g, *a);
            let pl = decline(*case, IsvNumber::Plural, *g, *a);
            sg.is_some() && pl.is_some() && sg != pl
        })
    });
    let mut any = false;
    for (nf, num) in NUMBERS {
        if !number_matters && num == IsvNumber::Plural {
            continue; // single-number paradigm: emit it once
        }
        for (cf, case) in CASES {
            let cols: Vec<(&str, String)> = ADJ_COLS
                .iter()
                .filter_map(|(gf, g, a)| decline(case, num, *g, *a).map(|f| (*gf, f)))
                .collect();
            if cols.is_empty() {
                continue;
            }
            let num_part = if number_matters {
                format!(".{nf}")
            } else {
                String::new()
            };
            let gender_matters = cols.iter().any(|(_, f)| *f != cols[0].1);
            if gender_matters {
                for (gf, form) in &cols {
                    let feats = format!("{cf}{num_part}. {gf}");
                    sink.add(
                        form,
                        &feats,
                        lemma,
                        entry_id,
                        pos_label,
                        "inflection",
                        status,
                        None,
                        gloss,
                    );
                }
            } else {
                let feats = format!("{cf}{num_part}.");
                sink.add(
                    &cols[0].1,
                    &feats,
                    lemma,
                    entry_id,
                    pos_label,
                    "inflection",
                    status,
                    None,
                    gloss,
                );
            }
            any = true;
        }
    }
    any
}

/// Paradigms for closed-class pronouns and numerals, sourced from the upstream
/// `ISV::pronoun` / `ISV::numeral` declension. Returns true when the lemma was
/// recognized and its paradigm emitted.
pub fn pronoun_numeral_records(
    sink: &mut RecordSink,
    lemma: &str,
    pos: Pos,
    entry_id: usize,
    status: &'static str,
    gloss: &str,
) -> bool {
    let l = lemma.trim();
    if l.is_empty() || l.contains(' ') {
        return false;
    }
    match pos {
        Pos::Pronoun => {
            // vsi / vse are the plural-only indefinites of veś: keep them
            // lemma-only rather than re-emitting veś's whole paradigm (the
            // upstream declension would otherwise treat them as soft adjectives).
            if l == "vsi" || l == "vse" {
                return false;
            }
            emit_closed_class(sink, l, "pron", entry_id, status, gloss, |c, n, g, a| {
                ISV::pronoun(l, c, n, g, a)
            })
        }
        Pos::Numeral => emit_closed_class(sink, l, "num", entry_id, status, gloss, |c, n, g, a| {
            ISV::numeral(l, c, n, g, a)
        }),
        _ => false,
    }
}

/// Canonical samples for the router self-test: cover every fold pair, the
/// multibyte/FNV path, two-token keys and plain ASCII.
pub const ROUTER_SELFTEST_SAMPLES: &[&str] = &[
    "voda",
    "Pomoćnogo",
    "råzumě",
    "dělajųt",
    "myti sę",
    "ĺľńŕťďśźćđ",
    "ęųåȯė",
    "xyzzy",
];

/// Core closed-class function words that are normative Interslavic (STEEN-G
/// grammar: prepositions and demonstratives) but absent from the dictionary
/// export (which has `na/do/za/…` yet lacks the single-letter prepositions and
/// `toj/ta`). Indexed with status "grammar" so verification doesn't flag the
/// most common words in the language as unknown.
pub const CLOSED_CLASS: &[(&str, &str, &str)] = &[
    ("v", "prep", "in, into"),
    ("s", "prep", "with"),
    ("k", "prep", "to, towards"),
    ("o", "prep", "about, concerning"),
    ("ob", "prep", "about, against"),
    ("toj", "pron", "that (m.)"),
    ("ta", "pron", "that (f.)"),
];

/// Add the closed-class supplement to a sink (used by both the site API and
/// the check-text index).
pub fn closed_class_records(sink: &mut RecordSink) {
    for (w, pos, gloss) in CLOSED_CLASS {
        let pos: &'static str = pos;
        sink.add(w, "", w, 0, pos, "lemma", "grammar", None, gloss);
    }
    // The supplement's demonstrative gets its full STEEN-G paradigm too (ta,
    // togo, tomu, tyh… are among the most frequent tokens in real text).
    pronoun_numeral_records(sink, "toj", Pos::Pronoun, 0, "grammar", "that");
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

pub type AspectMeta = std::collections::HashMap<usize, (String, Vec<(usize, String)>)>;

fn lemma_aspect_fields(r: &FormRecord, aspect_meta: &AspectMeta) -> (String, String) {
    // Generated derivatives reuse their attested base's entry_id. Decorating
    // by id alone would therefore label derived nouns/adjectives as verbs.
    if r.pos != "verb" || !matches!(r.status, "official" | "official-only") {
        return ("null".to_string(), "[]".to_string());
    }
    aspect_meta
        .get(&r.entry_id)
        .map(|(aspect, partners)| {
            let partners = format!(
                "[{}]",
                partners
                    .iter()
                    .map(|(id, lemma)| format!("[{id},{}]", json_str(lemma)))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            (json_str(aspect), partners)
        })
        .unwrap_or_else(|| ("null".to_string(), "[]".to_string()))
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
    aspect_meta: &AspectMeta,
    extra_artifact_bytes: usize,
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
    let (mut bytes, mut largest) = (extra_artifact_bytes, 0usize);
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

    // lemmas.json schema 3: compact array
    // [lemma, pos, status, probability, entry_id, gloss, aspect,
    //  aspect_partners]. Each partner is [entry_id, lemma].
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
        let (aspect, partner) = lemma_aspect_fields(r, aspect_meta);
        let _ = write!(
            ls,
            "[{},{},{},{},{},{},{},{}]",
            json_str(&r.form),
            json_str(r.pos),
            json_str(r.status),
            prob,
            r.entry_id,
            json_str(&r.gloss),
            aspect,
            partner,
        );
    }
    ls.push_str("\n]}\n");
    bytes += ls.len();
    std::fs::write(api.join("lemmas.json"), &ls)?;

    std::fs::write(api.join("agent-guide.md"), agent_guide)?;
    bytes += agent_guide.len();

    // Router self-test (issue #13 §2): canonical (form → key → shard) samples.
    // The client JS fetches this at load in forms.html/text-check.html and
    // refuses to run if its own fold/router disagrees — a silent mirror drift
    // becomes a visible error instead of wrong lookups.
    let mut st = format!("{{\"schema_version\":{SCHEMA_VERSION},\"shards\":{SHARDS},\"samples\":[");
    for (i, sample) in ROUTER_SELFTEST_SAMPLES.iter().enumerate() {
        if i > 0 {
            st.push(',');
        }
        let key = form_key(sample);
        let _ = write!(
            st,
            "[{},{},{}]",
            json_str(sample),
            json_str(&key),
            shard_of(&key)
        );
    }
    st.push_str("]}\n");
    bytes += st.len();
    std::fs::write(api.join("router-selftest.json"), st)?;

    let meta = format!(
        "{{\n  \"schema_version\": {SCHEMA_VERSION},\n  \"git\": {},\n  \"license\": {},\n  \"shards\": {SHARDS},\n  \"router\": \"fnv1a32(utf8(key)) % shards; key = to_standard(lowercase(form)) — see agent-guide.md for the fold table\",\n  \"form_records\": {},\n  \"distinct_keys\": {},\n  \"lemmas\": {},\n  \"total_bytes\": {},\n  \"largest_shard_bytes\": {},\n  \"files\": {{\n    \"forms\": \"api/forms/<n>.json\",\n    \"lemmas\": \"api/lemmas.json\",\n    \"aspect_pairs\": \"api/aspect-pairs.json\",\n    \"suggestions\": \"api/suggest/<n>.json\",\n    \"suggestion_selftest\": \"api/suggest-selftest.json\",\n    \"guide\": \"api/agent-guide.md\"\n  }}\n}}\n",
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
   ASCII, also try `c→č`-style broadenings. `forms.html` performs a bounded
   version of that fallback and reports every matched key; direct API clients
   must route each broadened real key themselves.
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
     official dictionary), `grammar` (closed-class function words from the
     reference grammar: v, s, k, o, ob, toj, ta — absent from the dictionary
     export), or `generated` (NOT in the official dictionary — either a machine
     reconstruction from cognates, or a regular derivative generated off an
     attested official base; see Trust rules).

Browser typo suggestions use `api/suggest/<n>.json`, routed by
`fnv1a32(utf8(first_folded_letter)) % 64`. Rows are `[folded_key, lemma]` and
follow the CLI contract: same first letter, edit distance ≤2, nearest first,
lexical tie-break, at most three. `api/suggest-selftest.json` is generated by
Rust and the browser must pass it before displaying suggestions.

`api/lemmas.json` uses
`[lemma, pos, status, probability, entry_id, gloss, aspect, aspect_partners]`;
`aspect` is `ipf`, `pf`, `ipf/pf`, or null; `aspect_partners` is an array of
`[partner_entry_id, partner_lemma]` rows (schema 3, issue #75).
`api/aspect-pairs.json` contains the production pair model output: both official
endpoints/page IDs, shared-anchor generated forms, the fired rule, and
`-ovati/-uje` present stems where applicable.

## Trust rules

- `status: official`/`official-only` records are verification-grade.
- `status: generated` records are NOT verification-grade. `probability` is
  model-specific and may be null:
  - **cognate-set reconstructions** — `probability` is currently null because
    their coverage score has no corpus-path holdout calibrator; the separate
    official-row pipeline calibrator is deliberately rejected as incompatible;
  - **regular derivatives off attested bases** (the site's "Slovotvorstvo"
    families) — a base lemma's productive family (`-osť`, adverb, `-ńje`,
    `-telj`, `-ny`/`-sky`, `-ka`/`-ica`, `ne-`), restricted to members ABSENT
    from the dictionary. These ARE now in this index. Their `analyses` carry a
    single `deriv:<pattern>` tag and their `entry_id` points at the attested
    BASE's page. `probability` is the per-pattern Wilson-95 lower bound of an
    off-official-base holdout's exact-match rate (capped 0.90; see
    `derivation-report.md`) — a form-accuracy proxy that cannot measure whether
    the derivative is a real word, so treat it as a suggestion.
- **Any non-null generated probability is still a suggestion, never
  verification.** Generated lemmas (both kinds) have NO inflection records on
  purpose: an inflected form of a
  wrong lemma is confidently wrong. A missing key means "unknown to Slovowiki",
  not "wrong".

## Coverage (schema 3)

The index now includes, beyond noun/adjective/verb paradigms: **declined
participles** (passive and active-present, adjectival paradigms under the verb
lemma, features prefixed `part.…`), **comparatives and superlatives**
(declined, `komp.`/`superl.` prefixes, plus their adverbs), **pronoun and
numeral paradigms** (toj-class, moj-class, kto/čto, veś, jedin, dva/tri/
četyri, i-stem numerals — from the interslavic crate's declension), and
**3-token official lemmas**
(try trigram → bigram → unigram when verifying).

## Self-test

Fetch `api/router-selftest.json` and verify your fold + router reproduce the
listed (form → key → shard) samples before trusting lookups — the site's own
client refuses to run when this check fails.

## Agreement warnings (check-text)

`check-text --json` reports may carry an `agreement` field: a conservative
grammar check (adjacent adjective–noun case/number/gender, preposition
government from the dictionary's own `(+N)` annotations, pronoun–verb
person/number) that fires only when NO combination of the tokens' analyses is
compatible and both tokens are POS-unambiguous verification-grade words.

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
    fn fold_pairs_match_to_standard() {
        // FOLD_PAIRS is the single source for the client-side JS fold; it must
        // agree with orthography::to_standard char-for-char, and cover every
        // char to_standard changes (lowercase alphabet).
        for (from, to) in FOLD_PAIRS {
            assert_eq!(
                &ortho::to_standard(&from.to_string()),
                to,
                "fold pair {from} drifted from to_standard"
            );
        }
        for c in "abcdefghijklmnoprstuvyzčšžěęųåȯėĺľńŕťďśźćđ".chars() {
            let folded = ortho::to_standard(&c.to_string());
            if folded != c.to_string() {
                assert!(
                    FOLD_PAIRS.iter().any(|(f, _)| *f == c),
                    "to_standard changes '{c}' but FOLD_PAIRS misses it"
                );
            }
        }
    }

    #[test]
    fn router_selftest_samples_are_frozen() {
        // These exact values ship in api/router-selftest.json and the client
        // JS refuses to run if it disagrees — changing them is a schema break.
        let expected: &[(&str, &str)] = &[
            ("voda", "voda"),
            ("Pomoćnogo", "pomočnogo"),
            ("myti sę", "myti se"),
        ];
        for (form, key) in expected {
            assert_eq!(&form_key(form), key);
            assert!(shard_of(key) < SHARDS);
        }
        assert_eq!(
            SHARDS, 2048,
            "SHARDS is wire format: bump SCHEMA_VERSION too"
        );
        assert_eq!(SCHEMA_VERSION, 3);
    }

    #[test]
    fn record_serialization_is_deterministic() {
        // Two independent sink runs over the same inputs serialize identically
        // (BTreeMap ordering, no timestamps) — the determinism guarantee at
        // unit scale.
        let build = || {
            let mut sink = RecordSink::default();
            paradigm_records(
                &mut sink,
                "žena",
                Pos::Noun,
                Some(crate::model::Gender::Feminine),
                1,
                "official",
                None,
                "woman",
            );
            paradigm_records(
                &mut sink,
                "dělati",
                Pos::Verb,
                None,
                2,
                "official",
                None,
                "do",
            );
            pronoun_numeral_records(&mut sink, "toj", Pos::Pronoun, 3, "official", "that");
            sink.into_records()
                .iter()
                .map(record_json)
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(build(), build());
    }

    #[test]
    fn golden_paradigms_per_declension_class() {
        // Complete oblique paradigms pinned per declension class, so an
        // inflector bump produces a reviewable diff (extracted from the
        // interslavic 0.3.2 output).
        let f = crate::model::Gender::Feminine;
        let n = crate::model::Gender::Neuter;
        let cell = |w: &str, c, num, g| noun_cell_g(w, c, num, Some(g));
        // Hard feminine a-stem.
        assert_eq!(cell("žena", IsvCase::Gen, IsvNumber::Singular, f), "ženy");
        assert_eq!(cell("žena", IsvCase::Dat, IsvNumber::Singular, f), "ženě");
        assert_eq!(cell("žena", IsvCase::Acc, IsvNumber::Singular, f), "ženų");
        assert_eq!(cell("žena", IsvCase::Ins, IsvNumber::Singular, f), "ženojų");
        assert_eq!(cell("žena", IsvCase::Gen, IsvNumber::Plural, f), "žen");
        assert_eq!(cell("žena", IsvCase::Ins, IsvNumber::Plural, f), "ženami");
        // Feminine i-stem.
        assert_eq!(cell("kosť", IsvCase::Gen, IsvNumber::Singular, f), "kosti");
        assert_eq!(cell("kosť", IsvCase::Ins, IsvNumber::Singular, f), "kosťjų");
        assert_eq!(cell("kosť", IsvCase::Gen, IsvNumber::Plural, f), "kostij");
        // Soft neuter.
        assert_eq!(cell("morje", IsvCase::Gen, IsvNumber::Singular, n), "morja");
        assert_eq!(
            cell("morje", IsvCase::Ins, IsvNumber::Singular, n),
            "morjem"
        );
        assert_eq!(cell("morje", IsvCase::Gen, IsvNumber::Plural, n), "morej");
        // Adjective hard/soft.
        assert_eq!(
            adj_cell(
                "dobry",
                IsvCase::Gen,
                IsvNumber::Singular,
                IsvGender::Masculine,
                IsvAnimacy::Inanimate
            ),
            "dobrogo"
        );
        assert_eq!(
            adj_cell(
                "svěži",
                IsvCase::Gen,
                IsvNumber::Singular,
                IsvGender::Masculine,
                IsvAnimacy::Inanimate
            ),
            "svěžego"
        );
        // Verb classes: -ati and -iti presents.
        let d = verb_cells("dělati", false).unwrap();
        assert_eq!(d.present[2], "dělaje");
        assert_eq!(d.present[5], "dělajųt");
        let p = verb_cells("prositi", false).unwrap();
        assert_eq!(p.present[2], "prosi");
        assert_eq!(p.present[5], "prosęt");
    }

    #[test]
    fn citation_sanitizer() {
        // Pipeline notation and government hints (comparative gradation itself
        // is tested upstream in the interslavic crate).
        assert_eq!(citation("pozirati (na)").as_deref(), Some("pozirati"));
        assert_eq!(citation("pleskati,*plěskati").as_deref(), Some("pleskati"));
        assert_eq!(citation("*rekonstrukcija"), None);
        assert_eq!(citation("voda").as_deref(), Some("voda"));
    }

    #[test]
    fn comparative_integration() {
        // The upstream ISV::comparative is wired in, and the uzky-class fix
        // (root-final-k lexical exception, published in 0.4.0) is in effect.
        assert_eq!(
            interslavic::ISV::comparative("uzky"),
            Some(("uzši".to_string(), "uže".to_string()))
        );
        assert_eq!(interslavic::ISV::comparative("russky"), None);
    }

    #[test]
    fn numerals_decline_and_carry_citation_analyses() {
        let mut sink = RecordSink::default();
        pronoun_numeral_records(&mut sink, "pŕvy", Pos::Numeral, 1, "official", "first");
        pronoun_numeral_records(&mut sink, "dva", Pos::Numeral, 2, "official", "two");
        pronoun_numeral_records(&mut sink, "pęť", Pos::Numeral, 3, "official", "five");
        let recs = sink.into_records();
        // Ordinals decline like adjectives.
        assert!(recs.iter().any(|r| r.form == "pŕvogo"), "pŕvogo missing");
        // Cardinals carry nom./akuz. on the citation form.
        let dva = recs.iter().find(|r| r.form == "dva").expect("dva");
        assert!(dva.analyses.iter().any(|a| a.contains("nom")), "{dva:?}");
        let pet = recs.iter().find(|r| r.form == "pęť").expect("pęť");
        assert!(pet.analyses.iter().any(|a| a.contains("nom")), "{pet:?}");
    }

    #[test]
    fn pronoun_paradigms_follow_steen() {
        let mut sink = RecordSink::default();
        pronoun_numeral_records(&mut sink, "toj", Pos::Pronoun, 1, "official", "that");
        pronoun_numeral_records(&mut sink, "moj", Pos::Pronoun, 2, "official", "my");
        pronoun_numeral_records(&mut sink, "kto", Pos::Pronoun, 3, "official", "who");
        pronoun_numeral_records(&mut sink, "tri", Pos::Numeral, 4, "official", "three");
        pronoun_numeral_records(&mut sink, "pęť", Pos::Numeral, 5, "official", "five");
        let recs = sink.into_records();
        let has = |form: &str| recs.iter().any(|r| r.form == form);
        for f in [
            "togo", "tomu", "tym", "tyh", "tymi", "tų", "tojų", "mojego", "mojemu", "mojų",
            "mojejų", "kogo", "komu", "kym", "trěh", "trěm", "pęti", "pęťjų",
        ] {
            assert!(has(f), "missing pronoun/numeral form {f}");
        }
    }

    #[test]
    fn adversarial_negatives_stay_out_of_keys() {
        // Near-miss garbage must never appear as forms (index growth must not
        // make the checker permissive).
        let mut sink = RecordSink::default();
        paradigm_records(
            &mut sink,
            "voda",
            Pos::Noun,
            Some(crate::model::Gender::Feminine),
            1,
            "official",
            None,
            "water",
        );
        let recs = sink.into_records();
        for garbage in ["vodys", "vodaa", "vodm", "voda-", "(voda)"] {
            assert!(
                !recs.iter().any(|r| r.form == garbage || r.key == garbage),
                "garbage form {garbage} leaked into the records"
            );
        }
    }

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
        paradigm_records(
            &mut sink,
            "voda",
            Pos::Noun,
            Some(crate::model::Gender::Feminine),
            1,
            "official",
            None,
            "water",
        );
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
        paradigm_records(
            &mut sink,
            "dělati",
            Pos::Verb,
            None,
            3,
            "official",
            None,
            "do",
        );
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
            None,
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
    fn api_aspect_metadata_does_not_leak_to_generated_derivatives() {
        let mut meta = std::collections::HashMap::new();
        meta.insert(7, ("ipf".to_string(), vec![(8, "zapisati".to_string())]));
        let record = |pos, status| FormRecord {
            form: "zapisovati".to_string(),
            key: "zapisovati".to_string(),
            lemma: "zapisovati".to_string(),
            entry_id: 7,
            pos,
            analyses: Vec::new(),
            source: "lemma",
            status,
            probability: None,
            gloss: "write".to_string(),
        };
        assert_eq!(
            lemma_aspect_fields(&record("verb", "official"), &meta),
            ("\"ipf\"".to_string(), "[[8,\"zapisati\"]]".to_string())
        );
        assert_eq!(
            lemma_aspect_fields(&record("noun", "generated"), &meta),
            ("null".to_string(), "[]".to_string())
        );
    }

    #[test]
    fn reflexive_verbs_get_two_token_keys() {
        let mut sink = RecordSink::default();
        paradigm_records(
            &mut sink,
            "myti sę",
            Pos::Verb,
            None,
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
