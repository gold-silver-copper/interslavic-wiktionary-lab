//! Reproducible benchmark of the candidate generator against the official
//! Interslavic dictionary.
//!
//! For every benchmarkable official entry we hand the generator the modern Slavic
//! cognates plus the meaning-level metadata the official row carries — POS, gender,
//! and the `genesis` internationalism flag — but **never the `isv` answer form**.
//! (So the reconstruction is leakage-free w.r.t. the form; it does use official
//! POS/gender/genesis, as a real generator would from a headword's part of speech.)
//! We run an *ablation ladder* — baseline, then each linguistic rule switched on
//! cumulatively — so the measured effect of every change is attributable. Rules
//! are kept on the **primary metric (exact top-1)**; a kept rule may still nudge a
//! secondary metric (e.g. `+nasals` is ~flat on exact, −0.1pp on normalized). All
//! metrics and the regression/improvement diffs are written under `target/eval/`.

use crate::consensus::{self, ConsensusConfig, MeaningInput, SourceForm};
use crate::model::{Candidate, CandidateSource, Confidence, Pos, RuleStep};
use crate::official::{self, OfficialEntry};
use crate::orthography as ortho;
use anyhow::Result;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// One rung of the ablation ladder.
struct Rung {
    name: &'static str,
    description: &'static str,
    cfg: ConsensusConfig,
}

/// The cumulative ladder of *kept* rules — each improved the primary metric
/// (exact top-1) without regressing it — ending exactly at
/// [`ConsensusConfig::production`] (enforced by a test).
fn kept_ladder() -> Vec<Rung> {
    let base = ConsensusConfig::baseline();
    let mut branch = base;
    branch.branch_balanced = true;
    branch.prefer_south_representative = true;
    let mut six = branch;
    six.six_subgroup_vote = true;
    let mut endings = six;
    endings.lemma_endings = true;
    let mut intl = endings;
    intl.internationalism = true;
    let mut prefix = intl;
    prefix.prefix_normalization = true;
    let mut deple = prefix;
    deple.depleophony = true;
    let mut nasal = deple;
    nasal.nasal_from_polish = true;
    let mut proto = nasal;
    proto.proto_derived_form = true;
    let mut intlpref = proto;
    intlpref.internationalism_preference = true;
    let mut adjfleet = intlpref;
    adjfleet.adj_fleeting_drop = true;
    let mut synalt = adjfleet;
    synalt.synonym_alternatives = true;
    let mut prefixstrip = synalt;
    prefixstrip.proto_prefix_stripping = true;
    let mut loanrepair = prefixstrip;
    loanrepair.loan_stem_repair = true;
    let mut verbclass = loanrepair;
    verbclass.verb_class_repair = true;
    let mut voicing = verbclass;
    voicing.voicing_repair = true;
    let mut explicit = voicing;
    explicit.explicit_etymology = true;
    let mut medoid = explicit;
    medoid.medoid_representative = true;
    let mut deriv = medoid;
    deriv.derivational_suffixes = true;
    let mut hiatus = deriv;
    hiatus.loan_hiatus = true;
    let mut spirant = hiatus;
    spirant.spirantization_repair = true;
    let mut stemclass = spirant;
    stemclass.proto_stem_class_endings = true;

    vec![
        Rung { name: "baseline", description: "Transliterate the first available form; no branch balancing, no repairs (the original prototype behavior).", cfg: base },
        Rung { name: "+branch-consensus", description: "Branch-balanced skeleton vote + South-Slavic representative.", cfg: branch },
        Rung { name: "+six-subgroup", description: "Six dialect-subgroup vote with population tie-break (§4.1).", cfg: six },
        Rung { name: "+lemma-endings", description: "Native POS lemma endings: noun nom.sg, adj -y/-i, verb -ti (§3).", cfg: endings },
        Rung { name: "+internationalism", description: "Internationalism ending table: -izm/-cija/-ičny/-alny/-ovati (§5.2).", cfg: intl },
        Rung { name: "+prefixes", description: "Normalize verbal/nominal prefixes råz-/prěd- (§2).", cfg: prefix },
        Rung { name: "+depleophony", description: "Undo East-Slavic pleophony / liquid metathesis (§2).", cfg: deple },
        Rung { name: "+nasals", description: "Recover ę/ų nasal vowels from Polish (§2 Phase C).", cfg: nasal },
        Rung { name: "+proto-derived", description: "Two-stage §4.4: consensus picks the root, the Proto-Slavic rule engine supplies the flavored form (ě/ć/đ/å/ȯ/y) via a leakage-free descendant+gloss link. Requires the proto cache.", cfg: proto },
        Rung { name: "+intl-preference", description: "Prefer the internationalism cluster over native synonyms (ISV design criteria favor international roots for modern vocabulary): aeroplan over samolot.", cfg: intlpref },
        Rung { name: "+adj-fleeting", description: "Drop a South-Slavic adjective's fleeting vowel before -y, gated on East/West consonant adjacency (dobar→dobry, zelen stays).", cfg: adjfleet },
        Rung { name: "+synonym-alts", description: "Seed alternatives from secondary translations (below every primary candidate) so the official lemma surfaces in top-3/top-5 when it is a 2nd/3rd translation.", cfg: synalt },
        Rung { name: "+prefix-strip", description: "Grow proto-link coverage: strip a shared prefix off the cognates, link the bare root, re-attach the Interslavic prefix (råzprostirati from *prostirati).", cfg: prefixstrip },
        Rung { name: "+loan-stem-repair", description: "Repair national adaptation quirks the representative leaks into a loan stem: Polish y→i, South-Slavic epenthetic vowel (akcenat→akcent), -ac→-ec, final -ia→-ija, masculine -a drop — each corroborated by a cognate or the internationalism gate.", cfg: loanrepair },
        Rung { name: "+verb-class", description: "Verb conjugation classes: jat after hushing spelled a (drzati, slysati), statives -eti on East/West e-stem evidence (kameneti).", cfg: verbclass },
        Rung { name: "+voicing", description: "Voicing correspondences: devoiced prefixes bes-/is- -> bez-/iz- and loan nz -> ns, each corroborated by a cognate with the voiced/Latin spelling.", cfg: voicing },
        Rung { name: "+explicit-etymology", description: "Use Wiktionary's stated (lang→ancestor) etymology to pick the Proto-Slavic reconstruction directly, before the fuzzy descendant+gloss link — the precise ancestor the corpus site uses.", cfg: explicit },
        Rung { name: "+medoid-rep", description: "Pick the winning cluster's representative as the medoid — the member minimizing total folded edit distance to the others (the most central attested form) — instead of the fixed REP_PRIORITY, avoiding dialectal/oblique outliers. Measured by rep-eval (+1.09pp exact), the biggest recoverable slice of the +3.7pp oracle-representative ceiling.", cfg: medoid },
        Rung { name: "+deriv-suffixes", description: "Derivational-suffix normalization (root-consistency invariant [DERIV]), each categorical in the dictionary: -telj- kept before suffixes (53 -teljstvo/-teljny/-teljsky vs 0 hard), feminine i-stem soft -sť (516 vs 0), deverbal -livy (152 vs 0 -ljivy).", cfg: deriv },
        Rung { name: "+loan-hiatus", description: "Keep the Graeco-Latin -ia-/-io- hiatus in internationalisms (socialny, entuziazm, sociolog) where the Slavic cognates' -ija- glide is a national adaptation: 24 -ial- vs 0 -ijal- in the dictionary, 139 midword -io- vs 1 -ijo-.", cfg: hiatus },
        Rung { name: "+spirantization", description: "Undo the *g→h spirantization a Czech/Slovak/Ukrainian/Belarusian representative leaks into the surface (blahosklonnost→blago-), corroborated per consonant position by ≥2 g-preserving cognates (ru/pl/South). ISV has no g→h rule (RULE_SPEC §2); genuine *x/loan h stays because the g-preserving lects write h there too.", cfg: spirant },
        Rung { name: "+stem-class-endings (production)", description: "Stem-class-aware citation endings (issue #76): a masculine n-stem's archaic nominative *-y survives the sound rules (*kamy→kamy) but the dictionary cites the extended oblique stem (kamenj, jęčmenj, plåmenj) — categorical in the official CSV. The Wiktionary declension category on the linked reconstruction supplies the class; link scoring stays stem_class-blind.", cfg: stemclass },
    ]
}

/// Load the Proto-Slavic cache if it exists (else the proto-derived rung is a
/// no-op that equals the +nasals config). A cache that exists but fails to
/// load (corrupt, or refused by its schema stamp) aborts with the loader's
/// message instead of silently dropping the proto engine from every rung —
/// that would read as a benchmark regression with no visible cause.
fn load_proto_index() -> Option<crate::dump::ProtoIndex> {
    let path = Path::new(crate::DEFAULT_PROTO_CACHE);
    // Note (rejected experiment): augmenting the explicit-etymology map with
    // Proto-Slavic ancestors parsed from the native RU/PL/CS Wiktionary prose
    // (3,369 extra links) measured **−0.10pp exact** — the English etymology +
    // fuzzy linker already saturate the *derivable* coverage, and the extra
    // explicit links (which run first) override correct fuzzy links on meanings
    // that were already right or aren't improvable. Coverage is not the
    // bottleneck; the remaining error is editorial/evidence-gap (see the
    // cluster-selection measurement). Left out.
    crate::dump::load_optional(path, crate::dump::ProtoIndex::load)
        .unwrap_or_else(|e| panic!("{}: {e:#}", path.display()))
}

/// Rules that were tried and *rejected*: each is the production config plus one
/// experimental rule, so its (negative) delta is measured in isolation.
fn rejected_experiments() -> Vec<Rung> {
    let prod = ConsensusConfig::production();
    let mut palatal = prod;
    palatal.palatal_from_south = true;
    let mut jat = prod;
    jat.jat_reconstruction = true;
    let mut adjrep = prod;
    adjrep.adj_longform_rep = true;
    let mut yrec = prod;
    yrec.y_recovery = true;
    let mut deepcorr = prod;
    deepcorr.proto_link_deep_corroboration = true;
    vec![
        Rung { name: "prod+palatals", description: "Recover ć/đ (*tj/*dj) from South Slavic — modern reflexes are too noisy; derive from Proto-Slavic instead.", cfg: palatal },
        Rung { name: "prod+jat", description: "Reconstruct jat ě from the cross-branch reflex — unreliable from modern reflexes.", cfg: jat },
        Rung { name: "prod+adj-longform", description: "Long-form (ru/pl/cs) adjective representative — East/West orthographic quirks outweigh the fleeting-vowel fix.", cfg: adjrep },
        Rung { name: "prod+y-recovery", description: "Recover *y from East/West where South merged *y→i — too aggressive, flips correct i→y.", cfg: yrec },
        Rung { name: "prod+link-corroboration", description: "Deep-ancestor corroboration rescue in the proto linker (issue #76): accept a sub-threshold link (confidence in [0.34, 0.42), floored to the gate) when ≥ half of the primary cognates' own Wiktionary etymologies name the candidate's Proto-Balto-Slavic/PIE ancestor. Measured +0.00pp exact/normalized: the rescue fires on exactly 1 of 16,300 meanings — only ~7.7% of lemma etymologies name a deep ancestor, so the corroboration bar is almost never reachable. Kept out of production.", cfg: deepcorr },
    ]
}

#[derive(Clone)]
struct EntryResult {
    id: String,
    isv: String,
    gloss: String,
    pos: Pos,
    predicted: String,
    exact: bool,
    normalized: bool,
    norm_edit: f32,
    branch_cov: usize,
    confidence: Option<Confidence>,
    score: f32,
    n_langs: usize,
}

#[derive(Default, Clone)]
struct Bucket {
    n: usize,
    exact: usize,
    normalized: usize,
}
impl Bucket {
    fn add(&mut self, r: &EntryResult) {
        self.n += 1;
        self.exact += r.exact as usize;
        self.normalized += r.normalized as usize;
    }
    fn rate(hits: usize, n: usize) -> f32 {
        if n == 0 {
            0.0
        } else {
            hits as f32 / n as f32
        }
    }
}

struct RunMetrics {
    name: String,
    description: String,
    n: usize,
    exact: usize,
    normalized: usize,
    skeleton: usize,
    top3: usize,
    top5: usize,
    sum_norm_edit: f32,
    by_pos: BTreeMap<&'static str, Bucket>,
    by_branch: [Bucket; 4],
    by_conf: BTreeMap<&'static str, Bucket>,
    results: Vec<EntryResult>,
}

/// Benchmark the **site's** generation path (`corpus::generate_set`) against the
/// official dictionary, leakage-free. For each scorable entry we build a cognate
/// set from the modern cognates + the leakage-free Proto-Slavic link (or the
/// internationalism flag) — exactly what the corpus site does — run
/// `generate_set`, and score its headword. This gives the site path its own
/// accuracy number, distinct from the consensus pipeline's headline.
pub fn run_corpus_eval(official_path: &Path) -> Result<()> {
    use crate::corpus::{self, CognateSet};
    use crate::dump::LemmaEntry;
    let entries = official::load(official_path)?;
    let proto = load_proto_index();
    if proto.is_none() {
        println!("(no proto cache — inherited words can't be derived; run extract-proto)");
    }
    let cfg = ConsensusConfig::production();
    let (mut n, mut exact, mut norm, mut inh, mut bor) = (0usize, 0usize, 0usize, 0usize, 0usize);

    for entry in &entries {
        let input = build_input(entry);
        if input.forms.iter().filter(|f| f.modern).count() < 2 || entry.isv.trim().is_empty() {
            continue;
        }
        if entry.isv.contains(' ') || entry.isv.contains('#') {
            continue;
        }
        let borrowed = entry.genesis.trim() == "I";
        // Leakage-free ancestor: the descendant/gloss link, never the isv form.
        let proto_word = if borrowed {
            String::new()
        } else {
            match proto.as_ref().and_then(|idx| {
                crate::proto_link::link(idx, &input, true, cfg.proto_link_deep_corroboration)
            }) {
                Some(l) => format!("*{}", l.entry.word),
                None => continue, // no ancestor and not international: site skips it
            }
        };
        let members: Vec<LemmaEntry> = input
            .forms
            .iter()
            .filter(|f| f.modern && f.primary)
            .map(|f| LemmaEntry {
                lang: f.lang_code.clone(),
                word: f.norm.original.clone(),
                pos: entry.pos.code().to_string(),
                gloss: entry.english.clone(),
                proto: proto_word.clone(),
                etymon: if borrowed {
                    "la loan".into()
                } else {
                    proto_word.clone()
                },
                etymology: Vec::new(),
                categories: Vec::new(),
                topics: Vec::new(),
                tags: Vec::new(),
            })
            .collect();
        let set = CognateSet {
            proto: if borrowed {
                "bor:loan".into()
            } else {
                proto_word.clone()
            },
            etymon: proto_word.clone(),
            borrowed,
            pos: entry.pos,
            gloss: entry.english.clone(),
            members,
        };
        let g = corpus::generate_set(set, &cfg);
        let form = g.form();
        n += 1;
        if borrowed {
            bor += 1
        } else {
            inh += 1
        }
        if ortho::exact_match(form, &entry.isv) {
            exact += 1;
        }
        if ortho::normalized_match(form, &entry.isv) {
            norm += 1;
        }
    }

    let pct = |a: usize| {
        if n == 0 {
            0.0
        } else {
            100.0 * a as f32 / n as f32
        }
    };
    println!(
        "Corpus site path (generate_set) vs official — {n} scorable entries ({inh} inherited + {bor} international):"
    );
    println!(
        "  exact top-1 {:.2}%, normalized top-1 {:.2}%",
        pct(exact),
        pct(norm)
    );
    Ok(())
}

fn build_input(entry: &OfficialEntry) -> MeaningInput {
    let forms: Vec<SourceForm> = consensus::source_forms_from_cells(&entry.cells, |code, form| {
        crate::enrich::english_source_url(form, Some(code))
    });
    let forms = consensus::lemma_forms(forms, entry.pos);
    let (forms, reflexive) = consensus::strip_reflexive(forms, entry.pos);
    MeaningInput {
        pos: entry.pos,
        gender: entry.noun_traits.gender,
        gloss: entry.english.clone(),
        forms,
        is_intl_meaning: entry.genesis.trim() == "I",
        reflexive,
    }
}

fn evaluate_config(
    entries: &[OfficialEntry],
    rung: &Rung,
    proto: Option<&crate::dump::ProtoIndex>,
) -> RunMetrics {
    let mut m = RunMetrics {
        name: rung.name.to_string(),
        description: rung.description.to_string(),
        n: 0,
        exact: 0,
        normalized: 0,
        skeleton: 0,
        top3: 0,
        top5: 0,
        sum_norm_edit: 0.0,
        by_pos: BTreeMap::new(),
        by_branch: Default::default(),
        by_conf: BTreeMap::new(),
        results: Vec::new(),
    };

    for entry in entries {
        let input = build_input(entry);
        // Need at least one modern cognate to have anything to reconstruct from.
        if !input.forms.iter().any(|f| f.modern) {
            continue;
        }
        let (cands, _recon): (Vec<Candidate>, _) =
            crate::pipeline::generate(&input, proto, &rung.cfg);
        let top = cands.first();
        let predicted = top.map(|c| c.form.clone()).unwrap_or_default();
        let confidence = top.map(|c| c.confidence);
        let score = top.map(|c| c.score).unwrap_or(0.0);
        let top_branch_cov = top.map(|c| c.branch_coverage as usize).unwrap_or(0);

        let exact = ortho::exact_match(&predicted, &entry.isv);
        let normalized = ortho::normalized_match(&predicted, &entry.isv);
        let skeleton = ortho::skeleton_match(&predicted, &entry.isv);
        let top3 = cands
            .iter()
            .take(3)
            .any(|c| ortho::normalized_match(&c.form, &entry.isv));
        let top5 = cands
            .iter()
            .take(5)
            .any(|c| ortho::normalized_match(&c.form, &entry.isv));
        let norm_edit = ortho::normalized_edit_distance(&predicted, &entry.isv);
        let branch_cov = top_branch_cov;
        let n_langs = input.forms.iter().filter(|f| f.modern).count();

        let r = EntryResult {
            id: entry.id.clone(),
            isv: entry.isv.clone(),
            gloss: entry.english.clone(),
            pos: entry.pos,
            predicted,
            exact,
            normalized,
            norm_edit,
            branch_cov,
            confidence,
            score,
            n_langs,
        };

        m.n += 1;
        m.exact += exact as usize;
        m.normalized += normalized as usize;
        m.skeleton += skeleton as usize;
        m.top3 += top3 as usize;
        m.top5 += top5 as usize;
        m.sum_norm_edit += norm_edit;
        m.by_pos.entry(r.pos.code()).or_default().add(&r);
        m.by_branch[branch_cov.min(3)].add(&r);
        if let Some(c) = confidence {
            m.by_conf.entry(conf_label(c)).or_default().add(&r);
        }
        m.results.push(r);
    }
    m
}

