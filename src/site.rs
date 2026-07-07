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
    Animacy as IsvAnimacy, Case as IsvCase, Gender as IsvGender, Number as IsvNumber,
    Person as IsvPerson, Tense as IsvTense, ISV,
};
use std::fmt::Write as _;
use std::path::Path;

/// Counts inflection-table panics swallowed by the quiet hook (see below).
static INFLECTION_PANICS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

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
    let proto_index = if proto_path.exists() {
        crate::dump::ProtoIndex::load(proto_path).ok()
    } else {
        None
    };
    let proto = proto_index.as_ref();
    if proto.is_some() {
        println!("Using Proto-Slavic cache for reconstruction-derived forms.");
    }

    let entry_dir = out_dir.join("entry");
    std::fs::create_dir_all(&entry_dir)?;

    // Streaming pass: render each entry, accumulate the search index + stats.
    let mut search = String::from("[\n");
    let mut first_search = true;
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
        let g = generator::generate(&input, official, proto, &cfg, &overrides);
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
        let html = entry_page(id, entry, &g, &evidence);
        std::fs::write(entry_dir.join(format!("{id}.html")), html)?;

        // search index row: [id, form, gloss, pos, statuschar]
        let statuschar = match g.match_status {
            MatchStatus::OfficialMatch => "O",
            MatchStatus::DiffersFromOfficial => "D",
            MatchStatus::NoOfficialEntry => "N",
        };
        if !first_search {
            search.push_str(",\n");
        }
        first_search = false;
        // row: [id, form, gloss, pos, statuschar, strengthLetter, score, keys]
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
        let _ = write!(
            search,
            "[{},{},{},{},{},{},{:.2},{}]",
            id,
            json_str(&form),
            json_str(&truncate(&entry.english, 70)),
            json_str(&entry.pos.code()),
            json_str(statuschar),
            json_str(conf_letter(top.confidence)),
            top.score,
            keys_json(&keys),
        );
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
        });
    }
    search.push_str("\n]\n");

    std::fs::write(out_dir.join("search.json"), search)?;
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

