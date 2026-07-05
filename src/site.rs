//! Local Wiktionary-style website driven by the candidate-generation engine.
//!
//! `build` runs the generator over the official dictionary's Slavic evidence and
//! writes a compact JSON artifact; `serve` loads it into memory and renders
//! entry pages that show, per meaning: the top candidate, the alternatives, the
//! rule trace, the evidence grouped by Slavic branch, the benchmark-calibrated
//! confidence, and the official-dictionary match status. It uses only local
//! Wiktionary-like CSS (no hotlinked Wikimedia assets) and no database.

use crate::consensus::{ConsensusConfig, MeaningInput};
use crate::generator;
use crate::lang::Branch;
use crate::model::{Candidate, Confidence, Evidence, MatchStatus};
use crate::official::{self, OfficialEntry};
use crate::overrides::Overrides;
use anyhow::{Context, Result};
use interslavic::{
    Animacy as IsvAnimacy, Case as IsvCase, Gender as IsvGender, Number as IsvNumber,
    Person as IsvPerson, Tense as IsvTense, ISV,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteDataset {
    pub meta: SiteMeta,
    pub entries: Vec<SiteEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteMeta {
    pub built_at_unix: u64,
    pub official_source: String,
    pub entry_count: usize,
    pub official_match: usize,
    pub differs: usize,
    pub no_official: usize,
    pub exact_rate: f32,
    pub normalized_rate: f32,
    pub overrides_applied: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteEntry {
    pub id: usize,
    pub gloss: String,
    pub pos: String,
    pub pos_code: String,
    pub official: Option<String>,
    pub match_status: MatchStatus,
    pub candidates: Vec<Candidate>,
    pub evidence: Vec<Evidence>,
    pub overridden: bool,
}

impl SiteEntry {
    fn top(&self) -> Option<&Candidate> {
        self.candidates.first()
    }
}

pub fn build(official_path: &Path, output: &Path) -> Result<()> {
    let entries = official::load(official_path)?;
    let overrides = Overrides::load(Path::new(crate::DEFAULT_OVERRIDES));
    let cfg = ConsensusConfig::production();

    let mut site_entries = Vec::new();
    let (mut n_match, mut n_diff, mut n_none, mut n_exact, mut n_over) = (0usize, 0, 0, 0, 0);

    for entry in &entries {
        // Only entries we can actually reconstruct from (some Slavic evidence)
        // and that carry a headword.
        let input = build_input(entry);
        if input.forms.iter().filter(|f| f.modern).count() < 2 {
            continue;
        }
        if entry.isv.trim().is_empty() {
            continue;
        }
        let official = if entry.isv.contains(' ') || entry.isv.contains('#') {
            None
        } else {
            Some(entry.isv.as_str())
        };
        let g = generator::generate(&input, official, None, &cfg, &overrides);
        let mut candidates = g.candidates;
        // Keep the evidence once at entry level; drop the per-candidate copies.
        let evidence = candidates
            .first()
            .map(|c| c.evidence.clone())
            .unwrap_or_default();
        for c in candidates.iter_mut() {
            c.evidence.clear();
        }
        candidates.truncate(5);

        match g.match_status {
            MatchStatus::OfficialMatch => n_match += 1,
            MatchStatus::DiffersFromOfficial => n_diff += 1,
            MatchStatus::NoOfficialEntry => n_none += 1,
        }
        if let (Some(off), Some(top)) = (official, candidates.first()) {
            if crate::orthography::exact_match(&top.form, off) {
                n_exact += 1;
            }
        }
        if g.overridden {
            n_over += 1;
        }

        site_entries.push(SiteEntry {
            id: site_entries.len() + 1,
            gloss: entry.english.clone(),
            pos: entry.pos_raw.clone(),
            pos_code: entry.pos.code().to_string(),
            official: g.official,
            match_status: g.match_status,
            candidates,
            evidence,
            overridden: g.overridden,
        });
    }

    let total = site_entries.len().max(1);
    let with_official = n_match + n_diff;
    let meta = SiteMeta {
        built_at_unix: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        official_source: official_path.display().to_string(),
        entry_count: site_entries.len(),
        official_match: n_match,
        differs: n_diff,
        no_official: n_none,
        exact_rate: n_exact as f32 / with_official.max(1) as f32,
        normalized_rate: n_match as f32 / with_official.max(1) as f32,
        overrides_applied: n_over,
    };

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let dataset = SiteDataset {
        meta,
        entries: site_entries,
    };
    let tmp = output.with_extension("json.tmp");
    serde_json::to_writer(&mut File::create(&tmp)?, &dataset)?;
    std::fs::rename(&tmp, output)?;
    println!(
        "wrote {} ({} entries: {} match official, {} differ, {} no official, {:.1}% normalized match)",
        output.display(),
        total,
        n_match,
        n_diff,
        n_none,
        100.0 * dataset.meta.normalized_rate
    );
    Ok(())
}

fn build_input(entry: &OfficialEntry) -> MeaningInput {
    let forms = crate::consensus::source_forms_from_cells(&entry.cells, |code, form| {
        format!(
            "https://en.wiktionary.org/wiki/{}#{}",
            form.replace(' ', "_"),
            code
        )
    });
    MeaningInput {
        pos: entry.pos,
        gender: entry.noun_traits.gender,
        gloss: entry.english.clone(),
        forms,
    }
}

// ---------------------------------------------------------------------------
// Serving
// ---------------------------------------------------------------------------

struct AppState {
    data: SiteDataset,
    by_id: HashMap<usize, usize>,
}

impl AppState {
    fn new(data: SiteDataset) -> Self {
        let by_id = data
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id, i))
            .collect();
        AppState { data, by_id }
    }
}

pub fn serve(data_path: &Path, host: &str, port: u16) -> Result<()> {
    let mut json = String::new();
    File::open(data_path)
        .with_context(|| format!("open {}", data_path.display()))?
        .read_to_string(&mut json)?;
    let data: SiteDataset = serde_json::from_str(&json).context("parse site dataset")?;
    let state = Arc::new(AppState::new(data));
    let addr = format!("{host}:{port}");
    let listener = TcpListener::bind(&addr).with_context(|| format!("bind {addr}"))?;
    println!(
        "Loaded {} generated entries into memory",
        state.data.entries.len()
    );
    println!("Serving http://{addr}");
    for stream in listener.incoming() {
        let stream = stream?;
        let state = Arc::clone(&state);
        thread::spawn(move || {
            if let Err(err) = handle(stream, state) {
                eprintln!("request error: {err:?}");
            }
        });
    }
    Ok(())
}

fn handle(mut stream: TcpStream, state: Arc<AppState>) -> Result<()> {
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf)?;
    if n == 0 {
        return Ok(());
    }
    let request = String::from_utf8_lossy(&buf[..n]);
    let path = request
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/");
    let (status, body, ctype) = route(path, &state);
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.as_bytes().len()
    );
    stream.write_all(response.as_bytes())?;
    Ok(())
}

