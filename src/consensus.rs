//! Modern-Slavic consensus engine.
//!
//! Given the cognate forms of one meaning across the modern Slavic languages,
//! reconstruct the Interslavic lemma the way the language's own vocabulary was
//! built: by consensus, balanced across the East / West / South branches so no
//! single large language dominates.
//!
//! The pipeline is:
//!   1. Normalize every cognate to phonemic Latin (done upstream).
//!   2. Vote on a *consonant-skeleton* alignment key, counting **branches**
//!      rather than languages. Because every form in a meaning group is cognate
//!      by construction, an aggressive key is safe and makes East pleophony /
//!      *g→h / vowel shifts collapse onto one fingerprint.
//!   3. Pick the surface representative closest to Interslavic orthography from
//!      the winning group, then run a set of individually-toggleable
//!      etymological repairs (nasal recovery, liquid metathesis, jat, palatals)
//!      to recover the flavored spelling.
//!   4. Score from branch coverage and agreement, and calibrate confidence.
//!
//! Every repair is gated by [`ConsensusConfig`] so its accuracy effect can be
//! measured in isolation on the benchmark.

use crate::lang::{Branch, LANGS};
use crate::model::{Candidate, CandidateSource, Evidence, EvidenceRelation, Gender, Pos, RuleStep};
use crate::normalize::NormForm;
use crate::orthography as ortho;
use std::collections::BTreeMap;

const DOC_DESIGN: &str = "https://interslavic.fun/learn/misc/design-criteria/";
const DOC_ORTHO: &str = "https://interslavic.fun/learn/orthography/";

/// One attested cognate form feeding the consensus.
#[derive(Debug, Clone)]
pub struct SourceForm {
    pub lang_code: String,
    pub branch: Branch,
    pub modern: bool,
    pub norm: NormForm,
    pub source_url: String,
    /// The canonical (first) translation for this language. Only primary forms
    /// vote for the top candidate; secondary variants can seed *alternatives*.
    pub primary: bool,
}

/// The consensus input for a single meaning.
#[derive(Debug, Clone)]
pub struct MeaningInput {
    pub pos: Pos,
    pub gender: Option<Gender>,
    pub gloss: String,
    pub forms: Vec<SourceForm>,
    /// The official dictionary marks this concept as an internationalism
    /// (`genesis = I`) — meaning-level metadata, used to prefer the international
    /// cluster. Not the answer form.
    pub is_intl_meaning: bool,
}

/// Toggles for each etymological repair, so the benchmark can attribute the
/// accuracy delta of every rule.
#[derive(Debug, Clone, Copy)]
pub struct ConsensusConfig {
    /// Count branches (not languages) when voting; the core anti-domination rule.
    pub branch_balanced: bool,
    /// Prefer a South-Slavic surface representative (closest to ISV shape).
    pub prefer_south_representative: bool,
    /// Recover nasal vowels (ę/ų) from the Polish cognate.
    pub nasal_from_polish: bool,
    /// Recover ć/đ (*tj/*dj) from the South-Slavic (esp. Serbo-Croatian) reflex.
    pub palatal_from_south: bool,
    /// Undo East-Slavic pleophony when an East form is the representative.
    pub depleophony: bool,
    /// Recover jat (ě) from the cross-branch reflex signature.
    pub jat_reconstruction: bool,
    /// Normalize native POS lemma endings (noun/adj/verb, §3).
    pub lemma_endings: bool,
    /// Apply the internationalism ending table (§5.2).
    pub internationalism: bool,
    /// Use the six-subgroup vote (§4.1) instead of three coarse branches.
    pub six_subgroup_vote: bool,
    /// Normalize verbal/nominal prefixes (råz-, prěd-).
    pub prefix_normalization: bool,
    /// Recover *y (kept in ISV) from East/West where South merged it to i.
    pub y_recovery: bool,
    /// Use a long-form-adjective representative (ru/pl/cs) for adjectives.
    pub adj_longform_rep: bool,
    /// Derive the form from a linked Proto-Slavic reconstruction (§4.4): consensus
    /// picks the root, the Proto-Slavic rule engine supplies the flavored form.
    pub proto_derived_form: bool,
    /// Prefer the internationalism cluster over native synonyms (ISV design
    /// criteria favor international roots for modern vocabulary).
    pub internationalism_preference: bool,
    /// Drop a South-Slavic adjective's fleeting vowel before -y, gated on the
    /// East/West long form showing the two consonants adjacent (dobar→dobry).
    pub adj_fleeting_drop: bool,
    /// Seed alternative candidates from secondary (non-primary) translations so
    /// the official lemma can appear in top-3/top-5. Never changes top-1.
    pub synonym_alternatives: bool,
}