/// Generate the static site from the Wiktionary cognate-set corpus. Every set of
/// etymologically-connected Slavic lemmas becomes one Interslavic word, with
/// confidence scaling by how many languages/branches attest it.
pub fn export_corpus(lemmas_path: &Path, out_dir: &Path) -> Result<()> {
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
    let enrich = crate::enrich::EnrichIndex::load(Path::new(crate::DEFAULT_ENRICH_CACHE)).ok();
    if let Some(e) = &enrich {
        println!(
            "Loaded {} native-Wiktionary enrichment entries (RU/PL/CS).",
            e.len()
        );
    } else {
        println!("(no enrichment cache — run extract-enrich for native etymology/links)");
    }

    let official_entries = official::load(Path::new(crate::DEFAULT_OFFICIAL)).unwrap_or_default();
    let mut official_map: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    for e in &official_entries {
        let isv = e.isv.trim();
        if isv.is_empty() || isv.contains(' ') || isv.contains('#') {
            continue;
        }
        official_map
            .entry(crate::orthography::to_standard(&isv.to_lowercase()))
            .or_insert_with(|| (isv.to_string(), e.english.clone()));
    }

    let entry_dir = out_dir.join("entry");
    let _ = std::fs::remove_dir_all(&entry_dir); // clear any stale pages
    std::fs::create_dir_all(&entry_dir)?;

    let mut search = String::from("[\n");
    let mut first_search = true;
    let mut rows: Vec<HomeRow> = Vec::new();
    let (mut n, mut high, mut med, mut low, mut official, mut borrowed) = (0usize, 0, 0, 0, 0, 0);
    let mut lemma_total = 0usize;
    // Folded spellings covered by any generated candidate, so official-only
    // pages are emitted exactly for the rest.
    let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();

    // First pass: generate every word, so ancestor families (shared proto stem
    // or loan etymon) can be cross-linked before any page is rendered.
    struct Prepared {
        id: usize,
        g: crate::corpus::GeneratedWord,
        display: String,
        status: MatchStatus,
        matched: Option<(usize, String, String)>,
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
    let mut id = 0usize;
    for set in sets {
        let members = set.members.len();
        let g = crate::corpus::generate_set(set, &cfg);
        let form = g.form().to_string();
        if form.is_empty() {
            continue;
        }
        id += 1;
        n += 1;
        lemma_total += members;
        if g.set.borrowed {
            borrowed += 1;
        }
        match g.confidence {
            Confidence::High => high += 1,
            Confidence::Medium => med += 1,
            Confidence::Low => low += 1,
        }
        // Authoritative match: ANY ranked candidate reproducing an official
        // lemma (folded) puts the entry under the official headword.
        let matched: Option<(usize, String, String)> =
            g.candidates.iter().take(5).enumerate().find_map(|(i, c)| {
                official_map
                    .get(&crate::orthography::to_standard(&c.form.to_lowercase()))
                    .map(|(isv, en)| (i + 1, isv.clone(), en.clone()))
            });
        for c in g.candidates.iter().take(5) {
            covered.insert(crate::orthography::to_standard(&c.form.to_lowercase()));
        }
        if matched.is_some() {
            official += 1;
        }
        let status = if matched.is_some() {
            MatchStatus::OfficialMatch
        } else {
            MatchStatus::NoOfficialEntry
        };
        let display = matched
            .as_ref()
            .map(|(_, isv, _)| isv.clone())
            .unwrap_or_else(|| form.clone());
        prepared.push(Prepared {
            id,
            g,
            display,
            status,
            matched,
        });
    }

    // Word families: entries whose ancestors share a Proto-Slavic stem
    // (*starъ/*starostь/*starьcь) or the same loan etymon (la magister →
    // majstor/maestro/magistr) cross-link each other.
    let mut families: std::collections::BTreeMap<String, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (i, p) in prepared.iter().enumerate() {
        if let Some(k) = family_key(&p.g.set) {
            families.entry(k).or_default().push(i);
        }
    }

    // Reverse index for intra-site cross-linking: every cognate member of every
    // entry points back to that entry's page, so an enrichment chip (related /
    // synonym / antonym term) that is itself a dictionary headword links to the
    // internal page instead of out to Wiktionary — turning the per-entry
    // enrichment into a site-wide semantic graph.
    let mut xref = crate::enrich::Xref::new();
    for p in &prepared {
        for m in &p.g.set.members {
            xref.insert(&m.lang, &m.word, p.id);
        }
    }
    println!(
        "Built {} cognate cross-reference keys for intra-site links.",
        xref.len()
    );

    // Second pass: render pages (with family links) + the search index.
    for (i, p) in prepared.iter().enumerate() {
        let family = family_block(i, &prepared, &families);
        let html = corpus_entry_page(
            p.id,
            &p.g,
            p.status,
            p.matched
                .as_ref()
                .map(|(r, isv, en)| (*r, isv.as_str(), en.as_str())),
            &family,
            enrich.as_ref(),
            Some(&xref),
        );
        std::fs::write(entry_dir.join(format!("{}.html", p.id)), html)?;

        if !first_search {
            search.push_str(",\n");
        }
        first_search = false;
        let mut keys = search_keys(&p.g.candidates, &p.display);
        // On an official-headword (matched) entry, make the official English gloss
        // searchable too — it is already searchable on official-only pages, so
        // this closes the parity gap without touching the entry HTML.
        if let Some((_, _, en)) = &p.matched {
            for tok in crate::dump::gloss_tokens(en) {
                if tok.chars().count() >= 3 && !keys.iter().any(|(k, _)| k == &tok) {
                    keys.push((tok, 6));
                }
            }
        }
        let _ = write!(
            search,
            "[{},{},{},{},{},{},{:.2},{}]",
            p.id,
            json_str(&p.display),
            json_str(&truncate(&p.g.set.gloss, 70)),
            json_str(p.g.set.pos.code()),
            json_str(if p.matched.is_some() { "O" } else { "N" }),
            json_str(conf_letter(p.g.confidence)),
            p.g.score,
            keys_json(&keys),
        );
        rows.push(HomeRow {
            // sort the home list by coverage (n_langs) so the best-attested show first
            freq: p.g.n_langs as f32 + p.g.n_branches as f32 / 10.0,
            id: p.id,
            form: p.display.clone(),
            gloss: p.g.set.gloss.clone(),
            pos: p.g.set.pos.code().to_string(),
            status: p.status,
            conf: p.g.confidence,
            score: p.g.score,
        });
    }

    // Official lemmas no candidate generates: still searchable, clearly badged
    // as official-but-not-yet-derivable, with the official cognate evidence.
    // Multi-word lemmas (`pęt na desęte`) and reflexives (`… sę`) are included
    // (the single-token generator never produces them, so they would otherwise
    // have no page at all) — display-only parity, generation is untouched.
    let mut official_only = 0usize;
    for e in &official_entries {
        let isv = e.isv.trim();
        if isv.is_empty() || isv.contains('#') {
            continue;
        }
        let fold = crate::orthography::to_standard(&isv.to_lowercase());
        if !covered.insert(fold.clone()) {
            continue; // generated, or an official homograph already emitted
        }
        id += 1;
        official_only += 1;
        let html = official_only_page(isv, e, enrich.as_ref(), Some(&xref), id);
        std::fs::write(entry_dir.join(format!("{id}.html")), html)?;
        let mut keys: Vec<(String, usize)> = Vec::new();
        for k in [fold.clone(), crate::orthography::ascii_skeleton(isv)] {
            if k.chars().count() >= 2
                && k != isv.to_lowercase()
                && !keys.iter().any(|(kk, _)| kk == &k)
            {
                keys.push((k, 1));
            }
        }
        if !first_search {
            search.push_str(",\n");
        }
        first_search = false;
        let _ = write!(
            search,
            "[{},{},{},{},{},{},{:.2},{}]",
            id,
            json_str(isv),
            json_str(&truncate(&e.english, 70)),
            json_str(&e.pos.code()),
            json_str("O"),
            json_str("V"),
            1.0,
            keys_json(&keys),
        );
    }
    search.push_str("\n]\n");

    std::fs::write(out_dir.join("search.json"), search)?;
    std::fs::write(out_dir.join("wiktionary.css"), css())?;
    std::fs::write(out_dir.join(".nojekyll"), "")?;

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
    std::fs::write(
        out_dir.join("about.html"),
        corpus_about(n, lemma_total, official),
    )?;

    let panics = INFLECTION_PANICS.load(std::sync::atomic::Ordering::Relaxed);
    println!(
        "wrote {n} cognate-word pages + {official_only} official-only pages ({high} high / {med} medium / {low} low confidence; {official} match an official ISV form){}",
        if panics > 0 { format!("; {panics} inflection cells blank") } else { String::new() }
    );
    Ok(())
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
        "<section><h2 id='rodina'>Rodina slov</h2>\
           <p class='muted'>Slova iz toj že etimologičnoj rodiny ({label}):</p>\
           <ul class='compact-list'>{items}</ul>\
         </section>"
    )
}