fn route(raw: &str, state: &AppState) -> (&'static str, String, &'static str) {
    let (path, query) = raw.split_once('?').unwrap_or((raw, ""));
    match path {
        "/" => (
            "200 OK",
            page("Medžuslovjansky generator", &home(state, query)),
            "text/html",
        ),
        "/static/wiktionary.css" => ("200 OK", css(), "text/css"),
        _ if path.starts_with("/entry/") => {
            match path.trim_start_matches("/entry/").parse::<usize>() {
                Ok(id) => ("200 OK", page("Zapis", &entry_page(state, id)), "text/html"),
                Err(_) => (
                    "404 Not Found",
                    page("Ne najdeno", "<div class='card'>Nepravilny id</div>"),
                    "text/html",
                ),
            }
        }
        _ => (
            "404 Not Found",
            page("Ne najdeno", "<div class='card'>Ne najdeno</div>"),
            "text/html",
        ),
    }
}

fn home(state: &AppState, query: &str) -> String {
    let q = query_param(query, "q");
    let m = &state.data.meta;
    let rows = search(state, &q, 150);
    let mut list = String::from("<table class='wikitable'><tr><th>Kandidat</th><th>Čęst rěči</th><th>Anglijski smysl</th><th>Ocěna</th><th>Status</th></tr>");
    for &idx in &rows {
        let e = &state.data.entries[idx];
        let form = e.top().map(|c| c.form.as_str()).unwrap_or("—");
        let score = e.top().map(|c| c.score).unwrap_or(0.0);
        list.push_str(&format!(
            "<tr><td><a href='/entry/{}'><b>{}</b></a></td><td>{}</td><td>{}</td><td class='score'>{:.3}</td><td>{}</td></tr>",
            e.id, esc(form), esc(&e.pos_code), esc(&truncate(&e.gloss, 60)), score, status_badge(e.match_status)
        ));
    }
    list.push_str("</table>");
    let list_title = if q.is_empty() {
        "Generovane kandidaty"
    } else {
        "Rezultaty iskanja"
    };
    format!(
        "<section class='home-heading'>
           <h1 class='firstHeading'>Medžuslovjansky generator</h1>
           <p class='muted'>Naučno obosnovany generator medžuslovjanskyh kandidatov iz slovjanskyh dokazov, s ocěnkoju točnosti protiv oficialnogo slovnika.</p>
           <form class='hero-search'><input type='search' name='q' value='{}' placeholder='Iskaj po kandidatu ili anglijskom smyslu'><button>Iskati</button></form>
         </section>
         <section class='wiki-layout'>
           <article class='wiki-main-list'>
             <h2><span class='mw-headline'>{}</span></h2>
             {}
           </article>
           <aside class='wiki-sidebar'>
             <div class='portal-box stats-portal'><h3>Točnost protiv oficialnogo</h3>
               <table class='wikitable compact-table'>
                 <tr><th>Zapisy</th><td>{}</td></tr>
                 <tr><th>Odgovara oficialnomu</th><td>{} ({:.1}%)</td></tr>
                 <tr><th>Razlikuje se</th><td>{}</td></tr>
                 <tr><th>Točny (exact)</th><td>{:.1}%</td></tr>
                 <tr><th>Ručne korektury</th><td>{}</td></tr>
               </table>
             </div>
             <div class='portal-box'><h3>Kako radi</h3><ul class='compact-list'>
               <li>Medžuvětvovy konsensus (6 podgrup).</li>
               <li>Praslovjanska pravila i internacionalizmy.</li>
               <li>Sled pravil i dokazy po větvah.</li>
               <li>Kalibrovana uvěrjenost.</li>
             </ul></div>
             <div class='portal-box'><h3>Legenda</h3>
               <p>{} — generovana forma = oficialna.</p>
               <p>{} — razlikuje se od oficialnoj.</p>
               <p>{} — nema oficialnoj.</p>
             </div>
           </aside>
         </section>",
        esc(&q), list_title, list,
        compact(m.entry_count), compact(m.official_match), 100.0 * m.normalized_rate,
        compact(m.differs), 100.0 * m.exact_rate, compact(m.overrides_applied),
        status_badge(MatchStatus::OfficialMatch),
        status_badge(MatchStatus::DiffersFromOfficial),
        status_badge(MatchStatus::NoOfficialEntry),
    )
}

