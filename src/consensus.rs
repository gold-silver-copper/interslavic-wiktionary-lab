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
    /// A reflexive verb (most cognates carry a reflexive marker). Interslavic
    /// cites these as `<lemma> sę`, so the generator appends the particle.
    pub reflexive: bool,
}

/// Toggles for each etymological repair, so the benchmark can attribute the
/// accuracy delta of every rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Use Wiktionary's explicit (lang→ancestor) etymology to pick the proto
    /// reconstruction, before the fuzzy descendant+gloss link.
    pub explicit_etymology: bool,
    /// Seed alternative candidates from secondary (non-primary) translations so
    /// the official lemma can appear in top-3/top-5. Never changes top-1.
    pub synonym_alternatives: bool,
    /// Grow Proto-Slavic link coverage by stripping a shared prefix off the
    /// cognates, linking the bare root, and re-attaching the Interslavic prefix.
    pub proto_prefix_stripping: bool,
    /// Repair national adaptation quirks the representative leaks into a loan
    /// stem (Polish y→i, South-Slavic epenthetic vowel/-ac/-a), each sub-repair
    /// gated on the internationalism shape and/or a corroborating cognate.
    pub loan_stem_repair: bool,
    /// Repair verb conjugation-class endings: jat after hushing is a
    /// (-žati/-čati/-šati), statives take -ěti on East/West e-stem evidence.
    pub verb_class_repair: bool,
    /// Repair voicing alternations the representative's orthography leaks:
    /// devoiced prefixes bes-/is- → bez-/iz- and the loan nz → ns, each
    /// corroborated by a cognate with the voiced/Latin spelling.
    pub voicing_repair: bool,
    /// Pick the winning group's representative as the *medoid* — the member
    /// minimizing total folded edit distance to the others (the most central
    /// attested form) — instead of the fixed REP_PRIORITY, avoiding dialectal /
    /// oblique outliers. The rep-eval probe measured +1.09pp exact.
    pub medoid_representative: bool,
    /// Derivational-suffix normalization (root-consistency invariant `DERIV`):
    /// -telj- kept before suffixes (-teljstvo/-teljny), feminine i-stem -sť
    /// (kosť, radosť), deverbal -livy — each categorical in the dictionary.
    pub derivational_suffixes: bool,
    /// Keep the Graeco-Latin -ia-/-io- hiatus in internationalisms (socialny,
    /// entuziazm, sociolog) where Slavic cognates insert the glide -ija-.
    pub loan_hiatus: bool,
    /// Undo the *g→h spirantization a Czech/Slovak/Ukrainian/Belarusian
    /// representative leaks (blahosklonnost → blago-), corroborated per
    /// consonant position by ≥2 g-preserving cognates (ru/pl/South). ISV has
    /// no g→h rule [RULE_SPEC §2].
    pub spirantization_repair: bool,
    /// Stem-class-aware citation endings in the proto engine (issue #76): a
    /// masculine n-stem keeps the archaic nominative in the reconstruction
    /// (*kamy) but the dictionary cites the extended oblique stem (kamenj);
    /// the Wiktionary declension category supplies the class.
    pub proto_stem_class_endings: bool,
    /// Rescue a sub-threshold proto link (confidence in [0.34, 0.42)) when the
    /// cognates' own Wiktionary etymologies name the same deep
    /// (Proto-Balto-Slavic / PIE) ancestor as the candidate reconstruction
    /// (issue #76) — coverage without loosening the confidence gate itself.
    pub proto_link_deep_corroboration: bool,
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
            explicit_etymology: false,
            synonym_alternatives: false,
            proto_prefix_stripping: false,
            loan_stem_repair: false,
            verb_class_repair: false,
            voicing_repair: false,
            medoid_representative: false,
            derivational_suffixes: false,
            loan_hiatus: false,
            spirantization_repair: false,
            proto_stem_class_endings: false,
            proto_link_deep_corroboration: false,
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
            explicit_etymology: true,
            synonym_alternatives: true,
            proto_prefix_stripping: true,
            loan_stem_repair: true,
            verb_class_repair: true,
            voicing_repair: true,
            medoid_representative: true,
            derivational_suffixes: true,
            loan_hiatus: true,
            spirantization_repair: true,
            // Stem-class-aware citation endings (issue #76): kept — categorical
            // in the official CSV and +0.07pp exact (11 fixed / 0 broken,
            // sign-test p = 0.0026), gaining on dev and holdout alike.
            proto_stem_class_endings: true,
            // Rejected by the benchmark (regress accuracy in the consensus path):
            y_recovery: false,
            adj_longform_rep: false,
            // Rejected by the benchmark (issue #76): the deep-corroboration
            // rescue fired on exactly 1 of 16,300 meanings (linked 3,929 →
            // 3,930) and moved nothing (+0.00pp exact/normalized, p = 1.0) —
            // only ~7.7% of lemma etymologies name a PBS/PIE ancestor, so the
            // ≥50%-of-cognates corroboration bar is almost never reachable.
            proto_link_deep_corroboration: false,
        }
    }
}

/// Surface-representative language priority: closest-to-Interslavic first.
/// Slovene/Croatian/Serbian keep *g and have the metathesized liquid
/// diphthongs; Czech/Slovak add clean sibilants; East Slavic is last (pleophony).
const REP_PRIORITY: &[&str] = &[
    // *g-preserving languages first (Interslavic keeps *g as g); Czech/Slovak/
    // Ukrainian/Belarusian, which shifted *g→h, come after so the surface keeps g.
    "sl", "hr", "sr", "sh", "pl", "bg", "mk", "ru", "cs", "sk", "uk", "be",
];
const REP_PRIORITY_NO_SOUTH_BIAS: &[&str] = &[
    "ru", "pl", "cs", "uk", "sk", "sl", "hr", "sr", "sh", "bg", "mk", "be",
];
/// Adjective representative priority: long-form languages first (they keep the
/// full -y/-ý ending and the *y vowel), South last.
const REP_PRIORITY_ADJ: &[&str] = &[
    "ru", "pl", "cs", "sk", "uk", "be", "hr", "sr", "sh", "sl", "bg", "mk",
];