fn conf_label(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

// ---------------------------------------------------------------------------
// Stage-attribution harness (V7 §2.3).
//
// For each normalized miss, name the *last pipeline stage whose output still
// folded to the official form* — i.e. the stage that destroyed (or never
// produced) the correct answer. This converts the coarse three-way miss
// classification and the "1-letter substitution" buckets into per-stage blame,
// which is the map for where to spend effort. The winning candidate carries a
// full `RuleStep` trace; we replay it, folding each intermediate form to the
// standard alphabet, and find where the correct answer was lost.
// ---------------------------------------------------------------------------

/// The coarse pipeline stage a trace-step id belongs to. `is_proto` disambiguates
/// the few ids shared between the consensus repairs and the Proto-Slavic rule
/// engine (notably `liquid-metathesis`).
fn stage_of_step(id: &str, is_proto: bool) -> &'static str {
    match id {
        "<input>" => "1-normalize/representative",
        "proto-link" => "5-proto-link",
        _ if is_proto => "6-proto-rule",
        "consensus-vote" | "synonym-alt" => "3-cluster/vote",
        "pick-representative" => "1-normalize/representative",
        // Consensus etymological repairs (Stage 4).
        "liquid-metathesis" | "tj-dj-palatal" | "nasal-vowel" | "y-recovery" | "jat-reflex"
        | "adj-fleeting-vowel" | "loan-y-i" | "loan-epenthesis" | "loan-ac-ec" | "loan-ija"
        | "loan-masc-a" | "loan-fem-a" | "loan-ok-suffix" | "loan-yvati" | "verb-husing-a"
        | "verb-stative-eti" | "prefix-voicing" | "loan-ns" | "spirantization-hg" => "4-repair",
        // Morphology / endings (Stage 7).
        "prefix-orz" | "prefix-perd" | "intl-diphthong" | "intl-ic-ical" | "intl-al"
        | "intl-ative" | "intl-ive" | "intl-ous" | "intl-ijny" | "intl-ism" | "intl-ist"
        | "intl-tion" | "intl-sion" | "intl-ssion" | "intl-verb" | "verb-inf-ti" | "adj-hard-y"
        | "adj-soft-i" | "adv-alno" | "noun-alnost" | "noun-ost" | "noun-verbal" | "noun-telj"
        | "noun-ija" | "noun-ika" | "deriv-telj" | "deriv-ost" | "deriv-liv" | "loan-hiatus" => {
            "7-endings"
        }
        _ => "4-repair",
    }
}

/// The consonant-key fingerprint of the *root the winning candidate chose*. For a
/// Proto-Slavic-derived candidate that is the linked reconstruction (so a surface
/// letter the engine mis-derived — e.g. a dropped epenthetic l — does not read as
/// a wrong cluster); otherwise it is the candidate surface itself.
fn winning_root_key(top: &Candidate) -> String {
    if top.source == CandidateSource::ProtoSlavicRule {
        if let Some(st) = top.trace.iter().find(|s| s.id == "proto-link") {
            let w = st.before.trim().trim_start_matches('*');
            return ortho::consonant_key(&ortho::to_standard(&w.to_lowercase()));
        }
    }
    ortho::consonant_key(&top.form)
}

/// Attribute one normalized miss to the pipeline stage responsible. Returns
/// `(stage, detail)` where `stage` is a coarse bucket and `detail` names the
/// specific step/cause.
fn attribute_miss(
    cands: &[Candidate],
    modern_keys: &[String],
    official: &str,
) -> (&'static str, String) {
    let target = ortho::to_standard(&official.trim().to_lowercase());
    let official_key = ortho::consonant_key(&target);

    let Some(top) = cands.first() else {
        return ("0-no-candidate", "empty".into());
    };

    // Merge/rank: a correct *primary* candidate was generated but demoted below
    // the winner (e.g. a wrong proto spelling outranked the correct consensus
    // form — sablja). A correct *synonym-alternative* does NOT count: those are
    // deliberately scored below every primary, so the official form being a
    // secondary translation is an editorial word-choice (wrong-cluster), not a
    // ranking bug.
    if let Some(correct) = cands.iter().find(|c| {
        ortho::normalized_match(&c.form, official) && !c.trace.iter().any(|s| s.id == "synonym-alt")
    }) {
        // A same-root demotion (correct surface of the SAME cluster lost to the
        // winner — e.g. a wrong proto spelling outranking the right consensus
        // form) is a genuine ranking bug; a different-root demotion is the
        // official picking a synonym we ranked as a non-top primary (editorial).
        let detail = if winning_root_key(correct) == winning_root_key(top) {
            "same-root-surface"
        } else {
            "diff-root-editorial"
        };
        return ("8-merge-rank", detail.into());
    }

    // Root absent from the modern evidence entirely: nothing downstream could
    // recover it (extraction/evidence gap).
    if !modern_keys.iter().any(|k| k == &official_key) {
        return ("0-root-absent", "evidence-gap".into());
    }

    // Wrong cluster: the winner chose a different root than the official one.
    if winning_root_key(top) != official_key {
        return ("3-cluster/vote", "wrong-cluster".into());
    }

    // Right cluster, wrong form. Walk the winning candidate's trace.
    let is_proto = top.source == CandidateSource::ProtoSlavicRule;
    if is_proto {
        // The reconstruction (root) is right; the engine derived a wrong surface.
        // Blame the last engine step that changed the folded form.
        let mut culprit = "proto-rule-residual".to_string();
        let mut prev = String::new();
        for (i, st) in top.trace.iter().enumerate() {
            if st.id == "proto-link" {
                continue;
            }
            let after = ortho::to_standard(&st.after.trim().to_lowercase());
            let before = if i == 0 {
                ortho::to_standard(&st.before.trim().to_lowercase())
            } else {
                prev.clone()
            };
            if after != before {
                culprit = st.id.clone();
            }
            prev = after;
        }
        return ("6-proto-rule", culprit);
    }

    // Consensus candidate: did some stage break a correct intermediate?
    attribute_within_consensus(&top.trace, &target)
}

/// Walk a consensus candidate's trace as a linear before→after chain and locate
/// the stage that destroyed a correct intermediate, or — if the form was never
/// correct — bucket by where the residual difference lands.
fn attribute_within_consensus(trace: &[RuleStep], target: &str) -> (&'static str, String) {
    if trace.is_empty() {
        return ("1-normalize/representative", "no-trace".into());
    }
    let mut ids: Vec<&str> = Vec::with_capacity(trace.len() + 1);
    let mut forms: Vec<String> = Vec::with_capacity(trace.len() + 1);
    ids.push("<input>");
    forms.push(ortho::to_standard(&trace[0].before.trim().to_lowercase()));
    for st in trace {
        ids.push(st.id.as_str());
        forms.push(ortho::to_standard(&st.after.trim().to_lowercase()));
    }
    // Last index that folded to the target.
    let last_ok = forms.iter().rposition(|f| f == target);
    if let Some(i) = last_ok {
        if i + 1 < ids.len() {
            let culprit = ids[i + 1];
            return (stage_of_step(culprit, false), culprit.to_string());
        }
    }
    // Never correct: attribute by residual. An ending-only difference is a
    // morphology miss; otherwise the surface never got close (representative /
    // missing repair), sub-classified by the kind of residual difference.
    let pred = &forms[forms.len() - 1];
    if diff_is_ending(pred, target) {
        ("7-endings", "ending-residual".into())
    } else {
        (
            "1-normalize/representative",
            format!("residual:{}", residual_kind(pred, target)),
        )
    }
}

/// The kind of a stem-level residual difference, so the representative bucket is
/// further broken down (flavored-letter vs y/i vs length vs substitution).
fn residual_kind(pred: &str, target: &str) -> &'static str {
    if ortho::ascii_skeleton(pred) == ortho::ascii_skeleton(target) {
        return "flavored-letter";
    }
    let py = pred.contains('y');
    let ty = target.contains('y');
    let pi = pred.contains('i');
    let ti = target.contains('i');
    if (py != ty) || (pi != ti) {
        return "y/i";
    }
    if pred.chars().count() != target.chars().count() {
        return "length";
    }
    "substitution"
}

/// True when two folded forms differ only in a short word-final region (an
/// ending/citation-form difference rather than a stem/root difference).
fn diff_is_ending(a: &str, b: &str) -> bool {
    let ca: Vec<char> = a.chars().collect();
    let cb: Vec<char> = b.chars().collect();
    let common = ca.iter().zip(cb.iter()).take_while(|(x, y)| x == y).count();
    let tail = ca.len().max(cb.len()) - common;
    // Shared stem of >=3 chars and a divergent tail of <=3 chars.
    common >= 3 && tail <= 3
}

