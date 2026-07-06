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
        // row: [id, form, gloss, pos, statuschar, strengthLetter, score]
        let _ = write!(
            search,
            "[{},{},{},{},{},{},{:.2}]",
            id,
            json_str(&form),
            json_str(&truncate(&entry.english, 70)),
            json_str(&entry.pos.code()),
            json_str(statuschar),
            json_str(conf_letter(top.confidence)),
            top.score,
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
    let forms = crate::consensus::lemma_forms(forms, entry.pos);
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

const SEARCH_JS: &str = r#"
let IDX=null;
async function ensure(){ if(IDX)return IDX; const r=await fetch('search.json'); IDX=await r.json(); return IDX; }
const q=document.getElementById('q'), out=document.getElementById('results');
let t=null;
const STR={V:['vysoka','conf-high'],S:['srědnja','conf-med'],N:['nizka','conf-low']};
function strBadge(e){ const s=STR[e[5]]||STR.N; return `<span class='reliability ${s[1]}'>${s[0]}</span> <span class='score muted'>${(e[6]||0).toFixed?e[6].toFixed(2):e[6]}</span>`; }
q.addEventListener('input',()=>{ clearTimeout(t); t=setTimeout(()=>{ run(); sync(); },120); });
function sync(){ const v=q.value.trim(); history.replaceState(null,'', v?('?q='+encodeURIComponent(v)):location.pathname); }
async function run(){
  let s=q.value.trim().toLowerCase(); if(!s){out.innerHTML='';return;}
  // English verbs are cited without the infinitive marker ("eat", not "to eat").
  const s2=s.replace(/^to\s+/,'');
  const idx=await ensure();
  const hits=[];
  for(const e of idx){ const f=e[1].toLowerCase(), g=e[2].toLowerCase();
    const gs=g.split(/[,;]\s*/).map(x=>x.trim());
    let score=0;
    if(f===s||f===s2)score=100; else if(f.startsWith(s2))score=60; else if(f.includes(s2))score=40;
    else if(gs.some(x=>x===s||x===s2))score=55; else if(g.includes(s2))score=20;
    if(score>0)hits.push([score,e]); if(hits.length>600)break; }
  hits.sort((a,b)=>b[0]-a[0]);
  out.innerHTML=hits.slice(0,60).map(([_,e])=>`<a class='hit' href='entry/${e[0]}.html'><b>${e[1]}</b> <span class='hp'>${e[3]}</span> <span class='hg'>${e[2]}</span> <span class='hs'>${strBadge(e)}</span></a>`).join('')||"<div class='muted'>Ničto ne najdeno.</div>";
}
// Random "word of the moment" for the sidebar spotlight.
async function randomWord(){
  const idx=await ensure(); if(!idx.length)return;
  const e=idx[Math.floor(Math.random()*idx.length)];
  const el=document.getElementById('spotlight'); if(!el)return;
  el.innerHTML=`<a class='spotlight-word' href='entry/${e[0]}.html'>${e[1]}</a><div class='muted'>${e[3]} · ${e[2]}</div><div class='spot-strength'>Sila dogadki: ${strBadge(e)}</div>`;
}
const rb=document.getElementById('randbtn'); if(rb) rb.addEventListener('click',randomWord);
if(document.getElementById('spotlight')) randomWord();
// Pre-fill and run from a ?q= URL so shared links (…/?q=to+eat) work.
(function(){ const p=new URLSearchParams(location.search).get('q'); if(p){ q.value=p; run(); } })();
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
const REPO_URL: &str = "https://github.com/gold-silver-copper/interslavic-wiktionary-lab";

fn page(title: &str, body: &str, depth: usize) -> String {
    let up = if depth == 0 { "" } else { "../" };
    format!(
        "<!doctype html><html lang='art'><head><meta charset='utf-8'><meta name='viewport' content='width=device-width, initial-scale=1'><title>{title}</title><link rel='stylesheet' href='{up}wiktionary.css'></head><body>\
         <header class='site-header'>\
           <a class='brand' href='{up}index.html'>Medžuslovjansky generator</a>\
           <nav class='nav'><a href='{up}index.html'>Slovnik</a><a href='{up}about.html'>O metodě</a><a href='{REPO_URL}'>Kod</a></nav>\
         </header>\
         <main>{body}</main>\
         <footer class='site-footer'>Mašinno generovane rekonstrukcije — ne oficialny standard bez prověrky. Dokazy: interslavic-dictionary.com, Wiktionary (CC BY-SA). <a href='{REPO_URL}'>Izvorny kod</a>.</footer>\
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