fn corpus_entry_page(
    id: usize,
    g: &crate::corpus::GeneratedWord,
    status: MatchStatus,
    official: Option<(usize, &str, &str)>,
    family: &str,
    enrich: Option<&crate::enrich::EnrichIndex>,
    xref: Option<&crate::enrich::Xref>,
) -> String {
    let top = g.candidates.first().unwrap();
    let pos_code = g.set.pos.code();
    // The official lemma is the authoritative headword when any candidate
    // reproduces it; the generated form stays visible as the reconstruction.
    let headword = official
        .map(|(_, isv, _)| isv.to_string())
        .unwrap_or_else(|| top.form.clone());
    let coverage = format!(
        "<span class='reliability {}'>uvěrjenost: {}</span> <span class='muted'>({} językov, {} větvi)</span>",
        conf_class(g.confidence),
        g.confidence.label(),
        g.n_langs,
        g.n_branches
    );
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
    let headline = format!(
        "<div class='headword-block'>
           <div class='headmeta'>
             <span class='badge pos'>{}</span>
             <span class='pill {}'>{}</span>
             {coverage}
             {}
           </div>
           <p class='def'><b>Smysl:</b> {}</p>
           {recon_line}
         </div>",
        esc(&pos_heading(g.set.pos.code())),
        source_class(top.source),
        esc(top.source.label()),
        status_pill(status),
        esc(&gloss),
    );

    let official_note = match official {
        Some((1, isv, _)) => {
            if crate::orthography::exact_match(&top.form, isv) {
                "Oficialna forma; rekonstrukcija ju <b>točno</b> reproduktuje.".to_string()
            } else {
                "Oficialna forma; rekonstrukcija ju reproduktuje (normalizovano — pravopisne znaky sę različajų).".to_string()
            }
        }
        Some((r, _, _)) => {
            format!("Oficialna forma; generator ju daje kako <a href='#cand-{r}'>kandidat {r}</a>.")
        }
        None => "Forma je generovana iz cognatov; ne v oficialnom slovniku.".to_string(),
    };
    let banner = format!(
        "<div class='banner {}'><b>Podpŕto {} językami v {} slovjanskyh větvah.</b> {}</div>",
        match g.confidence {
            Confidence::High => "ok",
            Confidence::Medium => "warn",
            Confidence::Low => "info",
        },
        g.n_langs,
        g.n_branches,
        official_note,
    );

    let etymology = if g.set.borrowed {
        format!(
            "<p>Internacionalizm (zaimka). Etimon: <span class='mention'>{}</span>. Niže sų slovjanske refleksy toj že korene.</p>",
            esc(&etymon_display(&g.set.etymon))
        )
    } else {
        format!(
            "<p>Iz praslovjanskogo <a class='mention' href='https://en.wiktionary.org/wiki/Reconstruction:Proto-Slavic/{p}'>*{p}</a>.</p>",
            p = esc(g.set.proto.trim_start_matches('*')),
        )
    };

    let inflection = inflection_table(&headword, pos_code);
    let cognates = cognate_block(g, enrich);
    let enrich_members: Vec<(String, String)> = g
        .set
        .members
        .iter()
        .map(|m| (m.lang.clone(), m.word.clone()))
        .collect();
    let native_etym = enrich
        .map(|e| enrich_etymology_section(&enrich_members, e))
        .unwrap_or_default();
    let native_conn = enrich
        .map(|e| enrich_connections_section(&enrich_members, e, xref, id))
        .unwrap_or_default();
    let alternatives = alternatives_block(&g.candidates);
    let trace = trace_block(top);
    let foot = if official.is_some() {
        "Oficialne slovo; rekonstrukcija i dokazy mašinno generovane (Wiktionary, CC BY-SA)."
    } else {
        "Mašinno generovana rekonstrukcija iz cognatov (Wiktionary, CC BY-SA). Ne oficialny standard."
    };
    let body = format!(
        "<article class='entry'>\
           <h1 class='page-title firstHeading'>{headword}</h1>\
           {banner}{headline}\
           <section><h2 id='formy'>Formy i kandidaty</h2>{alternatives}</section>\
           <section><h2 id='pregibanje'>Prěgibanje</h2>{inflection}</section>\
           <section><h2 id='cognaty'>Cognaty — {nlangs} językov</h2>{cognates}</section>\
           <section><h2 id='etimologija'>Etimologija</h2>{etymology}</section>\
           {native_etym}{native_conn}{family}\
           <section><h2 id='sled'>Sled pravil</h2>{trace}</section>\
           <p class='foot'>{foot}</p>\
         </article>",
        headword = esc(&headword),
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
    id: usize,
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
    let native_etym = enrich
        .map(|ix| enrich_etymology_section(&enrich_members, ix))
        .unwrap_or_default();
    let native_conn = enrich
        .map(|ix| enrich_connections_section(&enrich_members, ix, xref, id))
        .unwrap_or_default();
    let mut cog = String::new();
    if !evidence.is_empty() {
        cog.push_str("<table class='wikitable compact-table'><tbody>");
        for ev in &evidence {
            let _ = write!(
                cog,
                "<tr><td class='lc'>{}</td><td>{}</td></tr>",
                esc(&ev.lang_name),
                esc(&ev.form)
            );
        }
        cog.push_str("</tbody></table>");
    } else {
        cog.push_str("<p class='muted'>Bez slovjanskogo cognatnogo dokaza v slovniku.</p>");
    }
    let inflection = inflection_table(isv, e.pos.code());
    let body = format!(
        "<article class='entry'>\
           <h1 class='page-title firstHeading'>{isv}</h1>\
           <div class='banner info'><b>Oficialne slovo.</b> Generator ješče ne izvodi jego iz cognatnogo dokaza (redky korenj, mnogoslovny izraz ili redakcijny izbor).</div>\
           <div class='headword-block'><div class='headmeta'><span class='badge pos'>{pos}</span> <span class='pill src-official'>oficialny slovnik</span></div>\
             <p class='def'><b>Smysl:</b> {en}</p></div>\
           <section><h2 id='pregibanje'>Prěgibanje</h2>{inflection}</section>\
           <section><h2 id='cognaty'>Slovjanski dokaz</h2>{cog}</section>\
           {native_etym}{native_conn}\
           <p class='foot'>Oficialne slovo: interslavic-dictionary.com. Prěgibanje mašinno generovano.</p>\
         </article>",
        isv = esc(isv),
        pos = esc(&pos_heading(e.pos.code())),
        en = esc(&e.english),
    );
    page(&format!("{isv} — medžuslovjansky"), &body, 1)
}

/// The full search-results page (search.html). Reads `?q=` and lists every match;
/// the header search box (present on every page) submits here on Enter.
fn search_page() -> String {
    let body = "<article class='entry search-page'>\
      <h1 class='firstHeading'>Iskanje</h1>\
      <p class='muted'>Napiši v polje gore i pritisni <b>Enter</b>. Najdeno: <b id='rescount'>0</b> rezultatov.</p>\
      <div id='page-results' class='results full'></div>\
    </article>";
    page("Iskanje — medžuslovjansky", body, 0)
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
            .filter(|m| crate::corpus::branch_of(&m.lang) == Some(branch))
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
                .map(|x| truncate(x, 44))
                .unwrap_or_else(|| truncate(&m.gloss, 32));
            let _ = write!(
                s,
                "<tr><td class='lc'>{}</td><td><a href='https://en.wiktionary.org/wiki/{}#{}'>{}</a>{}</td><td class='muted'>{}</td></tr>",
                esc(&crate::lang::lang_name(&m.lang)),
                esc(&m.word.replace(' ', "_")),
                esc(&m.lang),
                esc(&m.word),
                native,
                esc(&gloss),
            );
        }
        s.push_str("</tbody></table></div>");
    }
    s.push_str("</div>");
    s
}