/// Data-quality / ceiling audit (§2/§6 of the V4 plan). For every benchmark miss
/// it asks: is the official root even present in the modern evidence (so better
/// cluster *selection* could fix it), was the right cluster chosen but the
/// surface/form wrong (engine error), or is the official root absent from the
/// evidence entirely (unfixable from the cognates we have)? Also reports the
/// cognate cohesion of each meaning. Uses `isv` only for this offline analysis —
/// never on the benchmark path.
pub fn run_audit(official_path: &Path, out_dir: &Path) -> Result<()> {
    let entries: Vec<OfficialEntry> = official::load(official_path)?
        .into_iter()
        .filter(|e| e.is_benchmarkable())
        .collect();
    let proto = load_proto_index();
    let cfg = ConsensusConfig::production();

    let (mut n, mut miss) = (0usize, 0usize);
    // miss classes
    let (mut wrong_cluster, mut right_cluster_wrong_form, mut root_absent) =
        (0usize, 0usize, 0usize);
    // cohesion: distinct consonant-keys among modern forms
    let mut cohesion_hist: BTreeMap<usize, usize> = BTreeMap::new();
    let mut miss_rows: Vec<String> = Vec::new();
    // Stage-attribution histograms (V7 §2.3): coarse stage → count, and the
    // (stage, detail) pair → count, computed over ALL misses (not just the
    // capped CSV sample) so the blame map is complete.
    let mut stage_hist: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut detail_hist: BTreeMap<(&'static str, String), usize> = BTreeMap::new();

    for entry in &entries {
        let input = build_input(entry);
        let modern: Vec<&crate::consensus::SourceForm> =
            input.forms.iter().filter(|f| f.modern).collect();
        if modern.is_empty() {
            continue;
        }
        n += 1;

        // Distinct cognate clusters (consonant-key) among the modern forms.
        let mut keys: Vec<String> = Vec::new();
        for f in &modern {
            let k = ortho::consonant_key(&f.norm.latin);
            if !k.is_empty() && !keys.contains(&k) {
                keys.push(k);
            }
        }
        *cohesion_hist.entry(keys.len()).or_default() += 1;

        let (cands, _) = crate::pipeline::generate(&input, proto.as_ref(), &cfg);
        let predicted = cands.first().map(|c| c.form.clone()).unwrap_or_default();
        if ortho::normalized_match(&predicted, &entry.isv) {
            continue;
        }
        miss += 1;

        let official_key = ortho::consonant_key(&ortho::to_standard(&entry.isv));
        let predicted_key = ortho::consonant_key(&predicted);
        let root_in_evidence = keys.iter().any(|k| k == &official_key);

        let class = if !root_in_evidence {
            root_absent += 1;
            "root-absent"
        } else if predicted_key == official_key {
            right_cluster_wrong_form += 1;
            "right-cluster-wrong-form"
        } else {
            wrong_cluster += 1;
            "wrong-cluster"
        };

        // Per-stage blame (V7 §2.3): which stage lost the official form.
        let (stage, detail) = attribute_miss(&cands, &keys, &entry.isv);
        *stage_hist.entry(stage).or_default() += 1;
        *detail_hist.entry((stage, detail.clone())).or_default() += 1;

        // All misses (no cap): predictions.csv has the hits, this has the
        // per-stage blame — together they make offline pattern mining possible.
        {
            miss_rows.push(format!(
                "{},{},{},{},{},{},{},{}",
                csv_escape(&entry.english),
                entry.pos.code(),
                csv_escape(&entry.isv),
                csv_escape(&predicted),
                keys.len(),
                class,
                stage,
                csv_escape(&detail),
            ));
        }
    }

    let pct = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f32 / b as f32
        }
    };
    println!("Audit over {} benchmarkable meanings ({} misses):", n, miss);
    println!(
        "  miss classes: wrong-cluster {:.1}% | right-cluster-wrong-form {:.1}% | root-absent {:.1}%",
        pct(wrong_cluster, miss),
        pct(right_cluster_wrong_form, miss),
        pct(root_absent, miss),
    );
    println!("  → cluster-selection ceiling: fixing wrong-cluster misses could recover up to {:.1}% of all misses ({} entries)", pct(wrong_cluster, miss), wrong_cluster);
    let single = *cohesion_hist.get(&1).unwrap_or(&0);
    println!(
        "  cohesion: {:.1}% of meanings are a single cognate cluster; {:.1}% have >=3 clusters",
        pct(single, n),
        pct(
            cohesion_hist
                .iter()
                .filter(|(k, _)| **k >= 3)
                .map(|(_, v)| *v)
                .sum(),
            n
        ),
    );

    // ---- Stage-attribution histogram (V7 §2.3) ----
    println!(
        "\n  Stage-attribution histogram ({} misses, per-stage blame):",
        miss
    );
    let mut stages: Vec<(&&'static str, &usize)> = stage_hist.iter().collect();
    stages.sort_by(|a, b| b.1.cmp(a.1));
    for (stage, cnt) in &stages {
        println!("    {:<26} {:>5}  {:>5.1}%", stage, cnt, pct(**cnt, miss));
        // Top detail causes within this stage.
        let mut details: Vec<(&(&'static str, String), &usize)> = detail_hist
            .iter()
            .filter(|((st, _), _)| st == *stage)
            .collect();
        details.sort_by(|a, b| b.1.cmp(a.1));
        for ((_, d), c) in details.iter().take(4) {
            println!("        · {:<24} {:>5}", d, c);
        }
    }

    std::fs::create_dir_all(out_dir)?;
    let mut s =
        String::from("gloss,pos,official,predicted,n_clusters,miss_class,stage,stage_detail\n");
    for r in &miss_rows {
        s.push_str(r);
        s.push('\n');
    }
    std::fs::write(out_dir.join("audit-misses.csv"), s)?;
    println!("Wrote {}", out_dir.join("audit-misses.csv").display());

    // Machine + human readable stage-attribution report.
    let mut sa = String::new();
    writeln!(sa, "# Stage-attribution histogram (V7 §2.3)\n")?;
    writeln!(
        sa,
        "For each of the **{}** normalized misses (of {} benchmarkable meanings), the last pipeline stage whose output still folded to the official form — i.e. the stage that destroyed, or never produced, the correct answer. Computed by replaying the winning candidate's `RuleStep` trace.\n",
        miss, n
    )?;
    writeln!(sa, "| Stage | misses | share |")?;
    writeln!(sa, "|---|---:|---:|")?;
    for (stage, cnt) in &stages {
        writeln!(sa, "| {} | {} | {:.1}% |", stage, cnt, pct(**cnt, miss))?;
    }
    writeln!(sa, "\n## Top causes within each stage\n")?;
    writeln!(sa, "| Stage | detail | misses |")?;
    writeln!(sa, "|---|---|---:|")?;
    let mut all_details: Vec<(&(&'static str, String), &usize)> = detail_hist.iter().collect();
    all_details.sort_by(|a, b| b.1.cmp(a.1));
    for ((stage, detail), cnt) in all_details.iter().take(30) {
        writeln!(sa, "| {} | {} | {} |", stage, detail, cnt)?;
    }
    std::fs::write(out_dir.join("stage-attribution.md"), sa)?;
    println!("Wrote {}", out_dir.join("stage-attribution.md").display());
    Ok(())
}

/// Diagnostic oracle ladder (V7 §2.4): the upper bound each stage would deliver
/// if it were made perfect while everything downstream stayed real. Every oracle
/// READS THE OFFICIAL ANSWER, so this path can NEVER feed production — it exists
/// only to rank stages by recoverable headroom (stage → headroom in pp of exact
/// top-1 over production).
pub fn run_oracle(official_path: &Path, out_dir: &Path) -> Result<()> {
    let entries: Vec<OfficialEntry> = official::load(official_path)?
        .into_iter()
        .filter(|e| e.is_benchmarkable())
        .collect();
    let proto = load_proto_index();
    let cfg = ConsensusConfig::production();

    // (name, cluster, representative, proto_link) — each flips exactly one oracle,
    // then "oracle-all" flips them together.
    let variants: &[(&str, bool, bool, bool)] = &[
        ("oracle-cluster", true, false, false),
        ("oracle-representative", false, true, false),
        ("oracle-proto-link", false, false, true),
        ("oracle-all", true, true, true),
    ];

    let run_pass = |cl: bool, rep: bool, pl: bool| -> (usize, usize, usize) {
        let (mut ex, mut nm, mut denom) = (0usize, 0usize, 0usize);
        for entry in &entries {
            let input = build_input(entry);
            if !input.forms.iter().any(|f| f.modern) {
                continue;
            }
            denom += 1;
            let (cands, _) = if !cl && !rep && !pl {
                crate::pipeline::generate(&input, proto.as_ref(), &cfg)
            } else {
                let oracle = consensus::Oracle {
                    official: &entry.isv,
                    cluster: cl,
                    representative: rep,
                    proto_link: pl,
                    force_cluster_key: None,
                    rep_rule: None,
                };
                crate::pipeline::generate_oracle(&input, proto.as_ref(), &cfg, Some(&oracle))
            };
            let pred = cands.first().map(|c| c.form.clone()).unwrap_or_default();
            ex += ortho::exact_match(&pred, &entry.isv) as usize;
            nm += ortho::normalized_match(&pred, &entry.isv) as usize;
        }
        (ex, nm, denom)
    };

    let (base_ex, base_nm, denom) = run_pass(false, false, false);
    let pct = |a: usize| {
        if denom == 0 {
            0.0
        } else {
            100.0 * a as f32 / denom as f32
        }
    };
    println!(
        "Diagnostic oracle ladder (DIAGNOSTIC ONLY — reads the answer, never production; {denom} meanings):"
    );
    println!(
        "  baseline (production)   exact {:.2}%   norm {:.2}%",
        pct(base_ex),
        pct(base_nm)
    );

    let mut rows: Vec<(String, f32, f32, f32, f32)> = Vec::new();
    for (name, cl, rep, pl) in variants {
        let (ex, nm, _) = run_pass(*cl, *rep, *pl);
        let (dex, dnm) = (pct(ex) - pct(base_ex), pct(nm) - pct(base_nm));
        println!(
            "  {:<23} exact {:.2}% ({:+.2}pp)   norm {:.2}% ({:+.2}pp)",
            name,
            pct(ex),
            dex,
            pct(nm),
            dnm
        );
        rows.push((name.to_string(), pct(ex), dex, pct(nm), dnm));
    }

    std::fs::create_dir_all(out_dir)?;
    let mut s = String::new();
    writeln!(s, "# Oracle ladder (V7 §2.4) — DIAGNOSTIC ONLY\n")?;
    writeln!(
        s,
        "Each row makes ONE pipeline stage perfect (by reading the official answer) while everything downstream stays the real production engine, over **{}** benchmarkable meanings. This path can never feed production; it exists only to rank stages by recoverable headroom. Spend effort top-down by Δ exact.\n",
        denom
    )?;
    writeln!(
        s,
        "| Stage oracle | exact top-1 | Δ exact | norm top-1 | Δ norm |"
    )?;
    writeln!(s, "|---|---:|---:|---:|---:|")?;
    writeln!(
        s,
        "| baseline (production) | {:.2}% | — | {:.2}% | — |",
        pct(base_ex),
        pct(base_nm)
    )?;
    for (name, ex, dex, nm, dnm) in &rows {
        writeln!(
            s,
            "| {} | {:.2}% | {:+.2}pp | {:.2}% | {:+.2}pp |",
            name, ex, dex, nm, dnm
        )?;
    }
    writeln!(
        s,
        "\n- **oracle-cluster** — force the vote to the cluster whose consonant key matches the official lemma; representative + repairs then run on the right cluster.\n- **oracle-representative** — pick the winning group's member whose folded form is closest to the official lemma.\n- **oracle-proto-link** — link the reconstruction whose derived form is closest to the official lemma (linker upper bound).\n- **oracle-all** — all three at once (an approximate ceiling for the stages below word-selection)."
    )?;
    std::fs::write(out_dir.join("oracle-ladder.md"), s)?;
    println!("Wrote {}", out_dir.join("oracle-ladder.md").display());
    Ok(())
}

/// Cluster-selection headroom (Measurement #2). The wrong-cluster miss bucket is
/// mostly the official dictionary choosing a different (editorial) root than our
/// vote's plurality — the oracle-cluster stage showed ~+3.9pp is *there* if we
/// pick the official root, but most of that reads the answer. This measures how
/// much a **leakage-free recognizability rule** recovers: force the winning
/// cluster by an answer-blind rule (most distinct languages / branches, or an
/// internationalism-first preference) and score the real downstream pipeline. If
/// the blind rules barely beat production, the editorial slice is a genuine
/// human-judgment ceiling; whatever they *do* recover is a concrete answer-free
/// fix. Also reports each rule's cluster-selection precision on the meanings
/// whose official root is present in the evidence (the recoverable slice).
pub fn run_select_eval(official_path: &Path, out_dir: &Path) -> Result<()> {
    let entries: Vec<OfficialEntry> = official::load(official_path)?
        .into_iter()
        .filter(|e| e.is_benchmarkable())
        .collect();
    let proto = load_proto_index();
    let cfg = ConsensusConfig::production();

    struct Clus {
        key: String,
        langs: usize,
        branches: usize,
        intl: bool,
    }
    // Cluster stats for a meaning: for each consonant-key group among the primary
    // modern cognates, how many distinct languages/branches attest it and whether
    // any member looks international.
    let clusters = |input: &MeaningInput| -> Vec<Clus> {
        let mut per_lang: BTreeMap<&str, &SourceForm> = BTreeMap::new();
        for f in &input.forms {
            if f.modern && f.primary {
                per_lang.entry(f.lang_code.as_str()).or_insert(f);
            }
        }
        let mut map: BTreeMap<String, (Vec<&str>, Vec<crate::lang::Branch>, bool)> =
            BTreeMap::new();
        for f in per_lang.values() {
            let k = ortho::consonant_key(&f.norm.latin);
            if k.is_empty() {
                continue;
            }
            let e = map.entry(k).or_default();
            if !e.0.contains(&f.lang_code.as_str()) {
                e.0.push(f.lang_code.as_str());
            }
            if !e.1.contains(&f.branch) {
                e.1.push(f.branch);
            }
            e.2 |= consensus::is_international_form(&f.norm.latin);
        }
        map.into_iter()
            .map(|(key, (l, b, i))| Clus {
                key,
                langs: l.len(),
                branches: b.len(),
                intl: i,
            })
            .collect()
    };

    // Leakage-free selection rules (except `oracle-cluster`, the upper bound).
    // Return the cluster key to force, or None = leave the real vote (production).
    let pick = |name: &str, cs: &[Clus], official: &str| -> Option<String> {
        match name {
            "max-langs" => cs
                .iter()
                .max_by_key(|c| (c.langs, c.branches))
                .map(|c| c.key.clone()),
            "max-branches" => cs
                .iter()
                .max_by_key(|c| (c.branches, c.langs))
                .map(|c| c.key.clone()),
            "intl-first" => cs
                .iter()
                .filter(|c| c.intl)
                .max_by_key(|c| (c.langs, c.branches))
                .map(|c| c.key.clone()),
            "oracle-cluster" => {
                let ok = ortho::consonant_key(&ortho::to_standard(&official.to_lowercase()));
                cs.iter().find(|c| c.key == ok).map(|c| c.key.clone())
            }
            _ => None, // "production"
        }
    };

    let rule_names = [
        "production",
        "max-langs",
        "max-branches",
        "intl-first",
        "oracle-cluster",
    ];
    let mut results: Vec<(String, f32, f32, f32)> = Vec::new(); // (name, exact%, norm%, cluster-hit%)
    let mut denom_ref = 0usize;
    for name in rule_names {
        let (mut ex, mut nm, mut denom) = (0usize, 0usize, 0usize);
        let (mut hit, mut hit_denom) = (0usize, 0usize);
        for entry in &entries {
            let input = build_input(entry);
            if !input.forms.iter().any(|f| f.modern) {
                continue;
            }
            denom += 1;
            let cs = clusters(&input);
            let official_key = ortho::consonant_key(&ortho::to_standard(&entry.isv.to_lowercase()));
            let official_present = cs.iter().any(|c| c.key == official_key);
            let forced = pick(name, &cs, &entry.isv);
            let (cands, _) = if let Some(k) = &forced {
                let oracle = consensus::Oracle {
                    official: &entry.isv,
                    cluster: false,
                    representative: false,
                    proto_link: false,
                    force_cluster_key: Some(k.as_str()),
                    rep_rule: None,
                };
                crate::pipeline::generate_oracle(&input, proto.as_ref(), &cfg, Some(&oracle))
            } else {
                crate::pipeline::generate(&input, proto.as_ref(), &cfg)
            };
            let pred = cands.first().map(|c| c.form.clone()).unwrap_or_default();
            ex += ortho::exact_match(&pred, &entry.isv) as usize;
            nm += ortho::normalized_match(&pred, &entry.isv) as usize;
            if official_present {
                hit_denom += 1;
                // Approximate cluster-selection precision: does the top candidate's
                // root fingerprint match the official one? (For proto-derived tops a
                // dropped/added consonant can shift the surface key, so this slightly
                // under-counts — it is a floor.)
                if winning_root_key(cands.first().unwrap()) == official_key {
                    hit += 1;
                }
            }
        }
        denom_ref = denom;
        let pct = |a: usize, b: usize| {
            if b == 0 {
                0.0
            } else {
                100.0 * a as f32 / b as f32
            }
        };
        results.push((
            name.to_string(),
            pct(ex, denom),
            pct(nm, denom),
            pct(hit, hit_denom),
        ));
    }

    let base_ex = results[0].1;
    let base_nm = results[0].2;
    println!(
        "Cluster-selection headroom (Measurement #2 — leakage-free rules vs production vs oracle; {denom_ref} meanings):"
    );
    println!(
        "  {:<16} {:>7}  {:>8}  {:>7}  {:>8}  {:>10}",
        "rule", "exact", "Δexact", "norm", "Δnorm", "cluster-hit"
    );
    for (name, exr, nmr, hitr) in &results {
        println!(
            "  {:<16} {:>6.2}%  {:>+7.2}  {:>6.2}%  {:>+7.2}  {:>9.1}%",
            name,
            exr,
            exr - base_ex,
            nmr,
            nmr - base_nm,
            hitr
        );
    }

    std::fs::create_dir_all(out_dir)?;
    let mut s = String::new();
    writeln!(s, "# Cluster-selection headroom (Measurement #2)\n")?;
    writeln!(
        s,
        "The wrong-cluster miss bucket is mostly the official dictionary choosing a different (editorial) root than our plurality vote. This forces the winning cluster by a **leakage-free** rule (except `oracle-cluster`, which reads the answer as the ceiling) and scores the real pipeline over {denom_ref} meanings. `cluster-hit%` is the share of meanings whose official root is in the evidence where the rule's top candidate lands on that root.\n"
    )?;
    writeln!(
        s,
        "| Rule | exact | Δ exact | norm | Δ norm | cluster-hit |"
    )?;
    writeln!(s, "|---|---:|---:|---:|---:|---:|")?;
    for (name, exr, nmr, hitr) in &results {
        writeln!(
            s,
            "| {} | {:.2}% | {:+.2}pp | {:.2}% | {:+.2}pp | {:.1}% |",
            name,
            exr,
            exr - base_ex,
            nmr,
            nmr - base_nm,
            hitr
        )?;
    }
    writeln!(s, "\n- **production** — the real branch-balanced six-subgroup vote (reference).\n- **max-langs / max-branches** — force the cluster attested by the most distinct languages / branches (a raw recognizability proxy).\n- **intl-first** — force any internationalism cluster (tests extending the genesis=I preference to every meaning).\n- **oracle-cluster** — force the official cluster (upper bound; reads the answer).")?;
    std::fs::write(out_dir.join("cluster-selection.md"), s)?;
    println!("Wrote {}", out_dir.join("cluster-selection.md").display());
    Ok(())
}

/// Synonym-aware accuracy (`synonym-eval`). The strict metric asks "did we match
/// the ONE official headword?", but the cluster-selection measurement proved ~49%
/// of misses are editorial word-choice — the engine produced a valid Interslavic
/// word the committee simply did not pick as *the* lemma. This credits a
/// prediction when it reproduces **any** official Interslavic lemma whose gloss
/// matches this concept (an acceptable synonym), and breaks the remaining misses
/// into "another official word, different sense" vs "no official word" (a genuinely
/// novel/wrong form). Diagnostic only — never a gate.
pub fn run_synonym_eval(official_path: &Path, out_dir: &Path) -> Result<()> {
    use std::collections::{HashMap, HashSet};
    let all = official::load(official_path)?;
    // Every official ISV lemma → the union of its English gloss tokens, so a
    // prediction can be checked against the meaning of any official word.
    let mut by_form: HashMap<String, HashSet<String>> = HashMap::new();
    for e in &all {
        let isv = e.isv.trim();
        if isv.is_empty() {
            continue;
        }
        let key = ortho::to_standard(&isv.to_lowercase());
        by_form
            .entry(key)
            .or_default()
            .extend(crate::dump::gloss_tokens(&e.english));
    }

    // The precise synonym signal: the dictionary-derived thesaurus (shared
    // translation ∩ gloss ∩ POS), so a "valid synonym" is a direct synonym of
    // *this* concept, not merely any official word sharing a common token.
    let thes = crate::thesaurus::Thesaurus::build(&all);

    let proto = load_proto_index();
    let cfg = ConsensusConfig::production();
    let (mut n, mut exact, mut norm) = (0usize, 0usize, 0usize);
    // Miss breakdown: valid synonym | official word, other sense | not any official.
    let (mut syn, mut other_sense, mut not_official) = (0usize, 0usize, 0usize);

    for entry in all.iter().filter(|e| e.is_benchmarkable()) {
        let input = build_input(entry);
        if !input.forms.iter().any(|f| f.modern) {
            continue;
        }
        n += 1;
        let (cands, _) = crate::pipeline::generate(&input, proto.as_ref(), &cfg);
        let pred = cands.first().map(|c| c.form.clone()).unwrap_or_default();
        exact += ortho::exact_match(&pred, &entry.isv) as usize;
        let nm = ortho::normalized_match(&pred, &entry.isv);
        norm += nm as usize;
        if nm {
            continue;
        }
        // A miss: classify the prediction against the thesaurus.
        let pk = ortho::to_standard(&pred.trim().to_lowercase());
        if !pred.is_empty() && thes.are_synonyms(&entry.isv, &pred) {
            syn += 1; // a valid synonym of this exact concept
        } else if !pred.is_empty() && by_form.contains_key(&pk) {
            other_sense += 1; // some official lemma, but a different meaning
        } else {
            not_official += 1; // not any official lemma (novel form or genuine error)
        }
    }

    let miss = n - norm;
    let pct = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f32 / b as f32
        }
    };
    let syn_incl = norm + syn;
    println!("Synonym-aware accuracy over {n} benchmarkable meanings:");
    println!("  exact top-1                 {:.2}%", pct(exact, n));
    println!("  normalized top-1 (strict)   {:.2}%", pct(norm, n));
    println!(
        "  synonym-inclusive top-1     {:.2}%   (+{:.2}pp: produced a valid ISV synonym)",
        pct(syn_incl, n),
        pct(syn, n)
    );
    println!("  Of the {miss} strict misses:");
    println!(
        "    valid ISV synonym (another official lemma, same concept) {:>5}  {:>5.1}%",
        syn,
        pct(syn, miss)
    );
    println!(
        "    another official lemma, different sense                  {:>5}  {:>5.1}%",
        other_sense,
        pct(other_sense, miss)
    );
    println!(
        "    not any official lemma (novel form or genuine error)     {:>5}  {:>5.1}%",
        not_official,
        pct(not_official, miss)
    );

    std::fs::create_dir_all(out_dir)?;
    let mut s = String::new();
    writeln!(s, "# Synonym-aware accuracy (synonym-eval)\n")?;
    writeln!(
        s,
        "The strict benchmark scores agreement with the ONE official headword. But ~49% of misses are editorial word-choice (see the cluster-selection measurement): the engine produced a valid Interslavic word the committee did not pick as *the* lemma. This credits a prediction that reproduces **any** official ISV lemma whose gloss matches the concept.\n"
    )?;
    writeln!(s, "| Metric | top-1 |")?;
    writeln!(s, "|---|---:|")?;
    writeln!(s, "| exact | {:.2}% |", pct(exact, n))?;
    writeln!(s, "| normalized (strict) | {:.2}% |", pct(norm, n))?;
    writeln!(
        s,
        "| **synonym-inclusive** | **{:.2}%** |",
        pct(syn_incl, n)
    )?;
    writeln!(s, "\n## What the {miss} strict misses actually are\n")?;
    writeln!(s, "| Class | count | share of misses |")?;
    writeln!(s, "|---|---:|---:|")?;
    writeln!(
        s,
        "| valid ISV synonym (another official lemma, same concept) | {} | {:.1}% |",
        syn,
        pct(syn, miss)
    )?;
    writeln!(
        s,
        "| another official lemma, different sense | {} | {:.1}% |",
        other_sense,
        pct(other_sense, miss)
    )?;
    writeln!(
        s,
        "| not any official lemma (novel form or genuine error) | {} | {:.1}% |",
        not_official,
        pct(not_official, miss)
    )?;
    std::fs::write(out_dir.join("synonym-accuracy.md"), s)?;
    println!("Wrote {}", out_dir.join("synonym-accuracy.md").display());
    Ok(())
}

/// Evidence-growth audit + augmentation A/B (Track E / issue #4,
/// `evidence-eval`).
///
/// `0-root-absent` misses (the official root is not among the dictionary's own
/// cited cognates) are an EVIDENCE ceiling, not a rule ceiling. This command:
///  1. measures the baseline root-absent rate,
///  2. quantifies how much of it is *recoverable* — the official root exists in
///     the committed Wiktionary lemma cache under a gloss-matched lemma,
///  3. runs the honest A/B: augment each meaning's evidence with gloss-matched
///     cache cognates for languages the dictionary row does NOT cite
///     (conservative: augmentation only widens language coverage, it never
///     competes with the dictionary's own citation for a language), and
///     re-measures accuracy + root-absent rate with a paired sign test.
///
/// Leakage story: the cache is Wiktionary-derived (never saw the `isv` answer),
/// matching uses only the English gloss + POS, and the answer is read only for
/// scoring — same discipline as the headline benchmark.
pub fn run_evidence_eval(official_path: &Path, out_dir: &Path) -> Result<()> {
    use std::collections::{HashMap, HashSet};
    let entries: Vec<OfficialEntry> = official::load(official_path)?
        .into_iter()
        .filter(|e| e.is_benchmarkable())
        .collect();
    let proto = load_proto_index();
    let cfg = ConsensusConfig::production();
    let corpus = crate::dump::LemmaCorpus::load(Path::new(crate::DEFAULT_LEMMA_CACHE))?;

    // Gloss-token index over the cache, restricted to the benchmark languages
    // (the small lects have no column in the dictionary anyway). `sh` is the
    // Serbo-Croatian macro-code English Wiktionary files hr/sr/bs entries under.
    const LANGS: &[&str] = &[
        "ru", "uk", "be", "pl", "cs", "sk", "sl", "hr", "sr", "sh", "bg", "mk",
    ];
    let mut by_token: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, l) in corpus.entries.iter().enumerate() {
        if !LANGS.contains(&l.lang.as_str()) || l.word.contains(' ') {
            continue;
        }
        for t in crate::dump::gloss_tokens(&l.gloss) {
            by_token.entry(t).or_default().push(i);
        }
    }

    // Candidate cache cognates for a meaning, ranked by gloss-token overlap.
    // A single shared frequent token ("piece", "make") matches thousands of
    // unrelated lemmas, so a candidate must share ≥2 tokens OR fully cover the
    // shorter gloss (single-token glosses like "apple" still match).
    let candidates = |e: &OfficialEntry| -> Vec<usize> {
        let etoks: HashSet<String> = crate::dump::gloss_tokens(&e.english).into_iter().collect();
        let mut overlap: HashMap<usize, usize> = HashMap::new();
        for t in &etoks {
            if let Some(v) = by_token.get(t) {
                for &i in v {
                    if corpus.entries[i].pos == e.pos.code() {
                        *overlap.entry(i).or_default() += 1;
                    }
                }
            }
        }
        let mut v: Vec<(usize, usize)> = overlap
            .into_iter()
            .filter(|(i, ov)| {
                let ltoks = crate::dump::gloss_tokens(&corpus.entries[*i].gloss).len();
                *ov >= 2 || *ov >= etoks.len().min(ltoks)
            })
            .collect();
        // Best overlap first, index as the deterministic tie-break.
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v.into_iter().map(|(i, _)| i).collect()
    };

    struct Pass {
        n: usize,
        exact: usize,
        norm: usize,
        root_absent: usize,
        results: HashMap<String, bool>, // id -> normalized hit
    }
    let run_pass = |augment: bool| -> Pass {
        let mut p = Pass {
            n: 0,
            exact: 0,
            norm: 0,
            root_absent: 0,
            results: HashMap::new(),
        };
        for e in &entries {
            let mut input = build_input(e);
            if !input.forms.iter().any(|f| f.modern) {
                continue;
            }
            if augment && !input.reflexive {
                // Fill ONLY languages the dictionary row does not cite.
                // (Reflexive meanings are skipped: added cognates would bypass
                // the reflexive-marker stripping build_input already ran.)
                let cited: HashSet<&str> =
                    input.forms.iter().map(|f| f.lang_code.as_str()).collect();
                let mut add_cells: HashMap<String, String> = HashMap::new();
                for i in candidates(e) {
                    let l = &corpus.entries[i];
                    if cited.contains(l.lang.as_str()) || add_cells.contains_key(&l.lang) {
                        continue;
                    }
                    add_cells.insert(l.lang.clone(), l.word.clone());
                }
                if !add_cells.is_empty() {
                    let extra = consensus::source_forms_from_cells(&add_cells, |code, form| {
                        crate::enrich::english_source_url(form, Some(code))
                    });
                    let extra = consensus::lemma_forms(extra, e.pos);
                    input.forms.extend(extra);
                }
            }
            p.n += 1;
            let official_key = ortho::consonant_key(&ortho::to_standard(&e.isv.to_lowercase()));
            let root_present = input
                .forms
                .iter()
                .filter(|f| f.modern)
                .any(|f| ortho::consonant_key(&f.norm.latin) == official_key);
            let (cands, _) = crate::pipeline::generate(&input, proto.as_ref(), &cfg);
            let pred = cands.first().map(|c| c.form.clone()).unwrap_or_default();
            let ex = ortho::exact_match(&pred, &e.isv);
            let nm = ortho::normalized_match(&pred, &e.isv);
            p.exact += ex as usize;
            p.norm += nm as usize;
            if !nm && !root_present {
                p.root_absent += 1;
            }
            p.results.insert(e.id.clone(), nm);
        }
        p
    };

    let base = run_pass(false);
    // Recoverability, split by whether the conservative augmentation is even
    // ALLOWED to inject the root-carrying lemma:
    //  - reachable: root under an UNCITED language (and not a bg/mk verb
    //    citation, which lemma_forms drops);
    //  - cited-language: the root sits in a language the row already cites —
    //    unreachable without displacing the dictionary's own citation;
    //  - bg/mk-verb: dropped by the no-infinitive rule.
    let (mut recoverable, mut rec_reachable, mut rec_cited, mut rec_bgmk) =
        (0usize, 0usize, 0usize, 0usize);
    for e in &entries {
        if base.results.get(&e.id).copied().unwrap_or(true) {
            continue;
        }
        let input = build_input(e);
        let official_key = ortho::consonant_key(&ortho::to_standard(&e.isv.to_lowercase()));
        let root_present = input
            .forms
            .iter()
            .filter(|f| f.modern)
            .any(|f| ortho::consonant_key(&f.norm.latin) == official_key);
        if root_present {
            continue;
        }
        let cited: HashSet<&str> = input.forms.iter().map(|f| f.lang_code.as_str()).collect();
        let carriers: Vec<usize> = candidates(e)
            .into_iter()
            .filter(|&i| {
                ortho::consonant_key(&crate::normalize::to_phonemic_latin(
                    &corpus.entries[i].lang,
                    &corpus.entries[i].word,
                )) == official_key
            })
            .collect();
        if carriers.is_empty() {
            continue;
        }
        recoverable += 1;
        let reachable = carriers.iter().any(|&i| {
            let l = &corpus.entries[i];
            !(cited.contains(l.lang.as_str())
                || e.pos == Pos::Verb && matches!(l.lang.as_str(), "bg" | "mk"))
        });
        if reachable {
            rec_reachable += 1;
        } else if carriers
            .iter()
            .any(|&i| e.pos == Pos::Verb && matches!(corpus.entries[i].lang.as_str(), "bg" | "mk"))
        {
            rec_bgmk += 1;
        } else {
            rec_cited += 1;
        }
    }
    let aug = run_pass(true);
    let (mut fixed, mut broke) = (0usize, 0usize);
    for (id, nm) in &aug.results {
        match (base.results.get(id).copied().unwrap_or(false), *nm) {
            (false, true) => fixed += 1,
            (true, false) => broke += 1,
            _ => {}
        }
    }

    let pct = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f32 / b as f32
        }
    };
    println!(
        "Evidence growth (Track E) over {} meanings; cache: {} lemmas",
        base.n, corpus.entry_count
    );
    println!(
        "  baseline: exact {:.2}%  norm {:.2}%  root-absent misses {} ({:.1}% of meanings)",
        pct(base.exact, base.n),
        pct(base.norm, base.n),
        base.root_absent,
        pct(base.root_absent, base.n),
    );
    println!(
        "  recoverable from the cache (gloss+POS matched, official root present): {} of {} root-absent ({:.1}%) — {} reachable by the conservative rule, {} only under a cited language, {} only as bg/mk verb citations",
        recoverable,
        base.root_absent,
        pct(recoverable, base.root_absent),
        rec_reachable,
        rec_cited,
        rec_bgmk,
    );
    println!(
        "  augmented (fill uncited languages only): exact {:.2}% ({:+.2}pp)  norm {:.2}% ({:+.2}pp)  root-absent {} ({:.1}%)",
        pct(aug.exact, aug.n),
        pct(aug.exact, aug.n) - pct(base.exact, base.n),
        pct(aug.norm, aug.n),
        pct(aug.norm, aug.n) - pct(base.norm, base.n),
        aug.root_absent,
        pct(aug.root_absent, aug.n),
    );
    println!(
        "  paired (normalized): fixed {} / broke {}  p = {:.4}",
        fixed,
        broke,
        sign_test_p(fixed, broke)
    );

    std::fs::create_dir_all(out_dir)?;
    let mut s = String::new();
    writeln!(
        s,
        "# Evidence growth vs the root-absent ceiling (evidence-eval)\n"
    )?;
    writeln!(
        s,
        "**Denominator:** {} benchmarkable meanings; the Wiktionary lemma cache holds {} lemmas. **Leakage story:** the cache never saw the `isv` answer; matching uses English gloss tokens + POS only; augmentation fills ONLY languages the dictionary row does not cite, so the dictionary's own evidence is never displaced.\n",
        base.n, corpus.entry_count
    )?;
    writeln!(s, "| Measurement | value |")?;
    writeln!(s, "|---|---:|")?;
    writeln!(
        s,
        "| baseline root-absent misses | {} ({:.1}% of meanings) |",
        base.root_absent,
        pct(base.root_absent, base.n)
    )?;
    writeln!(
        s,
        "| recoverable from the cache (official root present under a gloss-matched lemma) | {} ({:.1}% of root-absent) |",
        recoverable,
        pct(recoverable, base.root_absent)
    )?;
    writeln!(
        s,
        "| — of which reachable by the conservative rule (root under an uncited language) | {} |",
        rec_reachable
    )?;
    writeln!(
        s,
        "| — unreachable: root only under an already-cited language (adding it would displace the dictionary's own citation) | {} |",
        rec_cited
    )?;
    writeln!(
        s,
        "| — unreachable: root only as a bg/mk verb citation (dropped by the no-infinitive rule) | {} |",
        rec_bgmk
    )?;
    writeln!(
        s,
        "| root-absent after augmentation | {} ({:.1}%) |",
        aug.root_absent,
        pct(aug.root_absent, aug.n)
    )?;
    writeln!(
        s,
        "| accuracy: baseline → augmented (exact) | {:.2}% → {:.2}% ({:+.2}pp) |",
        pct(base.exact, base.n),
        pct(aug.exact, aug.n),
        pct(aug.exact, aug.n) - pct(base.exact, base.n)
    )?;
    writeln!(
        s,
        "| accuracy: baseline → augmented (normalized) | {:.2}% → {:.2}% ({:+.2}pp) |",
        pct(base.norm, base.n),
        pct(aug.norm, aug.n),
        pct(aug.norm, aug.n) - pct(base.norm, base.n)
    )?;
    writeln!(
        s,
        "| paired sign test (normalized) | fixed {} / broke {}, p = {:.4} |",
        fixed,
        broke,
        sign_test_p(fixed, broke)
    )?;
    writeln!(
        s,
        "\nDisclosed limits of the A/B: candidates need ≥2 shared gloss tokens (or full cover of the shorter gloss); the per-language pick is the highest-overlap candidate; reflexive meanings are excluded from augmentation (added forms would bypass reflexive-marker stripping); and the conservative fill-uncited-only rule cannot reach roots that sit under an already-cited language — the reachable share is reported separately above, and even a perfect recovery of it bounds the gain below {:.2}pp exact.\n\nThe native uk/sr/bg/sl Wiktionary enrichment named in issue #4 is **data-blocked** (no per-language wiktextract dumps on disk; enrichment affects display only, not benchmark evidence) and is recorded as out of scope here.",
        100.0 * rec_reachable as f32 / base.n.max(1) as f32
    )?;
    std::fs::write(out_dir.join("evidence-growth.md"), s)?;
    println!("Wrote {}", out_dir.join("evidence-growth.md").display());
    Ok(())
}