impl ConsensusConfig {
    /// The pre-existing behavior: transliterate the first available form, no
    /// branch balancing and no repairs. Used to measure the baseline.
    pub fn baseline() -> Self {
        ConsensusConfig {
            branch_balanced: false,
            prefer_south_representative: false,
            nasal_from_polish: false,
            palatal_from_south: false,
            depleophony: false,
            jat_reconstruction: false,
            lemma_endings: false,
            internationalism: false,
            six_subgroup_vote: false,
            prefix_normalization: false,
            y_recovery: false,
            adj_longform_rep: false,
            proto_derived_form: false,
            internationalism_preference: false,
            adj_fleeting_drop: false,
            synonym_alternatives: false,
        }
    }

    /// The configuration kept after benchmarking: every rule that improved
    /// measured accuracy, none that regressed (palatals and jat are excluded —
    /// the ablation ladder shows they regress in the consensus path). This is
    /// what the production site uses.
    pub fn production() -> Self {
        ConsensusConfig {
            branch_balanced: true,
            prefer_south_representative: true,
            nasal_from_polish: true,
            palatal_from_south: false,
            depleophony: true,
            jat_reconstruction: false,
            lemma_endings: true,
            internationalism: true,
            six_subgroup_vote: true,
            prefix_normalization: true,
            // Two-stage §4.4: derive the flavored form from the linked
            // Proto-Slavic reconstruction (kept — improves exact match).
            proto_derived_form: true,
            internationalism_preference: true,
            adj_fleeting_drop: true,
            synonym_alternatives: true,
            // Rejected by the benchmark (regress accuracy in the consensus path):
            y_recovery: false,
            adj_longform_rep: false,
        }
    }

    pub fn full() -> Self {
        ConsensusConfig {
            branch_balanced: true,
            prefer_south_representative: true,
            nasal_from_polish: true,
            palatal_from_south: true,
            depleophony: true,
            jat_reconstruction: true,
            lemma_endings: true,
            internationalism: true,
            six_subgroup_vote: true,
            prefix_normalization: true,
            y_recovery: true,
            adj_longform_rep: true,
            proto_derived_form: true,
            internationalism_preference: true,
            adj_fleeting_drop: true,
            synonym_alternatives: true,
        }
    }
}

/// Surface-representative language priority: closest-to-Interslavic first.
/// Slovene/Croatian/Serbian keep *g and have the metathesized liquid
/// diphthongs; Czech/Slovak add clean sibilants; East Slavic is last (pleophony).
const REP_PRIORITY: &[&str] = &[
    // *g-preserving languages first (Interslavic keeps *g as g); Czech/Slovak/
    // Ukrainian/Belarusian, which shifted *g→h, come after so the surface keeps g.
    "sl", "hr", "sr", "pl", "bg", "mk", "ru", "cs", "sk", "uk", "be",
];
const REP_PRIORITY_NO_SOUTH_BIAS: &[&str] = &[
    "ru", "pl", "cs", "uk", "sk", "sl", "hr", "sr", "bg", "mk", "be",
];
/// Adjective representative priority: long-form languages first (they keep the
/// full -y/-ý ending and the *y vowel), South last.
const REP_PRIORITY_ADJ: &[&str] = &[
    "ru", "pl", "cs", "sk", "uk", "be", "hr", "sr", "sl", "bg", "mk",
];