/// Diagnostic-only oracle hints (V7 §2.4). Every field READS THE OFFICIAL ANSWER
/// and must never feed production — this type exists solely so the eval can
/// measure each stage's upper-bound headroom by making it perfect while
/// everything downstream stays real. Only the `--diagnostic-oracle` eval path
/// ever constructs one.
#[derive(Clone, Copy)]
pub struct Oracle<'a> {
    pub official: &'a str,
    /// Force the vote to choose the cluster whose key matches the official key.
    pub cluster: bool,
    /// Pick the group member whose folded form is closest to the official lemma.
    pub representative: bool,
    /// Force the reconstruction whose derived form is closest to official (used
    /// by the pipeline, carried here so one struct threads all three oracles).
    pub proto_link: bool,
    /// Force the vote to a specific cluster key. Unlike `cluster` (which reads the
    /// answer), this key may be computed by a *leakage-free* selection rule, so
    /// the `select-eval` path can measure how much of the oracle-cluster ceiling a
    /// real answer-blind recognizability heuristic recovers.
    pub force_cluster_key: Option<&'a str>,
    /// Pick the winning group's representative by a named *leakage-free* rule
    /// (medoid, modal-skeleton, shortest…) instead of `REP_PRIORITY`. Lets the
    /// `rep-eval` path measure how much of the oracle-representative ceiling an
    /// answer-blind rule recovers. Ignores `official` (blind).
    pub rep_rule: Option<&'a str>,
}

/// Generate ranked Interslavic candidates from modern-Slavic consensus.
pub fn generate(input: &MeaningInput, cfg: &ConsensusConfig) -> Vec<Candidate> {
    generate_oracle(input, cfg, None)
}

