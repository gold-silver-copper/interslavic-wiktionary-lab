//! Static site generator for the Interslavic candidate dictionary.
//!
//! `export` runs the generator over the official dictionary's Slavic evidence and
//! writes a fully static website — one HTML page per meaning plus a home page
//! with client-side search — under an output directory. There is no server and
//! no in-memory database: the output is plain files hostable on GitHub Pages (or
//! any static host). All links are relative and all CSS is local.

use self::assets::css;
use self::coverage::{
    inject_generated_derivatives, insert_official_byform_aliases, near_official_match,
    official_surface_maps, plan_raw_pages, raw_intl_candidates, raw_intl_probabilities,
    select_official_surface, OfficialSurface,
};
use self::entries::{
    branch_evidence, build_input, corpus_about, corpus_entry_page, corpus_home, derivation_block,
    entry_page, family_block, official_only_page, raw_lemma_page, synonyms_block, CorpusHomeInput,
};
use self::layout::{json_str, truncate};
use self::model::{
    family_key, quality_label, razum_pct, BuildMeta, CorpusEntryInput, FamilyEntry, HeadwordIndex,
    OfficialDisplay, OfficialEntryInput, RawEntryInput, RenderContext, SiteEntryInput,
    SiteEntryMeta,
};
use self::navigation::{
    backlinks_by_target, build_edges, compact_entry_categories, entry_meta, entry_tabs,
    entry_wiki_blocks, homograph_groups, homograph_notice, load_curation_notes, raw_credit_line,
    union_razum_codes, wiktionary_category_paths_for_input, wiktionary_category_paths_for_members,
    write_wiki_indexes, WikiIndexInput,
};
use self::search::{
    collect_source_aliases, conf_letter, home_page, keys_json, official_cell_pairs, search_keys,
    search_page, search_row_buckets, source_aliases_json, write_search_index, HomeRow, SearchRow,
    SourceAlias,
};
use self::special::{
    about_page, build_proto_reflex_index, build_rule_index, datasets_coverage_section,
    datasets_page, forms_page, metrics_page, proposals_page, text_check_page, write_deriv_pages,
    DerivAgg, ProposalRow,
};
use crate::consensus::ConsensusConfig;
use crate::generator;
use crate::model::{Confidence, MatchStatus};
use crate::official::{self, OfficialEntry};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

fn add_official_byform_keys<'a>(
    keys: &mut Vec<(String, usize)>,
    byforms: impl IntoIterator<Item = &'a str>,
    display: &str,
    rank: usize,
) {
    let display_lower = display.to_lowercase();
    for byform in byforms {
        let lower = byform.to_lowercase();
        for k in [
            lower.clone(),
            crate::orthography::to_standard(&lower),
            crate::orthography::ascii_skeleton(byform),
        ] {
            if k.chars().count() >= 2 && k != display_lower && !keys.iter().any(|(kk, _)| kk == &k)
            {
                keys.push((k, rank));
            }
        }
    }
}