fn entry_page(state: &AppState, id: usize) -> String {
    let Some(&idx) = state.by_id.get(&id) else {
        return "<div class='notice'>Zapis ne najdeny</div>".to_string();
    };
    let e = &state.data.entries[idx];
    let Some(top) = e.top() else {
        return "<div class='notice'>Nema kandidatov</div>".to_string();
    };

    let banner = status_banner(e);
    let headword = format!(
        "<p class='inflection-head'><span class='Latn headword'>{}</span> <span class='badge'>{}</span> <span class='reliability {}'>uvěrjenost: {}</span></p>",
        esc(&top.form),
        esc(&e.pos_code),
        conf_class(top.confidence),
        top.confidence.label()
    );

    let alternatives = alternatives_block(e);
    let trace = trace_block(top);
    let evidence = evidence_block(e);
    let inflection = inflection_table(&top.form, e.pos_code.as_str());
    let calib = calibration_note(top.confidence);

    format!(
        "<h1 id='firstHeading' class='firstHeading'>{}</h1>
         {banner}
         <div class='toc' role='navigation'><div class='toc-title'>Sadržanje</div><ol>
           <li><a href='#kandidat'>Kandidat</a></li>
           <li><a href='#alternativy'>Alternativy</a></li>
           <li><a href='#sled'>Sled pravil</a></li>
           <li><a href='#dokazy'>Dokazy po větvah</a></li>
           <li><a href='#prěgibanje'>Prěgibanje</a></li>
         </ol></div>
         <h2><span id='kandidat' class='mw-headline'>Medžuslovjansky kandidat</span></h2>
         {headword}
         <p><b>Anglijski smysl:</b> {gloss}</p>
         {calib}
         <h3><span id='alternativy' class='mw-headline'>Alternativne kandidaty</span></h3>
         {alternatives}
         <h3><span id='sled' class='mw-headline'>Sled pravil (kako je forma izvedena)</span></h3>
         {trace}
         <h3><span id='dokazy' class='mw-headline'>Dokazy po slovjanskyh větvah</span></h3>
         {evidence}
         <h3><span id='prěgibanje' class='mw-headline'>Prěgibanje</span></h3>
         {inflection}
         <div class='footer-note'>Lokalno generovana stranica v stilu Wiktionary. Formy prěgibanja iz interslavic-rs. Bez Wikimedia CSS/JS.</div>",
        esc(&top.form),
        banner = banner,
        headword = headword,
        gloss = esc(&e.gloss),
        calib = calib,
        alternatives = alternatives,
        trace = trace,
        evidence = evidence,
        inflection = inflection,
    )
}

