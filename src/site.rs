//! Static site generator for the Interslavic candidate dictionary.
//!
//! `export` runs the generator over the official dictionary's Slavic evidence and
//! writes a fully static website — one HTML page per meaning plus a home page
//! with client-side search — under an output directory. There is no server and
//! no in-memory database: the output is plain files hostable on GitHub Pages (or
//! any static host). All links are relative and all CSS is local.

use crate::consensus::{ConsensusConfig, MeaningInput};
use crate::generator::{self, Generation};
use crate::lang::Branch;
use crate::model::{Candidate, CandidateSource, Confidence, Evidence, MatchStatus};
use crate::official::{self, OfficialEntry};
use crate::overrides::Overrides;
use anyhow::Result;
use interslavic::{
    Animacy as IsvAnimacy, Case as IsvCase, Gender as IsvGender, Number as IsvNumber, ISV,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

/// Counts inflection-table panics swallowed by the quiet hook (see below).
static INFLECTION_PANICS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Identity-safe headword routing. Exact scientific spelling wins; a folded
/// spelling resolves only when one page owns that fold. This keeps search
/// normalization from becoming lexical identity for homographs.
#[derive(Default)]
struct HeadwordIndex {
    exact: std::collections::HashMap<String, Vec<usize>>,
    folded: std::collections::HashMap<String, Vec<usize>>,
}

impl HeadwordIndex {
    fn insert(&mut self, title: &str, id: usize) {
        let exact = title.trim().to_lowercase();
        if exact.is_empty() {
            return;
        }
        let fold = crate::orthography::to_standard(&exact);
        let exact_ids = self.exact.entry(exact).or_default();
        if !exact_ids.contains(&id) {
            exact_ids.push(id);
        }
        let folded_ids = self.folded.entry(fold).or_default();
        if !folded_ids.contains(&id) {
            folded_ids.push(id);
        }
    }

    fn unique(ids: &[usize]) -> Option<usize> {
        (ids.len() == 1).then(|| ids[0])
    }

    fn resolve(&self, title: &str) -> Option<usize> {
        let exact = title.trim().to_lowercase();
        match self.exact.get(&exact) {
            Some(ids) => Self::unique(ids),
            None => self.resolve_fold(&crate::orthography::to_standard(&exact)),
        }
    }

    fn resolve_fold(&self, fold: &str) -> Option<usize> {
        self.folded.get(fold).and_then(|ids| Self::unique(ids))
    }
}

/// The bundled `interslavic` inflection crate *panics* (rather than erroring) on
/// stems it can't handle — reflexive `-sę` verbs and athematic `-ći` infinitives.
/// We already recover each one with `catch_unwind` (the cell shows "—"), but the
/// default panic hook still prints the message thousands of times. Install a hook
/// that swallows panics originating inside that crate (counting them) and passes
/// any real panic from our own code through to the default handler.
fn install_quiet_inflection_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let from_inflector = info
            .location()
            .map(|l| l.file().contains("interslavic"))
            .unwrap_or(false);
        if from_inflector {
            INFLECTION_PANICS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        default(info);
    }));
}

/// Generate the whole static site under `out_dir`.
pub fn export(official_path: &Path, out_dir: &Path) -> Result<()> {
    install_quiet_inflection_hook();
    let entries = official::load(official_path)?;
    let overrides = Overrides::load(Path::new(crate::DEFAULT_OVERRIDES));
    let cfg = ConsensusConfig::production();
    let proto_path = Path::new(crate::DEFAULT_PROTO_CACHE);
    let proto_index = crate::dump::load_optional(proto_path, crate::dump::ProtoIndex::load)?;
    let proto = proto_index.as_ref();
    if proto.is_some() {
        println!("Using Proto-Slavic cache for reconstruction-derived forms.");
    }
    // Calibrated confidence for display (issue #77): the legacy candidate
    // scores are the calibrator's native scale, so badges re-bucket through
    // the fitted probability map. Absent file → raw-score buckets stand.
    let calibration = crate::calibrate::Calibration::load_for_domain(
        Path::new(crate::calibrate::PATH),
        crate::calibrate::PIPELINE_SCORE_DOMAIN,
    )?;
    if calibration.is_none() {
        println!(
            "(no {} — run `evaluate` to fit the calibrator; badges fall back to raw-score buckets)",
            crate::calibrate::PATH
        );
    }

    let entry_dir = out_dir.join("entry");
    std::fs::create_dir_all(&entry_dir)?;

    // Streaming pass: render each entry, accumulate the search index + stats.
    let mut search_rows: Vec<SearchRow> = Vec::new();
    let mut top_rows: Vec<HomeRow> = Vec::new();
    let (mut n, mut n_match, mut n_diff, mut n_none, mut n_exact, mut n_top3) =
        (0usize, 0, 0, 0, 0, 0);

    let mut id = 0usize;
    for entry in &entries {
        let input = build_input(entry);
        if input.forms.iter().filter(|f| f.modern).count() < 2 || entry.isv.trim().is_empty() {
            continue;
        }
        let official = if entry.isv.contains(' ') || entry.isv.contains('#') {
            None
        } else {
            Some(entry.isv.as_str())
        };
        let mut g = generator::generate(&input, official, proto, &cfg, &overrides);
        // Display badges come from the calibrated probability, never the raw
        // score (issue #77); scores/ordering stay untouched.
        if let Some(cal) = &calibration {
            for c in g.candidates.iter_mut() {
                c.confidence = Confidence::from_probability(cal.probability(c.score));
            }
        }
        let Some(top) = g.candidates.first() else {
            continue;
        };
        id += 1;
        n += 1;
        match g.match_status {
            MatchStatus::OfficialMatch => n_match += 1,
            MatchStatus::DiffersFromOfficial => n_diff += 1,
            MatchStatus::NoOfficialEntry => n_none += 1,
        }
        if let Some(off) = official {
            if crate::orthography::exact_match(&top.form, off) {
                n_exact += 1;
            }
            if g.candidates
                .iter()
                .take(3)
                .any(|c| crate::orthography::normalized_match(&c.form, off))
            {
                n_top3 += 1;
            }
        }
        let form = top.form.clone();
        let evidence = branch_evidence(&input);
        let html = entry_page(id, entry, &g, &evidence, calibration.as_ref());
        std::fs::write(entry_dir.join(format!("{id}.html")), html)?;

        // search index row (14-element schema shared with the corpus path).
        let statuschar = match g.match_status {
            MatchStatus::OfficialMatch => "O",
            MatchStatus::DiffersFromOfficial => "D",
            MatchStatus::NoOfficialEntry => "N",
        };
        let mut keys = search_keys(&g.candidates, &form);
        if let Some(off) = official {
            // The official lemma is searchable even when no candidate spells it:
            // point it at the candidate that agrees (normalized), else the top.
            let rank = g
                .candidates
                .iter()
                .position(|c| crate::orthography::normalized_match(&c.form, off))
                .map(|i| i + 1)
                .unwrap_or(1);
            let lower = off.to_lowercase();
            for k in [
                lower.clone(),
                crate::orthography::to_standard(&lower),
                crate::orthography::ascii_skeleton(off),
            ] {
                if k.chars().count() >= 2
                    && !keys.iter().any(|(kk, _)| kk == &k)
                    && k != form.to_lowercase()
                {
                    keys.push((k, rank));
                }
            }
        }
        let gloss70 = truncate(&entry.english, 70);
        // Razumlivost (element 12) from the committee's own sameInLanguages
        // attestation — the translation cells are filled for every language
        // and would claim a constant ~99%; null when the column is empty.
        let razum = {
            let same_in = entry.same_in_langs();
            if same_in.is_empty() {
                "null".to_string()
            } else {
                (crate::lang::razumlivost(&same_in).overall.round() as u32).to_string()
            }
        };
        search_rows.push(SearchRow {
            id,
            head: format!(
                "[{},{},{},{},{},{},{},1,1,0,{},{},{}",
                id,
                json_str(&form),
                json_str(&gloss70),
                json_str(&entry.pos.code()),
                json_str(statuschar),
                json_str(conf_letter(top.confidence)),
                keys_json(&keys),
                json_str(""),
                json_str(""),
                razum,
            ),
            aliases: "[]".to_string(),
            core: true,
            buckets: search_row_buckets(&form, &gloss70, &keys, &[]),
        });
        let freq = entry.frequency.unwrap_or(0.0);
        top_rows.push(HomeRow {
            freq,
            id,
            form,
            gloss: entry.english.clone(),
            pos: entry.pos.code().to_string(),
            status: g.match_status,
            conf: top.confidence,
            score: top.score,
            prob: calibration.as_ref().map(|c| c.probability(top.score)),
        });
    }
    write_search_index(out_dir, &search_rows)?;
    let _ = std::fs::remove_file(out_dir.join("search.json"));
    std::fs::write(out_dir.join("wiktionary.css"), css())?;
    std::fs::write(out_dir.join(".nojekyll"), "")?; // don't run Jekyll on GitHub Pages

    // Home page: stats + client-side search + the most frequent entries.
    top_rows.sort_by(|a, b| b.freq.total_cmp(&a.freq));
    let with_official = n_match + n_diff;
    let rate = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f32 / b as f32
        }
    };
    let home = home_page(
        n,
        n_match,
        n_diff,
        n_none,
        rate(n_match, with_official),
        rate(n_exact, with_official),
        &top_rows,
    );
    std::fs::write(out_dir.join("index.html"), home)?;
    std::fs::write(out_dir.join("search.html"), search_page())?;
    std::fs::write(out_dir.join("forms.html"), forms_page())?;
    std::fs::write(out_dir.join("text-check.html"), text_check_page())?;
    std::fs::write(
        out_dir.join("about.html"),
        about_page(
            n,
            rate(n_match, with_official),
            rate(n_exact, with_official),
            rate(n_top3, with_official),
        ),
    )?;

    println!(
        "wrote {} static pages to {} ({} match official, {} differ, {} no official, {:.1}% normalized match)",
        n,
        out_dir.display(),
        n_match,
        n_diff,
        n_none,
        rate(n_match, with_official)
    );
    let panics = INFLECTION_PANICS.load(std::sync::atomic::Ordering::Relaxed);
    if panics > 0 {
        println!(
            "note: {panics} inflection cells left blank (stems the bundled inflector can't decline)"
        );
    }
    Ok(())
}

// ===========================================================================
// Corpus-driven site: a cognate-set dictionary built from ALL inherited Slavic
// lemmas in Wiktionary, independent of the official Interslavic dictionary.
// ===========================================================================

/// Core IDs are assigned from the finalized deterministic export order. They
/// deliberately do not consult previous output or a compatibility registry:
/// identical inputs produce identical IDs, while corpus changes may renumber.
#[derive(Default)]
struct DeterministicEntryIds {
    high_water: usize,
}

impl DeterministicEntryIds {
    fn alloc(&mut self) -> usize {
        self.high_water += 1;
        self.high_water
    }

    fn max_id(&self) -> usize {
        self.high_water
    }
}

/// Select an official sense only with positive lexical evidence. Exact/folded
/// spelling is a candidate lookup, not enough by itself to establish identity.
fn select_official_entry(
    rows: &[usize],
    official_entries: &[OfficialEntry],
    pos: crate::model::Pos,
    set_gloss: &str,
) -> Option<usize> {
    let set_tokens = crate::dump::gloss_tokens(set_gloss);
    let set_compact = set_tokens.join("");
    rows.iter()
        .copied()
        .filter(|&i| official_entries[i].pos == pos)
        .map(|i| {
            let gloss = crate::dump::gloss_tokens(&official_entries[i].english);
            let overlap = set_tokens.iter().filter(|t| gloss.contains(t)).count();
            // Wiktionaries vary compounds freely (fairy tale/fairytale,
            // feather grass/feathergrass). Joined content-token equality is a
            // positive semantic discriminator without admitting unrelated POS.
            let compound_match = !set_compact.is_empty() && set_compact == gloss.join("");
            (i, overlap, compound_match)
        })
        .filter(|(_, overlap, compound_match)| *overlap > 0 || *compound_match)
        .max_by_key(|(_, overlap, compound_match)| (*overlap, *compound_match))
        .map(|(i, _, _)| i)
}

/// Generate the static site from the Wiktionary cognate-set corpus. Every set of
/// etymologically-connected Slavic lemmas becomes one Interslavic word, with
/// confidence scaling by how many languages/branches attest it.
pub fn export_corpus(lemmas_path: &Path, official_path: &Path, out_dir: &Path) -> Result<()> {
    install_quiet_inflection_hook();
    let corpus = crate::dump::LemmaCorpus::load(lemmas_path)?;
    let cfg = ConsensusConfig::production();
    let sets = crate::corpus::build_sets(&corpus);
    println!(
        "built {} cognate sets from {} Slavic lemmas",
        sets.len(),
        corpus.entry_count
    );

    // The official dictionary is the authoritative display layer: any generated
    // word whose candidate reproduces an official lemma is shown under the
    // official headword, and official lemmas the corpus never generates still
    // get searchable pages (§V6 Front B). Display-only — generation never reads
    // this map.
    // Native-Wiktionary enrichment (RU/PL/CS etymology, senses, semantic links),
    // if the cache has been built. Display-only; generation never reads it.
    // Absent → degrade with a notice; present-but-stale/corrupt → hard error.
    let enrich = crate::dump::load_optional(
        Path::new(crate::DEFAULT_ENRICH_CACHE),
        crate::enrich::EnrichIndex::load,
    )?;
    if let Some(e) = &enrich {
        println!(
            "Loaded {} native-Wiktionary enrichment entries (RU/PL/CS).",
            e.len()
        );
    } else {
        println!("(no enrichment cache — run extract-enrich for native etymology/links)");
    }

    let official_entries = official::load(official_path)?;
    // Keep exact scientific spellings distinct. Standard folding is useful for
    // lookup, but it is not lexical identity: dŕžati/držati and legti/lęgti
    // are different official lemmas with different meanings/aspects.
    let mut official_by_exact: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    let mut official_by_fold: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, e) in official_entries.iter().enumerate() {
        let isv = e.isv.trim();
        if isv.is_empty() || isv.contains(' ') || isv.contains('#') {
            continue;
        }
        official_by_exact
            .entry(isv.to_lowercase())
            .or_default()
            .push(i);
        official_by_fold
            .entry(crate::orthography::to_standard(&isv.to_lowercase()))
            .or_default()
            .push(i);
    }

    // IDs depend only on the finalized deterministic export order. No previous
    // site or compatibility registry participates in allocation.
    let mut entry_ids = DeterministicEntryIds::default();

    let entry_dir = out_dir.join("entry");
    let _ = std::fs::remove_dir_all(&entry_dir); // clear any stale pages
    std::fs::create_dir_all(&entry_dir)?;

    // Search rows are staged and written as first-letter shards at the end
    // (issue #71; see `write_search_index`).
    let mut search_rows: Vec<SearchRow> = Vec::new();
    let mut rows: Vec<HomeRow> = Vec::new();
    let (mut official, mut borrowed) = (0usize, 0usize);
    // n / high / med / low are computed after same-concept suppression (below).
    let (n, high, med, low);
    let mut lemma_total = 0usize;
    // Official dictionary sense IDs represented by surviving generated pages,
    // so every other sense receives its own official-only page.
    let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();

    // First pass: generate every word, so ancestor families (shared proto stem
    // or loan etymon) can be cross-linked before any page is rendered.
    #[derive(Clone, Copy)]
    struct OfficialMatch {
        rank: usize,
        entry: usize,
    }
    struct Prepared {
        id: usize,
        g: crate::corpus::GeneratedWord,
        display: String,
        status: MatchStatus,
        matched: Option<OfficialMatch>,
        /// A redundant same-concept duplicate (same folded form + overlapping
        /// gloss as a better set): not rendered, kept out of search/links.
        suppressed: bool,
    }
    impl FamilyEntry for Prepared {
        fn id(&self) -> usize {
            self.id
        }
        fn display(&self) -> &str {
            &self.display
        }
        fn set(&self) -> &crate::corpus::CognateSet {
            &self.g.set
        }
    }
    let mut prepared: Vec<Prepared> = Vec::new();
    for set in sets {
        let members = set.members.len();
        let g = crate::corpus::generate_set(set, &cfg);
        let form = g.form().to_string();
        if form.is_empty() {
            continue;
        }
        lemma_total += members;
        if g.set.borrowed {
            borrowed += 1;
        }
        // n / high / med / low are recomputed after same-concept suppression.
        // Prefer an exact scientific spelling. A folded match is accepted only
        // when every row under that fold has the same exact spelling;
        // otherwise distinct lexemes get separate official-only pages.
        let matched: Option<OfficialMatch> =
            g.candidates
                .iter()
                .take(5)
                .enumerate()
                .find_map(|(rank, c)| {
                    let lower = c.form.trim().to_lowercase();
                    let rows = if let Some(rows) = official_by_exact.get(&lower) {
                        rows.as_slice()
                    } else {
                        let fold = crate::orthography::to_standard(&lower);
                        let rows = official_by_fold.get(&fold)?.as_slice();
                        let mut spellings = rows
                            .iter()
                            .map(|&i| official_entries[i].isv.trim().to_lowercase());
                        let first = spellings.next()?;
                        if spellings.any(|s| s != first) {
                            return None;
                        }
                        rows
                    };
                    let entry =
                        select_official_entry(rows, &official_entries, g.set.pos, &g.set.gloss)?;
                    Some(OfficialMatch {
                        rank: rank + 1,
                        entry,
                    })
                });
        if matched.is_some() {
            official += 1;
        }
        let status = if matched.is_some() {
            MatchStatus::OfficialMatch
        } else {
            MatchStatus::NoOfficialEntry
        };
        let display = matched
            .map(|m| official_entries[m.entry].isv.trim().to_string())
            .unwrap_or_else(|| form.clone());
        prepared.push(Prepared {
            // Assigned only after homograph demotion and suppression finalize
            // this page's rendered identity.
            id: 0,
            g,
            display,
            status,
            matched,
            suppressed: false,
        });
    }

    // ---- Confidence domain boundary (issue #89 J26/J27) ----
    // `generate_set` scores are cognate-coverage scores. The committed
    // calibrator is fitted on the separate official-row pipeline candidate
    // score and MUST NOT be applied here merely because both scales are 0–1.
    // Keep coverage badges, export null probabilities and pause the proposal
    // buckets until a corpus-coverage calibrator is fitted and holdout-validated.
    let calibration: Option<crate::calibrate::Calibration> = None;
    println!(
        "Corpus coverage scores are uncalibrated; generated probabilities and novel-word proposal buckets are disabled (issue #89 J26)."
    );

    // Homograph / duplicate dedup. Several corpus sets can fold to the same
    // official lemma: genuine homographs (`ja` = I / and / yes), redundant
    // same-meaning sets (`jedin` ×N, all "one"), or a borrowing colliding with a
    // native word (the French-borrowed *pisati* "piss" vs the native official
    // *pisati* "write"). Each official dictionary sense may be represented by
    // at most ONE set — the one whose POS and gloss positively match that row,
    // tie-broken by
    // the set's score. The losing sets keep their own page but lose the official
    // badge, so no page ever headlines an official meaning it does not carry.
    // Display-only: the leakage-free benchmark scores `generate_set` per official
    // row directly and is completely untouched by this.
    {
        let rank = |p: &Prepared, en: &str| -> (usize, i32) {
            let a = crate::dump::gloss_tokens(&p.g.set.gloss);
            let b = crate::dump::gloss_tokens(en);
            let overlap = a.iter().filter(|t| b.contains(t)).count();
            (overlap, (p.g.score * 1000.0) as i32)
        };
        let mut best: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for (i, p) in prepared.iter().enumerate() {
            if let Some(m) = p.matched {
                let e = &official_entries[m.entry];
                let key = e.id.clone();
                let win = match best.get(&key) {
                    Some(&j) => rank(p, &e.english) > rank(&prepared[j], &e.english),
                    None => true,
                };
                if win {
                    best.insert(key, i);
                }
            }
        }
        let mut demoted = 0usize;
        for i in 0..prepared.len() {
            let Some(m) = prepared[i].matched else {
                continue;
            };
            let key = official_entries[m.entry].id.clone();
            if best.get(&key) != Some(&i) {
                prepared[i].matched = None;
                prepared[i].status = MatchStatus::NoOfficialEntry;
                prepared[i].display = prepared[i].g.form().to_string();
                demoted += 1;
            }
        }
        println!("Deduped {demoted} duplicate official matches (one representative per dictionary sense).");
        official -= demoted;
    }

    // ---- Official-fact treatment for MATCHED entries (issue #86) ----
    // An entry whose candidate reproduces an official lemma is a verified
    // dictionary fact, not a prediction: the calibrated p is the PRIOR
    // P(matches an official decision) — for these entries the match already
    // resolved, so displaying "nizka p≈0.14" above "rekonstrukcija ju točno
    // reproduktuje" was contradictory (2,020 official words rendered nizka).
    // Give matched entries the same posture as official-only pages:
    // Confidence::High flows to the search-row letter (V), the home-sidebar
    // counts and meta.conf; the raw score stays untouched (it is a ranking
    // key); the calibrated prior moves to a display-only transparency line in
    // the provenance section (meta.prior below). Runs AFTER the homograph
    // dedup so demoted entries (matched cleared) keep their calibrated bucket.
    for p in prepared.iter_mut() {
        if p.matched.is_some() {
            p.g.confidence = Confidence::High;
        }
    }

    // Same-concept suppression: after the official representative is chosen,
    // collapse the remaining duplicate pages that share a folded form AND a gloss
    // token with a stronger set (numbers tagged noun vs num, `jaky` "strong,
    // firm" ×2, duplicate proper nouns). True homographs (disjoint gloss: `ja` =
    // I / and / yes) keep their own page. Suppressed pages are not rendered, and
    // are kept out of search, families, and cross-links. Display-only.
    {
        let gloss_of = |p: &Prepared| -> Vec<String> {
            match p.matched {
                Some(m) => crate::dump::gloss_tokens(&official_entries[m.entry].english),
                None => crate::dump::gloss_tokens(&p.g.set.gloss),
            }
        };
        let rank = |p: &Prepared| (p.matched.is_some(), (p.g.score * 1000.0) as i32);
        let mut by_form: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, p) in prepared.iter().enumerate() {
            by_form
                .entry(crate::orthography::to_standard(&p.g.form().to_lowercase()))
                .or_default()
                .push(i);
        }
        let mut suppressed_n = 0usize;
        for (_f, mut group) in by_form {
            if group.len() < 2 {
                continue;
            }
            group.sort_by(|&a, &b| rank(&prepared[b]).cmp(&rank(&prepared[a])));
            let mut kept: Vec<Vec<String>> = Vec::new();
            for &i in &group {
                let g = gloss_of(&prepared[i]);
                if !g.is_empty() && kept.iter().any(|k| g.iter().any(|t| k.contains(t))) {
                    prepared[i].suppressed = true;
                    suppressed_n += 1;
                } else {
                    kept.push(g);
                }
            }
        }
        // Recompute display counts over the surviving pages.
        n = prepared.iter().filter(|p| !p.suppressed).count();
        high = prepared
            .iter()
            .filter(|p| !p.suppressed && matches!(p.g.confidence, Confidence::High))
            .count();
        med = prepared
            .iter()
            .filter(|p| !p.suppressed && matches!(p.g.confidence, Confidence::Medium))
            .count();
        low = prepared
            .iter()
            .filter(|p| !p.suppressed && matches!(p.g.confidence, Confidence::Low))
            .count();
        println!("Suppressed {suppressed_n} same-concept duplicate pages.");
    }

    // Allocate only after demotion/suppression finalize the rendered sequence.
    for p in prepared.iter_mut().filter(|p| !p.suppressed) {
        p.id = entry_ids.alloc();
    }

    // Track-E issue metric: evidence growth for official internationalisms.
    // This is measured on surviving matched pages (the reader-visible class),
    // never fed back into grouping or scoring. The left-hand figures are the
    // frozen 7a8fc98 baseline reported in issue #86.
    let mut genesis_i_single = 0usize;
    let mut all_branch_single = 0usize;
    for p in prepared
        .iter()
        .filter(|p| !p.suppressed && p.matched.is_some())
    {
        let Some(m) = p.matched else {
            continue;
        };
        let e = &official_entries[m.entry];
        if e.genesis.trim() == "I" && p.g.n_langs == 1 {
            genesis_i_single += 1;
            let markers: BTreeSet<&str> = e.same_in.split_whitespace().collect();
            if markers == BTreeSet::from(["j", "v", "z"]) {
                all_branch_single += 1;
            }
        }
    }
    println!(
        "issue-86 internationalism evidence: genesis-I matched langs=1: 564 → {genesis_i_single}; sameInLanguages=v z j and langs=1: 176 → {all_branch_single}"
    );

    // Word families: entries whose ancestors share a Proto-Slavic stem
    // (*starъ/*starostь/*starьcь) or the same loan etymon (la magister →
    // majstor/maestro/magistr) cross-link each other.
    let mut families: std::collections::BTreeMap<String, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (i, p) in prepared.iter().enumerate() {
        if p.suppressed {
            continue;
        }
        if let Some(k) = family_key(&p.g.set) {
            families.entry(k).or_default().push(i);
        }
    }

    // Interslavic synonym thesaurus (dictionary-derived) + an exact-first,
    // ambiguity-aware headword index for identity-sensitive page links.
    let thesaurus = crate::thesaurus::Thesaurus::build(&official_entries);
    let mut isv_to_id = HeadwordIndex::default();
    for p in &prepared {
        if !p.suppressed {
            isv_to_id.insert(&p.display, p.id);
        }
    }

    // Reverse index for intra-site cross-linking: every cognate member of every
    // entry points back to that entry's page, so an enrichment chip (related /
    // synonym / antonym term) that is itself a dictionary headword links to the
    // internal page instead of out to Wiktionary — turning the per-entry
    // enrichment into a site-wide semantic graph.
    let mut xref = crate::enrich::Xref::new();
    for p in &prepared {
        if p.suppressed {
            continue;
        }
        for m in &p.g.set.members {
            xref.insert(&m.lang, &m.word, p.id);
        }
    }
    println!(
        "Built {} cognate cross-reference keys for intra-site links ({} ambiguous homograph/sense keys withheld).",
        xref.len(),
        xref.ambiguous_len()
    );

    // Official lemmas with no SURVIVING matched page get their own page. The
    // earlier candidate-coverage set includes top-5 alternatives and suppressed
    // pages, which can otherwise make an official verb disappear entirely.
    covered.clear();
    for p in prepared.iter().filter(|p| !p.suppressed) {
        if let Some(m) = p.matched {
            covered.insert(official_entries[m.entry].id.clone());
        }
    }
    // Reserve official-only ids before rendering so all wiki indexes can see
    // the complete static site graph.
    let mut official_only = 0usize;
    let mut official_only_records: Vec<(usize, OfficialEntry)> = Vec::new();
    for e in &official_entries {
        let isv = e.isv.trim();
        if isv.is_empty() || isv.contains('#') {
            continue;
        }
        if !covered.insert(e.id.clone()) {
            continue; // this exact official sense already has a generated page
        }
        let entry_id = entry_ids.alloc();
        official_only += 1;
        official_only_records.push((entry_id, e.clone()));
    }
    for (oid, e) in &official_only_records {
        isv_to_id.insert(e.isv.trim(), *oid);
    }

    // Raw-attestation pre-pass (issue #64): load the raw corpus and decide
    // every raw lemma's fate BEFORE any page renders, so (a) raw entry ids
    // exist by the time word chips on ANY page — including raw pages rendered
    // early in the raw loop — want to link to them, and (b) raw words whose
    // display fold deduped onto an official/generated page resolve to that
    // page's id. Fate is still decided by `raw_lemma_fate` (the single dedup
    // rule shared with `coverage`), exactly once per lemma per export; the
    // raw render loop below consumes this plan instead of re-classifying.
    let raw_corpus = crate::dump::load_optional(
        Path::new(crate::DEFAULT_RAW_LEMMA_CACHE),
        crate::dump::RawSlavicCorpus::load,
    )?;
    let raw_plan = raw_corpus
        .as_ref()
        .map(|rc| plan_raw_pages(&rc.lemmas, &xref, &isv_to_id, entry_ids.max_id()))
        .unwrap_or_default();
    // Raw-collision display credit census (issue #86 item 6).
    println!(
        "raw-credit: {} entries show {} fold-deduped raw attestations (display-only, issue #86).",
        raw_plan.credit.len(),
        raw_plan.credit.values().map(Vec::len).sum::<usize>(),
    );
    // `plan_raw_pages` starts after the largest deterministic core id, so raw ids
    // cannot collide even though the core id space may now contain holes.

    let mut metas: Vec<SiteEntryMeta> = Vec::new();
    for p in prepared.iter().filter(|p| !p.suppressed) {
        let ancestor = if p.g.set.borrowed {
            p.g.set.etymon.clone()
        } else {
            p.g.set.proto.clone()
        };
        let mut langs: Vec<String> =
            p.g.set
                .members
                .iter()
                .filter(|m| crate::lang::lang_info(&m.lang).is_some_and(|info| info.modern))
                .map(|m| m.lang.clone())
                .collect();
        langs.sort();
        langs.dedup();
        let wiki_categories =
            wiktionary_category_paths_for_members(&p.g.set.members, enrich.as_ref());
        // A matched entry is an official fact: no prediction probability
        // (`prob` = None, like official-only pages — entries.json emits null,
        // matching the API posture). The calibrated PRIOR is kept separately
        // for the provenance transparency line only (issue #86).
        let prior = calibration.as_ref().map(|c| c.probability(p.g.score));
        let mut meta = entry_meta(
            p.id,
            &p.display,
            p.matched
                .map(|m| official_entries[m.entry].english.as_str())
                .unwrap_or(&p.g.set.gloss),
            p.matched
                .map(|m| &official_entries[m.entry])
                .map(|e| {
                    if crate::aspect::aspect(&e.pos_raw).is_some() {
                        "verb"
                    } else {
                        e.pos.code()
                    }
                })
                .unwrap_or_else(|| p.g.set.pos.code()),
            p.status,
            p.g.confidence,
            p.g.score,
            if p.matched.is_some() { None } else { prior },
            p.g.n_langs,
            p.g.n_branches,
            p.g.set.borrowed,
            false,
            p.matched
                .map(|m| official_entries[m.entry].isv.trim().to_string()),
            ancestor,
            langs,
            wiki_categories,
        );
        if let Some(m) = p.matched {
            meta.prior = prior;
            meta.official_sense_id = Some(official_entries[m.entry].id.clone());
        }
        metas.push(meta);
    }
    for (oid, e) in &official_only_records {
        let input = build_input(e);
        let mut langs: Vec<String> = input
            .forms
            .iter()
            .filter(|f| f.modern)
            .map(|f| f.lang_code.clone())
            .collect();
        langs.sort();
        langs.dedup();
        let mut branches = std::collections::BTreeSet::new();
        for f in input.forms.iter().filter(|f| f.modern) {
            branches.insert(f.branch.label().to_string());
        }
        let wiki_categories = wiktionary_category_paths_for_input(&input, enrich.as_ref());
        let mut meta = entry_meta(
            *oid,
            e.isv.trim(),
            &e.english,
            if crate::aspect::aspect(&e.pos_raw).is_some() {
                "verb"
            } else {
                e.pos.code()
            },
            MatchStatus::OfficialMatch,
            Confidence::High,
            1.0,
            None,
            langs.len(),
            branches.len(),
            e.genesis.trim() == "I",
            true,
            Some(e.isv.trim().to_string()),
            String::new(),
            langs,
            wiki_categories,
        );
        meta.official_sense_id = Some(e.id.clone());
        metas.push(meta);
    }
    // Aspect metadata and bidirectional partner links (issue #75). Official
    // aspect/gloss data is appropriate on the display path; it never enters
    // candidate generation or the leakage-free benchmark path.
    let meta_pos: std::collections::HashMap<usize, usize> =
        metas.iter().enumerate().map(|(i, m)| (m.id, i)).collect();
    // Resolve every official dictionary sense through the page allocated for
    // that row. Exact spelling alone is insufficient: same-spelling homographs
    // such as pasti (fall, pf.) / pasti (graze, ipf.) are separate senses.
    let mut official_page_ids: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for p in prepared.iter().filter(|p| !p.suppressed) {
        if let Some(m) = p.matched {
            official_page_ids.insert(official_entries[m.entry].id.clone(), p.id);
        }
    }
    for (id, e) in &official_only_records {
        official_page_ids.insert(e.id.clone(), *id);
    }
    for e in &official_entries {
        let Some(aspect) = crate::aspect::aspect(&e.pos_raw) else {
            continue;
        };
        let Some(&id) = official_page_ids.get(&e.id) else {
            continue;
        };
        metas[meta_pos[&id]].aspect = Some(aspect.code().to_string());
    }
    for pair in crate::aspect::detect_pairs(&official_entries) {
        let ipf = &official_entries[pair.imperfective];
        let pf = &official_entries[pair.perfective];
        let (Some(&ii), Some(&pi)) = (
            official_page_ids.get(ipf.id.as_str()),
            official_page_ids.get(pf.id.as_str()),
        ) else {
            continue;
        };
        if ii == pi {
            continue;
        }
        let (im, pm) = (meta_pos[&ii], meta_pos[&pi]);
        // The tuple's label is the canonical title of the target page, not an
        // arbitrary folded-equivalent spelling from one official sense row.
        let ipf_title = metas[im].title.clone();
        let pf_title = metas[pm].title.clone();
        metas[im].aspect_partners.push((pi, pf_title));
        metas[pm].aspect_partners.push((ii, ipf_title));
    }
    for m in &mut metas {
        m.aspect_partners.sort();
        m.aspect_partners.dedup();
    }
    // Export-level invariant: aspect belongs only to official verb pages and
    // every emitted partner edge is reciprocal. Fail the build rather than
    // publish a fold-collision or one-way grammatical link.
    for m in metas
        .iter()
        .filter(|m| m.aspect.is_some() || !m.aspect_partners.is_empty())
    {
        anyhow::ensure!(
            m.aspect.is_some() && m.official_lemma.is_some() && m.pos == "verb",
            "aspect metadata leaked onto non-official/non-verb entry {} ({})",
            m.id,
            m.title
        );
        for (partner_id, _) in &m.aspect_partners {
            let Some(&p) = meta_pos.get(partner_id) else {
                anyhow::bail!("aspect partner {} for {} has no entry", partner_id, m.id);
            };
            anyhow::ensure!(
                metas[p].aspect_partners.iter().any(|x| x.0 == m.id),
                "aspect partner link is not reciprocal: {} -> {}",
                m.id,
                partner_id
            );
        }
    }
    compact_entry_categories(&mut metas);
    let meta_by_id: std::collections::HashMap<usize, SiteEntryMeta> =
        metas.iter().map(|m| (m.id, m.clone())).collect();
    let homographs = homograph_groups(&metas);
    let build_meta = BuildMeta::current(metas.len(), lemma_total);
    let curation = load_curation_notes();
    let edges = build_edges(
        &prepared,
        &families,
        &thesaurus,
        &isv_to_id,
        enrich.as_ref(),
        Some(&xref),
        &meta_by_id,
    );
    let backlinks = backlinks_by_target(&edges);

    // ---- Scholarly query layer (issue #73), display/export-side only ----
    // (a) Rule-fired sound-law index: every trace step of every TOP candidate,
    // keyed (engine, rule id). Collected from the same non-suppressed
    // `prepared` rows the generated loop below renders (`trace_block(top)`),
    // so the index can never disagree with what the pages show.
    let rule_index = build_rule_index(
        prepared
            .iter()
            .filter(|p| !p.suppressed)
            .map(|p| (p.id, p.display.as_str(), &p.g)),
    );
    let rule_rows: usize = rule_index.values().map(|a| a.rows.len()).sum();
    println!(
        "rules index: {} rules / {} rows (issue #73).",
        rule_index.len(),
        rule_rows
    );
    // (b) Proto-lemma reflex browse: the proto cache joins each non-borrowed
    // ancestor to its full reconstruction (glosses, descendants, pbs/pie,
    // stem class). Same load-optional posture as the other display caches:
    // absent → feature skipped with a note; present-but-bad → hard error.
    let proto_index = crate::dump::load_optional(
        Path::new(crate::DEFAULT_PROTO_CACHE),
        crate::dump::ProtoIndex::load,
    )?;
    if proto_index.is_none() {
        println!(
            "(no {} — skipping proto-lemma reflex pages; run extract-proto to build it)",
            crate::DEFAULT_PROTO_CACHE
        );
    }
    let proto_reflex = build_proto_reflex_index(
        proto_index.as_ref(),
        prepared
            .iter()
            .filter(|p| !p.suppressed)
            .map(|p| (p.id, &p.g.set)),
    );
    println!(
        "proto pages: {} pages / {} linked entries / {} lookup misses (issue #73).",
        proto_reflex.pages.len(),
        proto_reflex.linked,
        proto_reflex.misses,
    );

    // Run the SAME pair-generation path benchmarked by `aspect-eval` in the
    // production export. This machine-readable artifact makes pair repair an
    // actual shipped model, not benchmark-only analysis; official forms remain
    // the authoritative page titles.
    let mut aspect_pair_exports = Vec::new();
    for pair in crate::aspect::detect_pairs(&official_entries) {
        let ipf = &official_entries[pair.imperfective];
        let pf = &official_entries[pair.perfective];
        let Some(prediction) = crate::aspect::generate_pair(
            &build_input(ipf),
            &build_input(pf),
            proto_index.as_ref(),
            &cfg,
            crate::aspect::AspectConfig::production(),
        ) else {
            continue;
        };
        aspect_pair_exports.push((
            ipf.id.clone(),
            pf.id.clone(),
            official_page_ids.get(ipf.id.as_str()).copied(),
            official_page_ids.get(pf.id.as_str()).copied(),
            ipf.isv.trim().to_string(),
            pf.isv.trim().to_string(),
            prediction,
        ));
    }
    println!(
        "aspect model: generated {} ipf↔pf pairs through the production pair path (issue #75).",
        aspect_pair_exports.len()
    );

    write_wiki_indexes(
        out_dir,
        &metas,
        &edges,
        &backlinks,
        &homographs,
        &build_meta,
        &curation,
        &rule_index,
        proto_index.as_ref(),
        &proto_reflex,
    )?;
    // Some special pages intentionally probe inflection failures. Keep the final
    // export note about blank cells limited to the actual entry pages rendered below.
    INFLECTION_PANICS.store(0, std::sync::atomic::Ordering::Relaxed);

    // (d) Derivational-suffix browse (issue #73): `derivation_block` reports
    // every row it renders into this collector; the deriv/ pages are written
    // after the official-only loop, once BOTH render passes have contributed.
    let mut deriv_rows: BTreeMap<&'static str, DerivAgg> = BTreeMap::new();

    // Second pass: render pages (with family links) + the search index.
    // Script census (issue #66): a generated display headword must never carry
    // Cyrillic — count and report loudly if the normalization hygiene ever
    // regresses (sh dual-script lemmas, homoglyph protos).
    let mut cyrillic_displays = 0usize;
    for (i, p) in prepared.iter().enumerate() {
        if p.suppressed {
            continue;
        }
        if p.display.chars().any(crate::normalize::is_cyrillic_char) {
            cyrillic_displays += 1;
            eprintln!(
                "WARNING: generated display contains Cyrillic: id {} {:?} (issue #66 class)",
                p.id, p.display
            );
        }
        let family = family_block(i, &prepared, &families);
        // Synonyms only on official-headword pages, where the thesaurus lemma's
        // meaning matches (a form-collision homograph page would otherwise show
        // the official lemma's synonyms for a different sense).
        let matched_entry = p.matched.map(|m| &official_entries[m.entry]);
        let synonyms = match matched_entry {
            Some(e) => synonyms_block(e.isv.trim(), &thesaurus, &isv_to_id),
            None => String::new(),
        };
        // Word-formation family from the display headword: the official lemma
        // with its OFFICIAL part of speech when matched (the form-only match can
        // cross POS), else the reconstruction — marked as such in the block.
        let derivation = match matched_entry {
            Some(e) => {
                derivation_block(e.isv.trim(), e.pos, &isv_to_id, true, p.id, &mut deriv_rows)
            }
            None => derivation_block(
                p.g.form(),
                p.g.set.pos,
                &isv_to_id,
                false,
                p.id,
                &mut deriv_rows,
            ),
        };
        let meta = meta_by_id.get(&p.id).expect("generated entry meta");
        // Razumlivost basis (issue #86 defect 2): a MATCHED entry unions the
        // corpus cognate membership with the committee's own sameInLanguages
        // attestation of the matched official row — either basis alone
        // under-reads one tail (aloe: corpus=ru → 52% where same_in "v z j"
        // implies ~99%; vojevodstvo: same_in-only would crater a corpus-backed
        // ~99% to 0%). Non-matched entries keep the corpus basis. DISPLAY
        // ONLY: sameInLanguages never feeds extraction/grouping/evidence.
        let razum_codes: Vec<String> = match matched_entry {
            Some(e) => union_razum_codes(&meta.languages, &e.same_in_langs()),
            None => meta.languages.clone(),
        };
        // Predok infobox link to the proto-lemma reflex page (issue #73b),
        // gated on THIS entry's membership — the target page is guaranteed
        // to list the entry (never a slug-coincidence lexeme).
        let proto_link = proto_reflex
            .membership
            .get(&p.id)
            .map(|sl| format!(" <a href='../proto/{sl}.html'>(rekonstrukcija)</a>"))
            .unwrap_or_default();
        let wiki_top = entry_tabs(meta) + &homograph_notice(meta, &homographs);
        let wiki_bottom = entry_wiki_blocks(
            meta,
            backlinks.get(&p.id).map(Vec::as_slice).unwrap_or(&[]),
            &edges,
            &curation,
            &build_meta,
        );
        let official_pg = matched_entry.map(|e| (e.pos, e.noun_traits.gender));
        let official_display = matched_entry.map(OfficialDisplay::from_entry);
        let official_disp = official_display.as_ref();
        let html = corpus_entry_page(
            p.id,
            &p.g,
            p.status,
            p.matched.map(|m| {
                let e = &official_entries[m.entry];
                (m.rank, e.isv.trim(), e.english.as_str())
            }),
            official_pg,
            official_disp,
            &family,
            enrich.as_ref(),
            Some(&xref),
            &raw_plan.xref,
            &synonyms,
            &derivation,
            &wiki_top,
            meta,
            &razum_codes,
            &raw_credit_line(raw_plan.credit.get(&p.id)),
            &wiki_bottom,
            &proto_link,
        );
        std::fs::write(entry_dir.join(format!("{}.html", p.id)), html)?;

        let mut keys = search_keys(&p.g.candidates, &p.display);
        // On an official-headword (matched) entry, make the official English gloss
        // searchable too — it is already searchable on official-only pages, so
        // this closes the parity gap without touching the entry HTML.
        if let Some(e) = matched_entry {
            for tok in crate::dump::gloss_tokens(&e.english) {
                if tok.chars().count() >= 3 && !keys.iter().any(|(k, _)| k == &tok) {
                    keys.push((tok, 6));
                }
            }
        }
        // Slavic source/cognate aliases (issue #31): the generated set's cognate
        // members, plus — when this row sits under an official headword — the
        // committee's own per-language cells (which may list languages/variants
        // the set didn't carry). Verbatim dictionary evidence, deduped.
        let mut aliases: Vec<SourceAlias> = Vec::new();
        let mut alias_seen: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        collect_source_aliases(
            p.g.set
                .members
                .iter()
                .map(|m| (m.lang.as_str(), m.word.as_str())),
            &mut aliases,
            &mut alias_seen,
        );
        if let Some(e) = matched_entry {
            collect_source_aliases(official_cell_pairs(e), &mut aliases, &mut alias_seen);
        }
        // Search row schema — one 14-element positional array per entry,
        // emitted identically by THREE loops (generated / official-only / raw),
        // written into first-letter shards by `write_search_index` (issue #71),
        // and read by SEARCH_JS + the spotlight/random widgets. Keep all five
        // sides in lock-step:
        //   0 id · 1 display · 2 gloss (truncated 70) · 3 pos code ·
        //   4 status O/N/R · 5 confidence V/S/N · 6 keys [[key,rank],…]
        //   (rank 1-5 = candidate deep-link anchor, 6 = gloss-token sentinel,
        //   no anchor) · 7 n_langs · 8 n_branches · 9 borrowed 0/1 ·
        //   10 quality label · 11 proto ancestor · 12 razumlivost % (integer
        //   0-100, issue #79; basis = cognate members on generated rows —
        //   UNIONED with the matched official row's sameInLanguages on
        //   matched rows (issue #86) — the attesting language on raw rows,
        //   the committee's sameInLanguages on official-only rows — null
        //   there when that column is empty) ·
        //   13 source aliases [[lang,word,[folds]],…]
        //   (issue #31; MUST stay last — SearchRow splits head/aliases on it).
        let gloss70 = truncate(&meta.gloss, 70);
        search_rows.push(SearchRow {
            id: p.id,
            head: format!(
                "[{},{},{},{},{},{},{},{},{},{},{},{},{}",
                p.id,
                json_str(&p.display),
                json_str(&gloss70),
                json_str(&meta.pos),
                json_str(if p.matched.is_some() { "O" } else { "N" }),
                json_str(conf_letter(p.g.confidence)),
                keys_json(&keys),
                p.g.n_langs,
                p.g.n_branches,
                if p.g.set.borrowed { 1 } else { 0 },
                json_str(quality_label(meta)),
                json_str(&meta.ancestor),
                razum_pct(&razum_codes),
            ),
            aliases: source_aliases_json(&aliases),
            core: true,
            buckets: search_row_buckets(&p.display, &gloss70, &keys, &aliases),
        });
        rows.push(HomeRow {
            // sort the home list by coverage (n_langs) so the best-attested show first
            freq: p.g.n_langs as f32 + p.g.n_branches as f32 / 10.0,
            id: p.id,
            form: p.display.clone(),
            gloss: meta.gloss.clone(),
            pos: meta.pos.clone(),
            status: p.status,
            conf: p.g.confidence,
            score: p.g.score,
            prob: meta.prob,
        });
    }
    if cyrillic_displays > 0 {
        println!(
            "WARNING: {cyrillic_displays} generated display headwords contain Cyrillic letters (issue #66 class — see stderr for the list)."
        );
    } else {
        println!("script census: all generated display headwords are Latin (issue #66).");
    }

    // Official lemmas no candidate generates: still searchable, clearly badged
    // as official-but-not-yet-derivable, with the official cognate evidence.
    // Multi-word lemmas (`pęt na desęte`) and reflexives (`… sę`) are included
    // (the single-token generator never produces them, so they would otherwise
    // have no page at all) — display-only parity, generation is untouched.
    for (oid, e) in &official_only_records {
        let isv = e.isv.trim();
        let fold = crate::orthography::to_standard(&isv.to_lowercase());
        let syn = synonyms_block(isv, &thesaurus, &isv_to_id);
        let deriv = derivation_block(isv, e.pos, &isv_to_id, true, *oid, &mut deriv_rows);
        let meta = meta_by_id.get(oid).expect("official-only entry meta");
        let wiki_top = entry_tabs(meta) + &homograph_notice(meta, &homographs);
        let wiki_bottom = entry_wiki_blocks(
            meta,
            backlinks.get(oid).map(Vec::as_slice).unwrap_or(&[]),
            &edges,
            &curation,
            &build_meta,
        );
        let html = official_only_page(
            isv,
            e,
            enrich.as_ref(),
            Some(&xref),
            &raw_plan.xref,
            *oid,
            &syn,
            &deriv,
            &wiki_top,
            meta,
            &raw_credit_line(raw_plan.credit.get(oid)),
            &wiki_bottom,
        );
        std::fs::write(entry_dir.join(format!("{oid}.html")), html)?;
        let mut keys: Vec<(String, usize)> = Vec::new();
        for k in [fold.clone(), crate::orthography::ascii_skeleton(isv)] {
            if k.chars().count() >= 2
                && k != isv.to_lowercase()
                && !keys.iter().any(|(kk, _)| kk == &k)
            {
                keys.push((k, 1));
            }
        }
        // The committee's per-language translations (issue #31): this makes an
        // official-only lemma findable by any of its Slavic cognate spellings —
        // Cyrillic or Latinized — plus `de`/`nl`/`eo` as lower-weight
        // international aliases. Verbatim dictionary evidence, not a claim.
        let mut aliases: Vec<SourceAlias> = Vec::new();
        let mut alias_seen: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        collect_source_aliases(official_cell_pairs(e), &mut aliases, &mut alias_seen);
        // Same 14-element row schema as the generated loop above. Element 12
        // comes from the committee's own sameInLanguages attestation — the
        // translation cells are filled for every language and would claim a
        // constant ~99%; null when the column is empty (the client guards).
        let razum = {
            let same_in = e.same_in_langs();
            if same_in.is_empty() {
                "null".to_string()
            } else {
                (crate::lang::razumlivost(&same_in).overall.round() as u32).to_string()
            }
        };
        let gloss70 = truncate(&e.english, 70);
        search_rows.push(SearchRow {
            id: *oid,
            head: format!(
                "[{},{},{},{},{},{},{},{},{},{},{},{},{}",
                oid,
                json_str(isv),
                json_str(&gloss70),
                json_str(e.pos.code()),
                json_str("O"),
                json_str("V"),
                keys_json(&keys),
                meta.n_langs,
                meta.n_branches,
                if meta.borrowed { 1 } else { 0 },
                json_str(quality_label(meta)),
                json_str(&meta.ancestor),
                razum,
            ),
            aliases: source_aliases_json(&aliases),
            core: true,
            buckets: search_row_buckets(isv, &gloss70, &keys, &aliases),
        });
        rows.push(HomeRow {
            freq: 0.5,
            id: *oid,
            form: isv.to_string(),
            gloss: e.english.clone(),
            pos: e.pos.code().to_string(),
            status: MatchStatus::OfficialMatch,
            conf: Confidence::High,
            score: 1.0,
            prob: None,
        });
    }

    // Raw Slavic Wiktionary lemmas (issue #34, PR-2): a THIRD, SITE-ONLY loop,
    // after the generated and official-only loops, before the search index is
    // written as shards.
    // Every low-evidence dictionary word that no generated/official page already
    // covers gets a page + search row, badged as a raw attestation. These entries
    // are NEVER verification-grade: this loop touches neither `lemma_sink`/
    // `form_sink` nor any paradigm emission (those already emitted only from
    // `prepared`/`official_only_records`), so the forms API stays byte-identical.
    // They also stay out of the wiki/homograph/graph indexes (already written) and
    // the home list. Only the English-dump glosses + raw etymology are shown; the
    // native RU/PL/CS merge is a later PR. Skipped gracefully when the cache is
    // absent, so a checkout without the 68 MB cache still exports cleanly.
    let (mut raw_rendered, mut raw_deduped) = (0usize, 0usize);
    // Flavorization validation residue (spec §2 stage 5): rendered raw
    // headwords whose letters fall outside the ISV standard alphabet get
    // counted and reported loudly, never silently shipped.
    let mut flavor_residue_words = 0usize;
    let mut flavor_residue: BTreeMap<char, usize> = BTreeMap::new();
    match &raw_corpus {
        Some(raw_corpus) => {
            raw_deduped = raw_plan.deduped;
            // Cross-lingual "same meaning" index (reverse gloss links): every raw
            // + benchmark lemma's English gloss tokens -> its (lang, word), so each
            // raw page can show the words for its meaning(s) in other Slavic
            // languages. Display-only; approximate (bridged by shared English gloss).
            let mut gx = crate::glossxref::GlossXref::new();
            for l in &raw_corpus.lemmas {
                gx.add(&l.lang, &l.word, &l.glosses);
            }
            for e in &corpus.entries {
                gx.add(&e.lang, &e.word, std::slice::from_ref(&e.gloss));
            }
            gx.finalize();
            // Rendered-vs-deduped and the id sequence were decided by the raw
            // pre-pass above (`plan_raw_pages`, wrapping `raw_lemma_fate` — the
            // single dedup rule shared with the `coverage` command — so export
            // and coverage reconcile by construction and can never drift).
            for &(lemma_idx, id) in &raw_plan.pages {
                let lemma = &raw_corpus.lemmas[lemma_idx];
                let word = lemma.word.trim();
                // Display headword: the attested word flavorized into ISV
                // orthography (winyl→vinyl, дело→dělo; issue #62 /
                // data/FLAVORIZATION_SPEC.md). MUST stay the same call as in
                // `raw_lemma_fate`, which deduped on this display's fold.
                let display = crate::flavorize::flavorize_word(&lemma.lang, &lemma.pos, word);
                let mut had_residue = false;
                for c in crate::flavorize::residue_chars(&display) {
                    *flavor_residue.entry(c).or_default() += 1;
                    had_residue = true;
                }
                if had_residue {
                    flavor_residue_words += 1;
                }
                let gloss = lemma.glosses.join("; ");
                let meta = {
                    let mut m = entry_meta(
                        id,
                        &display,
                        &gloss,
                        &lemma.pos,
                        MatchStatus::NoOfficialEntry,
                        Confidence::Low,
                        0.0,
                        None,
                        1,
                        1,
                        false,
                        false,
                        None,
                        String::new(),
                        vec![lemma.lang.clone()],
                        Vec::new(),
                    );
                    m.raw = true;
                    m
                };
                let html = raw_lemma_page(
                    &display,
                    lemma,
                    id,
                    &meta,
                    enrich.as_ref(),
                    &gx,
                    Some(&xref),
                    &raw_plan.xref,
                );
                std::fs::write(entry_dir.join(format!("{id}.html")), html)?;

                // Search row (14 elements; schema documented at the generated
                // loop). Status char 'R'; the folds of the display headword are
                // keys; e[13] carries the verbatim attested spelling (Cyrillic
                // пластинка) + its Latin fold so a query in either script finds
                // the page via the client aliasMatch.
                let mut keys: Vec<(String, usize)> = Vec::new();
                let disp_lower = display.to_lowercase();
                for k in [
                    crate::orthography::to_standard(&disp_lower),
                    crate::orthography::ascii_skeleton(&display),
                ] {
                    if k.chars().count() >= 2
                        && k != disp_lower
                        && !keys.iter().any(|(kk, _)| kk == &k)
                    {
                        keys.push((k, 1));
                    }
                }
                let mut aliases: Vec<SourceAlias> = Vec::new();
                let mut alias_seen: std::collections::HashSet<(String, String)> =
                    std::collections::HashSet::new();
                collect_source_aliases(
                    std::iter::once((lemma.lang.as_str(), word)),
                    &mut aliases,
                    &mut alias_seen,
                );
                // Same 14-element row schema as the generated loop above; the
                // razumlivost element covers the single attesting language.
                let gloss70 = truncate(&gloss, 70);
                search_rows.push(SearchRow {
                    id,
                    head: format!(
                        "[{},{},{},{},{},{},{},{},{},{},{},{},{}",
                        id,
                        json_str(&display),
                        json_str(&gloss70),
                        json_str(&lemma.pos),
                        json_str("R"),
                        json_str("N"),
                        keys_json(&keys),
                        1,
                        1,
                        0,
                        json_str(quality_label(&meta)),
                        json_str(&meta.ancestor),
                        razum_pct(&meta.languages),
                    ),
                    aliases: source_aliases_json(&aliases),
                    core: false,
                    buckets: search_row_buckets(&display, &gloss70, &keys, &aliases),
                });
                raw_rendered += 1;
            }
            println!(
                "wrote {raw_rendered} raw Wiktionary attestation pages (site-only, low-evidence; {raw_deduped} deduped against generated/raw pages)."
            );
            // Loud flavorization validation (spec §2 stage 5 / issue #62).
            if flavor_residue_words > 0 {
                let top: Vec<String> = flavor_residue
                    .iter()
                    .map(|(c, n)| format!("{c}×{n}"))
                    .take(8)
                    .collect();
                println!(
                    "flavorize: {flavor_residue_words}/{raw_rendered} raw headwords carry non-ISV residue letters ({})",
                    top.join(", ")
                );
            } else {
                println!("flavorize: all {raw_rendered} raw headwords are ISV-alphabet-clean.");
            }
        }
        None => {
            println!(
                "(no {} — skipping raw Wiktionary attestation pages; run extract-raw-slavic to build it)",
                crate::DEFAULT_RAW_LEMMA_CACHE
            );
        }
    }

    let (shard_count, browse_count) = write_search_index(out_dir, &search_rows)?;
    println!(
        "search index: {} rows into {shard_count} first-letter shards + {browse_count} core browse rows (issue #71).",
        search_rows.len()
    );
    // The monolithic search.json is retired; remove a stale copy so old
    // clients can't silently read a frozen index.
    let _ = std::fs::remove_file(out_dir.join("search.json"));
    std::fs::write(out_dir.join("wiktionary.css"), css())?;
    std::fs::write(out_dir.join(".nojekyll"), "")?;

    // ---- Novel-word proposal pipeline (Track C / issue #3) ----
    // Only a calibrator fitted on THIS corpus-coverage score domain may assign
    // probabilities or proposal buckets. Until that artifact exists, publish
    // an empty worklist rather than relabel the official-row pipeline's
    // operating points as evidence for this different model (issue #89 J26).
    let mut proposals: Vec<ProposalRow> = Vec::new();
    for p in &prepared {
        if p.suppressed || p.matched.is_some() {
            continue;
        }
        let form = p.g.form();
        if form.is_empty() || form.contains(' ') || form.chars().count() < 3 {
            continue;
        }
        // A homograph-demoted entry has `matched` cleared but its form IS an
        // official lemma — never propose a word the dictionary already has.
        if official_by_fold.contains_key(&crate::orthography::to_standard(&form.to_lowercase())) {
            continue;
        }
        let Some(cal) = calibration.as_ref() else {
            continue;
        };
        let prob = cal.probability(p.g.score);
        if prob >= crate::calibrate::REVIEW_T {
            proposals.push(ProposalRow {
                id: p.id,
                form: form.to_string(),
                pos: p.g.set.pos.code().to_string(),
                prob,
                ancestor: p.g.set.etymon.clone(),
                n_langs: p.g.n_langs,
                n_branches: p.g.n_branches,
                langs: {
                    let mut l: Vec<String> =
                        p.g.set.members.iter().map(|m| m.lang.clone()).collect();
                    l.sort();
                    l.dedup();
                    l
                },
                gloss: p.g.set.gloss.clone(),
            });
        }
    }
    proposals.sort_by(|a, b| b.prob.total_cmp(&a.prob).then(a.id.cmp(&b.id)));
    let mut tsv =
        String::from("form\tpos\tprobability\tbucket\tancestor\tn_langs\tn_branches\tgloss\n");
    for r in &proposals {
        // Buckets are only meaningful in calibrated-probability space.
        let bucket = if r.prob >= crate::calibrate::PROPOSE_T {
            "predlog"
        } else {
            "pregled"
        };
        let _ = write!(
            tsv,
            "{}\t{}\t{:.3}\t{}\t{}\t{}\t{}\t{}\n",
            r.form,
            r.pos,
            r.prob,
            bucket,
            r.ancestor,
            r.n_langs,
            r.n_branches,
            r.gloss.replace(['\t', '\n'], " "),
        );
    }
    // Committed data artifact AND a served copy, so the page's download link
    // works on the static host.
    std::fs::write("data/novel-words.tsv", &tsv)?;
    std::fs::write(out_dir.join("novel-words.tsv"), &tsv)?;
    std::fs::write(
        out_dir.join("proposals.html"),
        proposals_page(&proposals, calibration.as_ref(), &curation),
    )?;

    // ---- Lexical verification API (issue #11) ----
    // One FormRecord pipeline: lemma records for every page headword, full
    // paradigm records for official headwords (an inflected form of a machine
    // reconstruction would be confidently wrong — generated lemmas contribute
    // their citation form with the calibrated probability instead). Written as
    // the sharded static API under api/ plus meta.json and the agent guide.
    let mut form_sink = crate::forms::RecordSink::default();
    let mut lemma_sink = crate::forms::RecordSink::default();
    crate::forms::closed_class_records(&mut form_sink);
    crate::forms::closed_class_records(&mut lemma_sink);
    let mut seen_paradigm: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Attested bases (official + official-only lemmas, with their OFFICIAL POS,
    // page id, and gloss) to derive `generated` families off (issue #37). Never
    // reconstructions: deriving off a wrong root inherits the ~33% wrong-root cap.
    let mut attested_bases: Vec<(String, crate::model::Pos, usize, String)> = Vec::new();
    for p in &prepared {
        if p.suppressed {
            continue;
        }
        let (headword, status, gloss): (String, &'static str, String) = match p.matched {
            Some(m) => {
                let e = &official_entries[m.entry];
                (e.isv.trim().to_string(), "official", e.english.clone())
            }
            None => (p.g.form().to_string(), "generated", p.g.set.gloss.clone()),
        };
        if headword.is_empty() || headword.contains('!') {
            continue;
        }
        // Sanitize the citation: generated forms can carry raw pipeline
        // notation ("pleskati,*plěskati"), official ones government hints
        // ("pozirati (na)") — neither belongs in a lookup key.
        let Some(headword) = crate::forms::citation(&headword) else {
            continue;
        };
        let prob = if status == "generated" {
            calibration.as_ref().map(|c| c.probability(p.g.score))
        } else {
            None
        };
        // A matched headword's paradigm must use the OFFICIAL part of speech —
        // the form-only official match can cross POS, and a wrong-POS paradigm
        // exported as verification-grade would be confidently wrong.
        let (pos, gender) = match p.matched {
            Some(m) => {
                let e = &official_entries[m.entry];
                if crate::aspect::aspect(&e.pos_raw).is_some() {
                    (crate::model::Pos::Verb, None)
                } else {
                    (e.pos, e.noun_traits.gender)
                }
            }
            None => (p.g.set.pos, None),
        };
        lemma_sink.add(
            &headword,
            "",
            &headword,
            p.id,
            pos.code(),
            "lemma",
            status,
            prob,
            &gloss,
        );
        form_sink.add(
            &headword,
            "",
            &headword,
            p.id,
            pos.code(),
            "lemma",
            status,
            prob,
            &gloss,
        );
        if status == "official" && seen_paradigm.insert(format!("{headword}|{}", pos.code())) {
            crate::forms::paradigm_records(
                &mut form_sink,
                &headword,
                pos,
                gender,
                p.id,
                "official",
                None,
                &gloss,
            );
            crate::forms::pronoun_numeral_records(
                &mut form_sink,
                &headword,
                pos,
                p.id,
                "official",
                &gloss,
            );
        }
        // An attested (official-matched) base: derive its family later. The
        // reconstruction path (status == "generated") is deliberately excluded.
        if status == "official" {
            attested_bases.push((headword.clone(), pos, p.id, gloss.clone()));
        }
    }
    for (oid, e) in &official_only_records {
        let api_pos = if crate::aspect::aspect(&e.pos_raw).is_some() {
            crate::model::Pos::Verb
        } else {
            e.pos
        };
        // ~230 rows list byform variants in one cell ("iměti, imati"): each
        // variant is its own lemma (and gets its own paradigm).
        for isv in e.isv.split(',').map(str::trim) {
            if isv.is_empty() || isv.contains('#') || isv.contains('!') {
                continue;
            }
            let Some(clean) = crate::forms::citation(isv) else {
                continue;
            };
            let isv = clean.as_str();
            lemma_sink.add(
                isv,
                "",
                isv,
                *oid,
                api_pos.code(),
                "lemma",
                "official-only",
                None,
                &e.english,
            );
            form_sink.add(
                isv,
                "",
                isv,
                *oid,
                api_pos.code(),
                "lemma",
                "official-only",
                None,
                &e.english,
            );
            if seen_paradigm.insert(format!("{isv}|{}", api_pos.code())) {
                crate::forms::paradigm_records(
                    &mut form_sink,
                    isv,
                    api_pos,
                    e.noun_traits.gender,
                    *oid,
                    "official-only",
                    None,
                    &e.english,
                );
                crate::forms::pronoun_numeral_records(
                    &mut form_sink,
                    isv,
                    api_pos,
                    *oid,
                    "official-only",
                    &e.english,
                );
            }
            attested_bases.push((isv.to_string(), api_pos, *oid, e.english.clone()));
        }
    }
    // ---- Generated derivatives off attested bases (issue #37) ----
    // Every official / official-only lemma's regular family is derived; a
    // derivative ships ONLY if its folded key is absent from the form index
    // (dedup against attested INFLECTED forms and already-emitted lemmas, not
    // just headwords), as `status = "generated"`, `source = "lemma"`, with a
    // per-pattern holdout-fit probability and NO inflected paradigm. Pure, so
    // the export stays byte-reproducible.
    let deriv_probs = crate::derive::pattern_probabilities(&official_entries);
    let mut taken = form_sink.form_key_set();
    let deriv_added = inject_generated_derivatives(
        &mut form_sink,
        &mut lemma_sink,
        &mut taken,
        &attested_bases,
        &deriv_probs,
    );
    println!("api: added {deriv_added} generated derivative lemmas off attested bases (issue #37)");
    let form_records = form_sink.into_records();
    let lemma_records = lemma_sink.into_records();
    // Semantic-trap notes for the web text-checker (same file the CLI reads),
    // re-keyed by folded form so the client looks up by key directly.
    if let Ok(raw) = std::fs::read_to_string(crate::check::SEMANTIC_NOTES) {
        if let Ok(parsed) = serde_json::from_str::<
            std::collections::BTreeMap<String, crate::check::SemanticNote>,
        >(&raw)
        {
            let mut js = String::from("{");
            for (i, (k, v)) in parsed.iter().enumerate() {
                if i > 0 {
                    js.push(',');
                }
                let _ = write!(
                    js,
                    "{}:{{\"warning\":{},\"prefer\":[{}]}}",
                    serde_json::to_string(&crate::forms::form_key(k))?,
                    serde_json::to_string(&v.warning)?,
                    v.prefer
                        .iter()
                        .map(|p| serde_json::to_string(p).unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join(",")
                );
            }
            js.push_str("}\n");
            std::fs::create_dir_all(out_dir.join("api"))?;
            std::fs::write(out_dir.join("api").join("notes.json"), js)?;
        }
    }
    let aspect_api: crate::forms::AspectMeta = metas
        .iter()
        .filter_map(|m| {
            m.aspect
                .as_ref()
                .map(|a| (m.id, (a.clone(), m.aspect_partners.clone())))
        })
        .collect();
    let mut pair_json = String::from("{\"schema_version\":3,\"pairs\":[\n");
    for (n, (ipf_oid, pf_oid, ipf_page, pf_page, ipf, pf, prediction)) in
        aspect_pair_exports.iter().enumerate()
    {
        if n > 0 {
            pair_json.push_str(",\n");
        }
        let ipf_page = ipf_page.map_or_else(|| "null".to_string(), |id| id.to_string());
        let pf_page = pf_page.map_or_else(|| "null".to_string(), |id| id.to_string());
        let ipf_present = crate::aspect::ovati_present_stem(&prediction.imperfective)
            .map(|s| json_str(&s))
            .unwrap_or_else(|| "null".to_string());
        let pf_present = crate::aspect::ovati_present_stem(&prediction.perfective)
            .map(|s| json_str(&s))
            .unwrap_or_else(|| "null".to_string());
        let _ = write!(
            pair_json,
            "{{\"imperfective\":{{\"official_id\":{},\"entry_id\":{},\"lemma\":{},\"generated\":{},\"generated_present_stem\":{}}},\"perfective\":{{\"official_id\":{},\"entry_id\":{},\"lemma\":{},\"generated\":{},\"generated_present_stem\":{}}},\"rule\":{}}}",
            json_str(ipf_oid),
            ipf_page,
            json_str(ipf),
            json_str(&prediction.imperfective),
            ipf_present,
            json_str(pf_oid),
            pf_page,
            json_str(pf),
            json_str(&prediction.perfective),
            pf_present,
            json_str(prediction.rule),
        );
    }
    pair_json.push_str("\n]}\n");
    std::fs::create_dir_all(out_dir.join("api"))?;
    std::fs::write(out_dir.join("api/aspect-pairs.json"), &pair_json)?;
    let api_counts = crate::forms::write_api(
        out_dir,
        &form_records,
        &lemma_records,
        &aspect_api,
        pair_json.len(),
        &build_meta.git,
        &crate::forms::agent_guide(),
    )?;
    println!(
        "api: {} form records / {} distinct keys / {} lemmas across {} shards ({} KB total, largest shard {} KB)",
        api_counts.records,
        api_counts.keys,
        api_counts.lemmas,
        crate::forms::SHARDS,
        api_counts.bytes / 1024,
        api_counts.largest_shard / 1024,
    );

    // (d) Derivational-suffix browse pages (issue #73): the rows the two
    // render loops reported, plus the SAME per-pattern Wilson-95 probability
    // the API's generated derivatives ship (`deriv_probs` above — one fit
    // serves both surfaces).
    let deriv_row_total = write_deriv_pages(out_dir, &deriv_rows, &deriv_probs)?;
    println!(
        "deriv pages: {} patterns / {deriv_row_total} rows (issue #73).",
        deriv_rows.len(),
    );

    rows.sort_by(|a, b| b.freq.total_cmp(&a.freq));
    let home = corpus_home(
        n,
        lemma_total,
        high,
        med,
        low,
        official,
        official_only,
        borrowed,
        &rows,
    );
    std::fs::write(out_dir.join("index.html"), home)?;
    std::fs::write(out_dir.join("search.html"), search_page())?;
    std::fs::write(out_dir.join("forms.html"), forms_page())?;
    std::fs::write(out_dir.join("text-check.html"), text_check_page())?;
    std::fs::write(
        out_dir.join("about.html"),
        corpus_about(n, lemma_total, official),
    )?;
    std::fs::write(
        out_dir.join("metrics.html"),
        metrics_page(calibration.as_ref()),
    )?;

    // Dataset-coverage page (issue #35): documents which Slavic-Wiktionary datasets
    // feed the site and the inclusion/exclusion counts. The extraction tally is the
    // deterministic companion `extract-raw-slavic` wrote; the raw render/dedup split
    // is this export's own count, so the page reconciles with `search.json`.
    let raw_cov_stats = crate::dump::RawCoverageStats::load(
        &Path::new(crate::DEFAULT_RAW_LEMMA_CACHE).with_file_name(crate::dump::RAW_COVERAGE_FILE),
    )
    .ok();
    let cov_section = datasets_coverage_section(
        raw_cov_stats.as_ref(),
        raw_rendered,
        raw_deduped,
        n,
        official_only,
    );
    std::fs::write(out_dir.join("datasets.html"), datasets_page(&cov_section))?;

    let panics = INFLECTION_PANICS.load(std::sync::atomic::Ordering::Relaxed);
    println!(
        "wrote {n} cognate-word pages + {official_only} official-only pages ({high} high / {med} medium / {low} low confidence; {official} match an official ISV form){}",
        if panics > 0 { format!("; {panics} inflection cells blank") } else { String::new() }
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Raw-lemma coverage reporting (issue #35)
//
// A transparent, auditable account of the RAW Slavic-Wiktionary datasets: which
// data went in, how many words were included, and how many were excluded and
// why. It stitches together three views:
//   1. EXTRACTION coverage — the drop-reason tally `extract-raw-slavic` wrote to
//      `data/raw-slavic-coverage.json` (Slavic pages seen → kept / dropped-by-reason).
//   2. SITE coverage — of the kept lemmas, how many `export` renders as raw-only
//      pages vs dedups against an official/generated headword. This *replicates*
//      the export dedup (same `build_sets`/`generate_set`, homograph + same-concept
//      suppression, xref + display-headword fold), so the `rendered-raw` number
//      must reconcile with the export's actual `R`-status page count.
//   3. NATIVE JOIN — the fraction of raw lemmas that gain native ru/pl/cs
//      enrichment (an `EnrichIndex` hit), by language.
// The report never reads the benchmark path; it only touches the raw corpus and
// the (display-only) site index, keeping the raw path benchmark-isolated.
// ---------------------------------------------------------------------------

/// One raw lemma's fate under the export dedup (site coverage view). The
/// deduped variants carry WHERE the word's content lives (issue #64), so the
/// raw pre-pass can point word chips at that internal page instead of out to
/// the native Wiktionary; `coverage` only distinguishes rendered vs deduped.
#[derive(Clone, PartialEq, Eq)]
enum RawFate {
    /// Gets its own raw page; carries the ě-blind display fold it claimed.
    Rendered { efold: String },
    /// Word or display fold empty — nothing to render or point at.
    Skipped,
    /// Verbatim `(lang, word)` is already a cognate member of an entry — the
    /// cognate `xref` resolves chip links to it, nothing extra to record.
    DedupedXref,
    /// Display fold (or its ě-blind variant) is an official / generated /
    /// official-only headword: `target` is that page's id.
    DedupedFold { target: usize },
    /// An earlier raw lemma claimed the same ě-blind fold; the pre-pass
    /// resolves the fold to that twin's page id.
    DedupedRawTwin { efold: String },
}

/// Minimal replica of `export_corpus`'s per-set state, enough to rebuild the
/// display-headword index (`isv_to_id`) and cognate cross-reference (`xref`)
/// that the raw dedup consults. Kept in lock-step with `export_corpus`.
struct CovPrepared {
    id: usize,
    g: crate::corpus::GeneratedWord,
    display: String,
    matched: Option<(usize, usize)>,
    suppressed: bool,
}

/// Build the identity-safe headword index (`isv_to_id`) and cognate cross-reference
/// (`xref`) exactly as `export_corpus` does, so a raw lemma is judged
/// "already covered" identically. Returns them plus the generated/official
/// headword counts used for the reconciliation lines.
fn build_corpus_render_index(
    corpus: &crate::dump::LemmaCorpus,
    official_entries: &[OfficialEntry],
) -> (
    crate::enrich::Xref,
    HeadwordIndex,
    usize, // generated pages (non-suppressed)
    usize, // official-only pages
) {
    let cfg = ConsensusConfig::production();
    let sets = crate::corpus::build_sets(corpus);

    let mut official_by_exact: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    let mut official_by_fold: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, e) in official_entries.iter().enumerate() {
        let isv = e.isv.trim();
        if isv.is_empty() || isv.contains(' ') || isv.contains('#') {
            continue;
        }
        official_by_exact
            .entry(isv.to_lowercase())
            .or_default()
            .push(i);
        official_by_fold
            .entry(crate::orthography::to_standard(&isv.to_lowercase()))
            .or_default()
            .push(i);
    }

    // First pass: generate every set (same as export).
    let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut prepared: Vec<CovPrepared> = Vec::new();
    let mut id = 0usize;
    for set in sets {
        let g = crate::corpus::generate_set(set, &cfg);
        let form = g.form().to_string();
        if form.is_empty() {
            continue;
        }
        id += 1;
        let matched: Option<(usize, usize)> =
            g.candidates
                .iter()
                .take(5)
                .enumerate()
                .find_map(|(rank, c)| {
                    let lower = c.form.trim().to_lowercase();
                    let rows = if let Some(rows) = official_by_exact.get(&lower) {
                        rows.as_slice()
                    } else {
                        let rows = official_by_fold
                            .get(&crate::orthography::to_standard(&lower))?
                            .as_slice();
                        let mut spellings = rows
                            .iter()
                            .map(|&i| official_entries[i].isv.trim().to_lowercase());
                        let first = spellings.next()?;
                        if spellings.any(|s| s != first) {
                            return None;
                        }
                        rows
                    };
                    let i = select_official_entry(rows, official_entries, g.set.pos, &g.set.gloss)?;
                    Some((rank + 1, i))
                });
        let display = matched
            .map(|(_, i)| official_entries[i].isv.trim().to_string())
            .unwrap_or_else(|| form.clone());
        prepared.push(CovPrepared {
            id,
            g,
            display,
            matched,
            suppressed: false,
        });
    }

    // Homograph / duplicate dedup: one representative per official sense.
    {
        let rank = |p: &CovPrepared, en: &str| -> (usize, i32) {
            let a = crate::dump::gloss_tokens(&p.g.set.gloss);
            let b = crate::dump::gloss_tokens(en);
            let overlap = a.iter().filter(|t| b.contains(t)).count();
            (overlap, (p.g.score * 1000.0) as i32)
        };
        let mut best: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for (i, p) in prepared.iter().enumerate() {
            if let Some((_, entry)) = p.matched {
                let e = &official_entries[entry];
                let key = e.id.clone();
                let win = match best.get(&key) {
                    Some(&j) => rank(p, &e.english) > rank(&prepared[j], &e.english),
                    None => true,
                };
                if win {
                    best.insert(key, i);
                }
            }
        }
        for (i, p) in prepared.iter_mut().enumerate() {
            let Some((_, entry)) = p.matched else {
                continue;
            };
            let key = official_entries[entry].id.clone();
            if best.get(&key) != Some(&i) {
                p.matched = None;
                p.display = p.g.form().to_string();
            }
        }
    }

    // Same-concept suppression: collapse duplicate pages sharing a folded form and
    // a gloss token with a stronger set.
    {
        let gloss_of = |p: &CovPrepared| -> Vec<String> {
            match p.matched {
                Some((_, entry)) => crate::dump::gloss_tokens(&official_entries[entry].english),
                None => crate::dump::gloss_tokens(&p.g.set.gloss),
            }
        };
        let rank = |p: &CovPrepared| (p.matched.is_some(), (p.g.score * 1000.0) as i32);
        let mut by_form: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, p) in prepared.iter().enumerate() {
            by_form
                .entry(crate::orthography::to_standard(&p.g.form().to_lowercase()))
                .or_default()
                .push(i);
        }
        for (_f, mut group) in by_form {
            if group.len() < 2 {
                continue;
            }
            group.sort_by(|&a, &b| rank(&prepared[b]).cmp(&rank(&prepared[a])));
            let mut kept: Vec<Vec<String>> = Vec::new();
            for &i in &group {
                let gl = gloss_of(&prepared[i]);
                if !gl.is_empty() && kept.iter().any(|k| gl.iter().any(|t| k.contains(t))) {
                    prepared[i].suppressed = true;
                } else {
                    kept.push(gl);
                }
            }
        }
    }

    // Exact-first, ambiguity-aware display-headword index, and the cognate
    // cross-reference: every member word of every surviving set.
    let mut isv_to_id = HeadwordIndex::default();
    let mut xref = crate::enrich::Xref::new();
    let generated_pages = prepared.iter().filter(|p| !p.suppressed).count();
    for p in &prepared {
        if p.suppressed {
            continue;
        }
        isv_to_id.insert(&p.display, p.id);
        for m in &p.g.set.members {
            xref.insert(&m.lang, &m.word, p.id);
        }
    }

    // Official lemmas no surviving generated page represents: reserve ids and
    // fold them into `isv_to_id`, so raw dedup mirrors the real export.
    covered.clear();
    for p in prepared.iter().filter(|p| !p.suppressed) {
        if let Some((_, entry)) = p.matched {
            covered.insert(official_entries[entry].id.clone());
        }
    }
    let mut official_only = 0usize;
    let mut official_only_records: Vec<(usize, String)> = Vec::new();
    for e in official_entries {
        let isv = e.isv.trim();
        if isv.is_empty() || isv.contains('#') {
            continue;
        }
        if !covered.insert(e.id.clone()) {
            continue;
        }
        id += 1;
        official_only += 1;
        official_only_records.push((id, isv.to_string()));
    }
    for (oid, isv) in &official_only_records {
        isv_to_id.insert(isv, *oid);
    }

    (xref, isv_to_id, generated_pages, official_only)
}

/// The single raw-lemma dedup rule: BOTH `export_corpus`'s raw render loop and
/// the `coverage` command classify through this function, so the rendered/deduped
/// split always reconciles. `raw_covered` carries the running raw-vs-raw dedup
/// set (mutated).
///
/// A lemma is deduped when: its word or display fold is empty (`Skipped`); it
/// is already a cognate member of a generated page (verbatim `(lang, word)`
/// xref match — `DedupedXref`); its display fold is already an official /
/// generated / official-only entry (`DedupedFold`; catches internationalisms
/// like konflikt whose source spelling isn't a cognate member but whose ISV
/// form has a page); or another raw lemma already claimed the same display
/// fold (`DedupedRawTwin`; same word under several POS, cross-language twins)
/// — so each attested ISV spelling gets exactly one page, while distinct words
/// the phonemic fold conflated (vođa / voda) stay separate.
fn raw_lemma_fate(
    lemma: &crate::dump::RawSlavicLemma,
    xref: &crate::enrich::Xref,
    isv_to_id: &HeadwordIndex,
    raw_covered: &mut std::collections::HashSet<String>,
) -> RawFate {
    let word = lemma.word.trim();
    if word.is_empty() {
        return RawFate::Skipped;
    }
    // Any generated-page membership (including an ambiguous homograph/sense
    // key) means this attestation is already represented. Link ambiguity must
    // not create a duplicate raw page.
    if xref.contains(&lemma.lang, word) {
        return RawFate::DedupedXref;
    }
    // Same call as the render loop's display headword, by construction —
    // dedup and display must never diverge (issue #62).
    let display = crate::flavorize::flavorize_word(&lemma.lang, &lemma.pos, word);
    let disp_fold = crate::orthography::to_standard(&display.to_lowercase());
    if disp_fold.is_empty() {
        return RawFate::Skipped;
    }
    // ě-tolerant dedup (spec §6): flavorization can over-mark ě relative to
    // the official jat (ru день→děnj vs official denj), so the official
    // collision check tries both the fold and its ě→e variant, and raw-vs-raw
    // dedup keys on the ě-blind fold (cs město and sr mesto = one page).
    let efold = disp_fold.replace('ě', "e");
    if let Some(target) = isv_to_id
        .resolve(&display)
        .or_else(|| isv_to_id.resolve_fold(&efold))
    {
        return RawFate::DedupedFold { target };
    }
    if !raw_covered.insert(efold.clone()) {
        return RawFate::DedupedRawTwin { efold };
    }
    RawFate::Rendered { efold }
}

/// The raw pre-pass result (issue #64): every rendered raw lemma with its
/// pre-assigned entry id, plus a cross-reference from EVERY raw `(lang, word)`
/// to the internal page that shows it — its own raw page, the official /
/// generated page its display fold collided with, or the earlier raw twin
/// that claimed the same fold. Built before any page renders so word chips on
/// every page (including raw pages rendered early in the loop) can link
/// internally.
#[derive(Default)]
struct RawPlan {
    /// (index into the raw corpus's lemma list, assigned entry id).
    pages: Vec<(usize, usize)>,
    /// (lang, verbatim attested word) → internal entry id. Consulted by word
    /// chips AFTER the cognate `xref` (which resolves generated membership).
    xref: crate::enrich::Xref,
    deduped: usize,
    /// Raw-collision display credit (issue #86 item 6): target entry id →
    /// the raw `(lang, word)` attestations whose display fold deduped onto
    /// that page (RawFate::DedupedFold — the site knew them but showed them
    /// nowhere). Sorted lang-then-word, deduped. DISPLAY ONLY: never counted
    /// in n_langs / Dokaz / razumlivost / the vote — raw evidence stays
    /// benchmark-forbidden by type. (DedupedXref attestations are already
    /// visible as cognate members on their page and are NOT repeated here.)
    credit: std::collections::BTreeMap<usize, Vec<(String, String)>>,
}

/// Classify every raw lemma once (via [`raw_lemma_fate`] — still the single
/// dedup rule shared with `coverage`), assigning sequential ids from
/// `next_id + 1` to the rendered ones in corpus order — the same ids the old
/// in-loop allocation produced.
fn plan_raw_pages(
    lemmas: &[crate::dump::RawSlavicLemma],
    xref: &crate::enrich::Xref,
    isv_to_id: &HeadwordIndex,
    mut next_id: usize,
) -> RawPlan {
    let mut plan = RawPlan::default();
    let mut raw_covered: std::collections::HashSet<String> = std::collections::HashSet::new();
    // ě-blind fold → the raw page id that claimed it (for twin resolution).
    let mut fold_owner: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, lemma) in lemmas.iter().enumerate() {
        match raw_lemma_fate(lemma, xref, isv_to_id, &mut raw_covered) {
            RawFate::Rendered { efold } => {
                next_id += 1;
                plan.pages.push((i, next_id));
                plan.xref.insert(&lemma.lang, lemma.word.trim(), next_id);
                fold_owner.insert(efold, next_id);
            }
            RawFate::DedupedFold { target } => {
                plan.deduped += 1;
                plan.xref.insert(&lemma.lang, lemma.word.trim(), target);
                plan.credit
                    .entry(target)
                    .or_default()
                    .push((lemma.lang.clone(), lemma.word.trim().to_string()));
            }
            RawFate::DedupedRawTwin { efold } => {
                plan.deduped += 1;
                if let Some(&owner) = fold_owner.get(&efold) {
                    plan.xref.insert(&lemma.lang, lemma.word.trim(), owner);
                }
            }
            RawFate::DedupedXref | RawFate::Skipped => plan.deduped += 1,
        }
    }
    // Deterministic credit rows: lang then word, duplicates collapsed (a raw
    // corpus can carry the same (lang, word) under several POS sections).
    for rows in plan.credit.values_mut() {
        rows.sort();
        rows.dedup();
    }
    plan
}

/// Compute the raw-lemma coverage report and write it to `out` as both
/// `raw-coverage.md` (human) and `raw-coverage.json` (machine). Reconciles the
/// extraction tally, the site render/dedup split, and the native-join rate.
pub fn run_coverage(out: &Path) -> Result<()> {
    let raw_path = Path::new(crate::DEFAULT_RAW_LEMMA_CACHE);
    let raw_corpus = crate::dump::RawSlavicCorpus::load(raw_path).map_err(|e| {
        anyhow::anyhow!(
            "coverage needs the raw cache {} — run `extract-raw-slavic` first ({e})",
            raw_path.display()
        )
    })?;
    let cov_stats_path = raw_path.with_file_name(crate::dump::RAW_COVERAGE_FILE);
    let cov_stats =
        crate::dump::load_optional(&cov_stats_path, crate::dump::RawCoverageStats::load)?;
    if cov_stats.is_none() {
        println!(
            "(no {} — re-run `extract-raw-slavic` to regenerate the extraction tally)",
            cov_stats_path.display()
        );
    }

    let corpus = crate::dump::LemmaCorpus::load(Path::new(crate::DEFAULT_LEMMA_CACHE))?;
    let official_entries = official::load(Path::new(crate::DEFAULT_OFFICIAL))?;
    let enrich = crate::dump::load_optional(
        Path::new(crate::DEFAULT_ENRICH_CACHE),
        crate::enrich::EnrichIndex::load,
    )?;

    // --- View 1: totals by language and POS over the kept raw lemmas ---
    let total = raw_corpus.lemmas.len();
    let mut by_lang: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_pos: BTreeMap<String, usize> = BTreeMap::new();
    for l in &raw_corpus.lemmas {
        *by_lang.entry(l.lang.clone()).or_default() += 1;
        *by_pos.entry(l.pos.clone()).or_default() += 1;
    }

    // --- View 2: replicate the export dedup to split kept → rendered vs deduped ---
    let (xref, isv_to_id, generated_pages, official_only_pages) =
        build_corpus_render_index(&corpus, &official_entries);
    let mut raw_covered: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut rendered = 0usize;
    let mut deduped = 0usize;
    let mut rendered_by_lang: BTreeMap<String, usize> = BTreeMap::new();
    let mut deduped_by_lang: BTreeMap<String, usize> = BTreeMap::new();
    // Flavorization residue over the rendered set (spec §2 stage 5 / #62):
    // rendered headwords whose letters fall outside the ISV alphabet.
    let mut flavor_residue_words = 0usize;
    let mut flavor_residue: BTreeMap<char, usize> = BTreeMap::new();
    for lemma in &raw_corpus.lemmas {
        match raw_lemma_fate(lemma, &xref, &isv_to_id, &mut raw_covered) {
            RawFate::Rendered { .. } => {
                rendered += 1;
                *rendered_by_lang.entry(lemma.lang.clone()).or_default() += 1;
                let display =
                    crate::flavorize::flavorize_word(&lemma.lang, &lemma.pos, lemma.word.trim());
                let mut had_residue = false;
                for c in crate::flavorize::residue_chars(&display) {
                    *flavor_residue.entry(c).or_default() += 1;
                    had_residue = true;
                }
                if had_residue {
                    flavor_residue_words += 1;
                }
            }
            _ => {
                deduped += 1;
                *deduped_by_lang.entry(lemma.lang.clone()).or_default() += 1;
            }
        }
    }

    // --- View 3: native ru/pl/cs enrichment hit rate, by language ---
    let mut native_hits: BTreeMap<String, usize> = BTreeMap::new();
    if let Some(en) = &enrich {
        for l in &raw_corpus.lemmas {
            if en.get(&l.lang, &l.word).is_some() {
                *native_hits.entry(l.lang.clone()).or_default() += 1;
            }
        }
    }
    let native_total: usize = native_hits.values().sum();

    // --- Provenance: the datasets that fed the raw path, with paths + sizes ---
    let file_line = |label: &str, path: &Path| -> String {
        match std::fs::metadata(path) {
            Ok(m) => format!("- {label}: `{}` ({})", path.display(), fmt_bytes(m.len())),
            Err(_) => format!("- {label}: `{}` (not present)", path.display()),
        }
    };
    let dump_path = cov_stats
        .as_ref()
        .map(|s| s.source.clone())
        .unwrap_or_else(|| crate::DEFAULT_DUMP.to_string());
    let mut provenance = vec![
        format!(
            "- English Wiktextract raw dump (single-token content-word gate): `{}`{}",
            dump_path,
            match std::fs::metadata(&dump_path) {
                Ok(m) => format!(" ({})", fmt_bytes(m.len())),
                Err(_) => " (not present here — the 22 GB source is streamed once)".to_string(),
            }
        ),
        file_line(
            "Derived raw lemma cache",
            Path::new(crate::DEFAULT_RAW_LEMMA_CACHE),
        ),
        file_line("Extraction coverage tally", &cov_stats_path),
        file_line(
            "Native ru/pl/cs Wiktionary enrichment cache",
            Path::new(crate::DEFAULT_ENRICH_CACHE),
        ),
    ];
    provenance.push(format!(
        "- Native editions merged: {} (ru = Russian, pl = Polish, cs = Czech Wiktionary)",
        crate::enrich::ENRICH_LANGS.join(", ")
    ));

    // --- Reconciliation checks ---
    let kept = total; // the cache is exactly the kept set
    let render_reconciles = rendered + deduped == kept;
    let extract_reconciles = cov_stats
        .as_ref()
        .map(|s| s.kept as usize == kept && s.kept + s.dropped_total() == s.slavic_pages_seen)
        .unwrap_or(false);

    std::fs::create_dir_all(out)?;

    // ---- Machine-readable JSON ----
    let report_json = coverage_report_json(
        &raw_corpus,
        cov_stats.as_ref(),
        &by_lang,
        &by_pos,
        rendered,
        deduped,
        &rendered_by_lang,
        generated_pages,
        official_only_pages,
        &native_hits,
        native_total,
        flavor_residue_words,
        &flavor_residue,
    );
    std::fs::write(out.join("raw-coverage.json"), report_json)?;

    // ---- Human-readable Markdown ----
    let mut md = String::new();
    let _ = writeln!(md, "# Raw Slavic-lemma coverage (issue #35)\n");
    let _ = writeln!(
        md,
        "Auditable account of the raw Slavic-Wiktionary datasets: what went in, how \
         many words were included, and how many were excluded and why. Deterministic; \
         regenerate with `coverage` after `extract-raw-slavic` + `export`.\n"
    );

    let _ = writeln!(md, "## Datasets used (provenance)\n");
    for line in &provenance {
        let _ = writeln!(md, "{line}");
    }

    let _ = writeln!(md, "\n## 1. Extraction coverage (English dump)\n");
    if let Some(s) = &cov_stats {
        let _ = writeln!(md, "Streamed {} dump lines.\n", s.lines_scanned);
        let _ = writeln!(md, "| Outcome | Pages | Share of Slavic pages |");
        let _ = writeln!(md, "|---|--:|--:|");
        let seen = s.slavic_pages_seen.max(1);
        let pct = |x: u64| format!("{:.2}%", 100.0 * x as f64 / seen as f64);
        let _ = writeln!(
            md,
            "| Slavic pages seen | {} | 100.00% |",
            s.slavic_pages_seen
        );
        let _ = writeln!(md, "| **KEPT** | {} | {} |", s.kept, pct(s.kept));
        let _ = writeln!(
            md,
            "| dropped — redirect (no senses) | {} | {} |",
            s.dropped_redirect_no_senses,
            pct(s.dropped_redirect_no_senses)
        );
        let _ = writeln!(
            md,
            "| dropped — multiword / empty | {} | {} |",
            s.dropped_multiword,
            pct(s.dropped_multiword)
        );
        let _ = writeln!(
            md,
            "| dropped — non-content POS | {} | {} |",
            s.dropped_non_content_pos,
            pct(s.dropped_non_content_pos)
        );
        let _ = writeln!(
            md,
            "| dropped — no real gloss | {} | {} |",
            s.dropped_no_real_gloss,
            pct(s.dropped_no_real_gloss)
        );
        let _ = writeln!(
            md,
            "\nReconciles: kept ({}) + dropped ({}) = slavic pages seen ({}) → **{}**.",
            s.kept,
            s.dropped_total(),
            s.slavic_pages_seen,
            if extract_reconciles { "OK" } else { "MISMATCH" }
        );
    } else {
        let _ = writeln!(
            md,
            "_Extraction tally unavailable ({} missing); re-run `extract-raw-slavic`._",
            cov_stats_path.display()
        );
    }

    let _ = writeln!(md, "\n## 2. Kept raw lemmas by language\n");
    let _ = writeln!(md, "| Lang | Kept | Rendered raw | Deduped | Native join |");
    let _ = writeln!(md, "|---|--:|--:|--:|--:|");
    for (lang, n_lang) in &by_lang {
        let r = rendered_by_lang.get(lang).copied().unwrap_or(0);
        let d = deduped_by_lang.get(lang).copied().unwrap_or(0);
        let h = native_hits.get(lang).copied().unwrap_or(0);
        let hp = if *n_lang > 0 {
            format!("{:.1}%", 100.0 * h as f64 / *n_lang as f64)
        } else {
            "0.0%".to_string()
        };
        let _ = writeln!(md, "| {lang} | {n_lang} | {r} | {d} | {h} ({hp}) |");
    }
    let _ = writeln!(
        md,
        "| **total** | **{total}** | **{rendered}** | **{deduped}** | **{native_total}** |"
    );

    let _ = writeln!(md, "\n## 3. Kept raw lemmas by part of speech\n");
    let _ = writeln!(md, "| POS | Kept |");
    let _ = writeln!(md, "|---|--:|");
    for (pos, n_pos) in &by_pos {
        let _ = writeln!(md, "| {pos} | {n_pos} |");
    }

    let _ = writeln!(md, "\n## 4. Site rendering (replicated export dedup)\n");
    let _ = writeln!(
        md,
        "Of the {total} kept raw lemmas, the site renders **{rendered}** as raw-only \
         attestation pages and dedups **{deduped}** against an existing official / \
         generated / official-only headword (verbatim `(lang, word)` cognate match, or \
         the display-headword fold already claimed)."
    );
    let _ = writeln!(
        md,
        "\nFor context, the same export renders {generated_pages} generated cognate \
         pages and {official_only_pages} official-only pages.\n"
    );
    let _ = writeln!(
        md,
        "Reconciles: rendered ({rendered}) + deduped ({deduped}) = kept ({kept}) → **{}**.",
        if render_reconciles { "OK" } else { "MISMATCH" }
    );
    let _ = writeln!(
        md,
        "\n> Correctness check: this `rendered-raw` count must equal the number of \
         distinct `R`-status rows across a fresh `export`'s `search/` shards \
         (rows are deliberately listed in several shards — dedup by id)."
    );
    let _ = writeln!(
        md,
        "\nDisplay headwords are flavorized into ISV orthography \
         (`data/FLAVORIZATION_SPEC.md`, issue #62). Validation residue: \
         **{flavor_residue_words}** of {rendered} rendered headwords carry a letter \
         outside the ISV standard alphabet{}.",
        if flavor_residue.is_empty() {
            String::new()
        } else {
            format!(
                " (by letter: {})",
                flavor_residue
                    .iter()
                    .map(|(c, n)| format!("{c}×{n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );

    let _ = writeln!(md, "\n## 5. Native-join (ru/pl/cs enrichment) hit rate\n");
    if enrich.is_some() {
        let _ = writeln!(
            md,
            "{native_total} of {total} raw lemmas ({:.1}%) gain a native ru/pl/cs \
             Wiktionary enrichment match. By language:\n",
            if total > 0 {
                100.0 * native_total as f64 / total as f64
            } else {
                0.0
            }
        );
        let _ = writeln!(md, "| Lang | Kept | Native hits | Rate |");
        let _ = writeln!(md, "|---|--:|--:|--:|");
        for (lang, n_lang) in &by_lang {
            let h = native_hits.get(lang).copied().unwrap_or(0);
            let hp = if *n_lang > 0 {
                format!("{:.1}%", 100.0 * h as f64 / *n_lang as f64)
            } else {
                "0.0%".to_string()
            };
            let _ = writeln!(md, "| {lang} | {n_lang} | {h} | {hp} |");
        }
    } else {
        let _ = writeln!(
            md,
            "_Enrichment cache unavailable ({}); run `extract-enrich`._",
            crate::DEFAULT_ENRICH_CACHE
        );
    }

    std::fs::write(out.join("raw-coverage.md"), &md)?;

    println!(
        "coverage: {total} kept raw lemmas across {} languages → {rendered} rendered raw / {deduped} deduped (reconcile: {}).",
        by_lang.len(),
        if render_reconciles { "OK" } else { "MISMATCH" }
    );
    if let Some(s) = &cov_stats {
        println!(
            "extraction: {} slavic pages seen → {} kept + {} dropped (redirect {} / multiword {} / non-content-pos {} / no-gloss {}); reconcile: {}.",
            s.slavic_pages_seen,
            s.kept,
            s.dropped_total(),
            s.dropped_redirect_no_senses,
            s.dropped_multiword,
            s.dropped_non_content_pos,
            s.dropped_no_real_gloss,
            if extract_reconciles { "OK" } else { "MISMATCH" }
        );
    }
    println!("native-join: {native_total}/{total} raw lemmas matched ru/pl/cs enrichment.");
    println!(
        "wrote {} and {}",
        out.join("raw-coverage.md").display(),
        out.join("raw-coverage.json").display()
    );
    if !render_reconciles || (cov_stats.is_some() && !extract_reconciles) {
        anyhow::bail!("coverage reconciliation FAILED — see report");
    }
    Ok(())
}

/// Human byte size for provenance lines.
fn fmt_bytes(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

/// Machine-readable coverage report (stable key order via serde_json::json!).
#[allow(clippy::too_many_arguments)]
fn coverage_report_json(
    raw_corpus: &crate::dump::RawSlavicCorpus,
    cov_stats: Option<&crate::dump::RawCoverageStats>,
    by_lang: &BTreeMap<String, usize>,
    by_pos: &BTreeMap<String, usize>,
    rendered: usize,
    deduped: usize,
    rendered_by_lang: &BTreeMap<String, usize>,
    generated_pages: usize,
    official_only_pages: usize,
    native_hits: &BTreeMap<String, usize>,
    native_total: usize,
    flavor_residue_words: usize,
    flavor_residue: &BTreeMap<char, usize>,
) -> Vec<u8> {
    let total = raw_corpus.lemmas.len();
    let extraction = cov_stats.map(|s| {
        serde_json::json!({
            "source": s.source,
            "lines_scanned": s.lines_scanned,
            "slavic_pages_seen": s.slavic_pages_seen,
            "kept": s.kept,
            "dropped": {
                "redirect_no_senses": s.dropped_redirect_no_senses,
                "multiword": s.dropped_multiword,
                "non_content_pos": s.dropped_non_content_pos,
                "no_real_gloss": s.dropped_no_real_gloss,
                "total": s.dropped_total(),
            },
            "kept_by_lang": s.kept_by_lang,
            "reconciles": s.kept as usize == total
                && s.kept + s.dropped_total() == s.slavic_pages_seen,
        })
    });
    let report = serde_json::json!({
        "kept_total": total,
        "kept_by_lang": by_lang,
        "kept_by_pos": by_pos,
        "site": {
            "rendered_raw": rendered,
            "deduped": deduped,
            "rendered_by_lang": rendered_by_lang,
            "generated_pages": generated_pages,
            "official_only_pages": official_only_pages,
            "reconciles": rendered + deduped == total,
            "flavorize_residue": {
                "words": flavor_residue_words,
                "by_letter": flavor_residue
                    .iter()
                    .map(|(c, n)| (c.to_string(), *n))
                    .collect::<BTreeMap<String, usize>>(),
            },
        },
        "native_join": {
            "total_hits": native_total,
            "by_lang": native_hits,
            "langs": crate::enrich::ENRICH_LANGS,
        },
        "extraction": extraction,
    });
    let mut v = serde_json::to_vec_pretty(&report).unwrap_or_default();
    v.push(b'\n');
    v
}

/// Inject `generated` derivative FormRecords off attested bases (issue #37) into
/// both sinks. Each base's regular family is derived on the attested-base path;
/// a derivative is kept ONLY if its folded key is absent from `taken` — the set
/// of every key already in the form index (official / official-only lemmas AND
/// their inflected forms, generated reconstructions, and derivatives added
/// earlier in this pass). That guarantees no derivative collides with an
/// attested `form_key`. Survivors ship as `status = "generated"`,
/// `source = "lemma"`, probability from the pattern's leakage-free holdout
/// precision, provenance = `deriv:<pattern>` (in analyses) + the base entry id —
/// and deliberately NO inflected paradigm (an inflected form of a proposed
/// derivative would be confidently wrong). Returns the count added; pure and
/// order-deterministic, so the export stays byte-reproducible.
fn inject_generated_derivatives(
    form_sink: &mut crate::forms::RecordSink,
    lemma_sink: &mut crate::forms::RecordSink,
    taken: &mut std::collections::HashSet<String>,
    bases: &[(String, crate::model::Pos, usize, String)],
    probs: &crate::derive::DerivationProbabilities,
) -> usize {
    let mut added = 0usize;
    for (base, pos, base_id, base_gloss) in bases {
        for d in crate::derive::derive_family(base, *pos) {
            let form = d.form.trim();
            if form.is_empty() || form.contains(' ') || form.contains(['!', '#']) {
                continue;
            }
            let key = crate::forms::form_key(form);
            // Absent-only + dedup in one step: skip when the key is already
            // taken (attested form, prior lemma, or an earlier derivative).
            if key.is_empty() || !taken.insert(key) {
                continue;
            }
            let prob = probs.probability(d.pattern);
            let gloss = format!("{} ← {} ({})", d.label, base, truncate(base_gloss, 50));
            let feats = format!("deriv:{}", d.pattern);
            for sink in [&mut *lemma_sink, &mut *form_sink] {
                sink.add(
                    form,
                    &feats,
                    form,
                    *base_id,
                    d.pos.code(),
                    "lemma",
                    "generated",
                    Some(prob),
                    &gloss,
                );
            }
            added += 1;
        }
    }
    added
}

/// The family key of a cognate set: inherited sets share a family when their
/// Proto-Slavic ancestors share a derivational stem (*starъ / *starostь /
/// *starьcь → "star"); borrowings share a family when they continue the same
/// etymon (la magister → majstor / maestro / magistr).
fn family_key(set: &crate::corpus::CognateSet) -> Option<String> {
    if set.borrowed {
        let e = set.etymon.trim();
        if e.is_empty() {
            None
        } else {
            Some(format!("et:{e}"))
        }
    } else {
        proto_stem(set.proto.trim_start_matches('*')).map(|s| format!("st:{s}"))
    }
}

/// Strip one derivational suffix off a Proto-Slavic reconstruction; the stem
/// must keep ≥4 characters so unrelated short roots don't collide.
fn proto_stem(w: &str) -> Option<String> {
    // Strip the accent/length marks Wiktionary reconstructions carry (pę̑tь),
    // so accent variants of one stem share a family key and the label is clean.
    let w: String = w
        .chars()
        .filter(|c| !('\u{0300}'..='\u{036F}').contains(c))
        .collect();
    let w = w.as_str();
    const SUF: &[&str] = &[
        "ovati", "irati", "nǫti", "ostь", "išče", "ьje", "ica", "ina", "ьcь", "ъka", "ъkъ", "ьnъ",
        "ěti", "ati", "iti", "ti", "y", "a", "o", "ъ", "ь", "ę", "ě", "i",
    ];
    let mut sufs: Vec<&str> = SUF.to_vec();
    sufs.sort_by_key(|s| std::cmp::Reverse(s.chars().count()));
    for s in sufs {
        if let Some(stem) = w.strip_suffix(s) {
            if stem.chars().count() >= 4 {
                return Some(stem.to_string());
            }
        }
    }
    if w.chars().count() >= 4 {
        Some(w.to_string())
    } else {
        None
    }
}

/// Slice-element view for `family_block` (keeps the private `Prepared` struct
/// out of the signature).
trait FamilyEntry {
    fn id(&self) -> usize;
    fn display(&self) -> &str;
    fn set(&self) -> &crate::corpus::CognateSet;
}

/// Render the "word family" section for entry `i`: links to the siblings that
/// share its ancestor stem/etymon. Empty when the entry has no family.
fn family_block<T: FamilyEntry>(
    i: usize,
    prepared: &[T],
    families: &std::collections::BTreeMap<String, Vec<usize>>,
) -> String {
    let Some(key) = family_key(prepared[i].set()) else {
        return String::new();
    };
    let Some(members) = families.get(&key) else {
        return String::new();
    };
    // 2..=15 members: below is no family, above is a suffix artefact.
    if members.len() < 2 || members.len() > 15 {
        return String::new();
    }
    let label = match key.split_once(':') {
        Some(("st", stem)) => format!("praslovjansky korenj <b>*{}-</b>", esc(stem)),
        Some(("et", etymon)) => format!("etimon <b>{}</b>", esc(&etymon_display(etymon))),
        _ => String::new(),
    };
    let mut items = String::new();
    let mut shown = 0;
    for &j in members {
        if j == i {
            continue;
        }
        let s = &prepared[j];
        let _ = write!(
            items,
            "<li><a href='{}.html'><b>{}</b></a> <span class='muted'>{} · {}</span></li>",
            s.id(),
            esc(s.display()),
            esc(s.set().pos.code()),
            esc(&truncate(&s.set().gloss, 48)),
        );
        shown += 1;
        if shown >= 12 {
            break;
        }
    }
    if items.is_empty() {
        return String::new();
    }
    format!(
        "<div class='formation-family'><h3>Etimologična rodina</h3>\
           <p class='muted'>Slova iz toj že etimologičnoj rodiny ({label}):</p>\
           <ul class='compact-list'>{items}</ul>\
         </div>"
    )
}

/// Committee-authored columns from the official dictionary, threaded to matched
/// entry pages for verbatim, attributed *display*. This is presentation-only: it
/// never feeds the generator, consensus vote, evidence, or home-list ranking
/// (those continue to read `OfficialEntry.cells`/`.frequency` directly).
#[derive(Clone, Default)]
struct OfficialDisplay {
    cells: std::collections::HashMap<String, String>,
    de: String,
    nl: String,
    eo: String,
    frequency: Option<f32>,
    intelligibility: String,
    using_example: String,
}

impl OfficialDisplay {
    fn from_entry(e: &OfficialEntry) -> Self {
        OfficialDisplay {
            cells: e.cells.clone(),
            de: e.de.clone(),
            nl: e.nl.clone(),
            eo: e.eo.clone(),
            frequency: e.frequency,
            intelligibility: e.intelligibility.clone(),
            using_example: e.using_example.clone(),
        }
    }
}

/// Strip a single leading `!` committee marker (e.g. `!Baum` → `Baum`).
fn strip_official_marker(s: &str) -> &str {
    let s = s.trim();
    s.strip_prefix('!').unwrap_or(s).trim()
}

/// A compact frequency chip for the headword line (verbatim committee value).
/// Empty when the row carries no frequency. Display-only.
fn official_frequency_chip(freq: Option<f32>) -> String {
    match freq {
        Some(f) => format!(
            "<div class='headmeta'><span class='pill info' title='Čęstota v oficialnom slovniku (interslavic-dictionary.com)'>Čęstota {f:.0}</span></div>"
        ),
        None => String::new(),
    }
}

/// The committee's ISV→language reference translations, rendered as a plain
/// wikitable in a `Prěvody` section — deliberately distinct from the branch-
/// grouped "Srodne slova" cognate *evidence*. Verbatim; the leading `!` marker
/// is stripped consistently. Display-only.
fn official_translations_block(
    cells: &std::collections::HashMap<String, String>,
    de: &str,
    nl: &str,
    eo: &str,
) -> String {
    let mut rows = String::new();
    // 12 Slavic columns in official CSV/branch order.
    for li in crate::lang::official_slavic_cols() {
        if let Some(raw) = cells.get(li.code) {
            let val = strip_official_marker(raw);
            if val.is_empty() {
                continue;
            }
            let _ = write!(
                rows,
                "<tr><td class='lc'>{}</td><td>{}</td></tr>",
                esc(li.name),
                esc(val)
            );
        }
    }
    // Non-Slavic reference languages have no LangInfo entry — label them here.
    for (name, raw) in [("němečsky", de), ("holandsky", nl), ("esperanto", eo)] {
        let val = strip_official_marker(raw);
        if val.is_empty() {
            continue;
        }
        let _ = write!(
            rows,
            "<tr><td class='lc'>{}</td><td>{}</td></tr>",
            esc(name),
            esc(val)
        );
    }
    if rows.is_empty() {
        return String::new();
    }
    format!(
        "<section><h2 id='prevody'>Prěvody</h2>\
         <p class='muted'>Oficialne prěvody komiteta — ne etimologičny dokaz.</p>\
         <table class='wikitable translations-table'><tbody>{rows}</tbody></table>\
         <p class='muted attr-official'>Oficialne danne: interslavic-dictionary.com</p></section>"
    )
}

/// The committee's per-language mutual-intelligibility ratings as a `.chips` row
/// of small language pills. Skipped for the bare `!` placeholder / empty value.
/// Display-only.
fn official_intelligibility_strip(intel: &str) -> String {
    let intel = intel.trim();
    if intel.is_empty() || intel == "!" {
        return String::new();
    }
    let mut chips = String::new();
    for tok in intel.split_whitespace() {
        let sign = match tok.chars().last() {
            Some(c @ ('+' | '~' | '-')) => c,
            _ => continue,
        };
        let code = &tok[..tok.len() - sign.len_utf8()];
        if code.is_empty() {
            continue;
        }
        let cls = match sign {
            '+' => "ok",
            '-' => "bad",
            _ => "",
        };
        let _ = write!(
            chips,
            "<span class='pill {cls}' title='{}'>{} {}</span>",
            esc(crate::lang::lang_name(code)),
            esc(code),
            sign
        );
    }
    if chips.is_empty() {
        return String::new();
    }
    format!(
        "<section><h2 id='razumlivost'>Vzajemna razumlivosť</h2>\
         <div class='chips'>{chips}</div>\
         <p class='muted attr-official'>Oficialne danne: interslavic-dictionary.com</p></section>"
    )
}

/// The committee's verbatim example sentence (rare — ~96 entries). Empty when
/// absent. Display-only.
fn official_example_block(ex: &str) -> String {
    let ex = ex.trim();
    if ex.is_empty() {
        return String::new();
    }
    format!(
        "<section><h2 id='primer'>Priměr</h2>\
         <blockquote class='example-official'>{}</blockquote>\
         <p class='muted attr-official'>Oficialny priměr: interslavic-dictionary.com</p></section>",
        esc(ex)
    )
}

/// The full committee display cluster (translations + intelligibility + example)
/// for the entry-main flow. Each sub-block self-omits when its column is empty.
fn official_display_sections(o: &OfficialDisplay) -> String {
    let mut s = official_translations_block(&o.cells, &o.de, &o.nl, &o.eo);
    s.push_str(&official_intelligibility_strip(&o.intelligibility));
    s.push_str(&official_example_block(&o.using_example));
    s
}

#[allow(clippy::too_many_arguments)]
fn corpus_entry_page(
    id: usize,
    g: &crate::corpus::GeneratedWord,
    status: MatchStatus,
    official: Option<(usize, &str, &str)>,
    // The OFFICIAL part of speech + gender when the headword is a matched
    // official lemma (the form-only match can cross POS; the inflection
    // table must use the official grammar, same as the API records).
    official_pg: Option<(crate::model::Pos, Option<crate::model::Gender>)>,
    // Committee-authored display columns for a matched official lemma (verbatim,
    // attributed, display-only — never feeds generation/ranking).
    official_disp: Option<&OfficialDisplay>,
    family: &str,
    enrich: Option<&crate::enrich::EnrichIndex>,
    xref: Option<&crate::enrich::Xref>,
    raw_xref: &crate::enrich::Xref,
    synonyms: &str,
    derivation: &str,
    wiki_top: &str,
    meta: &SiteEntryMeta,
    // Razumlivost basis codes (issue #86): the caller unions corpus cognate
    // membership with the matched official row's sameInLanguages; equals
    // `meta.languages` for non-matched entries. Display-only.
    razum_codes: &[String],
    // Prebuilt raw-collision credit line ([`raw_credit_line`]), or empty.
    raw_credit: &str,
    wiki_bottom: &str,
    // Prebuilt "(rekonstrukcija)" Predok link to the proto-lemma reflex page
    // (issue #73b), empty when no proto page exists for this ancestor.
    proto_link: &str,
) -> String {
    let top = g.candidates.first().unwrap();
    let pos_code = g.set.pos.code();
    // The official lemma is the authoritative headword when any candidate
    // reproduces it; the generated form stays visible as the reconstruction.
    let headword = official
        .map(|(_, isv, _)| isv.to_string())
        .unwrap_or_else(|| top.form.clone());
    let recon_line = if headword != top.form {
        format!(
            "<p class='def'><b>Rekonstrukcija generatora:</b> <span class='mention'>{}</span></p>",
            esc(&top.form)
        )
    } else {
        String::new()
    };
    // The official meaning is authoritative for a matched headword; the corpus
    // set's own gloss can be a wrong homonym that merely folded to the same form
    // (e.g. the borrowed *pisati* "piss" matching the native *pisati* "write"), so
    // on a match the official gloss headlines the page instead.
    let gloss = match official {
        Some((_, _, en)) if !en.trim().is_empty() => truncate(en, 140),
        _ => truncate(&g.set.gloss, 140),
    };
    let official_note = match official {
        Some((1, isv, _)) => {
            if crate::orthography::exact_match(&top.form, isv) {
                "Oficialna forma; rekonstrukcija ju <b>točno</b> reproduktuje.".to_string()
            } else {
                "Oficialna forma; rekonstrukcija ju reproduktuje (normalizovano — pravopisne znaky sę različajų).".to_string()
            }
        }
        Some((r, _, _)) => {
            format!(
                "Oficialna forma; generator ju davaje kako <a href='#cand-{r}'>kandidat {r}</a>."
            )
        }
        None => "Forma je generovana iz srodnyh slov; ne v oficialnom slovniku.".to_string(),
    };
    let (infl_pos, infl_gender) = match official_pg {
        Some((p, g)) => (p.code(), g),
        None => (pos_code, None),
    };
    let inflection = inflection_table_g(&headword, infl_pos, infl_gender);
    let mut info_rows = String::new();
    let _ = write!(info_rows, "<tr><th>Smysl</th><td>{}</td></tr>", esc(&gloss));
    if !recon_line.is_empty() {
        let _ = write!(
            info_rows,
            "<tr><th>Rekonstrukcija</th><td><span class='mention'>{}</span></td></tr>",
            esc(&top.form)
        );
    }
    let _ = write!(
        info_rows,
        "<tr><th>Izvor formy</th><td><span class='pill {}'>{}</span> {}</td></tr>",
        source_class(top.source),
        esc(top.source.label()),
        status_pill(status)
    );
    let _ = write!(
        info_rows,
        "<tr><th>Opomba</th><td>{}</td></tr>",
        official_note
    );
    // Generated page: razumlivost over the cognate-set membership; on a
    // MATCHED page the caller unioned in the official row's sameInLanguages
    // (issue #86), and the tooltip names the combined basis.
    let razum = {
        let codes: Vec<&str> = razum_codes.iter().map(String::as_str).collect();
        razum_row(
            &codes,
            if official.is_some() {
                RAZUM_TITLE_MATCHED
            } else {
                RAZUM_TITLE
            },
        )
    };
    let entry_card = entry_infobox(meta, &razum, &info_rows, proto_link);
    let freq_chip = official_disp
        .map(|o| official_frequency_chip(o.frequency))
        .unwrap_or_default();
    let official_sections = official_disp
        .map(official_display_sections)
        .unwrap_or_default();
    let cognates = cognate_block(g, enrich);
    let enrich_members: Vec<(String, String)> = g
        .set
        .members
        .iter()
        .map(|m| (m.lang.clone(), m.word.clone()))
        .collect();
    let etymology = unified_etymology_section(g, enrich);
    let native_conn = enrich
        .map(|e| enrich_connections_section(&enrich_members, e, xref, raw_xref, id))
        .unwrap_or_default();
    // On a matched page the top candidate is the official headword, so its
    // reader-facing razumlivost uses the same combined basis as the infobox.
    // Lower alternatives remain corpus-only hypotheses.
    let alternatives = alternatives_block(&g.candidates, official.is_some().then_some(razum_codes));
    let word_formation = word_formation_block(derivation, family);
    let trace = trace_block(top);
    let foot = if official.is_some() {
        "Oficialne slovo; rekonstrukcija i dokazy mašinno generovane (Wiktionary, CC BY-SA)."
    } else {
        "Mašinno generovana rekonstrukcija iz srodnyh slov (Wiktionary, CC BY-SA). Ne oficialny standard."
    };
    let body = format!(
        "<article class='entry entry-with-rail'>\
           {wiki_top}\
           <div class='entry-grid'>\
             <div class='entry-main'>\
               <h1 class='page-title firstHeading'>{headword}</h1>{freq_chip}\
               {etymology}{native_conn}\
               <section><h2 id='pregibanje'>Prěgibanje</h2>{inflection}<p class='muted'><a href='../forms.html?q={forms_q}'>Vse eksportovane formy togo slova (obratny indeks) →</a></p></section>\
               {official_sections}\
               {synonyms}{word_formation}\
               <section><h2 id='cognaty'>Srodne slova — {nlangs} językov</h2>{cognates}{raw_credit}</section>\
               <section><h2 id='sled'>Sled pravil</h2>{trace}</section>\
               {wiki_bottom}\
               <p class='foot'>{foot}</p>\
             </div>\
             <aside class='entry-rail'>{entry_card}<section class='rail-box'><h2 id='formy'>Formy i kandidaty</h2>{alternatives}</section></aside>\
           </div>\
         </article>",
        headword = esc(&headword),
        forms_q = urlencode_q(&headword),
        nlangs = g.n_langs,
    );
    page(&format!("{headword} — medžuslovjansky"), &body, 1)
}

/// A page for an official lemma the generator does not (yet) derive from the
/// cognate evidence: authoritative headword, gloss, inflection — clearly badged.
fn official_only_page(
    isv: &str,
    e: &OfficialEntry,
    enrich: Option<&crate::enrich::EnrichIndex>,
    xref: Option<&crate::enrich::Xref>,
    raw_xref: &crate::enrich::Xref,
    id: usize,
    synonyms: &str,
    derivation: &str,
    wiki_top: &str,
    meta: &SiteEntryMeta,
    // Prebuilt raw-collision credit line ([`raw_credit_line`]), or empty.
    raw_credit: &str,
    wiki_bottom: &str,
) -> String {
    let input = build_input(e);
    let evidence = branch_evidence(&input);
    // Native-Wiktionary enrichment for this official lemma's own cognate cells.
    let enrich_members: Vec<(String, String)> = input
        .forms
        .iter()
        .filter(|f| f.modern && f.primary)
        .map(|f| (f.lang_code.clone(), f.norm.original.clone()))
        .collect();
    let etymology = unified_official_etymology_section(&enrich_members, enrich);
    let native_conn = enrich
        .map(|ix| enrich_connections_section(&enrich_members, ix, xref, raw_xref, id))
        .unwrap_or_default();
    let mut cog = String::new();
    if !evidence.is_empty() {
        cog.push_str("<table class='wikitable compact-table'><tbody>");
        for ev in &evidence {
            let _ = write!(
                cog,
                "<tr><td class='lc'>{}</td><td>{}</td></tr>",
                esc(&ev.lang_name),
                esc(&crate::flavorize::flavorize_word(
                    &ev.lang_code,
                    "",
                    &ev.form
                ))
            );
        }
        cog.push_str("</tbody></table>");
    } else {
        cog.push_str("<p class='muted'>Bez slovjanskogo srodnogo dokaza v slovniku.</p>");
    }
    let inflection = inflection_table_g(isv, e.pos.code(), e.noun_traits.gender);
    let mut info_rows = String::new();
    let _ = write!(
        info_rows,
        "<tr><th>Smysl</th><td>{}</td></tr><tr><th>Opomba</th><td>Generator ješče ne izvodi tu formu iz srodnogo dokaza.</td></tr>",
        esc(&e.english)
    );
    // Official-only page: the honest razumlivost basis is the committee's
    // OWN sameInLanguages attestation — the translation cells are filled for
    // every language and would claim a constant ~99%. Empty column → no row.
    let same_in = e.same_in_langs();
    let razum = if same_in.is_empty() {
        String::new()
    } else {
        razum_row(&same_in, RAZUM_TITLE_OFFICIAL)
    };
    let entry_card = entry_infobox(meta, &razum, &info_rows, "");
    let word_formation = word_formation_block(derivation, "");
    let freq_chip = official_frequency_chip(e.frequency);
    let official_sections = official_display_sections(&OfficialDisplay::from_entry(e));
    let body = format!(
        "<article class='entry entry-with-rail'>\
           {wiki_top}\
           <div class='entry-grid'>\
             <div class='entry-main'>\
               <h1 class='page-title firstHeading'>{isv}</h1>{freq_chip}\
               {etymology}{native_conn}\
               <section><h2 id='pregibanje'>Prěgibanje</h2>{inflection}<p class='muted'><a href='../forms.html?q={forms_q}'>Vse eksportovane formy togo slova (obratny indeks) →</a></p></section>\
               {official_sections}\
               {synonyms}{word_formation}\
               <section><h2 id='cognaty'>Srodne slova</h2>{cog}{raw_credit}</section>\
               {wiki_bottom}\
               <p class='foot'>Oficialne slovo: interslavic-dictionary.com. Prěgibanje mašinno generovano.</p>\
             </div>\
             <aside class='entry-rail'>{entry_card}</aside>\
           </div>\
         </article>",
        isv = esc(isv),
        forms_q = urlencode_q(isv),
    );
    page(&format!("{isv} — medžuslovjansky"), &body, 1)
}

/// A SITE-ONLY, low-evidence entry for one raw Slavic Wiktionary lemma (issue #34).
/// Cloned from [`official_only_page`] but with every official-only / generation
/// section dropped. It MERGES two independent dictionary sources for the word,
/// each clearly labelled: the English-Wiktionary dump data (attested glosses +
/// raw etymology text) and — when the enrichment cache has it (issue #33) — the
/// NATIVE RU/PL/CS Wiktionary entry (its own senses, usage quotations, semantic
/// links, and etymology). It stays clearly badged as a raw, low-evidence
/// attestation that is NOT an Interslavic standard.
///
/// It is deliberately NOT wired into the verification/forms API, the cognate
/// graph, categories, homograph indexes, talk/backlink pages, or the home list:
/// these pages exist purely so every dictionary word is discoverable and
/// searchable. All dump text is escaped through [`esc`]. The `display`
/// headword arrives flavorized into ISV orthography (issue #62); the attested
/// original stays in the banner, infobox, and source URL. Running text is
/// transliterated via [`source_display`], never flavorized.
#[allow(clippy::too_many_arguments)]
fn raw_lemma_page(
    display: &str,
    lemma: &crate::dump::RawSlavicLemma,
    id: usize,
    meta: &SiteEntryMeta,
    enrich: Option<&crate::enrich::EnrichIndex>,
    gx: &crate::glossxref::GlossXref,
    xref: Option<&crate::enrich::Xref>,
    raw_xref: &crate::enrich::Xref,
) -> String {
    // Attested English-Wiktionary glosses, verbatim (escaped). Low-evidence.
    let mut gloss_items = String::new();
    for g in &lemma.glosses {
        let g = g.trim();
        if g.is_empty() {
            continue;
        }
        let _ = write!(gloss_items, "<li>{}</li>", esc(g));
    }
    let glosses = if gloss_items.is_empty() {
        "<p class='muted'>Bez zapisanogo smysla.</p>".to_string()
    } else {
        format!("<ul class='compact-list'>{gloss_items}</ul>")
    };
    // Native RU/PL/CS enrichment for THIS raw word (accent-stripped key lookup),
    // when the enrich cache carries it (issue #33). Its senses, usage quotations,
    // and semantic links render via the same helper the generated pages use.
    let native = enrich.and_then(|ix| ix.get(&lemma.lang, &lemma.word));
    let native_members = [(lemma.lang.clone(), lemma.word.clone())];
    // Semantic-link chips on raw pages now resolve internally too (issue #64):
    // pass both cross-references (this used to pass None and always link out).
    let native_conn = enrich
        .map(|ix| enrich_connections_section(&native_members, ix, xref, raw_xref, id))
        .unwrap_or_default();
    // Merged etymology: the native (non-stub) etymology and the English dump text,
    // side by side and source-labelled. A native `Происходит от ??` stub is
    // dropped so the English etymology_text fills the gap instead.
    let etymology = raw_etymology_section(lemma, native);
    // The page has native content to show when the native entry contributed
    // senses, links, or a non-stub etymology.
    let native_shown = !native_conn.is_empty()
        || native.is_some_and(|e| e.etymology.iter().any(|p| !etym_is_stub(p)));
    let lang_name = crate::lang::lang_name(&lemma.lang);
    let src_url = crate::enrich::source_url(&lemma.lang, &lemma.word);
    let banner = if native_shown {
        format!(
            "<div class='banner warn'><b>Surova atestacija iz Wiktionary — nizko dokazano.</b> \
             Ta zapis kombinuje dane iz anglijskoj Wiktionary i iz narodnoj {lang} Wiktionary \
             (<span class='mention'>{word}</span>): zapisane smysly, značenja, priměry i etimologiju. \
             Ne ma slovjanskogo konsensusa ni oficialnoj validacije — ne oficialny standard.</div>",
            lang = esc(lang_name),
            word = esc(&lemma.word),
        )
    } else {
        format!(
            "<div class='banner warn'><b>Surova atestacija iz Wiktionary — nizko dokazano.</b> \
             Ta zapis pokazuje samo dane iz anglijskoj Wiktionary ({lang} <span class='mention'>{word}</span>): \
             zapisane smysly i surovy etimologičny tekst. Ne ma slovjanskogo konsensusa ni oficialnoj \
             validacije — ne oficialny standard.</div>",
            lang = esc(lang_name),
            word = esc(&lemma.word),
        )
    };
    let mut info_rows = String::new();
    let _ = write!(
        info_rows,
        "<tr><th>Izvor</th><td><a href='{src}'>{lang} · Wiktionary</a></td></tr>\
         <tr><th>Atestovana forma</th><td><span class='mention'>{word}</span></td></tr>\
         <tr><th>Dokaz</th><td>surova (bez konsensusa)</td></tr>",
        src = esc(&src_url),
        lang = esc(lang_name),
        word = esc(&lemma.word),
    );
    // Raw page: razumlivost of the single attesting language.
    let razum = {
        let codes: Vec<&str> = meta.languages.iter().map(String::as_str).collect();
        razum_row(&codes, RAZUM_TITLE)
    };
    let entry_card = entry_infobox(meta, &razum, &info_rows, "");
    // Reverse gloss links: the same meaning(s) in other Slavic languages.
    let cross = cross_lingual_meanings_section(gx, &lemma.lang, &lemma.glosses, xref, raw_xref, id);
    let body = format!(
        "<article class='entry entry-with-rail'>\
           <div class='entry-grid'>\
             <div class='entry-main'>\
               <h1 class='page-title firstHeading'>{disp}</h1>\
               {banner}\
               <section><h2 id='smysly'>Anglijske značenja <span class='muted'>(anglijska Wiktionary)</span></h2>{glosses}</section>\
               {native_conn}\
               {etymology}\
               {cross}\
               <p class='foot'>Surova atestacija iz Wiktionary (CC BY-SA): <a href='{src}'>{lang} · {word}</a>. \
                Nizko dokazano; samo za sajt, ne oficialny standard. Prěgibanje ne generovano.</p>\
             </div>\
             <aside class='entry-rail'>{entry_card}</aside>\
           </div>\
         </article>",
        disp = esc(display),
        src = esc(&src_url),
        lang = esc(lang_name),
        word = esc(&lemma.word),
    );
    page(&format!("{display} — surova atestacija"), &body, 1)
}

/// "Same meaning in other Slavic languages" (reverse gloss links): for each
/// English gloss token of the entry, the words carrying that gloss in OTHER
/// Slavic languages, grouped by token then language. Bridged by a shared English
/// gloss — an approximate meaning link, not an etymological cognate. Chips link
/// into the site when the word is a dictionary headword (via `xref`), else out to
/// the native Wiktionary. Chip words are flavorized into ISV orthography
/// (`flavorize_word`, POS unknown here so ending adaptation is off).
/// One word chip: link to the Slovowiki page for `(lang, word)` when one
/// exists — the cognate `xref` first (generated cognate membership), then the
/// raw-attestation cross-reference (raw pages and their fold-dedup targets;
/// issue #64) — else out to the native Wiktionary. Self-links fall through to
/// the external target so a page never links to itself.
fn word_chip(
    lang: &str,
    word: &str,
    visible: &str,
    xref: Option<&crate::enrich::Xref>,
    raw_xref: &crate::enrich::Xref,
    self_id: usize,
) -> String {
    let generated = xref
        .map(|x| x.lookup(lang, word))
        .unwrap_or(crate::enrich::XrefMatch::Missing);
    // An ambiguous generated key must not fall through to a raw page: that
    // would merely replace one insertion-order-dependent sense choice with
    // another. Use the external source as the honest disambiguation surface.
    let target = match generated {
        crate::enrich::XrefMatch::Unique(t) if t != self_id => Some(t),
        crate::enrich::XrefMatch::Ambiguous => None,
        crate::enrich::XrefMatch::Missing | crate::enrich::XrefMatch::Unique(_) => {
            raw_xref.get(lang, word).filter(|&t| t != self_id)
        }
    };
    match target {
        Some(t) => format!(
            "<a class='chip xref' title='v slovniku' href='{t}.html'>{}</a>",
            esc(visible)
        ),
        None => format!(
            "<a class='chip' href='{}'>{}</a>",
            esc(&crate::enrich::source_url(lang, word)),
            esc(visible)
        ),
    }
}

fn cross_lingual_meanings_section(
    gx: &crate::glossxref::GlossXref,
    lang: &str,
    glosses: &[String],
    xref: Option<&crate::enrich::Xref>,
    raw_xref: &crate::enrich::Xref,
    self_id: usize,
) -> String {
    let groups = gx.matches(lang, glosses);
    if groups.is_empty() {
        return String::new();
    }
    let mut blocks = String::new();
    for (tok, others) in &groups {
        let mut by_lang: std::collections::BTreeMap<&str, Vec<&str>> =
            std::collections::BTreeMap::new();
        for (l, w) in others {
            by_lang.entry(l.as_str()).or_default().push(w.as_str());
        }
        let mut rows = String::new();
        for (l, ws) in by_lang.iter().take(crate::glossxref::MAX_LANGS) {
            let chips: String = ws
                .iter()
                .take(crate::glossxref::MAX_PER_LANG)
                .map(|w| {
                    let visible = crate::flavorize::flavorize_word(l, "", w);
                    word_chip(l, w, &visible, xref, raw_xref, self_id)
                })
                .collect();
            let _ = write!(
                rows,
                "<tr><td class='lc'>{}</td><td><div class='chips'>{}</div></td></tr>",
                esc(crate::lang::lang_name(l)),
                chips
            );
        }
        let _ = write!(
            blocks,
            "<div class='conn'><h5><span class='mention'>{}</span></h5><table class='wikitable compact-table'><tbody>{}</tbody></table></div>",
            esc(tok),
            rows
        );
    }
    format!(
        "<section><h2 id='drugojezyk'>To slovo v drugih slovjanskih językah <span class='muted'>(po značenju)</span></h2>\
         <p class='muted'>Slova v drugih slovjanskih językah s tym že anglijskym značenjem (most čerez anglijsku Wiktionary) — pomožny prěgled, ne etimologičny ni oficialny dokaz; slova sųt pokazane v medžuslovjanskoj latinici (flavorizacija), originalny zapis jest na strancě slova.</p>{blocks}</section>"
    )
}

/// True when a native etymology paragraph is a bare placeholder stub — empty or
/// wiktextract's `Происходит от ??` / `?? ` unknown-origin marker — which carries
/// no real etymology and should yield to the English `etymology_text` instead.
fn etym_is_stub(s: &str) -> bool {
    let t = s.trim();
    t.is_empty() || t.contains("??")
}

/// The merged etymology `<section>` for a raw lemma page (issue #33): the native
/// RU/PL/CS etymology (stubs dropped, RU transliterated) and the English-dump
/// `etymology_text` (verbatim), each rendered as a source-labelled card. Returns
/// an empty string when neither source has usable etymology.
fn raw_etymology_section(
    lemma: &crate::dump::RawSlavicLemma,
    native: Option<&crate::enrich::EnrichEntry>,
) -> String {
    let mut cards = String::new();
    // Native etymology (non-stub paragraphs only), from the native edition.
    if let Some(e) = native {
        let paras: String = e
            .etymology
            .iter()
            .filter(|p| !etym_is_stub(p))
            .map(|p| format!("<p>{}</p>", esc(&source_display(&lemma.lang, p))))
            .collect();
        if !paras.is_empty() {
            let _ = write!(
                cards,
                "<div class='etym-src'><div class='src-head'><span class='lc'>{} · Wiktionary</span> <a class='ext' href='{}'>{}↗</a></div>{}</div>",
                esc(crate::lang::lang_name(&lemma.lang)),
                esc(&crate::enrich::source_url(&lemma.lang, &lemma.word)),
                esc(&source_display(&lemma.lang, &lemma.word)),
                paras
            );
        }
    }
    // English-Wiktionary etymology_text, verbatim (escaped) — always shown when
    // present, and the fallback that fills a dropped native `??` stub.
    let t = lemma.etymology_text.trim();
    if !t.is_empty() {
        let _ = write!(
            cards,
            "<div class='etym-src'><div class='src-head'><span class='lc'>anglijska Wiktionary · {}</span> <a class='ext' href='https://en.wiktionary.org/wiki/{}#{}'>{}↗</a></div><p class='etym-raw'>{}</p></div>",
            esc(crate::lang::lang_name(&lemma.lang)),
            esc(&lemma.word.replace(' ', "_")),
            esc(&lemma.lang),
            esc(&source_display(&lemma.lang, &lemma.word)),
            esc(t).replace('\n', "<br>")
        );
    }
    if cards.trim().is_empty() {
        return String::new();
    }
    format!(
        "<section><h2 id='etimologija'>Etimologija</h2><div class='etym-sources'>{cards}</div>\
         <p class='muted'>Etimologije iz Wiktionary (CC BY-SA); anglijsky tekst ostaje anglijsky, rusky tekst jest transliterovany.</p></section>"
    )
}

/// The full search-results page (search.html). Reads `?q=` and lists every match;
/// the header search box (present on every page) submits here on Enter.
fn search_page() -> String {
    let body = "<article class='entry search-page'>\
      <h1 class='firstHeading'>Iskanje</h1>\
      <p class='muted'>Napiši v polje gore i pritisni <b>Enter</b>, ili filtruj statičny indeks. Najdeno: <b id='rescount'>0</b> rezultatov.</p>\
      <form class='filter-grid' onsubmit='return false'>\
        <label>Čęst rěči <select id='f-pos'><option value=''>vse</option><option value='noun'>imennik</option><option value='verb'>glagol</option><option value='adj'>pridavnik</option><option value='adv'>narěčje</option><option value='proper_noun'>vlastno imę</option><option value='num'>čislovnik</option></select></label>\
        <label>Stav <select id='f-status'><option value=''>vse</option><option value='O'>oficialne</option><option value='N'>samo generovane</option><option value='R'>surove atestacije</option></select></label>\
        <label>Uvěrjenost <select id='f-conf'><option value=''>vse</option><option value='V'>vysoka</option><option value='S'>srědnja</option><option value='N'>nizka</option></select></label>\
        <label>Tip <select id='f-borrowed'><option value=''>vse</option><option value='0'>naslědovane</option><option value='1'>zaimky</option></select></label>\
        <label>Min. językov <input id='f-langs' type='number' min='0' value='0'></label>\
      </form>\
      <div id='page-results' class='results full'></div>\
    </article>";
    page("Iskanje — medžuslovjansky", body, 0)
}

/// Display for RUNNING TEXT from a source language (quoted etymology
/// paragraphs, gloss truncations): script-faithful transliteration only —
/// Russian is transliterated, other editions pass through (extending them is
/// issue #38). Words displayed AS WORDS (raw headwords, chips, cognate
/// mentions) use [`crate::flavorize::flavorize_word`] instead (issue #62);
/// flavorizing a quoted sentence would misquote the source.
fn source_display(lang: &str, text: &str) -> String {
    crate::flavorize::translit_text(lang, text)
}

/// Human-readable borrowing source: `la computare` → `latinsky computare`.
fn etymon_display(etymon: &str) -> String {
    let (src, word) = etymon.split_once(' ').unwrap_or(("", etymon));
    let name = match src {
        "la" | "ML." | "LL." | "la-med" | "la-lat" => "latinsky",
        "grc" | "el" => "grečsky",
        "fr" | "frm" | "fro" => "francuzsky",
        "de" | "gmh" => "němečsky",
        "en" => "anglijsky",
        "it" => "italijsky",
        "nl" => "holandsky",
        "es" | "pt" => "iberijsky",
        "tr" | "ota" => "turecky",
        "ar" => "arabsky",
        "he" => "hebrejsky",
        _ => "",
    };
    if name.is_empty() {
        etymon.to_string()
    } else {
        format!("{name} „{word}“")
    }
}

/// The headword's dictionary synonyms (from the thesaurus), each cross-linked to
/// its own entry page when it is a site headword, else to a search for it.
fn synonyms_block(
    isv: &str,
    thes: &crate::thesaurus::Thesaurus,
    isv_to_id: &HeadwordIndex,
) -> String {
    let syns = thes.get(isv);
    if syns.is_empty() {
        return String::new();
    }
    let mut chips = String::new();
    for s in syns.iter().take(24) {
        let (cls, href) = match isv_to_id.resolve(s) {
            Some(id) => ("chip xref", format!("{id}.html")),
            None => ("chip redlink", format!("../search.html?q={}", esc(s))),
        };
        let _ = write!(chips, "<a class='{cls}' href='{href}'>{}</a>", esc(s));
    }
    format!("<section><h2 id='synonimy'>Synonimy</h2><div class='chips'>{chips}</div></section>")
}

/// The headword's regular derivational family (Track A / issue #1): each
/// seam-aware derivative as a chip — cross-linked when it is a site headword,
/// marked as a machine proposal otherwise. Derivation is deterministic
/// (`derive::derive_family`), so the block is reproducible byte-for-byte.
fn derivation_block(
    headword: &str,
    pos: crate::model::Pos,
    isv_to_id: &HeadwordIndex,
    attested_base: bool,
    base_id: usize,
    deriv_rows: &mut BTreeMap<&'static str, DerivAgg>,
) -> String {
    let fam = crate::derive::derive_family(headword, pos);
    if fam.is_empty() {
        return String::new();
    }
    let mut rows = String::new();
    let mut linked = 0usize;
    let mut proposed = 0usize;
    for d in &fam {
        let derived_entry_id = isv_to_id.resolve(&d.form);
        // Report the row EXACTLY as rendered (same derive_family inputs, same
        // isv_to_id resolution) to the derivational-suffix browse collector
        // (issue #73d) — the deriv/ pages can never drift from this block.
        deriv_rows
            .entry(d.pattern)
            .or_insert_with(|| DerivAgg {
                label: d.label,
                rows: Vec::new(),
            })
            .rows
            .push(DerivRow {
                base_id,
                base: headword.to_string(),
                form: d.form.clone(),
                derived_entry_id,
                official: attested_base,
            });
        let (form, status) = match derived_entry_id {
            Some(id) => {
                linked += 1;
                (
                    format!(
                        "<a href='{id}.html'><span class='mention'>{}</span></a>",
                        esc(&d.form)
                    ),
                    "strana na sajtě".to_string(),
                )
            }
            None => {
                proposed += 1;
                (
                    format!("<span class='mention'>{}</span>", esc(&d.form)),
                    "mašinovy kandidat".to_string(),
                )
            }
        };
        let _ = write!(
            rows,
            "<tr><td>{form}</td><td>{}</td><td>{}</td><td class='muted'>{}</td></tr>",
            esc(&pos_code_label(d.pos.code())),
            esc(d.label),
            esc(&status)
        );
    }
    let base_note = if attested_base {
        String::new()
    } else {
        " <b>Baza je mašinova rekonstrukcija</b> (ne oficialna lemma), zato odvodženja sųt hypotetične.".to_string()
    };
    format!(
        "<div class='formation-derived'><h3>Pravilne odvodženja</h3>\
         <table class='wikitable compact-table formation-table'><thead><tr><th>Forma</th><th>Čęst rěči</th><th>Obrazec</th><th>Stav</th></tr></thead><tbody>{rows}</tbody></table>\
         <p class='muted'>Pokazano {} tvorjenyh form: {} imajųt stranicu, {} sų samo pravilno tvorjeni kandidaty. Pravila vključajųt palatalizaciju prěd sufiksami, jotaciju prěd -ńje i O⇒E po mękkyh.{base_note}</p></div>",
        fam.len(),
        linked,
        proposed
    )
}

fn word_formation_block(derivation: &str, family: &str) -> String {
    if derivation.trim().is_empty() && family.trim().is_empty() {
        String::new()
    } else {
        format!("<section><h2 id='slovotvorstvo'>Slovotvorstvo</h2>{derivation}{family}</section>")
    }
}

/// The cognate set: every attesting Slavic lemma, grouped by branch.
fn cognate_block(
    g: &crate::corpus::GeneratedWord,
    enrich: Option<&crate::enrich::EnrichIndex>,
) -> String {
    let mut s = String::from("<div class='branch-grid'>");
    for branch in Branch::ALL {
        let items: Vec<&crate::dump::LemmaEntry> = g
            .set
            .members
            .iter()
            .filter(|m| {
                crate::lang::lang_info(&m.lang).is_some_and(|info| info.modern)
                    && crate::corpus::branch_of(&m.lang) == Some(branch)
            })
            .collect();
        if items.is_empty() {
            continue;
        }
        let _ = write!(
            s,
            "<div class='branch-box'><h4>{}</h4><table class='wikitable compact-table'><tbody>",
            esc(branch.label())
        );
        for m in items {
            // Native-Wiktionary link + native sense for the enriched editions
            // (ru/pl/cs); otherwise fall back to the English gloss.
            let hit = enrich.and_then(|e| e.get(&m.lang, &m.word));
            let native = match hit {
                Some(_) => format!(
                    " <a class='ext' title='{0}.wiktionary' href='{1}'>{0}↗</a>",
                    esc(&m.lang),
                    esc(&crate::enrich::source_url(&m.lang, &m.word))
                ),
                None => String::new(),
            };
            let gloss = hit
                .and_then(|e| e.senses.first())
                .map(|x| truncate(&source_display(&m.lang, x), 44))
                .unwrap_or_else(|| truncate(&source_display(&m.lang, &m.gloss), 32));
            let visible_word = crate::flavorize::flavorize_word(&m.lang, &m.pos, &m.word);
            let norm = crate::normalize::to_phonemic_latin(&m.lang, &m.word);
            let norm_note = if norm != visible_word {
                format!("<br><span class='muted'>→ {}</span>", esc(&norm))
            } else {
                String::new()
            };
            let _ = write!(
                s,
                "<tr><td class='lc'>{}</td><td><a href='https://en.wiktionary.org/wiki/{}#{}'>{}</a>{}{}</td><td class='muted'>{}</td></tr>",
                esc(&crate::lang::lang_name(&m.lang)),
                esc(&m.word.replace(' ', "_")),
                esc(&m.lang),
                esc(&visible_word),
                native,
                norm_note,
                esc(&gloss),
            );
        }
        s.push_str("</tbody></table></div>");
    }
    s.push_str("</div>");
    let historical: Vec<&crate::dump::LemmaEntry> = g
        .set
        .members
        .iter()
        .filter(|m| crate::lang::lang_info(&m.lang).is_some_and(|info| !info.modern))
        .collect();
    if !historical.is_empty() {
        s.push_str("<div class='historical-hints'><h4>Historijske podskazky</h4><p class='muted'>Te formy pomagajųt etimologiji, ale ne sųt moderne atestacije i ne povečšajųt pokrytje, razumlivost ili uvěrjenost.</p><table class='wikitable compact-table'><tbody>");
        for m in historical {
            let visible = crate::flavorize::flavorize_word(&m.lang, &m.pos, &m.word);
            let _ = write!(
                s,
                "<tr><td class='lc'>{}</td><td><a href='https://en.wiktionary.org/wiki/{}#{}'>{}</a></td><td class='muted'>{}</td></tr>",
                esc(crate::lang::lang_name(&m.lang)),
                esc(&m.word.replace(' ', "_")),
                esc(&m.lang),
                esc(&visible),
                esc(&truncate(&source_display(&m.lang, &m.gloss), 32)),
            );
        }
        s.push_str("</tbody></table></div>");
    }
    s
}

fn unified_etymology_section(
    g: &crate::corpus::GeneratedWord,
    enrich: Option<&crate::enrich::EnrichIndex>,
) -> String {
    let summary = if g.set.borrowed {
        format!(
            "<p>Internacionalizm (pozajęto slovo). Etimon: <span class='mention'>{}</span>. Slovjanske refleksy i izvorne etimologije sųt niže.</p>",
            esc(&etymon_display(&g.set.etymon))
        )
    } else {
        format!(
            "<p>Iz praslovjanskogo <a class='mention' href='https://en.wiktionary.org/wiki/Reconstruction:Proto-Slavic/{p}'>*{p}</a>. Niže sųt izvorne etimologije iz anglijskogo i narodnyh Wiktionary.</p>",
            p = esc(g.set.proto.trim_start_matches('*')),
        )
    };
    let english = english_etymology_cards(&g.set.members);
    let native_members: Vec<(String, String)> = g
        .set
        .members
        .iter()
        .map(|m| (m.lang.clone(), m.word.clone()))
        .collect();
    let native = enrich
        .map(|ix| native_etymology_cards(&native_members, ix))
        .unwrap_or_default();
    let cards = format!("{english}{native}");
    if cards.trim().is_empty() {
        format!("<section><h2 id='etimologija'>Etimologija</h2>{summary}</section>")
    } else {
        format!(
            "<section><h2 id='etimologija'>Etimologija</h2>{summary}<div class='etym-sources'>{cards}</div><p class='muted'>Izvorne etimologije sųt vzęte iz Wiktionary (CC BY-SA); anglijsky tekst ostaje anglijsky, rusky tekst jest transliterovany.</p></section>"
        )
    }
}

fn unified_official_etymology_section(
    members: &[(String, String)],
    enrich: Option<&crate::enrich::EnrichIndex>,
) -> String {
    let native = enrich
        .map(|ix| native_etymology_cards(members, ix))
        .unwrap_or_default();
    if native.trim().is_empty() {
        String::new()
    } else {
        format!(
            "<section><h2 id='etimologija'>Etimologija</h2><div class='etym-sources'>{native}</div><p class='muted'>Izvorne etimologije iz narodnyh Wiktionary (CC BY-SA); rusky tekst jest transliterovany.</p></section>"
        )
    }
}

fn english_etymology_cards(members: &[crate::dump::LemmaEntry]) -> String {
    let mut rows = String::new();
    let mut seen = BTreeSet::new();
    for m in members.iter().filter(|m| !m.etymology.is_empty()) {
        let key = m.etymology.join("\n");
        if !seen.insert(key) {
            continue;
        }
        let paras: String = m
            .etymology
            .iter()
            .map(|p| format!("<p>{}</p>", esc(p)))
            .collect();
        let visible_word = crate::flavorize::flavorize_word(&m.lang, &m.pos, &m.word);
        let _ = write!(
            rows,
            "<div class='etym-src'><div class='src-head'><span class='lc'>anglijska Wiktionary · {}</span> <a class='ext' href='https://en.wiktionary.org/wiki/{}#{}'>{}↗</a></div>{}</div>",
            esc(&crate::lang::lang_name(&m.lang)),
            esc(&m.word.replace(' ', "_")),
            esc(&m.lang),
            esc(&visible_word),
            paras
        );
        if seen.len() >= 4 {
            break;
        }
    }
    rows
}

/// Multi-source native etymology (RU / PL / CS Wiktionary) — one etymology per
/// edition, side by side, so each entry carries independent source histories.
fn native_etymology_cards(
    members: &[(String, String)],
    enrich: &crate::enrich::EnrichIndex,
) -> String {
    let mut rows = String::new();
    for &lang in crate::enrich::ENRICH_LANGS {
        let Some((word, e)) = members
            .iter()
            .filter(|(l, _)| l == lang)
            .find_map(|(l, w)| enrich.get(l, w).map(|e| (w, e)))
            .filter(|(_, e)| !e.etymology.is_empty())
        else {
            continue;
        };
        let paras: String = e
            .etymology
            .iter()
            .map(|p| format!("<p>{}</p>", esc(&source_display(lang, p))))
            .collect();
        let visible_word = crate::flavorize::flavorize_word(lang, "", word);
        let _ = write!(
            rows,
            "<div class='etym-src'><div class='src-head'><span class='lc'>{}</span> <a class='ext' href='{}'>{}↗</a></div>{}</div>",
            esc(&crate::lang::lang_name(lang)),
            esc(&crate::enrich::source_url(lang, word)),
            esc(&visible_word),
            paras
        );
    }
    rows
}

/// Source-language meanings, usage quotations, and semantic links (related /
/// synonyms / antonyms) drawn from the native RU / PL / CS Wiktionary entries for
/// the cognates. Every enriched member is shown (grouped by edition), its full
/// numbered sense list rendered under a heading naming that source lemma, with any
/// recorded usage quotations nested beneath the sense they illustrate. This is
/// source-language evidence tied to a specific cognate — never an authoritative
/// Interslavic definition. Chips link back to the source dictionary (or internally
/// when the term is itself a headword).
fn enrich_connections_section(
    members: &[(String, String)],
    enrich: &crate::enrich::EnrichIndex,
    xref: Option<&crate::enrich::Xref>,
    raw_xref: &crate::enrich::Xref,
    self_id: usize,
) -> String {
    let mut blocks = String::new();
    for &lang in crate::enrich::ENRICH_LANGS {
        // Every enriched member of this edition, in member order, deduped by word.
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for (l, w) in members.iter().filter(|(l, _)| l == lang) {
            let Some(e) = enrich.get(l, w) else { continue };
            if !seen.insert(w.to_lowercase()) {
                continue;
            }
            let inner = enrich_member_block(lang, e, xref, raw_xref, self_id);
            if inner.is_empty() {
                continue;
            }
            let visible_word = crate::flavorize::flavorize_word(lang, "", w);
            let _ = write!(
                blocks,
                "<div class='src-block'><div class='src-head'><span class='lc'>{}</span> <a class='ext' href='{}'>{}↗</a></div>{}</div>",
                esc(&crate::lang::lang_name(lang)),
                esc(&crate::enrich::source_url(lang, w)),
                esc(&visible_word),
                inner
            );
        }
    }
    if blocks.is_empty() {
        return String::new();
    }
    format!(
        "<section><h2 id='vezi'>Značenja srodnyh slov i semantične vęzi (RU / PL / CS)</h2>\
         <p class='muted'>Značenja i priměry upotrěby zapisane v narodnyh Wiktionary (RU / PL / CS) za navedene srodne slova — dokaz v izvornom języku, ne oficialne medžuslovjanske definicije; rusky tekst jest transliterovany.</p>{blocks}</section>"
    )
}

/// One enriched member's block: its full numbered sense list (each sense carrying
/// any recorded usage quotations) plus related / synonym / antonym chips.
fn enrich_member_block(
    lang: &str,
    e: &crate::enrich::EnrichEntry,
    xref: Option<&crate::enrich::Xref>,
    raw_xref: &crate::enrich::Xref,
    self_id: usize,
) -> String {
    let mut inner = String::new();
    // Every documented sense is shown (a single sense is legitimate evidence too);
    // usage quotations render beneath the sense they illustrate.
    if !e.senses.is_empty() {
        let items: String = e
            .senses
            .iter()
            .map(|sense| {
                let quotes: String = e
                    .examples
                    .iter()
                    .filter(|q| &q.sense == sense)
                    .map(|q| {
                        let cite = if q.source.is_empty() {
                            String::new()
                        } else {
                            format!(
                                " <span class='muted cite'>— {}</span>",
                                esc(&source_display(lang, &q.source))
                            )
                        };
                        format!(
                            "<li class='quote'>„{}“{cite}</li>",
                            esc(&source_display(lang, &q.text))
                        )
                    })
                    .collect();
                let quote_block = if quotes.is_empty() {
                    String::new()
                } else {
                    format!("<ul class='quotes'>{quotes}</ul>")
                };
                format!(
                    "<li>{}{quote_block}</li>",
                    esc(&source_display(lang, sense))
                )
            })
            .collect();
        let _ = write!(
            inner,
            "<div class='conn'><h5>Značenja</h5><ol>{items}</ol></div>"
        );
    }
    let chips = |title: &str, words: &[String]| -> String {
        if words.is_empty() {
            return String::new();
        }
        let cs: String = words
            .iter()
            .map(|w| {
                // Link internally when Slovowiki has ANY page for the term —
                // generated cognate membership or a raw attestation (#64);
                // otherwise out to native Wiktionary.
                let visible = crate::flavorize::flavorize_word(lang, "", w);
                word_chip(lang, w, &visible, xref, raw_xref, self_id)
            })
            .collect();
        format!("<div class='conn'><h5>{title}</h5><div class='chips'>{cs}</div></div>")
    };
    inner.push_str(&chips("Srodne slova", &e.related));
    inner.push_str(&chips("Sinonimy", &e.synonyms));
    inner.push_str(&chips("Antonimy", &e.antonyms));
    inner
}

#[allow(clippy::too_many_arguments)]
fn corpus_home(
    n: usize,
    lemma_total: usize,
    high: usize,
    med: usize,
    low: usize,
    official: usize,
    official_only: usize,
    borrowed: usize,
    rows: &[HomeRow],
) -> String {
    let mut list = String::from("<table class='wikitable'><thead><tr><th>Kandidat</th><th>Čęst rěči</th><th>Smysl</th><th>Sila dogadki</th><th>Srodne slova</th></tr></thead><tbody>");
    for r in rows.iter().take(400) {
        let langs = (r.freq as usize).max(1);
        // Official words (matched + official-only rows both carry
        // OfficialMatch) state the fact instead of a guess-strength number
        // (issue #86) — same treatment as the entry infobox badge.
        let strength = if matches!(r.status, MatchStatus::OfficialMatch) {
            "<span class='reliability conf-high'>oficialno</span>".to_string()
        } else {
            strength_cell(r.conf, r.prob, r.score)
        };
        let _ = write!(
            list,
            "<tr><td><a href='entry/{}.html'><b>{}</b></a></td><td>{}</td><td>{}</td><td>{}</td><td class='muted'>{}</td></tr>",
            r.id,
            esc(&r.form),
            esc(&pos_code_label(&r.pos)),
            esc(&truncate(&r.gloss, 50)),
            strength,
            langs
        );
    }
    list.push_str("</tbody></table>");

    let body = format!(
        "<section class='home-hero'>
           <h1 class='firstHeading'>Medžuslovjansky slovnik</h1>
           <p class='lede'>Naučno obosnovany generator medžuslovjanskyh slov iz slovjanskyh dokazov, měrjeny protiv oficialnogo slovnika. Iskaj v polju gore (Enter za vse rezultaty), ili prěgledaj slova niže.</p>
         </section>
         <div class='home-cols'>
           <article class='home-main'>
             <h2 id='slova'>Slova</h2>
             <p class='muted'>Prvyh 400 od <b>{total}</b> zapisov. „Sila dogadki“ = modelovy kȯšik uvěrjenosti + syrova ocěna.</p>
             {list}
           </article>
           <aside class='home-aside'>
             <div class='side-box'><div class='side-h'>Izbrano / slučajno</div><div id='spotlight'><p class='muted'>Nakladajě sę…</p></div><button id='randbtn' type='button'>Drugo slovo</button></div>
             <div class='side-box'><div class='side-h'>Wiki-navigacija</div><ul class='compact-list'><li><a href='special.html'>Speciaľne strany</a></li><li><a href='all-pages.html'>Vse strany</a></li><li><a href='categories.html'>Kategorije</a></li><li><a href='indices.html'>Abecedne indeksy</a></li><li><a href='portals.html'>Języčne portaly</a></li><li><a href='borrowings.html'>Pozajęta slova</a></li><li><a href='needs-review.html'>Trěbuje prověrky</a></li><li><a href='rules.html'>Indeks pravil</a></li><li><a href='proto-index.html'>Praslovjanske lemmy</a></li><li><a href='derivations.html'>Odvodženja po sufiksah</a></li><li><a href='site-stats.html'>Statistiky sajta</a></li><li><a href='graph.html'>Semantičny graf</a></li></ul></div>
             <div class='side-box'><div class='side-h'>Slovnik</div>
               <table class='wikitable compact-table'>
                 <tr><th>Slov</th><td>{total}</td></tr>
                 <tr><th>Lemmaty</th><td>{lemmas}</td></tr>
                 <tr><th>= oficialnomu</th><td>{official}</td></tr>
                 <tr><th>Samo oficialne</th><td>{official_only}</td></tr>
                 <tr><th>Pozajęta slova</th><td>{borrowed}</td></tr>
               </table>
             </div>
             <div class='side-box'><div class='side-h'>Uvěrjenost</div>
               <table class='wikitable compact-table'>
                 <tr><th>Vysoka</th><td>{high}</td></tr>
                 <tr><th>Srědnja</th><td>{med}</td></tr>
                 <tr><th>Nizka</th><td>{low}</td></tr>
               </table>
             </div>
             <div class='side-box'><div class='side-h'>Kako radi</div><ul class='compact-list'>
               <li>Medžuvětvovy konsensus (6 podgrup) izbira korenj.</li>
               <li>Praslovjansko pravilo davaje variantnu formu.</li>
               <li><a href='about.html'>O metodě →</a></li>
             </ul></div>
           </aside>
         </div>",
        total = compact(n),
        list = list,
        lemmas = compact(lemma_total),
        official = compact(official),
        official_only = compact(official_only),
        borrowed = compact(borrowed),
        high = compact(high),
        med = compact(med),
        low = compact(low),
    );
    page("Medžuslovjansky slovnik", &body, 0)
}

fn corpus_about(n: usize, lemma_total: usize, official: usize) -> String {
    let body = format!(
        "<article class='entry about'>
           <h1 class='firstHeading'>O metodě</h1>
           <p class='lede'>Toj slovnik je <b>statičny, dokazovy wiki-eksperiment</b>: ne kopija oficialnogo slovnika, ale generovany atlas slovjanskyh srodnyh slov, praslovjanskyh korenjev i medžuslovjanskyh kandidatov.</p>

           <table class='wikitable compact-table'>
             <tr><th>Srodne strany zapisov</th><td>{sets}</td><th>Slovjanske lemmaty v korpusu</th><td>{lemmas}</td></tr>
             <tr><th>Generovane formy s oficialnym sovpadenjem</th><td>{official}</td><th>Model sajta</th><td>prosty HTML + JSON, bez servera</td></tr>
           </table>

           <h2 id='kratko'>Kratko</h2>
           <p>Vsaka strana pytaje odgovor na wiki-podobno vprašanje: <i>ako mnogo slovjanskyh językov kaže na tu ideju, kaka medžuslovjanska forma je najvěrojętnějša, i čemu?</i> Zato strany zapisov pokazyvajųt ne samo slovo, ale i srodne slova, semantične vęzi, etimologiju, sled pravil, kategorije i izvory.</p>

           <h2 id='pipeline'>Kako nastaje zapis</h2>
           <pre class='pipeline-diagram'>Wiktionary lemmaty → srodne grupy → praslovjanske pravila → kandidaty → uvěrjenost → wiki-strana</pre>
           <ol>
             <li><b>Izvlečenje lemmatov.</b> Iz Wiktionary sȯbiramy slovjanske lemmy — imenniky, infinitivy glagolov, pozitivne pridavniki i internacionalizmy — zajedno s etimologičnym korenjem.</li>
             <li><b>Srodne grupy.</b> Lemmaty s tym že praslovjanskym prědkom ili s podobnym internacionalnym skeletom tvorę jednu grupu.</li>
             <li><b>Rekonstrukcija.</b> Praslovjansky pravilny stroj davaje variantno-medžuslovjansku formu; medžuvětvovy konsensus iz modernyh językov davaje alternativy.</li>
             <li><b>Ocěna dokaza.</b> Uvěrjenost raste s čislom językov i s pokrytjem trěh větvi: vȯzhod, zapad, jug.</li>
             <li><b>Wiki-sloj.</b> Sajt dodavaje kategorije, portaly, backlinks, homografne strany, semantičny graf i statične indeksy.</li>
           </ol>

           <h2 id='citati-entry'>Kako čitati stranu zapisa</h2>
           <ul>
             <li><b>Oznaka</b> govori, koliko językov i větvi podpira formu, i či ona sovpadaje s oficialnym slovnikom.</li>
             <li><b>Formy i kandidaty</b> pokazyvajųt alternativne pravopisy i rangy, ne samo poběditelja.</li>
             <li><b>Srodne slova</b> sųt surovy dokaz po slovjanskyh větvah; to je najvažnějša čęsť strany.</li>
             <li><b>Etimologija</b> veze zapis k praslovjanskoj rekonstrukciji ili internacionalnomu etimonu.</li>
             <li><b>Sled pravil</b> je sled prověrky: koje pravilo proměnilo formu i kako.</li>
             <li><b>Kategorije</b> i <b>portaly</b> pomagajų prěgledati slovnik kako wiki, ne samo kako polje iskanja.</li>
           </ul>

           <h2 id='wiki'>Wiki-navigacija</h2>
           <p>Najbolje startne točky: <a href='special.html'>posebne strany</a>, <a href='all-pages.html'>Vse strany</a>, <a href='categories.html'>Kategorije</a>, <a href='portals.html'>językove portaly</a>, <a href='borrowings.html'>portal zaimok</a>, <a href='needs-review.html'>spis za prověrku</a>, <a href='site-stats.html'>statistiky sajta</a>, <a href='graph.html'>semantičny graf</a>, <a href='rules.html'>indeks pravil (zvukove zakony)</a>, <a href='proto-index.html'>praslovjanske lemmy s refleksami</a>, <a href='derivations.html'>odvodženja po sufiksah</a> i <a href='metrics.html'>statistiky točnosti</a>.</p>

           <h2 id='validacija'>Validacija i granice</h2>
           <p>{official} generovanyh slov sovpadaje s oficialnym medžuslovjanskim slovnikom. To je kontrola, ale ne jedin cilj: mnogo validnyh medžuslovjanskyh slov može byti synonymami, regionalnymi izborami ili novymi kandidami, ktoryh oficialny slovnik ne imaje.</p>
           <p>Slabe strany sųt jasno označene: mala językova pokrytosť, nizka uvěrjenost, homografi, neoficialny stav i mašinno prěgibanje. Strana zato davaje <a href='{repo}/issues'>linky problemov</a> i kuratorske noty.</p>

           <h2 id='licencija'>Izvory i licencija</h2>
           <p>Dokazy i etimologije: Wiktionary i narodny Wiktionary (CC BY-SA). Oficialny slovnik: interslavic-dictionary.com. Prěgibanje: <code>interslavic-rs</code>. Kod projekta: <a href='{repo}'>MIT na GitHub</a>.</p>
         </article>",
        lemmas = compact(lemma_total),
        sets = compact(n),
        official = compact(official),
        repo = REPO_URL,
    );
    page("O metodě — medžuslovjansky generator", &body, 0)
}

fn build_input(entry: &OfficialEntry) -> MeaningInput {
    let forms = crate::consensus::source_forms_from_cells(&entry.cells, |code, form| {
        format!(
            "https://en.wiktionary.org/wiki/{}#{}",
            form.replace(' ', "_"),
            code
        )
    });
    let forms = crate::consensus::lemma_forms(forms, entry.pos);
    let (forms, reflexive) = crate::consensus::strip_reflexive(forms, entry.pos);
    MeaningInput {
        pos: entry.pos,
        gender: entry.noun_traits.gender,
        gloss: entry.english.clone(),
        forms,
        is_intl_meaning: entry.genesis.trim() == "I",
        reflexive,
    }
}

fn branch_evidence(input: &MeaningInput) -> Vec<Evidence> {
    input
        .forms
        .iter()
        .map(|f| Evidence {
            lang_code: f.lang_code.clone(),
            lang_name: crate::lang::lang_name(&f.lang_code).to_string(),
            branch: Some(f.branch),
            form: f.norm.original.clone(),
            normalized_form: f.norm.latin.clone(),
            relation: crate::model::EvidenceRelation::Cognate,
            source_url: f.source_url.clone(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Home page
// ---------------------------------------------------------------------------

/// One row of the home word list.
struct HomeRow {
    freq: f32,
    id: usize,
    form: String,
    gloss: String,
    pos: String,
    status: MatchStatus,
    conf: Confidence,
    score: f32,
    /// Calibrated probability shown in the strength cell (issue #77); `None`
    /// for official-only rows and calibrator-less exports (raw score shown).
    prob: Option<f64>,
}

/// Compact strength letter for the search index (V/S/N = high/medium/low).
fn conf_letter(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "V",
        Confidence::Medium => "S",
        Confidence::Low => "N",
    }
}

/// The "guess strength" cell: the confidence badge plus the calibrated
/// probability when a calibrator is fitted (issue #77), else the raw ranking
/// score (which is NOT a probability — ECE 0.185).
fn strength_cell(conf: Confidence, prob: Option<f64>, raw_score: f32) -> String {
    let num = match prob {
        Some(p) => format!("p≈{p:.2}"),
        None => format!("{raw_score:.2}"),
    };
    format!(
        "<span class='reliability {}'>{}</span> <span class='score muted'>{}</span>",
        conf_class(conf),
        conf.label(),
        num
    )
}

#[allow(clippy::too_many_arguments)]
fn home_page(
    n: usize,
    n_match: usize,
    n_diff: usize,
    n_none: usize,
    norm_rate: f32,
    exact_rate: f32,
    top_rows: &[HomeRow],
) -> String {
    let mut list = String::from("<table class='wikitable'><thead><tr><th>Kandidat</th><th>Čęst rěči</th><th>Anglijski smysl</th><th>Sila dogadki</th><th>Stav</th></tr></thead><tbody>");
    for r in top_rows.iter().take(300) {
        let _ = write!(
            list,
            "<tr><td><a href='entry/{}.html'><b>{}</b></a></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            r.id,
            esc(&r.form),
            esc(&pos_code_label(&r.pos)),
            esc(&truncate(&r.gloss, 55)),
            strength_cell(r.conf, r.prob, r.score),
            status_pill(r.status)
        );
    }
    list.push_str("</tbody></table>");

    let body = format!(
        "<section class='home-heading'>
           <h1 class='firstHeading'>Medžuslovjansky generator</h1>
           <p class='muted'>Naučno obosnovany generator medžuslovjanskyh slov iz slovjanskyh dokazov, s ocěnkoju točnosti protiv oficialnogo slovnika.</p>
           <div class='searchbox'><input id='q' type='search' placeholder='Iskaj po kandidatu ili anglijskom smyslu…' autocomplete='off'><div id='results' class='results'></div></div>
         </section>
         <section class='wiki-layout'>
           <article class='wiki-main-list'>
             <h2>Najčęstěje slova</h2>
             <p class='muted'>Najčęstějih 300 od <b>{total}</b> zapisov; iskaj gore za vse. „Sila dogadki“ = modelovy kȯšik uvěrjenosti + syrova ocěna.</p>
             {list}
           </article>
           <aside class='wiki-sidebar'>
             <div class='portal-box'><h3>Slučajno slovo</h3>
               <div id='spotlight'><p class='muted'>Nakladajě sę…</p></div>
               <button id='randbtn' type='button'>Drugo slovo</button>
             </div>
             <div class='portal-box stats-portal'><h3>Slovnik i točnosť</h3>
               <table class='wikitable compact-table'>
                 <tr><th>Zapisov</th><td>{total}</td></tr>
                 <tr><th>Odgovara oficialnomu</th><td>{n_match} ({norm:.1}%)</td></tr>
                 <tr><th>Razlikuje sę</th><td>{n_diff}</td></tr>
                 <tr><th>Točno (povno)</th><td>{exact:.1}%</td></tr>
                 <tr><th>Bez oficialnoj</th><td>{n_none}</td></tr>
               </table>
             </div>
             <div class='portal-box'><h3>Kako radi</h3><ul class='compact-list'>
               <li>Medžuvětvovy konsensus (6 podgrup) izbira korenj.</li>
               <li>Praslovjansko pravilo davaje variantnu formu.</li>
               <li>Sila dogadki = modelovy kȯšik uvěrjenosti; bez sovmestimoj kalibracije to ne jest věrojętnosť.</li>
               <li><a href='about.html'>O metodě →</a></li>
             </ul></div>
             <div class='portal-box'><h3>Legenda</h3>
               <p>{ok} — generovana forma = oficialna.</p>
               <p>{warn} — razlikuje sę od oficialnoj.</p>
               <p>{info} — nema oficialnoj.</p>
             </div>
           </aside>
         </section>
         <script>{js}</script>",
        total = compact(n),
        list = list,
        n_match = compact(n_match),
        norm = norm_rate,
        n_diff = compact(n_diff),
        exact = exact_rate,
        n_none = compact(n_none),
        ok = status_pill(MatchStatus::OfficialMatch),
        warn = status_pill(MatchStatus::DiffersFromOfficial),
        info = status_pill(MatchStatus::NoOfficialEntry),
        js = search_js(),
    );
    page("Medžuslovjansky generator", &body, 0)
}

// ---------------------------------------------------------------------------
// Sharded search index (issue #71). The monolithic search.json grew to 44 MB,
// so the index is written as first-letter shards: a query fetches a few-KB
// manifest plus exactly one shard. A row is listed in the shard of the folded
// first letter of EVERY string it can be found by (display, keys, gloss
// segments, source aliases in both scripts) — that multi-bucket listing is
// what keeps one-fetch lookups complete. Hot buckets split by second letter.
// ---------------------------------------------------------------------------

/// One staged search row. `head` is the 14-element row WITHOUT the trailing
/// aliases element or closing bracket; `aliases` is element 13's JSON (the
/// aliases must stay LAST so this split keeps working). The split lets
/// browse/spotlight files reuse the row bytes without the alias payload that
/// dominates the index size.
struct SearchRow {
    id: usize,
    head: String,
    aliases: String,
    /// O/N rows feed browse.json (filter-browse, substring fallback) and the
    /// spotlight sample; R rows are reachable through queries only.
    core: bool,
    /// (first, second-or-'_') folded letters of every searchable string.
    buckets: std::collections::BTreeSet<(char, char)>,
}

impl SearchRow {
    fn full(&self) -> String {
        format!("{},{}]", self.head, self.aliases)
    }
    fn no_alias(&self) -> String {
        format!("{},[]]", self.head)
    }
}

/// The client search fold, defined ONCE here and injected into SEARCH_JS as
/// `__SEARCH_FOLD__` — the exporter's bucketing and the browser's query fold
/// can never drift (closes #60 for the search page; the JS additionally
/// NFD-strips combining marks, which agrees with these pairs on every
/// precomposed letter). Latin diacritics fold to base letters; Cyrillic passes
/// through, giving Cyrillic queries their own shard alphabet.
const CLIENT_FOLD_PAIRS: &[(char, &str)] = &[
    ('á', "a"),
    ('à', "a"),
    ('â', "a"),
    ('ā', "a"),
    ('ǎ', "a"),
    ('å', "a"),
    ('ä', "a"),
    ('ą', "a"),
    ('ă', "a"),
    ('ã', "a"),
    ('é', "e"),
    ('è', "e"),
    ('ê', "e"),
    ('ë', "e"),
    ('ē', "e"),
    ('ě', "e"),
    ('ę', "e"),
    ('ė', "e"),
    ('í', "i"),
    ('ì', "i"),
    ('î', "i"),
    ('ï', "i"),
    ('ī', "i"),
    ('ó', "o"),
    ('ò', "o"),
    ('ô', "o"),
    ('ö', "o"),
    ('õ', "o"),
    ('ō', "o"),
    ('ŏ', "o"),
    ('ȯ', "o"),
    ('ő', "o"),
    ('ø', "o"),
    ('ú', "u"),
    ('ù', "u"),
    ('û', "u"),
    ('ü', "u"),
    ('ū', "u"),
    ('ů', "u"),
    ('ų', "u"),
    ('ű', "u"),
    ('ý', "y"),
    ('ÿ', "y"),
    ('ỳ', "y"),
    ('č', "c"),
    ('ć', "c"),
    ('ç', "c"),
    ('š', "s"),
    ('ś', "s"),
    ('ş', "s"),
    ('ž', "z"),
    ('ź', "z"),
    ('ż', "z"),
    ('đ', "d"),
    ('ď', "d"),
    ('ť', "t"),
    ('ţ', "t"),
    ('ň', "n"),
    ('ń', "n"),
    ('ñ', "n"),
    ('ľ', "l"),
    ('ĺ', "l"),
    ('ł', "l"),
    ('ř', "r"),
    ('ŕ', "r"),
    ('ß', "ss"),
    ('æ', "ae"),
    ('œ', "oe"),
];

/// Rust twin of the injected JS fold: lowercase, strip combining marks, fold
/// per [`CLIENT_FOLD_PAIRS`].
fn client_fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.to_lowercase().chars() {
        if ('\u{0300}'..='\u{036F}').contains(&c) {
            continue;
        }
        match CLIENT_FOLD_PAIRS.iter().find(|(f, _)| *f == c) {
            Some((_, r)) => out.push_str(r),
            None => out.push(c),
        }
    }
    out
}

/// The shard bucket of one searchable string: its first folded alphanumeric
/// letter, plus the second (or '_' when the string has only one) for
/// hot-bucket splits. None for strings with no letters at all.
fn search_bucket_pair(s: &str) -> Option<(char, char)> {
    let f = client_fold(s);
    let mut it = f.chars().filter(|c| c.is_alphanumeric());
    let b1 = it.next()?;
    Some((b1, it.next().unwrap_or('_')))
}

/// Every string a row can be found by → its bucket-pair set: the display
/// headword, every search key (candidate forms/folds + gloss tokens), every
/// gloss segment (the client's rank-55 exact-segment match splits on `,;`),
/// and every source alias, verbatim and folded (Cyrillic verbatim aliases give
/// the row a Cyrillic bucket, so `пластинка` is a one-shard query).
fn search_row_buckets(
    display: &str,
    gloss: &str,
    keys: &[(String, usize)],
    aliases: &[SourceAlias],
) -> std::collections::BTreeSet<(char, char)> {
    let mut b = std::collections::BTreeSet::new();
    let mut add = |s: &str| {
        if let Some(p) = search_bucket_pair(s) {
            b.insert(p);
        }
    };
    add(display);
    for (k, _) in keys {
        add(k);
    }
    for seg in gloss.split([',', ';']) {
        add(seg.trim());
    }
    for (_, word, folds) in aliases {
        add(word);
        for f in folds {
            add(f);
        }
    }
    b
}

/// Shard-key → file basename: ASCII alphanumerics keep their letter, anything
/// else is `uXXXX` hex — stable, URL-safe, collision-free per key.
fn shard_file_name(key: &str) -> String {
    let mut name = String::new();
    for c in key.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            name.push(c);
        } else {
            let _ = write!(name, "u{:04x}", c as u32);
        }
    }
    format!("{name}.json")
}

/// Buckets larger than this (serialized bytes) split by second letter.
const SHARD_SPLIT_BUDGET: usize = 1_500_000;

/// Write the sharded search index under `out_dir/search/`: per-letter shard
/// files, `manifest.json` (key → file, loaded eagerly by the client),
/// `browse.json` (core O/N rows without aliases — empty-query filter browse
/// and the substring fallback), and `spotlight.json` (a deterministic sample
/// of high-confidence rows for the random-word widgets). Ends with a
/// completeness self-check: every bucket pair of every row must RESOLVE (the
/// client's key2→key1 rule) to a shard that contains the row — a violation is
/// a hard error, never a silently unfindable page.
fn write_search_index(out_dir: &Path, rows: &[SearchRow]) -> Result<(usize, usize)> {
    let dir = out_dir.join("search");
    std::fs::create_dir_all(&dir)?;
    // Group row indices by first letter (deduped per bucket).
    let mut by1: BTreeMap<char, Vec<usize>> = BTreeMap::new();
    for (i, r) in rows.iter().enumerate() {
        let mut seen = std::collections::BTreeSet::new();
        for &(b1, _) in &r.buckets {
            if seen.insert(b1) {
                by1.entry(b1).or_default().push(i);
            }
        }
    }
    // Decide splits, produce key → row-index lists.
    let mut shards: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut splits: Vec<char> = Vec::new();
    for (b1, idxs) in &by1 {
        let bytes: usize = idxs.iter().map(|&i| rows[i].full().len() + 2).sum();
        if bytes > SHARD_SPLIT_BUDGET {
            splits.push(*b1);
            for &i in idxs {
                let mut seen = std::collections::BTreeSet::new();
                for &(p1, p2) in &rows[i].buckets {
                    if p1 == *b1 && seen.insert(p2) {
                        shards.entry(format!("{b1}{p2}")).or_default().push(i);
                    }
                }
            }
        } else {
            shards.insert(b1.to_string(), idxs.clone());
        }
    }
    // Write shard files + manifest.
    let mut manifest_shards = serde_json::Map::new();
    for (key, idxs) in &shards {
        let file = shard_file_name(key);
        let mut body = String::from("[\n");
        for (n, &i) in idxs.iter().enumerate() {
            if n > 0 {
                body.push_str(",\n");
            }
            body.push_str(&rows[i].full());
        }
        body.push_str("\n]\n");
        std::fs::write(dir.join(&file), &body)?;
        manifest_shards.insert(
            key.clone(),
            serde_json::json!({ "f": file, "n": idxs.len() }),
        );
    }
    // Core browse file (no aliases) + deterministic spotlight sample.
    let core: Vec<&SearchRow> = rows.iter().filter(|r| r.core).collect();
    let mut browse = String::from("[\n");
    for (n, r) in core.iter().enumerate() {
        if n > 0 {
            browse.push_str(",\n");
        }
        browse.push_str(&r.no_alias());
    }
    browse.push_str("\n]\n");
    std::fs::write(dir.join("browse.json"), &browse)?;
    let step = (core.len() / 1024).max(1);
    let mut spot = String::from("[\n");
    for (n, r) in core.iter().step_by(step).enumerate() {
        if n > 0 {
            spot.push_str(",\n");
        }
        spot.push_str(&r.no_alias());
    }
    spot.push_str("\n]\n");
    std::fs::write(dir.join("spotlight.json"), &spot)?;
    let manifest = serde_json::json!({
        // 2 = 14-element rows (razumlivost at 12, aliases moved to 13; #79).
        "schema": 2,
        "totalRows": rows.len(),
        "browse": "browse.json",
        "spotlight": "spotlight.json",
        "splits": splits.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "shards": manifest_shards,
    });
    let mut mbytes = serde_json::to_vec(&manifest)?;
    mbytes.push(b'\n');
    std::fs::write(dir.join("manifest.json"), mbytes)?;

    // Completeness self-check (loud; the client resolution rule, mirrored).
    let shard_ids: BTreeMap<&String, std::collections::HashSet<usize>> = shards
        .iter()
        .map(|(k, idxs)| (k, idxs.iter().map(|&i| rows[i].id).collect()))
        .collect();
    for r in rows {
        for &(b1, b2) in &r.buckets {
            let k2 = format!("{b1}{b2}");
            let k1 = b1.to_string();
            let hit = shard_ids
                .get(&k2)
                .or_else(|| shard_ids.get(&k1))
                .is_some_and(|ids| ids.contains(&r.id));
            anyhow::ensure!(
                hit,
                "search shard completeness violated: row {} bucket ({b1},{b2}) resolves to no shard containing it",
                r.id
            );
        }
    }
    Ok((shards.len(), core.len()))
}

/// SEARCH_JS with the generated fold map injected (computed once).
fn search_js() -> &'static str {
    static JS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    JS.get_or_init(|| {
        let mut map = String::from("{");
        for (i, (c, r)) in CLIENT_FOLD_PAIRS.iter().enumerate() {
            if i > 0 {
                map.push(',');
            }
            let _ = write!(map, "{}:{}", json_str(&c.to_string()), json_str(r));
        }
        map.push('}');
        SEARCH_JS.replace("__SEARCH_FOLD__", &map)
    })
}

/// Deduplicated searchable keys for one entry: every ranked candidate's form
/// plus its standard-alphabet and ASCII folds, tagged with the candidate rank
/// (1-based) so the client can deep-link an alternative hit (`#cand-2`). The
/// display form itself is excluded (the client already matches it), but its
/// folds are included so `kratoky` finds `kråtȯky`.
fn search_keys(candidates: &[Candidate], display: &str) -> Vec<(String, usize)> {
    let mut keys: Vec<(String, usize)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    seen.insert(display.to_lowercase());
    for (i, c) in candidates.iter().take(5).enumerate() {
        let lower = c.form.to_lowercase();
        for k in [
            lower.clone(),
            crate::orthography::to_standard(&lower),
            crate::orthography::ascii_skeleton(&c.form),
        ] {
            if k.chars().count() >= 2 && seen.insert(k.clone()) {
                keys.push((k, i + 1));
            }
        }
    }
    keys
}

/// JSON-encode the key list as `[["kratky",2],…]` for the search index row.
fn keys_json(keys: &[(String, usize)]) -> String {
    let mut s = String::from("[");
    for (i, (k, r)) in keys.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "[{},{}]", json_str(k), r);
    }
    s.push(']');
    s
}

/// One source-word alias for the search index: `(language code, attested word,
/// folded search forms)`. The attested word is matched verbatim (so a Cyrillic
/// query hits it); the folded forms — phonemic Latin, standard fold, ASCII
/// skeleton — let a transliterated or diacritic-folded query hit it too.
type SourceAlias = (String, String, Vec<String>);

/// The committee's source cells for one official entry, in a deterministic order
/// (the 12 Slavic CSV columns, then `de`/`nl`/`eo`). Kept stable so `search.json`
/// is byte-reproducible despite `cells` being a `HashMap`.
fn official_cell_pairs(e: &OfficialEntry) -> Vec<(&str, &str)> {
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    for li in crate::lang::LANGS.iter() {
        if li.csv_col.is_empty() {
            continue;
        }
        if let Some(cell) = e.cells.get(li.code) {
            pairs.push((li.code, cell.as_str()));
        }
    }
    for (code, cell) in [("de", &e.de), ("nl", &e.nl), ("eo", &e.eo)] {
        if !cell.trim().is_empty() {
            pairs.push((code, cell.as_str()));
        }
    }
    pairs
}

/// Fold `(lang, raw cell)` pairs into deduplicated [`SourceAlias`]es (issue #31).
///
/// Each cell is split into its listed variants with the same
/// [`normalize::normalize_cell`] the generation path uses, so a multi-variant
/// cell (`быстрый, скорый`) yields one alias per variant. Per variant we emit the
/// attested spelling plus its phonemic-Latin / standard-fold / ASCII-skeleton
/// search forms. This is verbatim **dictionary evidence** (the committee/cognate
/// spelling), never generated content. Dedup is by `(lang, attested word)`; the
/// caller shares one `seen` set across sources so a member and a committee cell
/// for the same word collapse.
fn collect_source_aliases<'a>(
    cells: impl IntoIterator<Item = (&'a str, &'a str)>,
    aliases: &mut Vec<SourceAlias>,
    seen: &mut std::collections::HashSet<(String, String)>,
) {
    for (code, cell) in cells {
        for nf in crate::normalize::normalize_cell(code, cell) {
            let original = nf.original.trim().to_lowercase();
            if original.chars().count() < 2 {
                continue;
            }
            if !seen.insert((code.to_string(), original.clone())) {
                continue;
            }
            let mut forms: Vec<String> = Vec::new();
            for f in [
                nf.latin.clone(),
                crate::orthography::to_standard(&nf.latin),
                nf.skeleton.clone(),
            ] {
                if f.chars().count() >= 2 && f != original && !forms.contains(&f) {
                    forms.push(f);
                }
            }
            aliases.push((code.to_string(), original, forms));
        }
    }
}

/// JSON-encode the alias list as `[["ru","пластинка",["plastinka"]],…]`.
fn source_aliases_json(aliases: &[SourceAlias]) -> String {
    let mut s = String::from("[");
    for (i, (lang, orig, forms)) in aliases.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "[{},{},[", json_str(lang), json_str(orig));
        for (j, f) in forms.iter().enumerate() {
            if j > 0 {
                s.push(',');
            }
            s.push_str(&json_str(f));
        }
        s.push_str("]]");
    }
    s.push(']');
    s
}

// Client-side search. Loaded on EVERY page (the search box lives in the header),
// so SITE_BASE ('' at root, '../' under /entry/) resolves the fetch and links.
// Typing shows a top-8 dropdown; Enter (or the full-results link) goes to
// search.html?q, which lists every match.
const SEARCH_JS: &str = r#"
// Sharded index (issue #71): a few-KB manifest loads on demand, then each
// query fetches exactly one first-letter shard. browse.json (core rows, no
// aliases) backs empty-query filter browsing, the substring fallback, and the
// random-word widgets via spotlight.json.
let MANIFEST=null, SHARDS={}, BROWSE=null, SPOT=null, NOTE='';
async function j(p){ const r=await fetch(SITE_BASE+p); return await r.json(); }
async function manifest(){ if(!MANIFEST) MANIFEST=await j('search/manifest.json'); return MANIFEST; }
async function browseRows(){ if(!BROWSE){ var m=await manifest(); BROWSE=await j('search/'+m.browse); } return BROWSE; }
async function spotRows(){ if(!SPOT){ var m=await manifest(); SPOT=await j('search/'+m.spotlight); } return SPOT; }
async function shardFor(sf){ var m=await manifest();
  var letters=''; for(var i=0;i<sf.length&&letters.length<2;i++){ if(/[\p{L}\p{N}]/u.test(sf[i]))letters+=sf[i]; }
  if(!letters)return null;
  var k2=letters.length>1?letters:letters+'_', k1=letters[0];
  var k=m.shards[k2]?k2:(m.shards[k1]?k1:null); if(!k)return null;
  if(!SHARDS[k]) SHARDS[k]=await j('search/'+m.shards[k].f);
  return SHARDS[k]; }
var q=document.getElementById('q'), out=document.getElementById('results'), pageRes=document.getElementById('page-results');
var STR={V:['vysoka','conf-high'],S:['srědnja','conf-med'],N:['nizka','conf-low']};
var POS={noun:'imennik',proper_noun:'vlastno imę',verb:'glagol',adj:'pridavnik',adv:'narěčje',num:'čislovnik',pron:'zaimennik'};
function posLabel(p){return POS[p]||p||'';}
function strBadge(e){ var s=STR[e[5]]||STR.N; return "<span class='reliability "+s[1]+"'>"+s[0]+"</span>"; }
function closeDropdown(){ if(out){ out.style.display='none'; out.innerHTML=''; } }
// The fold map is GENERATED by the exporter (CLIENT_FOLD_PAIRS) so client
// folding can never drift from shard bucketing (#60); NFD-stripping agrees
// with the map on every precomposed letter and additionally cleans combining
// marks typed separately.
var FOLDMAP=__SEARCH_FOLD__;
function fold(x){ x=(x||'').toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g,''); var o=''; for(var i=0;i<x.length;i++){ var c=x[i]; o+=(FOLDMAP[c]!==undefined?FOLDMAP[c]:c); } return o; }
// International committee columns (de/nl/eo) rank below the 12 Slavic cognates.
var INTL={de:1,nl:1,eo:1};
// Best source-word alias match for the query (issue #31 dictionary evidence:
// verbatim committee/cognate spellings, e[13]). Ranks exact source word high
// (just under the ISV headword), then transliteration/fold, then prefix; the
// international columns weigh less. Returns [score,'lang word'] so the hit can
// show why it matched.
function aliasMatch(al,s2,sf){ var best=0,lab='';
  for(var i=0;i<al.length;i++){ var a=al[i],lang=a[0],w=a[1]||'',wl=w.toLowerCase(),wf=fold(wl),fs=a[2]||[],lo=INTL[lang]?1:0,sc=0;
    if(wl===s2||wl===sf){ sc=lo?62:82; }
    else{ var hit=(wf===sf); for(var j=0;!hit&&j<fs.length;j++){ if(fs[j]===s2||fs[j]===sf)hit=1; } if(hit){ sc=lo?54:72; }
      else if(sf.length>=2){ var pre=(wl.indexOf(s2)===0||wf.indexOf(sf)===0); for(var j2=0;!pre&&j2<fs.length;j2++){ if(fs[j2].indexOf(sf)===0)pre=1; } if(pre){ sc=lo?44:56; } } }
    if(sc>best){ best=sc; lab=lang+' '+w; } }
  return [best,lab]; }
function filters(){ return {
  pos:(document.getElementById('f-pos')||{}).value||'', status:(document.getElementById('f-status')||{}).value||'',
  conf:(document.getElementById('f-conf')||{}).value||'', borrowed:(document.getElementById('f-borrowed')||{}).value||'',
  langs:parseInt((document.getElementById('f-langs')||{}).value||'0',10)||0
}; }
function pass(e,f){ if(f.pos&&e[3]!==f.pos)return false; if(f.status&&e[4]!==f.status)return false; if(f.conf&&e[5]!==f.conf)return false; if(f.borrowed!==''&&String(e[9]||0)!==f.borrowed)return false; if(f.langs&&Number(e[7]||0)<f.langs)return false; return true; }
function scoreRows(ROWS,raw,showAll){
  var s=(raw||'').trim().toLowerCase(), ftr=filters(); var s2=s.replace(/^to\s+/,''), sf=fold(s2), hits=[];
  for(var i=0;i<ROWS.length;i++){ var e=ROWS[i]; if(!pass(e,ftr))continue; var f=e[1].toLowerCase(), g=e[2].toLowerCase(), ks=e[6]||[];
    var gs=g.split(/[,;]\s*/), ff=fold(f), sc=showAll?1:0, anchor=0, srclab='';
    if(!showAll){
      if(f===s||f===s2)sc=100; else if(ff===sf)sc=90;
      else{ for(var k=0;k<ks.length;k++){ var kr=ks[k]; if(kr[0]===s2||kr[0]===sf){ sc=85-3*Math.min(kr[1],5); if(kr[1]>1&&kr[1]<6)anchor=kr[1]; break; } } }
      if(!sc){ if(f.indexOf(s2)===0||ff.indexOf(sf)===0)sc=60;
        else if(gs.some(function(x){return x.trim()===s||x.trim()===s2;}))sc=55;
        else if(ks.some(function(kr){return kr[0].indexOf(sf)===0;}))sc=50;
        else if(f.indexOf(s2)>=0)sc=40; else if(g.indexOf(s2)>=0)sc=20; }
      // A Slavic source/cognate match (committee evidence) outranks a mere
      // form/gloss substring and annotates the hit with the matched word.
      var am=aliasMatch(e[13]||[],s2,sf); if(am[0]>sc){ sc=am[0]; anchor=0; srclab=am[1]; } else if(am[0]>0&&am[0]===sc){ srclab=am[1]; }
    }
    if(sc>0)hits.push([sc,e,anchor,srclab]); if(hits.length>5000)break; }
  hits.sort(function(a,b){return b[0]-a[0] || a[1][1].localeCompare(b[1][1]);}); return hits;
}
// One query end-to-end: resolve + fetch the query's shard, score it, and when
// the shard yields little, widen with a curated-substring pass over the core
// browse rows (dedup by id; shard hits win). Empty query on the search page =
// filter browse over the core rows (raw attestations need a typed query).
async function searchFor(raw){
  NOTE='';
  var s=(raw||'').trim().toLowerCase();
  if(pageRes&&!s){ if(filters().status==='R')NOTE='Surove atestacije sųt dostupne prěz zapyt (napiši slovo), ne prěz prěgled.';
    return scoreRows(await browseRows(),raw,true); }
  var s2=s.replace(/^to\s+/,''), sf=fold(s2);
  var rows=await shardFor(sf)||[];
  var hits=scoreRows(rows,raw,false);
  var m=await manifest();
  if(s2.length===1&&m.splits.indexOf(sf[0])>=0)NOTE='Jedna bukva: pokazane sųt samo jednobukvene formy — napiši ješče bukvu.';
  if(hits.length<8&&s2.length>=3){
    var extra=scoreRows(await browseRows(),raw,false), have={};
    for(var i=0;i<hits.length;i++)have[hits[i][1][0]]=1;
    for(var k=0;k<extra.length;k++){ if(!have[extra[k][1][0]])hits.push(extra[k]); }
    hits.sort(function(a,b){return b[0]-a[0] || a[1][1].localeCompare(b[1][1]);});
  }
  return hits;
}
function eh(s){return String(s==null?'':s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');}
function hitHTML(e,a,src){ var meta="<span class='hs'>"+strBadge(e)+"</span> <span class='hq'>"+eh(e[10]||'')+"</span>"; if(e[11])meta+=" <span class='ha'>"+eh(e[11])+"</span>"; meta+=" <span class='hl'>"+(e[7]||0)+" jęz. / "+(e[8]||0)+" vět.</span>"; if(e[12]!=null)meta+=" <span class='hrz' title='razumlivosť: dolja govoriteljev slovjanskyh językov s poznatym srodnym ili istym slovom (po atestaciji) — ne izměrjena razumlivosť'>"+e[12]+"%</span>"; if(src)meta+=" <span class='hsrc' title='Slovnikovy dokaz: perevod komiteta / kognat'>"+eh(src)+"</span>"; return "<a class='hit' href='"+SITE_BASE+"entry/"+e[0]+".html"+(a?('#cand-'+a):'')+"'><b>"+eh(e[1])+"</b> <span class='hp'>"+eh(posLabel(e[3]))+"</span> <span class='hg'>"+eh(e[2])+"</span> "+meta+"</a>"; }
async function run(showDropdown){
  var v=q?q.value:''; var hits=await searchFor(v);
  var note=NOTE?"<div class='muted nohit'>"+eh(NOTE)+"</div>":'';
  // The search page has full results below the filters, so never reopen the
  // compact header dropdown there. Filter changes also pass showDropdown=false.
  if(out){ if(showDropdown && !pageRes && v.trim()){ var h=hits.slice(0,8).map(function(x){return hitHTML(x[1],x[2],x[3]);}).join('');
      if(!h)h="<div class='muted nohit'>Ničto ne najdeno.</div>";
      else if(hits.length>8)h+="<a class='hit more' href='"+SITE_BASE+"search.html?q="+encodeURIComponent(v.trim())+"'>Vse "+hits.length+" rezultatov -></a>";
      out.innerHTML=note+h; out.style.display='block'; } else closeDropdown(); }
  if(pageRes){ var c=document.getElementById('rescount'); if(c)c.textContent=hits.length;
    pageRes.innerHTML=note+(hits.slice(0,400).map(function(x){return hitHTML(x[1],x[2],x[3]);}).join('')||"<div class='muted'>Ničto ne najdeno.</div>"); }
}
function goSearch(e){
  e.preventDefault(); var v=q?q.value.trim():''; closeDropdown(); if(q)q.blur();
  if(pageRes){ if(history.replaceState){ history.replaceState(null,'',SITE_BASE+'search.html'+(v?'?q='+encodeURIComponent(v):'')); } run(false); return false; }
  if(v) location.href=SITE_BASE+'search.html?q='+encodeURIComponent(v);
  return false;
}
if(q){ var t=null; q.addEventListener('input',function(){ clearTimeout(t); t=setTimeout(function(){ run(true); },110); });
  q.addEventListener('focus',function(){ if(q.value.trim())run(true); });
  q.addEventListener('keydown',function(ev){ if(ev.key==='Escape'){ closeDropdown(); q.blur(); } }); }
['f-pos','f-status','f-conf','f-borrowed','f-langs'].forEach(function(id){ var el=document.getElementById(id); if(el)el.addEventListener('input',function(){run(false);}); if(el)el.addEventListener('change',function(){run(false);}); });
document.addEventListener('click',function(ev){ if(out&&!ev.target.closest('.hsearch'))closeDropdown(); });
async function randomWord(){ var SP=await spotRows(); if(!SP.length)return; var pool=SP.filter(function(e){return e[5]==='V'||e[4]==='O'}); if(!pool.length)pool=SP; var e=pool[Math.floor(Math.random()*pool.length)];
  var el=document.getElementById('spotlight'); if(!el)return; var box=document.getElementById('spotbox'); if(box)box.style.display='';
  el.innerHTML="<a class='spotlight-word' href='"+SITE_BASE+"entry/"+e[0]+".html'>"+eh(e[1])+"</a><div class='muted'>"+eh(posLabel(e[3]))+" · "+eh(e[2])+"</div><div class='spot-strength'>"+strBadge(e)+" "+eh(e[10]||'')+"</div>"; }
var rb=document.getElementById('randbtn'); if(rb) rb.addEventListener('click',randomWord);
if(document.getElementById('spotlight')) randomWord();
(function(){ var p=new URLSearchParams(location.search).get('q'); if(p&&q)q.value=p; if(pageRes||p)run(false); })();
"#;
/// Builds the "Na toj strane" contents tree in the sidebar from the section
/// headings, and hides the box when a page has none (home / search).
const TOC_JS: &str = r#"
(function(){ var nav=document.getElementById('toc-nav'); if(!nav)return;
  var hs=document.querySelectorAll('main h2[id], main h3[id]'); var box=nav.closest('.toc-box');
  if(!hs.length){ if(box)box.style.display='none'; return; }
  var html=''; hs.forEach(function(h){ html+="<a class='toc-"+h.tagName.toLowerCase()+"' href='#"+h.id+"'>"+h.textContent+"</a>"; });
  nav.innerHTML=html;
})();
"#;

// ---------------------------------------------------------------------------
// Entry page
// ---------------------------------------------------------------------------

fn entry_page(
    id: usize,
    entry: &OfficialEntry,
    g: &Generation,
    evidence: &[Evidence],
    cal: Option<&crate::calibrate::Calibration>,
) -> String {
    let top = g.candidates.first().unwrap();
    let status = g.match_status;
    let pos_code = entry.pos.code();

    let headline = format!(
        "<div class='headword-block'>
           <div class='headmeta'>
             <span class='badge pos'>{}</span>
             <span class='pill {}'>{}</span>
             <span class='reliability {}'>uvěrjenost: {}</span>
             {}
           </div>
           <p class='def'><b>Anglijski smysl:</b> {}</p>
         </div>",
        esc(&pos_heading(&entry.pos_raw)),
        source_class(top.source),
        esc(top.source.label()),
        conf_class(top.confidence),
        top.confidence.label(),
        status_pill(status),
        esc(&entry.english),
    );

    let banner = status_banner(status, top, entry.isv.as_str());
    let etymology = etymology_block(g);
    let inflection = inflection_table_g(&top.form, pos_code, entry.noun_traits.gender);
    let evidence_html = evidence_block(evidence);
    let alternatives = alternatives_block(&g.candidates, None);
    let trace = trace_block(top);
    let calib = calibration_note(top.confidence, cal);
    let freq = entry
        .frequency
        .map(|f| format!("<p class='muted'>Čęstota v slovniku: {f:.0}.</p>"))
        .unwrap_or_default();

    let body = format!(
        "<article class='entry'>
           <h1 class='page-title firstHeading'>{}</h1>
           {banner}
           {headline}
           {calib}{freq}
           <details class='sec' open><summary>Etimologija (praslovjanska rekonstrukcija)</summary>{etymology}</details>
           <details class='sec' open><summary>Prěgibanje</summary>{inflection}</details>
           <details class='sec' open><summary>Dokazy po slovjanskyh větvah</summary>{evidence_html}</details>
           <details class='sec'><summary>Alternativne kandidaty</summary>{alternatives}</details>
           <details class='sec'><summary>Sled pravil (kako je forma izvedena)</summary>{trace}</details>
           <p class='foot'>Lokalno generovana stranica. Formy prěgibanja iz interslavic-rs. Forma je mašinno generovana — ne oficialny standard bez prověrky.</p>
         </article>",
        esc(&top.form),
    );
    let _ = id;
    page(&format!("{} — medžuslovjansky", top.form), &body, 1)
}

fn status_banner(status: MatchStatus, top: &Candidate, official: &str) -> String {
    match status {
        MatchStatus::OfficialMatch => format!(
            "<div class='banner ok'><b>Oficialno potvŕđeno.</b> Generovana forma odgovara oficialnomu slovniku: <span class='mention'>{}</span>.</div>",
            esc(official)
        ),
        MatchStatus::DiffersFromOfficial => format!(
            "<div class='banner warn'><b>Razlikuje se od oficialnogo.</b> Generovany kandidat <span class='mention'>{}</span> · oficialna forma <span class='mention'>{}</span>.</div>",
            esc(&top.form),
            esc(official)
        ),
        MatchStatus::NoOfficialEntry => "<div class='banner info'><b>Nema oficialnogo zapisa.</b> Forma je čisto generovana iz slovjanskyh dokazov.</div>".to_string(),
    }
}

fn etymology_block(g: &Generation) -> String {
    let Some(r) = &g.reconstruction else {
        return "<p class='muted'>Za sej smysl ne najdena praslovjanska rekonstrukcija; forma je iz medžuvětvovogo konsensusa.</p>".to_string();
    };
    let mut s = format!(
        "<p>Iz praslovjanskogo <a class='mention' href='https://en.wiktionary.org/wiki/Reconstruction:Proto-Slavic/{}'>*{}</a> <span class='muted'>(uvěrjenost povezanja {:.0}%)</span>.</p>",
        esc(&r.word),
        esc(&r.word),
        100.0 * r.confidence
    );
    if !r.proto_balto_slavic.is_empty() {
        let _ = write!(
            s,
            "<p>Prabaltoslavjansky: <span class='mention'>{}</span>.</p>",
            esc(&r.proto_balto_slavic)
        );
    }
    if !r.proto_indo_european.is_empty() {
        let _ = write!(
            s,
            "<p>Praindoevropejsky: <span class='mention'>{}</span>.</p>",
            esc(&r.proto_indo_european)
        );
    }
    s.push_str("<p class='muted'>Medžuvětvovy konsensus izbira korenj; praslovjansko pravilo izvodi formu s pravilnymi znakami (ě, ć/đ, å, ȯ, y).</p>");
    s
}

fn alternatives_block(candidates: &[Candidate], top_razum_codes: Option<&[String]>) -> String {
    if candidates.is_empty() {
        return "<p class='muted'>Bez kandidatov.</p>".to_string();
    }
    // Always show the ranked forms (the top one is the headword); this is now a
    // primary section, so even a single-candidate entry lists its form + score.
    let mut s = String::from("<table class='wikitable'><thead><tr><th>#</th><th>Forma</th><th>Izvor</th><th title='rangovy ključ (syrova ocěna), ne věrojętnosť'>Ocěna</th><th title='");
    s.push_str(if top_razum_codes.is_some() {
        RAZUM_TITLE_MATCHED
    } else {
        RAZUM_TITLE
    });
    s.push_str("'>Razumlivosť</th><th>Větvi</th></tr></thead><tbody>");
    for (i, c) in candidates.iter().enumerate() {
        // Per-candidate razumlivost from its own cluster membership (issue
        // #79); an em-dash when the membership is unknown.
        let razum_codes = if i == 0 {
            top_razum_codes.unwrap_or(&c.langs)
        } else {
            &c.langs
        };
        let razum = if razum_codes.is_empty() {
            "—".to_string()
        } else {
            format!("{}%", razum_pct(razum_codes))
        };
        let _ = write!(
            s,
            "<tr id='cand-{}' class='{}'><td>{}</td><td><span class='mention'>{}</span></td><td><span class='pill {}'>{}</span></td><td class='score'>{:.3}</td><td class='score'>{}</td><td>{}</td></tr>",
            i + 1,
            if i == 0 { "top-candidate" } else { "" },
            i + 1,
            esc(&c.form),
            source_class(c.source),
            esc(c.source.label()),
            c.score,
            razum,
            c.branch_coverage
        );
    }
    s.push_str("</tbody></table>");
    s
}

fn trace_block(c: &Candidate) -> String {
    if c.trace.is_empty() {
        return "<p class='muted'>Bez transformacij (forma vzęta prěmo iz konsensusa).</p>"
            .to_string();
    }
    let mut s = String::from("<ol class='rule-trace'>");
    for step in &c.trace {
        let reference = step
            .reference
            .as_deref()
            .map(|r| format!(" <a class='doc-ref' href='{}'>[dok]</a>", esc(r)))
            .unwrap_or_default();
        let _ = write!(
            s,
            "<li><code class='rule-id'>{}</code>: <span class='mention'>{}</span> → <span class='mention'>{}</span><br><span class='muted'>{}</span>{}</li>",
            esc(&step.id), esc(&step.before), esc(&step.after), esc(&step.explanation), reference
        );
    }
    s.push_str("</ol>");
    if !c.warnings.is_empty() {
        s.push_str("<div class='notice'>");
        for w in &c.warnings {
            let _ = write!(s, "<p>⚠ {}</p>", esc(w));
        }
        s.push_str("</div>");
    }
    s
}

fn evidence_block(evidence: &[Evidence]) -> String {
    let mut s = String::new();
    for branch in Branch::ALL {
        let items: Vec<&Evidence> = evidence
            .iter()
            .filter(|ev| ev.branch == Some(branch))
            .collect();
        if items.is_empty() {
            continue;
        }
        let _ = write!(
            s,
            "<div class='branch-box'><h4>{}</h4><table class='wikitable compact-table'><tbody>",
            esc(branch.label())
        );
        for ev in items {
            let _ = write!(
                s,
                "<tr><td class='lc'>{}</td><td><a href='{}'>{}</a></td><td class='muted'>{}</td></tr>",
                esc(&ev.lang_name),
                esc(&ev.source_url),
                esc(&source_display(&ev.lang_code, &ev.form)),
                esc(&source_display(&ev.lang_code, &ev.normalized_form))
            );
        }
        s.push_str("</tbody></table></div>");
    }
    if s.is_empty() {
        "<p class='muted'>Bez dokazov.</p>".to_string()
    } else {
        format!("<div class='branch-grid'>{s}</div>")
    }
}

// ---------------------------------------------------------------------------
// Inflection (via the interslavic crate)
// ---------------------------------------------------------------------------

fn inflection_table(word: &str, pos_code: &str) -> String {
    inflection_table_g(word, pos_code, None)
}

/// As [`inflection_table`], with the dictionary's gender when known — the same
/// gendered declension the API records use (single source), so an
/// out-of-lexicon feminine i-stem (točnosť) is not mis-declined as masculine.
fn inflection_table_g(word: &str, pos_code: &str, gender: Option<crate::model::Gender>) -> String {
    // Decline/conjugate the bare stem for reflexive verbs; append invariant `sę`
    // to every generated verb form below.
    let reflexive = word.ends_with(" sę");
    let bare = word.strip_suffix(" sę").unwrap_or(word);
    match pos_code {
        "noun" | "proper_noun" => noun_table(bare, gender),
        "adj" => adj_table(bare),
        "verb" => verb_table(bare, reflexive),
        _ => "<p class='muted'>Za tų čęst rěči nema tablicy prěgibanja.</p>".to_string(),
    }
}

fn case_rows() -> [(&'static str, IsvCase); 6] {
    [
        ("Nominativ", IsvCase::Nom),
        ("Akuzativ", IsvCase::Acc),
        ("Genitiv", IsvCase::Gen),
        ("Dativ", IsvCase::Dat),
        ("Lokativ", IsvCase::Loc),
        ("Instrumental", IsvCase::Ins),
    ]
}

fn noun_table(word: &str, gender: Option<crate::model::Gender>) -> String {
    // Build the whole paradigm once (issue #20) and index it — the same
    // NounParadigm the API records enumerate, so the table and the API cannot
    // drift. clean_cell reproduces noun_cell_g byte-for-byte. If the build ever
    // panics (inflect-eval asserts 0 panics over the official corpus), fall back
    // to the per-cell getters, which degrade a panicking cell to "—" — keeping
    // the old robustness for generated (non-official) cognate pages.
    let forms = std::panic::catch_unwind(|| crate::forms::noun_paradigm_forms(word, gender)).ok();
    let cell = |case, num| match &forms {
        Some(f) => crate::forms::clean_cell(f.get(case, num)),
        None => crate::forms::noun_cell_g(word, case, num, gender),
    };
    let mut s = String::from("<table class='wikitable inflection-table'><thead><tr><th>Padež</th><th>Jednina</th><th>Množina</th></tr></thead><tbody>");
    for (label, case) in case_rows() {
        let _ = write!(
            s,
            "<tr><th>{}</th><td>{}</td><td>{}</td></tr>",
            label,
            esc(&cell(case, IsvNumber::Singular)),
            esc(&cell(case, IsvNumber::Plural)),
        );
    }
    s.push_str("</tbody></table>");
    s
}

fn adj_table(word: &str) -> String {
    // Build the whole paradigm once (issue #20) and index it — same AdjParadigm
    // as the API records. The four columns are exactly forms::ADJ_COLS. As in
    // noun_table, a panicking build (none in the official corpus) falls back to
    // the per-cell getters so generated cognate pages degrade to "—", not crash.
    let forms = std::panic::catch_unwind(|| ISV::adj_forms(word)).ok();
    let header = "<table class='wikitable inflection-table'><thead><tr><th>Padež</th><th>M. živ.</th><th>M. neživ.</th><th>Ž.</th><th>Sr.</th></tr></thead><tbody>";
    let number_block = |num: IsvNumber| {
        let mut s = String::new();
        for (label, case) in case_rows() {
            let _ = write!(s, "<tr><th>{label}</th>");
            for (_, g, a) in crate::forms::ADJ_COLS {
                let c = match &forms {
                    Some(f) => crate::forms::clean_cell(f.get(case, num, g, a)),
                    None => crate::forms::adj_cell(word, case, num, g, a),
                };
                let _ = write!(s, "<td>{}</td>", esc(&c));
            }
            s.push_str("</tr>");
        }
        s.push_str("</tbody></table>");
        s
    };
    format!(
        "<h3>Jednina</h3>{header}{}<h3>Množina</h3>{header}{}",
        number_block(IsvNumber::Singular),
        number_block(IsvNumber::Plural),
    )
}

fn verb_table(word: &str, reflexive: bool) -> String {
    let Some(cells) = crate::forms::verb_cells(word, reflexive) else {
        return "<p class='muted'>Prěgibanje glagola ne može byti generovano.</p>".to_string();
    };
    let finite_labels = [
        "1. jedn.",
        "2. jedn.",
        "3. jedn.",
        "1. množ.",
        "2. množ.",
        "3. množ.",
    ];
    let compound_labels = [
        "1. jedn.",
        "2. jedn.",
        "3. jedn. m.",
        "3. jedn. ž.",
        "3. jedn. sr.",
        "1. množ.",
        "2. množ.",
        "3. množ.",
    ];
    let imperative_labels = ["2. jedn.", "1. množ.", "2. množ."];
    let cell = |v: &[String], i: usize| -> String {
        v.get(i).cloned().unwrap_or_else(|| "—".to_string())
    };
    let mut s = String::new();
    s.push_str("<h3>Proste i složene vrěmena</h3><table class='wikitable inflection-table verb-wide'><thead><tr><th>Osoba</th><th>Teperešnje</th><th>Nedokončene prošlo</th><th>Budųće</th></tr></thead><tbody>");
    for (i, label) in finite_labels.iter().enumerate() {
        let _ = write!(
            s,
            "<tr><th>{}</th><td>{}</td><td>{}</td><td>{}</td></tr>",
            label,
            esc(&cell(&cells.present, i)),
            esc(&cell(&cells.imperfect, i)),
            esc(&cell(&cells.future, i)),
        );
    }
    s.push_str("</tbody></table>");

    s.push_str("<h3>Perfekt, pluskvamperfekt i kondicional</h3><table class='wikitable inflection-table verb-wide'><thead><tr><th>Osoba</th><th>Perfekt</th><th>Pluskvamperfekt</th><th>Kondicional</th></tr></thead><tbody>");
    for (i, label) in compound_labels.iter().enumerate() {
        let _ = write!(
            s,
            "<tr><th>{}</th><td>{}</td><td>{}</td><td>{}</td></tr>",
            label,
            esc(&cell(&cells.perfect, i)),
            esc(&cell(&cells.pluperfect, i)),
            esc(&cell(&cells.conditional, i)),
        );
    }
    s.push_str("</tbody></table>");

    s.push_str("<h3>Imperativ</h3><table class='wikitable inflection-table'><thead><tr><th>Osoba</th><th>Forma</th></tr></thead><tbody>");
    for (i, label) in imperative_labels.iter().enumerate() {
        let _ = write!(
            s,
            "<tr><th>{}</th><td>{}</td></tr>",
            label,
            esc(&cell(&cells.imperative, i)),
        );
    }
    s.push_str("</tbody></table>");

    let nonfinite_labels = [
        "Infinitiv",
        "Aktivny participij teperešnji",
        "Pasivny participij teperešnji",
        "Aktivny participij prošly",
        "Pasivny participij prošly",
        "Gerundij",
    ];
    s.push_str("<h3>Neosobne formy</h3><table class='wikitable inflection-table'><tbody>");
    for (label, (_, form)) in nonfinite_labels.iter().zip(cells.nonfinite.iter()) {
        let _ = write!(s, "<tr><th>{}</th><td>{}</td></tr>", label, esc(form));
    }
    s.push_str("</tbody></table>");
    s
}

fn verb_form(forms: &[String], idx: usize, reflexive: bool) -> String {
    forms
        .get(idx)
        .map(|s| append_reflexive(s, reflexive))
        .unwrap_or_else(|| "—".to_string())
}

fn append_reflexive(form: &str, reflexive: bool) -> String {
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

/// Inflection validation (Track F / issue #5, `inflect-eval`): run the
/// inflection engine over every single-word official lemma, count blank
/// (panicked) cells, and check the grammar invariants RULE_SPEC §3 states:
/// nom.sg echoes the citation form, masc/neut gen.sg ends -a (the pan-Slavic
/// diagnostic ending), adjective nom.sg agrees (-a fem / -o|-e neut), and the
/// lexicalized suppletive plurals surface. Report: inflection-report.md.
pub fn run_inflect_eval(official_path: &Path, out_dir: &Path) -> Result<()> {
    use crate::model::{Gender, Pos};
    install_quiet_inflection_hook();
    let entries = official::load(official_path)?;
    let fold = |x: &str| crate::orthography::to_standard(&x.trim().to_lowercase());

    let (mut n_words, mut n_cells, mut n_blank) = (0usize, 0usize, 0usize);
    // The dictionary has ~950 duplicated headwords (homograph rows); each
    // unique lemma is inflected and checked once.
    let mut seen_lemmas: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut by_pos: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new(); // (cells, blank)
                                                                              // Invariants: (checked, passed) per rule.
    let mut inv: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
    let check = |inv: &mut BTreeMap<&'static str, (usize, usize)>, rule: &'static str, ok: bool| {
        let e = inv.entry(rule).or_default();
        e.0 += 1;
        e.1 += ok as usize;
    };
    let mut fail_sample: Vec<String> = Vec::new();
    let mut blank_sample: Vec<String> = Vec::new();

    for e in &entries {
        let w = e.isv.trim();
        let bare = w.strip_suffix(" sę").unwrap_or(w);
        if bare.is_empty() || bare.contains(' ') || bare.contains('#') || bare.contains('!') {
            continue;
        }
        if !seen_lemmas.insert(format!("{}|{}", fold(bare), e.pos.code())) {
            continue;
        }
        let plurale_tantum = e.pos_raw.contains("pl.");
        match e.pos {
            Pos::Noun => {
                n_words += 1;
                let mut cells: Vec<(String, &'static str)> = Vec::new();
                for (_, case) in case_rows() {
                    cells.push((
                        crate::forms::noun_cell_g(
                            bare,
                            case,
                            IsvNumber::Singular,
                            e.noun_traits.gender,
                        ),
                        "sg",
                    ));
                    cells.push((
                        crate::forms::noun_cell_g(
                            bare,
                            case,
                            IsvNumber::Plural,
                            e.noun_traits.gender,
                        ),
                        "pl",
                    ));
                }
                // Full-corpus guard (issue #20): the paradigm-struct path that
                // noun_table AND the API records now render from must equal the
                // panic-guarded single-cell getters above, cell for cell, over
                // every lemma — a build-time upgrade of the unit-scale
                // noun_paradigm_roundtrip_matches_cells test.
                let struct_ok = std::panic::catch_unwind(|| {
                    let f = crate::forms::noun_paradigm_forms(bare, e.noun_traits.gender);
                    let mut v = Vec::new();
                    for (_, case) in case_rows() {
                        v.push(crate::forms::clean_cell(f.get(case, IsvNumber::Singular)));
                        v.push(crate::forms::clean_cell(f.get(case, IsvNumber::Plural)));
                    }
                    v
                })
                .ok()
                .is_some_and(|v| {
                    v.len() == cells.len() && v.iter().zip(&cells).all(|(a, (b, _))| a == b)
                });
                check(&mut inv, "noun table struct path = cell getter", struct_ok);
                if !struct_ok && fail_sample.len() < 30 {
                    fail_sample.push(format!("{bare}: noun struct/getter mismatch"));
                }
                let blanks = cells.iter().filter(|(c, _)| c == "—").count();
                n_cells += cells.len();
                n_blank += blanks;
                let bp = by_pos.entry("noun").or_default();
                bp.0 += cells.len();
                bp.1 += blanks;
                if blanks > 0 && blank_sample.len() < 30 {
                    blank_sample.push(format!("{bare} (noun, {blanks} blank)"));
                }
                // Invariant: nom.sg echoes the citation form (a multi-variant
                // cell like "den / denj" passes if any variant echoes it).
                // Pluralia tantum are cited in the plural — no singular echo.
                if !plurale_tantum {
                    let nom = crate::forms::noun_cell_g(
                        bare,
                        IsvCase::Nom,
                        IsvNumber::Singular,
                        e.noun_traits.gender,
                    );
                    let ok = nom.split('/').any(|v| fold(v) == fold(bare));
                    check(&mut inv, "noun nom.sg = citation form", ok);
                    if !ok && fail_sample.len() < 30 {
                        fail_sample.push(format!("{bare}: nom.sg → {nom}"));
                    }
                }
                // Invariant: masc/neut gen.sg ends -a (diagnostic ending).
                // Legitimate exemptions (RULE_SPEC §3): pluralia tantum have no
                // singular; §3.5 indeclinables (loans in -e/-i/-u) don't
                // decline; masculine ā-stems (vojevoda) take the feminine -y;
                // substantivized adjectives decline adjectivally (-ogo/-ego).
                let indeclinable = matches!(fold(bare).chars().last(), Some('e' | 'i' | 'u'));
                let a_stem = fold(bare).ends_with('a');
                let substantivized = matches!(fold(bare).chars().last(), Some('y'));
                if matches!(
                    e.noun_traits.gender,
                    Some(Gender::Masculine | Gender::Neuter)
                ) && !plurale_tantum
                    && !indeclinable
                    && !a_stem
                {
                    let gen = crate::forms::noun_cell_g(
                        bare,
                        IsvCase::Gen,
                        IsvNumber::Singular,
                        e.noun_traits.gender,
                    );
                    // A multi-variant cell (čuda / čudese) passes if any variant
                    // carries the diagnostic -a; substantivized adjectives pass
                    // on the adjectival -ogo/-ego.
                    let ok = gen != "—"
                        && gen.split('/').map(|v| fold(v)).any(|v| {
                            v.ends_with('a')
                                || (substantivized && (v.ends_with("ogo") || v.ends_with("ego")))
                        });
                    check(&mut inv, "masc/neut gen.sg ends -a", ok);
                    if !ok && fail_sample.len() < 30 {
                        fail_sample.push(format!("{bare}: gen.sg → {gen}"));
                    }
                }
            }
            Pos::Adjective => {
                n_words += 1;
                let mut blanks = 0usize;
                let mut cnt = 0usize;
                // Full-corpus guard (issue #20): the AdjParadigm path adj_table
                // AND the API records render from, compared cell-for-cell to the
                // panic-guarded getter over every lemma.
                let struct_forms = std::panic::catch_unwind(|| ISV::adj_forms(bare)).ok();
                let mut adj_struct_ok = struct_forms.is_some();
                for (_, case) in case_rows() {
                    for (g, a) in [
                        (IsvGender::Masculine, IsvAnimacy::Animate),
                        (IsvGender::Masculine, IsvAnimacy::Inanimate),
                        (IsvGender::Feminine, IsvAnimacy::Inanimate),
                        (IsvGender::Neuter, IsvAnimacy::Inanimate),
                    ] {
                        for num in [IsvNumber::Singular, IsvNumber::Plural] {
                            let c = crate::forms::adj_cell(bare, case, num, g, a);
                            if let Some(sf) = &struct_forms {
                                adj_struct_ok &=
                                    crate::forms::clean_cell(sf.get(case, num, g, a)) == c;
                            }
                            cnt += 1;
                            blanks += (c == "—") as usize;
                        }
                    }
                }
                check(
                    &mut inv,
                    "adj table struct path = cell getter",
                    adj_struct_ok,
                );
                if !adj_struct_ok && fail_sample.len() < 30 {
                    fail_sample.push(format!("{bare}: adj struct/getter mismatch"));
                }
                n_cells += cnt;
                n_blank += blanks;
                let bp = by_pos.entry("adj").or_default();
                bp.0 += cnt;
                bp.1 += blanks;
                if blanks > 0 && blank_sample.len() < 30 {
                    blank_sample.push(format!("{bare} (adj, {blanks} blank)"));
                }
                let m = catch(|| {
                    ISV::adj(
                        bare,
                        IsvCase::Nom,
                        IsvNumber::Singular,
                        IsvGender::Masculine,
                        IsvAnimacy::Inanimate,
                    )
                });
                let ok = fold(&m) == fold(bare);
                check(&mut inv, "adj nom.sg.m = citation form", ok);
                if !ok && fail_sample.len() < 30 {
                    fail_sample.push(format!("{bare}: nom.sg.m → {m}"));
                }
                let f = catch(|| {
                    ISV::adj(
                        bare,
                        IsvCase::Nom,
                        IsvNumber::Singular,
                        IsvGender::Feminine,
                        IsvAnimacy::Inanimate,
                    )
                });
                check(
                    &mut inv,
                    "adj nom.sg.f ends -a",
                    f != "—" && fold(&f).ends_with('a'),
                );
                let nt = catch(|| {
                    ISV::adj(
                        bare,
                        IsvCase::Nom,
                        IsvNumber::Singular,
                        IsvGender::Neuter,
                        IsvAnimacy::Inanimate,
                    )
                });
                check(
                    &mut inv,
                    "adj nom.sg.n ends -o/-e",
                    nt != "—" && (fold(&nt).ends_with('o') || fold(&nt).ends_with('e')),
                );
            }
            Pos::Verb => {
                n_words += 1;
                let ok = std::panic::catch_unwind(|| ISV::verb_forms(bare)).is_ok();
                // One "cell" per paradigm: the crate returns the whole set.
                n_cells += 1;
                n_blank += !ok as usize;
                let bp = by_pos.entry("verb (whole paradigm)").or_default();
                bp.0 += 1;
                bp.1 += !ok as usize;
                if !ok && blank_sample.len() < 30 {
                    blank_sample.push(format!("{bare} (verb paradigm panicked)"));
                }
            }
            _ => {}
        }
    }
    // Invariant: the suppletive plurals from RULE_SPEC §3.1 surface — asked of
    // the inflection crate itself (the pinned rev implements them, with the
    // heteroclite byforms); this guards the pin against a regressing rev bump.
    for (base, pl) in [
        ("člověk", "ljudi"),
        ("dětę", "děti"),
        ("oko", "oči"),
        ("uho", "uši"),
    ] {
        let got = crate::forms::noun_cell(base, IsvCase::Nom, IsvNumber::Plural);
        check(
            &mut inv,
            "suppletive plurals (§3.1, from the inflector)",
            got.split('/').any(|v| v.trim() == pl),
        );
    }

    let pct = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f32 / b as f32
        }
    };
    println!(
        "inflect-eval: {n_words} lemmas, {n_cells} cells, {n_blank} blank ({:.2}%)",
        pct(n_blank, n_cells)
    );
    for (rule, (chk, ok)) in &inv {
        println!("  {rule}: {ok}/{chk} ({:.1}%)", pct(*ok, *chk));
    }

    std::fs::create_dir_all(out_dir)?;
    let mut r = String::new();
    writeln!(r, "# Inflection validation (inflect-eval)\n")?;
    writeln!(
        r,
        "**Denominator:** every single-word official lemma (noun/adjective/verb), {n_words} lemmas → {n_cells} paradigm cells generated by the bundled `interslavic` crate. Blank cells are inflector panics recovered by `catch_unwind`. (The export's separate blank-cell count also covers machine-generated reconstruction headwords, whose irregular shapes are where those blanks come from — official lemmas inflect cleanly.) Grammar invariants are the citation-form and ending rules RULE_SPEC §3 states; the failure sample below is the genuine inflector worklist (soft -o loans, §3.5 indeclinables the lexicon must mark).\n"
    )?;
    writeln!(r, "| Measurement | value |")?;
    writeln!(r, "|---|---:|")?;
    writeln!(
        r,
        "| blank cells | **{n_blank}** of {n_cells} ({:.2}%) |",
        pct(n_blank, n_cells)
    )?;
    for (pos, (cells, blank)) in &by_pos {
        writeln!(
            r,
            "| — {pos} | {blank} of {cells} ({:.2}%) |",
            pct(*blank, *cells)
        )?;
    }
    writeln!(r, "\n## Grammar invariants (RULE_SPEC §3)\n")?;
    writeln!(r, "| invariant | pass | rate |")?;
    writeln!(r, "|---|---:|---:|")?;
    for (rule, (chk, ok)) in &inv {
        writeln!(r, "| {rule} | {ok}/{chk} | {:.1}% |", pct(*ok, *chk))?;
    }
    writeln!(r, "\n## Blank-cell sample\n")?;
    for b in &blank_sample {
        writeln!(r, "- {b}")?;
    }
    writeln!(r, "\n## Invariant-failure sample\n")?;
    for f in &fail_sample {
        writeln!(r, "- {f}")?;
    }
    std::fs::write(out_dir.join("inflection-report.md"), r)?;
    println!("Wrote {}", out_dir.join("inflection-report.md").display());
    // The struct-path ≡ cell-getter equivalences are hard guarantees — the site
    // tables AND the forms API render from the struct path, so any divergence
    // from the panic-guarded getters is a bug, and this command (and the CI
    // step running it) fails on it. Every other invariant is the inflector's
    // known worklist (soft -o loans, unmarked indeclinables) and stays
    // report-only.
    for (rule, (chk, ok)) in &inv {
        if rule.contains("struct path = cell getter") {
            anyhow::ensure!(
                ok == chk,
                "inflect-eval: `{rule}` failed on {} of {chk} lemmas — the struct paradigm \
                 path diverged from the cell getters",
                chk - ok
            );
        }
    }
    Ok(())
}

fn catch<F: FnOnce() -> String + std::panic::UnwindSafe>(f: F) -> String {
    std::panic::catch_unwind(f).unwrap_or_else(|_| "—".to_string())
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

fn pos_heading(raw: &str) -> String {
    crate::model::Pos::parse(raw).heading_isv().to_string()
}

fn pos_code_label(raw: &str) -> String {
    if raw.trim().is_empty() {
        "—".to_string()
    } else {
        pos_heading(raw)
    }
}

fn status_pill(s: MatchStatus) -> &'static str {
    match s {
        MatchStatus::OfficialMatch => "<span class='pill ok'>oficialno</span>",
        MatchStatus::DiffersFromOfficial => "<span class='pill warn'>razlika</span>",
        MatchStatus::NoOfficialEntry => "<span class='pill info'>generovano</span>",
    }
}

fn source_class(s: CandidateSource) -> &'static str {
    match s {
        CandidateSource::ProtoSlavicRule => "src-proto",
        CandidateSource::ManualOverride | CandidateSource::OfficialDictionary => "src-official",
        _ => "src-consensus",
    }
}

fn conf_class(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "conf-high",
        Confidence::Medium => "conf-med",
        Confidence::Low => "conf-low",
    }
}

/// The badge explainer under the legacy entry headline: measured operating
/// points read live from the committed calibrator (issue #77), never
/// hand-maintained rates that go stale.
fn calibration_note(c: Confidence, cal: Option<&crate::calibrate::Calibration>) -> String {
    let Some(cal) = cal else {
        return "<p class='muted calib'>Sovmestimaja kalibracija ne dostupna — znak uvěrjenosti jest nekalibrovany modelovy kȯšik, ne věrojętnosť.</p>".to_string();
    };
    let rate = match c {
        Confidence::High => format!(
            "predlog p≥{:.1}: {:.1}% takyh kandidatov odgovara oficialnomu slovniku ({:.1}% pokrytje)",
            crate::calibrate::PROPOSE_T,
            100.0 * cal.propose_pr.0,
            100.0 * cal.propose_pr.1,
        ),
        // Medium = the band p ∈ [0.3, 0.6) EXCLUSIVE of the High band —
        // review_pr's threshold-inclusive precision (all p ≥ 0.3) would
        // overstate this bucket's own match rate (~62% vs ~44%).
        Confidence::Medium => match cal.review_band_precision() {
            Some(band) => format!(
                "pregled p∈[{:.1},{:.1}): ≈{:.0}% takyh kandidatov odgovara oficialnomu slovniku",
                crate::calibrate::REVIEW_T,
                crate::calibrate::PROPOSE_T,
                100.0 * band,
            ),
            None => format!(
                "pregled p∈[{:.1},{:.1})",
                crate::calibrate::REVIEW_T,
                crate::calibrate::PROPOSE_T,
            ),
        },
        Confidence::Low => "pod pragom pregleda (p<0.3)".to_string(),
    };
    format!(
        "<p class='muted calib'>Kalibrovana věrodostojnosť: {rate} (izměrjeno na odloženoj četvrtině; ECE {:.3}).</p>",
        cal.holdout_ece
    )
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

fn compact(v: usize) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

#[derive(Clone)]
struct SiteEntryMeta {
    id: usize,
    title: String,
    gloss: String,
    pos: String,
    status: MatchStatus,
    conf: Confidence,
    score: f32,
    /// Calibrated P(matches an official decision) for generated entries
    /// (issue #77); `None` for official words — both matched (issue #86) and
    /// official-only — which are facts, not predictions, plus raw
    /// attestations and exports without a fitted calibrator.
    prob: Option<f64>,
    /// The calibrated PRIOR the generator assigned before the official match
    /// resolved it; set ONLY for matched entries (issue #86). Display-only:
    /// rendered as a muted transparency line in the provenance section, never
    /// as a badge/probability claim.
    prior: Option<f64>,
    n_langs: usize,
    n_branches: usize,
    borrowed: bool,
    official_only: bool,
    /// A SITE-ONLY, low-evidence raw Wiktionary attestation (issue #34): not
    /// verification-grade, not in the forms API, cognate graph, or wiki indexes.
    raw: bool,
    official_lemma: Option<String>,
    /// Authoritative dictionary row identity for official senses. This keeps
    /// same-title/POS/gloss rows distinct across exports and API validation.
    official_sense_id: Option<String>,
    /// Grammatical aspect for official verbs plus the bidirectional partner
    /// selected by the issue-75 deterministic 1:1 pairing model.
    aspect: Option<String>,
    aspect_partners: Vec<(usize, String)>,
    ancestor: String,
    languages: Vec<String>,
    first: String,
    categories: Vec<Vec<String>>,
}

#[derive(Clone)]
struct LinkEdge {
    source_id: usize,
    source_title: String,
    target_id: usize,
    target_title: String,
    kind: String,
}

struct BuildMeta {
    git: String,
    generated: String,
    total_entries: usize,
    lemma_total: usize,
}

/// `depth` 0 = site root (home), 1 = one subdirectory deep (entry/*.html).
const REPO_URL: &str = "https://github.com/gold-silver-copper/Slovowiki";
const SITE_URL: &str = "https://grift.rs/Slovowiki/";

impl BuildMeta {
    fn current(total_entries: usize, lemma_total: usize) -> Self {
        let git = std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "neznany".to_string());
        // Reproducible build stamp: wall-clock time would perturb build.json,
        // page footers and random-word selection on every identical export.
        let generated = std::process::Command::new("git")
            .args(["show", "-s", "--format=%ct", "HEAD"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| format!("{s} UNIX"))
            .unwrap_or_else(|| "0 UNIX".to_string());
        Self {
            git,
            generated,
            total_entries,
            lemma_total,
        }
    }
}

fn entry_meta(
    id: usize,
    title: &str,
    gloss: &str,
    pos: &str,
    status: MatchStatus,
    conf: Confidence,
    score: f32,
    prob: Option<f64>,
    n_langs: usize,
    n_branches: usize,
    borrowed: bool,
    official_only: bool,
    official_lemma: Option<String>,
    ancestor: String,
    languages: Vec<String>,
    wiki_categories: Vec<Vec<String>>,
) -> SiteEntryMeta {
    let first = first_bucket(title);
    let mut meta = SiteEntryMeta {
        id,
        title: title.to_string(),
        gloss: gloss.to_string(),
        pos: pos.to_string(),
        status,
        conf,
        score,
        prob,
        prior: None,
        n_langs,
        n_branches,
        borrowed,
        official_only,
        raw: false,
        official_lemma,
        official_sense_id: None,
        aspect: None,
        aspect_partners: Vec::new(),
        ancestor,
        languages,
        first,
        categories: Vec::new(),
    };
    meta.categories = entry_categories(&meta, wiki_categories);
    meta
}

fn first_bucket(title: &str) -> String {
    let folded = crate::orthography::ascii_skeleton(title);
    folded
        .chars()
        .find(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase().to_string())
        .unwrap_or_else(|| "#".to_string())
}

fn slug(v: &str) -> String {
    let folded =
        crate::orthography::ascii_skeleton(&crate::orthography::to_standard(&v.to_lowercase()));
    let mut out = String::new();
    let mut dash = false;
    for ch in folded.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "x".to_string()
    } else {
        out
    }
}

fn category_key(path: &[String]) -> String {
    path.iter().map(|s| slug(s)).collect::<Vec<_>>().join("__")
}

fn category_title(path: &[String]) -> String {
    path.join(" » ")
}

fn quality_label(m: &SiteEntryMeta) -> &'static str {
    if m.raw {
        "surova atestacija"
    } else if m.official_only {
        "samo oficialno"
    } else if m.official_lemma.is_some() {
        "oficialne sovpadenje"
    } else if matches!(m.conf, Confidence::High) && m.n_branches >= 3 {
        "vysoko dokazano"
    } else if matches!(m.conf, Confidence::Low) || m.n_branches < 2 {
        "trěbuje prověrky"
    } else {
        "generovano"
    }
}

/// The branch attestation PATTERN of a language set (issue #73c): WHICH of
/// the three branches attest the entry, rendered "V" / "Z" / "J" / "V+Z" /
/// "V+J" / "Z+J" / "V+Z+J" (V = vȯzhod/East, Z = zapad/West, J = jug/South,
/// always in that canonical order). Computed from the actual language SET via
/// `branch_of` — `n_branches` only counts and cannot distinguish V+Z from
/// Z+J. `None` when no code resolves to a branch.
fn branch_pattern(langs: &[String]) -> Option<String> {
    let mut set = std::collections::BTreeSet::new();
    for l in langs {
        if let Some(b) = crate::corpus::branch_of(l) {
            set.insert(match b {
                Branch::East => 0u8,
                Branch::West => 1,
                Branch::South => 2,
            });
        }
    }
    if set.is_empty() {
        return None;
    }
    const LETTERS: [&str; 3] = ["V", "Z", "J"];
    Some(
        set.iter()
            .map(|&i| LETTERS[i as usize])
            .collect::<Vec<_>>()
            .join("+"),
    )
}

fn entry_categories(m: &SiteEntryMeta, wiki_categories: Vec<Vec<String>>) -> Vec<Vec<String>> {
    let mut cats = Vec::new();
    add_category_path(
        &mut cats,
        vec!["Čęsti rěči".to_string(), pos_heading(&m.pos)],
    );
    add_category_path(
        &mut cats,
        vec!["Uvěrjenost".to_string(), m.conf.label().to_string()],
    );
    add_category_path(
        &mut cats,
        vec![
            "Stav".to_string(),
            if m.official_only {
                "oficialne slova bez generacije".to_string()
            } else if m.official_lemma.is_some() {
                "oficialne sovpadenja".to_string()
            } else {
                "generovane kandidaty".to_string()
            },
        ],
    );
    add_category_path(
        &mut cats,
        vec![
            "Etimologija".to_string(),
            if m.borrowed {
                "internacionalizmy i zaimky"
            } else {
                "naslědovane praslovjanske slova"
            }
            .to_string(),
        ],
    );
    add_category_path(
        &mut cats,
        vec![
            "Pokrytje větvi".to_string(),
            format!("{} větvy", m.n_branches),
        ],
    );
    // Attestation-pattern axis (issue #73c): the exact branch COMBINATION,
    // not just the count — "ktore slova sųt tȯlko vȯzhodno-južne?" becomes a
    // browsable category page (7 non-empty combinations).
    if let Some(pattern) = branch_pattern(&m.languages) {
        add_category_path(
            &mut cats,
            vec!["Pokrytje větvi (vzorec)".to_string(), pattern],
        );
    }
    add_category_path(
        &mut cats,
        vec!["Kvaliteta".to_string(), quality_label(m).to_string()],
    );
    // Etymological ancestors are already browsable through `root/*.html` and
    // entry reference links. Do not also make every one-off etymon a category:
    // it creates thousands of repetitive singleton pages.
    for path in wiki_categories {
        add_category_path(&mut cats, path);
    }
    cats
}

fn add_category_path(cats: &mut Vec<Vec<String>>, path: Vec<String>) {
    let path: Vec<String> = path
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if path.is_empty() {
        return;
    }
    if !cats.iter().any(|p| p == &path) {
        cats.push(path);
    }
}

fn wiktionary_category_paths_for_members(
    members: &[crate::dump::LemmaEntry],
    enrich: Option<&crate::enrich::EnrichIndex>,
) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for m in members {
        push_wiki_paths(&mut out, &m.lang, &m.categories, &m.topics, &m.tags);
        if let Some(e) = enrich.and_then(|ix| ix.get(&m.lang, &m.word)) {
            push_wiki_paths(&mut out, &e.lang, &e.categories, &e.topics, &e.tags);
        }
        if out.len() >= 24 {
            break;
        }
    }
    out
}

fn wiktionary_category_paths_for_input(
    input: &MeaningInput,
    enrich: Option<&crate::enrich::EnrichIndex>,
) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for f in input.forms.iter().filter(|f| f.modern && f.primary) {
        if let Some(e) = enrich.and_then(|ix| ix.get(&f.lang_code, &f.norm.original)) {
            push_wiki_paths(&mut out, &e.lang, &e.categories, &e.topics, &e.tags);
        }
        if out.len() >= 16 {
            break;
        }
    }
    out
}

fn push_wiki_paths(
    out: &mut Vec<Vec<String>>,
    lang: &str,
    categories: &[String],
    topics: &[String],
    _tags: &[String],
) {
    for topic in topics.iter().take(8) {
        if let Some(path) = topic_category_path(lang, topic) {
            add_category_path(out, path);
        }
    }
    for cat in categories.iter().take(10) {
        if is_maintenance_wiki_category(cat) {
            continue;
        }
        if let Some(path) = topic_category_path(lang, cat) {
            add_category_path(out, path);
        }
    }
    // Raw Wiktionary tags/categories are preserved in caches but intentionally
    // not promoted to public category pages. Most are maintenance, morphology,
    // pronunciation, or template artifacts and swamp the useful topic tree.
}

fn topic_category_path(lang: &str, label: &str) -> Option<Vec<String>> {
    let l = label
        .to_lowercase()
        .replace(['_', '-', ':'], " ")
        .replace("behaviour", "behavior");
    let topic = if l.contains("weapon") || l.contains("arms") {
        vec!["Tehnologija", "Instrumenty", "Oružje"]
    } else if l.contains("tool") || l.contains("implement") {
        vec!["Tehnologija", "Instrumenty"]
    } else if l.contains("comput") || l.contains("internet") || l.contains("software") {
        vec!["Tehnologija", "Kompjutery"]
    } else if l.contains("technology") || l.contains("engineering") {
        vec!["Tehnologija"]
    } else if l.contains("hunting") || l.contains("hunt ") {
        vec!["Člověk", "Člověčje povědanje", "Člověčja aktivnost", "Lov"]
    } else if l.contains("human activity") || l.contains("activities") {
        vec!["Člověk", "Člověčje povědanje", "Člověčja aktivnost"]
    } else if l.contains("behavior") || l.contains("behaviour") {
        vec!["Člověk", "Člověčje povědanje"]
    } else if l.contains("anatom") || l.contains("body") {
        vec!["Člověk", "Tělo"]
    } else if l.contains("emotion") || l.contains("feeling") {
        vec!["Člověk", "Emocije"]
    } else if l.contains("family") || l.contains("kinship") {
        vec!["Člověk", "Rodina"]
    } else if l.contains("animal") || l.contains("mammal") {
        vec!["Priroda", "Životinje"]
    } else if l.contains("bird") {
        vec!["Priroda", "Životinje", "Ptice"]
    } else if l.contains("fish") {
        vec!["Priroda", "Životinje", "Ryby"]
    } else if l.contains("insect") {
        vec!["Priroda", "Životinje", "Insekty"]
    } else if l.contains("plant") || l.contains("tree") || l.contains("botan") {
        vec!["Priroda", "Rastliny"]
    } else if l.contains("food") || l.contains("cuisine") || l.contains("drink") {
        vec!["Život", "Jedivo i pitje"]
    } else if l.contains("clothing") || l.contains("garment") {
        vec!["Život", "Oděža"]
    } else if l.contains("agricultur") || l.contains("farming") {
        vec!["Život", "Zemjedělstvo"]
    } else if l.contains("transport") || l.contains("vehicle") {
        vec!["Tehnologija", "Transport"]
    } else if l.contains("medicine") || l.contains("disease") || l.contains("medical") {
        vec!["Nauka", "Medicina"]
    } else if l.contains("mathematic") || l.contains("number") {
        vec!["Nauka", "Matematika"]
    } else if l.contains("law") || l.contains("legal") || l.contains("crime") {
        vec!["Družstvo", "Pravo"]
    } else if l.contains("military") || l.contains("war") || l.contains("army") {
        vec!["Družstvo", "Vojska"]
    } else if l.contains("politic") || l.contains("government") {
        vec!["Družstvo", "Politika"]
    } else if l.contains("religion") || l.contains("mytholog") {
        vec!["Kultura", "Religija"]
    } else if l.contains("music") {
        vec!["Kultura", "Muzyka"]
    } else if l.contains("literature") || l.contains("poetry") {
        vec!["Kultura", "Literatura"]
    } else if l.contains("sport") || l.contains("game") {
        vec!["Kultura", "Sport i igry"]
    } else if l.contains("time") || l.contains("calendar") {
        vec!["Abstraktne", "Čas"]
    } else {
        return None;
    };
    Some(wiki_topic_root(lang, topic))
}

fn wiki_topic_root(lang: &str, topic: Vec<&str>) -> Vec<String> {
    let mut path = vec![
        "Fundamentalne".to_string(),
        "Vsi języky".to_string(),
        crate::lang::lang_name(lang).to_string(),
        "Vse temy".to_string(),
    ];
    path.extend(topic.into_iter().map(|s| s.to_string()));
    path
}

fn raw_wiki_category_path(lang: &str, label: &str) -> Vec<String> {
    vec![
        "Fundamentalne".to_string(),
        "Vsi języky".to_string(),
        crate::lang::lang_name(lang).to_string(),
        "Kategorije Wiktionary".to_string(),
        translate_wiki_label(label),
    ]
}

fn is_maintenance_wiki_category(label: &str) -> bool {
    let l = label.to_lowercase();
    [
        "monitoring:",
        "pages with",
        "entries with",
        "terms with ipa",
        "terms with redundant",
        "terms needing",
        "requests for",
        "citation",
        "cleanup",
        "maintenance",
        "templates",
        "rhymes",
        "pronunciation",
    ]
    .iter()
    .any(|needle| l.contains(needle))
}

fn translate_wiki_label(label: &str) -> String {
    let mut s = label
        .trim()
        .trim_start_matches("Category:")
        .replace('_', " ")
        .replace("English terms related to ", "")
        .replace("Russian terms related to ", "")
        .replace("Polish terms related to ", "")
        .replace("Czech terms related to ", "")
        .replace("terms related to ", "")
        .replace("All topics", "Vse temy")
        .replace("All languages", "Vsi języky")
        .replace("Technology", "Tehnologija")
        .replace("Tools", "Instrumenty")
        .replace("Weapons", "Oružje")
        .replace("Human behaviour", "Člověčje povědanje")
        .replace("Human behavior", "Člověčje povědanje")
        .replace("Human activity", "Člověčja aktivnost")
        .replace("Hunting", "Lov");
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&s, 70)
}

fn compact_entry_categories(metas: &mut [SiteEntryMeta]) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for m in metas.iter() {
        for path in &m.categories {
            *counts.entry(category_key(path)).or_insert(0) += 1;
        }
    }
    for m in metas.iter_mut() {
        m.categories.retain(|path| {
            let Some(root) = path.first().map(String::as_str) else {
                return false;
            };
            if root == "Fundamentalne" {
                counts.get(&category_key(path)).copied().unwrap_or(0) >= 3
            } else {
                true
            }
        });
    }
}

fn issue_url(m: &SiteEntryMeta) -> String {
    let title = format!("Problem so zapisom: {}", m.title);
    let body = format!(
        "Zapis: {}\nStrana: entry/{}.html\nČęst rěči: {}\nSmysl: {}\n\nOpiši popravku ili dokaz tut:",
        m.title, m.id, pos_code_label(&m.pos), m.gloss
    );
    format!(
        "{REPO_URL}/issues/new?title={}&body={}",
        query_escape(&title),
        query_escape(&body)
    )
}

fn query_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn page(title: &str, body: &str, depth: usize) -> String {
    let up = if depth == 0 { "" } else { "../" };
    format!(
        "<!doctype html><html lang='art'><head>\
         <meta charset='utf-8'><meta name='viewport' content='width=device-width, initial-scale=1'>\
         <title>{title}</title><link rel='stylesheet' href='{up}wiktionary.css'>\
         <script>var SITE_BASE='{up}';</script></head><body>\
         <header class='site-header'>\
           <a class='brand' href='{up}index.html'>Medžuslovjansky <span class='brand-sub'>slovnik</span></a>\
           <form class='hsearch' onsubmit='return goSearch(event)' autocomplete='off' role='search'>\
             <input id='q' type='search' placeholder='Iskaj slovo ili anglijski smysl…  (Enter za vse rezultaty)' spellcheck='false'>\
             <button class='hsearch-go' type='submit' title='Iskaj'>→</button>\
             <div id='results' class='dropdown'></div>\
           </form>\
           <nav class='nav'><a href='{up}index.html'>Slovnik</a><a href='{up}special.html'>Speciaľne</a><a href='{up}all-pages.html'>Vse strany</a><a href='{up}categories.html'>Kategorije</a><a href='{up}site-stats.html'>Statistiky</a><a href='{up}search.html'>Iskanje</a><a href='{up}about.html'>O metodě</a><a href='{REPO_URL}'>Kod</a></nav>\
         </header>\
         <div class='layout'>\
           <aside class='sidebar'>\
             <div class='side-box toc-box'><div class='side-h'>Na toj straně</div><nav id='toc-nav' class='toc'></nav></div>\
             <div class='side-box'><div class='side-h'>Instrumenty</div>\
               <a class='side-link' href='{up}special.html'>★ Speciaľne strany</a>\
               <a class='side-link' href='{up}all-pages.html'>📖 Vse strany</a>\
               <a class='side-link' href='{up}categories.html'>🏷️ Kategorije</a>\
               <a class='side-link' href='{up}indices.html'>🔤 Indeksy</a>\
               <a class='side-link' href='{up}portals.html'>🌐 Języčne portaly</a>\
               <a class='side-link' href='{up}graph.html'>🕸️ Semantičny graf</a>\
               <a class='side-link' href='{up}site-stats.html'>📈 Statistiky sajta</a>\
               <a class='side-link' href='{up}borrowings.html'>↗ Pozajęta slova</a>\
               <a class='side-link' href='{up}needs-review.html'>⚑ Trěbuje prověrky</a>\
               <button id='randbtn' class='side-link' type='button'>🎲 Slučajno/izbrano slovo</button>\
               <a class='side-link' href='{up}search.html'>🔎 Råzširjeno iskanje</a>\
               <a class='side-link' href='{up}contribute.html'>✎ Prinos</a>\
               <a class='side-link' href='{up}about.html'>ⓘ O metodě</a>\
               <a class='side-link' href='{up}metrics.html'>📊 Statistiky točnosti</a>\
             </div>\
             <div class='side-box' id='spotbox' style='display:none'><div class='side-h'>Slučajno slovo</div><div id='spotlight'></div></div>\
           </aside>\
           <main>{body}</main>\
         </div>\
         <footer class='site-footer'>Mašinno generovane rekonstrukcije — ne oficialny standard bez prověrky. Dokazy: interslavic-dictionary.com, Wiktionary (CC BY-SA). <a href='{REPO_URL}'>Izvorny kod</a>.</footer>\
         <script>{search_js}</script>\
         <script>{TOC_JS}</script>\
         </body></html>",
        title = esc(title),
        search_js = search_js()
    )
}

fn homograph_groups(
    metas: &[SiteEntryMeta],
) -> std::collections::BTreeMap<String, Vec<SiteEntryMeta>> {
    let mut groups: std::collections::BTreeMap<String, Vec<SiteEntryMeta>> =
        std::collections::BTreeMap::new();
    for m in metas {
        groups
            .entry(crate::orthography::to_standard(&m.title.to_lowercase()))
            .or_default()
            .push(m.clone());
    }
    groups.retain(|_, v| v.len() > 1);
    groups
}

fn load_curation_notes() -> std::collections::HashMap<String, String> {
    let path = Path::new("data/curation-notes.json");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return std::collections::HashMap::new();
    };
    serde_json::from_str::<std::collections::HashMap<String, String>>(&raw).unwrap_or_default()
}

fn add_edge(
    edges: &mut Vec<LinkEdge>,
    meta_by_id: &std::collections::HashMap<usize, SiteEntryMeta>,
    source_id: usize,
    target_id: usize,
    kind: &str,
) {
    if source_id == target_id {
        return;
    }
    let (Some(src), Some(dst)) = (meta_by_id.get(&source_id), meta_by_id.get(&target_id)) else {
        return;
    };
    if edges
        .iter()
        .any(|e| e.source_id == source_id && e.target_id == target_id && e.kind == kind)
    {
        return;
    }
    edges.push(LinkEdge {
        source_id,
        source_title: src.title.clone(),
        target_id,
        target_title: dst.title.clone(),
        kind: kind.to_string(),
    });
}

fn build_edges<T: FamilyEntry>(
    prepared: &[T],
    families: &std::collections::BTreeMap<String, Vec<usize>>,
    thes: &crate::thesaurus::Thesaurus,
    isv_to_id: &HeadwordIndex,
    enrich: Option<&crate::enrich::EnrichIndex>,
    xref: Option<&crate::enrich::Xref>,
    meta_by_id: &std::collections::HashMap<usize, SiteEntryMeta>,
) -> Vec<LinkEdge> {
    let mut edges = Vec::new();
    for members in families.values() {
        if members.len() < 2 || members.len() > 15 {
            continue;
        }
        for &a in members {
            for &b in members {
                if a != b {
                    add_edge(
                        &mut edges,
                        meta_by_id,
                        prepared[a].id(),
                        prepared[b].id(),
                        "rodina",
                    );
                }
            }
        }
    }
    for m in meta_by_id.values() {
        let Some(isv) = &m.official_lemma else {
            continue;
        };
        for s in thes.get(isv) {
            if let Some(target) = isv_to_id.resolve(s) {
                add_edge(&mut edges, meta_by_id, m.id, target, "sinonim");
            }
        }
    }
    if let (Some(enrich), Some(xref)) = (enrich, xref) {
        for p in prepared {
            if !meta_by_id.contains_key(&p.id()) {
                continue;
            }
            for member in &p.set().members {
                let Some(e) = enrich.get(&member.lang, &member.word) else {
                    continue;
                };
                for (kind, words) in [
                    ("srodno", &e.related),
                    ("sinonim", &e.synonyms),
                    ("antonim", &e.antonyms),
                ] {
                    for w in words.iter().take(40) {
                        if let Some(target) = xref.get(&member.lang, w) {
                            add_edge(&mut edges, meta_by_id, p.id(), target, kind);
                        }
                    }
                }
            }
        }
    }
    edges.sort_by(|a, b| {
        a.source_id
            .cmp(&b.source_id)
            .then_with(|| a.target_id.cmp(&b.target_id))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.target_title.cmp(&b.target_title))
    });
    edges
}

fn backlinks_by_target(edges: &[LinkEdge]) -> std::collections::BTreeMap<usize, Vec<LinkEdge>> {
    let mut map: std::collections::BTreeMap<usize, Vec<LinkEdge>> =
        std::collections::BTreeMap::new();
    for e in edges {
        map.entry(e.target_id).or_default().push(e.clone());
    }
    map
}

fn render_word_table(rows: &[SiteEntryMeta], up: &str) -> String {
    let shown = rows.len().min(1200);
    let mut s = String::from("<table class='wikitable word-index'><thead><tr><th>Slovo</th><th>Čęst</th><th>Smysl</th><th>Kvaliteta</th><th>Dokaz</th></tr></thead><tbody>");
    for m in rows.iter().take(1200) {
        let _ = write!(
            s,
            "<tr><td><a href='{up}entry/{}.html'><b>{}</b></a></td><td>{}</td><td>{}</td><td><span class='badge'>{}</span></td><td>{} jęz. / {} vět.</td></tr>",
            m.id,
            esc(&m.title),
            esc(&pos_code_label(&m.pos)),
            esc(&truncate(&m.gloss, 72)),
            esc(quality_label(m)),
            m.n_langs,
            m.n_branches,
        );
    }
    s.push_str("</tbody></table>");
    if rows.len() > shown {
        let _ = write!(
            s,
            "<p class='muted'>Pokazano prvih {} od {} zapisov; koristi iskanje za polny spis.</p>",
            compact(shown),
            compact(rows.len())
        );
    }
    s
}

fn count_by<F>(rows: &[SiteEntryMeta], mut f: F) -> std::collections::BTreeMap<String, usize>
where
    F: FnMut(&SiteEntryMeta) -> String,
{
    let mut map = std::collections::BTreeMap::new();
    for m in rows {
        *map.entry(f(m)).or_insert(0) += 1;
    }
    map
}

fn counts_table(title: &str, counts: &std::collections::BTreeMap<String, usize>) -> String {
    let mut pairs: Vec<(&String, &usize)> = counts.iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let mut body = String::new();
    for (k, v) in pairs.into_iter().take(24) {
        let _ = write!(body, "<tr><th>{}</th><td>{}</td></tr>", esc(k), compact(*v));
    }
    format!("<div class='stat-box'><h3>{}</h3><table class='wikitable compact-table'><tbody>{body}</tbody></table></div>", esc(title))
}

fn index_summary(rows: &[SiteEntryMeta]) -> String {
    let official = rows.iter().filter(|m| m.official_lemma.is_some()).count();
    let generated = rows.len().saturating_sub(official);
    let high = rows
        .iter()
        .filter(|m| matches!(m.conf, Confidence::High))
        .count();
    let borrowed = rows.iter().filter(|m| m.borrowed).count();
    format!(
        "<table class='wikitable compact-table index-summary'><tbody>\
         <tr><th>Zapisov</th><td>{}</td><th>Oficialne</th><td>{}</td></tr>\
         <tr><th>Samo generovane</th><td>{}</td><th>Vysoka uvěrjenost</th><td>{}</td></tr>\
         <tr><th>Pozajęta slova / internacionalizmy</th><td>{}</td><th>Srědnje językov</th><td>{:.1}</td></tr>\
         </tbody></table>",
        compact(rows.len()),
        compact(official),
        compact(generated),
        compact(high),
        compact(borrowed),
        if rows.is_empty() { 0.0 } else { rows.iter().map(|m| m.n_langs).sum::<usize>() as f32 / rows.len() as f32 }
    )
}

fn simple_index_page(title: &str, intro: &str, rows: &[SiteEntryMeta], depth: usize) -> String {
    let up = if depth == 0 { "" } else { "../" };
    let pos = count_by(rows, |m| pos_code_label(&m.pos));
    let conf = count_by(rows, |m| m.conf.label().to_string());
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>{}</h1><p>{}</p>{}<div class='stat-grid wiki-stats'>{}{}</div>{}</article>",
        esc(title),
        esc(intro),
        index_summary(rows),
        counts_table("Čęsti rěči", &pos),
        counts_table("Uvěrjenost", &conf),
        render_word_table(rows, up)
    );
    page(title, &body, depth)
}

fn site_stats_page(
    metas: &[SiteEntryMeta],
    edges: &[LinkEdge],
    homographs: &std::collections::BTreeMap<String, Vec<SiteEntryMeta>>,
    build: &BuildMeta,
) -> String {
    let by_pos = count_by(metas, |m| pos_code_label(&m.pos));
    let by_conf = count_by(metas, |m| m.conf.label().to_string());
    let by_quality = count_by(metas, |m| quality_label(m).to_string());
    let by_branch = count_by(metas, |m| format!("{} větvy", m.n_branches));
    let mut by_lang: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for m in metas {
        for lang in &m.languages {
            *by_lang
                .entry(crate::lang::lang_name(lang).to_string())
                .or_insert(0) += 1;
        }
    }
    let official = metas.iter().filter(|m| m.official_lemma.is_some()).count();
    let borrowed = metas.iter().filter(|m| m.borrowed).count();
    let avg_lang = if metas.is_empty() {
        0.0
    } else {
        metas.iter().map(|m| m.n_langs).sum::<usize>() as f32 / metas.len() as f32
    };
    let body = format!(
        "<article class='entry stats-page'><h1 class='firstHeading'>Statistiky sajta</h1>\
         <p class='lede'>Ta strana je statičny ekvivalent wiki-strany <i>Speciaľno:Statistiky</i>: ne měri samo točnosť, ale pokazyvaje kako veliky i kaky je slovnikovy korpus.</p>\
         <table class='wikitable compact-table'>\
           <tr><th>Stran zapisov</th><td>{}</td><th>Oficialno povezane</th><td>{}</td></tr>\
           <tr><th>Pozajęta slova / internacionalizmy</th><td>{}</td><th>Homografne grupy</th><td>{}</td></tr>\
           <tr><th>Semantične vęzi</th><td>{}</td><th>Srědnje językov na zapis</th><td>{:.1}</td></tr>\
           <tr><th>Generacija</th><td>{}</td><th>Git</th><td><code>{}</code></td></tr>\
         </table>\
         <div class='stat-grid wiki-stats'>{}{}{}{}{} </div>\
         <p>Za točnost generatora ględaj <a href='metrics.html'>Statistiky točnosti</a>; za metodologiju <a href='about.html'>O metodě</a>.</p>\
         </article>",
        compact(metas.len()),
        compact(official),
        compact(borrowed),
        compact(homographs.len()),
        compact(edges.len()),
        avg_lang,
        esc(&build.generated),
        esc(&build.git),
        counts_table("Čęsti rěči", &by_pos),
        counts_table("Uvěrjenost", &by_conf),
        counts_table("Kvaliteta", &by_quality),
        counts_table("Pokrytje větvi", &by_branch),
        counts_table("Języčne portaly", &by_lang),
    );
    page("Statistiky sajta", &body, 0)
}

fn ancestor_slug(m: &SiteEntryMeta) -> Option<String> {
    if m.ancestor.trim().is_empty() || m.borrowed {
        None
    } else {
        Some(slug(m.ancestor.trim_start_matches('*')))
    }
}

fn borrowing_source(m: &SiteEntryMeta) -> String {
    let src = m.ancestor.split_whitespace().next().unwrap_or("");
    match src {
        "la" | "ML." | "LL." | "la-med" | "la-lat" => "latinsky".to_string(),
        "grc" | "el" => "grečsky".to_string(),
        "de" | "gmh" => "němečsky".to_string(),
        "fr" | "frm" | "fro" => "francuzsky".to_string(),
        "en" => "anglijsky".to_string(),
        "it" => "italijsky".to_string(),
        "tr" | "ota" => "turecky".to_string(),
        "ar" => "arabsky".to_string(),
        "" => "neznany".to_string(),
        other => other.to_string(),
    }
}

/// Curation-worklist membership. Official dictionary words — matched AND
/// official-only — are facts and can NEVER need review (issue #86: the old OR
/// chain pulled 2,020 official-matched words in through its confidence /
/// probability clauses). For everything else the old predicate's first clause
/// (`official_lemma.is_none()`) already held, so membership is exactly "not an
/// official word": every machine-only reconstruction remains curation work.
fn needs_review(m: &SiteEntryMeta) -> bool {
    m.official_lemma.is_none() && !m.official_only
}

fn language_portal_page(lang: &str, rows: &[SiteEntryMeta], all: &[SiteEntryMeta]) -> String {
    let unique: Vec<SiteEntryMeta> = rows
        .iter()
        .filter(|m| m.languages.len() == 1)
        .cloned()
        .collect();
    let pan_slavic: Vec<SiteEntryMeta> =
        rows.iter().filter(|m| m.n_branches >= 3).cloned().collect();
    let mut strongest = rows.to_vec();
    strongest.sort_by(|a, b| b.score.total_cmp(&a.score));
    let name = crate::lang::lang_name(lang);
    let intro = format!(
        "Portal za {}: strany zapisov, v ktoryh toj język davaje srodny dokaz. Unikatne slova pokazyvajųt korenje vidno samo v tom języku v našem korpusu; vseslovjanske slova imajųt dokaz iz vsih trěh větvi.",
        name
    );
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>Portal: {}</h1><p>{}</p>{}\
         <h2 id='silne'>Najsilnějše dokazani zapisy</h2>{}\
         <h2 id='vseslovjanske'>Slova s dokazom iz vsih trěh větvi</h2>{}\
         <h2 id='unikatne'>Unikatne v tom portalu</h2>{}\
         <h2 id='vse'>Vse zapisy v portalu</h2>{}</article>",
        esc(&name),
        esc(&intro),
        index_summary(rows),
        render_word_table(&strongest, "../"),
        render_word_table(&pan_slavic, "../"),
        render_word_table(&unique, "../"),
        render_word_table(rows, "../"),
    );
    let _ = all;
    page(&format!("Portal: {name}"), &body, 1)
}

/// `proto_link` is a prebuilt paragraph linking the matching proto-lemma
/// reflex page (issue #73b), or empty when the proto cache has no entry for
/// this root's slug.
fn root_page(root: &str, rows: &[SiteEntryMeta], proto_link: &str) -> String {
    let by_pos = count_by(rows, |m| pos_code_label(&m.pos));
    let by_lang = {
        let mut map = std::collections::BTreeMap::new();
        for m in rows {
            for l in &m.languages {
                *map.entry(crate::lang::lang_name(l).to_string())
                    .or_insert(0) += 1;
            }
        }
        map
    };
    let official: Vec<SiteEntryMeta> = rows
        .iter()
        .filter(|m| m.official_lemma.is_some())
        .cloned()
        .collect();
    let mut derived = rows.to_vec();
    derived.sort_by_key(|m| (m.pos.clone(), crate::orthography::ascii_skeleton(&m.title)));
    let title = format!("Rekonstrukcija: *{root}");
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>{}</h1>\
         <p class='lede'>Statična korenj-strana za praslovjansky korenj. Ona sobira vse medžuslovjanske strany zapisov, ktore v korpusu pokazyvajųt na toj prědȯk ili blizku derivaciju.</p>\
         {proto_link}\
         {}<div class='stat-grid wiki-stats'>{}{}</div>\
         <h2 id='official'>Oficialne sovpadenja pod tym korenjem</h2>{}\
         <h2 id='tree'>Derivacijsko drevo / rodina</h2>{}\
         <h2 id='desc'>Języčne potomky v sajtu</h2>{}</article>",
        esc(&title),
        index_summary(rows),
        counts_table("Čęsti rěči", &by_pos),
        counts_table("Języky", &by_lang),
        render_word_table(&official, "../"),
        render_word_table(&derived, "../"),
        counts_table("Potomky po językah", &by_lang),
    );
    page(&title, &body, 1)
}

fn borrowing_portal_page(rows: &[SiteEntryMeta]) -> String {
    let mut by_src = count_by(rows, borrowing_source);
    let mut strongest = rows.to_vec();
    strongest.sort_by(|a, b| {
        b.n_langs
            .cmp(&a.n_langs)
            .then_with(|| b.score.total_cmp(&a.score))
    });
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>Portal: Pozajęta slova i internacionalizmy</h1>\
         <p class='lede'>Slova grupovane po neslovjanskom etimonu ili internacionalnom fonemičnom skeletu.</p>\
         {}<div class='stat-grid wiki-stats'>{}</div><h2 id='najsilne'>Najširše dokazane zaimky</h2>{}<h2 id='vse'>Vse zaimky</h2>{}</article>",
        index_summary(rows),
        counts_table("Izvorni języky", &by_src),
        render_word_table(&strongest, ""),
        render_word_table(rows, ""),
    );
    by_src.clear();
    page("Portal: Pozajęta slova i internacionalizmy", &body, 0)
}

fn needs_review_page(rows: &[SiteEntryMeta]) -> String {
    let review: Vec<SiteEntryMeta> = rows.iter().filter(|m| needs_review(m)).cloned().collect();
    let low: Vec<SiteEntryMeta> = review
        .iter()
        .filter(|m| matches!(m.conf, Confidence::Low))
        .cloned()
        .collect();
    let one_branch: Vec<SiteEntryMeta> = review
        .iter()
        .filter(|m| m.n_branches < 2)
        .cloned()
        .collect();
    let generated: Vec<SiteEntryMeta> = review
        .iter()
        .filter(|m| m.official_lemma.is_none())
        .cloned()
        .collect();
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>Speciaľno:TrěbujePrověrky</h1>\
         <p class='lede'>Kuratorska robota: strany zapisov s nizkoj uvěrjenostju, malym pokrytjem ili bez oficialnogo sovpadenja.</p>\
         {}<h2 id='nizka'>Nizka uvěrjenost</h2>{}<h2 id='jedna-vetv'>Samo jedna větv</h2>{}<h2 id='neoficialne'>Samo generovane</h2>{}</article>",
        index_summary(&review),
        render_word_table(&low, ""),
        render_word_table(&one_branch, ""),
        render_word_table(&generated, ""),
    );
    page("Speciaľno:TrěbujePrověrky", &body, 0)
}

fn write_borrowing_subpages(out_dir: &Path, rows: &[SiteEntryMeta]) -> Result<()> {
    let mut by_src: std::collections::BTreeMap<String, Vec<SiteEntryMeta>> =
        std::collections::BTreeMap::new();
    for m in rows {
        by_src
            .entry(borrowing_source(m))
            .or_default()
            .push(m.clone());
    }
    for (src, items) in &mut by_src {
        items.sort_by_key(|m| crate::orthography::ascii_skeleton(&m.title));
        std::fs::write(
            out_dir
                .join("borrowings")
                .join(format!("{}.html", slug(src))),
            simple_index_page(
                &format!("Pozajęta slova: {src}"),
                "Pozajęta slova grupovana po izvornom języku/etimonu.",
                items,
                1,
            ),
        )?;
    }
    Ok(())
}

fn write_needs_review_subpages(out_dir: &Path, rows: &[SiteEntryMeta]) -> Result<()> {
    // Same membership rule as the hub page: official words (matched or
    // official-only) can never appear on a review worklist (issue #86) — the
    // per-axis filters below only slice the review set.
    let rows: Vec<SiteEntryMeta> = rows.iter().filter(|m| needs_review(m)).cloned().collect();
    let groups: [(&str, &str, Vec<SiteEntryMeta>); 4] = [
        (
            "nizka-uverjenost",
            "Nizka uvěrjenost",
            rows.iter()
                .filter(|m| matches!(m.conf, Confidence::Low))
                .cloned()
                .collect(),
        ),
        (
            "jedna-vetv",
            "Samo jedna větv",
            rows.iter().filter(|m| m.n_branches < 2).cloned().collect(),
        ),
        (
            "samo-generovane",
            "Samo generovane",
            rows.iter()
                .filter(|m| m.official_lemma.is_none())
                .cloned()
                .collect(),
        ),
        (
            "nizka-ocena",
            "Nizka ocěna (kalibrovana p < 0.3)",
            rows.iter()
                .filter(|m| m.prob.is_some_and(|p| p < crate::calibrate::REVIEW_T))
                .cloned()
                .collect(),
        ),
    ];
    for (file, title, mut items) in groups {
        items.sort_by_key(|m| crate::orthography::ascii_skeleton(&m.title));
        std::fs::write(
            out_dir.join("needs-review").join(format!("{file}.html")),
            simple_index_page(
                &format!("Trěbuje prověrky: {title}"),
                "Podspis kuratorskogo spiska.",
                &items,
                1,
            ),
        )?;
    }
    Ok(())
}

fn suffix_bucket(title: &str, pos: &str) -> String {
    let folded = crate::orthography::to_standard(&title.to_lowercase());
    if pos == "verb" {
        if folded.ends_with("ti") {
            "glagoly na -ti".to_string()
        } else {
            "druge glagoly".to_string()
        }
    } else if pos == "adj" {
        folded
            .chars()
            .last()
            .map(|c| format!("pridavniki na -{c}"))
            .unwrap_or_else(|| "pridavniki".to_string())
    } else {
        let suffix: String = folded
            .chars()
            .rev()
            .take(2)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if suffix.is_empty() {
            "druga zakončenja".to_string()
        } else {
            format!("zakončenje -{suffix}")
        }
    }
}

fn suffix_index_page(rows: &[SiteEntryMeta]) -> String {
    let mut groups: std::collections::BTreeMap<String, Vec<SiteEntryMeta>> =
        std::collections::BTreeMap::new();
    for m in rows {
        groups
            .entry(suffix_bucket(&m.title, &m.pos))
            .or_default()
            .push(m.clone());
    }
    let mut body = String::new();
    for (name, items) in groups.iter().filter(|(_, v)| v.len() >= 20).take(80) {
        let _ = write!(
            body,
            "<li><b>{}</b> <span class='muted'>({})</span></li>",
            esc(name),
            compact(items.len())
        );
    }
    page("Indeks po zakončenjah", &format!("<article class='entry'><h1 class='firstHeading'>Indeks po zakončenjah</h1><p class='lede'>Gruby indeks po zakončenjah: koristen za prěgled glagolov, pridavnikov i imennikov po obliku.</p><ul class='compact-list'>{body}</ul></article>"), 0)
}

fn inflection_issue(m: &SiteEntryMeta) -> bool {
    matches!(m.pos.as_str(), "noun" | "proper_noun" | "adj" | "verb")
        && inflection_table(&m.title, &m.pos).contains('—')
}

fn inflection_issues_page(rows: &[SiteEntryMeta]) -> String {
    let mut issues: Vec<SiteEntryMeta> = rows
        .iter()
        .filter(|m| inflection_issue(m))
        .cloned()
        .collect();
    issues.sort_by_key(|m| crate::orthography::ascii_skeleton(&m.title));
    page("Speciaľno:ProblemyPrěgibanja", &format!("<article class='entry'><h1 class='firstHeading'>Speciaľno:ProblemyPrěgibanja</h1><p class='lede'>Stran zapisovy, gdě prěgibanje je nepolno ili vrnulo —. To je praktičny spis za popravki v interslavic-rs.</p>{}</article>", render_word_table(&issues, "")), 0)
}

fn featured_page(rows: &[SiteEntryMeta], build: &BuildMeta) -> String {
    let mut featured: Vec<SiteEntryMeta> = rows
        .iter()
        .filter(|m| matches!(m.conf, Confidence::High) || m.official_lemma.is_some())
        .cloned()
        .collect();
    featured.sort_by(|a, b| {
        b.n_branches
            .cmp(&a.n_branches)
            .then_with(|| b.n_langs.cmp(&a.n_langs))
            .then_with(|| b.score.total_cmp(&a.score))
    });
    let seed = build.generated.bytes().map(|b| b as usize).sum::<usize>();
    let daily = featured.get(seed % featured.len().max(1));
    let daily_html = daily
        .map(|m| {
            format!(
                "<div class='notice'><b>Izbrano:</b> <a href='entry/{}.html'>{}</a> — {}</div>",
                m.id,
                esc(&m.title),
                esc(&m.gloss)
            )
        })
        .unwrap_or_default();
    page("Speciaľno:Izbrano", &format!("<article class='entry'><h1 class='firstHeading'>Speciaľno:Izbrano</h1><p class='lede'>Determinističny izbor dobro dokazanyh stran zapisov za tu generaciju sajta.</p>{daily_html}{} </article>", render_word_table(&featured, "")), 0)
}

fn random_page() -> String {
    let body = r#"<article class='entry'><h1 class='firstHeading'>Speciaľno:Slučajno</h1><p>Ta statična strana koristi lokalny <code>search/spotlight.json</code> i izbere slučajnu stranu zapisa bez servera.</p><p id='random-target' class='notice'>Nakladajě sę…</p><script>document.addEventListener('DOMContentLoaded',function(){spotRows().then(function(idx){if(!idx.length)return;var eh=function(s){return String(s==null?'':s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');};var e=idx[Math.floor(Math.random()*idx.length)];var a='entry/'+e[0]+'.html';document.getElementById('random-target').innerHTML='<a href="'+a+'">'+eh(e[1])+'</a> — '+eh(e[2])+'<br><a href="'+a+'">Idi</a>';});});</script></article>"#;
    page("Speciaľno:Slučajno", body, 0)
}

fn special_pages_hub() -> String {
    let body = "<article class='entry'><h1 class='firstHeading'>Speciaľne strany</h1>\
      <p class='lede'>Statične wiki-podobne služebne strany za prěgledanje slovnika.</p>\
      <ul class='compact-list'>\
        <li><a href='all-pages.html'>Speciaľno:VseStrany</a></li>\
        <li><a href='categories.html'>Speciaľno:Kategorije</a></li>\
        <li><a href='site-stats.html'>Speciaľno:Statistiky</a></li>\
        <li><a href='needs-review.html'>Speciaľno:TrěbujePrověrky</a></li>\
        <li><a href='inflection-issues.html'>Speciaľno:ProblemyPrěgibanja</a></li>\
        <li><a href='random.html'>Speciaľno:Slučajno</a></li>\
        <li><a href='featured.html'>Speciaľno:Izbrano</a></li>\
        <li><a href='borrowings.html'>Portal:PozajętaSlova</a></li>\
        <li><a href='rules.html'>Indeks pravil (zvukove zakony)</a></li>\
        <li><a href='proto-index.html'>Praslovjanske lemmy (refleksy)</a></li>\
        <li><a href='derivations.html'>Odvodženja po sufiksah</a></li>\
        <li><a href='suffix-index.html'>Indeks po zakončenjah</a></li>\
        <li><a href='datasets.html'>Fajly za dostavanje</a></li>\
        <li><a href='proposals.html'>Predloženja novyh slov</a></li>\
        <li><a href='forms.html'>Iskanje form</a></li>\
        <li><a href='text-check.html'>Prověrka teksta</a></li>\
        <li><a href='portals.html'>Języčne portaly</a></li>\
        <li><a href='indices.html'>Abecedne indeksy</a></li>\
        <li><a href='graph.html'>Semantičny graf</a></li>\
      </ul></article>";
    page("Speciaľne strany", body, 0)
}

fn talk_page(m: &SiteEntryMeta, note: Option<&String>, incoming: &[LinkEdge]) -> String {
    let note_html = note
        .map(|n| format!("<div class='notice'>{}</div>", esc(n)))
        .unwrap_or_else(|| "<p class='muted'>Ješče nema kuratorskyh not.</p>".to_string());
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>Diskusija: {}</h1>\
         <p><a href='../entry/{}.html'>← stran zapisova</a></p>\
         <h2 id='noty'>Kuratorske noty</h2>{}\
         <h2 id='review'>Spis prověrky</h2><ul><li>Prověr srodne slova i semantiku.</li><li>Prověr či oficialny synonym bolje odgovarja.</li><li>Prověr prěgibanje i pravopisne variantne znaky.</li></ul>\
         <h2 id='issue'>GitHub</h2><p><a href='{}'>Otvori problem za tu stran zapisovu</a>.</p>\
         <h2 id='links'>Obratne linky</h2><p>{} stran kaže sem.</p></article>",
        esc(&m.title),
        m.id,
        note_html,
        esc(&issue_url(m)),
        incoming.len(),
    );
    page(&format!("Diskusija: {}", m.title), &body, 1)
}

#[derive(Default)]
struct CategoryNode {
    path: Vec<String>,
    pages: Vec<SiteEntryMeta>,
    children: BTreeSet<String>,
}

fn build_category_tree(metas: &[SiteEntryMeta]) -> BTreeMap<String, CategoryNode> {
    let mut tree: BTreeMap<String, CategoryNode> = BTreeMap::new();
    for m in metas {
        for path in &m.categories {
            for i in 1..=path.len() {
                let prefix = path[..i].to_vec();
                let key = category_key(&prefix);
                tree.entry(key.clone()).or_insert_with(|| CategoryNode {
                    path: prefix.clone(),
                    pages: Vec::new(),
                    children: BTreeSet::new(),
                });
                if i > 1 {
                    let parent_key = category_key(&path[..i - 1]);
                    tree.entry(parent_key.clone())
                        .or_default()
                        .children
                        .insert(key.clone());
                }
            }
            let leaf = category_key(path);
            if let Some(node) = tree.get_mut(&leaf) {
                node.pages.push(m.clone());
            }
        }
    }
    for node in tree.values_mut() {
        node.pages
            .sort_by_key(|m| crate::orthography::ascii_skeleton(&m.title));
    }
    tree
}

fn write_category_pages(out_dir: &Path, metas: &[SiteEntryMeta]) -> Result<()> {
    let tree = build_category_tree(metas);
    let mut root_links = String::new();
    for (key, node) in tree.iter().filter(|(_, n)| n.path.len() == 1) {
        let count = category_descendant_page_count(&tree, key);
        let _ = write!(
            root_links,
            "<li><a href='category/{}.html'>{}</a> <span class='muted'>({})</span></li>",
            esc(key),
            esc(&category_title(&node.path)),
            compact(count)
        );
    }
    for (key, node) in &tree {
        std::fs::write(
            out_dir.join("category").join(format!("{key}.html")),
            category_page(&tree, key, node),
        )?;
    }
    std::fs::write(
        out_dir.join("categories.html"),
        page(
            "Kategorije",
            &format!("<article class='entry'><h1 class='firstHeading'>Kategorije</h1><p class='lede'>Hierarhične kategorije po wiki-sistemu: najprvo podkategorije, potom strany. Avtomatične kategorije sųt směšane s temami i oznakami Wiktionary, kȯgda te metadany sųt v lokalnyh cache-fajlah.</p><h2 id='podkategorije'>Podkategorije</h2><ul class='compact-list category-list'>{root_links}</ul></article>"),
            0,
        ),
    )?;
    Ok(())
}

fn category_descendant_page_count(tree: &BTreeMap<String, CategoryNode>, key: &str) -> usize {
    let mut ids = BTreeSet::new();
    collect_category_page_ids(tree, key, &mut ids);
    ids.len()
}

fn collect_category_page_ids(
    tree: &BTreeMap<String, CategoryNode>,
    key: &str,
    ids: &mut BTreeSet<usize>,
) {
    let Some(node) = tree.get(key) else { return };
    for m in &node.pages {
        ids.insert(m.id);
    }
    for child in &node.children {
        collect_category_page_ids(tree, child, ids);
    }
}

fn category_page(tree: &BTreeMap<String, CategoryNode>, _key: &str, node: &CategoryNode) -> String {
    let mut subcats = String::new();
    for child in &node.children {
        if let Some(c) = tree.get(child) {
            let count = category_descendant_page_count(tree, child);
            let label = c.path.last().map(String::as_str).unwrap_or(child);
            let _ = write!(
                subcats,
                "<li><a href='{}.html'>{}</a> <span class='muted'>({})</span></li>",
                esc(child),
                esc(label),
                compact(count)
            );
        }
    }
    let subcat_block = if subcats.is_empty() {
        String::new()
    } else {
        format!("<h2 id='podkategorije'>Podkategorije</h2><ul class='compact-list category-list'>{subcats}</ul>")
    };
    let pages = if node.pages.is_empty() {
        if node.children.is_empty() {
            String::new()
        } else {
            "<p class='muted'>Izberi podkategoriju vyše.</p>".to_string()
        }
    } else {
        render_word_table(&node.pages, "../")
    };
    let parent = if node.path.len() > 1 {
        let pk = category_key(&node.path[..node.path.len() - 1]);
        format!("<p><a href='{pk}.html'>← vyšša kategorija</a></p>")
    } else {
        "<p><a href='../categories.html'>← vse kategorije</a></p>".to_string()
    };
    let title = format!("Kategorija: {}", category_title(&node.path));
    page(
        &title,
        &format!("<article class='entry'><h1 class='firstHeading'>{}</h1>{parent}{subcat_block}<h2 id='strany'>Strany v kategoriji</h2>{pages}</article>", esc(&title)),
        1,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_wiki_indexes(
    out_dir: &Path,
    metas: &[SiteEntryMeta],
    edges: &[LinkEdge],
    backlinks: &std::collections::BTreeMap<usize, Vec<LinkEdge>>,
    homographs: &std::collections::BTreeMap<String, Vec<SiteEntryMeta>>,
    build: &BuildMeta,
    curation: &std::collections::HashMap<String, String>,
    rule_index: &RuleIndex,
    proto: Option<&crate::dump::ProtoIndex>,
    proto_reflex: &ProtoReflexIndex,
) -> Result<()> {
    for dir in [
        "category",
        "index",
        "portal",
        "what-links-here",
        "homograph",
        "root",
        "talk",
        "special",
        "borrowings",
        "needs-review",
        "rule",
        "proto",
        "deriv",
    ] {
        let p = out_dir.join(dir);
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p)?;
    }
    let mut sorted = metas.to_vec();
    sorted.sort_by_key(|m| crate::orthography::ascii_skeleton(&m.title));
    std::fs::write(
        out_dir.join("all-pages.html"),
        simple_index_page(
            "Vse strany",
            "Abecedny spis vsih slovnikovyh stran zapisov. To je podobno do Speciaľno:VseStrany: prosty, statičny indeks bez JavaScript-trebovanja.",
            &sorted,
            0,
        ),
    )?;

    write_category_pages(out_dir, metas)?;

    let mut by_first: std::collections::BTreeMap<String, Vec<SiteEntryMeta>> =
        std::collections::BTreeMap::new();
    for m in metas {
        by_first.entry(m.first.clone()).or_default().push(m.clone());
    }
    let mut letter_links = String::new();
    for (letter, rows) in &mut by_first {
        rows.sort_by_key(|m| crate::orthography::ascii_skeleton(&m.title));
        let file = format!("{}.html", slug(letter));
        std::fs::write(
            out_dir.join("index").join(&file),
            simple_index_page(
                &format!("Indeks: {letter}"),
                "Abecedny indeks po prvoj bukvě.",
                rows,
                1,
            ),
        )?;
        let _ = write!(letter_links, "<a href='index/{file}'>{}</a> ", esc(letter));
    }
    std::fs::write(
        out_dir.join("indices.html"),
        page("Indeksy", &format!("<article class='entry'><h1 class='firstHeading'>Abecedne indeksy</h1><p class='muted'>Klasičny slovnikovy indeks po prvoj bukvě.</p><p class='plainlinks alphabet-index'>{letter_links}</p></article>"), 0),
    )?;

    let mut by_lang: std::collections::BTreeMap<String, Vec<SiteEntryMeta>> =
        std::collections::BTreeMap::new();
    for m in metas {
        for lang in &m.languages {
            by_lang.entry(lang.clone()).or_default().push(m.clone());
        }
    }
    let mut portal_links = String::new();
    for (lang, rows) in &mut by_lang {
        rows.sort_by_key(|m| crate::orthography::ascii_skeleton(&m.title));
        let file = format!("{}.html", slug(lang));
        std::fs::write(
            out_dir.join("portal").join(&file),
            language_portal_page(lang, rows, metas),
        )?;
        let _ = write!(
            portal_links,
            "<li><a href='portal/{file}'>{}</a> <span class='muted'>({})</span></li>",
            esc(&crate::lang::lang_name(lang)),
            rows.len()
        );
    }
    std::fs::write(
        out_dir.join("portals.html"),
        page("Portaly", &format!("<article class='entry'><h1 class='firstHeading'>Języčne portaly</h1><p class='lede'>Vsaky portal pokazyvaje strany zapisov, v ktoryh dany slovjansky język davaje srodny dokaz. To pomagaje viděti, ktore formy sųt vȯzhodne, zapadne, južne ili vseslovjanske.</p><ul class='compact-list'>{portal_links}</ul></article>"), 0),
    )?;

    for m in metas {
        let incoming = backlinks.get(&m.id).map(Vec::as_slice).unwrap_or(&[]);
        let body = backlink_page_body(m, incoming);
        std::fs::write(
            out_dir
                .join("what-links-here")
                .join(format!("{}.html", m.id)),
            page(&format!("Čto veze k {}", m.title), &body, 1),
        )?;
        let note_key = crate::orthography::to_standard(&m.title.to_lowercase());
        let note = curation
            .get(&note_key)
            .or_else(|| curation.get(&m.id.to_string()));
        std::fs::write(
            out_dir.join("talk").join(format!("{}.html", m.id)),
            talk_page(m, note, incoming),
        )?;
    }

    let mut root_map: std::collections::BTreeMap<String, Vec<SiteEntryMeta>> =
        std::collections::BTreeMap::new();
    for m in metas {
        if let Some(sl) = ancestor_slug(m) {
            root_map.entry(sl).or_default().push(m.clone());
        }
    }
    for (sl, rows) in &mut root_map {
        rows.sort_by_key(|m| crate::orthography::ascii_skeleton(&m.title));
        let root_label = rows
            .first()
            .map(|m| m.ancestor.trim_start_matches('*').to_string())
            .unwrap_or_else(|| sl.clone());
        // Link every proto-lemma reflex page that lists at least one of THIS
        // root's entries (membership-gated, issue #73b review): a root can
        // mix ancestors that resolve to different reconstructions (cělo vs
        // čelo both slug to "celo"), so each gets its own labeled link.
        let mut root_proto: std::collections::BTreeSet<&String> = std::collections::BTreeSet::new();
        for m in rows.iter() {
            if let Some(psl) = proto_reflex.membership.get(&m.id) {
                root_proto.insert(psl);
            }
        }
        let proto_link = if root_proto.is_empty() {
            String::new()
        } else {
            let links = root_proto
                .iter()
                .map(|psl| {
                    let label = proto_reflex
                        .pages
                        .get(psl.as_str())
                        .map(|p| p.word.as_str())
                        .unwrap_or(psl.as_str());
                    format!(
                        "<a href='../proto/{psl}.html'><span class='mention'>*{}</span></a>",
                        esc(label)
                    )
                })
                .collect::<Vec<_>>()
                .join(" · ");
            format!("<p>Praslovjanske lemma-strany (rekonstrukcija, glosy, potomky): {links} →</p>")
        };
        std::fs::write(
            out_dir.join("root").join(format!("{sl}.html")),
            root_page(&root_label, rows, &proto_link),
        )?;
    }

    // Rule-fired sound-law index (issue #73a): one page per (engine, rule id)
    // + the rules.html overview.
    for ((engine, id), agg) in rule_index {
        std::fs::write(
            out_dir
                .join("rule")
                .join(format!("{}.html", rule_file_stem(engine, id))),
            rule_page(engine, id, agg),
        )?;
    }
    std::fs::write(out_dir.join("rules.html"), rules_index_page(rule_index))?;

    // Proto-lemma reflex browse (issue #73b): one page per ancestor slug with
    // a proto-cache hit + the proto-index.html overview (always written, so
    // the hardcoded hub/sidebar links never dangle on a cache-less checkout).
    if let Some(pi) = proto {
        let by_id: std::collections::HashMap<usize, &SiteEntryMeta> =
            metas.iter().map(|m| (m.id, m)).collect();
        for (sl, pg) in &proto_reflex.pages {
            std::fs::write(
                out_dir.join("proto").join(format!("{sl}.html")),
                proto_page(sl, pg, pi, &by_id),
            )?;
        }
    }
    std::fs::write(
        out_dir.join("proto-index.html"),
        proto_index_page(proto_reflex, proto),
    )?;

    for (fold, rows) in homographs {
        let body = format!(
            "<article class='entry'><h1 class='firstHeading'>Raznoznačnost: {}</h1><p class='muted'>Nekoliko stran děli tu že napisanu formu.</p>{}</article>",
            esc(fold),
            render_word_table(rows, "../")
        );
        std::fs::write(
            out_dir
                .join("homograph")
                .join(format!("{}.html", slug(fold))),
            page(&format!("Raznoznačnost: {fold}"), &body, 1),
        )?;
    }

    std::fs::write(
        out_dir.join("site-stats.html"),
        site_stats_page(metas, edges, homographs, build),
    )?;

    let borrowings: Vec<SiteEntryMeta> = metas.iter().filter(|m| m.borrowed).cloned().collect();
    std::fs::write(
        out_dir.join("borrowings.html"),
        borrowing_portal_page(&borrowings),
    )?;
    write_borrowing_subpages(out_dir, &borrowings)?;
    std::fs::write(out_dir.join("needs-review.html"), needs_review_page(metas))?;
    write_needs_review_subpages(out_dir, metas)?;
    std::fs::write(out_dir.join("suffix-index.html"), suffix_index_page(metas))?;
    std::fs::write(
        out_dir.join("inflection-issues.html"),
        inflection_issues_page(metas),
    )?;
    std::fs::write(out_dir.join("featured.html"), featured_page(metas, build))?;
    std::fs::write(out_dir.join("random.html"), random_page())?;
    std::fs::write(out_dir.join("special.html"), special_pages_hub())?;

    std::fs::write(out_dir.join("graph.html"), graph_page(edges, metas))?;
    std::fs::write(out_dir.join("contribute.html"), contribute_page())?;
    std::fs::write(out_dir.join("build.json"), build_json(build))?;
    std::fs::write(out_dir.join("entries.json"), entries_json(metas))?;
    std::fs::write(out_dir.join("edges.json"), graph_json(edges))?;
    std::fs::write(out_dir.join("categories.json"), categories_json(metas))?;
    std::fs::write(out_dir.join("roots.json"), roots_json(&root_map))?;
    std::fs::write(out_dir.join("rules.json"), rules_json(rule_index))?;
    // `datasets.html` is written by `export_corpus` after the raw-lemma loop, so it
    // can document the site-level raw render/dedup coverage counts (issue #35).
    std::fs::write(out_dir.join("sitemap.xml"), sitemap_xml(metas))?;
    Ok(())
}

fn backlink_page_body(m: &SiteEntryMeta, incoming: &[LinkEdge]) -> String {
    let mut rows = String::new();
    for e in incoming {
        let _ = write!(
            rows,
            "<li><a href='../entry/{}.html'>{}</a> <span class='badge'>{}</span></li>",
            e.source_id,
            esc(&e.source_title),
            esc(&e.kind)
        );
    }
    if rows.is_empty() {
        rows.push_str("<li class='muted'>Nijedna statična strana nyně ne kaže sem.</li>");
    }
    format!(
        "<article class='entry'><h1 class='firstHeading'>Čto kaže sem: {}</h1><p><a href='../entry/{}.html'>← nazad k zapisu</a></p><ul class='compact-list'>{rows}</ul></article>",
        esc(&m.title),
        m.id
    )
}

fn entry_tabs(m: &SiteEntryMeta) -> String {
    format!(
        "<nav class='entry-tabs'><a class='active' href='{}.html'>Strana</a><a href='../talk/{}.html'>Diskusija</a><a href='../what-links-here/{}.html'>Čto kaže sem</a><a href='../graph.html#n{}'>Graf</a><a href='{}'>Popraviti / problem</a></nav>",
        m.id,
        m.id,
        m.id,
        m.id,
        esc(&issue_url(m))
    )
}

/// The razumlivost row-label / chip tooltip (issue #79): what the number is —
/// and is not — plus the population source.
const RAZUM_TITLE: &str = "dolja govoriteljev slovjanskyh językov s poznatym srodnym slovom (po atestaciji) — ne izměrjena razumlivosť; izvor populacij: voting machine (steen)";

/// The official-only variant of [`RAZUM_TITLE`]: the basis there is the
/// committee's own sameInLanguages attestation, NOT cognate-set membership
/// (the translation cells are filled for every language and would claim a
/// constant ~99%).
const RAZUM_TITLE_OFFICIAL: &str = "dolja govoriteljev slovjanskyh językov, v ktoryh slovo je isto po oficialnom slovniku (sameInLanguages) — ne izměrjena razumlivosť; izvor populacij: voting machine (steen)";

/// The MATCHED-entry variant of [`RAZUM_TITLE`] (issue #86): the basis is the
/// union of the corpus cognate membership and the matched official row's own
/// sameInLanguages attestation — either basis alone misreads one tail.
const RAZUM_TITLE_MATCHED: &str = "dolja govoriteljev slovjanskyh językov s poznatym srodnym slovom — po srodnyh slovah v korpusu i po oficialnom sameInLanguages; ne izměrjena razumlivosť; izvor populacij: voting machine (steen)";

/// The raw-collision display credit line (issue #86 item 6): raw Wiktionary
/// attestations whose display fold deduped onto this page
/// ([`RawFate::DedupedFold`]) — the site already knew these words but showed
/// them nowhere ("uk алое carries NO raw row anywhere and credits no
/// evidence"). Rendered as a compact muted line in the cognate section, each
/// item linking to the source-language Wiktionary (same
/// [`crate::enrich::source_url`] the raw pages use), capped at 12 with a
/// "+N dalje" tail. DISPLAY ONLY — never counted in n_langs / Dokaz /
/// razumlivost / the vote: raw evidence stays benchmark-forbidden by type.
fn raw_credit_line(credits: Option<&Vec<(String, String)>>) -> String {
    let Some(credits) = credits else {
        return String::new();
    };
    if credits.is_empty() {
        return String::new();
    }
    const CAP: usize = 12;
    let mut items: Vec<String> = credits
        .iter()
        .take(CAP)
        .map(|(lang, word)| {
            format!(
                "<a href='{}'>{} {}</a>",
                esc(&crate::enrich::source_url(lang, word)),
                esc(lang),
                esc(word)
            )
        })
        .collect();
    if credits.len() > CAP {
        items.push(format!("+{} dalje", credits.len() - CAP));
    }
    format!(
        "<p class='muted raw-credit'>Takože atestovano <span title='surove atestacije iz Wiktionary, ktoryh pisanje sovpada s tojų stranojų — ne sųt dokaz i ne vlivajųt na razumlivosť'>(surova atestacija)</span>: {}</p>",
        items.join(" · ")
    )
}

/// The razumlivost basis for a MATCHED entry (issue #86): the union of the
/// corpus cognate membership and the matched official row's sameInLanguages
/// expansion. Sorted + deduped so the basis is deterministic. Display-only —
/// this never feeds extraction, grouping, evidence counts or the vote.
fn union_razum_codes(corpus_langs: &[String], same_in: &[&'static str]) -> Vec<String> {
    let mut codes: Vec<String> = corpus_langs.to_vec();
    for c in same_in {
        if !codes.iter().any(|x| x == c) {
            codes.push(c.to_string());
        }
    }
    codes.sort();
    codes.dedup();
    codes
}

/// Razumlivost overall percent for a language-code list, as the integer the
/// search row carries in element 12 (issue #79).
fn razum_pct(langs: &[String]) -> u32 {
    let codes: Vec<&str> = langs.iter().map(String::as_str).collect();
    crate::lang::razumlivost(&codes).overall.round() as u32
}

/// The infobox "Razumlivosť" row for a set of attesting language codes, with
/// the basis-appropriate tooltip (issue #79).
fn razum_row(codes: &[&str], title: &str) -> String {
    let r = crate::lang::razumlivost(codes);
    format!(
        "<tr><th title='{title}'>Razumlivosť</th><td><b>{:.0}%</b> {}</td></tr>",
        r.overall,
        razum_bars(&r),
    )
}

/// Three compact per-branch coverage bars for a [`crate::lang::Razumlivost`]
/// value, labeled V/Z/J (East/West/South) with the full branch label as the
/// tooltip. Deliberately NOT under an html id "razumlivost" — that id belongs
/// to the committee intelligibility strip.
fn razum_bars(r: &crate::lang::Razumlivost) -> String {
    let bar = |label: &str, branch: Branch, v: f32| {
        format!(
            "<span class='razb' title='{}: {v:.0}%'>{label}<span class='razt'><span class='razf' style='width:{:.0}%'></span></span></span>",
            branch.label(),
            v.clamp(0.0, 100.0),
        )
    };
    format!(
        "{}{}{}",
        bar("V", Branch::East, r.east),
        bar("Z", Branch::West, r.west),
        bar("J", Branch::South, r.south),
    )
}

/// `razum` is the prebuilt "Razumlivosť" row (or empty to omit): the honest
/// membership differs per page kind — cognate-set members on generated
/// pages, the single attesting language on raw pages, and the committee's
/// sameInLanguages on official-only pages (empty column → no row), so the
/// caller supplies it (issue #79).
/// `proto_link` is the prebuilt "(rekonstrukcija)" link to the proto-lemma
/// reflex page (issue #73b), or empty — the caller checks whether that page
/// exists (only the generated loop knows the emitted `proto/` slugs). It is
/// ADDED next to the existing root link, never replacing it.
fn entry_infobox(m: &SiteEntryMeta, razum: &str, extra_rows: &str, proto_link: &str) -> String {
    let root = ancestor_slug(m)
        .map(|sl| format!("<a href='../root/{sl}.html'>{}</a>", esc(&m.ancestor)))
        .unwrap_or_else(|| {
            esc(if m.ancestor.is_empty() {
                "—"
            } else {
                &m.ancestor
            })
        });
    // Calibrated reliability badge (issue #77). Official words state the
    // fact ("oficialno" — not a prediction, no p): official-only pages AND
    // matched entries (issue #86 — the calibrated prior moved to the
    // provenance transparency line); raw attestation pages keep no badge row,
    // as before.
    let reliability = if m.raw {
        String::new()
    } else if m.official_only || m.official_lemma.is_some() {
        "<tr><th>Uvěrjenost</th><td><span class='reliability conf-high'>oficialno</span></td></tr>"
            .to_string()
    } else {
        let p = m
            .prob
            .map(|p| {
                format!(
                    " <span class='score muted' title='kalibrovana věrojętnosť P(odgovara oficialnomu rěšenju); metodologija: target/eval/methodology.md'>p≈{p:.2}</span>"
                )
            })
            .unwrap_or_default();
        format!(
            "<tr><th>Uvěrjenost</th><td><span class='reliability {}'>{}</span>{p}</td></tr>",
            conf_class(m.conf),
            m.conf.label(),
        )
    };
    let aspect_rows = m
        .aspect
        .as_ref()
        .map(|aspect| {
            let partner = if m.aspect_partners.is_empty() {
                String::new()
            } else {
                let links = m
                    .aspect_partners
                    .iter()
                    .map(|(id, title)| format!("<a href='{id}.html'>{}</a>", esc(title)))
                    .collect::<Vec<_>>()
                    .join(" · ");
                format!("<tr><th>Vidovy partneri</th><td>{links}</td></tr>")
            };
            format!(
                "<tr><th>Glagolsky vid</th><td>{}</td></tr>{partner}",
                esc(aspect)
            )
        })
        .unwrap_or_default();
    format!(
        "<aside class='entry-infobox'><table class='wikitable compact-table'><caption>{}</caption>\
         <tr><th>Čęst rěči</th><td>{}</td></tr>{aspect_rows}<tr><th>Stav</th><td>{}</td></tr>{reliability}\
         <tr><th>Kvaliteta</th><td>{}</td></tr><tr><th>Dokaz</th><td>{} jęz. / {} vět.</td></tr>{razum}\
         <tr><th>Tip</th><td>{}</td></tr><tr><th>Predok</th><td>{}{proto_link}</td></tr>{extra_rows}<tr><th>ID</th><td>{}</td></tr></table></aside>",
        esc(&m.title),
        esc(&pos_code_label(&m.pos)),
        if m.official_lemma.is_some() { "oficialno povezano" } else { "generovano" },
        esc(quality_label(m)),
        m.n_langs,
        m.n_branches,
        if m.borrowed { "zaimka" } else { "naslědovano" },
        root,
        m.id,
    )
}

fn homograph_notice(
    m: &SiteEntryMeta,
    groups: &std::collections::BTreeMap<String, Vec<SiteEntryMeta>>,
) -> String {
    let key = crate::orthography::to_standard(&m.title.to_lowercase());
    let Some(rows) = groups.get(&key) else {
        return String::new();
    };
    if rows.len() < 2 {
        return String::new();
    }
    format!(
        "<div class='notice dab'>Ta napis imaje <b>{}</b> značenja. <a href='../homograph/{}.html'>Ględi raznoznačnosť</a>.</div>",
        rows.len(),
        slug(&key)
    )
}

fn entry_wiki_blocks(
    m: &SiteEntryMeta,
    incoming: &[LinkEdge],
    edges: &[LinkEdge],
    curation: &std::collections::HashMap<String, String>,
    build: &BuildMeta,
) -> String {
    let mut out = String::new();
    let note_key = crate::orthography::to_standard(&m.title.to_lowercase());
    if let Some(note) = curation
        .get(&note_key)
        .or_else(|| curation.get(&m.id.to_string()))
    {
        let _ = write!(
            out,
            "<section><h2 id='notes'>Kuratorske noty</h2><div class='notice'>{}</div></section>",
            esc(note)
        );
    }
    out.push_str(&local_graph_block(m, incoming, edges));
    let _ = write!(
        out,
        "<details id='source-meta' class='bottom-meta'><summary>Izvory i metadany</summary>{}{}</details>",
        references_block(m),
        provenance_block(m, build)
    );
    out.push_str(&category_footer(m));
    out
}

fn local_graph_block(m: &SiteEntryMeta, incoming: &[LinkEdge], edges: &[LinkEdge]) -> String {
    let mut items = String::new();
    for e in edges.iter().filter(|e| e.source_id == m.id).take(18) {
        let _ = write!(
            items,
            "<li><span class='badge'>{}</span> <a href='{}.html'>{}</a></li>",
            esc(&e.kind),
            e.target_id,
            esc(&e.target_title)
        );
    }
    for e in incoming.iter().take(18) {
        let _ = write!(
            items,
            "<li><span class='badge'>← {}</span> <a href='{}.html'>{}</a></li>",
            esc(&e.kind),
            e.source_id,
            esc(&e.source_title)
        );
    }
    if items.is_empty() {
        return String::new();
    }
    format!("<section><h2 id='graf'>Semantičny graf</h2><ul class='compact-list graph-list'>{items}</ul></section>")
}

fn references_block(m: &SiteEntryMeta) -> String {
    let mut rows = String::new();
    if let Some(isv) = &m.official_lemma {
        let _ = write!(
            rows,
            "<tr><th>Oficialny slovnik</th><td><span class='mention'>{}</span></td><td>lemmat / validacija</td></tr>",
            esc(isv)
        );
    }
    if !m.ancestor.trim().is_empty() {
        if m.borrowed {
            let _ = write!(
                rows,
                "<tr><th>Etimon</th><td><span class='mention'>{}</span></td><td>zaimka / internacionalizm</td></tr>",
                esc(&m.ancestor)
            );
        } else {
            let p = m.ancestor.trim_start_matches('*');
            let root = ancestor_slug(m)
                .map(|sl| format!("; <a href='../root/{sl}.html'>korenj-strana</a>"))
                .unwrap_or_default();
            let _ = write!(rows, "<tr><th>Praslovjansky prědȯk</th><td><a href='https://en.wiktionary.org/wiki/Reconstruction:Proto-Slavic/{}'>*{}</a>{}</td><td>rekonstrukcija Wiktionary</td></tr>", esc(p), esc(p), root);
        }
    }
    rows.push_str("<tr><th>Srodne slova</th><td>anglijska Wiktionary + narodne Wiktionary</td><td>CC BY-SA; konkretne linky sųt v tablicah vyše</td></tr>");
    rows.push_str(
        "<tr><th>Prěgibanje</th><td>interslavic-rs</td><td>mašinno generovane formy</td></tr>",
    );
    rows.push_str("<tr><th>Generator</th><td><a href='https://github.com/gold-silver-copper/Slovowiki'>izvorny kod</a></td><td>pravila, indeks iskanja, statičny eksport</td></tr>");
    format!("<section><h2 id='references'>Izvory</h2><table class='wikitable source-table'><tbody>{rows}</tbody></table></section>")
}

fn provenance_block(m: &SiteEntryMeta, build: &BuildMeta) -> String {
    // Matched entries: the calibrated PRIOR the generator assigned before the
    // official match resolved it — a muted transparency line, deliberately in
    // the provenance section and not the infobox badge (issue #86: the badge
    // states the fact "oficialno"; this line documents what the model thought
    // beforehand).
    let prior_row = m
        .prior
        .filter(|_| m.official_lemma.is_some() && !m.official_only)
        .map(|p| {
            format!(
                "<tr><th>Priorna ocěna</th><td><span class='muted'>Priorna kalibrovana ocěna generatora: p≈{p:.2} (prěd sravnjenjem s oficialnym slovnikom)</span></td></tr>"
            )
        })
        .unwrap_or_default();
    format!(
        "<section><h2 id='provenance'>Istorija i metadany</h2><table class='wikitable compact-table'>\
         <tr><th>Generacija</th><td>{}</td></tr><tr><th>Git</th><td><code>{}</code></td></tr>\
         <tr><th>Tip</th><td>{}</td></tr><tr><th>Kvaliteta</th><td>{}</td></tr>\
         <tr><th>Ocěna</th><td>{:.2}</td></tr>{prior_row}<tr><th>Dokaz</th><td>{} językov / {} větvy</td></tr>\
         <tr><th>Popraviti</th><td><a href='{}'>Otvori problem na GitHub za tu stranu</a></td></tr></table></section>",
        esc(&build.generated),
        esc(&build.git),
        if m.official_only { "samo oficialno" } else if m.borrowed { "zaimka / internacionalizm" } else { "srodna rekonstrukcija" },
        esc(quality_label(m)),
        m.score,
        m.n_langs,
        m.n_branches,
        esc(&issue_url(m)),
    )
}

fn category_footer(m: &SiteEntryMeta) -> String {
    let link_for = |path: &Vec<String>| {
        format!(
            "<a href='../category/{}.html'>{}</a>",
            esc(&category_key(path)),
            esc(&category_title(path))
        )
    };
    let visible = 12usize;
    let mut links = m
        .categories
        .iter()
        .take(visible)
        .map(link_for)
        .collect::<Vec<_>>()
        .join(" | ");
    if m.categories.len() > visible {
        let rest = m
            .categories
            .iter()
            .skip(visible)
            .map(link_for)
            .collect::<Vec<_>>()
            .join(" | ");
        let _ = write!(
            links,
            " <details class='cat-more'><summary>+{} kategorij</summary>{}</details>",
            m.categories.len() - visible,
            rest
        );
    }
    format!("<div id='categories' class='catlinks'><b>Kategorije</b>: {links}</div>")
}

fn graph_json(edges: &[LinkEdge]) -> String {
    let mut s = String::from("[\n");
    for (i, e) in edges.iter().take(50000).enumerate() {
        if i > 0 {
            s.push_str(",\n");
        }
        let _ = write!(
            s,
            "[{},{},{},{}]",
            e.source_id,
            e.target_id,
            json_str(&e.kind),
            json_str(&e.target_title)
        );
    }
    s.push_str("\n]\n");
    s
}

fn graph_page(edges: &[LinkEdge], metas: &[SiteEntryMeta]) -> String {
    let mut kind_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut degree: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for e in edges {
        *kind_counts.entry(e.kind.clone()).or_insert(0) += 1;
        *degree.entry(e.source_id).or_insert(0) += 1;
        *degree.entry(e.target_id).or_insert(0) += 1;
    }
    let meta_by_id: std::collections::HashMap<usize, &SiteEntryMeta> =
        metas.iter().map(|m| (m.id, m)).collect();
    let mut top: Vec<(usize, usize)> = degree.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut top_items = String::new();
    for (id, n) in top.into_iter().take(40) {
        if let Some(m) = meta_by_id.get(&id) {
            let _ = write!(
                top_items,
                "<li><a href='entry/{id}.html'>{}</a> <span class='muted'>({} vęzej)</span></li>",
                esc(&m.title),
                n
            );
        }
    }
    let mut items = String::new();
    for e in edges.iter().take(800) {
        let _ = write!(items, "<li class='graph-edge' data-kind='{}' id='n{}'><a href='entry/{}.html'>{}</a> — <span class='badge'>{}</span> → <a href='entry/{}.html'>{}</a></li>", esc(&e.kind), e.source_id, e.source_id, esc(&e.source_title), esc(&e.kind), e.target_id, esc(&e.target_title));
    }
    let mut filter = String::from("<button type='button' data-kind=''>vse</button> ");
    for k in kind_counts.keys() {
        let _ = write!(
            filter,
            "<button type='button' data-kind='{}'>{}</button> ",
            esc(k),
            esc(k)
        );
    }
    let body = format!("<article class='entry'><h1 class='firstHeading'>Semantičny graf</h1><p class='muted'>Statičny spis prvih vęzej; polny kompaktny JSON je v <a href='edges.json'><code>edges.json</code></a>. Filtry rabotajų bez servera.</p><div class='graph-filter'>{filter}</div><div class='stat-grid wiki-stats'>{}</div><h2 id='top'>Najbolje povezane strany</h2><ol>{top_items}</ol><h2 id='edges'>Vęzi</h2><ul class='compact-list'>{items}</ul><script>document.querySelectorAll('.graph-filter button').forEach(function(b){{b.onclick=function(){{var k=b.dataset.kind;document.querySelectorAll('.graph-edge').forEach(function(e){{e.style.display=(!k||e.dataset.kind===k)?'':'none';}});}};}});</script></article>", counts_table("Tipy vęzej", &kind_counts));
    page("Semantičny graf", &body, 0)
}

fn contribute_page() -> String {
    let body = "<article class='entry'><h1 class='firstHeading'>Kako pomagati</h1>\
      <p>Projekt je statično generovany: změni podatky, regeneruj sajt, zapusti testy, pošlji prošnju za spoj.</p>\
      <ol><li><code>cargo test</code></li><li><code>cargo run --release -- export --out site</code></li><li>Za ručne noty dodaj <code>data/curation-notes.json</code> s ključem zaglavnogo slova ili id-ja.</li><li>Za grešku v zapisu klikni <i>Popraviti / problem</i> na vrhu strany.</li></ol>\
      <h2>Kuracija bez koda</h2>\
      <ul>\
        <li><b>Semantične pasti</b> (falšive prijatelje): <code>data/semantic-notes.json</code> — vsaka nota mųsi citovati oficialno značenje; noty sę pokazujųt v <a href='text-check.html'>Prověrkě teksta</a> i v CLI <code>check-text</code>.</li>\
        <li><b>Predloženja novyh slov</b>: prěgledaj <a href='proposals.html'>Predloženja</a>, kogda korpusny model bude iměti vlastnu validovanu kalibraciju, i dodaj kuratorsku notu za slovo.</li>\
        <li><b>Prověrka form</b>: <a href='forms.html'>Iskanje form</a> pokazyvaje vse analizy kojejkoli fleksijnoj formy.</li>\
      </ul>\
      <p>Za stroje i skripty: statičny leksikalny API pod <code>api/</code> (<a href='api/agent-guide.md'>agent-guide.md</a>, <a href='datasets.html'>datoteky</a>).</p>\
      <p><a href='https://github.com/gold-silver-copper/Slovowiki'>Izvorny kod na GitHub</a> — vidi <code>CONTRIBUTING.md</code> za metodologiju (benchmark-gated pravila, dev/holdout, značimost).</p></article>";
    page("Prinos", body, 0)
}

/// Machine-queryable entry metadata. Fields per entry: id, title, gloss, pos,
/// quality, confidence, prob (calibrated probability, null for
/// official/raw), langs (attesting-language COUNT), `langs_list` (the sorted
/// attesting language-code SET, issue #73c), branches (branch count),
/// `branch_pattern` (the exact branch combination "V"/"Z"/"J"/"V+Z"/…/
/// "V+Z+J", null when no code resolves — issue #73c), borrowed, official,
/// `official_id` (authoritative dictionary sense row, null otherwise),
/// ancestor, aspect, and aspect_partners (`[{id,title},…]`; issue #75).
/// `langs_list` + `branch_pattern` make any attestation-pattern
/// query a jq one-liner (e.g. `.[] | select(.branch_pattern == "V+J")`).
fn entries_json(metas: &[SiteEntryMeta]) -> String {
    let mut s = String::from("[\n");
    for (i, m) in metas.iter().enumerate() {
        if i > 0 {
            s.push_str(",\n");
        }
        // `prob` is null for ALL official words — official-only AND matched
        // (issue #86): a verified dictionary fact carries no prediction
        // probability, mirroring the API's lemma records.
        let prob = m
            .prob
            .map(|p| format!("{p:.3}"))
            .unwrap_or_else(|| "null".to_string());
        let langs_list = m
            .languages
            .iter()
            .map(|l| json_str(l))
            .collect::<Vec<_>>()
            .join(",");
        let pattern = branch_pattern(&m.languages)
            .map(|p| json_str(&p))
            .unwrap_or_else(|| "null".to_string());
        let official_id = m
            .official_sense_id
            .as_ref()
            .map(|id| json_str(id))
            .unwrap_or_else(|| "null".to_string());
        let aspect = m
            .aspect
            .as_ref()
            .map(|a| json_str(a))
            .unwrap_or_else(|| "null".to_string());
        let partners = m
            .aspect_partners
            .iter()
            .map(|(id, title)| format!("{{\"id\":{id},\"title\":{}}}", json_str(title)))
            .collect::<Vec<_>>()
            .join(",");
        let _ = write!(s, "{{\"id\":{},\"title\":{},\"gloss\":{},\"pos\":{},\"quality\":{},\"confidence\":{},\"prob\":{},\"langs\":{},\"langs_list\":[{}],\"branches\":{},\"branch_pattern\":{},\"borrowed\":{},\"official\":{},\"official_id\":{},\"ancestor\":{},\"aspect\":{},\"aspect_partners\":[{}]}}",
            m.id, json_str(&m.title), json_str(&m.gloss), json_str(&m.pos), json_str(quality_label(m)), json_str(m.conf.label()), prob, m.n_langs, langs_list, m.n_branches, pattern, m.borrowed, m.official_lemma.is_some(), official_id, json_str(&m.ancestor), aspect, partners);
    }
    s.push_str("\n]\n");
    s
}

fn categories_json(metas: &[SiteEntryMeta]) -> String {
    let tree = build_category_tree(metas);
    let mut s = String::from("[\n");
    for (i, (key, node)) in tree.iter().enumerate() {
        if i > 0 {
            s.push_str(",\n");
        }
        let path = node
            .path
            .iter()
            .map(|p| json_str(p))
            .collect::<Vec<_>>()
            .join(",");
        let pages = node
            .pages
            .iter()
            .map(|m| m.id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let children = node
            .children
            .iter()
            .map(|c| json_str(c))
            .collect::<Vec<_>>()
            .join(",");
        let _ = write!(
            s,
            "  {{\"key\":{},\"path\":[{}],\"children\":[{}],\"pages\":[{}]}}",
            json_str(key),
            path,
            children,
            pages
        );
    }
    s.push_str("\n]\n");
    s
}

fn roots_json(roots: &std::collections::BTreeMap<String, Vec<SiteEntryMeta>>) -> String {
    let mut s = String::from("{\n");
    for (i, (root, rows)) in roots.iter().enumerate() {
        if i > 0 {
            s.push_str(",\n");
        }
        let list = rows
            .iter()
            .map(|m| m.id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let _ = write!(s, "  {}: [{}]", json_str(root), list);
    }
    s.push_str("\n}\n");
    s
}

// ---------------------------------------------------------------------------
// Scholarly query layer (issue #73)
//
// Four browse surfaces over facts the export already computes: (a) a
// rule-fired sound-law index over the rendered rule traces, (b) proto-lemma
// reflex pages joining ancestors to the proto cache's full reconstructions,
// (c) branch attestation-pattern categories (in `entry_categories` /
// `entries.json`), and (d) a derivational-suffix browse over the rendered
// derivation blocks. Display/export-side only: nothing here feeds generation,
// the benchmark, the forms API, or the search rows.
// ---------------------------------------------------------------------------

/// One firing of a trace rule on a rendered entry (issue #73a): the page's
/// TOP candidate applied `before → after` under this rule.
struct RuleRow {
    id: usize,
    display: String,
    before: String,
    after: String,
    pos: String,
    n_langs: usize,
    n_branches: usize,
}

/// All firings of one (engine, rule id). The explanation texts are DYNAMIC
/// for several ids (consensus-vote, pick-representative, proto-ancestor, …:
/// they embed the entry's own vote counts / forms), so the first firing's
/// text is kept only as an attributed EXAMPLE — `example_display` names the
/// entry it came from and the pages label it "Priklad (…)", never as the
/// rule's general description. The doc reference IS constant per id.
struct RuleAgg {
    explanation: String,
    /// Display headword + page id of the entry `explanation` was taken from.
    example_display: String,
    example_id: usize,
    reference: Option<String>,
    rows: Vec<RuleRow>,
}

/// (engine, rule id) → aggregated firings. Rule ids are stable but NOT
/// globally unique across engines — "liquid-metathesis" is emitted by both
/// the Proto-Slavic rule engine and the consensus repairs — so the index keys
/// on the pair, the same disambiguation `eval::stage_of_step` uses.
type RuleIndex = BTreeMap<(&'static str, String), RuleAgg>;

/// The proto-lemma reflex join (issue #73b). One page per RESOLVED
/// reconstruction word (the accent-folded proto-cache word — canonical, so
/// accented and unaccented ancestor spellings of one lemma share a page),
/// carrying ALL homonymous ProtoEntries under that word. `membership` is the
/// authoritative entry→page map: the infobox "(rekonstrukcija)" link and the
/// root-page links are gated on it, so a link can never point at a proto
/// page that does not list the linking entry.
#[derive(Default)]
struct ProtoReflexIndex {
    /// Page slug → group. Slugs derive from the canonical folded word;
    /// collisions between DIFFERENT folded words (cělo vs čelo → "celo") get
    /// deterministic "-2"/"-3"… suffixes in folded-word order.
    pages: BTreeMap<String, ProtoReflexPage>,
    /// Entry id → page slug, only for entries whose ancestor resolved.
    membership: BTreeMap<usize, String>,
    linked: usize,
    misses: usize,
}

/// One proto reflex page: a canonical folded reconstruction word, ALL
/// proto-cache entries folding to it (homonyms / accent variants), and the
/// rendered entries whose ancestor resolved to it.
struct ProtoReflexPage {
    /// Canonical accent-folded cache word (no '*').
    word: String,
    /// ProtoEntry indexes whose folded word equals `word`, in cache order.
    recons: Vec<usize>,
    /// Linked entry ids, ascending.
    entry_ids: Vec<usize>,
}

/// Engine tag for the rule index, derived from the top candidate's source
/// exactly as the benchmark derives its `is_proto` flag.
fn rule_engine(source: CandidateSource) -> &'static str {
    if source == CandidateSource::ProtoSlavicRule {
        "proto"
    } else {
        "konsensus"
    }
}

/// The machine key `rules.json` uses for one rule: `<engine>:<id>`.
fn rule_key(engine: &str, id: &str) -> String {
    format!("{engine}:{id}")
}

/// The `rule/` page file stem for one rule: `<engine>-<slug(id)>`. Rule ids
/// are kebab-case ASCII literals today, so `slug` is normally the identity —
/// it only guards a future id against file-name-unsafe characters.
fn rule_file_stem(engine: &str, id: &str) -> String {
    format!("{engine}-{}", slug(id))
}

/// Build the rule-fired index from the entries the generated loop renders:
/// only the TOP candidate's trace (pages render `trace_block(top)`, so the
/// index agrees with what pages show), suppressed entries already filtered by
/// the caller. Rows are sorted by display-headword skeleton, deterministic.
fn build_rule_index<'a>(
    entries: impl Iterator<Item = (usize, &'a str, &'a crate::corpus::GeneratedWord)>,
) -> RuleIndex {
    let mut index: RuleIndex = BTreeMap::new();
    for (id, display, g) in entries {
        let Some(top) = g.candidates.first() else {
            continue;
        };
        let engine = rule_engine(top.source);
        for step in &top.trace {
            let agg = index
                .entry((engine, step.id.clone()))
                .or_insert_with(|| RuleAgg {
                    explanation: step.explanation.clone(),
                    example_display: display.to_string(),
                    example_id: id,
                    reference: step.reference.clone(),
                    rows: Vec::new(),
                });
            agg.rows.push(RuleRow {
                id,
                display: display.to_string(),
                before: step.before.clone(),
                after: step.after.clone(),
                pos: g.set.pos.code().to_string(),
                n_langs: g.n_langs,
                n_branches: g.n_branches,
            });
        }
    }
    for agg in index.values_mut() {
        agg.rows.sort_by(|a, b| {
            crate::orthography::ascii_skeleton(&a.display)
                .cmp(&crate::orthography::ascii_skeleton(&b.display))
                .then(a.id.cmp(&b.id))
        });
    }
    index
}

/// Fold a Proto-Slavic word for the reflex join: strip COMBINING accent /
/// length marks (U+0300–U+036F, e.g. *pę̑tь) AND fold the PREcomposed
/// accented vowels (à á â ã ā ȁ ȃ ò ì … — the majority of ancestor accents)
/// via the proto engine's own [`crate::proto::debase_vowel`] table, which
/// preserves the etymological letters (ě ę ǫ ъ ь y). Applied identically to
/// the ancestor string and the cache word, so the two sides cannot drift.
fn fold_proto_accents(w: &str) -> String {
    w.chars()
        .filter(|c| !('\u{0300}'..='\u{036F}').contains(c))
        .map(crate::proto::debase_vowel)
        .collect()
}

/// Resolve every rendered entry's non-borrowed '*' ancestor against the
/// proto cache — both sides folded through [`fold_proto_accents`] (they are
/// already homoglyph-folded at load) — and group pages by the RESOLVED
/// folded cache word. Where several ProtoEntries share the folded word
/// (homonyms, accent variants), the page carries ALL of them; where two
/// different folded words collide on a slug (cělo vs čelo → "celo"), the
/// later word (folded-word order) gets a deterministic "-2"/"-3"… suffix.
/// Misses are expected — the corpus carries ancestors the 5.4k-entry cache
/// does not.
fn build_proto_reflex_index<'a>(
    proto: Option<&crate::dump::ProtoIndex>,
    entries: impl Iterator<Item = (usize, &'a crate::corpus::CognateSet)>,
) -> ProtoReflexIndex {
    let mut index = ProtoReflexIndex::default();
    let Some(pi) = proto else {
        return index;
    };
    // Folded cache word → ALL ProtoEntry indexes (a single-index map would
    // silently attribute homonyms to whichever entry came first).
    let mut by_fold: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, e) in pi.entries.iter().enumerate() {
        by_fold
            .entry(fold_proto_accents(&e.word))
            .or_default()
            .push(i);
    }
    // Resolve entries; group linked ids by the canonical folded word.
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (id, set) in entries {
        if set.borrowed || !set.proto.starts_with('*') {
            continue;
        }
        let folded = fold_proto_accents(set.proto.trim_start_matches('*'));
        if by_fold.contains_key(&folded) {
            groups.entry(folded).or_default().push(id);
            index.linked += 1;
        } else {
            index.misses += 1;
        }
    }
    // Assign page slugs in folded-word order (deterministic) and build the
    // membership map that gates every inbound link.
    let mut used: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (word, mut ids) in groups {
        let base = slug(&word);
        let n = used.entry(base.clone()).or_insert(0);
        *n += 1;
        let sl = if *n == 1 { base } else { format!("{base}-{n}") };
        ids.sort_unstable();
        ids.dedup();
        for &id in &ids {
            index.membership.insert(id, sl.clone());
        }
        let recons = by_fold.get(&word).cloned().unwrap_or_default();
        index.pages.insert(
            sl,
            ProtoReflexPage {
                word,
                recons,
                entry_ids: ids,
            },
        );
    }
    index
}

/// One rule's page: lede (engine tag, explanation, [dok] reference), then
/// every rendered entry whose top-candidate trace fired it.
fn rule_page(engine: &str, id: &str, agg: &RuleAgg) -> String {
    let dok = agg
        .reference
        .as_deref()
        .map(|r| format!(" <a class='doc-ref' href='{}'>[dok]</a>", esc(r)))
        .unwrap_or_default();
    let mut rows = String::new();
    for r in &agg.rows {
        let _ = write!(
            rows,
            "<tr><td><a href='../entry/{}.html'><b>{}</b></a></td><td><span class='mention'>{}</span> → <span class='mention'>{}</span></td><td>{}</td><td class='muted'>{} jęz. / {} vět.</td></tr>",
            r.id,
            esc(&r.display),
            esc(&r.before),
            esc(&r.after),
            esc(&pos_code_label(&r.pos)),
            r.n_langs,
            r.n_branches,
        );
    }
    let title = format!("Pravilo: {id}");
    // The explanation text is per-entry (several ids embed the entry's own
    // vote counts / forms), so it is shown ONLY as an attributed example —
    // the rule's stable identity is the id + engine + [dok] reference.
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>{}</h1>\
         <p class='lede'><span class='badge'>{}</span> <code class='rule-id'>{}</code>{dok}</p>\
         <p>Priklad (<a href='../entry/{}.html'>{}</a>): <span class='muted'>{}</span></p>\
         <p class='muted'>Strany zapisov, na ktoryh sled pravil pokazanogo (najvyše rangovanogo) kandidata koristi to pravilo — {} použitij. „Prěd → po“ je točna transformacija togo kroka; objasnjenje kroka je specifično za vsaky zapis (vidi jego stranu).</p>\
         <p><a href='../rules.html'>← vse pravila</a></p>\
         <table class='wikitable'><thead><tr><th>Slovo</th><th>Prěd → po</th><th>Čęst rěči</th><th>Dokaz</th></tr></thead><tbody>{rows}</tbody></table></article>",
        esc(&title),
        esc(engine),
        esc(id),
        agg.example_id,
        esc(&agg.example_display),
        esc(&agg.explanation),
        agg.rows.len(),
    );
    page(&title, &body, 1)
}

/// The rules.html overview: every (engine, id) with its firing count and doc
/// link, sorted by count desc then key.
fn rules_index_page(rule_index: &RuleIndex) -> String {
    let mut items: Vec<(&(&'static str, String), &RuleAgg)> = rule_index.iter().collect();
    items.sort_by(|a, b| {
        b.1.rows
            .len()
            .cmp(&a.1.rows.len())
            .then_with(|| a.0.cmp(b.0))
    });
    let mut rows = String::new();
    for ((engine, id), agg) in items {
        let dok = agg
            .reference
            .as_deref()
            .map(|r| format!("<a class='doc-ref' href='{}'>[dok]</a>", esc(r)))
            .unwrap_or_default();
        let _ = write!(
            rows,
            "<tr><td><a href='rule/{}.html'><code class='rule-id'>{}</code></a></td><td><span class='badge'>{}</span></td><td>{}</td><td class='muted'>({}) {}</td><td>{}</td></tr>",
            rule_file_stem(engine, id),
            esc(id),
            esc(engine),
            compact(agg.rows.len()),
            esc(&agg.example_display),
            esc(&truncate(&agg.explanation, 110)),
            dok,
        );
    }
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>Indeks pravil (zvukove zakony)</h1>\
         <p class='lede'>Obratny indeks sledov pravil: za vsako pravilo generatora — vse strany zapisov, na ktoryh pokazany kandidat prošel črěz njego. Motor <span class='badge'>proto</span> je praslovjansky pravilny stroj, <span class='badge'>konsensus</span> — medžuvětvovy konsensus s reparacijami; id pravila NE je unikatny črěz motory (napr. <code>liquid-metathesis</code>), zato indeks ključuje na paru motor+id.</p>\
         <p class='muted'>Strojevo čitatelna forma: <a href='rules.json'>rules.json</a> (\u{201e}motor:id\u{201c} → spis id zapisov). Stolpec „Priklad“ je objasnjenje kroka iz JEDNOGO zapisa (v skobkah) — tekst je specifičny za zapis, ne obča definicija pravila.</p>\
         <table class='wikitable'><thead><tr><th>Pravilo</th><th>Motor</th><th>Zapisov</th><th>Priklad</th><th>Dok.</th></tr></thead><tbody>{rows}</tbody></table></article>",
    );
    page("Indeks pravil (zvukove zakony)", &body, 0)
}

/// `rules.json`: `"<engine>:<id>"` → sorted deduped entry-id list, the
/// machine-queryable twin of the `rule/` pages (roots.json precedent).
fn rules_json(rule_index: &RuleIndex) -> String {
    let mut s = String::from("{\n");
    for (i, ((engine, id), agg)) in rule_index.iter().enumerate() {
        if i > 0 {
            s.push_str(",\n");
        }
        let mut ids: Vec<usize> = agg.rows.iter().map(|r| r.id).collect();
        ids.sort_unstable();
        ids.dedup();
        let list = ids
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let _ = write!(s, "  {}: [{}]", json_str(&rule_key(engine, id)), list);
    }
    s.push_str("\n}\n");
    s
}

/// One proto-lemma reflex page (issue #73b): the canonical (accent-folded)
/// reconstruction word with ALL proto-cache entries folding to it — genuine
/// homonyms and accent variants alike. The join is by word alone, so a
/// generated reflex cannot honestly be attributed to one homonym over
/// another: the linked entries are listed ONCE at page level, and every
/// entry in that list carries the inbound "(rekonstrukcija)" link (same
/// membership map — the audit invariant).
fn proto_page(
    _slug_key: &str,
    pg: &ProtoReflexPage,
    pi: &crate::dump::ProtoIndex,
    by_id: &std::collections::HashMap<usize, &SiteEntryMeta>,
) -> String {
    let title = format!("Praslovjanska lemma: *{}", pg.word);
    // Page-level reflex list: exactly the membership set.
    let mut reflexes = String::new();
    for id in &pg.entry_ids {
        let Some(m) = by_id.get(id) else { continue };
        let _ = write!(
            reflexes,
            "<li><a href='../entry/{}.html'><b>{}</b></a> <span class='muted'>{} — {}</span></li>",
            m.id,
            esc(&m.title),
            esc(&pos_code_label(&m.pos)),
            esc(&truncate(&m.gloss, 60)),
        );
    }
    let reflex_block = if reflexes.is_empty() {
        String::new()
    } else {
        format!(
            "<section><h2 id='refleksy'>Generovany refleks v Slovowiki</h2><ul class='compact-list'>{reflexes}</ul></section>"
        )
    };
    // Root back-link via a linked entry's own root page (the proto slug is
    // canonical and need not equal any root slug).
    let root_link = pg
        .entry_ids
        .iter()
        .find_map(|id| by_id.get(id).and_then(|m| ancestor_slug(m)))
        .map(|sl| {
            format!(
                "<a href='../root/{sl}.html'>← korenj-strana (vse zapisy pod tym korenjem)</a> · "
            )
        })
        .unwrap_or_default();
    let homonym_note = if pg.recons.len() > 1 {
        format!(
            "<p class='muted'>{} rekonstrukcije v proto-cache dělę tu formu (homonimy ili akcentne varianty) — refleksy vyše sųt povezane s formoju, ne s jednoj iz njih.</p>",
            pg.recons.len()
        )
    } else {
        String::new()
    };
    let mut order: Vec<usize> = pg.recons.clone();
    order.sort_by_key(|&i| (crate::orthography::ascii_skeleton(&pi.entries[i].word), i));
    let mut sections = String::new();
    for &i in &order {
        let e = &pi.entries[i];
        let mut info = String::new();
        let _ = write!(
            info,
            "<tr><th>Čęst rěči</th><td>{}</td></tr>",
            esc(&pos_code_label(&e.pos))
        );
        if !e.glosses.is_empty() {
            let _ = write!(
                info,
                "<tr><th>Glosy</th><td>{}</td></tr>",
                esc(&e.glosses.join("; "))
            );
        }
        // Sparse in the cache: render only when present, silently skip
        // otherwise.
        if let Some(sc) = e.stem_class.as_deref() {
            let _ = write!(
                info,
                "<tr><th>Osnova</th><td><span class='muted'>{}</span></td></tr>",
                esc(sc)
            );
        }
        if !e.pbs.trim().is_empty() {
            let _ = write!(
                info,
                "<tr><th>Proto-baltoslovjansky</th><td><span class='mention'>{}</span></td></tr>",
                esc(e.pbs.trim())
            );
        }
        if !e.pie.trim().is_empty() {
            let _ = write!(
                info,
                "<tr><th>Praindoevropejsky</th><td><span class='mention'>{}</span></td></tr>",
                esc(e.pie.trim())
            );
        }
        // Attested descendants grouped by branch; codes outside the lang
        // registry (Baltic, non-Slavic IE comparanda) go under a muted
        // "ine/pročeje" group and display their raw code (the registry
        // name fallback would mislabel them "slovjansky").
        fn dname(code: &str) -> &str {
            crate::lang::lang_info(code).map(|i| i.name).unwrap_or(code)
        }
        let mut by_branch: BTreeMap<u8, Vec<(String, String)>> = BTreeMap::new();
        let mut historical_descendants: Vec<(String, String)> = Vec::new();
        for (code, form) in &e.descendants {
            if crate::lang::lang_info(code).is_some_and(|info| !info.modern) {
                historical_descendants.push((code.clone(), form.clone()));
                continue;
            }
            let key = match crate::corpus::branch_of(code) {
                Some(Branch::East) => 0u8,
                Some(Branch::West) => 1,
                Some(Branch::South) => 2,
                None => 3,
            };
            by_branch
                .entry(key)
                .or_default()
                .push((code.clone(), form.clone()));
        }
        let mut desc = String::new();
        for (key, label) in [
            (0u8, Branch::East.label()),
            (1, Branch::West.label()),
            (2, Branch::South.label()),
            (3, "ine/pročeje"),
        ] {
            let Some(items) = by_branch.get_mut(&key) else {
                continue;
            };
            items.sort_by(|a, b| dname(&a.0).cmp(dname(&b.0)).then_with(|| a.1.cmp(&b.1)));
            let muted = if key == 3 { " muted" } else { "" };
            let _ = write!(
                desc,
                "<div class='branch-box{muted}'><h4>{}</h4><table class='wikitable compact-table'><tbody>",
                esc(label)
            );
            for (code, form) in items.iter() {
                let _ = write!(
                    desc,
                    "<tr><td class='lc'>{}</td><td>{}</td></tr>",
                    esc(dname(code)),
                    esc(form),
                );
            }
            desc.push_str("</tbody></table></div>");
        }
        let desc_block = if desc.is_empty() {
            "<p class='muted'>Bez modernyh zapisanyh potomkov v proto-cache.</p>".to_string()
        } else {
            format!("<div class='branch-grid'>{desc}</div>")
        };
        historical_descendants
            .sort_by(|a, b| dname(&a.0).cmp(dname(&b.0)).then_with(|| a.1.cmp(&b.1)));
        let mut historical = String::new();
        if !historical_descendants.is_empty() {
            historical.push_str("<div class='proto-historical-hints'><h4>Historijske podskazky</h4><p class='muted'>Te potomky pomagajųt etimologiji, ale ne sųt moderne atestacije.</p><table class='wikitable compact-table'><tbody>");
            for (code, form) in &historical_descendants {
                let _ = write!(
                    historical,
                    "<tr><td class='lc'>{}</td><td>{}</td></tr>",
                    esc(dname(code)),
                    esc(form),
                );
            }
            historical.push_str("</tbody></table></div>");
        }
        let _ = write!(
            sections,
            "<section><h2 id='p{i}'><span class='mention'>*{}</span></h2>\
             <table class='wikitable compact-table'><tbody>{info}</tbody></table>\
             <h3>Atestovane moderne potomky (Wiktionary)</h3>{desc_block}{historical}</section>",
            esc(&e.word),
        );
    }
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>{}</h1>\
         <p class='lede'>Strana praslovjanskoj lemmy iz proto-cache: rekonstrukcija s glosami, dubjejšeju etimologijeju i atestovanymi potomkami po větvah — i generovane medžuslovjanske refleksy na sajtu.</p>\
         <p>{root_link}<a href='../proto-index.html'>vse praslovjanske lemmy</a></p>\
         {reflex_block}{homonym_note}\
         {sections}\
         <p class='foot'>Rekonstrukcije i potomky: Wiktionary (CC BY-SA), en.wiktionary Reconstruction:Proto-Slavic.</p></article>",
        esc(&title),
    );
    page(&title, &body, 1)
}

/// The proto-index.html overview: every reconstruction with a reflex page,
/// sorted by word skeleton (homonyms each get a row, linking their shared
/// page). Written even without the proto cache (with a note), so hub/sidebar
/// links never dangle.
fn proto_index_page(
    proto_reflex: &ProtoReflexIndex,
    proto: Option<&crate::dump::ProtoIndex>,
) -> String {
    let mut rows_data: Vec<(&str, usize, usize)> = Vec::new(); // (slug, proto idx, linked)
    for (sl, pg) in &proto_reflex.pages {
        for &i in &pg.recons {
            rows_data.push((sl.as_str(), i, pg.entry_ids.len()));
        }
    }
    let body = match proto {
        Some(pi) => {
            rows_data.sort_by_key(|&(_, i, _)| {
                (
                    crate::orthography::ascii_skeleton(&pi.entries[i].word),
                    i,
                )
            });
            let mut rows = String::new();
            for (sl, i, linked) in &rows_data {
                let e = &pi.entries[*i];
                let _ = write!(
                    rows,
                    "<tr><td><a href='proto/{sl}.html'><span class='mention'>*{}</span></a></td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    esc(&e.word),
                    esc(&truncate(&e.glosses.join("; "), 60)),
                    e.descendants.len(),
                    linked,
                );
            }
            format!(
                "<article class='entry'><h1 class='firstHeading'>Praslovjanske lemmy (refleksy)</h1>\
                 <p class='lede'>Vse praslovjanske rekonstrukcije iz proto-cache, ktore imajųt generovany medžuslovjansky refleks na sajtu: {} rekonstrukcij na {} stranah. Vsaka strana pokazyvaje glosy, dubjejšu etimologiju, atestovane potomky po větvah i linky na generovane refleksy; homonimne rekonstrukcije dělę jednu stranu.</p>\
                 <table class='wikitable'><thead><tr><th>Rekonstrukcija</th><th>Glosa</th><th>Potomkov</th><th>Refleksov na straně</th></tr></thead><tbody>{rows}</tbody></table></article>",
                compact(rows_data.len()),
                compact(proto_reflex.pages.len()),
            )
        }
        None => "<article class='entry'><h1 class='firstHeading'>Praslovjanske lemmy (refleksy)</h1>\
                 <p class='muted'>Proto-cache ne je nakladeny v tutoj gradbě (<code>data/proto-slavic.cache.json</code>) — pusti <code>extract-proto</code> i eksportuj znova.</p></article>"
            .to_string(),
    };
    page("Praslovjanske lemmy (refleksy)", &body, 0)
}

/// One rendered derivation-table row (issue #73d), recorded by
/// [`derivation_block`] EXACTLY as rendered — same `derive_family` inputs,
/// same `isv_to_id` resolution — so the deriv/ browse can never drift from
/// the entry pages.
struct DerivRow {
    base_id: usize,
    base: String,
    form: String,
    derived_entry_id: Option<usize>,
    /// Whether the base is an attested official headword (else a machine
    /// reconstruction, marked as such on the pattern page).
    official: bool,
}

/// All rendered rows of one derivation pattern plus its human label (the
/// label is constant per pattern; carried here so the page header needs no
/// second derivation pass).
struct DerivAgg {
    label: &'static str,
    rows: Vec<DerivRow>,
}

/// Write the deriv/ pattern pages + the derivations.html overview; returns
/// the total row count for the export's console report.
fn write_deriv_pages(
    out_dir: &Path,
    deriv_rows: &BTreeMap<&'static str, DerivAgg>,
    probs: &crate::derive::DerivationProbabilities,
) -> Result<usize> {
    let mut total = 0usize;
    for (pattern, agg) in deriv_rows {
        total += agg.rows.len();
        std::fs::write(
            out_dir.join("deriv").join(format!("{pattern}.html")),
            deriv_page(pattern, agg, probs.probability(pattern)),
        )?;
    }
    std::fs::write(
        out_dir.join("derivations.html"),
        derivations_index_page(deriv_rows, probs),
    )?;
    Ok(total)
}

/// The tooltip explaining what the per-pattern Wilson-95 probability is and
/// what it gates (shared by the pattern pages and the overview).
const DERIV_P_TITLE: &str = "Wilson-95: dolnja granica 95% intervala točnosti togo obrazca na odloženyh oficialnyh parah — ta že věrojętnosť bramkuje mašinove predlogy odvodženj v formovom API (api/forms)";

/// One derivational pattern's page: label + probability header, then every
/// rendered base → derivative row.
fn deriv_page(pattern: &str, agg: &DerivAgg, p: f64) -> String {
    let mut rows_sorted: Vec<&DerivRow> = agg.rows.iter().collect();
    rows_sorted.sort_by(|a, b| {
        crate::orthography::ascii_skeleton(&a.base)
            .cmp(&crate::orthography::ascii_skeleton(&b.base))
            .then_with(|| a.form.cmp(&b.form))
            .then(a.base_id.cmp(&b.base_id))
    });
    let mut rows = String::new();
    for r in rows_sorted {
        let base_note = if r.official {
            ""
        } else {
            " <span class='muted'>(rekonstrukcija)</span>"
        };
        let odvod = match r.derived_entry_id {
            Some(id) => format!(
                "<a href='../entry/{id}.html'><span class='mention'>{}</span></a>",
                esc(&r.form)
            ),
            None => format!("<span class='mention muted'>{}</span>", esc(&r.form)),
        };
        let _ = write!(
            rows,
            "<tr><td><a href='../entry/{}.html'><b>{}</b></a>{base_note}</td><td>{odvod}</td></tr>",
            r.base_id,
            esc(&r.base),
        );
    }
    let title = format!("Odvodženje: {} (-{pattern})", agg.label);
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>{}</h1>\
         <p class='lede'><code>{}</code> — {} pokazanyh odvodženj. <b title='{}'>Wilson-95 p≈{p:.2}</b>.</p>\
         <p class='muted'>Odvody bez linku (prigašene) sųt pravilno tvorjene formy, ktoryh něma v naslovnom množstvě sajta. Kȯgda baza je oficialna lemma, taky odvod je dostupny v formovom API kako p-bramkovany predlog (vidi <a href='../datasets.html'>Fajly za dostavanje</a>); kȯgda baza je označena „(rekonstrukcija)“, odvodženja sųt hypotetične — baza sama je mašinova rekonstrukcija — i NE sųt v API.</p>\
         <p><a href='../derivations.html'>← vse obrazcy</a></p>\
         <table class='wikitable'><thead><tr><th>Baza</th><th>Odvod</th></tr></thead><tbody>{rows}</tbody></table></article>",
        esc(&title),
        esc(pattern),
        agg.rows.len(),
        DERIV_P_TITLE,
    );
    page(&title, &body, 1)
}

/// The derivations.html overview: pattern | label | rendered-row count | p,
/// sorted by count desc then pattern id.
fn derivations_index_page(
    deriv_rows: &BTreeMap<&'static str, DerivAgg>,
    probs: &crate::derive::DerivationProbabilities,
) -> String {
    let mut items: Vec<(&&'static str, &DerivAgg)> = deriv_rows.iter().collect();
    items.sort_by(|a, b| {
        b.1.rows
            .len()
            .cmp(&a.1.rows.len())
            .then_with(|| a.0.cmp(b.0))
    });
    let mut rows = String::new();
    for (pattern, agg) in items {
        let _ = write!(
            rows,
            "<tr><td><a href='deriv/{pattern}.html'><code>{pattern}</code></a></td><td>{}</td><td>{}</td><td title='{}'>p≈{:.2}</td></tr>",
            esc(agg.label),
            compact(agg.rows.len()),
            DERIV_P_TITLE,
            probs.probability(pattern),
        );
    }
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>Odvodženja po sufiksah</h1>\
         <p class='lede'>Prěgled pravilnogo slovotvorstva: za vsaky obrazec — vse pary baza → odvod, ktore strany zapisov pokazyvajųt v bloku „Pravilne odvodženja“. Věrojętnosť p je ta sama Wilson-95 ocěna, ktora bramkuje mašinove predlogy v formovom API.</p>\
         <table class='wikitable'><thead><tr><th>Obrazec</th><th>Nazva</th><th>Parov</th><th>p</th></tr></thead><tbody>{rows}</tbody></table></article>",
    );
    page("Odvodženja po sufiksah", &body, 0)
}

/// One novel-vocabulary proposal (a generated word with no official match).
struct ProposalRow {
    id: usize,
    form: String,
    pos: String,
    prob: f64,
    ancestor: String,
    n_langs: usize,
    n_branches: usize,
    /// Attesting language codes (sorted, deduped) for the razumlivost column
    /// (issue #79). Display-only; NOT written to novel-words.tsv.
    langs: Vec<String>,
    gloss: String,
}

/// The Predloženja page: ranked novel-word proposals with the calibrated
/// probability, evidence summary and curation notes. The full list is in
/// data/novel-words.tsv; the page shows the propose bucket plus counts.
fn proposals_page(
    proposals: &[ProposalRow],
    calibration: Option<&crate::calibrate::Calibration>,
    curation: &std::collections::HashMap<String, String>,
) -> String {
    let propose_t = crate::calibrate::PROPOSE_T;
    let review_t = crate::calibrate::REVIEW_T;
    let n_propose = proposals.iter().filter(|r| r.prob >= propose_t).count();
    let n_review = proposals.len() - n_propose;
    let mut rows = String::new();
    for r in proposals.iter().filter(|r| r.prob >= propose_t).take(600) {
        // Curation-note keys follow the site-wide convention: standard
        // orthography, lowercase (see data/curation-notes.example.json).
        let note = curation
            .get(&crate::orthography::to_standard(&r.form.to_lowercase()))
            .or_else(|| curation.get(&r.form))
            .or_else(|| curation.get(&r.id.to_string()))
            .map(|n| format!(" <span class='muted' title='{}'>[nota]</span>", esc(n)))
            .unwrap_or_default();
        let _ = write!(
            rows,
            "<tr><td><a href='entry/{}.html'>{}</a>{}</td><td>{}</td><td>{:.2}</td><td class='score'>{}%</td><td class='mention'>{}</td><td>{} / {}</td><td>{}</td></tr>",
            r.id,
            esc(&r.form),
            note,
            esc(&r.pos),
            r.prob,
            razum_pct(&r.langs),
            esc(&r.ancestor),
            r.n_langs,
            r.n_branches,
            esc(&truncate(&r.gloss, 90)),
        );
    }
    let cal_note = match calibration {
        Some(c) => format!(
            "Věrojetnost je <b>izotonično kalibrovana</b> (naučena na razvojnoj čęsti benchmarka, prověrjena na odloženoj: ECE {:.3}) — čitaj ju kako <i>P(slovo by sovpalo s oficialnym rěšenjem)</i>. Pragy sųt izměrjene operacijne točky (na odloženoj četvrtině): predlog p≥{propose_t:.1} ({:.1}% točnost / {:.1}% pokrytje), pregled p≥{review_t:.1} ({:.1}% / {:.1}%).",
            c.holdout_ece,
            100.0 * c.propose_pr.0,
            100.0 * c.propose_pr.1,
            100.0 * c.review_pr.0,
            100.0 * c.review_pr.1,
        ),
        None => "Kalibracija za ocěnu pokryća korpusa ne jest dostupna. Predloženja i věrojętnosti sųt časovo izključene, da ne priměnjajemo kalibraciju drugogo modela (issue #89 J26).".to_string(),
    };
    let summary = if calibration.is_some() {
        format!(
            "<b>{n_propose}</b> predloženj (p≥{propose_t:.1}) + <b>{n_review}</b> k pregledu (p≥{review_t:.1}); polny spisok: <a href='novel-words.tsv'>novel-words.tsv</a>."
        )
    } else {
        "Spisok jest prazdny do holdout-validovanoj kalibracije ocěny pokryća korpusa; <a href='novel-words.tsv'>novel-words.tsv</a> zato sadrži samo zaglavje.".to_string()
    };
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>Predloženja novyh slov</h1>\
         <p class='lede'>Slova, ktore stroj pravilno izvodi iz slovjanskogo dokaza, ale ktoryh <b>něma</b> v oficialnom slovniku — kandidaty za novu leksiku.</p>\
         <p>{cal_note}</p>\
         <p>{summary} Kuratorske noty prihodęt iz <code>data/curation-notes.json</code>.</p>\
         <table class='wikitable'><thead><tr><th>slovo</th><th>vrsta</th><th>p</th><th title='{razum_title}'>razumlivosť</th><th>prědok</th><th>językov / větvi</th><th>značenje</th></tr></thead><tbody>{rows}</tbody></table>\
         <p class='muted'>Pokazano najviše 600 predlogov; polny spisok v TSV. Mašinove rekonstrukcije, ne normativna leksika.</p></article>",
        razum_title = RAZUM_TITLE,
    );
    page("Predloženja novyh slov — medžuslovjansky", &body, 0)
}

/// Shared client-side JS for the form index: the fold mirrors
/// `orthography::to_standard` and the router mirrors `forms::fnv1a32` —
/// changing either side is a schema break (see forms.rs).
/// Minimal query-string encoder for `forms.html?q=` links (space and the few
/// HTML-hostile characters; non-ASCII letters are legal in query strings).
fn urlencode_q(s: &str) -> String {
    s.replace('%', "%25")
        .replace(' ', "%20")
        .replace('&', "%26")
        .replace('\'', "%27")
        .replace('"', "%22")
}

fn forms_js() -> String {
    const JS: &str = r#"
function escHtml(s){return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;').replace(/'/g,'&#39;');}
function isvFold(s){s=s.toLowerCase().trim();const M=__FOLD_MAP__;let out='';for(const c of s){out+=(M[c]!==undefined)?M[c]:c;}return out;}
let routerOk=null;
async function routerSelftest(base){if(routerOk!==null)return routerOk;try{const j=await fetch(base+'api/router-selftest.json').then(r=>r.json());routerOk=j.shards===__SHARDS__&&j.samples.every(([form,key,shard])=>isvFold(form)===key&&fnv1a32(key)%__SHARDS__===shard);}catch(e){routerOk=false;}if(!routerOk){console.error('router selftest FAILED: JS fold/router drifted from the exporter');}return routerOk;}
function fnv1a32(s){const b=new TextEncoder().encode(s);let h=0x811c9dc5>>>0;for(const x of b){h^=x;h=Math.imul(h,16777619)>>>0;}return h>>>0;}
const shardCache={};
async function isvShard(base,n){if(shardCache[n])return shardCache[n];shardCache[n]=fetch(base+'api/forms/'+n+'.json').then(r=>r.ok?r.json():{records:{}}).catch(()=>({records:{}}));return shardCache[n];}
async function isvLookup(base,q){const ok=await routerSelftest(base);const key=isvFold(q);if(!ok){return{key:key,recs:[],selftestFailed:true};}const shard=fnv1a32(key)%__SHARDS__;const j=await isvShard(base,shard);return{key:key,recs:(j.records&&j.records[key])||[]};}
function recHtml(base,rec){const[form,lemma,id,pos,analyses,,status,prob,gloss]=rec;
 const st=status==='generated'?('<span class="pill">mašinova rekonstrukcija p='+(prob==null?'?':prob.toFixed(2))+'</span>'):('<span class="pill src-official">'+escHtml(status)+'</span>');
 const an=analyses.length?('<span class="muted">'+escHtml(analyses.join(', '))+'</span>'):'<span class="muted">(citatna forma)</span>';
 return '<li><b>'+escHtml(form)+'</b> — <a href="'+base+'entry/'+id+'.html">'+escHtml(lemma)+'</a> <span class="badge pos">'+escHtml(pos)+'</span> '+an+' '+st+' <span class="muted">'+escHtml(gloss)+'</span></li>';}
"#;
    let fold_map = crate::forms::FOLD_PAIRS
        .iter()
        .map(|(from, to)| format!("'{from}':'{to}'"))
        .collect::<Vec<_>>()
        .join(",");
    JS.replace("__SHARDS__", &crate::forms::SHARDS.to_string())
        .replace("__FOLD_MAP__", &format!("{{{fold_map}}}"))
}

/// The reverse-lookup page for surface forms (issue #11 phase 2): folds the
/// query, routes to the shard client-side, renders every analysis.
fn forms_page() -> String {
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>Iskanje form</h1>\
         <p class='lede'>Vpiši kojukoli <b>fleksijnu formu</b> (ne tolika lemmu) — na priklad <span class='mention'>pomoćnogo</span>, <span class='mention'>ljudi</span>, <span class='mention'>piše</span> — i vidiš vse analizy: lemmu, padež/čislo/rod, i stranicu zapisa.</p>\
         <p><input id='formq' type='search' placeholder='forma…' style='min-width:16em' onkeydown='if(event.key===String.fromCharCode(69,110,116,101,114))go()'> <button onclick='go()'>Iskaj</button></p>\
         <div id='out'></div>\
         <p class='muted'>Iste dane služęt strojam: <code>api/forms/&lt;n&gt;.json</code> (indeks razděljeny na {} častij), <code>api/lemmas.json</code>, <code>api/meta.json</code>, <a href='api/agent-guide.md'>api/agent-guide.md</a>.</p>\
         <script>{}\
async function go(){{const q=document.getElementById('formq').value;if(!q)return;const r=await isvLookup('',q);const out=document.getElementById('out');\
if(r.selftestFailed){{out.innerHTML='<p class=\"notice\">Samoprověrka routera ne prošla — klient sę ne shoduje s eksporterom (vidi konzolų). Iskanje je zaprěno da ne davaje krive rezultaty.</p>';return;}}\
if(!r.recs.length){{out.innerHTML='<p>Ničto ne najdeno za ključ <b>'+escHtml(r.key)+'</b>. (Nepoznata forma ili mašinovo prědloženje bez zapisa.)</p>';return;}}\
out.innerHTML='<p>Ključ: <b>'+escHtml(r.key)+'</b>, '+r.recs.length+' analiz:</p><ul>'+r.recs.map(x=>recHtml('',x)).join('')+'</ul>';}}\
const p=new URLSearchParams(location.search).get('q');if(p){{document.getElementById('formq').value=p;go();}}\
</script></article>",
        crate::forms::SHARDS,
        forms_js(),
    );
    page("Iskanje form — medžuslovjansky", &body, 0)
}

/// Client-side text verification (issue #11 phase 3): the static twin of the
/// `check-text` CLI. Same tokenizer contract (internal hyphens kept, general
/// two-token lookup so reflexive `sę` verbs and multi-word official lemmas
/// resolve), same semantic-trap notes (fetched from `api/notes.json`); the
/// CLI additionally offers nearest-lemma suggestions for unknown tokens.
fn text_check_page() -> String {
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>Prověrka teksta</h1>\
         <p class='lede'>Vstavi medžuslovjansky tekst — vsaky token bųde prověrjeny v slovniku i v polnom indeksu form. Sinje = poznato, žėlta obvodka = mašinova rekonstrukcija, čŕveno = nepoznato, ⚠ = semantična past.</p>\
         <p><textarea id='t' rows='6' style='width:100%'></textarea></p>\
         <p><button onclick='checkText()'>Prověri</button> <span class='muted'>CLI-blizenec: <code>cargo run -- check-text tekst.txt --json</code> (dodatno davaje predloženja za nepoznate tokeny).</span></p>\
         <div id='out'></div>\
         <script>{}\
let notes=null;\
async function getNotes(){{if(notes)return notes;notes=fetch('api/notes.json').then(r=>r.ok?r.json():{{}}).catch(()=>({{}}));return notes;}}\
async function checkText(){{\
const text=document.getElementById('t').value;\
const toks=text.match(/\\p{{L}}+(?:-\\p{{L}}+)*/gu)||[];\
const out=document.getElementById('out');out.innerHTML='<p>Prověrjanje…</p>';\
const nts=await getNotes();\
const parts=[];let i=0;\
while(i<toks.length){{\
 const tok=toks[i];\
 if(i+1<toks.length){{const bi=await isvLookup('',tok+' '+toks[i+1]);if(bi.selftestFailed){{out.innerHTML='<p class=\"notice\">Samoprověrka routera ne prošla — prověrka je zaprěna (vidi konzolų).</p>';return;}}if(bi.recs.length){{parts.push(render(tok+' '+toks[i+1],bi.recs,nts,bi.key));i+=2;continue;}}}}\
 const r=await isvLookup('',tok);if(r.selftestFailed){{out.innerHTML='<p class=\"notice\">Samoprověrka routera ne prošla — prověrka je zaprěna (vidi konzolų).</p>';return;}}parts.push(render(tok,r.recs,nts,r.key));i+=1;\
}}\
out.innerHTML='<p>'+parts.join(' ')+'</p><p class='+String.fromCharCode(39)+'muted'+String.fromCharCode(39)+'>Klikni slovo za polnu analizu.</p>';\
}}\
function render(tok,recs,nts,key){{\
 const note=nts&&nts[key];\
 if(!recs.length)return '<a class=\"chip redlink\" href=\"forms.html?q='+encodeURIComponent(tok)+'\" title=\"nepoznato\">'+escHtml(tok)+'</a>';\
 const gen=recs.every(r=>r[6]==='generated');\
 let ttl=gen?('mašinova rekonstrukcija p='+(recs[0][7]==null?'?':recs[0][7].toFixed(2))):recs.map(r=>r[1]+' ('+(r[4].join(', ')||'lemma')+')').slice(0,4).join('; ');\
 if(note)ttl='⚠ '+note.warning+(note.prefer&&note.prefer.length?' Prefer: '+note.prefer.join(', ')+'.':'')+' — '+ttl;\
 const style=gen?' style=\"border-color:#c90\"':(note?' style=\"border-color:#c33\"':'');\
 return '<a class=\"chip xref\" href=\"forms.html?q='+encodeURIComponent(tok)+'\" title=\"'+escHtml(ttl)+'\"'+style+'>'+(note?'⚠':'')+escHtml(tok)+'</a>';\
}}\
</script></article>",
        forms_js(),
    );
    page("Prověrka teksta — medžuslovjansky", &body, 0)
}

fn datasets_page(coverage: &str) -> String {
    let body = format!("<article class='entry'><h1 class='firstHeading'>Fajly za dostavanje</h1><p class='lede'>Statične JSON fajly za raziskovanje i ponovno upotrěbljenje.</p><table class='wikitable'><tr><th>Fajl</th><th>Opis</th></tr><tr><td><a href='entries.json'>entries.json</a></td><td>Metadany zapisa: id, naslov, smysl, čęst rěči, uvěrjenost (kalibrovany kȯšik), <code>prob</code> = modelovo-specifična kalibrovana věrojętnosť (null bez sovmestimoj kalibracije i za oficialne/surove zapisy), <code>official_id</code> = id smysla v izvornom oficialnom slovniku (null za neoficialne), prědȯk, <code>langs_list</code> = sortovany spis kodov atestujučih językov i <code>branch_pattern</code> = vzorec větvi (V/Z/J kombinacija, null bez větvi), <code>aspect</code> i <code>aspect_partners</code> za glagoly — vsako zapytanje po vzorcu atestacije je jedna jq-linija (issues #73, #75).</td></tr><tr><td><a href='edges.json'>edges.json</a></td><td>Vęzi semantičnogo grafa.</td></tr><tr><td><a href='categories.json'>categories.json</a></td><td>Členstvo v kategorijah.</td></tr><tr><td><a href='roots.json'>roots.json</a></td><td>Členstvo v praslovjanskyh korenjah.</td></tr><tr><td><a href='rules.json'>rules.json</a></td><td>Obratny indeks pravil: \u{201e}motor:id-pravila\u{201c} (motor = proto ili konsensus — id pravila ne je unikatny črěz motory) → spis id zapisov, ktoryh pokazany kandidat koristil to pravilo (vidi <a href='rules.html'>indeks pravil</a>; issue #73).</td></tr><tr><td><a href='search/manifest.json'>search/manifest.json</a></td><td>Klientsky indeks iskanja: manifest + razděly po prvoj bukvě (search/*.json; vidi #71).</td></tr><tr><td><a href='novel-words.tsv'>novel-words.tsv</a></td><td>Predloženja novyh slov; samo zaglavje dokolě korpusny model ne imaje vlastnu holdout-validovanu kalibraciju.</td></tr><tr><td><a href='api/meta.json'>api/meta.json</a></td><td>Leksikalny API za stroje: šema, ličby, licencija, routing indeksa.</td></tr><tr><td><a href='api/lemmas.json'>api/lemmas.json</a></td><td>Vse lemmy s statusom, opcionalnoju modelovo-specifičnoju věrojetnostju i vidovymi partnerami glagolov (schema 3).</td></tr><tr><td><a href='api/aspect-pairs.json'>api/aspect-pairs.json</a></td><td>Produkcijny model glagolskyh par: oficialne i generovane ipf↔pf formy, stranice i pravilo.</td></tr><tr><td>api/forms/&lt;n&gt;.json</td><td>Fleksijny indeks (razděljeny; vidi <a href='api/agent-guide.md'>agent-guide.md</a> i <a href='forms.html'>Iskanje form</a>).</td></tr><tr><td><a href='build.json'>build.json</a></td><td>Metadany aktualnoj gradby (git, ličby).</td></tr></table>{coverage}</article>");
    page("Fajly za dostavanje", &body, 0)
}

/// The dataset-coverage block on `datasets.html` (issue #35): documents exactly
/// which Slavic-Wiktionary datasets feed the site and the inclusion/exclusion
/// counts. `stats` is the deterministic extraction tally; `rendered`/`deduped` are
/// the site-level split from the raw loop. All numbers regenerate on export.
fn datasets_coverage_section(
    stats: Option<&crate::dump::RawCoverageStats>,
    rendered: usize,
    deduped: usize,
    generated: usize,
    official_only: usize,
) -> String {
    let mut s = String::new();
    s.push_str("<h2 id='pokrytje'>Pokrytje slovjanskyh datasetov</h2>");
    s.push_str("<p class='lede'>Čto znači „vse slovjanske Wiktionary dataset-y“: srovy tok iz anglijskoga Wiktextract-a (jednoslovne polnoznačne slova) + nativne ru/pl/cs izdanja za obogaćenje. Niže — koliko slov je vključeno i koliko izključeno, s pričinoju.</p>");
    if let Some(st) = stats {
        let seen = st.slavic_pages_seen.max(1);
        let pct = |x: u64| format!("{:.1}%", 100.0 * x as f64 / seen as f64);
        s.push_str("<table class='wikitable'><tr><th>Ekstrakcija (anglijsky dump)</th><th>Strany</th><th>Dělj</th></tr>");
        let _ = write!(
            s,
            "<tr><th>Slovjanske strany viděne</th><td>{}</td><td>100%</td></tr>",
            st.slavic_pages_seen
        );
        let _ = write!(
            s,
            "<tr><th>Zadŕžane (vključene)</th><td>{}</td><td>{}</td></tr>",
            st.kept,
            pct(st.kept)
        );
        let _ = write!(
            s,
            "<tr><th>Odbrošene — prěnapravjenje (bez smyslov)</th><td>{}</td><td>{}</td></tr>",
            st.dropped_redirect_no_senses,
            pct(st.dropped_redirect_no_senses)
        );
        let _ = write!(
            s,
            "<tr><th>Odbrošene — mnogoslovne / prazdne</th><td>{}</td><td>{}</td></tr>",
            st.dropped_multiword,
            pct(st.dropped_multiword)
        );
        let _ = write!(
            s,
            "<tr><th>Odbrošene — ne polnoznačna čęsť rěči</th><td>{}</td><td>{}</td></tr>",
            st.dropped_non_content_pos,
            pct(st.dropped_non_content_pos)
        );
        let _ = write!(
            s,
            "<tr><th>Odbrošene — bez pravoj definicije</th><td>{}</td><td>{}</td></tr>",
            st.dropped_no_real_gloss,
            pct(st.dropped_no_real_gloss)
        );
        s.push_str("</table>");
        let _ = write!(
            s,
            "<p class='muted'>Zadŕžane ({}) + odbrošene ({}) = viděne slovjanske strany ({}).</p>",
            st.kept,
            st.dropped_total(),
            st.slavic_pages_seen
        );
    } else {
        s.push_str("<p class='muted'>Statistika ekstrakcije ješče ne generovana (<code>data/raw-slavic-coverage.json</code>). Pokreni <code>extract-raw-slavic</code>.</p>");
    }
    s.push_str("<table class='wikitable'><tr><th>Na sajtu</th><th>Strany</th></tr>");
    let _ = write!(
        s,
        "<tr><th>Srove atestacije (samo surove, R)</th><td>{rendered}</td></tr>"
    );
    let _ = write!(
        s,
        "<tr><th>Surove dublikovane (uže pokryte)</th><td>{deduped}</td></tr>"
    );
    let _ = write!(
        s,
        "<tr><th>Generovane srodne strany</th><td>{generated}</td></tr>"
    );
    let _ = write!(
        s,
        "<tr><th>Samo oficialne strany</th><td>{official_only}</td></tr>"
    );
    s.push_str("</table>");
    s.push_str("<p class='muted'>Podrobny izvěst: <code>target/eval/raw-coverage.md</code> (komanda <code>coverage</code>).</p>");
    s
}

fn build_json(build: &BuildMeta) -> String {
    format!(
        "{{\n  \"generated\": {},\n  \"git\": {},\n  \"entries\": {},\n  \"lemmas\": {}\n}}\n",
        json_str(&build.generated),
        json_str(&build.git),
        build.total_entries,
        build.lemma_total
    )
}

fn sitemap_xml(metas: &[SiteEntryMeta]) -> String {
    let mut s = String::from("<?xml version='1.0' encoding='UTF-8'?>\n<urlset xmlns='http://www.sitemaps.org/schemas/sitemap/0.9'>\n");
    for loc in [
        "index.html",
        "search.html",
        "all-pages.html",
        "categories.html",
        "portals.html",
        "indices.html",
        "site-stats.html",
        "needs-review.html",
        "borrowings.html",
        "special.html",
        "datasets.html",
        "rules.html",
        "proto-index.html",
        "derivations.html",
        "suffix-index.html",
        "inflection-issues.html",
        "featured.html",
        "random.html",
        "graph.html",
        "contribute.html",
    ] {
        let _ = write!(s, "  <url><loc>{}{}</loc></url>\n", SITE_URL, loc);
    }
    for m in metas {
        let _ = write!(
            s,
            "  <url><loc>{}entry/{}.html</loc></url>\n",
            SITE_URL, m.id
        );
    }
    s.push_str("</urlset>\n");
    s
}

/// A full explainer of every accuracy statistic tracked against the official
/// dictionary. Mostly static content; the confidence-calibration section is
/// rendered live from the committed calibrator (issue #77) so it can never
/// drift from data/score-calibration.json.
fn metrics_page(cal: Option<&crate::calibrate::Calibration>) -> String {
    let head = r##"<article class='entry metrics'>
  <h1 class='firstHeading'>Statistiky točnosti</h1>
  <p class='lede'>Ta strana objasnjaje <b>vsaku statistiku</b>, ktoru měrimo, da bismo proverili točnosť generatora protiv oficialnogo medžuslovjanskogo slovnika. Čisla sųt aktualne měrjenja produkcijnoj konfiguracije; vsaky artefakt sę regeneruje v <code>target/eval/</code>.</p>

  <h2 id='setup'>Kako radi testovo množstvo</h2>
  <p>Za vsaky smysl (16&nbsp;300 jednoslovnyh zapisov) generator dostaje <b>moderne slovjanske srodne slova</b> + časť rěči, rod i priznak internacionalizma (<code>genesis</code>) — ale <b>nikȯgda</b> oficialnu medžuslovjansku formu (<code>isv</code>). On rekonstruuje lemmu, a my ju sravnjajemo s oficialnoju. Tako testovo množstvo je <b>bez utečki</b> ględe formy. Komanda: <code>evaluate</code>.</p>

  <h2 id='pravopis'>Dva pravopisa: točno protiv normalizovano</h2>
  <p>Medžuslovjansky imaje dva pravopisa. <b>Naučny (variantny)</b> dŕži etimologične znaky (ě, ę, ų, å, ȯ, ć, đ, y, mękke ĺ&nbsp;ń&nbsp;ŕ). <b>Standardny</b> jih složaje: ě→e, ę→e, ų→u, å→a, ȯ→o, ć→č, đ→dž. Zato imamo dva urovni sovpadenja — strogo (variantno) i normalizovano.</p>

  <h2 id='osnovne'>Osnovne měrky sovpadenja (evaluate)</h2>
  <table class='wikitable'>
    <thead><tr><th>Statistika</th><th>Aktualno</th><th>Značenje</th></tr></thead>
    <tbody>
    <tr><td><b>točno pŕvy izbor</b> (povno)</td><td>41,65%</td><td>Prědvidženje je <b>identično</b> oficialnoj variantnoj lemmě, znak-v-znak.</td></tr>
    <tr><td><b>normalizovano — pŕvy izbor</b></td><td>49,59%</td><td>Identično <b>po složenju</b> oběh v standardny alfavit (ě=e, ć=č…). Glavna měrka i porog stalnoj integracije.</td></tr>
    <tr><td>skelet pŕvy izbor</td><td>—</td><td>Identično po agresivnom ASCII-složenju (bez diakritiky, složene sibilanty). Najslabějše sito.</td></tr>
    <tr><td><b>normalizovano pŕve 3 / pŕve 5</b></td><td>60,48% / 63,12%</td><td>Nekotory od prvyh 3 / 5 rangovanyh kandidatov sovpadaje (normalizovano).</td></tr>
    <tr><td><b>srědnja pravopisna distancija</b></td><td>0,224</td><td>Srědnja normalizovana Levenshtein-distancija (0 = identično, 1 = vpolno različno).</td></tr>
    </tbody>
  </table>

  <h2 id='ladder'>Lěstvica odstranjenja</h2>
  <p>Točnosť raste od <b>osnovy</b> (27,52% točno — prvobytny prototip) do <b>produkcije</b> (41,65%). Vsaky stųpenj dodavaje <b>točno jedno</b> pravilo, tako že jego dělta je pripisiva. Pravila, ktore izměrjeno <b>uhudšajųt</b> točnosť, sųt odbrošene i zapisane kako „odbrošene eksperimenty“. Polny izvěsť: <code>candidate-generation-report.md</code>.</p>

  <h2 id='razbivka'>Děljeńje po kategorijah</h2>
  <ul>
    <li><b>Po čęsti rěči</b> — točnosť za imenniky, glagoly, pridavniky, čislovniky itd. odděljeno.</li>
    <li><b>Po pokrytju větvi</b> — koliko od trěh větvi (iztok / zapad / jug) potvŕđaje formu; više pokrytja = viša točnosť.</li>
    <li><b>Po věrodostojnosti</b> — vidi niže.</li>
  </ul>
"##;
    // Live confidence-calibration section: fitted provenance, holdout ECE and
    // both measured operating points come straight from the persisted struct.
    let mut calib = String::from("<h2 id='kalibracija'>Kalibracija věrodostojnosti</h2>\n");
    match cal {
        Some(c) => {
            let _ = write!(
                calib,
                "  <p>Znak uvěrjenosti jest kȯšik izotonično kalibrovanoj věrojętnosti <i>P(sovpadenja s oficialnoju lemmoju)</i> za tutočny score-domain, ne syrovoj ocěny.</p>\n  <table class='wikitable'><tbody>\n  <tr><th>Naučeno na</th><td>{}</td></tr>\n  <tr><th>ECE na odloženoj četvrtině</th><td>{:.3}</td></tr>\n  <tr><th>predlog p≥{:.1}</th><td>{:.1}% točnost / {:.1}% pokrytje</td></tr>\n  <tr><th>pregled p≥{:.1}</th><td>{:.1}% točnost / {:.1}% pokrytje</td></tr>\n  </tbody></table>\n",
                esc(&c.fitted_on),
                c.holdout_ece,
                crate::calibrate::PROPOSE_T,
                100.0 * c.propose_pr.0,
                100.0 * c.propose_pr.1,
                crate::calibrate::REVIEW_T,
                100.0 * c.review_pr.0,
                100.0 * c.review_pr.1,
            );
        }
        None => calib.push_str(
            "  <p>Sovmestimaja kalibracija ne dostupna za tutu modelovu ocěnu; syrovy rang/znak uvěrjenosti ne jest věrojętnosť.</p>\n",
        ),
    }
    calib.push_str("  <p>Podrobna kalibracija oficialno-redkovogo modela (decilna tablica, ECE i Brier) je v <code>methodology.md</code>. Korpusny model potrěbuje svoju vlastnu holdout-validovanu kalibraciju prěd publikovanjem věrojętnostij ili predloženj.</p>\n");
    let tail = r##"
  <h2 id='corpus'>Sajtovy pųť (corpus-eval)</h2>
  <p>Sajt koristi ne glavny proces, a svoj <b>put srodnyh množin</b> (<code>corpus::generate_set</code>), měrjeny odděljeno: <b>58,31% točno / 62,84% normalizovano</b> na 7&nbsp;398 zapisah s znanym prědkom. Više od glavne linije, potomu što ocěnjaje tȯlko slova, ktore sajt izvodi iz znanogo prědka. Komanda: <code>corpus-eval</code>.</p>

  <h2 id='proto'>Praslovjansky stroj (proto-eval)</h2>
  <p>Praslovjansky pravilny stroj izměrjeny izolovano od povęzanja, ranga i konsensusa:</p>
  <ul>
    <li><b>pokrytosť povęzanja</b>: <b>20,1%</b> smyslov je pouzdano povęzano s rekonstrukcijeju.</li>
    <li><b>točnosť na povęzanyh</b>: <b>46,68% točno / 52,74% normalizovano</b>.</li>
  </ul>
  <p>Komanda: <code>proto-eval</code>.</p>

  <h2 id='audit'>Analiza grěšek (prověrka)</h2>
  <ul>
    <li><b>Tri klasy grěšek</b>: <i>križna grupa</i> (~48% — oficialny korenj je v dokazě, ale izbran drugy), <i>prava grupa–kriva forma</i> (~30%), <i>korenj otsutny</i> (~21% — oficialnogo korenja net v srodnyh slovah).</li>
    <li><b>Histogram pripisanja stupnjam</b>: prěigrivaje sled pravil pobědnika i pripisyvaje grěšku stųpnju, ktora izgubila odgovor — grupa/glas ~33%, sľanje/rang ~22%, korenj-otsutny ~22%, normalizacija/prědstavitelj ~15%, zakončenja ~6%, praslovjansky stroj ~1,6%. Vidi <code>stage-attribution.md</code>.</li>
    <li><b>Kohezija</b>: koliko različnyh srodnyh grup imaje vsaky smysl (89,5% imaje ≥3).</li>
  </ul>
  <p>Komanda: <code>audit</code>.</p>

  <h2 id='oracle'>Diagnostične granice (idealny test)</h2>
  <p>Da izměriti <b>gorny prědel</b> vsake stupnje, dělajemo ju „idealnų“ (čitajų oficialny odgovor) dok vse niže ostaje realno. To <b>nikȯgda</b> ne ide v produkciju — samo pokazyvaje, gdě je vȯzstanovima greška.</p>
  <table class='wikitable'><thead><tr><th>Idealny stųpenj</th><th>Δ točno</th></tr></thead>
  <tbody><tr><td>izbor grupy</td><td>+4,5pp — glavno redakcijno, nedostižno slěpo</td></tr><tr><td>izbor prědstavitelja</td><td>+2,3pp (medoid uže vzęl +1,1pp)</td></tr><tr><td>proto-povęzanje</td><td>+2,7pp</td></tr><tr><td>vse trě zajedno</td><td>+9,4pp</td></tr></tbody></table>
  <p>Komanda: <code>oracle</code>.</p>

  <h2 id='probes'>Izbor grupy i prědstavitelja (select-eval / rep-eval)</h2>
  <p>Měrimo, koliko od gornih prědelov može vȯzstanoviti <b>pravilo bez utečki</b> (ne čitajuče odgovor):</p>
  <ul>
    <li><b>select-eval</b> (izbor grupy): vse slěpe pravila (najviše językov / větvi, internacionalizm-prvo) <b>uhudšajųt</b> — potvŕđaje, že križna grupa je redakcijna granica, ne programna greška.</li>
    <li><b>rep-eval</b> (izbor prědstavitelja): pravilo <b>medoid</b> (najcentralnějša forma, najmenša suma distancij do drugih) davaje <b>+1,09pp</b> i je uže v produkciji; ostaje ~+2,3pp do granice.</li>
  </ul>

  <h2 id='synonym'>Sinonimno-svěstna točnosť (synonym-eval)</h2>
  <p>Strogo testovo množstvo pytaje „sovpadaje li s <b>jedinoju</b> oficialnoju lemmoju?“, ale medžuslovjansky imaje mnogo validnyh slov na jedno značenje, a slovnik zapisuje samo jedno. Ta měrka pripisuje prědvidženju, ktore reproduktuje <b>kojukoli</b> oficialnu lemmu s tym že značenjem (iz sinonimnogo tezaurusa):</p>
  <table class='wikitable'><thead><tr><th>Měrka</th><th>pŕvy izbor</th></tr></thead>
  <tbody><tr><td>točno</td><td>41,65%</td></tr><tr><td>normalizovano (strogo)</td><td>49,59%</td></tr><tr><td><b>sinonimno-vključno</b></td><td><b>55,76%</b></td></tr></tbody></table>
  <p>Děljeńje strogih grěšek: <b>12,2% validny sinonim</b> (druga oficialna lemma, isto značenje), 7,9% druga oficialna lemma (drugo značenje), 79,8% ne-oficialna forma (nova ili prava greška — nerazlučima bez tezaurusa maternjego govoritelja). Komanda: <code>synonym-eval</code>.</p>

  <h2 id='artefakty'>Artefakty</h2>
  <p>Vse měrjenja sųt zapisane v <code>target/eval/</code>: <code>candidate-generation-report.md</code>, <code>stage-attribution.md</code>, <code>oracle-ladder.md</code>, <code>cluster-selection.md</code>, <code>rep-selection.md</code>, <code>synonym-accuracy.md</code>, <code>methodology.md</code> (razděl razvoj/kontrola bez prěučenja, značimosť stupnjev, bootstrap-intervaly, kalibracija), <code>predictions.csv</code> (vse prědvidženja). Vsaka je reproducibilna jednoju komandoju.</p>
</article>"##;
    let body = format!("{head}\n  {calib}{tail}");
    page("Statistiky točnosti — medžuslovjansky", &body, 0)
}

fn about_page(n: usize, norm_rate: f32, exact_rate: f32, top3: f32) -> String {
    let body = format!(
        "<article class='entry'>
           <h1>O metodě</h1>
           <p class='lede'>Toj slovnik ne je rųčno napisany — vsaka forma je <b>generovana</b> iz slovjanskyh dokazov i měrjena protiv oficialnogo medžuslovjanskogo slovnika.</p>

           <h2>Dvostupnjovy model</h2>
           <p>Za vsaky smysl:</p>
           <ol>
             <li><b>Konsensus izbira korenj.</b> Iz srodnyh slov v {langs} slovjanskyh językah glasujemo po <i>větvah</i> (izток / zapad / jug), da najveći język ne dominuje. Šest poddialektnyh grup s populacijnym vagom rěša, kotory korenj je najbolje medžuslovjansky.</li>
             <li><b>Praslovjansko pravilo davaje formu.</b> Kǫda smysl je bez utečki povezany s praslovjanskoju rekonstrukcijeju (*word) črěz naslědnikov + glosų, determinističny stroj izvodi formų s pravilnymi variantnymi znakami (ě, ć/đ, å, ȯ, y), kotoryh moderne refleksy ne mogųt vȯzstanoviti.</li>
           </ol>

           <h2>Točnost (měrjeno)</h2>
           <div class='statgrid'>
             <div class='stat ok'><div class='statnum'>{exact:.1}%</div><div class='statlbl'>povno točno</div></div>
             <div class='stat'><div class='statnum'>{norm:.1}%</div><div class='statlbl'>normalizovano — pŕvy izbor</div></div>
             <div class='stat'><div class='statnum'>{top3:.1}%</div><div class='statlbl'>pŕve 3</div></div>
           </div>
           <p class='muted'>Testovo množstvo: {n} zapisov s ≥2 modernymi srodnymi slovami. Generator nikǫda ne vidi oficialnų formų — jedino srodne slova + čęsť rěči + glosų — tako da měrjenje je bez propuščanja. Vsako pravilo je zadŕžano jedino ako je izměrjeno pobolšanje (lěstvica odstranjenja).</p>

           <h2>Poznaty prědel</h2>
           <p>Okolo 38% ostatnyh razlik sųt <i>redakcijne</i> izbory (medžuslovjansky komitet izbral menšinny korenj) kotore se ne mogųt vȯzstanoviti iz modernyh srodnyh slov. Čestny algoritmičny prědel je okolo 45–48% točno.</p>

           <h2>Izvory i licencija</h2>
           <p>Oficialny slovnik: interslavic-dictionary.com. Praslovjanske rekonstrukcije: Wiktionary (CC BY-SA). Formy prěgibanja: interslavic-rs. Kod: <a href='{repo}'>MIT</a>.</p>
         </article>",
        langs = 11,
        exact = exact_rate,
        norm = norm_rate,
        top3 = top3,
        n = compact(n),
        repo = REPO_URL,
    );
    page("O metodě — medžuslovjansky generator", &body, 0)
}

fn css() -> String {
    format!("{}\n{}", BASE_CSS, EXTRA_CSS)
}

const BASE_CSS: &str = include_str!("../static/wiktionary.css");
// Wiktionary/MediaWiki look for the generated pages (light theme, one column).
const EXTRA_CSS: &str = r#"
:root{--border:#a2a9b1;--line:#c8ccd1;--link:#36c;--visited:#6b4ba1;--text:#202122;--muted:#54595d;--page:#f8f9fa;--th:#eaecf0}
html,body{margin:0;padding:0;background:var(--page);color:var(--text);font:14px/1.6 -apple-system,'Segoe UI',Helvetica,Arial,sans-serif}
a{color:var(--link);text-decoration:none}
a:visited{color:var(--visited)}
a:hover{text-decoration:underline}
main{max-width:1160px;margin:0 auto;background:#fff;padding:1.1rem 1.6rem 2.4rem;border-left:1px solid var(--line);border-right:1px solid var(--line);min-height:70vh}
.serif{font-family:Georgia,'Linux Libertine','Times New Roman',serif}
.site-header{background:#fff;border-bottom:1px solid var(--border);padding:.45rem 1.2rem;display:flex;align-items:baseline;gap:1rem;flex-wrap:wrap}
.brand{font-family:Georgia,'Linux Libertine','Times New Roman',serif;font-size:1.4rem;font-weight:normal;color:var(--text);text-decoration:none}
.tagline{color:var(--muted);font-size:.88rem}
.nav{margin-left:auto;display:flex;gap:1.1rem}
.nav a{color:var(--link);font-size:.92rem}
.site-footer{max-width:1160px;margin:0 auto;background:#fff;border-left:1px solid var(--line);border-right:1px solid var(--line);border-top:1px solid var(--line);padding:.9rem 1.6rem 1.4rem;color:var(--muted);font-size:.88rem}

/* Headings — serif with the MediaWiki underline. */
h1.firstHeading,.page-title,.hero h1,.entry>h1,.about h1{font-family:Georgia,'Linux Libertine','Times New Roman',serif;font-weight:normal;font-size:1.9rem;line-height:1.25;margin:0 0 .35rem;border-bottom:1px solid var(--border);padding-bottom:.12em;color:var(--text)}
h2{font-family:Georgia,'Linux Libertine','Times New Roman',serif;font-weight:normal;font-size:1.5rem;margin:1.1em 0 .3em;border-bottom:1px solid var(--border);padding-bottom:.08em}
h3,h4{font-family:Georgia,'Linux Libertine','Times New Roman',serif;font-weight:normal;margin:.7em 0 .2em}

/* Tables. */
.wikitable{background:var(--page);color:var(--text);border:1px solid var(--border);border-collapse:collapse;width:100%;margin:.6em 0;font-size:.95em}
.wikitable th,.wikitable td{border:1px solid var(--border);padding:.3em .55em;text-align:left;vertical-align:top}
.wikitable th,.wikitable thead th{background:var(--th);font-weight:bold}
.inflection-table th{white-space:nowrap}
.compact-table td.lc{color:var(--muted);white-space:nowrap}
.translations-table td.lc{color:var(--muted);white-space:nowrap;width:9em}
.example-official{border-left:3px solid var(--border);background:var(--page);padding:.45rem .75rem;margin:.5rem 0;font-style:italic}
.attr-official{font-size:.82em;margin:.35rem 0 0}
.top-candidate{background:#eafaef}
tr:target{background:#fff3bf;outline:2px solid #f0c000}
.score{font-variant-numeric:tabular-nums}

/* Search. */
.hero{border-bottom:1px solid var(--border);padding-bottom:1rem;margin-bottom:1rem}
.lede{color:var(--muted);max-width:72ch}
.searchbox{margin:.9rem 0}
#q{width:100%;box-sizing:border-box;padding:.45rem .55rem;font-size:1.05rem;border:1px solid var(--border);border-radius:2px;background:#fff;color:var(--text)}
.results{margin-top:.3rem}
.hit{display:block;padding:.35em .55em;border:1px solid var(--line);border-top:none;text-decoration:none;color:var(--text)}
.hit:first-child{border-top:1px solid var(--line)}
.hit:hover{background:#eaf3ff;text-decoration:none}
.hit b{color:var(--link)}
.hit .hp{color:var(--muted);font-size:.8em;margin:0 .4em}
.hit .hg{color:var(--muted)}
.hit .hsrc{font-size:.8em;color:var(--muted);background:var(--th);border:1px solid var(--line);border-radius:2px;padding:.02rem .35rem;margin-left:.35em;white-space:nowrap}

/* Stat cards. */
.statgrid{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:.7rem;margin:1rem 0}
.stat{border:1px solid var(--border);background:var(--page);padding:.6rem .7rem;text-align:center}
.stat.ok{background:#eafaef}
.statnum{font-size:1.45rem;font-family:Georgia,'Linux Libertine','Times New Roman',serif}
.statlbl{color:var(--muted);font-size:.85em}

/* Entry header line. */
.page-title{margin-bottom:.4rem}
.headword-block{border:1px solid var(--line);background:var(--page);padding:.55rem .8rem;margin:.4rem 0 1rem}
.headmeta{display:flex;gap:.45em;flex-wrap:wrap;align-items:center}
.def{margin:.55em 0 0}
.badge{display:inline-block;background:var(--th);border:1px solid var(--line);border-radius:2px;padding:.05rem .35rem;font-size:.85em;color:var(--text)}
.pill{display:inline-block;border:1px solid var(--line);border-radius:2px;padding:.03rem .4rem;font-size:.8em;background:var(--th);white-space:nowrap}
.pill.ok{background:#d5f4d5;border-color:#9cce9c}
.pill.bad{background:#f6dada;border-color:#e0a0a0}
.pill.warn{background:#fbeecb;border-color:#e3cd86}
.pill.info,.pill.src-consensus{background:#dbe8fb;border-color:#a7c4ee}
.pill.src-proto{background:#ece3fb;border-color:#c1abef}
.pill.src-official{background:#d5f4d5;border-color:#9cce9c}
.reliability{display:inline-block;border:1px solid var(--line);border-radius:2px;padding:.03rem .4rem;font-size:.8em}
.reliability.conf-high{background:#d5f4d5}
.reliability.conf-med{background:#fbeecb}
.reliability.conf-low{background:#f6dada}

/* Banners → MediaWiki notice look. */
.banner{border:1px solid var(--border);border-left:6px solid var(--border);background:var(--page);padding:.6rem .8rem;margin:.85rem 0}
.banner.ok{border-left-color:#14866d}
.banner.warn{border-left-color:#f2a900}
.banner.info{border-left-color:var(--link)}

/* Collapsible sections. */
.sec{border:1px solid var(--line);margin:.7em 0;padding:0 .8rem .55rem}
.sec>summary{margin:0 -.8rem;padding:.4em .8rem;background:var(--page);border-bottom:1px solid var(--line);cursor:pointer;font-family:Georgia,'Linux Libertine','Times New Roman',serif;font-size:1.15rem}
.sec[open]>summary{margin-bottom:.5em}

/* Evidence. */
.branch-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(240px,1fr));gap:1rem}
.branch-box h4{margin:.3em 0;font-size:1.05rem}

.mention,.Latn{font-style:italic;font-weight:bold}
.muted,.qualifier{color:var(--muted)}
.calib{font-style:italic}
.foot{color:var(--muted);font-size:.88em;margin-top:1.4rem;border-top:1px solid var(--line);padding-top:.6rem}
.rule-trace li{margin:.35em 0}
.rule-id{background:var(--th);border:1px solid var(--line);padding:.02em .3em;font-size:.85em}
.notice{border:1px solid var(--border);background:var(--page);padding:.6rem .8rem;margin:.6rem 0}
/* Sidebar spotlight + search strength. */
#spotlight{margin:.2rem 0 .5rem}
.spotlight-word{display:inline-block;font-family:Georgia,'Linux Libertine','Times New Roman',serif;font-size:1.35rem}
.spot-strength{margin-top:.45rem;font-size:.9em;color:var(--muted)}
.portal-box button{margin-top:.4rem;padding:.3rem .7rem;border:1px solid var(--link);background:var(--link);color:#fff;border-radius:2px;cursor:pointer;font-size:.9em}
.portal-box button:hover{background:#447ff5}
.hit .hs,.hit .ha,.hit .hl,.hit .hrz{font-size:.85em;white-space:nowrap}.hit .ha,.hit .hl,.hit .hrz{color:var(--muted)}

/* Razumlivost (issue #79): compact per-branch coverage bars + search chip. */
.razb{display:inline-flex;align-items:center;gap:.2em;margin-left:.45em;font-size:.8em;color:var(--muted);white-space:nowrap}
.razt{display:inline-block;width:2.4em;height:.55em;background:var(--th);border:1px solid var(--line);border-radius:2px;overflow:hidden}
.razf{display:block;height:100%;background:#a7c4ee}
.wiki-main-list .wikitable td:nth-child(4){white-space:nowrap}
@media (max-width:720px){main,.site-footer{padding-left:.8rem;padding-right:.8rem;border-left:none;border-right:none}.wikitable{font-size:.9em}}

/* Native-Wiktionary enrichment: etymology sources, extra senses, semantic chips. */
a.ext{font-size:.78em;color:var(--muted);border:1px solid var(--line);border-radius:2px;padding:0 .25em;margin-left:.25em;white-space:nowrap}
a.ext:hover{color:var(--link);text-decoration:none;border-color:var(--link)}
.etym-sources{display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:.8rem;margin:.4rem 0}
.etym-src,.src-block{border:1px solid var(--line);border-left:3px solid var(--border);background:var(--page);padding:.5rem .7rem;border-radius:2px}
.src-block{margin:.6rem 0}
.src-head{font-weight:bold;margin-bottom:.35rem}
.src-head .lc{color:var(--muted);font-weight:normal;margin-right:.4em}
.etym-src p{margin:.25em 0;font-size:.95em}
.conn{margin:.5rem 0}
.conn h5{margin:.3em 0;font-size:.82rem;color:var(--muted);text-transform:uppercase;letter-spacing:.03em}
.conn ol{margin:.2em 0 .2em 1.2em}
.conn ul.quotes{list-style:none;margin:.2em 0 .4em 0;padding:0}
.conn li.quote{font-size:.92em;color:var(--muted);font-style:italic;margin:.15em 0;border-left:2px solid var(--line);padding-left:.5em}
.conn li.quote .cite{font-style:normal;font-size:.9em}
.chips{display:flex;flex-wrap:wrap;gap:.3rem}
a.chip{display:inline-block;background:var(--th);border:1px solid var(--line);border-radius:10px;padding:.05em .55em;font-size:.9em;color:var(--text)}
a.chip:hover{background:#eaf3ff;border-color:var(--link);text-decoration:none}
a.chip.xref{border-color:var(--link);color:var(--link);background:#eaf3ff}
a.chip.xref::before{content:'→\00a0';opacity:.65}
a.chip.xref:hover{background:var(--link);color:#fff}
a.redlink{color:#ba0000!important;border-color:#d33!important;background:#fff5f5!important}
a.redlink::after{content:' ?';font-size:.8em}
.entry-tabs{display:flex;gap:.2rem;border-bottom:1px solid var(--border);margin:.1rem 0 .75rem;flex-wrap:wrap}
.entry-tabs a{display:inline-block;padding:.25rem .65rem;border:1px solid var(--border);border-bottom:none;background:var(--th);color:var(--link);border-radius:2px 2px 0 0}
.entry-tabs a.active{background:#fff;color:var(--text);font-weight:bold;position:relative;top:1px;text-decoration:none}
.catlinks{border:1px solid var(--line);background:var(--page);padding:.35rem .55rem;margin:1.2rem 0 .7rem;font-size:.92em}.catlinks a{color:var(--link);background:none;border:0;padding:0}.catlinks a:visited{color:var(--visited)}.word-index td:first-child{white-space:nowrap}.filter-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:.6rem;border:1px solid var(--line);background:var(--page);padding:.7rem;margin:.8rem 0}.filter-grid label{font-size:.9em;color:var(--muted)}.filter-grid select,.filter-grid input{width:100%;box-sizing:border-box;margin-top:.15rem;padding:.3rem;border:1px solid var(--border);background:#fff}.hq{color:var(--muted);font-size:.82em;margin-left:.4em}.graph-list .badge{min-width:4.5em;text-align:center}.dab{border-left:6px solid #36c}.reference-list li{margin:.25rem 0}.alphabet-index a{display:inline-block;margin:.05rem .35rem .05rem 0}.stat-box h3{font-size:1.05rem;margin:.2rem 0;border-bottom:1px solid var(--line)}.index-summary th{width:24%}.category-list{columns:2;column-gap:2rem}.entry-infobox{float:right;width:260px;margin:.2rem 0 .9rem 1rem;font-size:.9em}.entry-infobox caption{font-family:Georgia,serif;font-weight:bold;padding:.25rem}.entry-grid{display:grid;grid-template-columns:minmax(0,1fr) 320px;gap:1.15rem;align-items:start}.entry-main{min-width:0}.entry-rail{position:sticky;top:.75rem;max-height:calc(100vh - 1.5rem);overflow:auto;align-self:start}.entry-rail .entry-infobox{float:none;width:auto;margin:0 0 .8rem;font-size:.9em}.rail-box{border:1px solid var(--line);background:var(--page);padding:.55rem .65rem;margin:0 0 .8rem;overflow-x:auto}.rail-box h2{font-size:1.18rem;margin:.05rem 0 .45rem}.rail-box .wikitable{font-size:.86em;margin:.2rem 0}.rail-box .wikitable th,.rail-box .wikitable td{padding:.22rem .32rem}.pipeline-diagram{border:1px solid var(--line);background:var(--page);padding:.55rem;white-space:pre-wrap}.graph-filter button{margin:.15rem .25rem .15rem 0;border:1px solid var(--line);background:var(--page);color:var(--link);padding:.2rem .45rem}.source-table th{width:10rem}@media(max-width:1150px){.entry-grid{display:block}.entry-rail{position:static;max-height:none;overflow:visible}.entry-rail .entry-infobox{margin:.6rem 0}.rail-box{margin:.8rem 0}}@media(max-width:900px){.entry-infobox{float:none;width:auto;margin:.6rem 0}}

/* ===== V-next layout: sticky header search + sidebar + always-open sections ===== */
.site-header{position:sticky;top:0;z-index:50;align-items:center;gap:.8rem 1rem;padding:.4rem 1rem}
.brand{font-size:1.2rem;white-space:nowrap}
.brand-sub{color:var(--muted)}
.hsearch{position:relative;flex:1 1 300px;max-width:620px;display:flex;margin:0}
.hsearch input{flex:1;min-width:0;padding:.4rem .6rem;font-size:1rem;border:1px solid var(--border);border-right:none;border-radius:2px 0 0 2px;background:#fff;color:var(--text)}
.hsearch input:focus{outline:2px solid #a8c7ff;outline-offset:-1px}
.hsearch-go{padding:0 .85rem;border:1px solid var(--link);background:var(--link);color:#fff;border-radius:0 2px 2px 0;cursor:pointer;font-size:1.05rem;line-height:1}
.hsearch-go:hover{background:#447ff5}
.dropdown{display:none;position:absolute;top:100%;left:0;right:0;background:#fff;border:1px solid var(--border);border-top:none;max-height:72vh;overflow-y:auto;z-index:60;box-shadow:0 8px 20px rgba(0,0,0,.14)}
.dropdown .hit{display:block;padding:.35rem .6rem;border-bottom:1px solid var(--line);color:var(--text);text-decoration:none}
.dropdown .hit:hover{background:#eaf3ff}
.dropdown .hit.more{text-align:center;font-weight:bold;color:var(--link);background:var(--th)}
.nav{margin-left:auto;gap:.9rem}
.layout{max-width:1400px;margin:0 auto;display:grid;grid-template-columns:232px minmax(0,1fr);align-items:start}
.sidebar{position:sticky;top:50px;align-self:start;max-height:calc(100vh - 50px);overflow-y:auto;padding:1rem .85rem;border-right:1px solid var(--line);font-size:.9rem}
main{max-width:940px;margin:0;padding:1rem 1.9rem 2.6rem;border:none}
.side-box{margin-bottom:1.15rem}
.side-h{font-weight:bold;text-transform:uppercase;font-size:.7rem;letter-spacing:.05em;color:var(--muted);border-bottom:1px solid var(--line);padding-bottom:.2rem;margin-bottom:.35rem}
.toc a{display:block;padding:.13rem 0;color:var(--link);line-height:1.3}
.toc a.toc-h3{padding-left:.9rem;font-size:.88em}
.side-link{display:block;width:100%;text-align:left;padding:.22rem 0;color:var(--link);background:none;border:none;cursor:pointer;font:inherit;text-decoration:none}
.side-link:hover{text-decoration:underline}
#spotlight .spotlight-word{font-family:Georgia,serif;font-size:1.15rem;display:block}
.entry section{margin:1.3rem 0}
.entry section>h2{font-family:Georgia,'Linux Libertine',serif;font-weight:normal;font-size:1.35rem;margin:.1em 0 .45em;border-bottom:1px solid var(--border);padding-bottom:.1em;scroll-margin-top:58px}
.headword-block{margin:.2rem 0 .5rem}
.headmeta{display:flex;flex-wrap:wrap;gap:.4rem;align-items:center;margin-bottom:.3rem}
.banner{margin:.5rem 0}
.home-hero{border-bottom:1px solid var(--border);padding-bottom:.7rem;margin-bottom:1rem}
.home-cols{display:grid;grid-template-columns:minmax(0,1fr) 236px;gap:1.5rem;align-items:start}
.home-aside .side-box{border:1px solid var(--line);border-radius:2px;padding:.5rem .7rem}
.search-page #page-results .hit{display:block;padding:.45rem .3rem;border-bottom:1px solid var(--line);color:var(--text);text-decoration:none}
.search-page #page-results .hit:hover{background:#eaf3ff}
.search-page .hit .hp{color:var(--muted);margin:0 .5em;font-size:.9em}
.search-page .hit .hg{color:var(--muted)}
@media (max-width:900px){.layout{grid-template-columns:1fr}.sidebar{position:static;max-height:none;border-right:none;border-bottom:1px solid var(--line)}main{max-width:none;padding:1rem}.home-cols{grid-template-columns:1fr}.nav{width:100%;order:3}}

/* Strict wiki link styling: links are plain blue text, never button/chip pills. */
*{border-radius:0!important}
a.ext,a.chip,a.chip.xref,a.redlink,.entry-tabs a,.hit,.dropdown .hit,.dropdown .hit.more,.search-page #page-results .hit,.stat-card{display:inline!important;background:none!important;border:0!important;box-shadow:none!important;padding:0!important;color:var(--link)!important;text-decoration:none!important}
a.ext:hover,a.chip:hover,a.chip.xref:hover,a.redlink:hover,.entry-tabs a:hover,.hit:hover,.dropdown .hit:hover,.search-page #page-results .hit:hover,.stat-card:hover{background:none!important;color:var(--link)!important;text-decoration:underline!important}
a.chip.xref::before,a.redlink::after{content:''!important}.chips{display:block}.chips a{margin-right:.7em}.entry-tabs{display:block;border-bottom:1px solid var(--border);padding-bottom:.2rem}.entry-tabs a{margin-right:1em}.entry-tabs a.active{font-weight:bold;position:static;color:var(--text)!important}.results .hit,.dropdown .hit,.search-page #page-results .hit{display:block!important;padding:.18rem 0!important;border-bottom:1px solid var(--line)!important;color:var(--text)!important}.results .hit b,.dropdown .hit b,.search-page #page-results .hit b{color:var(--link)}button,.portal-box button,.graph-filter button,.hsearch-go,.side-link{background:none!important;border:0!important;box-shadow:none!important;color:var(--link)!important;padding:0!important;font:inherit!important;cursor:pointer!important}.hsearch-go{padding:0 .35rem!important;border:1px solid var(--border)!important;border-left:0!important}.hsearch-go:hover,button:hover,.portal-box button:hover,.graph-filter button:hover,.side-link:hover{text-decoration:underline!important;background:none!important;color:var(--link)!important}.badge,.pill,.reliability{border-radius:0!important}.cat-more summary{color:var(--link);cursor:pointer}.cat-more summary:hover{text-decoration:underline}

/* Wider readable canvas and sticky rails that stay below the fixed header. */
.layout{max-width:1680px}
main{max-width:none;width:100%;box-sizing:border-box;padding-left:2rem;padding-right:2rem}
.site-footer{max-width:1680px;box-sizing:border-box}
.sidebar{top:56px;max-height:calc(100vh - 56px)}
.bottom-meta{border-top:1px solid var(--line);border-bottom:1px solid var(--line);margin:1.2rem 0 .8rem;padding:.35rem 0}.bottom-meta>summary{color:var(--link);cursor:pointer}.bottom-meta>summary:hover{text-decoration:underline}.bottom-meta section{margin:.75rem 0}.bottom-meta h2{font-size:1.15rem}
@media (min-width:1151px){.entry-grid{grid-template-columns:minmax(0,1fr) 340px;gap:1.4rem}.entry-rail{position:sticky;top:64px;max-height:calc(100vh - 76px);overflow-y:auto;overflow-x:hidden}}
@media (max-width:900px){main{width:auto;padding-left:1rem;padding-right:1rem}}

"#;

fn esc(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn json_str(v: &str) -> String {
    let mut out = String::from("\"");
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push(' '),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_pattern_renders_the_seven_combinations_canonically() {
        // Issue #73c: the pattern is the exact branch SET (via branch_of), in
        // the fixed V→Z→J order, independent of input order; codes outside
        // the registry drop out; an empty/unresolvable set yields None.
        let l = |codes: &[&str]| -> Vec<String> { codes.iter().map(|s| s.to_string()).collect() };
        assert_eq!(branch_pattern(&l(&["ru"])).as_deref(), Some("V"));
        assert_eq!(branch_pattern(&l(&["pl", "cs"])).as_deref(), Some("Z"));
        assert_eq!(branch_pattern(&l(&["sh"])).as_deref(), Some("J"));
        assert_eq!(branch_pattern(&l(&["ru", "pl"])).as_deref(), Some("V+Z"));
        assert_eq!(branch_pattern(&l(&["uk", "bg"])).as_deref(), Some("V+J"));
        assert_eq!(branch_pattern(&l(&["sk", "mk"])).as_deref(), Some("Z+J"));
        assert_eq!(
            branch_pattern(&l(&["ru", "pl", "sl"])).as_deref(),
            Some("V+Z+J")
        );
        // Canonical order regardless of input order; unknown codes ignored.
        assert_eq!(
            branch_pattern(&l(&["bg", "xx", "ru"])).as_deref(),
            Some("V+J")
        );
        assert_eq!(branch_pattern(&l(&["xx"])), None);
        assert_eq!(branch_pattern(&[]), None);
    }

    #[test]
    fn rule_keys_disambiguate_engines_and_map_to_safe_files() {
        // Issue #73a: "liquid-metathesis" exists in BOTH engines, so the
        // machine key and the page file must carry the engine tag.
        assert_eq!(
            rule_key("proto", "liquid-metathesis"),
            "proto:liquid-metathesis"
        );
        assert_eq!(
            rule_key("konsensus", "liquid-metathesis"),
            "konsensus:liquid-metathesis"
        );
        assert_ne!(
            rule_file_stem("proto", "liquid-metathesis"),
            rule_file_stem("konsensus", "liquid-metathesis")
        );
        assert_eq!(
            rule_file_stem("proto", "liquid-metathesis"),
            "proto-liquid-metathesis"
        );
        // A hostile id cannot escape the rule/ directory.
        assert_eq!(rule_file_stem("proto", "../x"), "proto-x");
        // Engine derives from the candidate source exactly like the
        // benchmark's is_proto flag (eval::stage_of_step).
        assert_eq!(rule_engine(CandidateSource::ProtoSlavicRule), "proto");
        assert_eq!(rule_engine(CandidateSource::BranchConsensus), "konsensus");
        assert_eq!(
            rule_engine(CandidateSource::BorrowingInternationalism),
            "konsensus"
        );
    }

    #[test]
    fn proto_reflex_join_folds_accents_and_attributes_by_membership() {
        // Issue #73b (review-hardened). The fold must strip BOTH combining
        // marks (*pę̑tь) and precomposed accented vowels (*vodà) while
        // keeping etymological letters (ě ≠ e, č ≠ c) apart.
        assert_eq!(
            fold_proto_accents("p\u{119}\u{311}t\u{44c}"),
            "p\u{119}t\u{44c}"
        );
        assert_eq!(fold_proto_accents("vod\u{e0}"), "voda"); // à precomposed
        assert_eq!(fold_proto_accents("\u{f2}ko"), "oko"); // ò precomposed
        assert_ne!(fold_proto_accents("cělo"), fold_proto_accents("čelo"));

        let pe = |word: &str, gloss: &str| crate::dump::ProtoEntry {
            word: word.to_string(),
            pos: "noun".to_string(),
            glosses: vec![gloss.to_string()],
            descendants: vec![("ru".to_string(), "x".to_string())],
            pbs: String::new(),
            pie: String::new(),
            stem_class: None,
        };
        // Cache: homonymous *voda ×2 + the cělo/čelo slug-collision pair.
        let pi = crate::dump::ProtoIndex::build(vec![
            pe("voda", "water"),
            pe("voda", "leash (homonym)"),
            pe("cělo", "whole"),
            pe("čelo", "forehead"),
        ]);
        let set = |proto: &str| crate::corpus::CognateSet {
            proto: proto.to_string(),
            etymon: proto.to_string(),
            borrowed: false,
            pos: crate::model::Pos::Noun,
            gloss: String::new(),
            members: Vec::new(),
        };
        let sets = [
            (1usize, set("*vod\u{e0}")), // precomposed accent joins "voda"
            (2usize, set("*cělo")),
            (3usize, set("*čelo")),
            (4usize, set("*neznajemo")), // honest miss
        ];
        let index = build_proto_reflex_index(Some(&pi), sets.iter().map(|(id, s)| (*id, s)));
        assert_eq!((index.linked, index.misses), (3, 1));
        // Homonyms: the voda page lists BOTH cache entries.
        let voda_slug = index.membership.get(&1).expect("voda membership");
        let voda = index.pages.get(voda_slug).unwrap();
        assert_eq!(voda.recons, vec![0, 1], "homonyms share the page");
        assert_eq!(voda.entry_ids, vec![1]);
        // Slug collision: cělo and čelo both slug to "celo" but are DIFFERENT
        // lexemes → two pages with a deterministic suffix, and membership
        // sends each entry to ITS OWN lexeme's page (entry 679 class bug).
        let celo_slug = index.membership.get(&2).unwrap();
        let chelo_slug = index.membership.get(&3).unwrap();
        assert_ne!(celo_slug, chelo_slug, "cělo vs čelo must not share a page");
        assert_eq!(celo_slug, "celo");
        assert_eq!(chelo_slug, "celo-2");
        assert_eq!(index.pages[celo_slug].recons, vec![2]);
        assert_eq!(index.pages[chelo_slug].recons, vec![3]);
        // The membership invariant the link audit checks: every membership
        // target page lists the member.
        for (id, sl) in &index.membership {
            assert!(
                index.pages[sl].entry_ids.contains(id),
                "membership target must list entry {id}"
            );
        }
        // A borrowed set or a non-'*' ancestor is never looked up.
        let mut borrowed = set("*voda");
        borrowed.borrowed = true;
        let i2 = build_proto_reflex_index(Some(&pi), std::iter::once((8usize, &borrowed)));
        assert!(i2.pages.is_empty() && i2.linked == 0 && i2.misses == 0);
    }

    #[test]
    fn generated_derivatives_never_collide_and_are_all_generated() {
        // Issue #37 invariants: injected derivatives are pure ADDITIONS that (a)
        // never collide with an official / official-only form_key (incl. INFLECTED
        // forms), (b) are all status="generated", source="lemma", probability set,
        // provenance-tagged, and (c) never re-emit a form the dictionary already has.
        use crate::model::{Gender, Pos};
        let mut form_sink = crate::forms::RecordSink::default();
        let mut lemma_sink = crate::forms::RecordSink::default();
        // An official base + its FULL paradigm — dedup must see inflected forms.
        for s in [&mut form_sink, &mut lemma_sink] {
            s.add(
                "kniga", "", "kniga", 1, "n", "lemma", "official", None, "book",
            );
        }
        crate::forms::paradigm_records(
            &mut form_sink,
            "kniga",
            Pos::Noun,
            Some(Gender::Feminine),
            1,
            "official",
            None,
            "book",
        );
        // Force a would-be derivative to already be official (knižny, denominal
        // adjective): the derivation of it MUST be dropped as a collision.
        for s in [&mut form_sink, &mut lemma_sink] {
            s.add(
                "knižny", "", "knižny", 2, "adj", "lemma", "official", None, "bookish",
            );
        }

        let official_keys = form_sink.form_key_set();
        let mut taken = official_keys.clone();
        let bases = vec![("kniga".to_string(), Pos::Noun, 1usize, "book".to_string())];
        let probs = crate::derive::DerivationProbabilities::flat(0.5);
        let added = inject_generated_derivatives(
            &mut form_sink,
            &mut lemma_sink,
            &mut taken,
            &bases,
            &probs,
        );
        assert!(
            added > 0,
            "kniga must derive at least one absent family member"
        );

        let records = form_sink.into_records();
        for r in &records {
            if r.status == "generated" {
                assert!(
                    !official_keys.contains(&r.key),
                    "generated {} collides with an official/official-only key",
                    r.key
                );
                assert_eq!(r.source, "lemma", "generated {} must be lemma-only", r.key);
                assert!(
                    r.probability.is_some(),
                    "generated {} must carry a probability",
                    r.key
                );
                assert!(
                    r.analyses.iter().any(|a| a.starts_with("deriv:")),
                    "generated {} must carry deriv provenance",
                    r.key
                );
            }
        }
        // The colliding member survives ONLY as the seeded official record.
        let knizny_key = crate::forms::form_key("knižny");
        assert!(
            records
                .iter()
                .filter(|r| r.key == knizny_key)
                .all(|r| r.status == "official"),
            "knižny must not be re-emitted as a generated derivative"
        );
        // A non-colliding member (the diminutive knižka) ships as generated.
        let knizka_key = crate::forms::form_key("knižka");
        assert!(
            records
                .iter()
                .any(|r| r.key == knizka_key && r.status == "generated"),
            "knižka should ship as a generated derivative"
        );
    }

    #[test]
    fn source_aliases_index_cyrillic_latin_and_folded_forms() {
        // Issue #31: committee/cognate source words become searchable aliases.
        // Each alias carries the attested spelling + its folded search forms so
        // the entry is findable by Cyrillic, transliteration, and diacritic-fold.
        let mut aliases: Vec<SourceAlias> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        collect_source_aliases(
            [
                ("ru", "пластинка"),
                ("pl", "płyta"),
                ("cs", "žena"),
                // A multi-variant cell splits into one alias per listed variant.
                ("uk", "швидкий, скорий"),
            ],
            &mut aliases,
            &mut seen,
        );
        let has = |lang: &str, word: &str| aliases.iter().any(|(l, w, _)| l == lang && w == word);
        // Attested spellings are indexed verbatim (Cyrillic query hits directly).
        assert!(has("ru", "пластинка"), "{aliases:?}");
        assert!(
            has("uk", "швидкий") && has("uk", "скорий"),
            "split: {aliases:?}"
        );
        // Folded search forms make the Latinized / diacritic-folded query hit.
        let forms = |lang: &str, word: &str| {
            aliases
                .iter()
                .find(|(l, w, _)| l == lang && w == word)
                .map(|(_, _, f)| f.clone())
                .unwrap_or_default()
        };
        assert!(
            forms("ru", "пластинка").iter().any(|f| f == "plastinka"),
            "ru transliteration: {aliases:?}"
        );
        // Polish ł does not decompose under the client's NFD fold, so the ASCII
        // skeleton (plyta) must be stored explicitly for the folded query to hit.
        assert!(
            forms("pl", "płyta").iter().any(|f| f == "plyta"),
            "pl skeleton: {aliases:?}"
        );
        assert!(
            forms("cs", "žena").iter().any(|f| f == "zena"),
            "cs skeleton: {aliases:?}"
        );
        // JSON is well-formed and preserves the attested spelling.
        let json = source_aliases_json(&aliases);
        assert!(json.starts_with('[') && json.ends_with(']'), "{json}");
        assert!(json.contains("[\"ru\",\"пластинка\","), "{json}");
    }

    #[test]
    fn search_keys_make_alternatives_and_folds_findable() {
        // The kråtky case (V6 §2): the top candidate is flavored, the official
        // spelling appears as candidate 2 — both the alternative's form and the
        // ASCII folds must be searchable keys.
        let cands = vec![
            Candidate::new(
                "kråtȯky".to_string(),
                CandidateSource::ProtoSlavicRule,
                0.98,
            ),
            Candidate::new(
                "kratky".to_string(),
                CandidateSource::BranchConsensus,
                0.967,
            ),
        ];
        let keys = search_keys(&cands, "kråtȯky");
        let has = |k: &str, r: usize| keys.iter().any(|(kk, rr)| kk == k && *rr == r);
        assert!(has("kratoky", 1), "ASCII fold of the headword: {keys:?}");
        assert!(has("kratky", 2), "the alternative's own form: {keys:?}");
        // The raw headword itself is NOT duplicated (the client matches it).
        assert!(!keys.iter().any(|(k, _)| k == "kråtȯky"));
    }

    #[test]
    fn proto_stems_group_word_families() {
        // One derivational suffix stripped, ≥4-char stem kept.
        assert_eq!(proto_stem("starъ").as_deref(), Some("star"));
        assert_eq!(proto_stem("starostь").as_deref(), Some("star"));
        assert_eq!(proto_stem("starьcь").as_deref(), Some("star"));
        // Combining accent marks (lemma-corpus reconstructions carry them:
        // pę̑tь) are folded first, so accent variants share one key.
        assert_eq!(proto_stem("sta\u{0301}rъ").as_deref(), Some("star"));
        // A root too short after stripping keeps the whole word as its key…
        assert_eq!(proto_stem("pьsъ").as_deref(), Some("pьsъ"));
        // …and a genuinely tiny fragment gets none.
        assert_eq!(proto_stem("kъ"), None);
    }

    #[test]
    fn search_keys_json_is_well_formed() {
        let cands = vec![Candidate::new(
            "běly".to_string(),
            CandidateSource::ProtoSlavicRule,
            0.9,
        )];
        let keys = search_keys(&cands, "běly");
        let json = keys_json(&keys);
        assert!(json.starts_with('[') && json.ends_with(']'));
        assert!(json.contains("[\"bely\",1]"), "{json}");
    }

    #[test]
    fn suppletive_plurals_come_from_the_inflector() {
        // RULE_SPEC §3.1: člověk→ljudi, oko→oči — the pinned inflector rev
        // implements them (with the heteroclite byforms); a rev bump that
        // loses them must fail here, not silently reshape the tables.
        assert!(noun_table("člověk", None).contains("ljudi"));
        assert!(noun_table("oko", None).contains("oči"));
    }

    #[test]
    fn canonical_paradigms_pin_the_inflector_rev() {
        // A crate rev bump that changes these canonical cells (STEEN-G tables)
        // must fail CI, not silently reshape 30k inflection tables.
        let fold = |x: String| crate::orthography::to_standard(&x.to_lowercase());
        assert_eq!(
            fold(ISV::noun("žena", IsvCase::Gen, IsvNumber::Singular)),
            "ženy"
        );
        assert_eq!(
            fold(ISV::noun("grad", IsvCase::Gen, IsvNumber::Singular)),
            "grada"
        );
        assert_eq!(
            fold(ISV::adj(
                "dobry",
                IsvCase::Nom,
                IsvNumber::Singular,
                IsvGender::Feminine,
                IsvAnimacy::Inanimate
            )),
            "dobra"
        );
    }

    fn raw_lem(lang: &str, word: &str, pos: &str) -> crate::dump::RawSlavicLemma {
        crate::dump::RawSlavicLemma {
            word: word.to_string(),
            lang: lang.to_string(),
            pos: pos.to_string(),
            glosses: vec!["g".to_string()],
            etymology_text: String::new(),
            proto: String::new(),
            etymon: String::new(),
        }
    }

    #[test]
    fn historical_cognates_render_as_labeled_hints_not_branch_evidence() {
        let member = |lang: &str, word: &str| crate::dump::LemmaEntry {
            lang: lang.to_string(),
            word: word.to_string(),
            pos: "noun".to_string(),
            gloss: "water".to_string(),
            proto: "*voda".to_string(),
            etymon: String::new(),
            etymology: Vec::new(),
            categories: Vec::new(),
            topics: Vec::new(),
            tags: Vec::new(),
        };
        let set = crate::corpus::CognateSet {
            proto: "*voda".to_string(),
            etymon: "*voda".to_string(),
            borrowed: false,
            pos: crate::model::Pos::Noun,
            gloss: "water".to_string(),
            members: vec![member("ru", "вода"), member("cu", "вода")],
        };
        let generated = crate::corpus::generate_set(set, &ConsensusConfig::production());
        let html = cognate_block(&generated, None);
        let historical = html.find("historical-hints").unwrap();
        assert!(html[..historical].contains("rusky"), "{html}");
        assert!(!html[..historical].contains("starocŕkov"), "{html}");
        assert!(html[historical..].contains("starocŕkov"), "{html}");
    }

    #[test]
    fn official_matching_requires_pos_and_gloss_evidence() {
        let entries = crate::official::load(Path::new("data/official-isv.csv")).unwrap();
        let rows: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.isv.trim() == "držati")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            select_official_entry(
                &rows,
                &entries,
                crate::model::Pos::Noun,
                "chills, trembling"
            ),
            None
        );
        let matched = select_official_entry(
            &rows,
            &entries,
            crate::model::Pos::Verb,
            "to shiver and tremble",
        )
        .unwrap();
        assert_eq!(entries[matched].english, "shudder, shiver, tremble");

        let bajka_rows: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.isv.trim() == "bajka")
            .map(|(i, _)| i)
            .collect();
        let bajka =
            select_official_entry(&bajka_rows, &entries, crate::model::Pos::Noun, "fairy tale")
                .unwrap();
        assert_eq!(entries[bajka].english, "fairytale");
    }

    #[test]
    fn headword_routes_exactly_and_abstains_on_ambiguous_folds() {
        let mut index = HeadwordIndex::default();
        index.insert("legti", 1);
        index.insert("lęgti", 2);
        assert_eq!(index.resolve("legti"), Some(1));
        assert_eq!(index.resolve("lęgti"), Some(2));
        assert_eq!(index.resolve("lěgti"), None);
        assert_eq!(index.resolve_fold("legti"), None);
    }

    /// Issue #64 invariants: the raw pre-pass assigns sequential ids in corpus
    /// order, and every raw `(lang, word)` with a Slovowiki home resolves in
    /// the plan's cross-reference — its own raw page, the official page its
    /// display fold collided with, or the earlier raw twin that claimed the
    /// same ě-blind fold. Cognate-xref members stay with the cognate xref;
    /// empty words resolve nowhere.
    #[test]
    fn raw_plan_assigns_ids_and_points_chips_at_internal_pages() {
        let mut xref = crate::enrich::Xref::new();
        xref.insert("pl", "xyz", 7);
        xref.insert("pl", "xyz", 8); // ambiguous, but represented by generated pages
        let mut isv_to_id = HeadwordIndex::default();
        isv_to_id.insert("delo", 42); // an official headword
        let lemmas = vec![
            raw_lem("pl", "winyl", "noun"), // rendered → id 101
            raw_lem("cs", "mouka", "noun"), // rendered (muka) → id 102
            raw_lem("sl", "muka", "noun"),  // raw twin of mouka → points at 102
            raw_lem("sl", "delo", "noun"),  // folds onto official 42
            raw_lem("pl", "xyz", "noun"),   // cognate member → xref resolves
            raw_lem("pl", "", "noun"),      // skipped
        ];
        let plan = plan_raw_pages(&lemmas, &xref, &isv_to_id, 100);
        assert_eq!(plan.pages, vec![(0, 101), (1, 102)]);
        assert_eq!(plan.deduped, 4);
        assert_eq!(plan.xref.get("pl", "winyl"), Some(101));
        assert_eq!(plan.xref.get("cs", "mouka"), Some(102));
        assert_eq!(plan.xref.get("sl", "muka"), Some(102));
        assert_eq!(plan.xref.get("sl", "delo"), Some(42));
        assert_eq!(plan.xref.get("pl", "xyz"), None);
        assert_eq!(plan.xref.get("pl", ""), None);
        // Raw-collision display credit (issue #86 item 6): ONLY the
        // fold-deduped attestation is credited to its target page — cognate
        // members (xref) and raw twins are already visible elsewhere.
        assert_eq!(
            plan.credit.get(&42),
            Some(&vec![("sl".to_string(), "delo".to_string())])
        );
        assert_eq!(plan.credit.len(), 1);
        // The rendered line links to the source-language Wiktionary and
        // carries the display-only disclaimer.
        let line = raw_credit_line(plan.credit.get(&42));
        assert!(line.contains("Takože atestovano"), "{line}");
        assert!(line.contains("sl delo"), "{line}");
        assert!(line.contains("surova atestacija"), "{line}");
        assert_eq!(raw_credit_line(None), "");
        // Cap: 13 credits render 12 + "+1 dalje".
        let many: Vec<(String, String)> = (0..13)
            .map(|i| ("uk".to_string(), format!("слово{i}")))
            .collect();
        let line = raw_credit_line(Some(&many));
        assert!(line.contains("+1 dalje"), "{line}");
    }

    #[test]
    fn deterministic_entry_ids_ignore_previous_output() {
        let sequence = || {
            let mut ids = DeterministicEntryIds::default();
            (ids.alloc(), ids.alloc(), ids.max_id())
        };
        assert_eq!(sequence(), (1, 2, 2));
        assert_eq!(sequence(), sequence());
        assert_eq!(
            BuildMeta::current(1, 1).generated,
            BuildMeta::current(1, 1).generated
        );
    }

    /// Test metas for the official-fact-treatment invariants (issue #86).
    fn meta_for(
        conf: Confidence,
        prob: Option<f64>,
        prior: Option<f64>,
        official_only: bool,
        official_lemma: Option<&str>,
        langs: &[&str],
    ) -> SiteEntryMeta {
        let mut m = entry_meta(
            1,
            "aloe",
            "aloe",
            "noun",
            if official_lemma.is_some() {
                MatchStatus::OfficialMatch
            } else {
                MatchStatus::NoOfficialEntry
            },
            conf,
            0.30,
            prob,
            langs.len(),
            1,
            true,
            official_only,
            official_lemma.map(|s| s.to_string()),
            "grc ἀλόη".to_string(),
            langs.iter().map(|s| s.to_string()).collect(),
            Vec::new(),
        );
        m.prior = prior;
        m
    }

    /// Issue #86 defect 1: official dictionary words — matched AND
    /// official-only — are facts. They never land on review worklists no
    /// matter what the (now-irrelevant) confidence/probability say, while
    /// machine-only reconstructions remain curation work.
    #[test]
    fn official_words_never_need_review() {
        let matched = meta_for(
            Confidence::High,
            None,
            Some(0.14),
            false,
            Some("aloe"),
            &["ru"],
        );
        assert!(!needs_review(&matched));
        let official_only = meta_for(Confidence::High, None, None, true, Some("aloe"), &["ru"]);
        assert!(!needs_review(&official_only));
        // A machine-only reconstruction stays on the worklist even at high
        // confidence (the old first clause `official_lemma.is_none()`).
        let generated = meta_for(
            Confidence::High,
            Some(0.73),
            None,
            false,
            None,
            &["ru", "pl"],
        );
        assert!(needs_review(&generated));
    }

    /// Issue #86 defect 1: the matched infobox states the fact ("oficialno",
    /// no p≈ prior), like official-only pages; the calibrated prior surfaces
    /// only as the muted provenance transparency line. Generated entries keep
    /// the calibrated badge + p≈ and get no prior line.
    #[test]
    fn matched_meta_gets_the_official_fact_treatment() {
        let matched = meta_for(
            Confidence::High,
            None,
            Some(0.14),
            false,
            Some("aloe"),
            &["ru"],
        );
        let box_html = entry_infobox(&matched, "", "", "");
        assert!(box_html.contains(">oficialno</span>"), "{box_html}");
        assert!(!box_html.contains("p≈"), "{box_html}");
        let build = BuildMeta {
            git: "test".into(),
            generated: "0 UNIX".into(),
            total_entries: 1,
            lemma_total: 1,
        };
        let prov = provenance_block(&matched, &build);
        assert!(
            prov.contains("Priorna kalibrovana ocěna generatora: p≈0.14"),
            "{prov}"
        );
        // Generated entry: calibrated badge with p≈, no prior line.
        let generated = meta_for(Confidence::Low, Some(0.14), None, false, None, &["ru"]);
        let box_html = entry_infobox(&generated, "", "", "");
        assert!(box_html.contains("nizka"), "{box_html}");
        assert!(box_html.contains("p≈0.14"), "{box_html}");
        assert!(!provenance_block(&generated, &build).contains("Priorna"));
        // Search-row letter: the fact treatment sets g.confidence High for
        // matched entries, so conf_letter must yield "V" for them.
        assert_eq!(conf_letter(Confidence::High), "V");
    }

    /// Issue #75: aspect metadata is bidirectional machine-readable data and
    /// a direct partner link in the entry infobox.
    #[test]
    fn aspect_partners_are_exported_and_linked() {
        let mut m = meta_for(
            Confidence::High,
            None,
            None,
            true,
            Some("dobaviti"),
            &["ru", "pl"],
        );
        m.aspect = Some("pf".to_string());
        m.aspect_partners = vec![
            (24712, "dobavjati".to_string()),
            (24713, "pridobaviti".to_string()),
        ];
        let json = entries_json(&[m.clone()]);
        assert!(json.contains(r#""aspect":"pf""#), "{json}");
        assert!(
            json.contains(r#""aspect_partners":[{"id":24712,"title":"dobavjati"},{"id":24713,"title":"pridobaviti"}]"#),
            "{json}"
        );
        let html = entry_infobox(&m, "", "", "");
        assert!(html.contains("Glagolsky vid</th><td>pf"), "{html}");
        assert!(html.contains("href='24712.html'>dobavjati</a>"), "{html}");
        assert!(html.contains("href='24713.html'>pridobaviti</a>"), "{html}");
    }

    /// Issue #86 defect 2: the razumlivost basis for a matched entry is the
    /// UNION of corpus members and the official row's sameInLanguages — an
    /// aloe-like case (corpus = ru only, committee says "v z j") must read
    /// ≈99%, not ru's 52% share. An empty sameInLanguages leaves the corpus
    /// basis untouched (and official-only pages keep their same_in-only
    /// basis — no union with translation cells, which would re-saturate).
    #[test]
    fn matched_razum_union_basis() {
        let members = vec!["ru".to_string()];
        // "v z j" expands to every modern CSV language across the branches.
        let same_in: Vec<&'static str> = crate::lang::official_slavic_cols()
            .iter()
            .filter(|l| l.modern)
            .map(|l| l.code)
            .collect();
        let union = union_razum_codes(&members, &same_in);
        assert!(union.contains(&"ru".to_string()));
        assert!(union.contains(&"pl".to_string()));
        assert!(union.contains(&"bg".to_string()));
        let pct = razum_pct(&union);
        assert!(pct >= 95, "union basis should read ≈99%, got {pct}");
        // Corpus basis alone (ru) is far lower — the defect the union fixes.
        assert!(razum_pct(&members) < 60);
        // Empty same_in: the corpus basis is unchanged.
        assert_eq!(union_razum_codes(&members, &[]), members);
    }

    /// The client fold is generated from CLIENT_FOLD_PAIRS (injected as
    /// __SEARCH_FOLD__), so Rust bucketing and JS query folding share one
    /// definition (#60/#71). Pin its semantics: Latin diacritics fold to base
    /// letters, đ/ł included; Cyrillic passes through for its own shards.
    #[test]
    fn client_fold_and_buckets_are_stable() {
        assert_eq!(client_fold("Čech"), "cech");
        assert_eq!(client_fold("kråtȯky"), "kratoky");
        assert_eq!(client_fold("vođa"), "voda");
        assert_eq!(client_fold("łapeć"), "lapec");
        assert_eq!(client_fold("пластинка"), "пластинка");
        assert_eq!(client_fold("вода́"), "вода"); // combining mark stripped
        assert_eq!(search_bucket_pair("rěka"), Some(('r', 'e')));
        assert_eq!(search_bucket_pair("s"), Some(('s', '_')));
        assert_eq!(search_bucket_pair("пластинка"), Some(('п', 'л')));
        assert_eq!(search_bucket_pair("…—"), None);
        assert_eq!(shard_file_name("vo"), "vo.json");
        assert_eq!(shard_file_name("п"), "u043f.json");
        assert_eq!(shard_file_name("s_"), "s_.json");
    }

    /// write_search_index end-to-end on synthetic rows: manifest + shard
    /// resolution (two-letter split for a hot bucket, one-letter otherwise),
    /// browse/spotlight carry only core rows, the 14-element row shape
    /// (aliases last, razumlivost at 12; #79), and the completeness
    /// self-check accepts the layout.
    #[test]
    fn search_index_shards_resolve_completely() {
        fn row(id: usize, display: &str, gloss: &str, core: bool) -> SearchRow {
            let keys: Vec<(String, usize)> = Vec::new();
            let aliases: Vec<SourceAlias> =
                vec![("ru".into(), "пример".into(), vec![format!("primer{id}")])];
            SearchRow {
                id,
                head: format!(
                    "[{},{},{},\"noun\",\"N\",\"N\",[],1,1,0,\"\",\"\",0",
                    id,
                    json_str(display),
                    json_str(gloss)
                ),
                aliases: source_aliases_json(&aliases),
                core,
                buckets: search_row_buckets(display, gloss, &keys, &aliases),
            }
        }
        let mut rows: Vec<SearchRow> = vec![
            row(1, "voda", "water", true),
            row(2, "rěka", "river", true),
            row(3, "gramplastinka", "gramophone record", false),
        ];
        // A hot bucket: enough s-rows to exceed the split budget.
        for i in 0..2100 {
            let mut r = row(1000 + i, &format!("slovo{i}"), &"x".repeat(700), false);
            r.aliases = "[]".into();
            r.buckets = search_row_buckets(&format!("slovo{i}"), "", &[], &[]);
            rows.push(r);
        }
        let dir = std::env::temp_dir().join(format!("shard-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (shards, browse) = write_search_index(&dir, &rows).unwrap();
        assert!(shards >= 4, "expected multiple shards, got {shards}");
        assert_eq!(browse, 2); // only the core rows
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("search/manifest.json")).unwrap())
                .unwrap();
        let sh = manifest["shards"].as_object().unwrap();
        // Hot 's' bucket split by second letter; 'v' stayed single-letter.
        assert!(manifest["splits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "s"));
        assert!(sh.contains_key("sl") && !sh.contains_key("s") && sh.contains_key("v"));
        // A Cyrillic alias gives the row a Cyrillic shard (пример → п…).
        assert!(sh.keys().any(|k| k.starts_with('п')));
        // The raw row is reachable through its own bucket but not in browse.
        let v_file = sh["v"]["f"].as_str().unwrap();
        let v_rows: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("search").join(v_file)).unwrap())
                .unwrap();
        assert!(v_rows.as_array().unwrap().iter().any(|r| r[0] == 1));
        // Written row shape (schema 2, #79): 14 elements, razumlivost integer
        // at 12, aliases array LAST at 13.
        let voda = v_rows
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r[0] == 1)
            .unwrap();
        assert_eq!(voda.as_array().unwrap().len(), 14, "{voda}");
        assert!(voda[12].is_u64(), "{voda}");
        assert!(voda[13].is_array(), "{voda}");
        assert_eq!(manifest["schema"], 2);
        let browse_rows: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("search/browse.json")).unwrap())
                .unwrap();
        assert!(browse_rows.as_array().unwrap().iter().all(|r| r[0] != 3));
        // no_alias rows keep the same shape with an empty aliases tail.
        let b0 = &browse_rows.as_array().unwrap()[0];
        assert_eq!(b0.as_array().unwrap().len(), 14, "{b0}");
        assert!(b0[13].as_array().unwrap().is_empty(), "{b0}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The chip lookup chain: cognate xref beats the raw cross-reference beats
    /// the external native-Wiktionary fallback, and a self-link falls through
    /// to the external target.
    #[test]
    fn word_chip_prefers_generated_then_raw_then_external() {
        let mut xref = crate::enrich::Xref::new();
        xref.insert("ru", "дело", 5);
        let mut raw_xref = crate::enrich::Xref::new();
        raw_xref.insert("ru", "грампластинка", 123);
        raw_xref.insert("ru", "дело", 999); // shadowed by the cognate xref
        assert!(
            word_chip("ru", "дело", "dělo", Some(&xref), &raw_xref, 0).contains("href='5.html'")
        );
        assert!(word_chip(
            "ru",
            "грампластинка",
            "gramplastinka",
            Some(&xref),
            &raw_xref,
            0
        )
        .contains("href='123.html'"));
        let self_chip = word_chip(
            "ru",
            "грампластинка",
            "gramplastinka",
            Some(&xref),
            &raw_xref,
            123,
        );
        assert!(self_chip.contains("ru.wiktionary.org"), "{self_chip}");
        assert!(word_chip("uk", "щось", "ščos", Some(&xref), &raw_xref, 0)
            .contains("uk.wiktionary.org"));

        // Once a generated key is ambiguous, do not replace its sense choice
        // with an arbitrary raw-page target; link to the native disambiguation
        // surface instead.
        xref.insert("ru", "дело", 6);
        let ambiguous = word_chip("ru", "дело", "dělo", Some(&xref), &raw_xref, 0);
        assert!(ambiguous.contains("ru.wiktionary.org"), "{ambiguous}");
        assert!(!ambiguous.contains("999.html"), "{ambiguous}");
    }
}