/// As [`generate`], but with diagnostic oracle hints (never used in production).
pub fn generate_oracle(
    input: &MeaningInput,
    cfg: &ConsensusConfig,
    oracle: Option<&Oracle>,
) -> Vec<Candidate> {
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

    // Cluster choice override: force a chosen cluster to the front so the
    // representative + repairs run on it and we can measure selection headroom.
    // The key is either the official one (oracle-cluster — reads the answer) or a
    // leakage-free rule-computed key (select-eval).
    let forced_key: Option<String> = oracle.and_then(|o| {
        o.force_cluster_key
            .map(std::string::ToString::to_string)
            .or_else(|| {
                o.cluster
                    .then(|| ortho::consonant_key(&ortho::fold_key(o.official)))
            })
    });
    if let Some(key) = forced_key {
        if let Some(p) = groups.iter().position(|g| g.key == key) {
            let g = groups.remove(p);
            groups.insert(0, g);
        }
    }

    let total_langs = per_lang.len();
    let mut candidates = Vec::new();
    for (rank, g) in groups.iter().enumerate().take(3) {
        let branch_coverage = g.branches.len();
        let agreement = g.langs.len() as f32 / total_langs as f32;
        let subvote = subgroup_score(&g.langs, &per_lang);

        let (form, mut trace) = reconstruct(g.langs.as_slice(), &per_lang, input, cfg, oracle);
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
        // This candidate's own cluster membership (issue #79) — until now it
        // survived only as the comma-joined string in trace[0].
        cand.langs = g.langs.iter().map(|f| f.lang_code.clone()).collect();

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
            let (form, mut trace) = reconstruct(langs.as_slice(), &per_lang, input, cfg, oracle);
            if form.is_empty() {
                continue;
            }
            let std = ortho::fold_key(&form);
            if candidates.iter().any(|c| ortho::fold_key(&c.form) == std) {
                continue;
            }
            let score = (min_primary - 0.02 - 0.01 * candidates.len() as f32).clamp(0.03, 0.5);
            let mut cand =
                Candidate::new(form, CandidateSource::MajorityModernSlavic, round3(score));
            cand.langs = langs.iter().map(|f| f.lang_code.clone()).collect();
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
/// Leakage-free representative-selection rules (diagnostic `rep-eval` only): pick
/// the winning group's representative surface *without reading the official
/// lemma*, so we can measure how much of the oracle-representative ceiling an
/// answer-blind rule recovers.
fn pick_rep_by_rule<'a>(
    rule: &str,
    group: &[&'a SourceForm],
    priority: &[&str],
) -> Option<&'a SourceForm> {
    match rule {
        // Medoid: the member minimizing total folded edit distance to the others —
        // the most "central" attested form, avoiding dialectal / oblique outliers.
        "medoid" => group
            .iter()
            .min_by_key(|f| {
                let a = ortho::fold_key(&f.norm.latin);
                group
                    .iter()
                    .map(|o| ortho::levenshtein(&a, &ortho::fold_key(&o.norm.latin)))
                    .sum::<usize>()
            })
            .copied(),
        // The most common ascii-skeleton in the group, then REP_PRIORITY among the
        // members that carry it (a "typical cognate, best surface" choice).
        "modal-skeleton" => {
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for f in group {
                *counts.entry(f.norm.skeleton.as_str()).or_default() += 1;
            }
            let best = counts
                .into_iter()
                .max_by_key(|(_, n)| *n)
                .map(|(s, _)| s.to_string())?;
            priority
                .iter()
                .find_map(|code| {
                    group
                        .iter()
                        .find(|f| f.norm.skeleton == best && &f.lang_code == code)
                })
                .or_else(|| group.iter().find(|f| f.norm.skeleton == best))
                .copied()
        }
        // The shortest form (nominatives tend to be shorter than oblique cases).
        "shortest" => group
            .iter()
            .min_by_key(|f| f.norm.latin.chars().count())
            .copied(),
        _ => None,
    }
}

fn reconstruct(
    group: &[&SourceForm],
    per_lang: &BTreeMap<&str, &SourceForm>,
    input: &MeaningInput,
    cfg: &ConsensusConfig,
    oracle: Option<&Oracle>,
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
    // Pick representative by priority; fall back to first in group. Under the
    // (diagnostic) representative oracle, instead pick the group member whose
    // folded form is closest to the official lemma — the upper bound of a perfect
    // representative choice while the repairs downstream stay real.
    let by_priority = || {
        priority
            .iter()
            .find_map(|code| group.iter().find(|f| &f.lang_code == code))
            .or_else(|| group.first())
            .copied()
    };
    let rep = if let Some(o) = oracle.filter(|o| o.representative) {
        // Oracle (reads the answer): the member folded-closest to the official
        // lemma — the upper bound of a perfect representative choice.
        let target = ortho::fold_key(o.official);
        group
            .iter()
            .min_by_key(|f| ortho::levenshtein(&ortho::fold_key(&f.norm.latin), &target))
            .copied()
    } else if let Some(rule) = oracle.and_then(|o| o.rep_rule) {
        pick_rep_by_rule(rule, group, priority).or_else(by_priority)
    } else if cfg.medoid_representative {
        // Production: the medoid (most central attested form) — +1.09pp exact.
        pick_rep_by_rule("medoid", group, priority).or_else(by_priority)
    } else {
        by_priority()
    };
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
                        "*y vȯzstanovljeny iz {donor} (jug slil *y→i, medžuslovjansky dŕži y)."
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
            input.gender,
            crate::morph::LemmaRules {
                intl: cfg.internationalism,
                endings: cfg.lemma_endings,
                prefixes: cfg.prefix_normalization,
                deriv: cfg.derivational_suffixes,
                loan_hiatus: cfg.loan_hiatus,
            },
        );
        form = fixed;
        trace.extend(steps);
    } else {
        form = tidy_ending(&form, input.pos, input.gender);
    }

    // Repair 6: national adaptation quirks the representative leaks into a loan
    // stem — Polish y for /i/, the South-Slavic epenthetic vowel (akcenat) and
    // -ac agentive, a masculine loan's -a. Each sub-repair is gated on the
    // internationalism shape and/or a corroborating cognate.
    if cfg.loan_stem_repair {
        let (fixed, steps) = loan_stem_repair(&form, per_lang, input);
        if fixed != form {
            trace.extend(steps);
            form = fixed;
        }
    }

    // Repair 7: verb conjugation-class endings the representative miscites.
    if cfg.verb_class_repair && input.pos == Pos::Verb {
        let (fixed, steps) = verb_class_repair(&form, per_lang);
        if fixed != form {
            trace.extend(steps);
            form = fixed;
        }
    }

    // Repair 8: voicing alternations (devoiced prefixes, loan nz/ns).
    if cfg.voicing_repair {
        let (fixed, steps) = voicing_repair(&form, per_lang, input);
        if fixed != form {
            trace.extend(steps);
            form = fixed;
        }
    }

    // Repair 9: undo the *g→h spirantization a Czech/Slovak/Ukrainian/Belarusian
    // representative leaks (blahosklonnost, kalihrafija). Interslavic has no g→h
    // rule (RULE_SPEC §2), and only those four lects shifted *g→h, so each h is
    // checked per consonant position against the g-preserving cognates
    // (ru/pl/South): ≥2 attesting g at that position restore the g. Genuine h
    // (*x: duh, suh; loans: alkohol) stays — the g-preserving lects write h there
    // too.
    if cfg.spirantization_repair
        && matches!(rep.lang_code.as_str(), "cs" | "sk" | "uk" | "be")
        && form.contains('h')
    {
        if let Some(fixed) = spirantize_h_to_g(&form, per_lang) {
            if fixed != form {
                trace.push(RuleStep::new(
                    "spirantization-hg",
                    form.clone(),
                    fixed.clone(),
                    "Spirantizacija *g→h (češsko/slovačsko/ukrajinsko/běloruska) vrnjena: g-držeči kognaty potvŕdžajut g."
                        .to_string(),
                    Some(DOC_ORTHO),
                ));
                form = fixed;
            }
        }
    }
    (form, trace)
}

/// Languages that did NOT undergo the *g→h spirantization, so their cognates
/// witness the etymological g/h faithfully.
const G_PRESERVING: &[&str] = &["ru", "pl", "sl", "hr", "sr", "bs", "sh", "bg", "mk"];

/// The consonant sequence of a form in folded-ASCII space (vowels and the glide
/// `j` dropped), used to align consonant positions across cognates whose vowels
/// and endings differ.
fn cons_seq(s: &str) -> Vec<char> {
    ortho::ascii_skeleton(s)
        .chars()
        .filter(|c| !"aeiouy".contains(*c) && *c != 'j')
        .collect()
}

