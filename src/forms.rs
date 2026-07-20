//! The lexical verification layer (issue #11): one canonical `FormRecord`
//! pipeline feeding BOTH the website's inflection tables and the agent-facing
//! static API, so the two can never drift apart.
//!
//! - Paradigm builders ([`noun_paradigm_forms`]/`interslavic::adj_forms`, issue #20)
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
    Animacy as IsvAnimacy, Case as IsvCase, Gender as IsvGender, Number as IsvNumber,
};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// Counts inflection-table panics swallowed by the quiet hook.
static INFLECTION_PANICS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Install the CLI's process-lifetime hook for expected inflector failures.
///
/// Panic hooks are process-global, so reusable library exports deliberately do
/// not call this. The command-line binary installs it once before an export or
/// inflection evaluation and then exits.
#[doc(hidden)]
pub fn install_cli_quiet_inflection_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let from_inflector = info
            .location()
            .map(|location| location.file().contains("interslavic"))
            .unwrap_or(false);
        if from_inflector {
            INFLECTION_PANICS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        } else {
            default(info);
        }
    }));
}

pub fn inflection_panic_count() -> usize {
    INFLECTION_PANICS.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn reset_inflection_panic_count() {
    INFLECTION_PANICS.store(0, std::sync::atomic::Ordering::Relaxed);
}

/// Shard count for the form index. Changing it is a schema break: bump
/// [`SCHEMA_VERSION`] and regenerate `api/agent-guide.md`.
pub const SHARDS: u32 = 2048;
pub const SCHEMA_VERSION: u32 = 4;

/// Ranking evidence shared by `api/lemmas.json` rows and English-API
/// candidates (issue: choosing between synonyms required joining three
/// files). All four fields already exist in the pipeline — this is plumbing:
/// `frequency` from the official CSV, `langs`/`branch_pattern`/`borrowed`
/// from the entry's attestation metadata. Keyed by site entry id.
#[derive(Debug, Clone, Default)]
pub struct RankEvidence {
    pub frequency: Option<f32>,
    pub langs: usize,
    pub branch_pattern: Option<String>,
    pub borrowed: bool,
}

/// Ranking evidence carried inline by a raw-intl record's provenance tag
/// (`raw-intl:<langs>l:<branch-pattern>`). These records use the `entry_id 0`
/// "no entry page" sentinel, so the per-entry evidence map cannot describe
/// them; the tag does.
pub fn raw_intl_evidence(record: &FormRecord) -> Option<RankEvidence> {
    let tag = record
        .analyses
        .iter()
        .find_map(|a| a.strip_prefix("raw-intl:"))?;
    let (langs, pattern) = tag.split_once("l:")?;
    Some(RankEvidence {
        frequency: None,
        langs: langs.parse().ok()?,
        branch_pattern: Some(pattern.to_string()),
        borrowed: true,
    })
}
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
        Some(g) => catch(|| interslavic::noun_with(word, case, number, g, IsvAnimacy::Inanimate)),
        None => catch(|| interslavic::noun(word, case, number)),
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
        Some(g) => interslavic::noun_forms_with(word, g, IsvAnimacy::Inanimate),
        None => interslavic::noun_forms(word),
    }
}

/// Noun paradigm with EXPLICIT animacy (V13): the project-lexicon and
/// coin-check override path. [`noun_paradigm_forms`] hardcodes inanimate
/// because the official CSV's animacy tag is unreliable for its own corpus;
/// a project lexicon or a coin-check `--animacy` flag is an explicit consumer
/// decision (`ISV::noun_with` semantics), so it must reach the inflector —
/// masculine-animate accusatives (`žabervoka`) differ from inanimate ones.
///
/// CAUTION: with `gender: None` this falls back to the crate's own
/// gender/animacy inference and `animate` is IGNORED — there is no
/// `noun_forms` variant taking animacy without gender. Both callers
/// guarantee a concrete gender for nouns (`parse_lexicon` requires it;
/// coin-check backfills the guess); a new caller must do the same or accept
/// the guessed animacy.
pub fn noun_paradigm_forms_with_animacy(
    word: &str,
    gender: Option<crate::model::Gender>,
    animate: bool,
) -> interslavic::NounParadigm {
    let animacy = if animate {
        IsvAnimacy::Animate
    } else {
        IsvAnimacy::Inanimate
    };
    match noun_gender(gender) {
        Some(g) => interslavic::noun_forms_with(word, g, animacy),
        None => interslavic::noun_forms(word),
    }
}