/// Generate the whole static site under `out_dir`.
pub fn export(official_path: &Path, out_dir: &Path) -> Result<()> {
    let entries = official::load(official_path)?;
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
        let official_byforms: Vec<String> = entry
            .citation_byforms()
            .into_iter()
            .filter(|byform| !byform.form.contains(' '))
            .map(|byform| byform.form)
            .collect();
        let mut g = generator::generate_with_official_byforms(
            &input,
            official_byforms.iter().map(String::as_str),
            proto,
            &cfg,
        );
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
        if !official_byforms.is_empty() {
            if official_byforms
                .iter()
                .any(|off| crate::orthography::exact_match(&top.form, off))
            {
                n_exact += 1;
            }
            if g.candidates.iter().take(3).any(|c| {
                official_byforms
                    .iter()
                    .any(|off| crate::orthography::normalized_match(&c.form, off))
            }) {
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
        if !official_byforms.is_empty() {
            // The official lemma is searchable even when no candidate spells it:
            // point it at the candidate that agrees (normalized), else the top.
            let rank = g
                .candidates
                .iter()
                .position(|c| {
                    official_byforms
                        .iter()
                        .any(|off| crate::orthography::normalized_match(&c.form, off))
                })
                .map(|i| i + 1)
                .unwrap_or(1);
            add_official_byform_keys(
                &mut keys,
                official_byforms.iter().map(String::as_str),
                &form,
                rank,
            );
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
                json_str(entry.pos.code()),
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
    let panics = crate::forms::inflection_panic_count();
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

/// Generate the static site from the Wiktionary cognate-set corpus. Every set of
/// etymologically-connected Slavic lemmas becomes one Interslavic word, with
/// confidence scaling by how many languages/branches attest it.
pub fn export_corpus(lemmas_path: &Path, official_path: &Path, out_dir: &Path) -> Result<()> {
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
    let (official_by_exact, official_by_fold) = official_surface_maps(&official_entries);

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
    struct OfficialOnlyRecord {
        id: usize,
        entry: OfficialEntry,
        display: String,
        byforms: Vec<String>,
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
        let official_surface_match: Option<(usize, OfficialSurface)> = g
            .candidates
            .iter()
            .take(5)
            .enumerate()
            .find_map(|(rank, c)| {
                select_official_surface(
                    &official_by_exact,
                    &official_by_fold,
                    &c.form,
                    &official_entries,
                    g.set.pos,
                    &g.set.gloss,
                )
                .map(|surface| (rank + 1, surface))
            });
        let matched = official_surface_match
            .as_ref()
            .map(|(rank, surface)| OfficialMatch {
                rank: *rank,
                entry: surface.entry,
            });
        if matched.is_some() {
            official += 1;
        }
        let status = if matched.is_some() {
            MatchStatus::OfficialMatch
        } else {
            MatchStatus::NoOfficialEntry
        };
        let display = official_surface_match
            .map(|(_, surface)| surface.form)
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

    // ---- Confidence domain boundary (issue #89 J26/J27, closed by V11) ----
    // `generate_set` scores are cognate-coverage scores; the official-row
    // pipeline calibrator MUST NOT be applied to them. The corpus path now
    // has its OWN committed calibrator (data/corpus-calibration.json, fitted
    // by `corpus-eval --fit` on the dev split and holdout-validated), loaded
    // with a machine-checked score domain. Absent file → the V10 fail-closed
    // posture (null probabilities, proposals paused) remains.
    let calibration: Option<crate::calibrate::CorpusCalibration> =
        crate::calibrate::CorpusCalibration::load_for_domain(
            Path::new(crate::calibrate::CORPUS_CALIBRATION_PATH),
            crate::calibrate::CORPUS_BANDED_DOMAIN,
        )?;
    match &calibration {
        Some(cal) => println!(
            "Corpus coverage probabilities from the committed corpus calibrator (holdout ECE {:.4}; {}).",
            cal.holdout_ece, cal.fitted_on
        ),
        None => println!(
            "Corpus coverage scores are uncalibrated (no {} — run `corpus-eval --fit`); generated probabilities and novel-word proposal buckets are disabled (issue #89 J26).",
            crate::calibrate::CORPUS_CALIBRATION_PATH
        ),
    }

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
        for (i, prepared) in prepared.iter_mut().enumerate() {
            let Some(m) = prepared.matched else {
                continue;
            };
            let key = official_entries[m.entry].id.clone();
            if best.get(&key) != Some(&i) {
                prepared.matched = None;
                prepared.status = MatchStatus::NoOfficialEntry;
                prepared.display = prepared.g.form().to_string();
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
            if let Some(m) = p.matched {
                insert_official_byform_aliases(&mut isv_to_id, &official_entries, m.entry, p.id);
            }
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
    let mut official_only_records: Vec<OfficialOnlyRecord> = Vec::new();
    for e in &official_entries {
        let byforms: Vec<String> = e.citation_byforms().into_iter().map(|b| b.form).collect();
        let Some(display) = byforms.first().cloned() else {
            continue;
        };
        if !covered.insert(e.id.clone()) {
            continue; // this exact official sense already has a generated page
        }
        let entry_id = entry_ids.alloc();
        official_only += 1;
        official_only_records.push(OfficialOnlyRecord {
            id: entry_id,
            entry: e.clone(),
            display,
            byforms,
        });
    }
    for record in &official_only_records {
        for byform in &record.byforms {
            isv_to_id.insert(byform, record.id);
        }
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
    // Computed false-friend notes (replaces the retired curated
    // data/semantic-notes.json): detected from the same evidence caches that
    // are already in memory, then shared by api/notes.json, the English API
    // candidates, and the checker index below.
    let ff_notes = crate::falsefriends::compute(
        &official_entries,
        Some(&corpus),
        raw_corpus.as_ref(),
        enrich.as_ref(),
    );
    println!(
        "false-friends: {} computed notes ({} collisions) from cache surface × gloss divergence.",
        ff_notes.len(),
        ff_notes.values().map(|n| n.collisions.len()).sum::<usize>(),
    );
    // Raw-collision display credit census (issue #86 item 6).
    println!(
        "raw-credit: {} entries show {} fold-deduped raw attestations (display-only, issue #86).",
        raw_plan.credit.len(),
        raw_plan.credit.values().map(Vec::len).sum::<usize>(),
    );
    let render_context = RenderContext {
        enrich: enrich.as_ref(),
        xref: Some(&xref),
        raw_xref: &raw_plan.xref,
    };
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
        let prior = calibration
            .as_ref()
            .map(|c| c.probability(p.g.score, p.g.n_langs));
        let mut meta = entry_meta(SiteEntryInput {
            id: p.id,
            title: &p.display,
            gloss: p
                .matched
                .map(|m| official_entries[m.entry].english.as_str())
                .unwrap_or(&p.g.set.gloss),
            pos: p
                .matched
                .map(|m| &official_entries[m.entry])
                .map(|e| {
                    if crate::aspect::aspect(&e.pos_raw).is_some() {
                        "verb"
                    } else {
                        e.pos.code()
                    }
                })
                .unwrap_or_else(|| p.g.set.pos.code()),
            confidence: p.g.confidence,
            score: p.g.score,
            probability: if p.matched.is_some() { None } else { prior },
            n_languages: p.g.n_langs,
            n_branches: p.g.n_branches,
            borrowed: p.g.set.borrowed,
            official_only: false,
            official_lemma: p.matched.map(|_| p.display.clone()),
            ancestor,
            languages: langs,
            wiki_categories,
        });
        if let Some(m) = p.matched {
            meta.prior = prior;
            meta.official_sense_id = Some(official_entries[m.entry].id.clone());
        }
        metas.push(meta);
    }
    for record in &official_only_records {
        let oid = record.id;
        let e = &record.entry;
        let isv = record.display.as_str();
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
        let mut meta = entry_meta(SiteEntryInput {
            id: oid,
            title: isv,
            gloss: &e.english,
            pos: if crate::aspect::aspect(&e.pos_raw).is_some() {
                "verb"
            } else {
                e.pos.code()
            },
            confidence: Confidence::High,
            score: 1.0,
            probability: None,
            n_languages: langs.len(),
            n_branches: branches.len(),
            borrowed: e.genesis.trim() == "I",
            official_only: true,
            official_lemma: Some(isv.to_string()),
            ancestor: String::new(),
            languages: langs,
            wiki_categories,
        });
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
    for record in &official_only_records {
        official_page_ids.insert(record.entry.id.clone(), record.id);
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
    let build_meta = BuildMeta::current(metas.len(), lemma_total)?;
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
            ipf.primary_citation_byform()
                .unwrap_or_else(|| ipf.isv.trim().to_string()),
            pf.primary_citation_byform()
                .unwrap_or_else(|| pf.isv.trim().to_string()),
            prediction,
        ));
    }
    println!(
        "aspect model: generated {} ipf↔pf pairs through the production pair path (issue #75).",
        aspect_pair_exports.len()
    );

    write_wiki_indexes(WikiIndexInput {
        out_dir,
        entries: &metas,
        edges: &edges,
        backlinks: &backlinks,
        homographs: &homographs,
        build: &build_meta,
        curation: &curation,
        rule_index: &rule_index,
        proto: proto_index.as_ref(),
        proto_reflex: &proto_reflex,
    })?;
    // Some special pages intentionally probe inflection failures. Keep the final
    // export note about blank cells limited to the actual entry pages rendered below.
    crate::forms::reset_inflection_panic_count();

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
            Some(_) => synonyms_block(&p.display, &thesaurus, &isv_to_id),
            None => String::new(),
        };
        // Word-formation family from the display headword: the official lemma
        // with its OFFICIAL part of speech when matched (the form-only match can
        // cross POS), else the reconstruction — marked as such in the block.
        let derivation = match matched_entry {
            Some(e) => derivation_block(&p.display, e.pos, &isv_to_id, true, p.id, &mut deriv_rows),
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
        let html = corpus_entry_page(CorpusEntryInput {
            id: p.id,
            generated: &p.g,
            status: p.status,
            official: p.matched.map(|m| {
                let e = &official_entries[m.entry];
                (m.rank, p.display.as_str(), e.english.as_str())
            }),
            official_grammar: official_pg,
            official_display: official_disp,
            family: &family,
            synonyms: &synonyms,
            derivation: &derivation,
            wiki_top: &wiki_top,
            meta,
            razum_codes: &razum_codes,
            raw_credit: &raw_credit_line(raw_plan.credit.get(&p.id)),
            wiki_bottom: &wiki_bottom,
            proto_link: &proto_link,
            context: &render_context,
        });
        std::fs::write(entry_dir.join(format!("{}.html", p.id)), html)?;

        let mut keys = search_keys(&p.g.candidates, &p.display);
        // On an official-headword (matched) entry, make the official English gloss
        // searchable too — it is already searchable on official-only pages, so
        // this closes the parity gap without touching the entry HTML.
        if let Some(e) = matched_entry {
            let byforms: Vec<String> = e.citation_byforms().into_iter().map(|b| b.form).collect();
            add_official_byform_keys(
                &mut keys,
                byforms.iter().map(String::as_str),
                &p.display,
                p.matched.map(|m| m.rank).unwrap_or(1),
            );
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
    for record in &official_only_records {
        let oid = record.id;
        let e = &record.entry;
        let isv = record.display.as_str();
        let fold = crate::orthography::to_standard(&isv.to_lowercase());
        let syn = synonyms_block(isv, &thesaurus, &isv_to_id);
        let deriv = derivation_block(isv, e.pos, &isv_to_id, true, oid, &mut deriv_rows);
        let meta = meta_by_id.get(&oid).expect("official-only entry meta");
        let wiki_top = entry_tabs(meta) + &homograph_notice(meta, &homographs);
        let wiki_bottom = entry_wiki_blocks(
            meta,
            backlinks.get(&oid).map(Vec::as_slice).unwrap_or(&[]),
            &edges,
            &curation,
            &build_meta,
        );
        let html = official_only_page(OfficialEntryInput {
            isv,
            entry: e,
            id: oid,
            synonyms: &syn,
            derivation: &deriv,
            wiki_top: &wiki_top,
            meta,
            raw_credit: &raw_credit_line(raw_plan.credit.get(&oid)),
            wiki_bottom: &wiki_bottom,
            context: &render_context,
        });
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
        add_official_byform_keys(&mut keys, record.byforms.iter().map(String::as_str), isv, 1);
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
            id: oid,
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
            id: oid,
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
                    let mut m = entry_meta(SiteEntryInput {
                        id,
                        title: &display,
                        gloss: &gloss,
                        pos: &lemma.pos,
                        confidence: Confidence::Low,
                        score: 0.0,
                        probability: None,
                        n_languages: 1,
                        n_branches: 1,
                        borrowed: false,
                        official_only: false,
                        official_lemma: None,
                        ancestor: String::new(),
                        languages: vec![lemma.lang.clone()],
                        wiki_categories: Vec::new(),
                    });
                    m.raw = true;
                    m
                };
                let html = raw_lemma_page(RawEntryInput {
                    display: &display,
                    lemma,
                    id,
                    meta: &meta,
                    gloss_xref: &gx,
                    context: &render_context,
                });
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
        let prob = cal.probability(p.g.score, p.g.n_langs);
        if prob >= crate::calibrate::REVIEW_T {
            // V12 item 3: a proposal one edit away from a gloss+POS-matched
            // official byform is a reconstruction near-miss, not a novel
            // word — reclassify with the official lemma cited.
            let near = near_official_match(form, p.g.set.pos, &p.g.set.gloss, &official_entries);
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
                classification: if near.is_some() {
                    "near-official"
                } else {
                    "novel"
                },
                official_lemma: near.unwrap_or_default(),
            });
        }
    }
    proposals.sort_by(|a, b| b.prob.total_cmp(&a.prob).then(a.id.cmp(&b.id)));
    let n_near = proposals
        .iter()
        .filter(|r| r.classification == "near-official")
        .count();
    println!(
        "proposals: {} rows ({} truly novel, {n_near} near-official reconstruction diagnostics)",
        proposals.len(),
        proposals.len() - n_near,
    );
    let mut tsv = String::from(
        "form\tpos\tprobability\tbucket\tancestor\tn_langs\tn_branches\tgloss\tclassification\tofficial\n",
    );
    for r in &proposals {
        // Buckets are only meaningful in calibrated-probability space.
        let bucket = if r.prob >= crate::calibrate::PROPOSE_T {
            "predlog"
        } else {
            "pregled"
        };
        let _ = writeln!(
            tsv,
            "{}\t{}\t{:.3}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            r.form,
            r.pos,
            r.prob,
            bucket,
            r.ancestor,
            r.n_langs,
            r.n_branches,
            r.gloss.replace(['\t', '\n'], " "),
            r.classification,
            r.official_lemma,
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
        let (headwords, status, gloss): (Vec<String>, &'static str, String) = match p.matched {
            Some(m) => {
                let e = &official_entries[m.entry];
                let mut headwords: Vec<String> =
                    e.citation_byforms().into_iter().map(|b| b.form).collect();
                if let Some(i) = headwords.iter().position(|form| form == &p.display) {
                    headwords.swap(0, i);
                }
                (headwords, "official", e.english.clone())
            }
            None => (
                vec![p.g.form().to_string()],
                "generated",
                p.g.set.gloss.clone(),
            ),
        };
        let prob = if status == "generated" {
            calibration
                .as_ref()
                .map(|c| c.probability(p.g.score, p.g.n_langs))
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
        for headword in headwords {
            if headword.is_empty() || headword.contains('!') {
                continue;
            }
            // Sanitize the citation: generated forms can carry raw pipeline
            // notation ("pleskati,*plěskati"), official ones government hints
            // ("pozirati (na)") — neither belongs in a lookup key.
            let Some(headword) = crate::forms::citation(&headword) else {
                continue;
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
    }
    for record in &official_only_records {
        let oid = record.id;
        let e = &record.entry;
        let api_pos = if crate::aspect::aspect(&e.pos_raw).is_some() {
            crate::model::Pos::Verb
        } else {
            e.pos
        };
        // ~230 rows list byform variants in one cell ("iměti, imati"): each
        // variant is its own lemma (and gets its own paradigm).
        for isv in &record.byforms {
            let Some(clean) = crate::forms::citation(isv) else {
                continue;
            };
            let isv = clean.as_str();
            lemma_sink.add(
                isv,
                "",
                isv,
                oid,
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
                oid,
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
                    oid,
                    "official-only",
                    None,
                    &e.english,
                );
                crate::forms::pronoun_numeral_records(
                    &mut form_sink,
                    isv,
                    api_pos,
                    oid,
                    "official-only",
                    &e.english,
                );
            }
            attested_bases.push((isv.to_string(), api_pos, oid, e.english.clone()));
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
    // ---- Borrowed internationalisms recovered from RAW attestations (2e) ----
    // Cognate sets the evidence gate never saw (no etymology section on any
    // member — the teleport family): ≥2 languages / ≥2 branches sharing an
    // international shape AND a gloss token, run through the ordinary
    // pipeline with is_intl_meaning. `generated`, `borrowed`, NO paradigm,
    // NO probability (no calibrator for this path — fail closed), and never
    // fed to build_sets or any benchmark.
    // V11 item 5: per-bucket Wilson-95 probabilities from the leakage-free
    // genesis=I holdout, computed before the production (deduped) pass.
    let raw_intl_probs = raw_corpus
        .as_ref()
        .map(|rc| raw_intl_probabilities(&rc.lemmas, &official_entries))
        .unwrap_or_default();
    if !raw_intl_probs.is_empty() {
        let stats: Vec<String> = raw_intl_probs
            .iter()
            .map(|((l, b), p)| format!("{l}l{b}b={p:.2}"))
            .collect();
        println!(
            "raw-intl calibration (Wilson-95 by langs/branches): {}",
            stats.join(" ")
        );
    }
    let raw_intl = raw_corpus
        .as_ref()
        .map(|rc| raw_intl_candidates(&rc.lemmas, &mut taken, &raw_intl_probs))
        .unwrap_or_default();
    for c in &raw_intl {
        // entry_id 0 is the established "no entry page" sentinel (the raw
        // pages that attest these words are not entries.json rows, and the
        // linguistic-logic CI guard requires nonzero ids to resolve there).
        // The attestation evidence rides in the provenance tag, which the
        // English API parses back out.
        let mut feat_list = vec![format!("raw-intl:{}l:{}", c.langs.len(), c.branch_pattern)];
        if let Some(noun) = &c.deriv_of {
            // V11 item 4: derivational completion — provenance points at the
            // recovered noun; the regular present stem rides along.
            feat_list.push(format!("deriv:intl-ovati←{noun}"));
            if let Some(stem) = &c.present_stem {
                feat_list.push(format!("pres:{stem}"));
            }
        }
        for feats in &feat_list {
            for sink in [&mut lemma_sink, &mut form_sink] {
                sink.add(
                    &c.form,
                    feats,
                    &c.form,
                    0,
                    c.pos.code(),
                    "lemma",
                    "generated",
                    c.probability,
                    &c.gloss,
                );
            }
        }
    }
    println!(
        "raw-intl: {} borrowed internationalism candidates recovered from raw attestations (2e).",
        raw_intl.len()
    );
    let form_records = form_sink.into_records();
    let lemma_records = lemma_sink.into_records();
    // Computed false-friend notes for the web text-checker (the CLI computes
    // the same records), keyed by folded form and SHARDED like the suggest
    // index (V11 item 6): api/notes/<n>.json via fnv1a32(key) % 64, with a
    // frozen router selftest. The retired monolithic api/notes.json is
    // removed so stale copies can't shadow the shards.
    let notes_dir = out_dir.join("api").join("notes");
    let _ = std::fs::remove_dir_all(&notes_dir);
    std::fs::create_dir_all(&notes_dir)?;
    let _ = std::fs::remove_file(out_dir.join("api").join("notes.json"));
    let mut notes_bytes = 0usize;
    {
        let mut shards: std::collections::BTreeMap<
            u32,
            std::collections::BTreeMap<&String, &crate::falsefriends::Note>,
        > = std::collections::BTreeMap::new();
        for (key, note) in &ff_notes {
            shards
                .entry(crate::forms::fnv1a32(key) % crate::falsefriends::NOTES_SHARDS)
                .or_default()
                .insert(key, note);
        }
        for shard in 0..crate::falsefriends::NOTES_SHARDS {
            let body = serde_json::json!({
                "schema_version": crate::falsefriends::NOTES_SCHEMA_VERSION,
                "shard": shard,
                "notes": shards.get(&shard).cloned().unwrap_or_default(),
            });
            let body = serde_json::to_string(&body)? + "\n";
            notes_bytes += body.len();
            std::fs::write(notes_dir.join(format!("{shard}.json")), body)?;
        }
        let samples: Vec<serde_json::Value> = crate::falsefriends::NOTES_SELFTEST_SAMPLES
            .iter()
            .map(|k| {
                serde_json::json!([
                    k,
                    crate::forms::fnv1a32(k) % crate::falsefriends::NOTES_SHARDS
                ])
            })
            .collect();
        let st = serde_json::json!({
            "schema_version": crate::falsefriends::NOTES_SCHEMA_VERSION,
            "shards": crate::falsefriends::NOTES_SHARDS,
            "router": "fnv1a32(utf8(folded_key)) % shards",
            "samples": samples,
        });
        let st = serde_json::to_string(&st)? + "\n";
        notes_bytes += st.len();
        std::fs::write(out_dir.join("api").join("notes-selftest.json"), st)?;
    }
    let aspect_api: crate::forms::AspectMeta = metas
        .iter()
        .filter_map(|m| {
            m.aspect
                .as_ref()
                .map(|a| (m.id, (a.clone(), m.aspect_partners.clone())))
        })
        .collect();
    // Ranking evidence per entry id (schema-4 / en-schema-2 plumbing): the
    // official CSV frequency joined via the entry's official sense id, plus
    // the attestation metadata already on SiteEntryMeta.
    let freq_by_sense: std::collections::HashMap<&str, f32> = official_entries
        .iter()
        .filter_map(|e| e.frequency.map(|f| (e.id.as_str(), f)))
        .collect();
    let rank_evidence: std::collections::BTreeMap<usize, crate::forms::RankEvidence> = metas
        .iter()
        .map(|m| {
            (
                m.id,
                crate::forms::RankEvidence {
                    frequency: m
                        .official_sense_id
                        .as_deref()
                        .and_then(|sid| freq_by_sense.get(sid).copied()),
                    langs: m.n_langs,
                    branch_pattern: navigation::branch_pattern(&m.languages),
                    borrowed: m.borrowed,
                },
            )
        })
        .collect();
    let english_counts = english_api::write_en_api(
        out_dir,
        &lemma_records,
        &metas,
        &aspect_api,
        &ff_notes,
        &rank_evidence,
        &build_meta.git,
    )?;
    println!(
        "english api: {} keys / {} candidate records across {} shards ({} KB total, largest shard {} KB)",
        english_counts.keys,
        english_counts.candidates,
        english_api::EN_SHARDS,
        english_counts.bytes / 1024,
        english_counts.largest_shard / 1024,
    );
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
    let checker_index = crate::check::build_index(
        &official_entries,
        Some(std::path::Path::new("data/novel-words.tsv")),
        ff_notes.clone(),
    );
    let suggest_bytes = crate::check::write_web_suggestions(out_dir, &checker_index)?;
    let api_counts = crate::forms::write_api(
        out_dir,
        &form_records,
        &lemma_records,
        &aspect_api,
        &rank_evidence,
        ff_notes.len(),
        pair_json.len() + suggest_bytes + english_counts.bytes + notes_bytes,
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
    let home = corpus_home(CorpusHomeInput {
        entries: n,
        lemmas: lemma_total,
        high,
        medium: med,
        low,
        official,
        official_only,
        borrowed,
        rows: &rows,
    });
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
        // The metrics page documents the PIPELINE calibrator
        // (score-calibration.json) — not the corpus one (a V11 wiring slip
        // had it rendering corpus fit stats under the pipeline heading).
        metrics_page(
            crate::calibrate::Calibration::load_for_domain(
                Path::new(crate::calibrate::PATH),
                crate::calibrate::PIPELINE_SCORE_DOMAIN,
            )?
            .as_ref(),
        ),
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

    let panics = crate::forms::inflection_panic_count();
    println!(
        "wrote {n} cognate-word pages + {official_only} official-only pages ({high} high / {med} medium / {low} low confidence; {official} match an official ISV form){}",
        if panics > 0 { format!("; {panics} inflection cells blank") } else { String::new() }
    );
    Ok(())
}

mod assets;
mod coverage;
mod english_api;
mod entries;
mod layout;
mod model;
mod navigation;
mod search;
mod special;

pub use self::coverage::run_coverage;
pub use self::english_api::{
    english_gloss_tokens, run_en_batch, run_en_lookup, run_translation_probe, PROBE_FILE,
};

#[cfg(test)]
mod tests;
