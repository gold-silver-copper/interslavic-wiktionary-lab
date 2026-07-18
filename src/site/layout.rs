//! Shared HTML shell plus small escaping and presentation helpers.
//!
//! Page-specific renderers pass their body and search payload into this layer;
//! the layout does not depend on any page renderer.

use super::assets::{site_base_js, TOC_JS};
use super::model::REPO_URL;

pub(super) fn page(title: &str, body: &str, depth: usize, search_js: &str) -> String {
    let up = if depth == 0 { "" } else { "../" };
    let site_base_js = site_base_js(up);
    format!(
        "<!doctype html><html lang='art'><head>\
         <meta charset='utf-8'><meta name='viewport' content='width=device-width, initial-scale=1'>\
         <title>{title}</title><link rel='stylesheet' href='{up}wiktionary.css'>\
         <script>{site_base_js}</script></head><body>\
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
    )
}

pub(super) fn esc(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub(super) fn json_str(v: &str) -> String {
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

pub(super) fn pos_heading(raw: &str) -> String {
    crate::model::Pos::parse(raw).heading_isv().to_string()
}

pub(super) fn pos_code_label(raw: &str) -> String {
    if raw.trim().is_empty() {
        "—".to_string()
    } else {
        pos_heading(raw)
    }
}

pub(super) fn status_pill(status: crate::model::MatchStatus) -> &'static str {
    match status {
        crate::model::MatchStatus::OfficialMatch => "<span class='pill ok'>oficialno</span>",
        crate::model::MatchStatus::DiffersFromOfficial => "<span class='pill warn'>razlika</span>",
        crate::model::MatchStatus::NoOfficialEntry => "<span class='pill info'>generovano</span>",
    }
}

pub(super) fn conf_class(confidence: crate::model::Confidence) -> &'static str {
    match confidence {
        crate::model::Confidence::High => "conf-high",
        crate::model::Confidence::Medium => "conf-med",
        crate::model::Confidence::Low => "conf-low",
    }
}

pub(super) fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        format!("{}…", value.chars().take(max_chars).collect::<String>())
    }
}

pub(super) fn compact(value: usize) -> String {
    let value = value.to_string();
    let mut out = String::new();
    for (index, ch) in value.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

/// Minimal query-string encoder for `forms.html?q=` links.
pub(super) fn urlencode_q(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace(' ', "%20")
        .replace('&', "%26")
        .replace('\'', "%27")
        .replace('"', "%22")
}