/// Multi-source native etymology (RU / PL / CS Wiktionary) — one etymology per
/// edition, side by side, so each entry carries three independent histories.
/// `members` is a list of `(lang_code, word)` cognates.
fn enrich_etymology_section(
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
            .map(|p| format!("<p>{}</p>", esc(p)))
            .collect();
        let _ = write!(
            rows,
            "<div class='etym-src'><div class='src-head'><span class='lc'>{}</span> <a class='ext' href='{}'>{}↗</a></div>{}</div>",
            esc(&crate::lang::lang_name(lang)),
            esc(&crate::enrich::source_url(lang, word)),
            esc(word),
            paras
        );
    }
    if rows.is_empty() {
        return String::new();
    }
    format!(
        "<section><h2 id='etym-nar'>Etimologija po narodnyh slovnikah (RU / PL / CS)</h2>\
         <div class='etym-sources'>{rows}</div>\
         <p class='muted'>Iz narodnyh Wiktionary (ru/pl/cs), CC BY-SA — različne pogledy na istoriju slova.</p></section>"
    )
}

/// Extra meanings and semantic links (related / synonyms / antonyms) drawn from
/// the native RU / PL / CS Wiktionary entries for the cognates, each chip linking
/// back to its source dictionary.
fn enrich_connections_section(
    members: &[(String, String)],
    enrich: &crate::enrich::EnrichIndex,
    xref: Option<&crate::enrich::Xref>,
    self_id: usize,
) -> String {
    let mut blocks = String::new();
    for &lang in crate::enrich::ENRICH_LANGS {
        // The richest enriched member for this edition.
        let mut best: Option<(&str, &crate::enrich::EnrichEntry)> = None;
        for (l, w) in members.iter().filter(|(l, _)| l == lang) {
            if let Some(e) = enrich.get(l, w) {
                let score = e.senses.len() + e.related.len() + e.synonyms.len();
                let better = best
                    .map(|(_, b)| score > b.senses.len() + b.related.len() + b.synonyms.len())
                    .unwrap_or(true);
                if better {
                    best = Some((w, e));
                }
            }
        }
        let Some((word, e)) = best else { continue };
        let mut inner = String::new();
        if e.senses.len() > 1 {
            let items: String = e
                .senses
                .iter()
                .map(|x| format!("<li>{}</li>", esc(x)))
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
                    // Link internally when the term is itself a dictionary headword
                    // (and not this very page); otherwise out to native Wiktionary.
                    match xref.and_then(|x| x.get(lang, w)).filter(|&t| t != self_id) {
                        Some(target) => format!(
                            "<a class='chip xref' title='v slovniku' href='{target}.html'>{}</a>",
                            esc(w)
                        ),
                        None => format!(
                            "<a class='chip' href='{}'>{}</a>",
                            esc(&crate::enrich::source_url(lang, w)),
                            esc(w)
                        ),
                    }
                })
                .collect();
            format!("<div class='conn'><h5>{title}</h5><div class='chips'>{cs}</div></div>")
        };
        inner.push_str(&chips("Sŕodne slova", &e.related));
        inner.push_str(&chips("Sinonimy", &e.synonyms));
        inner.push_str(&chips("Antonimy", &e.antonyms));
        if inner.is_empty() {
            continue;
        }
        let _ = write!(
            blocks,
            "<div class='src-block'><div class='src-head'><span class='lc'>{}</span> <a class='ext' href='{}'>{}↗</a></div>{}</div>",
            esc(&crate::lang::lang_name(lang)),
            esc(&crate::enrich::source_url(lang, word)),
            esc(word),
            inner
        );
    }
    if blocks.is_empty() {
        return String::new();
    }
    format!(
        "<section><h2 id='vezi'>Značenja i semantične vęzi (RU / PL / CS)</h2>{blocks}</section>"
    )
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
    let mut list = String::from("<table class='wikitable'><thead><tr><th>Kandidat</th><th>Čęst rěči</th><th>Smysl</th><th>Sila dogadki</th><th>Cognaty</th></tr></thead><tbody>");
    for r in rows.iter().take(400) {
        let langs = (r.freq as usize).max(1);
        let _ = write!(
            list,
            "<tr><td><a href='entry/{}.html'><b>{}</b></a></td><td>{}</td><td>{}</td><td>{}</td><td class='muted'>{}</td></tr>",
            r.id,
            esc(&r.form),
            esc(&r.pos),
            esc(&truncate(&r.gloss, 50)),
            strength_cell(r.conf, r.score),
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
             <p class='muted'>Prvyh 400 od <b>{total}</b> zapisov. „Sila dogadki“ = kalibrovana uvěrjenost + ocěna.</p>
             {list}
           </article>
           <aside class='home-aside'>
             <div class='side-box'><div class='side-h'>Slovnik</div>
               <table class='wikitable compact-table'>
                 <tr><th>Slov</th><td>{total}</td></tr>
                 <tr><th>Lemm</th><td>{lemmas}</td></tr>
                 <tr><th>= oficialnomu</th><td>{official}</td></tr>
                 <tr><th>Oficialne-only</th><td>{official_only}</td></tr>
                 <tr><th>Zaimky</th><td>{borrowed}</td></tr>
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
               <li>Praslovjansko pravilo daje flavornų formų.</li>
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
        "<article class='entry'>
           <h1 class='firstHeading'>O metodě</h1>
           <p class='muted'>Toj slovnik ne je izvedeny iz oficialnogo medžuslovjanskogo slovnika — on je generovany iz <b>vsěh {lemmas} naslědovanyh slovjanskyh lemm</b> v Wiktionary.</p>
           <h2>Kako</h2>
           <ol>
             <li>Iz Wiktionary sȯbiramy vsakų slovjanskų lemmų (imennik, infinitiv glagola, positiv prilagatelnogo) s jeje praslovjanskym korenem.</li>
             <li>Lemmy s tym že korenem tvorę <b>cognatnų grupų</b> — {sets} grup.</li>
             <li>Praslovjansko pravilo izvodi medžuslovjanskų formų; medžuvětvovy konsensus daje alternativų.</li>
             <li><b>Uvěrjenost raste s čislom językov i větvej</b> kotore potvŕđajų korenj: slovo v jednom języku = niska uvěrjenost, slovo v vsěh třěh větvah = visoka.</li>
           </ol>
           <h2>Validacija</h2>
           <p>{official} generovanyh slov takože postoji v oficialnom medžuslovjanskom slovniku — nězavisna kontrola točnosti.</p>
           <p class='muted'>Dokazy: Wiktionary (CC BY-SA). Praslovjanske rekonstrukcije i naslědniky iz Wiktionary etimologij. Kod: <a href='{repo}'>MIT</a>.</p>
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
}

/// Compact strength letter for the search index (V/S/N = high/medium/low).
fn conf_letter(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "V",
        Confidence::Medium => "S",
        Confidence::Low => "N",
    }
}