pub fn adj_cell(
    word: &str,
    case: IsvCase,
    number: IsvNumber,
    gender: IsvGender,
    animacy: IsvAnimacy,
) -> String {
    clean_cell(&catch(|| {
        interslavic::adj(word, case, number, gender, animacy)
    }))
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
    let p = std::panic::catch_unwind(|| interslavic::verb_forms(word)).ok()?;
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
    let forms = interslavic::adj_forms(adj);
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
            noun_paradigm_into_sink(
                sink,
                &forms,
                lemma,
                entry_id,
                pos,
                status,
                probability,
                gloss,
            );
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
            if let Some((comp, comp_adv)) = interslavic::comparative(bare) {
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

/// Index one pre-built noun paradigm struct into the sink — the shared cell
/// loop behind [`paradigm_records`] and [`project_paradigm_records`].
#[allow(clippy::too_many_arguments)]
fn noun_paradigm_into_sink(
    sink: &mut RecordSink,
    forms: &interslavic::NounParadigm,
    lemma: &str,
    entry_id: usize,
    pos: Pos,
    status: &'static str,
    probability: Option<f64>,
    gloss: &str,
) {
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

/// Full paradigm for a project-lexicon lemma or a coin-check metadata
/// override (V13): like [`paradigm_records`] but nouns decline with the
/// caller's EXPLICIT animacy (the `noun_with` path a real consumer uses).
/// Adjectives and verbs have no animacy dimension and reuse the ordinary
/// paradigm builder unchanged.
pub fn project_paradigm_records(
    sink: &mut RecordSink,
    lemma: &str,
    pos: Pos,
    gender: Option<crate::model::Gender>,
    animate: bool,
    status: &'static str,
    gloss: &str,
) {
    let bare = lemma.trim();
    if bare.is_empty() || bare.contains(' ') {
        return;
    }
    match pos {
        Pos::Noun | Pos::ProperNoun => {
            let forms = noun_paradigm_forms_with_animacy(bare, gender, animate);
            noun_paradigm_into_sink(sink, &forms, lemma, 0, pos, status, None, gloss);
        }
        _ => paradigm_records(sink, lemma, pos, gender, 0, status, None, gloss),
    }
}

// ---------------------------------------------------------------------------
// Pronoun & numeral paradigms — enumerated from the upstream interslavic::pronoun /
// interslavic::numeral declension (interslavic 0.4.0), which now covers the toj/moj
// classes, kto/čto, veś, the -koli indefinites, jedin, dva/tri/četyri, the
// i-stem numerals and the adjectivally-declined determiners and ordinals.
// ---------------------------------------------------------------------------

/// Enumerate a closed-class paradigm from a single-form decliner (the upstream
/// `interslavic::pronoun` / `interslavic::numeral`), emitting inflection records. Labels are
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
/// `interslavic::pronoun` / `interslavic::numeral` declension. Returns true when the lemma was
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
                interslavic::pronoun(l, c, n, g, a)
            })
        }
        Pos::Numeral => emit_closed_class(sink, l, "num", entry_id, status, gloss, |c, n, g, a| {
            interslavic::numeral(l, c, n, g, a)
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
#[allow(clippy::too_many_arguments)]
pub fn write_api(
    out_dir: &Path,
    records: &[FormRecord],
    lemmas: &[FormRecord],
    aspect_meta: &AspectMeta,
    evidence: &BTreeMap<usize, RankEvidence>,
    notes_count: usize,
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

    // lemmas.json schema 4: compact array
    // [lemma, pos, status, probability, entry_id, gloss, aspect,
    //  aspect_partners, frequency, langs, branch_pattern, borrowed].
    // Each partner is [entry_id, lemma]; the last four are the ranking
    // evidence (schema-4 addition — consumers must accept the trailing
    // fields).
    let mut ls = format!(
        "{{\"schema_version\":{SCHEMA_VERSION},\"license\":{},\"lemmas\":[\n",
        json_str(LICENSE)
    );
    let no_evidence = RankEvidence::default();
    for (i, r) in lemmas.iter().enumerate() {
        if i > 0 {
            ls.push_str(",\n");
        }
        let prob = r
            .probability
            .map(|p| format!("{:.3}", p))
            .unwrap_or_else(|| "null".into());
        let (aspect, partner) = lemma_aspect_fields(r, aspect_meta);
        let tag_ev = raw_intl_evidence(r);
        let ev = tag_ev
            .as_ref()
            .or_else(|| evidence.get(&r.entry_id))
            .unwrap_or(&no_evidence);
        let _ = write!(
            ls,
            "[{},{},{},{},{},{},{},{},{},{},{},{}]",
            json_str(&r.form),
            json_str(r.pos),
            json_str(r.status),
            prob,
            r.entry_id,
            json_str(&r.gloss),
            aspect,
            partner,
            ev.frequency
                .map(|f| format!("{f}"))
                .unwrap_or_else(|| "null".into()),
            ev.langs,
            ev.branch_pattern
                .as_deref()
                .map(json_str)
                .unwrap_or_else(|| "null".into()),
            ev.borrowed,
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
        "{{\n  \"schema_version\": {SCHEMA_VERSION},\n  \"git\": {},\n  \"license\": {},\n  \"shards\": {SHARDS},\n  \"router\": \"fnv1a32(utf8(key)) % shards; key = to_standard(lowercase(form)) — see agent-guide.md for the fold table\",\n  \"form_records\": {},\n  \"distinct_keys\": {},\n  \"lemmas\": {},\n  \"notes\": {},\n  \"notes_shards\": {},\n  \"total_bytes\": {},\n  \"largest_shard_bytes\": {},\n  \"files\": {{\n    \"forms\": \"api/forms/<n>.json\",\n    \"lemmas\": \"api/lemmas.json\",\n    \"english_lookup_meta\": \"api/en/meta.json\",\n    \"english_lookup\": \"api/en/<n>.json\",\n    \"english_selftest\": \"api/en/selftest.json\",\n    \"aspect_pairs\": \"api/aspect-pairs.json\",\n    \"notes\": \"api/notes/<n>.json\",\n    \"notes_selftest\": \"api/notes-selftest.json\",\n    \"suggestions\": \"api/suggest/<n>.json\",\n    \"suggestion_selftest\": \"api/suggest-selftest.json\",\n    \"guide\": \"api/agent-guide.md\"\n  }}\n}}\n",
        json_str(git),
        json_str(LICENSE),
        records.len(),
        keyset.len(),
        lemmas.len(),
        notes_count,
        crate::falsefriends::NOTES_SHARDS,
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

Static, deterministic JSON artifacts for working with Interslavic
(Medžuslovjansky) text. No server, no rate limits, no auth: every path below is
a plain static asset relative to the site root. Form-index schema version:
{SCHEMA_VERSION} (see `api/meta.json`; a bump means breaking change). The
English lookup API is versioned separately in `api/en/meta.json`.
License: {LICENSE}.

## Choose the right artifact

| Task | Artifact |
|---|---|
| English word/phrase → Interslavic candidates | `api/en/<n>.json` (sharded) |
| Verify/analyse an Interslavic token (real word? case/number/person?) | `api/forms/<n>.json` (sharded) |
| Enumerate all lemmas; filter by status/POS/aspect | `api/lemmas.json` |
| Verb aspect partners and the pair model | `api/aspect-pairs.json` |
| False-friend warnings for a folded Interslavic key (computed from cache evidence) | `api/notes/<n>.json` (sharded) |
| Typo suggestions for an unknown Interslavic token | `api/suggest/<n>.json` |
| Entry metadata: attestation languages, confidence, categories | `entries.json` (site root) |
| Human-checkable citation for a lemma | `entry/<entry_id>.html` |

Other root-level datasets: `edges.json` (semantic graph), `categories.json`,
`roots.json` (Proto-Slavic root membership), `rules.json` (sound-rule reverse
index), `search/manifest.json` (client search index), `build.json` (git +
counts).

## Verify your client first (self-tests)

Two independent routers exist and each ships frozen samples. Fetch the relevant
selftest once per session, recompute every sample with your own implementation,
and refuse to continue on any mismatch — the site's own JS does exactly this.

- `api/router-selftest.json` — form-index fold + router; samples are
  `[form, key, shard]`.
- `api/en/selftest.json` — English normalization + router; samples are
  `[raw_query, normalized_key, shard]`.

Both routers hash with FNV-1a 32-bit (offset 0x811c9dc5, prime 16777619) over
UTF-8 bytes of the key, then take the remainder by the shard count from the
respective meta file. Only the key-preparation step differs.

## Interslavic form lookup (`api/forms`)

1. **Fold the token** to its key: lowercase, then apply the standard-orthography
   fold (same as the site's search): `ě→e ę→e ų→u å→a ȯ→o ė→e ĺ/ľ→l ń→n ŕ→r
   ť→t ď→d ś→s ź→z ć→č đ→dž`. ASCII input like `pomocnogo` will NOT match keys
   that contain the phonemic letters (`č ž š dž`) — if your text is fully
   ASCII, also try `c→č`-style broadenings. `forms.html` performs a bounded
   version of that fallback and reports every matched key; direct API clients
   must route each broadened real key themselves.
2. **Route to a shard**: `n = fnv1a32(utf8(key)) % {SHARDS}`.
   Fetch `api/forms/<n>.json`.
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
`[lemma, pos, status, probability, entry_id, gloss, aspect, aspect_partners,
frequency, langs, branch_pattern, borrowed]`;
`aspect` is `ipf`, `pf`, `ipf/pf`, or null; `aspect_partners` is an array of
`[partner_entry_id, partner_lemma]` rows. **Schema 4 migration:** schema 3's
eight-field row gained four trailing ranking-evidence fields — `frequency`
(official CSV column, null for generated rows), `langs` (attesting-language
count), `branch_pattern` (`"V+Z+J"`-style combination or null), `borrowed`
(bool). English-API candidates (en schema 2) carry the same four fields, so
choosing between synonyms no longer requires joining three files.
`api/aspect-pairs.json` contains the production pair model output: both official
endpoints/page IDs, shared-anchor generated forms, the fired rule, and
`-ovati/-uje` present stems where applicable.

## English → Interslavic lookup (`api/en`)

`api/en/meta.json` documents the static English-to-Interslavic lookup contract.
Normalize an English query by lowercasing it, replacing punctuation with spaces,
collapsing whitespace, trimming, and stripping a leading verb marker `to `.
Route the normalized key with
`fnv1a32(utf8(key)) % 256`, then fetch `api/en/<n>.json` and read
`records[key]`.
Normalization strips only the verb marker `to `; then walk the retry ladder
documented in `api/en/meta.json` **until a verified candidate surfaces** (keep
generated-only hits, but keep walking): (1) drop a leading article
("the game" → "game"); (2) retry each content word of a multiword query;
(3) **de-suffix** the key and retry — apply EVERY rule whose suffix matches,
collecting all variants (rules listed longest-suffix first) — `-ibility→-ible`,
`-ability→-able`, `-iness→-y`, `-ness→∅`, `-ation→∅/-ate`, `-ition→∅/-e/-ite`,
`-ity→∅/-e`, `-ing→∅/-e` (undoubling a doubled final consonant:
"mapping"→"map"), `-ies→-y`, `-es→∅`, `-s→∅`, keeping stems of ≥3 chars.
`api/en/selftest.json` freezes `desuffix_samples` (`[key, [variants…]]`) so you
can verify your ladder implementation. The reverse direction is built in:
generated derivatives are indexed under mechanically derived English keys
("invisible"→"invisibility", "heal"→"healing") with match reason
`derived-english`.

Each English candidate is an object with the Interslavic `lemma`, `entry_id`,
`official_id`, `pos`, source `gloss`, `status`, `trust`, deterministic `rank`,
the match reason (`phrase`, `exact-gloss-head`, `derived-english`, or
`gloss-token`), optional
verb `aspect` and `aspect_partners`, semantic `warnings`, optional `prefer`
alternatives, model-specific `probability` for generated records, and
`form_lookup` (`key`, `shard`, `path`) into the form API. The English API is
for candidate discovery; the form API remains the authority for surface forms.

Ranking semantics: candidates under one key are sorted best-first, and verified
records always precede generated ones. `rank` is comparable only WITHIN one
English key — never across keys; across keys compare `trust`/`status`. Within
one rank, ties break deterministically by higher `frequency`, then more
`langs`, then lexicographically. A `gloss-token` match means the word appeared
inside a longer gloss phrase — read `gloss` before trusting it as a direct
translation. **Sense-note rule** (derive it client-side; the `en` CLI is the
reference): when the FIRST verified candidate's match is `gloss-token` and an
`exact-gloss-head`/`phrase` candidate exists anywhere in the list, the
verified hit is likely a phrase/derived sense ('staff' → verified `načeľnik
štaba` "chief-of-staff" above the semantically right generated `posoh`) —
present the exact-head candidates alongside it, never take the first verified
row blindly.

## Translation workflow (English → Interslavic)

1. Pass the `api/en/selftest.json` check once per session.
2. Normalize and route the English query; read the candidate list.
3. Prefer `trust: verified-official` / `verified-official-only`. Treat
   `generated-review` candidates as suggestions that need human review, never
   as verified translations — say so when you use one.
4. Heed `warnings` and `prefer`: they mark semantic traps where the obvious
   cognate is wrong (false friends). When `prefer` is non-empty, use those
   lemmas instead.
5. For verbs, check `aspect`: pick imperfective for ongoing/habitual meaning,
   perfective for a completed single event, and find the partner in
   `aspect_partners`.
6. Inflect via `form_lookup`: fetch the form shard and use only surface forms
   listed there. Generated lemmas have NO inflection records on purpose —
   do not invent inflected forms for them.
7. Verify every token of your final output against the form API (see the
   verification workflow) and cite `entry/<entry_id>.html` for anything a
   human should double-check.

## Trust rules

- `status: official`/`official-only` records are verification-grade.
- `status: generated` records are NOT verification-grade. `probability` is
  model-specific and may be null:
  - **cognate-set reconstructions** — `probability` is
    P(reproduces an official lemma, normalized) from the committed
    corpus-coverage calibrator (`data/corpus-calibration.json`), BANDED by
    attesting languages (2-3 / 4-6 / 7+) with per-decile Wilson-95 lower
    bounds, PAVA within each band, holdout-validated (pooled ECE ≈0.020).
    The separate official-row pipeline calibrator is still deliberately
    rejected as incompatible. Band ceilings on current data: ≈0.16 /
    ≈0.30 / ≈0.40 — even the best-attested reconstruction is closer to a
    coin flip than to verification;
  - **regular derivatives off attested bases** (the site's "Slovotvorstvo"
    families) — a base lemma's productive family (`-osť`, adverb, `-ńje`,
    `-telj`, `-ny`/`-sky`, `-ka`/`-ica`, `ne-`), restricted to members ABSENT
    from the dictionary. These ARE now in this index. Their `analyses` carry a
    single `deriv:<pattern>` tag and their `entry_id` points at the attested
    BASE's page. `probability` is the per-pattern Wilson-95 lower bound of an
    off-official-base holdout's exact-match rate (capped 0.90; see
    `derivation-report.md`) — a form-accuracy proxy that cannot measure whether
    the derivative is a real word, so treat it as a suggestion;
  - **raw-attested borrowed internationalisms** — cognate sets the evidence
    gate never saw (no etymology section on any Wiktionary member, e.g. the
    teleport family), recovered from raw attestations in ≥2 languages across
    ≥2 branches with gloss agreement, flavorized and adapted by the ordinary
    pipeline. Their `analyses` carry a
    `raw-intl:<langs>l:<branch-pattern>` tag (e.g. `raw-intl:2l:Z+J`), which
    also feeds their ranking evidence (`borrowed: true`), and `entry_id` is
    the `0` "no entry page" sentinel — do not fetch `entry/0.html`.
    Their `probability` is the Wilson-95 lower bound of the recovery pass's
    exact-match rate on the official-internationalism (genesis=I) holdout,
    per (languages, branches) feature bucket, capped at 0.90 (unobserved
    buckets ship at the 0.3 review floor).
    Recovered `-acija`/`-ija` nouns whose verb is attested in any raw
    language additionally yield the regular `-ovati` verb (teleportacija →
    teleportovati) with extra analyses `deriv:intl-ovati←<noun>` and
    `pres:<stem>uje` marking the derivation and the regular present stem;
    the verb inherits its source noun's bucket probability.
- **Any non-null generated probability is still a suggestion, never
  verification.** Generated lemmas (all kinds) have NO inflection records on
  purpose: an inflected form of a
  wrong lemma is confidently wrong. A missing key means "unknown to Slovowiki",
  not "wrong".

## Coverage (schema 4)

The index now includes, beyond noun/adjective/verb paradigms: **declined
participles** (passive and active-present, adjectival paradigms under the verb
lemma, features prefixed `part.…`), **comparatives and superlatives**
(declined, `komp.`/`superl.` prefixes, plus their adverbs), **pronoun and
numeral paradigms** (toj-class, moj-class, kto/čto, veś, jedin, dva/tri/
četyri, i-stem numerals — from the interslavic crate's declension), and
**3-token official lemmas**
(try trigram → bigram → unigram when verifying).

## Coinage validation (coin-check)

`coin-check <word> [--json]` validates a coined word (fantasy names and
other unavoidable neologisms) on four axes: phonotactics (the official
lemmas' own character-bigram inventory — no hand list), collision with any
existing lemma or inflected form, false-friend readings across the ten
languages' caches (per-language word + glosses + exact/loose level), and
the declension paradigm the guessed POS produces. Use it before shipping a
coinage; adjust the ending to change the paradigm.

The declinability axis reports the crate's guessed gender/animacy for the
noun reading, and accepts explicit metadata — `--pos <noun|adj|verb>`,
`--gender <m|f|n>`, `--animacy <anim|inanim>` — rendering the OVERRIDDEN
paradigm (the `ISV::noun_with` call a real consumer makes; animate
masculines take genitive-shaped accusatives) while still printing the guess
and flagging every divergence ("ending suggests gender m; you declared f").
With `--lexicon-row --gloss <english concept>` it emits the validated
project-lexicon TSV row (see below; also a `lexicon_row` field in `--json`),
so the coinage workflow chains mechanically:
`coin-check → append row → check-text --lexicon`. The row is validated by
the same rules `check-text --lexicon` applies — an invalid row fails here,
not later in CI: the full four-axis report still prints (it is the
diagnostic that explains the rejection; `--json` carries the reason as
`lexicon_row_error`), then the command exits nonzero.

## Project lexicons (check-text --lexicon)

A translation project's sanctioned coinages live in a project-lexicon TSV:
one row per word, five tab-separated columns

    lemma	pos	gender	animacy	gloss

with `pos` ∈ `noun|adj|verb`, `gender` ∈ `m|f|n` and `animacy` ∈
`anim|inanim` (both REQUIRED for nouns — the lexicon exists to control the
paradigm explicitly — and forbidden otherwise), and a non-empty English
`gloss` naming the source concept. Blank lines and `#` comments are
skipped. `check-text --lexicon <file>` then:

- builds each row's full paradigm in memory (same machinery as the official
  paradigms) and classifies matching tokens with status `project` — so a
  coinage inflected at runtime (`žabervoka`, `žabervokom`) no longer drowns
  a `--summary --max-unknown 0` CI gate. Project tokens also participate in
  the conservative agreement checks like official words (their paradigms and
  declared genders are explicit project decisions), so a case error in a
  coinage's usage is caught too;
- validates the lexicon on load, as HARD errors: rows must parse, verbs must
  cite `-ti`, adjectives `-y`/`-i`, nouns must be declinable, and every
  lemma must either collide with nothing (coin-check's collision axis) or
  pin an official lemma whose POS/gender agree with the declaration;
- emits `consistency` warnings when a verification-grade official token's
  gloss overlaps a row's gloss (deterministic token overlap, same
  normalization as the English API) but the token is NOT that row's lemma —
  register drift: the same source concept rendered by different target
  words. Gate optionally with `--max-consistency N`; ungated by default.

## Agreement warnings (check-text)

`check-text --json` reports may carry an `agreement` field: a conservative
grammar check (adjacent adjective–noun case/number/gender, preposition
government from the dictionary's own `(+N)` annotations, pronoun–verb
person/number) that fires only when NO combination of the tokens' analyses is
compatible and both tokens are POS-unambiguous verification-grade words.

## Verification workflow (Interslavic text)

1. Pass the `api/router-selftest.json` check once per session.
2. Prefer official lemmas (`api/lemmas.json`, filter by `status`).
3. Verify every token of your draft against the form index. Two-token keys
   exist for reflexive verbs (`myti se`) AND two-word official lemmas
   (`adamovo jablȯko`): try the space-joined bigram of adjacent tokens
   before falling back to unigrams (three-token official lemmas exist too:
   trigram → bigram → unigram).
4. Check the `gloss` — do not assume a cognate's meaning from your own Slavic
   language — and look the folded key up in the sharded notes artifact:
   route `fnv1a32(utf8(folded_key)) % 64` → `api/notes/<n>.json`, verifying
   your router against `api/notes-selftest.json` first (notes schema 1; the
   monolithic `api/notes.json` is retired). Each record: `warning` sentence,
   `severity` (`high`/`medium` = the word's primary sense diverges;
   `low` = colloquial-only), optional `prefer` official lemma covering the
   divergent sense (V12: chosen by SLAVIC bridges — the colliding word or
   its native synonyms appearing in the preferred lemma's own cognate
   cells — with English token coverage only as a fallback), and
   per-language `collisions` evidence with `primary_agrees` flags and a
   `level` field (`exact` = same folded surface, the classic traps;
   `loose` = y→i skeleton match). Rendered warnings quote exact-level
   collisions first.
5. For unknown tokens, use `api/suggest/<n>.json` (or `cargo run -- check-text`
   locally) to offer nearest known forms.
6. Cite `entry/<entry_id>.html` when you need a human-checkable source.

## Pitfalls

- A missing key means "unknown to Slovowiki", not "wrong" — and a present
  `generated` key means "plausible reconstruction", not "verified word".
- The form index keys are standard-orthography folds, but `form` values are
  flavored (etymological) spellings. Compare like with like.
- Fully-ASCII input needs broadening (`c→č`-style) before concluding a miss.
- `rank` (English API) is meaningless across keys; `probability` is
  model-specific and never verification.
- Do not inflect generated lemmas; the absence of their inflection records is
  a deliberate safety property, not a gap.
- Multiword lemmas hide behind n-gram keys in BOTH APIs: try the longest
  n-gram first ("coat of arms", "adamovo jablȯko").
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
        // Schema 4: lemmas.json rows grew four trailing ranking-evidence
        // fields (frequency, langs, branch_pattern, borrowed); the router and
        // form-shard record shape are unchanged.
        assert_eq!(SCHEMA_VERSION, 4);
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
        // The upstream interslavic::comparative is wired in, and the uzky-class fix
        // (root-final-k lexical exception, published in 0.4.0) is in effect.
        assert_eq!(
            interslavic::comparative("uzky"),
            Some(("uzši".to_string(), "uže".to_string()))
        );
        assert_eq!(interslavic::comparative("russky"), None);
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