/// Multi-word & aspect-pair benchmark (Track B / issue #2, `multiword-eval`).
///
/// The headline benchmark excludes every multi-word official lemma
/// (`is_benchmarkable` drops them), so their denominators were invisible. This
/// measures them honestly, in three slices:
///  * **reflexive `X sę`** — the existing pipeline already reconstructs these
///    (cognate reflexive markers are stripped, `sę` re-attached); they were
///    just never scored.
///  * **two-token collocations** — each position is reconstructed as its own
///    consensus problem from the position-split cognate cells (adjective
///    position agreed with the head noun's gender), then joined.
///  * **aspect pairs** — same-gloss ipf/pf lemma pairs that are morphologically
///    related (shared consonant root); both members go through the standard
///    pipeline and the pair is scored both/one/neither.
///
/// Leakage story: the gold `isv` selects the slice (token count / aspect tag)
/// and is never fed to generation; generation sees only the cognate cells +
/// POS/gender metadata, exactly like the headline benchmark.
pub fn run_multiword_eval(official_path: &Path, out_dir: &Path) -> Result<()> {
    use std::collections::HashMap;
    let entries = official::load(official_path)?;
    let proto = load_proto_index();
    let cfg = ConsensusConfig::production();
    let pct = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f32 / b as f32
        }
    };

    // ---- Quantification of the multi-word inventory ----
    let multi: Vec<&OfficialEntry> = entries
        .iter()
        .filter(|e| {
            let w = e.isv.trim();
            w.contains(' ') && !w.contains('#') && !w.contains('!')
        })
        .collect();
    let mut n_sie = 0usize;
    let mut n_two = 0usize;
    let mut n_long = 0usize;
    for e in &multi {
        let toks: Vec<&str> = e.isv.split_whitespace().collect();
        match (toks.len(), toks.last()) {
            (2, Some(&"sę")) => n_sie += 1,
            (2, _) => n_two += 1,
            _ => n_long += 1,
        }
    }

    // ---- Slice A: reflexive "X sę" through the existing pipeline ----
    let (mut a_n, mut a_ex, mut a_nm) = (0usize, 0usize, 0usize);
    // Entries whose cognates carry no detectable reflexive marker: the pipeline
    // never appends " sę", so the full-lemma comparison is a structural miss —
    // reported as its own bucket, not hidden inside the accuracy number.
    let mut a_nodetect = 0usize;
    let (mut a_dev, mut a_dev_nm, mut a_held, mut a_held_nm) = (0usize, 0usize, 0usize, 0usize);
    for e in &multi {
        let toks: Vec<&str> = e.isv.split_whitespace().collect();
        if toks.len() != 2 || toks[1] != "sę" {
            continue;
        }
        let input = build_input(e);
        if !input.forms.iter().any(|f| f.modern) {
            continue;
        }
        a_n += 1;
        if !input.reflexive {
            a_nodetect += 1;
        }
        let (cands, _) = crate::pipeline::generate(&input, proto.as_ref(), &cfg);
        let pred = cands.first().map(|c| c.form.clone()).unwrap_or_default();
        let ex = ortho::exact_match(&pred, e.isv.trim());
        let nm = ortho::normalized_match(&pred, e.isv.trim());
        a_ex += ex as usize;
        a_nm += nm as usize;
        if is_holdout_id(&e.id) {
            a_held += 1;
            a_held_nm += nm as usize;
        } else {
            a_dev += 1;
            a_dev_nm += nm as usize;
        }
    }

    // ---- Slice B: two-token collocations, per-position reconstruction ----
    // Position-split the cognate cells: languages whose primary variant also
    // has two tokens vote per position.
    let (mut b_total, mut b_gen, mut b_ex, mut b_nm) = (0usize, 0usize, 0usize, 0usize);
    let (mut b_dev, mut b_dev_nm, mut b_held, mut b_held_nm) = (0usize, 0usize, 0usize, 0usize);
    let mut b_miss: Vec<String> = Vec::new();
    for e in &multi {
        let toks: Vec<&str> = e.isv.split_whitespace().collect();
        if toks.len() != 2 || toks[1] == "sę" {
            continue;
        }
        b_total += 1;
        let mut cells1: HashMap<String, String> = HashMap::new();
        let mut cells2: HashMap<String, String> = HashMap::new();
        for (lang, cell) in &e.cells {
            // Any variant citing a two-token form votes — languages often list
            // a one-word synonym first and the collocation second.
            for (variant, _) in crate::normalize::split_cell(cell) {
                let t: Vec<&str> = variant.split_whitespace().collect();
                if t.len() == 2 {
                    cells1.insert(lang.clone(), t[0].to_string());
                    cells2.insert(lang.clone(), t[1].to_string());
                    break;
                }
            }
        }
        if cells1.len() < 2 {
            continue; // not generatable: fewer than 2 two-token cognates
        }
        b_gen += 1;
        let make_input = |cells: &HashMap<String, String>, pos: Pos| -> MeaningInput {
            let forms = consensus::source_forms_from_cells(cells, |code, form| {
                crate::enrich::english_source_url(form, Some(code))
            });
            let forms = consensus::lemma_forms(forms, pos);
            MeaningInput {
                pos,
                gender: e.noun_traits.gender,
                gloss: e.english.clone(),
                forms,
                is_intl_meaning: e.genesis.trim() == "I",
                reflexive: false,
            }
        };
        // Heuristic (disclosed): modifier + head — position 1 adjective agreed
        // with the head's gender, position 2 the entry's own POS.
        let in1 = make_input(&cells1, Pos::Adjective);
        let in2 = make_input(&cells2, e.pos);
        let top = |input: &MeaningInput| -> String {
            let (cands, _) = crate::pipeline::generate(input, proto.as_ref(), &cfg);
            cands.first().map(|c| c.form.clone()).unwrap_or_default()
        };
        let w1 = agree_adjective(&top(&in1), e.noun_traits.gender);
        let w2 = top(&in2);
        if w1.is_empty() || w2.is_empty() {
            b_gen -= 1; // a position produced no candidate: not generatable
            continue;
        }
        let pred = format!("{w1} {w2}");
        let ex = ortho::exact_match(&pred, e.isv.trim());
        let nm = ortho::normalized_match(&pred, e.isv.trim());
        b_ex += ex as usize;
        b_nm += nm as usize;
        if is_holdout_id(&e.id) {
            b_held += 1;
            b_held_nm += nm as usize;
        } else {
            b_dev += 1;
            b_dev_nm += nm as usize;
        }
        if !nm && b_miss.len() < 40 {
            b_miss.push(format!("{} → {}", e.isv.trim(), pred));
        }
    }

    // ---- Aspect pairs: same gloss, ipf vs pf, morphologically related ----
    let mut by_gloss_ipf: HashMap<&str, Vec<&OfficialEntry>> = HashMap::new();
    let mut by_gloss_pf: HashMap<&str, Vec<&OfficialEntry>> = HashMap::new();
    for e in &entries {
        let w = e.isv.trim();
        if w.is_empty() || w.contains(' ') || w.contains('#') {
            continue;
        }
        let g = e.english.trim();
        if g.is_empty() {
            continue;
        }
        if e.pos_raw.contains("ipf.") {
            by_gloss_ipf.entry(g).or_default().push(e);
        } else if e.pos_raw.contains("pf.") {
            by_gloss_pf.entry(g).or_default().push(e);
        }
    }
    let (mut p_gloss, mut p_morph) = (0usize, 0usize);
    let (mut p_both, mut p_one, mut p_neither) = (0usize, 0usize, 0usize);
    // Memoized per-entry correctness: an entry's pipeline result is independent
    // of its partner, and 1:1 greedy matching keeps hub lemmas from dominating
    // the metric (each entry participates in at most one pair per gloss).
    let mut score_memo: HashMap<String, bool> = HashMap::new();
    let mut glosses: Vec<&&str> = by_gloss_ipf.keys().collect();
    glosses.sort(); // deterministic pair selection
    for g in glosses {
        let ipfs = &by_gloss_ipf[*g];
        let Some(pfs) = by_gloss_pf.get(*g) else {
            continue;
        };
        p_gloss += ipfs.len().min(pfs.len());
        let mut used: Vec<bool> = vec![false; pfs.len()];
        for i in ipfs {
            let ki = ortho::consonant_key(&ortho::to_standard(&i.isv.to_lowercase()));
            let Some(qi) = pfs.iter().enumerate().position(|(x, q)| {
                if used[x] {
                    return false;
                }
                let kq = ortho::consonant_key(&ortho::to_standard(&q.isv.to_lowercase()));
                ki.ends_with(&kq) || kq.ends_with(&ki) || ortho::shares_consonant_root(&ki, &kq)
            }) else {
                continue;
            };
            used[qi] = true;
            let q = pfs[qi];
            p_morph += 1;
            let mut score = |e: &OfficialEntry| -> bool {
                if let Some(&v) = score_memo.get(&e.id) {
                    return v;
                }
                let input = build_input(e);
                let v = if input.forms.iter().any(|f| f.modern) {
                    let (cands, _) = crate::pipeline::generate(&input, proto.as_ref(), &cfg);
                    let pred = cands.first().map(|c| c.form.clone()).unwrap_or_default();
                    ortho::normalized_match(&pred, e.isv.trim())
                } else {
                    false
                };
                score_memo.insert(e.id.clone(), v);
                v
            };
            match (score(i), score(q)) {
                (true, true) => p_both += 1,
                (false, false) => p_neither += 1,
                _ => p_one += 1,
            }
        }
    }

    println!(
        "Multi-word inventory: {} lemmas — {} reflexive 'X sę', {} two-token, {} longer",
        multi.len(),
        n_sie,
        n_two,
        n_long
    );
    println!(
        "Slice A (reflexive sę): n={a_n}  exact {:.2}%  normalized {:.2}%  (dev {:.2}% / holdout {:.2}%)",
        pct(a_ex, a_n),
        pct(a_nm, a_n),
        pct(a_dev_nm, a_dev),
        pct(a_held_nm, a_held),
    );
    println!(
        "Slice B (two-token): {} lemmas, {} generatable (≥2 two-token cognates): exact {:.2}%  normalized {:.2}%  (dev {:.2}% / holdout {:.2}%)",
        b_total,
        b_gen,
        pct(b_ex, b_gen),
        pct(b_nm, b_gen),
        pct(b_dev_nm, b_dev),
        pct(b_held_nm, b_held),
    );
    println!(
        "Aspect pairs: {} gloss-matched, {} morphologically related; of those both correct {:.1}%, one {:.1}%, neither {:.1}%",
        p_gloss,
        p_morph,
        pct(p_both, p_morph),
        pct(p_one, p_morph),
        pct(p_neither, p_morph),
    );

    std::fs::create_dir_all(out_dir)?;
    let mut s = String::new();
    writeln!(s, "# Multi-word & aspect-pair benchmark (multiword-eval)\n")?;
    writeln!(
        s,
        "**Denominators:** {} multi-word official lemmas ({} reflexive `X sę`, {} two-token, {} longer — the headline benchmark excludes all of them); {} morphologically related 1:1 aspect pairs (of {} gloss-matched candidates). **Leakage story:** the gold `isv` only selects the slice; generation sees the cognate cells + POS/gender, as in the headline benchmark. **Dev/holdout (seeded id split, normalized, over the scored subsets):** reflexive {:.2}%/{:.2}%, two-token {:.2}%/{:.2}%.\n",
        multi.len(),
        n_sie,
        n_two,
        n_long,
        p_morph,
        p_gloss,
        pct(a_dev_nm, a_dev),
        pct(a_held_nm, a_held),
        pct(b_dev_nm, b_dev),
        pct(b_held_nm, b_held),
    )?;
    writeln!(s, "| Slice | n | exact | normalized |")?;
    writeln!(s, "|---|---:|---:|---:|")?;
    writeln!(
        s,
        "| reflexive `X sę` (existing pipeline, newly scored) | {} | {:.2}% | {:.2}% |",
        a_n,
        pct(a_ex, a_n),
        pct(a_nm, a_n)
    )?;
    writeln!(
        s,
        "| — of which no reflexive marker detected in the cognates (structural miss: ` sę` is never appended) | {} | — | — |",
        a_nodetect
    )?;
    writeln!(
        s,
        "| two-token collocation (per-position reconstruction) | {} of {} generatable | {:.2}% | {:.2}% |",
        b_gen,
        b_total,
        pct(b_ex, b_gen),
        pct(b_nm, b_gen)
    )?;
    writeln!(
        s,
        "\n## Aspect pairs (both members through the standard pipeline)\n\n| outcome | share of {} pairs |\n|---|---:|\n| both correct (normalized) | {:.1}% |\n| exactly one correct | {:.1}% |\n| neither | {:.1}% |",
        p_morph,
        pct(p_both, p_morph),
        pct(p_one, p_morph),
        pct(p_neither, p_morph),
    )?;
    writeln!(
        s,
        "\nThe two-token heuristic (disclosed): position 1 is reconstructed as an adjective and agreed with the head's gender, position 2 as the entry's own POS — right for the dominant modifier+head class, wrong for adv+adv or verb phrases; 'not generatable' means fewer than 2 cognates cite a two-token form.\n\n## Two-token nearest misses (sample)\n"
    )?;
    for m in &b_miss {
        writeln!(s, "- {}", m)?;
    }
    std::fs::write(out_dir.join("multiword-aspect.md"), s)?;
    println!("Wrote {}", out_dir.join("multiword-aspect.md").display());
    Ok(())
}