/// The "guess strength" cell: a calibrated-confidence label + the numeric score.
fn strength_cell(conf: Confidence, score: f32) -> String {
    format!(
        "<span class='reliability {}'>{}</span> <span class='score muted'>{:.2}</span>",
        conf_class(conf),
        conf.label(),
        score
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
    let mut list = String::from("<table class='wikitable'><thead><tr><th>Kandidat</th><th>Čęst rěči</th><th>Anglijski smysl</th><th>Sila dogadki</th><th>Status</th></tr></thead><tbody>");
    for r in top_rows.iter().take(300) {
        let _ = write!(
            list,
            "<tr><td><a href='entry/{}.html'><b>{}</b></a></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            r.id,
            esc(&r.form),
            esc(&r.pos),
            esc(&truncate(&r.gloss, 55)),
            strength_cell(r.conf, r.score),
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
             <p class='muted'>Najčęstějih 300 od <b>{total}</b> zapisov; iskaj gore za vse. „Sila dogadki“ = kalibrovana uvěrjenost + ocěna.</p>
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
                 <tr><th>Točno (exact)</th><td>{exact:.1}%</td></tr>
                 <tr><th>Bez oficialnoj</th><td>{n_none}</td></tr>
               </table>
             </div>
             <div class='portal-box'><h3>Kako radi</h3><ul class='compact-list'>
               <li>Medžuvětvovy konsensus (6 podgrup) izbira korenj.</li>
               <li>Praslovjansko pravilo daje flavornų formų.</li>
               <li>Sila dogadki = kalibrovana uvěrjenost.</li>
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
        js = SEARCH_JS,
    );
    page("Medžuslovjansky generator", &body, 0)
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

// Client-side search. Loaded on EVERY page (the search box lives in the header),
// so SITE_BASE ('' at root, '../' under /entry/) resolves the fetch and links.
// Typing shows a top-8 dropdown; Enter (or the full-results link) goes to
// search.html?q, which lists every match.
const SEARCH_JS: &str = r#"
let IDX=null;
async function ensure(){ if(IDX)return IDX; const r=await fetch(SITE_BASE+'search.json'); IDX=await r.json(); return IDX; }
var q=document.getElementById('q'), out=document.getElementById('results'), pageRes=document.getElementById('page-results');
var STR={V:['vysoka','conf-high'],S:['srědnja','conf-med'],N:['nizka','conf-low']};
function strBadge(e){ var s=STR[e[5]]||STR.N; return "<span class='reliability "+s[1]+"'>"+s[0]+"</span>"; }
function fold(x){ return x.toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g,'').replace(/đ/g,'d'); }
function scoreAll(raw){
  var s=raw.trim().toLowerCase(); if(!s) return [];
  var s2=s.replace(/^to\s+/,''), sf=fold(s2), hits=[];
  for(var i=0;i<IDX.length;i++){ var e=IDX[i], f=e[1].toLowerCase(), g=e[2].toLowerCase(), ks=e[7]||[];
    var gs=g.split(/[,;]\s*/), ff=fold(f), sc=0, anchor=0;
    if(f===s||f===s2)sc=100; else if(ff===sf)sc=90;
    else{ for(var k=0;k<ks.length;k++){ var kr=ks[k]; if(kr[0]===s2||kr[0]===sf){ sc=85-3*Math.min(kr[1],5); if(kr[1]>1)anchor=kr[1]; break; } } }
    if(!sc){ if(f.indexOf(s2)===0||ff.indexOf(sf)===0)sc=60;
      else if(gs.some(function(x){return x.trim()===s||x.trim()===s2;}))sc=55;
      else if(ks.some(function(kr){return kr[0].indexOf(sf)===0;}))sc=50;
      else if(f.indexOf(s2)>=0)sc=40; else if(g.indexOf(s2)>=0)sc=20; }
    if(sc>0)hits.push([sc,e,anchor]); if(hits.length>3000)break; }
  hits.sort(function(a,b){return b[0]-a[0];}); return hits;
}
function hitHTML(e,a){ return "<a class='hit' href='"+SITE_BASE+"entry/"+e[0]+".html"+(a?('#cand-'+a):'')+"'><b>"+e[1]+"</b> <span class='hp'>"+e[3]+"</span> <span class='hg'>"+e[2]+"</span> <span class='hs'>"+strBadge(e)+"</span></a>"; }
async function run(){
  await ensure(); var v=q?q.value:''; var hits=scoreAll(v);
  if(out){ if(v.trim()){ var h=hits.slice(0,8).map(function(x){return hitHTML(x[1],x[2]);}).join('');
      if(!h)h="<div class='muted nohit'>Ničto ne najdeno.</div>";
      else if(hits.length>8)h+="<a class='hit more' href='"+SITE_BASE+"search.html?q="+encodeURIComponent(v.trim())+"'>Vse "+hits.length+" rezultatov -></a>";
      out.innerHTML=h; out.style.display='block'; } else out.style.display='none'; }
  if(pageRes){ var c=document.getElementById('rescount'); if(c)c.textContent=hits.length;
    pageRes.innerHTML=hits.slice(0,400).map(function(x){return hitHTML(x[1],x[2]);}).join('')||"<div class='muted'>Ničto ne najdeno.</div>"; }
}
function goSearch(e){ e.preventDefault(); var v=q.value.trim(); if(v) location.href=SITE_BASE+'search.html?q='+encodeURIComponent(v); return false; }
if(q){ var t=null; q.addEventListener('input',function(){ clearTimeout(t); t=setTimeout(run,110); });
  q.addEventListener('focus',function(){ if(q.value.trim())run(); }); }
document.addEventListener('click',function(ev){ if(out&&!ev.target.closest('.hsearch'))out.style.display='none'; });
async function randomWord(){ await ensure(); if(!IDX.length)return; var e=IDX[Math.floor(Math.random()*IDX.length)];
  var el=document.getElementById('spotlight'); if(!el)return; var box=document.getElementById('spotbox'); if(box)box.style.display='';
  el.innerHTML="<a class='spotlight-word' href='"+SITE_BASE+"entry/"+e[0]+".html'>"+e[1]+"</a><div class='muted'>"+e[3]+" · "+e[2]+"</div>"; }
var rb=document.getElementById('randbtn'); if(rb) rb.addEventListener('click',randomWord);
if(document.getElementById('spotlight')) randomWord();
(function(){ var p=new URLSearchParams(location.search).get('q'); if(p&&q){ q.value=p; run(); } })();
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

fn entry_page(id: usize, entry: &OfficialEntry, g: &Generation, evidence: &[Evidence]) -> String {
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
    let inflection = inflection_table(&top.form, pos_code);
    let evidence_html = evidence_block(evidence);
    let alternatives = alternatives_block(&g.candidates);
    let trace = trace_block(top);
    let calib = calibration_note(top.confidence);
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

fn alternatives_block(candidates: &[Candidate]) -> String {
    if candidates.is_empty() {
        return "<p class='muted'>Bez kandidatov.</p>".to_string();
    }
    // Always show the ranked forms (the top one is the headword); this is now a
    // primary section, so even a single-candidate entry lists its form + score.
    let mut s = String::from("<table class='wikitable'><thead><tr><th>#</th><th>Forma</th><th>Izvor</th><th>Ocěna</th><th>Uvěrjenost</th><th>Větvi</th></tr></thead><tbody>");
    for (i, c) in candidates.iter().enumerate() {
        let _ = write!(
            s,
            "<tr id='cand-{}' class='{}'><td>{}</td><td><span class='mention'>{}</span></td><td><span class='pill {}'>{}</span></td><td class='score'>{:.3}</td><td>{}</td><td>{}</td></tr>",
            i + 1,
            if i == 0 { "top-candidate" } else { "" },
            i + 1,
            esc(&c.form),
            source_class(c.source),
            esc(c.source.label()),
            c.score,
            c.confidence.label(),
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
                esc(&ev.form),
                esc(&ev.normalized_form)
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
    // Decline the bare stem for reflexive verbs (the ` sę` particle is invariant).
    let word = word.strip_suffix(" sę").unwrap_or(word);
    match pos_code {
        "noun" | "proper_noun" => noun_table(word),
        "adj" => adj_table(word),
        "verb" => verb_table(word),
        _ => "<p class='muted'>Za tų čęst rěči nema tablicy prěgibanja.</p>".to_string(),
    }
}

fn noun_table(word: &str) -> String {
    let rows = [
        ("Nominativ", IsvCase::Nom),
        ("Akuzativ", IsvCase::Acc),
        ("Genitiv", IsvCase::Gen),
        ("Dativ", IsvCase::Dat),
        ("Lokativ", IsvCase::Loc),
        ("Instrumental", IsvCase::Ins),
    ];
    let mut s = String::from("<table class='wikitable inflection-table'><thead><tr><th>Padež</th><th>Jednina</th><th>Množina</th></tr></thead><tbody>");
    for (label, case) in rows {
        let _ = write!(
            s,
            "<tr><th>{}</th><td>{}</td><td>{}</td></tr>",
            label,
            esc(&catch(|| ISV::noun(word, case, IsvNumber::Singular))),
            esc(&catch(|| ISV::noun(word, case, IsvNumber::Plural))),
        );
    }
    s.push_str("</tbody></table>");
    s
}

fn adj_table(word: &str) -> String {
    let rows = [
        ("Nominativ", IsvCase::Nom),
        ("Genitiv", IsvCase::Gen),
        ("Dativ", IsvCase::Dat),
        ("Instrumental", IsvCase::Ins),
    ];
    let mut s = String::from("<table class='wikitable inflection-table'><thead><tr><th>Padež</th><th>M. živ.</th><th>M. neživ.</th><th>Ž.</th><th>Sr.</th></tr></thead><tbody>");
    for (label, case) in rows {
        let _ = write!(
            s,
            "<tr><th>{}</th><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            label,
            esc(&catch(|| ISV::adj(
                word,
                case,
                IsvNumber::Singular,
                IsvGender::Masculine,
                IsvAnimacy::Animate
            ))),
            esc(&catch(|| ISV::adj(
                word,
                case,
                IsvNumber::Singular,
                IsvGender::Masculine,
                IsvAnimacy::Inanimate
            ))),
            esc(&catch(|| ISV::adj(
                word,
                case,
                IsvNumber::Singular,
                IsvGender::Feminine,
                IsvAnimacy::Inanimate
            ))),
            esc(&catch(|| ISV::adj(
                word,
                case,
                IsvNumber::Singular,
                IsvGender::Neuter,
                IsvAnimacy::Inanimate
            ))),
        );
    }
    s.push_str("</tbody></table>");
    s
}

fn verb_table(word: &str) -> String {
    let rows = [
        ("1. jedn.", IsvPerson::First, IsvNumber::Singular),
        ("2. jedn.", IsvPerson::Second, IsvNumber::Singular),
        ("3. jedn.", IsvPerson::Third, IsvNumber::Singular),
        ("1. množ.", IsvPerson::First, IsvNumber::Plural),
        ("2. množ.", IsvPerson::Second, IsvNumber::Plural),
        ("3. množ.", IsvPerson::Third, IsvNumber::Plural),
    ];
    let mut s = String::from("<table class='wikitable inflection-table'><thead><tr><th>Osoba</th><th>Teperešnje vrěme</th></tr></thead><tbody>");
    for (label, person, number) in rows {
        let _ = write!(
            s,
            "<tr><th>{}</th><td>{}</td></tr>",
            label,
            esc(&catch(|| ISV::verb(
                word,
                person,
                number,
                IsvGender::Masculine,
                IsvTense::Present
            )))
        );
    }
    s.push_str("</tbody></table>");
    s
}

fn catch<F: FnOnce() -> String + std::panic::UnwindSafe>(f: F) -> String {
    std::panic::catch_unwind(f).unwrap_or_else(|_| "—".to_string())
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

fn pos_heading(raw: &str) -> String {
    let p = crate::model::Pos::parse(raw);
    format!("{} ({})", p.heading_isv(), raw.trim())
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

fn calibration_note(c: Confidence) -> String {
    let rate = match c {
        Confidence::High => "≈67% takyh kandidatov odgovara oficialnomu slovniku",
        Confidence::Medium => "≈35% takyh kandidatov odgovara oficialnomu slovniku",
        Confidence::Low => "≈10% takyh kandidatov odgovara oficialnomu slovniku",
    };
    format!("<p class='muted calib'>Kalibrovana pouzdanost: {rate} (izměrjeno na benchmarku).</p>")
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

/// `depth` 0 = site root (home), 1 = one subdirectory deep (entry/*.html).
const REPO_URL: &str = "https://github.com/gold-silver-copper/interslavic-wiktionary-lab";

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
           <nav class='nav'><a href='{up}index.html'>Slovnik</a><a href='{up}search.html'>Iskanje</a><a href='{up}about.html'>O metodě</a><a href='{REPO_URL}'>Kod</a></nav>\
         </header>\
         <div class='layout'>\
           <aside class='sidebar'>\
             <div class='side-box toc-box'><div class='side-h'>Na toj straně</div><nav id='toc-nav' class='toc'></nav></div>\
             <div class='side-box'><div class='side-h'>Nastroje</div>\
               <a class='side-link' href='{up}index.html'>📖 Vse slova</a>\
               <button id='randbtn' class='side-link' type='button'>🎲 Slučajno slovo</button>\
               <a class='side-link' href='{up}search.html'>🔎 Rozšireno iskanje</a>\
               <a class='side-link' href='{up}about.html'>ⓘ O metodě</a>\
             </div>\
             <div class='side-box' id='spotbox' style='display:none'><div class='side-h'>Slučajno slovo</div><div id='spotlight'></div></div>\
           </aside>\
           <main>{body}</main>\
         </div>\
         <footer class='site-footer'>Mašinno generovane rekonstrukcije — ne oficialny standard bez prověrky. Dokazy: interslavic-dictionary.com, Wiktionary (CC BY-SA). <a href='{REPO_URL}'>Izvorny kod</a>.</footer>\
         <script>{SEARCH_JS}</script>\
         <script>{TOC_JS}</script>\
         </body></html>",
        title = esc(title)
    )
}
fn about_page(n: usize, norm_rate: f32, exact_rate: f32, top3: f32) -> String {
    let body = format!(
        "<article class='entry'>
           <h1>O metodě</h1>
           <p class='lede'>Toj slovnik ne je rųčno napisany — vsaka forma je <b>generovana</b> iz slovjanskyh dokazov i měrjena protiv oficialnogo medžuslovjanskogo slovnika.</p>

           <h2>Dvostupnjovy model</h2>
           <p>Za vsaky smysl:</p>
           <ol>
             <li><b>Konsensus izbira korenj.</b> Iz cognatov v {langs} slovjanskyh językah glasujemo po <i>větvah</i> (izток / zapad / jug), da najveći język ne dominuje. Šest poddialektnyh grup s populacijnym vagom rěša, kotory korenj je najbolje medžuslovjansky.</li>
             <li><b>Praslovjansko pravilo daje formu.</b> Kǫda smysl je leakage-frějno povezany s praslovjanskoju rekonstrukcijeju (*word) črěz naslědnikov + glosų, determinističny stroj izvodi formų s pravilnymi flavornymi znakami (ě, ć/đ, å, ȯ, y), kotoryh moderne refleksy ne mogųt vȯzstanoviti.</li>
           </ol>

           <h2>Točnost (měrjeno)</h2>
           <div class='statgrid'>
             <div class='stat ok'><div class='statnum'>{exact:.1}%</div><div class='statlbl'>točno (exact)</div></div>
             <div class='stat'><div class='statnum'>{norm:.1}%</div><div class='statlbl'>normalizovano top-1</div></div>
             <div class='stat'><div class='statnum'>{top3:.1}%</div><div class='statlbl'>top-3</div></div>
           </div>
           <p class='muted'>Benchmark: {n} zapisov s ≥2 modernymi cognatami. Generator nikǫda ne vidi oficialnų formų — jedino cognate + čęsť rěči + glosų — tako da měrjenje je bez propuščanja (leakage-free). Vsako pravilo je zadŕžano jedino ako je izměrjeno pobolšanje (ablation ladder).</p>

           <h2>Poznaty prědel</h2>
           <p>Okolo 38% ostatnyh razlik sų <i>redakcijne</i> izbory (medžuslovjansky komitet izbral menšinny korenj) kotore se ne mogųt vȯzstanoviti iz modernyh cognatov. Čestny algoritmičny prědel je okolo 45–48% exact.</p>

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
.hit .hs{font-size:.85em;white-space:nowrap}
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
.chips{display:flex;flex-wrap:wrap;gap:.3rem}
a.chip{display:inline-block;background:var(--th);border:1px solid var(--line);border-radius:10px;padding:.05em .55em;font-size:.9em;color:var(--text)}
a.chip:hover{background:#eaf3ff;border-color:var(--link);text-decoration:none}
a.chip.xref{border-color:var(--link);color:var(--link);background:#eaf3ff}
a.chip.xref::before{content:'→\00a0';opacity:.65}
a.chip.xref:hover{background:var(--link);color:#fff}

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
}