/// See Repair 9: replace an `h` with `g` when ≥2 g-preserving cognates attest a
/// `g` at the same consonant position (and g outvotes h there).
fn spirantize_h_to_g(form: &str, per_lang: &BTreeMap<&str, &SourceForm>) -> Option<String> {
    let donors: Vec<Vec<char>> = G_PRESERVING
        .iter()
        .filter_map(|l| per_lang.get(l))
        .map(|f| cons_seq(&f.norm.latin))
        .collect();
    if donors.len() < 2 {
        return None;
    }
    let mut out = String::new();
    let mut k = 0usize; // consonant index in cons_seq space
    let mut changed = false;
    for ch in form.chars() {
        let folded: String = ortho::ascii_skeleton(&ch.to_string());
        let n_cons = folded
            .chars()
            .filter(|c| !"aeiouy".contains(*c) && *c != 'j')
            .count();
        if ch == 'h' {
            let (mut g, mut h) = (0usize, 0usize);
            for d in &donors {
                match d.get(k) {
                    Some('g') => g += 1,
                    Some('h') => h += 1,
                    _ => {}
                }
            }
            if g >= 2 && g > h {
                out.push('g');
                changed = true;
            } else {
                out.push(ch);
            }
        } else {
            out.push(ch);
        }
        k += n_cons;
    }
    changed.then_some(out)
}

/// Voicing repairs (see `ConsensusConfig::voicing_repair`):
///   (a) South-Slavic/orthographic prefix devoicing undone: bes-/is- → bez-/iz-
///       (besplatny→bezplatny, isključiti→izključiti) when a cognate attests
///       the voiced prefix — Interslavic always writes the voiced form. Roots
///       that merely begin bes-/is- (beseda, iskra, istorija) have no voiced
///       cognate and are untouched.
///   (b) Latin loans keep etymological ns (compensare): nz → ns when a cognate
///       spells ns (kompenzovati→kompensovati; benzin has no ns cognate).
fn voicing_repair(
    form: &str,
    per_lang: &BTreeMap<&str, &SourceForm>,
    input: &MeaningInput,
) -> (String, Vec<RuleStep>) {
    let mut w = form.to_string();
    let mut steps = Vec::new();

    for (devoiced, voiced) in [("bes", "bez"), ("is", "iz")] {
        if let Some(rest) = w.strip_prefix(devoiced) {
            let cons_stem = rest.chars().next().is_some_and(is_cons);
            if cons_stem && rest.chars().count() >= 3 {
                let confirmed = per_lang
                    .values()
                    .any(|f| f.norm.latin.to_lowercase().starts_with(voiced));
                if confirmed {
                    let fixed = format!("{voiced}{rest}");
                    steps.push(RuleStep::new(
                        "prefix-voicing",
                        w.clone(),
                        fixed.clone(),
                        format!("Predpona {voiced}- piše sę zvųčno (ne {devoiced}-)."),
                        Some(DOC_ORTHO),
                    ));
                    w = fixed;
                }
            }
        }
    }

    if (input.is_intl_meaning || is_international_form(&w)) && w.contains("nz") {
        let confirmed = per_lang
            .values()
            .any(|f| f.norm.latin.to_lowercase().contains("ns"));
        if confirmed {
            let fixed = w.replace("nz", "ns");
            steps.push(RuleStep::new(
                "loan-ns",
                w.clone(),
                fixed.clone(),
                "Latinsko pozajęto slovo dŕži etimologično ns (compensare → kompensovati)."
                    .to_string(),
                Some(DOC_ORTHO),
            ));
            w = fixed;
        }
    }

    (w, steps)
}

/// Verb conjugation-class repairs (see `ConsensusConfig::verb_class_repair`):
///   (a) *ě after a hushing consonant/j is spelled a — Interslavic never has
///       -žeti/-četi/-šeti/-jeti (držati, slyšati, stojati);
///   (b) the stative/inchoative class is -ěti, not -iti, when an East/West
///       cognate cites the e-stem infinitive (ru каменеть / cs kamenět →
///       kameněti, against South kameniti).
fn verb_class_repair(
    form: &str,
    per_lang: &BTreeMap<&str, &SourceForm>,
) -> (String, Vec<RuleStep>) {
    let mut w = form.to_string();
    let mut steps = Vec::new();

    // (a) hushing/j + ěti/eti → ati (RULE_SPEC: *ě > a after č/ž/š/j).
    for h in ['ž', 'č', 'š', 'j'] {
        for tail in ["ěti", "eti"] {
            let suf = format!("{h}{tail}");
            if let Some(stem) = w.strip_suffix(&suf) {
                if !stem.is_empty() {
                    let fixed = format!("{stem}{h}ati");
                    steps.push(RuleStep::new(
                        "verb-husing-a",
                        w.clone(),
                        fixed.clone(),
                        "Jať po šumnom sųglasniku (ž/č/š/j) piše sę a: držati, slyšati."
                            .to_string(),
                        Some(DOC_ORTHO),
                    ));
                    w = fixed;
                }
            }
        }
    }

    // (b) -iti → -ěti when an East/West cognate cites the stative e-stem: its
    // form ends with the same stem consonant + "et"/"eti"/"ět".
    if let Some(stem) = w.strip_suffix("iti") {
        if let Some(last) = stem.chars().last().filter(|c| is_cons(*c)) {
            // East only: the Russian stative -еть is reliable, while the Czech
            // iterative -ět (navádět vs naváděti/navoditi) is not.
            let confirmed = per_lang.values().any(|f| {
                if f.branch != Branch::East {
                    return false;
                }
                let l = f.norm.latin.to_lowercase();
                ["et", "eti", "ět", "ěti"]
                    .iter()
                    .any(|t| l.ends_with(&format!("{last}{t}")))
            });
            if confirmed {
                let fixed = format!("{stem}ěti");
                steps.push(RuleStep::new(
                    "verb-stative-eti",
                    w.clone(),
                    fixed.clone(),
                    "Stativny/inhoativny glagol na -ěti (ru -еть / cs -ět), ne -iti.".to_string(),
                    Some(DOC_ORTHO),
                ));
                w = fixed;
            }
        }
    }

    (w, steps)
}