/// Dedicated perfective↔imperfective benchmark (issue #75).
///
/// Gold aspect/gloss/root data defines the evaluation pairs only. Each member
/// is first generated independently from its modern Slavic cognates. The pair
/// repair receives only those two generated candidates and their measured
/// scores; it never receives either official lemma.
pub fn run_aspect_eval(official_path: &Path, out_dir: &Path) -> Result<()> {
    use std::collections::BTreeMap;
    let entries = official::load(official_path)?;
    let pairs = crate::aspect::detect_pairs(&entries);
    let mut manifest = String::from("imperfective_id\tperfective_id\timperfective\tperfective\n");
    for pair in &pairs {
        let ipf = &entries[pair.imperfective];
        let pf = &entries[pair.perfective];
        writeln!(
            manifest,
            "{}\t{}\t{}\t{}",
            ipf.id,
            pf.id,
            ipf.isv.trim(),
            pf.isv.trim()
        )?;
    }
    let pair_hash = fnv1a(&manifest);
    const EXPECTED_PAIRS: usize = 1_440;
    const EXPECTED_MANIFEST_FNV: u64 = 0x5ab3_e19e_c5d7_58dd;
    anyhow::ensure!(
        pairs.len() == EXPECTED_PAIRS && pair_hash == EXPECTED_MANIFEST_FNV,
        "aspect-pair benchmark slice drifted: got {} pairs / {pair_hash:016x}, expected {EXPECTED_PAIRS} / {EXPECTED_MANIFEST_FNV:016x}; inspect and explicitly re-register target/eval/aspect-pairs.tsv",
        pairs.len(),
    );
    let proto = load_proto_index();
    let cfg = ConsensusConfig::production();
    let pct = |n: usize, d: usize| 100.0 * n as f64 / d.max(1) as f64;

    #[derive(Default)]
    struct Counts {
        n: usize,
        both: usize,
        either: usize,
        paired: usize,
    }
    let mut baseline = Counts::default();
    let mut suffix = Counts::default();
    let mut core = Counts::default();
    let mut model = Counts::default();
    let mut baseline_dev = Counts::default();
    let mut baseline_holdout = Counts::default();
    let mut suffix_dev = Counts::default();
    let mut suffix_holdout = Counts::default();
    let mut prefix_dev = Counts::default();
    let mut prefix_holdout = Counts::default();
    let mut secondary_dev = Counts::default();
    let mut secondary_holdout = Counts::default();
    let mut dev = Counts::default();
    let mut holdout = Counts::default();
    let mut fixed_both = 0usize;
    let mut broke_both = 0usize;
    let mut fixed_either = 0usize;
    let mut broke_either = 0usize;
    let mut rules: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut samples = Vec::new();
    let mut lexical_exceptions = 0usize;

    for pair in &pairs {
        let ipf = &entries[pair.imperfective];
        let pf = &entries[pair.perfective];
        let (ipf_input, pf_input) = (build_input(ipf), build_input(pf));
        let generate = |aspect_cfg| {
            crate::aspect::generate_pair(&ipf_input, &pf_input, proto.as_ref(), &cfg, aspect_cfg)
                .unwrap_or(crate::aspect::PairPrediction {
                    imperfective: String::new(),
                    perfective: String::new(),
                    rule: "missing-candidate",
                })
        };
        let independent = generate(crate::aspect::AspectConfig::baseline());
        let suffix_repaired = generate(crate::aspect::AspectConfig {
            suffix_repair: true,
            prefix_perfectivization: false,
            secondary_imperfectives: false,
        });
        let core_repaired = generate(crate::aspect::AspectConfig::production());
        let repaired = generate(crate::aspect::AspectConfig::with_secondary_imperfectives());
        // Exclude a lexical exception only when the production path actually
        // consumed the closed list. Gold membership alone is insufficient:
        // when neither generated candidate identifies the lexical pair, the
        // production export still emits an ordinary (and potentially wrong)
        // prediction, so the benchmark must score it.
        if core_repaired.rule == "closed-suppletive-pair" {
            lexical_exceptions += 1;
            continue;
        }
        let (ipf_base, pf_base) = (
            independent.imperfective.as_str(),
            independent.perfective.as_str(),
        );
        *rules.entry(core_repaired.rule).or_default() += 1;

        let score = |a: &str, b: &str, gi: &str, gp: &str| {
            let i = ortho::normalized_match(a, gi);
            let p = ortho::normalized_match(b, gp);
            (i && p, i || p, crate::aspect::pairing_correct(a, b))
        };
        let b = score(ipf_base, pf_base, ipf.isv.trim(), pf.isv.trim());
        let s = score(
            &suffix_repaired.imperfective,
            &suffix_repaired.perfective,
            ipf.isv.trim(),
            pf.isv.trim(),
        );
        let c = score(
            &core_repaired.imperfective,
            &core_repaired.perfective,
            ipf.isv.trim(),
            pf.isv.trim(),
        );
        let m = score(
            &repaired.imperfective,
            &repaired.perfective,
            ipf.isv.trim(),
            pf.isv.trim(),
        );
        let add = |c: &mut Counts, v: (bool, bool, bool)| {
            c.n += 1;
            c.both += v.0 as usize;
            c.either += v.1 as usize;
            c.paired += v.2 as usize;
        };
        add(&mut baseline, b);
        add(&mut suffix, s);
        add(&mut core, c);
        add(&mut model, m);
        let split_id = format!("{}:{}", ipf.id, pf.id);
        let held = is_holdout_id(&split_id);
        add(
            if held {
                &mut baseline_holdout
            } else {
                &mut baseline_dev
            },
            b,
        );
        add(
            if held {
                &mut suffix_holdout
            } else {
                &mut suffix_dev
            },
            s,
        );
        add(
            if held {
                &mut prefix_holdout
            } else {
                &mut prefix_dev
            },
            c,
        );
        add(
            if held {
                &mut secondary_holdout
            } else {
                &mut secondary_dev
            },
            m,
        );
        add(if held { &mut holdout } else { &mut dev }, c);
        match (b.0, c.0) {
            (false, true) => fixed_both += 1,
            (true, false) => broke_both += 1,
            _ => {}
        }
        match (b.1, c.1) {
            (false, true) => fixed_either += 1,
            (true, false) => broke_either += 1,
            _ => {}
        }
        if core_repaired.rule != "independent-roots-agree" && samples.len() < 40 {
            samples.push(format!(
                "{} ↔ {}: {} / {} → {} / {} ({})",
                ipf.isv.trim(),
                pf.isv.trim(),
                ipf_base,
                pf_base,
                core_repaired.imperfective,
                core_repaired.perfective,
                core_repaired.rule
            ));
        }
    }

    std::fs::create_dir_all(out_dir)?;
    std::fs::write(out_dir.join("aspect-pairs.tsv"), &manifest)?;
    let mut report = String::new();
    writeln!(report, "# Aspect-pair benchmark (aspect-eval)\n")?;
    writeln!(report, "**Frozen reproducible inventory:** {} deterministic 1:1 same-gloss, morphologically-related official ipf↔pf pairs (ordered manifest `aspect-pairs.tsv`, FNV-1a-64 `{pair_hash:016x}`). **Scored denominator:** {} regular pairs; {} closed suppletive predictions are excluded only when production actually fires the lexical rule, so unrecognized lexical pairs remain honest scored misses. **Keep metrics:** normalized both-correct (primary), normalized either-correct, and consonant-root fingerprint consistency. **Leakage:** official aspect/gloss/root spelling selects the evaluation slice only; both baseline forms are independently generated from cognate cells, and pair repair sees only those generated forms plus their scores. The shared seeded hash holds out {} scored pairs.\n", pairs.len(), baseline.n, lexical_exceptions, holdout.n)?;
    writeln!(
        report,
        "| model | n | normalized both correct | normalized either correct | fingerprint consistency |"
    )?;
    writeln!(report, "|---|---:|---:|---:|---:|")?;
    writeln!(
        report,
        "| independent baseline | {} | {:.2}% | {:.2}% | {:.2}% |",
        baseline.n,
        pct(baseline.both, baseline.n),
        pct(baseline.either, baseline.n),
        pct(baseline.paired, baseline.n)
    )?;
    writeln!(
        report,
        "| +core suffix repair | {} | {:.2}% | {:.2}% | {:.2}% |",
        suffix.n,
        pct(suffix.both, suffix.n),
        pct(suffix.either, suffix.n),
        pct(suffix.paired, suffix.n)
    )?;
    writeln!(
        report,
        "| +prefix perfectivization (production) | {} | {:.2}% | {:.2}% | {:.2}% |",
        core.n,
        pct(core.both, core.n),
        pct(core.either, core.n),
        pct(core.paired, core.n)
    )?;
    writeln!(
        report,
        "| +secondary imperfectives and -ovati→-ovyvati (experimental; holdout-flat) | {} | {:.2}% | {:.2}% | {:.2}% |",
        model.n,
        pct(model.both, model.n),
        pct(model.either, model.n),
        pct(model.paired, model.n)
    )?;
    let unrepaired = rules.get("unrepaired").copied().unwrap_or(0);
    writeln!(report, "\nThe secondary `-yva-/-iva-/-ava-` and `-ovati→-ovyvati` families are controlled by `AspectConfig.secondary_imperfectives`. They remain implemented but disabled in production because the rung is flat on holdout normalized both-correct. The production prefix repair improves the declared primary **normalized both-correct** metric with no breaks and improves consonant-root fingerprint consistency ({unrepaired} pairs remain unrepaired), but it lowers the secondary normalized either-correct metric. The `-ovati→-uje` present stem is exported and unit-tested grammar metadata, not part of this infinitive-pair accuracy metric; the paired table below discloses that tradeoff rather than relabeling it as a universal accuracy gain.\n")?;
    writeln!(report, "\n## Dev / holdout\n\n| model / split | n | normalized both correct | normalized either correct | fingerprint consistency |\n|---|---:|---:|---:|---:|\n| baseline dev | {} | {:.2}% | {:.2}% | {:.2}% |\n| baseline holdout | {} | {:.2}% | {:.2}% | {:.2}% |\n| suffix rung dev | {} | {:.2}% | {:.2}% | {:.2}% |\n| suffix rung holdout | {} | {:.2}% | {:.2}% | {:.2}% |\n| prefix rung dev | {} | {:.2}% | {:.2}% | {:.2}% |\n| prefix rung holdout | {} | {:.2}% | {:.2}% | {:.2}% |\n| secondary experimental dev | {} | {:.2}% | {:.2}% | {:.2}% |\n| secondary experimental holdout | {} | {:.2}% | {:.2}% | {:.2}% |\n| production dev | {} | {:.2}% | {:.2}% | {:.2}% |\n| production holdout | {} | {:.2}% | {:.2}% | {:.2}% |", baseline_dev.n, pct(baseline_dev.both, baseline_dev.n), pct(baseline_dev.either, baseline_dev.n), pct(baseline_dev.paired, baseline_dev.n), baseline_holdout.n, pct(baseline_holdout.both, baseline_holdout.n), pct(baseline_holdout.either, baseline_holdout.n), pct(baseline_holdout.paired, baseline_holdout.n), suffix_dev.n, pct(suffix_dev.both, suffix_dev.n), pct(suffix_dev.either, suffix_dev.n), pct(suffix_dev.paired, suffix_dev.n), suffix_holdout.n, pct(suffix_holdout.both, suffix_holdout.n), pct(suffix_holdout.either, suffix_holdout.n), pct(suffix_holdout.paired, suffix_holdout.n), prefix_dev.n, pct(prefix_dev.both, prefix_dev.n), pct(prefix_dev.either, prefix_dev.n), pct(prefix_dev.paired, prefix_dev.n), prefix_holdout.n, pct(prefix_holdout.both, prefix_holdout.n), pct(prefix_holdout.either, prefix_holdout.n), pct(prefix_holdout.paired, prefix_holdout.n), secondary_dev.n, pct(secondary_dev.both, secondary_dev.n), pct(secondary_dev.either, secondary_dev.n), pct(secondary_dev.paired, secondary_dev.n), secondary_holdout.n, pct(secondary_holdout.both, secondary_holdout.n), pct(secondary_holdout.either, secondary_holdout.n), pct(secondary_holdout.paired, secondary_holdout.n), dev.n, pct(dev.both, dev.n), pct(dev.either, dev.n), pct(dev.paired, dev.n), holdout.n, pct(holdout.both, holdout.n), pct(holdout.either, holdout.n), pct(holdout.paired, holdout.n))?;
    writeln!(report, "\n## Paired significance vs independent baseline\n\n| metric | fixed | broke | two-sided sign-test p |\n|---|---:|---:|---:|\n| normalized both correct | {fixed_both} | {broke_both} | {:.4} |\n| normalized either correct | {fixed_either} | {broke_either} | {:.4} |", sign_test_p(fixed_both, broke_both), sign_test_p(fixed_either, broke_either))?;
    writeln!(report, "\n## Rule census\n")?;
    for (rule, n) in &rules {
        writeln!(report, "- `{rule}`: {n}")?;
    }
    writeln!(report, "\n## Changed-pair sample\n")?;
    for sample in &samples {
        writeln!(report, "- {sample}")?;
    }
    let path = out_dir.join("aspect-pairs.md");
    std::fs::write(&path, report)?;
    println!("Aspect pairs: inventory {} / scored {}; baseline both {:.2}% / either {:.2}% / fingerprint {:.2}%; production both {:.2}% / either {:.2}% / fingerprint {:.2}%", pairs.len(), baseline.n, pct(baseline.both, baseline.n), pct(baseline.either, baseline.n), pct(baseline.paired, baseline.n), pct(core.both, core.n), pct(core.either, core.n), pct(core.paired, core.n));
    println!("Wrote {}", path.display());
    Ok(())
}