/// Generate ranked Interslavic candidates from modern-Slavic consensus.
pub fn generate(input: &MeaningInput, cfg: &ConsensusConfig) -> Vec<Candidate> {
    // The top-1 vote uses only each language's primary (canonical) translation;
    // secondary variants are kept aside to seed alternatives (see below).
    let mut per_lang: BTreeMap<&str, &SourceForm> = BTreeMap::new();
    for f in &input.forms {
        if !f.modern || !f.primary {
            continue;
        }
        per_lang.entry(f.lang_code.as_str()).or_insert(f);
    }
    if per_lang.is_empty() {
        return Vec::new();
    }

    // Group languages by consonant-skeleton key.
    struct Group<'a> {
        key: String,
        langs: Vec<&'a SourceForm>,
        branches: Vec<Branch>,
    }
    let mut groups: Vec<Group> = Vec::new();
    for f in per_lang.values() {
        let key = ortho::consonant_key(&f.norm.latin);
        if let Some(g) = groups.iter_mut().find(|g| g.key == key) {
            g.langs.push(f);
            if !g.branches.contains(&f.branch) {
                g.branches.push(f.branch);
            }
        } else {
            groups.push(Group {
                key,
                langs: vec![f],
                branches: vec![f.branch],
            });
        }
    }

    // Rank groups by vote strength. With the six-subgroup vote each dialect
    // subgroup contributes one vote (½ on internal splits); otherwise fall back
    // to distinct-branch coverage; the plain-majority baseline uses raw count.
    let vote = |g: &Group| -> f32 {
        let base = if cfg.six_subgroup_vote {
            subgroup_score(&g.langs, &per_lang)
        } else if cfg.branch_balanced {
            g.branches.len() as f32
        } else {
            0.0
        };
        // Interslavic prefers the international root for modern/technical
        // vocabulary; boost a recognizably-international cluster so it outranks a
        // more-widespread native synonym (aeroplan over samolot).
        let intl_bonus = if cfg.internationalism_preference
            && input.is_intl_meaning
            && g.langs.iter().any(|f| is_international_form(&f.norm.latin))
        {
            2.0
        } else {
            0.0
        };
        base + intl_bonus
    };
    groups.sort_by(|a, b| {
        vote(b)
            .total_cmp(&vote(a))
            .then(b.langs.len().cmp(&a.langs.len()))
            .then(population_weight(&b.langs).total_cmp(&population_weight(&a.langs)))
            .then(a.key.cmp(&b.key))
    });

    let total_langs = per_lang.len();
    let mut candidates = Vec::new();
    for (rank, g) in groups.iter().enumerate().take(3) {
        let branch_coverage = g.branches.len();
        let agreement = g.langs.len() as f32 / total_langs as f32;
        let subvote = subgroup_score(&g.langs, &per_lang);

        let (form, mut trace) = reconstruct(g.langs.as_slice(), &per_lang, input, cfg);
        if form.is_empty() {
            continue;
        }

        let mut score = if cfg.six_subgroup_vote {
            // subvote is in [0,6]; a form attested in all six subgroups is the
            // strongest possible pan-Slavic consensus.
            0.28 + 0.105 * subvote + 0.12 * agreement
        } else if cfg.branch_balanced {
            0.30 + 0.16 * branch_coverage as f32 + 0.22 * agreement
        } else {
            0.30 + 0.10 + 0.22 * agreement
        };
        // First-ranked group is the chosen consensus; demote the rest.
        score -= 0.12 * rank as f32;
        let score = score.clamp(0.05, 0.97);

        let source = if cfg.branch_balanced {
            CandidateSource::BranchConsensus
        } else {
            CandidateSource::MajorityModernSlavic
        };
        let mut cand = Candidate::new(form, source, round3(score));
        cand.branch_coverage = branch_coverage as u8;

        trace.insert(
            0,
            RuleStep::new(
                "consensus-vote",
                g.langs
                    .iter()
                    .map(|f| f.lang_code.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
                g.key.clone(),
                format!(
                    "Konsensusna forma podpŕta {} větvami ({}), {} od {} językov.",
                    branch_coverage,
                    g.branches
                        .iter()
                        .map(|b| b.label())
                        .collect::<Vec<_>>()
                        .join("+"),
                    g.langs.len(),
                    total_langs
                ),
                Some(DOC_DESIGN),
            ),
        );
        cand.trace = trace;

        // Evidence: each language's primary source form, marked by branch.
        for f in input.forms.iter().filter(|f| f.primary) {
            cand.evidence.push(Evidence {
                lang_code: f.lang_code.clone(),
                lang_name: crate::lang::lang_name(&f.lang_code).to_string(),
                branch: Some(f.branch),
                form: f.norm.original.clone(),
                normalized_form: f.norm.latin.clone(),
                relation: EvidenceRelation::Cognate,
                source_url: f.source_url.clone(),
            });
        }
        if branch_coverage < 2 {
            cand.warnings
                .push("Konsensus opŕt na jednoj větvi; slaby medžuslovjansky dokaz.".to_string());
        }
        candidates.push(cand);
    }

    // Seed alternatives from secondary translations whose cluster isn't already a
    // candidate. Scored strictly below every primary candidate, so top-1 is
    // unchanged — this only fills the remaining alternative slots so the official
    // lemma, if it is a 2nd/3rd translation, still surfaces (top-3/top-5).
    if cfg.synonym_alternatives {
        let min_primary = candidates.iter().map(|c| c.score).fold(1.0_f32, f32::min);
        let mut sec_groups: BTreeMap<String, Vec<&SourceForm>> = BTreeMap::new();
        for f in input.forms.iter().filter(|f| f.modern && !f.primary) {
            let key = ortho::consonant_key(&f.norm.latin);
            if key.is_empty() {
                continue;
            }
            let e = sec_groups.entry(key).or_default();
            if !e.iter().any(|x| x.lang_code == f.lang_code) {
                e.push(f);
            }
        }
        let mut secs: Vec<(String, Vec<&SourceForm>)> = sec_groups.into_iter().collect();
        secs.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
        for (_key, langs) in secs.into_iter().take(3) {
            if candidates.len() >= 6 {
                break;
            }
            let (form, mut trace) = reconstruct(langs.as_slice(), &per_lang, input, cfg);
            if form.is_empty() {
                continue;
            }
            let std = ortho::to_standard(&form.to_lowercase());
            if candidates
                .iter()
                .any(|c| ortho::to_standard(&c.form.to_lowercase()) == std)
            {
                continue;
            }
            let score = (min_primary - 0.02 - 0.01 * candidates.len() as f32).clamp(0.03, 0.5);
            let mut cand =
                Candidate::new(form, CandidateSource::MajorityModernSlavic, round3(score));
            let support = langs.len();
            trace.insert(
                0,
                RuleStep::new(
                    "synonym-alt",
                    langs
                        .iter()
                        .map(|f| f.lang_code.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                    std,
                    format!("Alternativa iz sekundarnyh prěvodov ({support} językov)."),
                    Some(DOC_DESIGN),
                ),
            );
            cand.trace = trace;
            cand.warnings
                .push("Sekundarny prěvod (ne glavny konsensus).".to_string());
            candidates.push(cand);
        }
    }

    candidates
}

/// Build the surface form from the winning group and apply etymological repairs.
fn reconstruct(
    group: &[&SourceForm],
    per_lang: &BTreeMap<&str, &SourceForm>,
    input: &MeaningInput,
    cfg: &ConsensusConfig,
) -> (String, Vec<RuleStep>) {
    let priority = if cfg.adj_longform_rep && input.pos == Pos::Adjective {
        // Adjectives: East/West cite the full long form (-y/-ý) while South cites
        // the short predicative form with a fleeting vowel (dober vs dobry).
        REP_PRIORITY_ADJ
    } else if cfg.prefer_south_representative {
        REP_PRIORITY
    } else {
        REP_PRIORITY_NO_SOUTH_BIAS
    };
    // Pick representative by priority; fall back to first in group.
    let rep = priority
        .iter()
        .find_map(|code| group.iter().find(|f| &f.lang_code == code))
        .or_else(|| group.first())
        .copied();
    let Some(rep) = rep else {
        return (String::new(), Vec::new());
    };

    let mut trace = Vec::new();
    let mut form = rep.norm.latin.clone();
    trace.push(RuleStep::new(
        "pick-representative",
        rep.norm.original.clone(),
        form.clone(),
        format!(
            "Izbrana izvorna forma iz jezyka {} kako najbliža medžuslovjanskomu pravopisu.",
            crate::lang::lang_name(&rep.lang_code)
        ),
        Some(DOC_ORTHO),
    ));

    // Repair 1: undo East-Slavic pleophony if the representative is East.
    if cfg.depleophony && rep.branch == Branch::East {
        if let Some(fixed) = undo_pleophony(&form) {
            if fixed != form {
                trace.push(RuleStep::new(
                    "liquid-metathesis",
                    form.clone(),
                    fixed.clone(),
                    "Vȯzhodnoslovjanske polnoglasje (-oro-/-olo-/-ere-) prěvedeno v medžuslovjansku metatezu.".to_string(),
                    Some("https://steen.free.fr/interslavic/grammar.html"),
                ));
                form = fixed;
            }
        }
    }

    // Repair 2: recover ć/đ from South-Slavic reflex before other edits.
    if cfg.palatal_from_south {
        if let Some((fixed, from)) = palatal_from_south(&form, per_lang) {
            if fixed != form {
                trace.push(RuleStep::new(
                    "tj-dj-palatal",
                    form.clone(),
                    fixed.clone(),
                    format!(
                        "Refleks *tj/*dj (ć/đ) vȯzstanovljeny iz južnoslovjanskej formy ({from})."
                    ),
                    Some(DOC_ORTHO),
                ));
                form = fixed;
            }
        }
    }

    // Repair 3: recover nasal vowels (ę/ų) from Polish.
    if cfg.nasal_from_polish {
        if let Some(pl) = per_lang.get("pl") {
            if let Some(fixed) = nasal_from_polish(&form, &pl.norm.latin) {
                if fixed != form {
                    trace.push(RuleStep::new(
                        "nasal-vowel",
                        form.clone(),
                        fixed.clone(),
                        "Nosovy glasnik (ę/ų) vȯzstanovljeny iz poljskej formy (ę/ą).".to_string(),
                        Some("https://interslavic.fun/learn/phonology/"),
                    ));
                    form = fixed;
                }
            }
        }
    }

    // Repair: recover *y (preserved in Interslavic) from East/West cognates
    // where the South representative merged it to i.
    if cfg.y_recovery {
        if let Some((fixed, donor)) = recover_y(&form, per_lang) {
            if fixed != form {
                trace.push(RuleStep::new(
                    "y-recovery",
                    form.clone(),
                    fixed.clone(),
                    format!(
                        "*y vȯzstanovljeny iz {donor} (jug slil *y→i, medžuslovjansky drži y)."
                    ),
                    Some("https://interslavic.fun/learn/phonology/"),
                ));
                form = fixed;
            }
        }
    }

    // Baseline path: keep a raw ǫ→u fold if nasals were not recovered.
    form = form.replace('ǫ', "u").replace('ř', "r");

    // Repair 4: reconstruct jat from the cross-branch signature.
    if cfg.jat_reconstruction {
        if let Some(fixed) = jat_reconstruction(&form, per_lang) {
            if fixed != form {
                trace.push(RuleStep::new(
                    "jat-reflex",
                    form.clone(),
                    fixed.clone(),
                    "Jať (ě) vȯzstanovljeny iz medžuvětvovogo refleksa (ru e / uk i / pl ie)."
                        .to_string(),
                    Some("https://interslavic.fun/learn/phonology/"),
                ));
                form = fixed;
            }
        }
    }

    // Repair 5: drop a South-Slavic adjective's fleeting vowel (dobar→dobr,
    // besplatan→besplatn) before the -y ending is appended — but only when an
    // East/West cognate confirms the two flanking consonants are adjacent (so
    // real root vowels like zelen- are preserved). Fixes the epenthesis bug
    // without the pl y/i noise the long-form-representative trap imported.
    if cfg.adj_fleeting_drop && input.pos == Pos::Adjective {
        if let Some(fixed) = drop_adj_fleeting(&form, per_lang) {
            if fixed != form {
                trace.push(RuleStep::new(
                    "adj-fleeting-vowel",
                    form.clone(),
                    fixed.clone(),
                    "Beglyj glasnik kratkoj južnoslovjanskoj formy prilagatelnogo padaje (dobar→dobr)."
                        .to_string(),
                    Some("https://interslavic.fun/learn/grammar/nouns/"),
                ));
                form = fixed;
            }
        }
    }

    // POS-aware lemma ending normalization: internationalism table (§5.2) and
    // native endings (§3), both individually gated so the benchmark can
    // attribute their effect.
    if cfg.internationalism || cfg.lemma_endings || cfg.prefix_normalization {
        let (fixed, steps) = crate::morph::normalize_lemma(
            &form,
            input.pos,
            cfg.internationalism,
            cfg.lemma_endings,
            cfg.prefix_normalization,
        );
        form = fixed;
        trace.extend(steps);
    } else {
        form = tidy_ending(&form, input.pos, input.gender);
    }
    (form, trace)
}

/// Undo East-Slavic pleophony: -oroC->-raC, -oloC->-laC(/-lěC), -ereC->-rěC.
/// Conservative: only fires inside a clear C V r/l V C environment.
fn undo_pleophony(word: &str) -> Option<String> {
    let chars: Vec<char> = word.chars().collect();
    let n = chars.len();
    let mut out = String::new();
    let mut i = 0;
    let mut changed = false;
    while i < n {
        // pattern: C (o|e) (r|l) (o|e) C
        if i + 4 < n
            && is_cons(chars[i])
            && matches!(chars[i + 1], 'o' | 'e')
            && matches!(chars[i + 2], 'r' | 'l')
            && matches!(chars[i + 3], 'o' | 'e')
            && is_cons(chars[i + 4])
        {
            out.push(chars[i]);
            out.push(chars[i + 2]);
            // -oro- -> -ra-, -olo- -> -la-, -ere- -> -re-(jat added later)
            let nucleus = if chars[i + 1] == 'e' { 'e' } else { 'a' };
            out.push(nucleus);
            i += 4; // consume up to (but not including) the closing consonant
            changed = true;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    changed.then_some(out)
}

/// If South-Slavic (hr/sr/bs, then sl) shows ć/đ where the representative has
/// another reflex, adopt the palatal.
fn palatal_from_south(
    word: &str,
    per_lang: &BTreeMap<&str, &SourceForm>,
) -> Option<(String, String)> {
    for code in ["hr", "sr", "bs", "sl", "mk"] {
        if let Some(sf) = per_lang.get(code) {
            let s = &sf.norm.latin;
            if s.contains('ć') && !word.contains('ć') {
                // Replace a č/c/t before the front region with ć (coarse).
                if let Some(rep) = replace_first_of(word, &['č', 'c', 't'], 'ć') {
                    return Some((rep, code.to_string()));
                }
            }
            if s.contains('đ') && !word.contains('đ') {
                if let Some(rep) = replace_first_of(word, &['ž', 'z', 'd'], 'đ') {
                    return Some((rep, code.to_string()));
                }
            }
        }
    }
    None
}

fn replace_first_of(word: &str, targets: &[char], to: char) -> Option<String> {
    let mut out = String::with_capacity(word.len());
    let mut done = false;
    for ch in word.chars() {
        if !done && targets.contains(&ch) {
            out.push(to);
            done = true;
        } else {
            out.push(ch);
        }
    }
    done.then_some(out)
}

/// Transfer a nasal vowel from the Polish cognate onto the aligned vowel slot.
fn nasal_from_polish(word: &str, pl: &str) -> Option<String> {
    if !pl.contains('ę') && !pl.contains('ǫ') {
        return None;
    }
    if ortho::consonant_key(word) != ortho::consonant_key(pl) {
        return None;
    }
    let pl_slots = vowel_slots(pl);
    let nasal = pl_slots.iter().find(|(_, v, _)| *v == 'ę' || *v == 'ǫ')?;
    let target = if nasal.1 == 'ę' { 'ę' } else { 'ų' };
    let word_slots = vowel_slots(word);
    let slot = word_slots
        .iter()
        .min_by_key(|(c, _, _)| (*c as isize - nasal.0 as isize).unsigned_abs())?;
    let mut chars: Vec<char> = word.chars().collect();
    if slot.2 < chars.len() {
        chars[slot.2] = target;
        return Some(chars.into_iter().collect());
    }
    None
}

/// Drop the fleeting vowel of a South-Slavic short adjective (final C-V-C where
/// the vowel is fleeting), confirmed by an East/West cognate showing the two
/// consonants adjacent. `dobar`→`dobr` (ru `dobr-yj`), but `zelen` stays (ru
/// `zelen-yj` keeps the vowel).
fn drop_adj_fleeting(form: &str, per_lang: &BTreeMap<&str, &SourceForm>) -> Option<String> {
    let chars: Vec<char> = form.chars().collect();
    let n = chars.len();
    if n < 3 {
        return None;
    }
    // Pattern: ...C1 V C2$  (final consonant, preceded by a vowel, preceded by a
    // consonant). The vowel is a candidate fleeting vowel.
    let (c1, v, c2) = (chars[n - 3], chars[n - 2], chars[n - 1]);
    if !(is_cons(c1) && ortho::is_vowel(v) && is_cons(c2)) {
        return None;
    }
    if !matches!(v, 'a' | 'e' | 'o' | 'ȯ' | 'å') {
        return None;
    }
    // Adjacency evidence: an East/West long form has C1C2 with no vowel between.
    let pair = ortho::ascii_skeleton(&format!("{c1}{c2}"));
    let has_adjacency = ["ru", "pl", "cs", "sk", "uk", "be"].iter().any(|d| {
        per_lang
            .get(*d)
            .map(|f| ortho::ascii_skeleton(&f.norm.latin).contains(&pair))
            .unwrap_or(false)
    });
    if !has_adjacency {
        return None;
    }
    let mut out: String = chars[..n - 2].iter().collect();
    out.push(c2);
    Some(out)
}

/// Reconstruct jat: if East shows `e`/`ě` at a vowel slot where Ukrainian shows
/// `i` and/or Czech/Slovak/South shows a differing front vowel, mark ě.
fn jat_reconstruction(word: &str, per_lang: &BTreeMap<&str, &SourceForm>) -> Option<String> {
    // Signature: Russian has `e` and Ukrainian has `i` in the same slot.
    let ru = per_lang.get("ru")?;
    let uk = per_lang.get("uk")?;
    if ortho::consonant_key(&ru.norm.latin) != ortho::consonant_key(word) {
        return None;
    }
    let ru_slots = vowel_slots(&ru.norm.latin);
    let uk_slots = vowel_slots(&uk.norm.latin);
    for (c, v, _) in &ru_slots {
        if *v != 'e' {
            continue;
        }
        // Ukrainian shows i at the aligned slot => jat.
        if uk_slots
            .iter()
            .any(|(uc, uv, _)| (*uc as isize - *c as isize).abs() <= 1 && *uv == 'i')
        {
            let word_slots = vowel_slots(word);
            if let Some(slot) = word_slots
                .iter()
                .find(|(wc, wv, _)| (*wc as isize - *c as isize).abs() <= 1 && *wv == 'e')
            {
                let mut chars: Vec<char> = word.chars().collect();
                chars[slot.2] = 'ě';
                return Some(chars.into_iter().collect());
            }
        }
    }
    None
}

/// Recover *y where a South-Slavic representative has i but an East/West cognate
/// (Russian/Polish/Czech, which preserve *y) has y at the aligned slot.
fn recover_y(word: &str, per_lang: &BTreeMap<&str, &SourceForm>) -> Option<(String, String)> {
    if !word.contains('i') {
        return None;
    }
    for donor in ["ru", "pl", "cs", "sk", "uk", "be"] {
        let Some(sf) = per_lang.get(donor) else {
            continue;
        };
        if !sf.norm.latin.contains('y') {
            continue;
        }
        if ortho::consonant_key(&sf.norm.latin) != ortho::consonant_key(word) {
            continue;
        }
        let donor_slots = vowel_slots(&sf.norm.latin);
        let word_slots = vowel_slots(word);
        let mut chars: Vec<char> = word.chars().collect();
        let mut changed = false;
        for (dc, dv, _) in &donor_slots {
            if *dv != 'y' {
                continue;
            }
            if let Some((_, _, idx)) = word_slots
                .iter()
                .find(|(wc, wv, _)| *wv == 'i' && (*wc as isize - *dc as isize).abs() <= 1)
            {
                if *idx < chars.len() && chars[*idx] == 'i' {
                    chars[*idx] = 'y';
                    changed = true;
                }
            }
        }
        if changed {
            return Some((chars.into_iter().collect(), donor.to_string()));
        }
    }
    None
}

/// (consonants-before, vowel-char, char-index) for each vowel nucleus.
fn vowel_slots(word: &str) -> Vec<(usize, char, usize)> {
    let mut slots = Vec::new();
    let mut cons = 0usize;
    let mut prev_vowel = false;
    for (idx, ch) in word.chars().enumerate() {
        if is_vowelish(ch) {
            if !prev_vowel {
                slots.push((cons, ch, idx));
            }
            prev_vowel = true;
        } else {
            if ch != 'j' {
                cons += 1;
            }
            prev_vowel = false;
        }
    }
    slots
}

fn is_vowelish(ch: char) -> bool {
    matches!(
        ch,
        'a' | 'e' | 'i' | 'o' | 'u' | 'y' | 'ě' | 'ę' | 'ǫ' | 'ų' | 'å' | 'ȯ'
    )
}

fn is_cons(ch: char) -> bool {
    ch.is_alphabetic() && !is_vowelish(ch)
}

/// Light POS-aware ending normalization for the consensus lemma.
fn tidy_ending(word: &str, pos: Pos, gender: Option<Gender>) -> String {
    let mut w = word.trim().to_string();
    match pos {
        Pos::Verb => {
            // Most languages cite verbs with an infinitive suffix; normalize the
            // common Slavic infinitive endings to Interslavic -ti/-ať->-ati.
            if w.ends_with("ť") {
                w.truncate(w.len() - "ť".len());
                w.push_str("ti");
            } else if w.ends_with("t") && !w.ends_with("ti") {
                // Russian -ть already lost soft sign -> ends in t
                w.push_str("i");
            } else if w.ends_with("ć") {
                w.truncate(w.len() - "ć".len());
                w.push_str("ti");
            }
        }
        Pos::Adjective => {
            // Interslavic hard adjective lemma ends in -y (masc nom sg).
            if w.ends_with("i") && !w.ends_with("ji") {
                w.pop();
                w.push('y');
            }
        }
        Pos::Noun => {
            if gender == Some(Gender::Neuter) && !ends_with_vowel(&w) {
                // leave; neuter usually already ends in -o/-e
            }
        }
        _ => {}
    }
    w
}

fn ends_with_vowel(w: &str) -> bool {
    w.chars().last().map(ortho::is_vowel).unwrap_or(false)
}

fn round3(x: f32) -> f32 {
    (x * 1000.0).round() / 1000.0
}

/// The six dialect subgroups (§4.1). Each gets one vote; RU and PL stand alone
/// so they cannot over-count, and the large BCMS cluster shares a single vote.
const SUBGROUPS: &[&[&str]] = &[
    &["ru"],
    &["uk", "be"],
    &["pl"],
    &["cs", "sk"],
    &["sl", "hr", "sr", "bs"],
    &["bg", "mk"],
];

/// Relative speaker weights, used only as a population tie-break (§4.3).
fn pop_weight(code: &str) -> f32 {
    match code {
        "ru" => 1.0,
        "pl" => 0.44,
        "uk" => 0.42,
        "cs" => 0.10,
        "be" => 0.10,
        "sr" => 0.09,
        "bg" => 0.08,
        "sk" => 0.05,
        "hr" => 0.05,
        "bs" => 0.03,
        "sl" => 0.02,
        "mk" => 0.02,
        _ => 0.0,
    }
}

/// §4.3: Σ over subgroups of (present members agreeing / present members). A
/// form attested across all six subgroups scores 6.0; ½ votes fall out
/// naturally from intra-subgroup splits.
fn subgroup_score(langs: &[&SourceForm], present: &BTreeMap<&str, &SourceForm>) -> f32 {
    let mut total = 0.0;
    for sg in SUBGROUPS {
        let present_members: Vec<&str> = sg
            .iter()
            .copied()
            .filter(|c| present.contains_key(*c))
            .collect();
        if present_members.is_empty() {
            continue;
        }
        let agree = present_members
            .iter()
            .filter(|c| langs.iter().any(|f| &f.lang_code == *c))
            .count();
        total += agree as f32 / present_members.len() as f32;
    }
    total
}

fn population_weight(langs: &[&SourceForm]) -> f32 {
    langs.iter().map(|f| pop_weight(&f.lang_code)).sum()
}

/// Heuristic: does this form look like a Graeco-Latin internationalism? Uses
/// distinctive international morphology (suffixes/roots that native Slavic words
/// almost never carry). Deliberately conservative to avoid boosting native words.
fn is_international_form(latin: &str) -> bool {
    let s = crate::orthography::ascii_skeleton(latin);
    // Distinctive international suffixes.
    const SUF: &[&str] = &[
        "cija", "zija", "sija", "izm", "izem", "izam", "ist", "ura", "tet", "alny", "aln", "ivny",
        "ivn", "ozny", "ozn", "icny", "icn", "acij", "olog", "ograf", "grafij", "logij", "onom",
        "teka", "skop", "metr", "fon", "torij", "ator", "ancij", "encij", "izacij", "izovat",
        "ovati", "irat", "ment",
    ];
    if SUF.iter().any(|suf| s.ends_with(suf)) {
        return true;
    }
    // Distinctive international roots / prefixes.
    const ROOT: &[&str] = &[
        "avto", "auto", "tele", "mikro", "makro", "mega", "foto", "radio", "termo", "geo", "bio",
        "hidro", "hydro", "elektr", "polit", "ekonom", "filozof", "psiho", "demokr", "kompjut",
        "internet", "aero", "kosmo", "video", "audio",
    ];
    ROOT.iter().any(|r| s.contains(r))
}

/// Convenience: build [`SourceForm`]s for every modern Slavic language present in
/// a cell map (used by the evaluator and site builder).
pub fn source_forms_from_cells(
    cells: &std::collections::HashMap<String, String>,
    url_for: impl Fn(&str, &str) -> String,
) -> Vec<SourceForm> {
    let mut forms = Vec::new();
    for li in LANGS.iter() {
        if li.csv_col.is_empty() {
            continue;
        }
        let Some(cell) = cells.get(li.code) else {
            continue;
        };
        let normed = crate::normalize::normalize_cell(li.code, cell);
        let Some(primary) = crate::normalize::primary(&normed) else {
            continue;
        };
        let primary_skel = primary.skeleton.clone();
        // Emit the primary form, then up to 3 distinct secondary variants (marked
        // non-primary) so the official lemma — which the dictionary sometimes
        // lists as a 2nd/3rd translation — can seed an alternative candidate.
        let mut seen = vec![primary_skel.clone()];
        forms.push(SourceForm {
            lang_code: li.code.to_string(),
            branch: li.branch,
            modern: li.modern,
            source_url: url_for(li.code, &primary.original),
            norm: primary.clone(),
            primary: true,
        });
        for nf in &normed {
            if seen.len() >= 4 {
                break;
            }
            if seen.contains(&nf.skeleton) {
                continue;
            }
            seen.push(nf.skeleton.clone());
            forms.push(SourceForm {
                lang_code: li.code.to_string(),
                branch: li.branch,
                modern: li.modern,
                source_url: url_for(li.code, &nf.original),
                norm: nf.clone(),
                primary: false,
            });
        }
    }
    forms
}