fn status_banner(e: &SiteEntry) -> String {
    match e.match_status {
        MatchStatus::OfficialMatch => format!(
            "<div class='banner ok'><b>Oficialno potvŕđeno.</b> Generovana forma odgovara oficialnomu slovniku: <span class='mention'>{}</span>.</div>",
            esc(e.official.as_deref().unwrap_or(""))
        ),
        MatchStatus::DiffersFromOfficial => format!(
            "<div class='banner warn'><b>Razlikuje se od oficialnogo.</b> Generovany kandidat: <span class='mention'>{}</span>; oficialna forma: <span class='mention'>{}</span>. Nižej sų oba i objasnjenje.</div>",
            esc(e.top().map(|c| c.form.as_str()).unwrap_or("")),
            esc(e.official.as_deref().unwrap_or(""))
        ),
        MatchStatus::NoOfficialEntry => "<div class='banner info'><b>Nema oficialnogo zapisa.</b> Forma je čisto generovana iz slovjanskyh dokazov.</div>".to_string(),
    }
}

fn alternatives_block(e: &SiteEntry) -> String {
    if e.candidates.len() <= 1 {
        return "<p class='muted'>Nema alternativnyh kandidatov.</p>".to_string();
    }
    let mut s = String::from("<table class='wikitable'><tr><th>#</th><th>Forma</th><th>Izvor</th><th>Ocěna</th><th>Uvěrjenost</th><th>Větvi</th></tr>");
    for (i, c) in e.candidates.iter().enumerate() {
        s.push_str(&format!(
            "<tr class='{}'><td>{}</td><td><span class='mention'>{}</span></td><td>{}</td><td class='score'>{:.3}</td><td>{}</td><td>{}</td></tr>",
            if i == 0 { "top-candidate" } else { "" },
            i + 1,
            esc(&c.form),
            esc(c.source.label()),
            c.score,
            c.confidence.label(),
            c.branch_coverage
        ));
    }
    s.push_str("</table>");
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
        s.push_str(&format!(
            "<li><code class='rule-id'>{}</code>: <span class='mention'>{}</span> → <span class='mention'>{}</span><br><span class='muted'>{}</span>{}</li>",
            esc(&step.id), esc(&step.before), esc(&step.after), esc(&step.explanation), reference
        ));
    }
    s.push_str("</ol>");
    if !c.warnings.is_empty() {
        s.push_str("<div class='notice'>");
        for w in &c.warnings {
            s.push_str(&format!("<p>⚠ {}</p>", esc(w)));
        }
        s.push_str("</div>");
    }
    s
}