/// Agree a masculine-cited adjective with the head noun's gender (nom.sg):
/// hard -y → -a / -o, soft -i → -a / -e — the stem already carries the
/// softness (svěži→svěža, domašnji→domašnja; RULE_SPEC §3.2 O⇒E).
fn agree_adjective(masc: &str, gender: Option<crate::model::Gender>) -> String {
    use crate::model::Gender;
    let (fem, neut) = match masc.chars().last() {
        Some('y') => ("a", "o"),
        Some('i') => ("a", "e"),
        _ => return masc.to_string(),
    };
    let stem = &masc[..masc.len() - 1];
    match gender {
        Some(Gender::Feminine) => format!("{stem}{fem}"),
        Some(Gender::Neuter) => format!("{stem}{neut}"),
        _ => masc.to_string(),
    }
}

/// Representative-selection headroom (the rep-eval probe). Oracle-representative
/// showed ~+3.7pp is available from picking a better surface *within the winning
/// cluster* — and, unlike cluster choice, that is a mechanical (non-editorial)
/// decision a leakage-free rule might actually make. This measures how much of
/// that ceiling answer-blind rules recover: force the representative by a rule
/// (medoid / modal-skeleton / shortest) and score the real pipeline, vs the fixed
/// REP_PRIORITY (production) and the answer-reading oracle-representative.
pub fn run_rep_eval(official_path: &Path, out_dir: &Path) -> Result<()> {
    let entries: Vec<OfficialEntry> = official::load(official_path)?
        .into_iter()
        .filter(|e| e.is_benchmarkable())
        .collect();
    let proto = load_proto_index();
    let cfg = ConsensusConfig::production();

    let run_pass = |rule: &str| -> (usize, usize, usize) {
        let (mut ex, mut nm, mut denom) = (0usize, 0usize, 0usize);
        for entry in &entries {
            let input = build_input(entry);
            if !input.forms.iter().any(|f| f.modern) {
                continue;
            }
            denom += 1;
            let (cands, _) = if rule == "production" {
                crate::pipeline::generate(&input, proto.as_ref(), &cfg)
            } else {
                let oracle = consensus::Oracle {
                    official: &entry.isv,
                    cluster: false,
                    representative: rule == "oracle-representative",
                    proto_link: false,
                    force_cluster_key: None,
                    rep_rule: if rule == "oracle-representative" {
                        None
                    } else {
                        Some(rule)
                    },
                };
                crate::pipeline::generate_oracle(&input, proto.as_ref(), &cfg, Some(&oracle))
            };
            let pred = cands.first().map(|c| c.form.clone()).unwrap_or_default();
            ex += ortho::exact_match(&pred, &entry.isv) as usize;
            nm += ortho::normalized_match(&pred, &entry.isv) as usize;
        }
        (ex, nm, denom)
    };

    let rules = [
        "production",
        "medoid",
        "modal-skeleton",
        "shortest",
        "oracle-representative",
    ];
    let (base_ex, base_nm, denom) = run_pass("production");
    let pct = |a: usize| {
        if denom == 0 {
            0.0
        } else {
            100.0 * a as f32 / denom as f32
        }
    };
    println!(
        "Representative-selection headroom (leakage-free rules vs REP_PRIORITY vs oracle; {denom} meanings):"
    );
    println!(
        "  {:<22} {:>7}  {:>8}  {:>7}  {:>8}",
        "rule", "exact", "Δexact", "norm", "Δnorm"
    );
    let mut rows: Vec<(String, f32, f32, f32, f32)> = Vec::new();
    for rule in rules {
        let (ex, nm) = if rule == "production" {
            (base_ex, base_nm)
        } else {
            let (e, n, _) = run_pass(rule);
            (e, n)
        };
        let (dex, dnm) = (pct(ex) - pct(base_ex), pct(nm) - pct(base_nm));
        println!(
            "  {:<22} {:>6.2}%  {:>+7.2}  {:>6.2}%  {:>+7.2}",
            rule,
            pct(ex),
            dex,
            pct(nm),
            dnm
        );
        rows.push((rule.to_string(), pct(ex), dex, pct(nm), dnm));
    }

    std::fs::create_dir_all(out_dir)?;
    let mut s = String::new();
    writeln!(s, "# Representative-selection headroom (rep-eval)\n")?;
    writeln!(
        s,
        "Given the right cluster, which attested surface should represent it? This forces the winning group's representative by a **leakage-free** rule (except `oracle-representative`, which reads the answer as the ceiling) and scores the real pipeline over {denom} meanings.\n"
    )?;
    writeln!(s, "| Rule | exact | Δ exact | norm | Δ norm |")?;
    writeln!(s, "|---|---:|---:|---:|---:|")?;
    for (name, ex, dex, nm, dnm) in &rows {
        writeln!(
            s,
            "| {} | {:.2}% | {:+.2}pp | {:.2}% | {:+.2}pp |",
            name, ex, dex, nm, dnm
        )?;
    }
    writeln!(s, "\n- **production** — the fixed REP_PRIORITY (sl, hr, sr, pl, …) surface choice.\n- **medoid** — the group member minimizing total folded edit distance to the others (most central form).\n- **modal-skeleton** — the most common ascii-skeleton in the group, then REP_PRIORITY among its members.\n- **shortest** — the shortest attested form (nominatives tend shorter than oblique cases).\n- **oracle-representative** — the member folded-closest to the official lemma (ceiling; reads the answer).")?;
    std::fs::write(out_dir.join("rep-selection.md"), s)?;
    println!("Wrote {}", out_dir.join("rep-selection.md").display());
    Ok(())
}

/// Proto-engine-only benchmark (§A of the V3 plan). Isolates the Proto-Slavic
/// rule engine's accuracy from linking/ranking/consensus: for every meaning that
/// gets a confident proto link, derive the form straight from the reconstruction
/// and compare to the official lemma. Reports link coverage and proto-only
/// accuracy by POS so the engine rules can be iterated against a tight signal.
pub fn run_proto_engine(official_path: &Path, out_dir: &Path) -> Result<()> {
    let entries: Vec<OfficialEntry> = official::load(official_path)?
        .into_iter()
        .filter(|e| e.is_benchmarkable())
        .collect();
    let Some(proto) = load_proto_index() else {
        anyhow::bail!(
            "no Proto-Slavic cache ({}); run `extract-proto` first.",
            crate::DEFAULT_PROTO_CACHE
        );
    };

    let (mut n, mut linked, mut exact, mut norm) = (0usize, 0usize, 0usize, 0usize);
    let mut by_pos: BTreeMap<&'static str, (usize, usize, usize)> = BTreeMap::new(); // (linked, exact, norm)
    let mut errors: Vec<(String, String, String, String, f32)> = Vec::new(); // gloss, official, proto_form, proto_word, conf

    for entry in &entries {
        let input = build_input(entry);
        if !input.forms.iter().any(|f| f.modern) {
            continue;
        }
        n += 1;
        // Direct links only: this benchmark isolates the derivation engine, so it
        // derives the bare entry word without prefix re-attachment and without
        // the deep-corroboration rescue.
        let Some(l) = crate::proto_link::link(&proto, &input, false, false) else {
            continue;
        };
        linked += 1;
        let recon_key = ortho::consonant_key(&ortho::to_standard(&l.entry.word.to_lowercase()));
        let reflexes: Vec<String> = input
            .forms
            .iter()
            .filter(|f| f.modern)
            .map(|f| f.norm.latin.clone())
            .filter(|r| ortho::shares_consonant_root(&ortho::consonant_key(r), &recon_key))
            .collect();
        let form = crate::proto::generate_with_reflexes(
            &l.entry.word,
            input.pos,
            input.gender,
            &reflexes,
            l.entry.stem_class.as_deref(),
        )
        .form;
        let e = ortho::exact_match(&form, &entry.isv);
        let nm = ortho::normalized_match(&form, &entry.isv);
        exact += e as usize;
        norm += nm as usize;
        let bp = by_pos.entry(entry.pos.code()).or_default();
        bp.0 += 1;
        bp.1 += e as usize;
        bp.2 += nm as usize;
        if !nm {
            errors.push((
                entry.english.clone(),
                entry.isv.clone(),
                form,
                l.entry.word.clone(),
                l.confidence,
            ));
        }
    }

    let rate = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f32 / b as f32
        }
    };
    println!(
        "Proto-engine benchmark: {} linked / {} ({:.1}% coverage); on linked: exact {:.2}%, normalized {:.2}%",
        linked,
        n,
        rate(linked, n),
        rate(exact, linked),
        rate(norm, linked),
    );

    std::fs::create_dir_all(out_dir)?;
    let mut s = String::new();
    writeln!(s, "# Proto-Slavic engine benchmark\n")?;
    writeln!(
        s,
        "Isolates `proto::generate_with_reflexes` from linking/ranking/consensus: derive the form straight from the linked reconstruction and compare to the official lemma.\n"
    )?;
    writeln!(
        s,
        "- Benchmark entries with modern evidence: **{}**\n- Confidently linked to a Proto-Slavic entry: **{}** ({:.1}% coverage)\n- On the linked subset: **exact {:.2}%**, **normalized {:.2}%**\n",
        n,
        linked,
        rate(linked, n),
        rate(exact, linked),
        rate(norm, linked),
    )?;
    writeln!(s, "## Proto-engine accuracy by POS (linked subset)\n")?;
    writeln!(s, "| POS | linked | exact | normalized |")?;
    writeln!(s, "|---|---:|---:|---:|")?;
    for (pos, (ln, ex, nm)) in &by_pos {
        writeln!(
            s,
            "| {} | {} | {:.2}% | {:.2}% |",
            pos,
            ln,
            rate(*ex, *ln),
            rate(*nm, *ln)
        )?;
    }
    errors.sort_by(|a, b| b.4.total_cmp(&a.4)); // most-confident errors first (most actionable)
    writeln!(s, "\n## Confident proto-engine errors (sample)\n")?;
    writeln!(
        s,
        "| gloss | official | proto form | *reconstruction | link conf |"
    )?;
    writeln!(s, "|---|---|---|---|---:|")?;
    for (g, off, form, word, conf) in errors.iter().take(60) {
        writeln!(
            s,
            "| {} | {} | {} | *{} | {:.2} |",
            g.replace('|', "/"),
            off,
            form,
            word,
            conf
        )?;
    }
    std::fs::write(out_dir.join("proto-engine-report.md"), s)?;
    println!("Wrote {}", out_dir.join("proto-engine-report.md").display());
    Ok(())
}

/// Print the generator's full reasoning for one word/gloss (manual spot-check).
pub fn explain(official_path: &Path, query: &str) -> Result<()> {
    let entries = official::load(official_path)?;
    let ql = query.trim().to_lowercase();
    let entry = entries
        .iter()
        .find(|e| e.isv.to_lowercase() == ql)
        .or_else(|| {
            // Folded match, so a query without the flavored letters still finds
            // the lemma: "kratky" → kråtky, "medzu" → medžu.
            let qs = ortho::to_standard(&ql);
            let qk = ortho::ascii_skeleton(&ql);
            entries
                .iter()
                .find(|e| ortho::to_standard(&e.isv.to_lowercase()) == qs)
                .or_else(|| {
                    entries
                        .iter()
                        .find(|e| !qk.is_empty() && ortho::ascii_skeleton(&e.isv) == qk)
                })
        })
        .or_else(|| {
            entries.iter().find(|e| {
                e.english
                    .to_lowercase()
                    .split(&[',', ';'][..])
                    .any(|g| g.trim() == ql)
            })
        })
        .or_else(|| {
            entries
                .iter()
                .find(|e| e.english.to_lowercase().contains(&ql))
        });

    let Some(entry) = entry else {
        println!("No official entry found matching '{query}'.");
        return Ok(());
    };

    let input = build_input(entry);
    let overrides = crate::overrides::Overrides::load(Path::new(crate::DEFAULT_OVERRIDES));
    let cfg = crate::consensus::ConsensusConfig::production();
    let proto = load_proto_index();
    let gen =
        crate::generator::generate(&input, Some(&entry.isv), proto.as_ref(), &cfg, &overrides);
    if let Some(r) = &gen.reconstruction {
        println!(
            "Reconstruction: *{} (link conf {:.2})",
            r.word, r.confidence
        );
    }

    println!("Gloss:    {}", entry.english);
    println!("POS:      {} ({})", entry.pos.code(), entry.pos_raw);
    println!("Official: {}", entry.isv);
    println!(
        "Status:   {:?} ({})",
        gen.match_status,
        gen.match_status.label()
    );
    println!("\nEvidence by branch:");
    for f in &input.forms {
        println!(
            "  [{}] {:<3} {:<18} -> {}",
            f.branch.code().chars().next().unwrap().to_uppercase(),
            f.lang_code,
            f.norm.original,
            f.norm.latin
        );
    }
    println!("\nRanked candidates:");
    for (i, c) in gen.candidates.iter().enumerate().take(5) {
        println!(
            "  {}. {:<20} score {:.3}  conf {:<7} branches {}  [{}]",
            i + 1,
            c.form,
            c.score,
            c.confidence.label(),
            c.branch_coverage,
            c.source.label()
        );
        for step in &c.trace {
            println!(
                "       · {}: {} -> {} ({})",
                step.id, step.before, step.after, step.explanation
            );
        }
        for w in &c.warnings {
            println!("       ! {w}");
        }
    }
    Ok(())
}

pub fn run(official_path: &Path, out_dir: &Path) -> Result<()> {
    let mut entries_all = official::load(official_path)?;
    // The metadata TSV has no per-language translations, so the consensus
    // benchmark is impossible from it. Fall back to the bundled full export.
    let with_cells = entries_all.iter().filter(|e| !e.cells.is_empty()).count();
    if with_cells < 100 {
        let fallback = Path::new(crate::DEFAULT_OFFICIAL);
        if fallback != official_path && fallback.exists() {
            eprintln!(
                "note: {} has no per-language translations; using {} for the consensus benchmark.",
                official_path.display(),
                fallback.display()
            );
            entries_all = official::load(fallback)?;
        }
    }
    let entries: Vec<OfficialEntry> = entries_all
        .into_iter()
        .filter(|e| e.is_benchmarkable())
        .collect();
    println!(
        "Loaded {} benchmarkable official entries from {}",
        entries.len(),
        official_path.display()
    );

    // Load the Proto-Slavic cache if present; the +proto-derived rung needs it.
    let proto_index = load_proto_index();
    if proto_index.is_some() {
        println!("Loaded Proto-Slavic cache for the proto-derived rung.");
    } else {
        println!(
            "note: no Proto-Slavic cache ({}); run `extract-proto` to enable the proto-derived rung.",
            crate::DEFAULT_PROTO_CACHE
        );
    }
    let proto = proto_index.as_ref();

    let kept = kept_ladder();
    let runs: Vec<RunMetrics> = kept
        .iter()
        .map(|r| evaluate_config(&entries, r, proto))
        .collect();
    let rejected: Vec<RunMetrics> = rejected_experiments()
        .iter()
        .map(|r| evaluate_config(&entries, r, proto))
        .collect();

    println!("Kept ladder (cumulative):");
    for r in &runs {
        println!(
            "  {:<22} exact {:>6.2}%  norm {:>6.2}%  top3 {:>6.2}%  edit {:.3}",
            r.name,
            100.0 * Bucket::rate(r.exact, r.n),
            100.0 * Bucket::rate(r.normalized, r.n),
            100.0 * Bucket::rate(r.top3, r.n),
            r.sum_norm_edit / r.n.max(1) as f32,
        );
    }
    println!("Rejected experiments (production + one rule, deltas negative):");
    for r in &rejected {
        println!(
            "  {:<22} exact {:>6.2}%  norm {:>6.2}%",
            r.name,
            100.0 * Bucket::rate(r.exact, r.n),
            100.0 * Bucket::rate(r.normalized, r.n),
        );
    }

    std::fs::create_dir_all(out_dir)?;
    let baseline = &runs[0];
    // The shipped config is the LAST rung of the ladder (which is defined to end
    // exactly at `ConsensusConfig::production`). The Headline and the CI floor
    // must report *that* rung — not the empirical best — so a production rule
    // that regresses the final rung below an earlier one cannot slip past CI.
    debug_assert_eq!(
        kept_ladder().last().map(|r| r.cfg),
        Some(ConsensusConfig::production()),
        "the kept ladder must end at ConsensusConfig::production()"
    );
    let production = runs.last().unwrap();
    println!("Shipped production config: {}", production.name);

    // Still surface if some earlier rung actually scored higher (a real regression).
    if let Some(better) = runs
        .iter()
        .find(|r| Bucket::rate(r.exact, r.n) > Bucket::rate(production.exact, production.n) + 1e-6)
    {
        println!(
            "WARNING: rung '{}' (exact {:.2}%) outscores the shipped production rung (exact {:.2}%) — production regressed.",
            better.name,
            100.0 * Bucket::rate(better.exact, better.n),
            100.0 * Bucket::rate(production.exact, production.n),
        );
    }

    write_summary_json(out_dir, &runs)?;
    write_report_md(out_dir, &runs, &rejected, production)?;
    write_diffs(out_dir, baseline, production)?;
    write_errors_sample(out_dir, production)?;
    write_methodology(out_dir, &runs)?;

    // Overfitting guard: the production config on the seeded holdout split.
    let sp = split_rates(&production.results);
    println!(
        "Overfitting guard (seeded 75/25 split, {} held out): exact dev {:.2}% / holdout {:.2}% (gap {:+.2}pp), norm dev {:.2}% / holdout {:.2}% (gap {:+.2}pp)",
        sp.held_n,
        sp.dev_exact,
        sp.held_exact,
        sp.dev_exact - sp.held_exact,
        sp.dev_norm,
        sp.held_norm,
        sp.dev_norm - sp.held_norm,
    );

    println!("Wrote benchmark report to {}", out_dir.display());
    println!(
        "Headline: normalized top-1 {:.2}% (baseline {:.2}%), exact top-1 {:.2}% (baseline {:.2}%)",
        100.0 * Bucket::rate(production.normalized, production.n),
        100.0 * Bucket::rate(baseline.normalized, baseline.n),
        100.0 * Bucket::rate(production.exact, production.n),
        100.0 * Bucket::rate(baseline.exact, baseline.n),
    );
    Ok(())
}