/// Loan-stem repairs (see `ConsensusConfig::loan_stem_repair`). Returns the
/// repaired form plus trace steps; identity when nothing fires.
fn loan_stem_repair(
    form: &str,
    per_lang: &BTreeMap<&str, &SourceForm>,
    input: &MeaningInput,
) -> (String, Vec<RuleStep>) {
    let mut w = form.to_string();
    let mut steps = Vec::new();
    let intl = input.is_intl_meaning || is_international_form(&w);

    // (a) Polish/Czech orthographic y inside a loan stem is /i/: Interslavic
    // internationalisms write i (arystokratyczny → aristokratičny). The final
    // letter (adjective -y ending) is never touched.
    if intl && w.chars().rev().skip(1).any(|c| c == 'y') {
        let n = w.chars().count();
        let fixed: String = w
            .chars()
            .enumerate()
            .map(|(i, c)| if c == 'y' && i + 1 < n { 'i' } else { c })
            .collect();
        if fixed != w {
            steps.push(RuleStep::new(
                "loan-y-i",
                w.clone(),
                fixed.clone(),
                "Pravopisne y v internacionalizmu → i (poljska/češska adaptacija).".to_string(),
                Some(DOC_ORTHO),
            ));
            w = fixed;
        }
    }

    if input.pos == Pos::Noun {
        let chars: Vec<char> = w.chars().collect();
        let n = chars.len();
        // (b) South-Slavic epenthetic vowel in a word-final cluster (akcenat,
        // aker, alabaster): drop it when another cognate shows the two
        // consonants adjacent at the word end.
        if n >= 4 {
            let (c1, v, c2) = (chars[n - 3], chars[n - 2], chars[n - 1]);
            if matches!(v, 'a' | 'e' | 'ȯ') && is_cons(c1) && is_cons(c2) {
                let cluster: String = [c1, c2].iter().collect();
                if per_lang
                    .values()
                    .any(|f| f.norm.latin.to_lowercase().ends_with(&cluster))
                {
                    let fixed: String = chars[..n - 2].iter().chain([&c2]).collect();
                    steps.push(RuleStep::new(
                        "loan-epenthesis",
                        w.clone(),
                        fixed.clone(),
                        "Južnoslovjansky epentetičny glasnik v koncovoj grupě padaje (akcenat→akcent).".to_string(),
                        Some(DOC_ORTHO),
                    ));
                    w = fixed;
                }
            }
        }
        // (c) South-Slavic -ac for the agentive/fleeting-vowel suffix: the
        // Interslavic suffix is -ec (amerikanac→amerikanec), corroborated by a
        // cognate ending -ec.
        if w.ends_with("ac")
            && per_lang
                .values()
                .any(|f| f.norm.latin.to_lowercase().ends_with("ec"))
        {
            let fixed = format!("{}ec", &w[..w.len() - 2]);
            steps.push(RuleStep::new(
                "loan-ac-ec",
                w.clone(),
                fixed.clone(),
                "Južnoslovjansky sufiks -ac → medžuslovjansky -ec.".to_string(),
                Some(DOC_ORTHO),
            ));
            w = fixed;
        }
        // (d) Word-final -ia → -ija: Interslavic always writes the glide
        // (awaria→avarija, Dania→Danija).
        if w.ends_with("ia") {
            let fixed = format!("{}ja", &w[..w.len() - 1]);
            steps.push(RuleStep::new(
                "loan-ija",
                w.clone(),
                fixed.clone(),
                "Koncove -ia → -ija (medžuslovjansky vsegda piše j).".to_string(),
                Some(DOC_ORTHO),
            ));
            w = fixed;
        }
        // (e) A masculine loan's final -a (Serbo-Croatian atleta, adresa):
        // Interslavic cites the bare masculine (atlet), corroborated by a
        // consonant-final cognate with the same skeleton.
        if intl && input.gender == Some(Gender::Masculine) && w.ends_with('a') && !w.ends_with("ja")
        {
            let stem = &w[..w.len() - 1];
            let stem_skel = ortho::ascii_skeleton(stem);
            if per_lang
                .values()
                .any(|f| ortho::ascii_skeleton(&f.norm.latin.to_lowercase()) == stem_skel)
            {
                steps.push(RuleStep::new(
                    "loan-masc-a",
                    w.clone(),
                    stem.to_string(),
                    "Mužsky internacionalizm bez koncovogo -a (atleta→atlet).".to_string(),
                    Some(DOC_ORTHO),
                ));
                w = stem.to_string();
            }
        }
        // (f) A feminine loan cited without its -a (banknot, aksiom): restore
        // it when the gender says feminine and a cognate shows the -a form.
        if input.gender == Some(Gender::Feminine) && w.chars().last().is_some_and(is_cons) {
            let with_a = format!("{w}a");
            let skel = ortho::ascii_skeleton(&with_a);
            if per_lang
                .values()
                .any(|f| ortho::ascii_skeleton(&f.norm.latin.to_lowercase()) == skel)
            {
                steps.push(RuleStep::new(
                    "loan-fem-a",
                    w.clone(),
                    with_a.clone(),
                    "Žensky rod: koncove -a vȯzstanovjeno (banknot→banknota).".to_string(),
                    Some(DOC_ORTHO),
                ));
                w = with_a;
            }
        }
        // (g) Diminutive/deverbal suffix -ȯk: the East-Slavic reflex -ok
        // corroborates the fleeting ȯ where West cites -ek (budynek→budynȯk).
        if w.ends_with("ek")
            && per_lang
                .values()
                .any(|f| f.branch == Branch::East && f.norm.latin.to_lowercase().ends_with("ok"))
        {
            let fixed = format!("{}ȯk", &w[..w.len() - 2]);
            steps.push(RuleStep::new(
                "loan-ok-suffix",
                w.clone(),
                fixed.clone(),
                "Sufiks -ȯk (běgly ȯ): vȯzhodnoslovjansky -ok proti zapadnomu -ek.".to_string(),
                Some(DOC_ORTHO),
            ));
            w = fixed;
        }
    }

    // (h) Secondary imperfective -yvati: the East-Slavic -yva- stem corroborates
    // y where South cites -avati (namotavati→namotyvati).
    if input.pos == Pos::Verb
        && w.ends_with("avati")
        && per_lang
            .values()
            .any(|f| f.branch == Branch::East && f.norm.latin.to_lowercase().contains("yva"))
    {
        let fixed = format!("{}yvati", &w[..w.len() - 5]);
        steps.push(RuleStep::new(
            "loan-yvati",
            w.clone(),
            fixed.clone(),
            "Sekundarny imperfektiv -yvati (vȯzhodnoslovjansko -yva-).".to_string(),
            Some(DOC_ORTHO),
        ));
        w = fixed;
    }

    (w, steps)
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
    for code in ["hr", "sr", "bs", "sh", "sl", "mk"] {
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
    let word_slots = vowel_slots(word);
    let slot = word_slots
        .iter()
        .min_by_key(|(c, _, _)| (*c as isize - nasal.0 as isize).unsigned_abs())?;
    // Polish ą (→ǫ) is reliably the BACK nasal ų. Polish ę, however, descends from
    // *ę AND *ǫ (ręka<*rǫka, gęś<*gǫsь), so its quality is ambiguous — decide
    // front/back from the representative's own reflex vowel at that slot: a back
    // reflex (u/o/…) means the ISV nasal is back (ų), else front (ę) (B6).
    let target = if nasal.1 == 'ǫ' || matches!(slot.1, 'u' | 'o' | 'ų' | 'ǫ' | 'å' | 'ȯ' | 'y')
    {
        'ų'
    } else {
        'ę'
    };
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
            .is_some_and(|f| ortho::ascii_skeleton(&f.norm.latin).contains(&pair))
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
                w.push('i');
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
    w.chars().last().is_some_and(ortho::is_vowel)
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
    &["sl", "hr", "sr", "bs", "sh"],
    &["bg", "mk"],
];

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

// The §4.3 tie-break weights themselves live in the lang.rs registry
// (crate::lang::pop_weight) since issue #79; behavior is identical.
fn population_weight(langs: &[&SourceForm]) -> f32 {
    langs
        .iter()
        .map(|f| crate::lang::pop_weight(&f.lang_code))
        .sum()
}

/// Heuristic: does this form look like a Graeco-Latin internationalism? Uses
/// distinctive international morphology (suffixes/roots that native Slavic words
/// almost never carry). Deliberately conservative to avoid boosting native words.
pub fn is_international_form(latin: &str) -> bool {
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

/// Only lemmas should shape a generated word. Bulgarian and Macedonian have no
/// infinitive, so their dictionaries cite verbs by the present tense
/// (`абдикирам`, `јаде`); that present-tense shape misleads the infinitive-based
/// Interslavic lemma, so drop those two languages from verb meanings (they still
/// contribute to non-verb meanings). Measured: +0.07pp exact, denominator
/// unchanged (verbs keep ≥2 infinitive-citing cognates).
pub fn lemma_forms(forms: Vec<SourceForm>, pos: Pos) -> Vec<SourceForm> {
    if pos == Pos::Verb {
        forms
            .into_iter()
            .filter(|f| f.lang_code != "bg" && f.lang_code != "mk")
            .collect()
    } else {
        forms
    }
}

/// Detect a reflexive verb and strip the reflexive marker off its cognates, so
/// the consensus votes on the clean stem. Interslavic writes reflexives as
/// `<lemma> sę`, added later (see the pipeline). Returns `(forms, reflexive)`.
/// Only fires for verbs and only when ≥2 cognates (and at least half) carry a
/// marker, so a stem that merely ends in `-sa` isn't misread as reflexive.
pub fn strip_reflexive(mut forms: Vec<SourceForm>, pos: Pos) -> (Vec<SourceForm>, bool) {
    if pos != Pos::Verb {
        return (forms, false);
    }
    let primary: Vec<usize> = forms
        .iter()
        .enumerate()
        .filter(|(_, f)| f.modern && f.primary)
        .map(|(i, _)| i)
        .collect();
    let marked = primary
        .iter()
        .filter(|&&i| reflexive_stem(&forms[i].norm.latin).is_some())
        .count();
    if marked < 2 || marked * 2 < primary.len() {
        return (forms, false);
    }
    for f in &mut forms {
        if let Some(stem) = reflexive_stem(&f.norm.latin) {
            f.norm.latin = stem.clone();
            f.norm.skeleton = crate::orthography::ascii_skeleton(&stem);
        }
    }
    forms.retain(|f| !f.norm.skeleton.is_empty());
    (forms, true)
}

/// The stem of a reflexive cognate form, or `None` if it carries no marker.
fn reflexive_stem(latin: &str) -> Option<String> {
    // Space-separated particle (West/South: się, se, sa, sę, sobě). Slovak/Czech
    // cite " sa"/" se" — omitting " sa" left Slovak reflexives unstripped, then the
    // pipeline appended a second particle → "bat sa sę".
    for p in [" się", " sie", " sę", " se", " sa", " sobě", " sobe"] {
        if let Some(h) = latin.strip_suffix(p) {
            let h = h.trim_end();
            if h.chars().count() >= 3 {
                return Some(h.to_string());
            }
        }
    }
    // Glued East-Slavic reflexive infinitive: the reliable marker is the
    // reflexive `-t(i)-sja`/`-t(i)-sa` (ru `-ться`→tsa, uk `-тися`→tysja) — the
    // `-t-` distinguishes it from a stem that merely ends in `-sa`. Bare `-sa`
    // and `-сь`→s are too ambiguous to strip.
    for suf in ["tsja", "tsa", "tisja", "tisa", "tysja", "tysa", "sję", "sę"] {
        if let Some(h) = latin.strip_suffix(suf) {
            // Restore the stem-final -t- that the reflexive infinitive carries.
            let stem = if suf.starts_with('t') {
                format!("{h}t")
            } else {
                h.to_string()
            };
            if stem.chars().count() >= 4 {
                return Some(stem);
            }
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflexive_marker_detection() {
        // Reflexive infinitive -tsa / space particle → stem recovered.
        assert_eq!(reflexive_stem("žalovatsa").as_deref(), Some("žalovat"));
        assert_eq!(reflexive_stem("učiti sę").as_deref(), Some("učiti"));
        // Bare -sa (no -t-) and short stems are NOT treated as reflexive.
        assert!(reflexive_stem("kolbasa").is_none());
        assert!(reflexive_stem("nesę").is_none());
    }

    fn form(lang: &str, branch: Branch, latin: &str) -> SourceForm {
        SourceForm {
            lang_code: lang.to_string(),
            branch,
            modern: true,
            norm: NormForm {
                original: latin.to_string(),
                latin: latin.to_string(),
                skeleton: ortho::ascii_skeleton(latin),
                flagged: false,
            },
            source_url: String::new(),
            primary: true,
        }
    }

    fn meaning(pos: Pos, gender: Option<Gender>, intl: bool) -> MeaningInput {
        MeaningInput {
            pos,
            gender,
            gloss: String::new(),
            forms: Vec::new(),
            is_intl_meaning: intl,
            reflexive: false,
        }
    }

    #[test]
    fn spirantization_restores_g_from_g_preserving_cognates() {
        let ru = form("ru", Branch::East, "blagosklonnostj");
        let pl = form("pl", Branch::West, "blagosklonnošč");
        let hr = form("hr", Branch::South, "blagosklonost");
        let mut per_lang: BTreeMap<&str, &SourceForm> = BTreeMap::new();
        per_lang.insert("ru", &ru);
        per_lang.insert("pl", &pl);
        per_lang.insert("hr", &hr);
        // Czech/Slovak h with ≥2 g-preserving witnesses at the same consonant
        // position → g restored.
        assert_eq!(
            spirantize_h_to_g("blahosklonnosť", &per_lang).as_deref(),
            Some("blagosklonnosť")
        );

        // Genuine *x/loan h stays: the g-preserving cognates write h there too.
        let ru2 = form("ru", Branch::East, "alkogolj");
        let pl2 = form("pl", Branch::West, "alkohol");
        let hr2 = form("hr", Branch::South, "alkohol");
        let mut per_lang2: BTreeMap<&str, &SourceForm> = BTreeMap::new();
        per_lang2.insert("ru", &ru2);
        per_lang2.insert("pl", &pl2);
        per_lang2.insert("hr", &hr2);
        // ru g is outvoted 1-2 at that position → no change (returns None).
        assert_eq!(spirantize_h_to_g("alkohol", &per_lang2), None);
    }

    #[test]
    fn medoid_representative_picks_the_central_form() {
        // The medoid is the member minimizing total folded edit distance to the
        // others, so a dialectal/oblique outlier does not become the representative.
        let ru = form("ru", Branch::East, "voda");
        let pl = form("pl", Branch::West, "voda");
        let sl = form("sl", Branch::South, "vodica"); // outlier (diminutive)
        let group = vec![&ru, &pl, &sl];
        let rep = pick_rep_by_rule("medoid", &group, REP_PRIORITY).unwrap();
        assert_eq!(rep.norm.latin, "voda", "medoid should avoid the outlier");
        // An unknown rule yields None (falls back to REP_PRIORITY in reconstruct).
        assert!(pick_rep_by_rule("nonesuch", &group, REP_PRIORITY).is_none());
    }

    #[test]
    fn loan_stem_repairs_national_quirks() {
        let ru = form("ru", Branch::East, "akcent");
        let per: BTreeMap<&str, &SourceForm> = [("ru", &ru)].into_iter().collect();

        // (a) Polish orthographic y inside an internationalism → i; final -y kept.
        let inp = meaning(Pos::Adjective, None, true);
        assert_eq!(
            loan_stem_repair("arystokratyčny", &per, &inp).0,
            "aristokratičny"
        );
        // Not an internationalism → untouched (native *y must survive).
        let native = meaning(Pos::Adjective, None, false);
        assert_eq!(loan_stem_repair("ryba", &per, &native).0, "ryba");

        // (b) Epenthetic vowel dropped only with a corroborating cognate.
        let noun = meaning(Pos::Noun, None, true);
        assert_eq!(loan_stem_repair("akcenat", &per, &noun).0, "akcent");
        let uncorroborated: BTreeMap<&str, &SourceForm> = BTreeMap::new();
        assert_eq!(
            loan_stem_repair("akcenat", &uncorroborated, &noun).0,
            "akcenat"
        );

        // (c) -ac → -ec with a corroborating -ec cognate.
        let ru_kupec = form("ru", Branch::East, "kupec");
        let per_kupec: BTreeMap<&str, &SourceForm> = [("ru", &ru_kupec)].into_iter().collect();
        let native_noun = meaning(Pos::Noun, None, false);
        assert_eq!(
            loan_stem_repair("kupac", &per_kupec, &native_noun).0,
            "kupec"
        );

        // (d) Final -ia always takes the glide.
        assert_eq!(loan_stem_repair("avaria", &per, &native_noun).0, "avarija");

        // (e) Masculine loan drops Serbo-Croatian -a when a bare cognate exists.
        let ru_atlet = form("ru", Branch::East, "atlet");
        let per_atlet: BTreeMap<&str, &SourceForm> = [("ru", &ru_atlet)].into_iter().collect();
        let masc = meaning(Pos::Noun, Some(Gender::Masculine), true);
        assert_eq!(loan_stem_repair("atleta", &per_atlet, &masc).0, "atlet");
        // Feminine keeps -a.
        let fem = meaning(Pos::Noun, Some(Gender::Feminine), true);
        assert_eq!(loan_stem_repair("atleta", &per_atlet, &fem).0, "atleta");

        // (f) Feminine loan cited bare gets its -a back from a cognate.
        let sh_banknota = form("sr", Branch::South, "banknota");
        let per_bank: BTreeMap<&str, &SourceForm> = [("sr", &sh_banknota)].into_iter().collect();
        assert_eq!(loan_stem_repair("banknot", &per_bank, &fem).0, "banknota");
        // No corroborating -a cognate → untouched.
        let ru_akcent = form("ru", Branch::East, "akcent");
        let per_akc: BTreeMap<&str, &SourceForm> = [("ru", &ru_akcent)].into_iter().collect();
        assert_eq!(loan_stem_repair("akcent", &per_akc, &fem).0, "akcent");

        // (g) -ek → -ȯk on the East-Slavic -ok reflex.
        let uk_budynok = form("uk", Branch::East, "budynok");
        let per_bud: BTreeMap<&str, &SourceForm> = [("uk", &uk_budynok)].into_iter().collect();
        let noun_native = meaning(Pos::Noun, Some(Gender::Masculine), false);
        assert_eq!(
            loan_stem_repair("budynek", &per_bud, &noun_native).0,
            "budynȯk"
        );

        // (h) Secondary imperfective -yvati from the East-Slavic -yva- stem.
        let ru_yva = form("ru", Branch::East, "namatyvat");
        let per_yva: BTreeMap<&str, &SourceForm> = [("ru", &ru_yva)].into_iter().collect();
        let verb = meaning(Pos::Verb, None, false);
        assert_eq!(
            loan_stem_repair("namotavati", &per_yva, &verb).0,
            "namotyvati"
        );
        // A West-only -avati verb stays -avati.
        let cs_davati = form("cs", Branch::West, "davati");
        let per_dav: BTreeMap<&str, &SourceForm> = [("cs", &cs_davati)].into_iter().collect();
        assert_eq!(loan_stem_repair("davati", &per_dav, &verb).0, "davati");
    }

    #[test]
    fn verb_class_repairs() {
        // (a) jat after a hushing consonant is spelled a — unconditional law.
        let empty: BTreeMap<&str, &SourceForm> = BTreeMap::new();
        assert_eq!(verb_class_repair("držeti", &empty).0, "držati");
        assert_eq!(verb_class_repair("slyšeti", &empty).0, "slyšati");
        // Non-hushing stems are untouched.
        assert_eq!(verb_class_repair("viděti", &empty).0, "viděti");

        // (b) stative -ěti on the East e-stem infinitive (ru каменеть).
        let ru = form("ru", Branch::East, "kamenet");
        let per: BTreeMap<&str, &SourceForm> = [("ru", &ru)].into_iter().collect();
        assert_eq!(verb_class_repair("kameniti", &per).0, "kameněti");
        // The Czech iterative -ět does NOT confirm (navádět vs navoditi).
        let cs = form("cs", Branch::West, "navádět");
        let per_cs: BTreeMap<&str, &SourceForm> = [("cs", &cs)].into_iter().collect();
        assert_eq!(verb_class_repair("navoditi", &per_cs).0, "navoditi");
    }

    #[test]
    fn voicing_repairs() {
        let intl = meaning(Pos::Noun, None, true);
        let native = meaning(Pos::Adjective, None, false);

        // (a) Devoiced prefix undone on a voiced-cognate witness.
        let pl = form("pl", Branch::West, "bezplatny");
        let per: BTreeMap<&str, &SourceForm> = [("pl", &pl)].into_iter().collect();
        assert_eq!(voicing_repair("besplatny", &per, &native).0, "bezplatny");
        // A root that merely begins is-/bes- has no voiced cognate: untouched.
        let ru = form("ru", Branch::East, "istorija");
        let per_ist: BTreeMap<&str, &SourceForm> = [("ru", &ru)].into_iter().collect();
        assert_eq!(voicing_repair("istorija", &per_ist, &intl).0, "istorija");

        // (b) Loan nz → ns only with an ns cognate (benzin stays).
        let ru_ns = form("ru", Branch::East, "kompensirovat");
        let per_ns: BTreeMap<&str, &SourceForm> = [("ru", &ru_ns)].into_iter().collect();
        let verb = meaning(Pos::Verb, None, true);
        assert_eq!(
            voicing_repair("kompenzovati", &per_ns, &verb).0,
            "kompensovati"
        );
        let pl_nz = form("pl", Branch::West, "benzyna");
        let per_nz: BTreeMap<&str, &SourceForm> = [("pl", &pl_nz)].into_iter().collect();
        assert_eq!(voicing_repair("benzin", &per_nz, &intl).0, "benzin");
    }
}
