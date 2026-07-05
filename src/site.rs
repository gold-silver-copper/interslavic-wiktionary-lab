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

/// Generate the whole static site under `out_dir`.
pub fn export(official_path: &Path, out_dir: &Path) -> Result<()> {
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
    let mut top_rows: Vec<(f32, usize, String, String, String, MatchStatus)> = Vec::new(); // freq,id,form,gloss,pos,status
    let (mut n, mut n_match, mut n_diff, mut n_none, mut n_exact) = (0usize, 0, 0, 0, 0);

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
        let _ = write!(
            search,
            "[{},{},{},{},{}]",
            id,
            json_str(&form),
            json_str(&truncate(&entry.english, 70)),
            json_str(&entry.pos.code()),
            json_str(statuschar)
        );
        let freq = entry.frequency.unwrap_or(0.0);
        top_rows.push((
            freq,
            id,
            form,
            entry.english.clone(),
            entry.pos.code().to_string(),
            g.match_status,
        ));
    }
    search.push_str("\n]\n");

    std::fs::write(out_dir.join("search.json"), search)?;
    std::fs::write(out_dir.join("wiktionary.css"), css())?;
    std::fs::write(out_dir.join(".nojekyll"), "")?; // don't run Jekyll on GitHub Pages

    // Home page: stats + client-side search + the most frequent entries.
    top_rows.sort_by(|a, b| b.0.total_cmp(&a.0));
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

    println!(
        "wrote {} static pages to {} ({} match official, {} differ, {} no official, {:.1}% normalized match)",
        n,
        out_dir.display(),
        n_match,
        n_diff,
        n_none,
        rate(n_match, with_official)
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
        is_intl_meaning: entry.genesis.trim() == "I",
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

#[allow(clippy::too_many_arguments)]
fn home_page(
    n: usize,
    n_match: usize,
    n_diff: usize,
    n_none: usize,
    norm_rate: f32,
    exact_rate: f32,
    top_rows: &[(f32, usize, String, String, String, MatchStatus)],
) -> String {
    let mut list = String::from("<table class='wikitable'><thead><tr><th>Kandidat</th><th>Čęst rěči</th><th>Anglijski smysl</th><th>Status</th></tr></thead><tbody>");
    for (_freq, id, form, gloss, pos, status) in top_rows.iter().take(200) {
        let _ = write!(
            list,
            "<tr><td><a href='entry/{id}.html'><b>{}</b></a></td><td>{}</td><td>{}</td><td>{}</td></tr>",
            esc(form),
            esc(pos),
            esc(&truncate(gloss, 60)),
            status_pill(*status)
        );
    }
    list.push_str("</tbody></table>");

    let body = format!(
        "<section class='hero'>
           <h1>Medžuslovjansky generator</h1>
           <p class='lede'>Naučno obosnovany generator medžuslovjanskyh slov iz slovjanskyh dokazov, s ocěnkoju točnosti protiv oficialnogo slovnika.</p>
           <div class='searchbox'><input id='q' type='search' placeholder='Iskaj po kandidatu ili anglijskom smyslu…' autocomplete='off'><div id='results' class='results'></div></div>
         </section>
         <section class='statgrid'>
           <div class='stat'><div class='statnum'>{}</div><div class='statlbl'>zapisov</div></div>
           <div class='stat ok'><div class='statnum'>{:.1}%</div><div class='statlbl'>odgovara oficialnomu</div></div>
           <div class='stat'><div class='statnum'>{:.1}%</div><div class='statlbl'>točny (exact)</div></div>
           <div class='stat'><div class='statnum'>{}</div><div class='statlbl'>razlikuje se</div></div>
           <div class='stat'><div class='statnum'>{}</div><div class='statlbl'>nema oficialnoj</div></div>
         </section>
         <section>
           <h2>Najčęstěje slova</h2>
           <p class='muted'>{} match oficialnomu · {} razlika · {} bez oficialnoj. {} — oficialno, {} — razlika, {} — generovano.</p>
           {}
         </section>
         <script>{}</script>",
        compact(n),
        norm_rate,
        exact_rate,
        compact(n_diff),
        compact(n_none),
        compact(n_match),
        compact(n_diff),
        compact(n_none),
        status_pill(MatchStatus::OfficialMatch),
        status_pill(MatchStatus::DiffersFromOfficial),
        status_pill(MatchStatus::NoOfficialEntry),
        list,
        SEARCH_JS,
    );
    page("Medžuslovjansky generator", &body, 0)
}

const SEARCH_JS: &str = r#"
let IDX=null;
async function ensure(){ if(IDX)return IDX; const r=await fetch('search.json'); IDX=await r.json(); return IDX; }
const q=document.getElementById('q'), out=document.getElementById('results');
let t=null;
q.addEventListener('input',()=>{ clearTimeout(t); t=setTimeout(run,120); });
async function run(){
  const s=q.value.trim().toLowerCase(); if(!s){out.innerHTML='';return;}
  const idx=await ensure();
  const hits=[];
  for(const e of idx){ const f=e[1].toLowerCase(), g=e[2].toLowerCase();
    let score=0; if(f===s)score=100; else if(f.startsWith(s))score=60; else if(f.includes(s))score=40;
    else if(g.split(/[,;] /).some(x=>x.trim()===s))score=50; else if(g.includes(s))score=20;
    if(score>0)hits.push([score,e]); if(hits.length>400)break; }
  hits.sort((a,b)=>b[0]-a[0]);
  out.innerHTML=hits.slice(0,60).map(([_,e])=>`<a class='hit' href='entry/${e[0]}.html'><b>${e[1]}</b> <span class='hp'>${e[3]}</span> <span class='hg'>${e[2]}</span></a>`).join('')||"<div class='muted'>Ničto ne najdeno.</div>";
}
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
           <div class='headword'>{}</div>
           <div class='headmeta'>
             <span class='badge pos'>{}</span>
             <span class='pill {}'>{}</span>
             <span class='reliability {}'>uvěrjenost: {}</span>
             {}
           </div>
           <p class='def'><b>Anglijski smysl:</b> {}</p>
         </div>",
        esc(&top.form),
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
           <h1 class='page-title'>{}</h1>
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
    if candidates.len() <= 1 {
        return "<p class='muted'>Nema alternativnyh kandidatov.</p>".to_string();
    }
    let mut s = String::from("<table class='wikitable'><thead><tr><th>#</th><th>Forma</th><th>Izvor</th><th>Ocěna</th><th>Uvěrjenost</th><th>Větvi</th></tr></thead><tbody>");
    for (i, c) in candidates.iter().enumerate() {
        let _ = write!(
            s,
            "<tr class='{}'><td>{}</td><td><span class='mention'>{}</span></td><td><span class='pill {}'>{}</span></td><td class='score'>{:.3}</td><td>{}</td><td>{}</td></tr>",
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
fn page(title: &str, body: &str, depth: usize) -> String {
    let up = if depth == 0 { "" } else { "../" };
    format!(
        "<!doctype html><html lang='art'><head><meta charset='utf-8'><meta name='viewport' content='width=device-width, initial-scale=1'><title>{}</title><link rel='stylesheet' href='{up}wiktionary.css'></head><body><header class='site-header'><a class='brand' href='{up}index.html'>Medžuslovjansky generator</a><span class='tagline'>naučno obosnovany generator kandidatov</span></header><main>{}</main><footer class='site-footer'>Mašinno generovane rekonstrukcije. Formy nisų oficialny standard bez prověrky. Dokazy: interslavic-dictionary.com, Wiktionary (CC BY-SA).</footer></body></html>",
        esc(title), body
    )
}

fn css() -> String {
    format!("{}\n{}", BASE_CSS, EXTRA_CSS)
}

const BASE_CSS: &str = include_str!("../static/wiktionary.css");
const EXTRA_CSS: &str = r#"
:root{--bg:#fff;--fg:#202122;--muted:#72777d;--line:#c8ccd1;--accent:#36c;--ok:#d5f4d5;--warn:#fbeecb;--info:#dbe8fb}
@media (prefers-color-scheme:dark){:root{--bg:#0f1113;--fg:#e6e6e6;--muted:#9aa0a6;--line:#2a2f34;--accent:#7aa7ff;--ok:#1e3a1e;--warn:#3a331a;--info:#1a2a3a}}
body{background:var(--bg);color:var(--fg);margin:0;font:16px/1.55 -apple-system,Segoe UI,Roboto,Helvetica,Arial,sans-serif}
main{max-width:900px;margin:0 auto;padding:1.2em}
.site-header{display:flex;align-items:baseline;gap:.8em;padding:.7em 1.2em;border-bottom:1px solid var(--line);flex-wrap:wrap}
.brand{font-weight:700;font-size:1.15em;color:var(--fg);text-decoration:none}
.tagline{color:var(--muted);font-size:.85em}
.site-footer{max-width:900px;margin:2em auto;padding:1em 1.2em;color:var(--muted);font-size:.85em;border-top:1px solid var(--line)}
a{color:var(--accent)}
.hero h1{margin:.2em 0}
.lede{color:var(--muted);max-width:60ch}
.searchbox{position:relative;margin:1em 0}
#q{width:100%;box-sizing:border-box;padding:.7em .9em;font-size:1.05em;border:1px solid var(--line);border-radius:8px;background:var(--bg);color:var(--fg)}
.results{margin-top:.4em}
.hit{display:block;padding:.45em .6em;border:1px solid var(--line);border-top:none;text-decoration:none;color:var(--fg)}
.hit:first-child{border-top:1px solid var(--line);border-radius:6px 6px 0 0}
.hit:hover{background:var(--info)}
.hit .hp{color:var(--muted);font-size:.8em;margin:0 .4em}
.hit .hg{color:var(--muted)}
.statgrid{display:flex;gap:.7em;flex-wrap:wrap;margin:1.4em 0}
.stat{flex:1;min-width:120px;border:1px solid var(--line);border-radius:8px;padding:.7em .9em;text-align:center}
.stat.ok{background:var(--ok)}
.statnum{font-size:1.5em;font-weight:700}
.statlbl{color:var(--muted);font-size:.82em}
.page-title{font-size:1.1em;color:var(--muted);font-weight:400;margin:.2em 0}
.headword-block{border:1px solid var(--line);border-radius:10px;padding:1em 1.1em;margin:.6em 0}
.headword{font-size:2.2em;font-weight:700;line-height:1.1}
.headmeta{display:flex;gap:.5em;flex-wrap:wrap;align-items:center;margin:.5em 0}
.def{margin:.5em 0 0}
.badge.pos{background:#eef1f4;color:#333;padding:.15em .6em;border-radius:6px;font-size:.85em}
@media (prefers-color-scheme:dark){.badge.pos{background:#23272b;color:#ccc}}
.pill{font-size:.78em;padding:.12em .55em;border-radius:11px;white-space:nowrap}
.pill.ok{background:var(--ok)}.pill.warn{background:var(--warn)}.pill.info{background:var(--info)}
.pill.src-proto{background:#e5dcf7}.pill.src-consensus{background:var(--info)}.pill.src-official{background:var(--ok)}
@media (prefers-color-scheme:dark){.pill.src-proto{background:#33285a}}
.reliability{font-size:.82em;padding:.12em .55em;border-radius:11px}
.reliability.conf-high{background:var(--ok)}.reliability.conf-med{background:var(--warn)}.reliability.conf-low{background:#f6dada}
@media (prefers-color-scheme:dark){.reliability.conf-low{background:#4a2020}}
.banner{padding:.7em 1em;border-radius:8px;margin:1em 0;border:1px solid var(--line)}
.banner.ok{background:var(--ok)}.banner.warn{background:var(--warn)}.banner.info{background:var(--info)}
.mention{font-weight:600}
.muted{color:var(--muted)}
.sec{border:1px solid var(--line);border-radius:8px;margin:.7em 0;padding:.2em .9em}
.sec>summary{cursor:pointer;font-weight:600;padding:.5em 0}
.wikitable{border-collapse:collapse;width:100%;margin:.5em 0;font-size:.95em}
.wikitable th,.wikitable td{border:1px solid var(--line);padding:.35em .55em;text-align:left}
.wikitable thead th{background:#f2f4f7}
@media (prefers-color-scheme:dark){.wikitable thead th{background:#1a1e22}}
.inflection-table th{white-space:nowrap}
.compact-table td.lc{color:var(--muted);white-space:nowrap}
.branch-grid{display:flex;flex-wrap:wrap;gap:1em}
.branch-box{flex:1;min-width:240px}
.branch-box h4{margin:.4em 0}
.rule-trace li{margin:.4em 0}
.rule-id{background:#eef1f4;padding:.05em .35em;border-radius:3px;font-size:.85em}
@media (prefers-color-scheme:dark){.rule-id{background:#23272b}}
.top-candidate{background:rgba(120,200,120,.12)}
.score{font-variant-numeric:tabular-nums}
.foot{color:var(--muted);font-size:.85em;margin-top:1.5em}
.calib{font-style:italic}
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