fn evidence_block(e: &SiteEntry) -> String {
    let mut s = String::new();
    for branch in Branch::ALL {
        let items: Vec<&Evidence> = e
            .evidence
            .iter()
            .filter(|ev| ev.branch == Some(branch))
            .collect();
        if items.is_empty() {
            continue;
        }
        s.push_str(&format!(
            "<div class='branch-box'><h4>{} <span class='muted'>({})</span></h4><table class='wikitable compact-table'><tr><th>Język</th><th>Forma</th><th>Normalizovano</th></tr>",
            esc(branch.label()),
            branch.code()
        ));
        for ev in items {
            s.push_str(&format!(
                "<tr><td>{}</td><td><a href='{}'>{}</a></td><td>{}</td></tr>",
                esc(&ev.lang_name),
                esc(&ev.source_url),
                esc(&ev.form),
                esc(&ev.normalized_form)
            ));
        }
        s.push_str("</table></div>");
    }
    if s.is_empty() {
        "<p class='muted'>Bez dokazov.</p>".to_string()
    } else {
        format!("<div class='branch-grid'>{s}</div>")
    }
}

fn inflection_table(word: &str, pos_code: &str) -> String {
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
    let mut s = String::from("<table class='wikitable inflection-table'><tr><th>Padež</th><th>Jednina</th><th>Množina</th></tr>");
    for (label, case) in rows {
        s.push_str(&format!(
            "<tr><th>{}</th><td>{}</td><td>{}</td></tr>",
            label,
            esc(&catch(|| ISV::noun(word, case, IsvNumber::Singular))),
            esc(&catch(|| ISV::noun(word, case, IsvNumber::Plural))),
        ));
    }
    s.push_str("</table>");
    s
}

fn adj_table(word: &str) -> String {
    let rows = [
        ("Nominativ", IsvCase::Nom),
        ("Genitiv", IsvCase::Gen),
        ("Dativ", IsvCase::Dat),
        ("Instrumental", IsvCase::Ins),
    ];
    let mut s = String::from("<table class='wikitable inflection-table'><tr><th>Padež</th><th>M. živ.</th><th>M. neživ.</th><th>Ž.</th><th>Sr.</th></tr>");
    for (label, case) in rows {
        s.push_str(&format!(
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
        ));
    }
    s.push_str("</table>");
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
    let mut s = String::from("<table class='wikitable inflection-table'><tr><th>Osoba</th><th>Teperešnje vrěme</th></tr>");
    for (label, person, number) in rows {
        s.push_str(&format!(
            "<tr><th>{}</th><td>{}</td></tr>",
            label,
            esc(&catch(|| ISV::verb(
                word,
                person,
                number,
                IsvGender::Masculine,
                IsvTense::Present
            )))
        ));
    }
    s.push_str("</table>");
    s
}

fn catch<F: FnOnce() -> String + std::panic::UnwindSafe>(f: F) -> String {
    std::panic::catch_unwind(f).unwrap_or_else(|_| "—".to_string())
}

fn calibration_note(c: Confidence) -> String {
    let rate = match c {
        Confidence::High => "≈67% takyh kandidatov odgovara oficialnomu slovniku",
        Confidence::Medium => "≈35% takyh kandidatov odgovara oficialnomu slovniku",
        Confidence::Low => "≈10% takyh kandidatov odgovara oficialnomu slovniku",
    };
    format!("<p class='muted calib'>Kalibrovana pouzdanost: {rate} (izměrjeno na benchmarku).</p>")
}