fn write_summary_json(out_dir: &Path, runs: &[RunMetrics]) -> Result<()> {
    let mut arr = Vec::new();
    for r in runs {
        let by_pos: BTreeMap<String, serde_json::Value> = r
            .by_pos
            .iter()
            .map(|(k, b)| {
                (
                    k.to_string(),
                    serde_json::json!({
                        "n": b.n,
                        "exact": Bucket::rate(b.exact, b.n),
                        "normalized": Bucket::rate(b.normalized, b.n),
                    }),
                )
            })
            .collect();
        let by_branch: Vec<serde_json::Value> = r
            .by_branch
            .iter()
            .enumerate()
            .map(|(i, b)| {
                serde_json::json!({
                    "branch_coverage": i,
                    "n": b.n,
                    "exact": Bucket::rate(b.exact, b.n),
                    "normalized": Bucket::rate(b.normalized, b.n),
                })
            })
            .collect();
        let by_conf: BTreeMap<String, serde_json::Value> = r
            .by_conf
            .iter()
            .map(|(k, b)| {
                (
                    k.to_string(),
                    serde_json::json!({
                        "n": b.n,
                        "normalized": Bucket::rate(b.normalized, b.n),
                    }),
                )
            })
            .collect();
        arr.push(serde_json::json!({
            "name": r.name,
            "description": r.description,
            "n": r.n,
            "exact_top1": Bucket::rate(r.exact, r.n),
            "normalized_top1": Bucket::rate(r.normalized, r.n),
            "skeleton_top1": Bucket::rate(r.skeleton, r.n),
            "normalized_top3": Bucket::rate(r.top3, r.n),
            "normalized_top5": Bucket::rate(r.top5, r.n),
            "mean_normalized_edit_distance": r.sum_norm_edit / r.n.max(1) as f32,
            "by_pos": by_pos,
            "by_branch_coverage": by_branch,
            "by_confidence": by_conf,
        }));
    }
    let doc = serde_json::json!({ "runs": arr });
    std::fs::write(
        out_dir.join("candidate-generation-summary.json"),
        serde_json::to_string_pretty(&doc)?,
    )?;
    Ok(())
}

fn write_report_md(
    out_dir: &Path,
    runs: &[RunMetrics],
    rejected: &[RunMetrics],
    best: &RunMetrics,
) -> Result<()> {
    let baseline = &runs[0];
    let mut s = String::new();
    writeln!(s, "# Candidate-generation benchmark\n")?;
    writeln!(
        s,
        "Benchmark: reconstruct the official Interslavic lemma from the modern Slavic cognates in the official dictionary, **without showing the generator the answer**. Evaluated on **{}** benchmarkable single-word entries. Every rule is kept only if it improved measured accuracy.\n",
        baseline.n
    )?;
    writeln!(
        s,
        "- **Metrics.** *exact*: identical to the official flavored lemma; *normalized*: identical after reducing both to the standard alphabet (§1.3); *skeleton*: identical after an ASCII fold; *top-3/5*: any of the first N candidates matches (normalized); *mean edit*: mean normalized Levenshtein distance to the official lemma.\n"
    )?;
    writeln!(s, "## Kept rules — cumulative ablation ladder\n")?;
    writeln!(
        s,
        "Each rung adds exactly one rule to the previous, so its accuracy delta is attributable. The last rung is the kept **production** configuration.\n"
    )?;
    writeln!(
        s,
        "| Rung | exact top-1 | norm top-1 | Δ norm | top-3 | mean edit |"
    )?;
    writeln!(s, "|---|---:|---:|---:|---:|---:|")?;
    let mut prev_norm = Bucket::rate(baseline.normalized, baseline.n);
    for r in runs {
        let norm = Bucket::rate(r.normalized, r.n);
        let delta = norm - prev_norm;
        writeln!(
            s,
            "| {} | {:.2}% | {:.2}% | {:+.2} pp | {:.2}% | {:.3} |",
            r.name,
            100.0 * Bucket::rate(r.exact, r.n),
            100.0 * norm,
            100.0 * delta,
            100.0 * Bucket::rate(r.top3, r.n),
            r.sum_norm_edit / r.n.max(1) as f32,
        )?;
        prev_norm = norm;
    }
    writeln!(s)?;
    for r in runs {
        writeln!(s, "- **{}** — {}", r.name, r.description)?;
    }

    writeln!(s, "\n## Rejected rules — tested and reverted\n")?;
    writeln!(
        s,
        "Each is the production config plus one experimental rule. All regress accuracy on the benchmark and are therefore **not** in the production config, per the keep-only-if-it-improves rule.\n"
    )?;
    let prod_norm = Bucket::rate(best.normalized, best.n);
    let prod_exact = Bucket::rate(best.exact, best.n);
    writeln!(
        s,
        "| Experiment | exact top-1 | Δ exact | norm top-1 | Δ norm |"
    )?;
    writeln!(s, "|---|---:|---:|---:|---:|")?;
    for r in rejected {
        writeln!(
            s,
            "| {} | {:.2}% | {:+.2} pp | {:.2}% | {:+.2} pp |",
            r.name,
            100.0 * Bucket::rate(r.exact, r.n),
            100.0 * (Bucket::rate(r.exact, r.n) - prod_exact),
            100.0 * Bucket::rate(r.normalized, r.n),
            100.0 * (Bucket::rate(r.normalized, r.n) - prod_norm),
        )?;
    }
    writeln!(s)?;
    for r in rejected {
        writeln!(s, "- **{}** — {}", r.name, r.description)?;
    }

    writeln!(s, "\n## POS-specific accuracy (final config)\n")?;
    writeln!(s, "| POS | n | exact | normalized |")?;
    writeln!(s, "|---|---:|---:|---:|")?;
    for (pos, b) in &best.by_pos {
        writeln!(
            s,
            "| {} | {} | {:.2}% | {:.2}% |",
            pos,
            b.n,
            100.0 * Bucket::rate(b.exact, b.n),
            100.0 * Bucket::rate(b.normalized, b.n)
        )?;
    }

    writeln!(s, "\n## Branch coverage vs accuracy (final config)\n")?;
    writeln!(s, "| branches with the consensus form | n | normalized |")?;
    writeln!(s, "|---:|---:|---:|")?;
    for (i, b) in best.by_branch.iter().enumerate() {
        writeln!(
            s,
            "| {} | {} | {:.2}% |",
            i,
            b.n,
            100.0 * Bucket::rate(b.normalized, b.n)
        )?;
    }

    writeln!(s, "\n## Confidence calibration (final config)\n")?;
    writeln!(
        s,
        "High-confidence candidates should match the official dictionary more often than low-confidence ones.\n"
    )?;
    writeln!(s, "| confidence | n | normalized match |")?;
    writeln!(s, "|---|---:|---:|")?;
    for label in ["high", "medium", "low"] {
        if let Some(b) = best.by_conf.get(label) {
            writeln!(
                s,
                "| {} | {} | {:.2}% |",
                label,
                b.n,
                100.0 * Bucket::rate(b.normalized, b.n)
            )?;
        }
    }

    writeln!(s, "\n## Before / after\n")?;
    writeln!(
        s,
        "- Baseline normalized top-1: **{:.2}%**\n- Final normalized top-1: **{:.2}%** ({:+.2} pp)\n- Baseline exact top-1: **{:.2}%**\n- Final exact top-1: **{:.2}%** ({:+.2} pp)",
        100.0 * Bucket::rate(baseline.normalized, baseline.n),
        100.0 * Bucket::rate(best.normalized, best.n),
        100.0 * (Bucket::rate(best.normalized, best.n) - Bucket::rate(baseline.normalized, baseline.n)),
        100.0 * Bucket::rate(baseline.exact, baseline.n),
        100.0 * Bucket::rate(best.exact, best.n),
        100.0 * (Bucket::rate(best.exact, best.n) - Bucket::rate(baseline.exact, baseline.n)),
    )?;

    // Remaining systematic errors: classify the misses by a cheap heuristic so
    // the largest remaining buckets are visible.
    writeln!(s, "\n## Remaining systematic errors (final config)\n")?;
    let misses: Vec<&EntryResult> = best.results.iter().filter(|r| !r.normalized).collect();
    let total_miss = misses.len();
    let near = misses.iter().filter(|r| r.norm_edit < 0.20).count();
    let far = total_miss - near;
    let mut by_cause: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &misses {
        *by_cause.entry(classify_error(r)).or_default() += 1;
    }
    writeln!(
        s,
        "Of **{}** misses, **{}** ({:.0}%) are near-misses (normalized edit < 0.20 — an ending/one-letter fix) and **{}** are farther (usually a different root chosen by Interslavic).\n",
        total_miss,
        near,
        100.0 * near as f32 / total_miss.max(1) as f32,
        far
    )?;
    let mut causes: Vec<(&&str, &usize)> = by_cause.iter().collect();
    causes.sort_by(|a, b| b.1.cmp(a.1));
    writeln!(s, "| Error class | count | share of misses |")?;
    writeln!(s, "|---|---:|---:|")?;
    for (cause, n) in causes {
        writeln!(
            s,
            "| {} | {} | {:.1}% |",
            cause,
            n,
            100.0 * (*n as f32) / total_miss.max(1) as f32
        )?;
    }

    writeln!(s, "\n## Next recommended linguistic rules\n")?;
    writeln!(
        s,
        "The Proto-Slavic-derived-form path (§4.4) is implemented — consensus picks the root and the Proto-Slavic rule engine supplies the flavored form via a leakage-free descendant+gloss link. Yer resolution now uses a genuine **tense-yer rule** (yer before *j → i/y) plus **reflex-guided vocalization** (a lexically-ambiguous weak yer is retained when the reflexes vote to keep it: `*pьsati`→`pisati` vs `*bьrati`→`brati`), and a length-free **reflex-shape-agreement** ranking rule replaced the earlier length heuristic. Ranked next steps, from the remaining-error analysis:\n\n1. **Expand Proto-Slavic link coverage.** Only meanings with a matched `sla-pro` reconstruction get the flavored derivation; raising cache coverage and loosening the link gate (without admitting bad links) directly grows the proto-derived slice.\n2. **Reduce the reconstruction's non-yer errors** (endings, palatalizations) so the proto form can be trusted even when it disagrees with the reflexes — currently such disagreements defer to the reflexes, capping the proto gain.\n3. **Divergent-root modeling (semantic families, §4.2 step 3).** The ~{far} far-misses are mostly cases where Interslavic picked a different root than the plurality skeleton; scoring candidate *roots* (not surface forms) over the six subgroups, clustered by the proto descendant graph, would recover many.\n4. **Secondary-imperfective verb stems** (`-yva-/-iva-/-ava-`) and the agentive `-telj`/abstract `-teljstvo` suffixes, seen repeatedly in the verb/noun error tail.\n5. **POS-specific gender/animacy inference** to pick the right nominal ending where the modern citation forms disagree.",
        far = far
    )?;

    std::fs::write(out_dir.join("candidate-generation-report.md"), s)?;
    Ok(())
}

/// Cheap heuristic bucketing of a miss into a systematic-error class.
fn classify_error(r: &EntryResult) -> &'static str {
    let off = &r.isv;
    let pred = &r.predicted;
    if pred.is_empty() {
        return "no candidate produced";
    }
    if r.norm_edit >= 0.34 {
        return "different root / derivation";
    }
    let so = ortho::to_standard(&off.to_lowercase());
    let sp = ortho::to_standard(&pred.to_lowercase());
    // Same skeleton but different flavored letters => a flavor-recovery miss.
    if ortho::ascii_skeleton(off) == ortho::ascii_skeleton(pred) {
        return "flavored letter (ě/ę/ų/å/ć/đ) not recovered";
    }
    if so.len() != sp.len() {
        if so.chars().count() > sp.chars().count() {
            return "missing letter (fleeting vowel / cluster)";
        }
        return "extra letter (epenthesis / ending)";
    }
    if off.contains('y') != pred.contains('y') || off.contains('i') != pred.contains('i') {
        return "y / i distinction";
    }
    "single-letter substitution"
}

fn write_diffs(out_dir: &Path, baseline: &RunMetrics, best: &RunMetrics) -> Result<()> {
    let base_map: BTreeMap<&str, &EntryResult> = baseline
        .results
        .iter()
        .map(|r| (r.id.as_str(), r))
        .collect();
    let mut regressions = String::from("id,gloss,pos,official,baseline_pred,final_pred\n");
    let mut improvements = String::from("id,gloss,pos,official,baseline_pred,final_pred\n");
    for r in &best.results {
        let Some(b) = base_map.get(r.id.as_str()) else {
            continue;
        };
        if b.normalized && !r.normalized {
            writeln!(
                regressions,
                "{},{},{},{},{},{}",
                r.id,
                csv_escape(&r.gloss),
                r.pos.code(),
                csv_escape(&r.isv),
                csv_escape(&b.predicted),
                csv_escape(&r.predicted)
            )?;
        }
        if !b.normalized && r.normalized {
            writeln!(
                improvements,
                "{},{},{},{},{},{}",
                r.id,
                csv_escape(&r.gloss),
                r.pos.code(),
                csv_escape(&r.isv),
                csv_escape(&b.predicted),
                csv_escape(&r.predicted)
            )?;
        }
    }
    std::fs::write(out_dir.join("regressions.csv"), regressions)?;
    std::fs::write(out_dir.join("improvements.csv"), improvements)?;
    Ok(())
}

fn write_errors_sample(out_dir: &Path, best: &RunMetrics) -> Result<()> {
    let mut errors: Vec<&EntryResult> = best.results.iter().filter(|r| !r.normalized).collect();
    // Sort by closeness (largest edit distance last) so the sample surfaces the
    // near-misses first, which are the most actionable.
    errors.sort_by(|a, b| a.norm_edit.total_cmp(&b.norm_edit));
    let mut s = String::from("id,gloss,pos,official,predicted,norm_edit,branch_cov,n_langs\n");
    for r in errors.iter().take(400) {
        writeln!(
            s,
            "{},{},{},{},{},{:.3},{},{}",
            r.id,
            csv_escape(&r.gloss),
            r.pos.code(),
            csv_escape(&r.isv),
            csv_escape(&r.predicted),
            r.norm_edit,
            r.branch_cov,
            r.n_langs
        )?;
    }
    std::fs::write(out_dir.join("errors-sample.csv"), s)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Evaluation-methodology instruments: overfitting guard (seeded holdout split),
// paired significance for ladder rungs, bootstrap confidence intervals, and
// score calibration. All deterministic (seeded, no system RNG) so the report is
// reproducible byte-for-byte.
// ---------------------------------------------------------------------------

/// Deterministic FNV-1a hash of an entry id, used for the seeded dev/holdout
/// split. Stable across runs, platforms and rule changes (depends only on the
/// entry's id string), so the same entries are held out forever.
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// ~25% of entries form the HOLDOUT split; rules are developed against the DEV
/// split and must generalize to the holdout. A rule that gains on dev but not on
/// holdout is memorizing dictionary idiosyncrasies (overfitting guard).
/// The ONE seeded dev/holdout split, shared by every benchmark (evaluate,
/// derive-eval, …) so all dev/holdout numbers are computed on the same
/// entries. Never change the hash or the modulus.
pub fn is_holdout_id(id: &str) -> bool {
    fnv1a(id).is_multiple_of(4)
}

/// Exact two-sided sign test on discordant pairs. Under H0 the smaller tail is
/// Binomial(n, 1/2); compute its PMF at `m=min(fixed,broke)` in log space, then
/// recur downward. This stays stable for both tiny and large ladder deltas.
fn sign_test_p(fixed: usize, broke: usize) -> f64 {
    let n = fixed + broke;
    if n == 0 {
        return 1.0;
    }
    let m = fixed.min(broke);
    let log_choose = (1..=m).fold(0.0, |acc, i| {
        acc + ((n + 1 - i) as f64).ln() - (i as f64).ln()
    });
    let mut p = (log_choose - n as f64 * std::f64::consts::LN_2).exp();
    let mut tail = p;
    for k in (1..=m).rev() {
        p *= k as f64 / (n - k + 1) as f64;
        tail += p;
    }
    (2.0 * tail).min(1.0)
}

/// Deterministic xorshift64* PRNG for the bootstrap (seeded, reproducible).
struct XorShift64(u64);
impl XorShift64 {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Percentile-bootstrap 95% CI (1000 resamples, seeded) for a hit rate over
/// per-entry booleans. Returns (low%, high%).
fn bootstrap_ci(hits: &[bool]) -> (f32, f32) {
    let n = hits.len();
    if n == 0 {
        return (0.0, 0.0);
    }
    let mut rng = XorShift64(0x9e37_79b9_7f4a_7c15);
    let mut rates: Vec<f32> = (0..1000)
        .map(|_| {
            let mut hit = 0usize;
            for _ in 0..n {
                hit += hits[rng.below(n)] as usize;
            }
            100.0 * hit as f32 / n as f32
        })
        .collect();
    rates.sort_by(|a, b| a.total_cmp(b));
    (rates[25], rates[974])
}

struct SplitRates {
    dev_exact: f32,
    dev_norm: f32,
    held_exact: f32,
    held_norm: f32,
    held_n: usize,
}

fn split_rates(results: &[EntryResult]) -> SplitRates {
    let (mut de, mut dn, mut dd) = (0usize, 0usize, 0usize);
    let (mut he, mut hn, mut hd) = (0usize, 0usize, 0usize);
    for r in results {
        if is_holdout_id(&r.id) {
            hd += 1;
            he += r.exact as usize;
            hn += r.normalized as usize;
        } else {
            dd += 1;
            de += r.exact as usize;
            dn += r.normalized as usize;
        }
    }
    let pct = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f32 / b as f32
        }
    };
    SplitRates {
        dev_exact: pct(de, dd),
        dev_norm: pct(dn, dd),
        held_exact: pct(he, hd),
        held_norm: pct(hn, hd),
        held_n: hd,
    }
}