fn search(state: &AppState, q: &str, limit: usize) -> Vec<usize> {
    if q.trim().is_empty() {
        let mut idxs: Vec<usize> = (0..state.data.entries.len()).collect();
        idxs.sort_by(|&a, &b| {
            let sa = state.data.entries[a].top().map(|c| c.score).unwrap_or(0.0);
            let sb = state.data.entries[b].top().map(|c| c.score).unwrap_or(0.0);
            sb.total_cmp(&sa)
        });
        idxs.truncate(limit);
        return idxs;
    }
    let ql = q.to_lowercase();
    let nq = crate::orthography::ascii_skeleton(q);
    let mut scored: Vec<(usize, i32)> = state
        .data
        .entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let mut sc = 0;
            if let Some(top) = e.top() {
                let sk = crate::orthography::ascii_skeleton(&top.form);
                if sk == nq {
                    sc += 30;
                } else if sk.contains(&nq) {
                    sc += 15;
                }
            }
            if e.gloss.to_lowercase().contains(&ql) {
                sc += 12;
            }
            if e.official
                .as_deref()
                .map(|o| o.to_lowercase().contains(&ql))
                .unwrap_or(false)
            {
                sc += 8;
            }
            (sc > 0).then_some((i, sc))
        })
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.into_iter().map(|(i, _)| i).take(limit).collect()
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

fn status_badge(s: MatchStatus) -> &'static str {
    match s {
        MatchStatus::OfficialMatch => "<span class='pill ok'>oficialno</span>",
        MatchStatus::DiffersFromOfficial => "<span class='pill warn'>razlika</span>",
        MatchStatus::NoOfficialEntry => "<span class='pill info'>generovano</span>",
    }
}

fn conf_class(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "conf-high",
        Confidence::Medium => "conf-med",
        Confidence::Low => "conf-low",
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
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

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset='utf-8'><meta name='viewport' content='width=device-width, initial-scale=1'><title>{}</title><link rel='stylesheet' href='/static/wiktionary.css'></head><body><div class='vector-page'><header class='vector-header'><h1><a href='/'>Medžuslovjansky generator</a></h1><span class='tagline'>naučno obosnovany generator kandidatov</span></header><main class='mw-body'><div class='mw-body-content'><div class='mw-parser-output'>{}</div></div></main></div></body></html>",
        esc(title), body
    )
}

fn css() -> String {
    format!("{}\n{}", BASE_CSS, EXTRA_CSS)
}

const BASE_CSS: &str = include_str!("../static/wiktionary.css");
const EXTRA_CSS: &str = r#"
.banner{padding:.7em 1em;border-radius:6px;margin:1em 0;border:1px solid #ccc}
.banner.ok{background:#e7f6e7;border-color:#8bcf8b}
.banner.warn{background:#fff5e0;border-color:#e6c15a}
.banner.info{background:#e7f0fb;border-color:#8bb2e6}
.pill{font-size:.8em;padding:.1em .5em;border-radius:10px;white-space:nowrap}
.pill.ok{background:#d6f2d6}.pill.warn{background:#fbeecb}.pill.info{background:#dbe8fb}
.reliability{font-size:.85em;padding:.1em .5em;border-radius:10px;margin-left:.5em}
.reliability.conf-high{background:#d6f2d6}.reliability.conf-med{background:#fbeecb}.reliability.conf-low{background:#f6dada}
.headword{font-size:1.5em;font-weight:bold}
.rule-trace li{margin:.4em 0}
.rule-id{background:#f0f0f0;padding:.05em .35em;border-radius:3px;font-size:.85em}
.branch-grid{display:flex;flex-wrap:wrap;gap:1em}
.branch-box{flex:1;min-width:240px}
.top-candidate{background:#f3faf3}
.calib{font-style:italic}
.doc-ref{font-size:.8em}
"#;

fn esc(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn query_param(query: &str, key: &str) -> String {
    for part in query.split('&') {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        if k == key {
            return percent_decode(v);
        }
    }
    String::new()
}

fn percent_decode(input: &str) -> String {
    let input = input.replace('+', " ");
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(hex);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