/// The methodology report: per-rung dev-vs-holdout generalization, paired
/// significance of each rung's delta, bootstrap CI on the headline, and the
/// score-calibration table (reliability, ECE, Brier).
fn write_methodology(out_dir: &Path, runs: &[RunMetrics]) -> Result<()> {
    let production = runs.last().unwrap();
    let mut s = String::new();
    writeln!(s, "# Evaluation methodology — statistical instruments\n")?;

    // ---- 1. Overfitting guard: seeded 75/25 dev/holdout split ----
    let prod_split = split_rates(&production.results);
    writeln!(s, "## Overfitting guard — seeded 75/25 dev/holdout split\n")?;
    writeln!(
        s,
        "Entries are split by a deterministic hash of their dictionary id (~25% held out, **{}** entries; the split never changes). Rules are developed against dev; a kept rule must not gain on dev while flat/negative on holdout — that gap is the overfitting signal. The dev−holdout gap for the production config should stay within the holdout's sampling noise (±~1pp).\n",
        prod_split.held_n
    )?;
    writeln!(
        s,
        "| Rung | exact dev | exact holdout | gap | norm dev | norm holdout | gap |"
    )?;
    writeln!(s, "|---|---:|---:|---:|---:|---:|---:|")?;
    for r in runs {
        let sp = split_rates(&r.results);
        writeln!(
            s,
            "| {} | {:.2}% | {:.2}% | {:+.2} | {:.2}% | {:.2}% | {:+.2} |",
            r.name,
            sp.dev_exact,
            sp.held_exact,
            sp.dev_exact - sp.held_exact,
            sp.dev_norm,
            sp.held_norm,
            sp.dev_norm - sp.held_norm,
        )?;
    }

    // ---- 2. Paired significance of each ladder rung ----
    writeln!(s, "\n## Ladder-rung significance (paired sign test)\n")?;
    writeln!(
        s,
        "Each rung vs the previous rung, paired per entry: `fixed` = newly matched, `broke` = newly missed, on the **exact** metric (the primary keep-metric) and the normalized metric. p is the two-sided sign test on the discordant pairs — a rung whose p ≳ 0.05 on its keep-metric is not distinguishable from noise on this benchmark and should be treated as provisional, not proven.\n"
    )?;
    writeln!(
        s,
        "| Rung | Δ exact | fixed/broke (exact) | p (exact) | Δ norm | fixed/broke (norm) | p (norm) |"
    )?;
    writeln!(s, "|---|---:|---:|---:|---:|---:|---:|")?;
    for w in runs.windows(2) {
        let (prev, cur) = (&w[0], &w[1]);
        let prev_map: BTreeMap<&str, &EntryResult> =
            prev.results.iter().map(|r| (r.id.as_str(), r)).collect();
        let (mut fixed_n, mut broke_n) = (0usize, 0usize);
        let (mut fixed_e, mut broke_e) = (0usize, 0usize);
        for r in &cur.results {
            if let Some(p) = prev_map.get(r.id.as_str()) {
                match (p.normalized, r.normalized) {
                    (false, true) => fixed_n += 1,
                    (true, false) => broke_n += 1,
                    _ => {}
                }
                match (p.exact, r.exact) {
                    (false, true) => fixed_e += 1,
                    (true, false) => broke_e += 1,
                    _ => {}
                }
            }
        }
        let d_ex = 100.0 * (Bucket::rate(cur.exact, cur.n) - Bucket::rate(prev.exact, prev.n));
        let d_nm =
            100.0 * (Bucket::rate(cur.normalized, cur.n) - Bucket::rate(prev.normalized, prev.n));
        writeln!(
            s,
            "| {} | {:+.2}pp | {}/{} | {:.4} | {:+.2}pp | {}/{} | {:.4} |",
            cur.name,
            d_ex,
            fixed_e,
            broke_e,
            sign_test_p(fixed_e, broke_e),
            d_nm,
            fixed_n,
            broke_n,
            sign_test_p(fixed_n, broke_n)
        )?;
    }

    // ---- 3. Bootstrap CI on the headline ----
    let exact_hits: Vec<bool> = production.results.iter().map(|r| r.exact).collect();
    let norm_hits: Vec<bool> = production.results.iter().map(|r| r.normalized).collect();
    let (exl, exh) = bootstrap_ci(&exact_hits);
    let (nml, nmh) = bootstrap_ci(&norm_hits);
    writeln!(
        s,
        "\n## Headline uncertainty (percentile bootstrap, 1000 seeded resamples)\n"
    )?;
    writeln!(
        s,
        "- exact top-1 **{:.2}%** (95% CI {:.2}–{:.2}%)\n- normalized top-1 **{:.2}%** (95% CI {:.2}–{:.2}%)\n\nDeltas smaller than ~half this interval width should not be read as real without the paired test above (the paired test is far more sensitive than comparing two independent CIs).\n",
        100.0 * Bucket::rate(production.exact, production.n),
        exl,
        exh,
        100.0 * Bucket::rate(production.normalized, production.n),
        nml,
        nmh,
    )?;

    // ---- 4. Score calibration: reliability table, ECE, Brier ----
    writeln!(
        s,
        "## Score calibration (production top-1 score as P(normalized match))\n"
    )?;
    let mut bins: Vec<(usize, f64, usize)> = vec![(0, 0.0, 0); 10]; // (n, Σscore, hits)
    let mut brier = 0.0f64;
    for r in &production.results {
        let p = r.score.clamp(0.0, 1.0) as f64;
        let y = r.normalized as u8 as f64;
        brier += (p - y) * (p - y);
        let b = ((p * 10.0) as usize).min(9);
        bins[b].0 += 1;
        bins[b].1 += p;
        bins[b].2 += r.normalized as usize;
    }
    let n_tot = production.results.len().max(1);
    brier /= n_tot as f64;
    let mut ece = 0.0f64;
    writeln!(s, "| score bin | n | mean score | empirical match | gap |")?;
    writeln!(s, "|---|---:|---:|---:|---:|")?;
    for (i, (n, sum_p, hits)) in bins.iter().enumerate() {
        if *n == 0 {
            continue;
        }
        let conf = sum_p / *n as f64;
        let acc = *hits as f64 / *n as f64;
        ece += (*n as f64 / n_tot as f64) * (conf - acc).abs();
        writeln!(
            s,
            "| {:.1}–{:.1} | {} | {:.3} | {:.3} | {:+.3} |",
            i as f64 / 10.0,
            (i + 1) as f64 / 10.0,
            n,
            conf,
            acc,
            acc - conf,
        )?;
    }
    writeln!(
        s,
        "\n- **ECE (expected calibration error): {:.4}** — mean |score − empirical match rate| weighted by bin size; 0 is perfectly calibrated.\n- **Brier score: {:.4}** (lower is better; a constant base-rate predictor scores {:.4}).\n- The three-way confidence badge (high/medium/low, thresholds 0.72/0.45 in `Confidence::from_score`) is derived from this score; if a bin's gap drifts past ~0.1 the thresholds should be re-fit.",
        ece,
        brier,
        {
            let base = Bucket::rate(production.normalized, production.n) as f64;
            base * (1.0 - base)
        }
    )?;

    // ---- 5. Isotonic recalibration, dev-fit / holdout-validated ----
    // Fit a monotone score→probability map on the DEV split only (histogram
    // bins pooled by PAVA), then measure ECE/Brier on the HOLDOUT split. This is
    // the leakage-disciplined recipe for fixing the overconfidence above: the
    // holdout numbers say what the calibrator is worth on unseen entries.
    let dev: Vec<&EntryResult> = production
        .results
        .iter()
        .filter(|r| !is_holdout_id(&r.id))
        .collect();
    let held: Vec<&EntryResult> = production
        .results
        .iter()
        .filter(|r| is_holdout_id(&r.id))
        .collect();
    let mut dev_bins = vec![(0usize, 0usize); 10]; // (n, hits) per score decile
    for r in &dev {
        let b = ((r.score.clamp(0.0, 1.0) * 10.0) as usize).min(9);
        dev_bins[b].0 += 1;
        dev_bins[b].1 += r.normalized as usize;
    }
    // PAVA: pool adjacent bins that violate monotonicity (weighted means).
    let mut pools: Vec<(f64, f64)> = Vec::new(); // (weight, mean)
    for (n, hits) in &dev_bins {
        if *n == 0 {
            continue;
        }
        pools.push((*n as f64, *hits as f64 / *n as f64));
        while pools.len() >= 2 && pools[pools.len() - 2].1 > pools[pools.len() - 1].1 {
            let (w2, m2) = pools.pop().unwrap();
            let (w1, m1) = pools.pop().unwrap();
            pools.push((w1 + w2, (w1 * m1 + w2 * m2) / (w1 + w2)));
        }
    }
    // Expand the pooled means back onto the 10 deciles.
    let mut iso = [0.0f64; 10];
    {
        let mut pi = 0usize;
        let mut left = pools.first().map(|p| p.0).unwrap_or(0.0);
        for (b, (n, _)) in dev_bins.iter().enumerate() {
            if *n == 0 {
                iso[b] = pools.get(pi).map(|p| p.1).unwrap_or(0.0);
                continue;
            }
            iso[b] = pools[pi].1;
            left -= *n as f64;
            if left <= 0.0 && pi + 1 < pools.len() {
                pi += 1;
                left = pools[pi].0;
            }
        }
    }
    let calibrate = |score: f32| iso[((score.clamp(0.0, 1.0) * 10.0) as usize).min(9)];
    let eval_split = |rs: &[&EntryResult], use_cal: bool| -> (f64, f64) {
        // (ECE, Brier) binning by the (possibly calibrated) probability.
        let mut b10 = vec![(0usize, 0.0f64, 0usize); 10];
        let mut brier = 0.0f64;
        for r in rs {
            let p = if use_cal {
                calibrate(r.score)
            } else {
                r.score.clamp(0.0, 1.0) as f64
            };
            let y = r.normalized as u8 as f64;
            brier += (p - y) * (p - y);
            let b = ((p * 10.0) as usize).min(9);
            b10[b].0 += 1;
            b10[b].1 += p;
            b10[b].2 += r.normalized as usize;
        }
        let n = rs.len().max(1) as f64;
        let mut ece = 0.0f64;
        for (bn, sp, hits) in &b10 {
            if *bn == 0 {
                continue;
            }
            ece += (*bn as f64 / n) * (sp / *bn as f64 - *hits as f64 / *bn as f64).abs();
        }
        (ece, brier / n)
    };
    let (ece_raw, brier_raw) = eval_split(&held, false);
    let (ece_cal, brier_cal) = eval_split(&held, true);
    writeln!(
        s,
        "\n### Isotonic recalibration (fit on dev, validated on holdout)\n\nA monotone score→probability map (decile histogram + pool-adjacent-violators) fit on the dev split only, then applied to the untouched holdout:\n\n| Holdout metric | raw score | recalibrated | Δ |\n|---|---:|---:|---:|\n| ECE | {ece_raw:.4} | {ece_cal:.4} | {:+.4} |\n| Brier | {brier_raw:.4} | {brier_cal:.4} | {:+.4} |\n\nThe recalibrated probability is valid for downstream consumers of this same official-row pipeline score as *P(matches the official lemma)*; the raw score remains the ranking key. It is not valid for the corpus path's separate coverage score (issue #89 J26). Refit whenever the ladder changes.",
        ece_cal - ece_raw,
        brier_cal - brier_raw,
    )?;

    // Persist the fitted calibrator (Track C / issue #3) with a machine-checked
    // score domain. Consumers of another ranking function (notably the corpus
    // coverage score) must reject it rather than publish cross-model
    // probabilities. Regenerated (refitted) by every `evaluate` run.
    let pr_at = |t: f64| -> (f64, f64) {
        let sel: Vec<&&EntryResult> = held.iter().filter(|r| calibrate(r.score) >= t).collect();
        let hits = sel.iter().filter(|r| r.normalized).count();
        let total = held.iter().filter(|r| r.normalized).count().max(1);
        (
            hits as f64 / sel.len().max(1) as f64,
            hits as f64 / total as f64,
        )
    };
    let cal = crate::calibrate::Calibration {
        score_domain: crate::calibrate::PIPELINE_SCORE_DOMAIN.to_string(),
        fitted_on: format!(
            "evaluate dev split ({} entries), production rung '{}'",
            dev.len(),
            production.name
        ),
        holdout_ece: ece_cal,
        propose_pr: pr_at(crate::calibrate::PROPOSE_T),
        review_pr: pr_at(crate::calibrate::REVIEW_T),
        deciles: iso,
    };
    std::fs::write(crate::calibrate::PATH, serde_json::to_string_pretty(&cal)?)?;

    // ---- 6. Pipeline operating points (Track C / issue #3) ----
    // These thresholds describe the official-row pipeline score domain only.
    // They are evidence for compatible consumers: at each cutoff t, `precision` =
    // P(normalized match | p ≥ t) on the benchmark, `recall` = share of all
    // matches captured. Computed on the HOLDOUT split so the operating points
    // are honest out-of-sample numbers.
    writeln!(
        s,
        "\n### Official-row pipeline operating points (calibrated p, holdout split)\n\n| threshold | n ≥ t | precision | recall |\n|---:|---:|---:|---:|"
    )?;
    let total_hits = held.iter().filter(|r| r.normalized).count().max(1);
    for t10 in [3usize, 4, 5, 6, 7] {
        let t = t10 as f64 / 10.0;
        let sel: Vec<&&EntryResult> = held.iter().filter(|r| calibrate(r.score) >= t).collect();
        let hits = sel.iter().filter(|r| r.normalized).count();
        writeln!(
            s,
            "| ≥ {:.1} | {} | {:.1}% | {:.1}% |",
            t,
            sel.len(),
            100.0 * hits as f64 / sel.len().max(1) as f64,
            100.0 * hits as f64 / total_hits as f64,
        )?;
    }
    writeln!(
        s,
        "\nThese operating points apply only to consumers of the official-row pipeline score. Corpus novel-word buckets remain disabled until that separate coverage score has its own holdout-validated calibrator. The committed pipeline calibrator is `{}`.",
        crate::calibrate::PATH
    )?;

    std::fs::write(out_dir.join("methodology.md"), s)?;

    // Full per-entry predictions dump (hits AND misses) for offline pattern
    // mining and run-to-run diffing — the capped errors-sample.csv only shows
    // the nearest 400 misses.
    let mut p = String::from(
        "id,holdout,gloss,pos,official,predicted,exact,normalized,score,confidence,branch_cov,n_langs,norm_edit\n",
    );
    for r in &production.results {
        writeln!(
            p,
            "{},{},{},{},{},{},{},{},{:.3},{},{},{},{:.3}",
            r.id,
            is_holdout_id(&r.id) as u8,
            csv_escape(&r.gloss),
            r.pos.code(),
            csv_escape(&r.isv),
            csv_escape(&r.predicted),
            r.exact as u8,
            r.normalized as u8,
            r.score,
            r.confidence.map(conf_label).unwrap_or("-"),
            r.branch_cov,
            r.n_langs,
            r.norm_edit,
        )?;
    }
    std::fs::write(out_dir.join("predictions.csv"), p)?;
    Ok(())
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjective_gender_agreement() {
        use crate::model::Gender;
        // Hard -y: nova / novo; soft -i: svěža... -ja/-je; None/masc unchanged.
        assert_eq!(agree_adjective("novy", Some(Gender::Feminine)), "nova");
        assert_eq!(agree_adjective("novy", Some(Gender::Neuter)), "novo");
        assert_eq!(agree_adjective("svěži", Some(Gender::Feminine)), "svěža");
        assert_eq!(agree_adjective("novy", Some(Gender::Masculine)), "novy");
        assert_eq!(agree_adjective("novy", None), "novy");
        // Non-adjectival tails pass through untouched.
        assert_eq!(agree_adjective("dom", Some(Gender::Feminine)), "dom");
    }

    #[test]
    fn exact_sign_test_matches_binomial_tails() {
        assert!((sign_test_p(18, 0) - 0.000_007_629_394_531_25).abs() < 1e-12);
        assert_eq!(sign_test_p(0, 0), 1.0);
        assert_eq!(sign_test_p(1, 1), 1.0);
    }

    #[test]
    fn ladder_ends_at_production() {
        // The Headline / CI floor reports runs.last(); it MUST be production().
        assert_eq!(
            kept_ladder().last().map(|r| r.cfg),
            Some(ConsensusConfig::production())
        );
    }

    #[test]
    fn ladder_starts_at_baseline() {
        assert_eq!(
            kept_ladder().first().map(|r| r.cfg),
            Some(ConsensusConfig::baseline())
        );
    }
}
