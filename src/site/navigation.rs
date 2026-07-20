//! Wiki-style navigation, categories, backlinks, graph data, and index pages.
//!
//! Individual renderers are pure. The high-level index writers group the
//! related filesystem operations that `site::export_corpus` orchestrates.

use super::assets::{GRAPH_FILTER_JS, RANDOM_PAGE_JS};
use super::layout::{compact, conf_class, esc, json_str, pos_code_label, pos_heading, truncate};
use super::model::{
    ancestor_slug, quality_label, slug, BuildMeta, FamilyEntry, HeadwordIndex, LinkEdge,
    SiteEntryInput, SiteEntryMeta, REPO_URL,
};
use super::search::search_js;
use super::special::{
    build_json, proto_index_page, proto_page, rule_file_stem, rule_page, rules_index_page,
    rules_json, sitemap_xml, ProtoReflexIndex, RuleIndex,
};
use crate::consensus::MeaningInput;
use crate::lang::Branch;
use crate::model::Confidence;
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

fn page(title: &str, body: &str, depth: usize) -> String {
    super::layout::page(title, body, depth, search_js())
}

pub(super) fn entry_meta(input: SiteEntryInput<'_>) -> SiteEntryMeta {
    let SiteEntryInput {
        id,
        title,
        gloss,
        pos,
        confidence: conf,
        score,
        probability: prob,
        n_languages: n_langs,
        n_branches,
        borrowed,
        official_only,
        official_lemma,
        ancestor,
        languages,
        wiki_categories,
    } = input;
    let first = first_bucket(title);
    let mut meta = SiteEntryMeta {
        id,
        title: title.to_string(),
        gloss: gloss.to_string(),
        pos: pos.to_string(),
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

pub(super) fn first_bucket(title: &str) -> String {
    let folded = crate::orthography::ascii_skeleton(title);
    folded
        .chars()
        .find(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase().to_string())
        .unwrap_or_else(|| "#".to_string())
}

pub(super) fn category_key(path: &[String]) -> String {
    path.iter().map(|s| slug(s)).collect::<Vec<_>>().join("__")
}

pub(super) fn category_title(path: &[String]) -> String {
    path.join(" » ")
}

/// The branch attestation PATTERN of a language set (issue #73c): WHICH of
/// the three branches attest the entry, rendered "V" / "Z" / "J" / "V+Z" /
/// "V+J" / "Z+J" / "V+Z+J" (V = vȯzhod/East, Z = zapad/West, J = jug/South,
/// always in that canonical order). Computed from the actual language SET via
/// `branch_of` — `n_branches` only counts and cannot distinguish V+Z from
/// Z+J. `None` when no code resolves to a branch.
pub(super) fn branch_pattern(langs: &[String]) -> Option<String> {
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

pub(super) fn entry_categories(
    m: &SiteEntryMeta,
    wiki_categories: Vec<Vec<String>>,
) -> Vec<Vec<String>> {
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

pub(super) fn add_category_path(cats: &mut Vec<Vec<String>>, path: Vec<String>) {
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

pub(super) fn wiktionary_category_paths_for_members(
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

pub(super) fn wiktionary_category_paths_for_input(
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

pub(super) fn push_wiki_paths(
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

pub(super) fn topic_category_path(lang: &str, label: &str) -> Option<Vec<String>> {
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

pub(super) fn wiki_topic_root(lang: &str, topic: Vec<&str>) -> Vec<String> {
    let mut path = vec![
        "Fundamentalne".to_string(),
        "Vsi języky".to_string(),
        crate::lang::lang_name(lang).to_string(),
        "Vse temy".to_string(),
    ];
    path.extend(topic.into_iter().map(|s| s.to_string()));
    path
}

pub(super) fn is_maintenance_wiki_category(label: &str) -> bool {
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

pub(super) fn compact_entry_categories(metas: &mut [SiteEntryMeta]) {
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

pub(super) fn issue_url(m: &SiteEntryMeta) -> String {
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

pub(super) fn query_escape(s: &str) -> String {
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

pub(super) fn homograph_groups(
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

pub(super) fn load_curation_notes() -> std::collections::HashMap<String, String> {
    let path = Path::new("data/curation-notes.json");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return std::collections::HashMap::new();
    };
    serde_json::from_str::<std::collections::HashMap<String, String>>(&raw).unwrap_or_default()
}

pub(super) fn add_edge(
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

pub(super) fn build_edges<T: FamilyEntry>(
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

pub(super) fn backlinks_by_target(
    edges: &[LinkEdge],
) -> std::collections::BTreeMap<usize, Vec<LinkEdge>> {
    let mut map: std::collections::BTreeMap<usize, Vec<LinkEdge>> =
        std::collections::BTreeMap::new();
    for e in edges {
        map.entry(e.target_id).or_default().push(e.clone());
    }
    map
}

pub(super) fn render_word_table(rows: &[SiteEntryMeta], up: &str) -> String {
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

pub(super) fn count_by<F>(
    rows: &[SiteEntryMeta],
    mut f: F,
) -> std::collections::BTreeMap<String, usize>
where
    F: FnMut(&SiteEntryMeta) -> String,
{
    let mut map = std::collections::BTreeMap::new();
    for m in rows {
        *map.entry(f(m)).or_insert(0) += 1;
    }
    map
}

pub(super) fn counts_table(
    title: &str,
    counts: &std::collections::BTreeMap<String, usize>,
) -> String {
    let mut pairs: Vec<(&String, &usize)> = counts.iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let mut body = String::new();
    for (k, v) in pairs.into_iter().take(24) {
        let _ = write!(body, "<tr><th>{}</th><td>{}</td></tr>", esc(k), compact(*v));
    }
    format!("<div class='stat-box'><h3>{}</h3><table class='wikitable compact-table'><tbody>{body}</tbody></table></div>", esc(title))
}

pub(super) fn index_summary(rows: &[SiteEntryMeta]) -> String {
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

pub(super) fn simple_index_page(
    title: &str,
    intro: &str,
    rows: &[SiteEntryMeta],
    depth: usize,
) -> String {
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

pub(super) fn site_stats_page(
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

pub(super) fn borrowing_source(m: &SiteEntryMeta) -> String {
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
pub(super) fn needs_review(m: &SiteEntryMeta) -> bool {
    m.official_lemma.is_none() && !m.official_only
}

pub(super) fn language_portal_page(
    lang: &str,
    rows: &[SiteEntryMeta],
    all: &[SiteEntryMeta],
) -> String {
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
        esc(name),
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
pub(super) fn root_page(root: &str, rows: &[SiteEntryMeta], proto_link: &str) -> String {
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

pub(super) fn borrowing_portal_page(rows: &[SiteEntryMeta]) -> String {
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

pub(super) fn needs_review_page(rows: &[SiteEntryMeta]) -> String {
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

pub(super) fn write_borrowing_subpages(out_dir: &Path, rows: &[SiteEntryMeta]) -> Result<()> {
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

pub(super) fn write_needs_review_subpages(out_dir: &Path, rows: &[SiteEntryMeta]) -> Result<()> {
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

pub(super) fn suffix_bucket(title: &str, pos: &str) -> String {
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

pub(super) fn suffix_index_page(rows: &[SiteEntryMeta]) -> String {
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

/// True when the entry renderer would emit at least one em-dash placeholder in
/// its inflection table.
fn has_inflection_issue(word: &str, pos: &str) -> bool {
    use interslavic::Number;

    let reflexive = word.ends_with(" sę");
    let bare = word.strip_suffix(" sę").unwrap_or(word);
    let cases = crate::forms::CASES.map(|(_, case)| case);
    match pos {
        "noun" | "proper_noun" => {
            let forms =
                std::panic::catch_unwind(|| crate::forms::noun_paradigm_forms(bare, None)).ok();
            cases.into_iter().any(|case| {
                [Number::Singular, Number::Plural]
                    .into_iter()
                    .any(|number| match &forms {
                        Some(forms) => crate::forms::clean_cell(forms.get(case, number)) == "—",
                        None => crate::forms::noun_cell_g(bare, case, number, None) == "—",
                    })
            })
        }
        "adj" => {
            let forms = std::panic::catch_unwind(|| interslavic::adj_forms(bare)).ok();
            cases.into_iter().any(|case| {
                [Number::Singular, Number::Plural]
                    .into_iter()
                    .any(|number| {
                        crate::forms::ADJ_COLS.into_iter().any(
                            |(_, gender, animacy)| match &forms {
                                Some(forms) => {
                                    crate::forms::clean_cell(
                                        forms.get(case, number, gender, animacy),
                                    ) == "—"
                                }
                                None => {
                                    crate::forms::adj_cell(bare, case, number, gender, animacy)
                                        == "—"
                                }
                            },
                        )
                    })
            })
        }
        "verb" => crate::forms::verb_cells(bare, reflexive).is_some_and(|cells| {
            [
                (&cells.present, 6),
                (&cells.imperfect, 6),
                (&cells.future, 6),
            ]
            .into_iter()
            .chain([
                (&cells.perfect, 8),
                (&cells.pluperfect, 8),
                (&cells.conditional, 8),
                (&cells.imperative, 3),
            ])
            .any(|(values, expected)| {
                values.len() < expected || values.iter().any(|value| value == "—")
            }) || cells.nonfinite.iter().any(|(_, value)| value == "—")
        }),
        _ => false,
    }
}

pub(super) fn inflection_issue(m: &SiteEntryMeta) -> bool {
    matches!(m.pos.as_str(), "noun" | "proper_noun" | "adj" | "verb")
        && has_inflection_issue(&m.title, &m.pos)
}

pub(super) fn inflection_issues_page(rows: &[SiteEntryMeta]) -> String {
    let mut issues: Vec<SiteEntryMeta> = rows
        .iter()
        .filter(|m| inflection_issue(m))
        .cloned()
        .collect();
    issues.sort_by_key(|m| crate::orthography::ascii_skeleton(&m.title));
    page("Speciaľno:ProblemyPrěgibanja", &format!("<article class='entry'><h1 class='firstHeading'>Speciaľno:ProblemyPrěgibanja</h1><p class='lede'>Stran zapisovy, gdě prěgibanje je nepolno ili vrnulo —. To je praktičny spis za popravki v interslavic-rs.</p>{}</article>", render_word_table(&issues, "")), 0)
}

pub(super) fn featured_page(rows: &[SiteEntryMeta], build: &BuildMeta) -> String {
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

pub(super) fn random_page() -> String {
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>Speciaľno:Slučajno</h1><p>Ta statična strana koristi lokalny <code>search/spotlight.json</code> i izbere slučajnu stranu zapisa bez servera.</p><p id='random-target' class='notice'>Nakladajě sę…</p><script>{RANDOM_PAGE_JS}</script></article>"
    );
    page("Speciaľno:Slučajno", &body, 0)
}

pub(super) fn special_pages_hub() -> String {
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

pub(super) fn talk_page(m: &SiteEntryMeta, note: Option<&String>, incoming: &[LinkEdge]) -> String {
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
pub(super) struct CategoryNode {
    pub(super) path: Vec<String>,
    pub(super) pages: Vec<SiteEntryMeta>,
    pub(super) children: BTreeSet<String>,
}

pub(super) fn build_category_tree(metas: &[SiteEntryMeta]) -> BTreeMap<String, CategoryNode> {
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

pub(super) fn write_category_pages(out_dir: &Path, metas: &[SiteEntryMeta]) -> Result<()> {
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

pub(super) fn category_descendant_page_count(
    tree: &BTreeMap<String, CategoryNode>,
    key: &str,
) -> usize {
    let mut ids = BTreeSet::new();
    collect_category_page_ids(tree, key, &mut ids);
    ids.len()
}

pub(super) fn collect_category_page_ids(
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

pub(super) fn category_page(
    tree: &BTreeMap<String, CategoryNode>,
    _key: &str,
    node: &CategoryNode,
) -> String {
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

pub(super) struct WikiIndexInput<'a> {
    pub(super) out_dir: &'a Path,
    pub(super) entries: &'a [SiteEntryMeta],
    pub(super) edges: &'a [LinkEdge],
    pub(super) backlinks: &'a std::collections::BTreeMap<usize, Vec<LinkEdge>>,
    pub(super) homographs: &'a std::collections::BTreeMap<String, Vec<SiteEntryMeta>>,
    pub(super) build: &'a BuildMeta,
    pub(super) curation: &'a std::collections::HashMap<String, String>,
    pub(super) rule_index: &'a RuleIndex,
    pub(super) proto: Option<&'a crate::dump::ProtoIndex>,
    pub(super) proto_reflex: &'a ProtoReflexIndex,
}

pub(super) fn write_wiki_indexes(input: WikiIndexInput<'_>) -> Result<()> {
    let WikiIndexInput {
        out_dir,
        entries: metas,
        edges,
        backlinks,
        homographs,
        build,
        curation,
        rule_index,
        proto,
        proto_reflex,
    } = input;
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
            esc(crate::lang::lang_name(lang)),
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

pub(super) fn backlink_page_body(m: &SiteEntryMeta, incoming: &[LinkEdge]) -> String {
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

pub(super) fn entry_tabs(m: &SiteEntryMeta) -> String {
    format!(
        "<nav class='entry-tabs'><a class='active' href='{}.html'>Strana</a><a href='../talk/{}.html'>Diskusija</a><a href='../what-links-here/{}.html'>Čto kaže sem</a><a href='../graph.html#n{}'>Graf</a><a href='{}'>Popraviti / problem</a></nav>",
        m.id,
        m.id,
        m.id,
        m.id,
        esc(&issue_url(m))
    )
}

/// The raw-collision display credit line (issue #86 item 6): raw Wiktionary
/// attestations whose display fold deduped onto this page
/// ([`RawFate::DedupedFold`]) — the site already knew these words but showed
/// them nowhere ("uk алое carries NO raw row anywhere and credits no
/// evidence"). Rendered as a compact muted line in the cognate section, each
/// item linking to the source-language Wiktionary (same
/// [`crate::enrich::source_url`] the raw pages use), capped at 12 with a
/// "+N dalje" tail. DISPLAY ONLY — never counted in n_langs / Dokaz /
/// razumlivost / the vote: raw evidence stays benchmark-forbidden by type.
pub(super) fn raw_credit_line(credits: Option<&Vec<(String, String)>>) -> String {
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
pub(super) fn union_razum_codes(corpus_langs: &[String], same_in: &[&'static str]) -> Vec<String> {
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

/// The infobox "Razumlivosť" row for a set of attesting language codes, with
/// the basis-appropriate tooltip (issue #79).
pub(super) fn razum_row(codes: &[&str], title: &str) -> String {
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
pub(super) fn razum_bars(r: &crate::lang::Razumlivost) -> String {
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
pub(super) fn entry_infobox(
    m: &SiteEntryMeta,
    razum: &str,
    extra_rows: &str,
    proto_link: &str,
) -> String {
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
    let government_rows = if m.pos == "prep" && (m.official_only || m.official_lemma.is_some()) {
        let lemma = m.official_lemma.as_deref().unwrap_or(&m.title);
        crate::check::preposition_government()
            .get(&crate::forms::form_key(lemma))
            .filter(|cases| !cases.is_empty())
            .map(|cases| {
                let labels = cases
                    .iter()
                    .map(|case| format!("{case}."))
                    .collect::<Vec<_>>()
                    .join(" / ");
                format!(
                    "<tr><th>Upravljanje</th><td>{} + {}</td></tr>",
                    esc(lemma),
                    esc(&labels)
                )
            })
            .unwrap_or_default()
    } else {
        String::new()
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
         <tr><th>Čęst rěči</th><td>{}</td></tr>{aspect_rows}{government_rows}<tr><th>Stav</th><td>{}</td></tr>{reliability}\
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

pub(super) fn homograph_notice(
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

pub(super) fn entry_wiki_blocks(
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

pub(super) fn local_graph_block(
    m: &SiteEntryMeta,
    incoming: &[LinkEdge],
    edges: &[LinkEdge],
) -> String {
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

pub(super) fn references_block(m: &SiteEntryMeta) -> String {
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
            let _ = write!(rows, "<tr><th>Praslovjansky prědȯk</th><td><a href='{}'>*{}</a>{}</td><td>rekonstrukcija Wiktionary</td></tr>", esc(&crate::enrich::proto_source_url(p)), esc(p), root);
        }
    }
    rows.push_str("<tr><th>Srodne slova</th><td>anglijska Wiktionary + narodne Wiktionary</td><td>CC BY-SA; konkretne linky sųt v tablicah vyše</td></tr>");
    rows.push_str(
        "<tr><th>Prěgibanje</th><td>interslavic-rs</td><td>mašinno generovane formy</td></tr>",
    );
    rows.push_str("<tr><th>Generator</th><td><a href='https://github.com/gold-silver-copper/Slovowiki'>izvorny kod</a></td><td>pravila, indeks iskanja, statičny eksport</td></tr>");
    format!("<section><h2 id='references'>Izvory</h2><table class='wikitable source-table'><tbody>{rows}</tbody></table></section>")
}

pub(super) fn provenance_block(m: &SiteEntryMeta, build: &BuildMeta) -> String {
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

pub(super) fn category_footer(m: &SiteEntryMeta) -> String {
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

pub(super) fn graph_json(edges: &[LinkEdge]) -> String {
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

pub(super) fn graph_page(edges: &[LinkEdge], metas: &[SiteEntryMeta]) -> String {
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
    let body = format!("<article class='entry'><h1 class='firstHeading'>Semantičny graf</h1><p class='muted'>Statičny spis prvih vęzej; polny kompaktny JSON je v <a href='edges.json'><code>edges.json</code></a>. Filtry rabotajų bez servera.</p><div class='graph-filter'>{filter}</div><div class='stat-grid wiki-stats'>{}</div><h2 id='top'>Najbolje povezane strany</h2><ol>{top_items}</ol><h2 id='edges'>Vęzi</h2><ul class='compact-list'>{items}</ul><script>{GRAPH_FILTER_JS}</script></article>", counts_table("Tipy vęzej", &kind_counts));
    page("Semantičny graf", &body, 0)
}

pub(super) fn contribute_page() -> String {
    let body = "<article class='entry'><h1 class='firstHeading'>Kako pomagati</h1>\
      <p>Projekt je statično generovany: změni podatky, regeneruj sajt, zapusti testy, pošlji prošnju za spoj.</p>\
      <ol><li><code>cargo test</code></li><li><code>cargo run --release -- export --out site</code></li><li>Za ručne noty dodaj <code>data/curation-notes.json</code> s ključem zaglavnogo slova ili id-ja.</li><li>Za grešku v zapisu klikni <i>Popraviti / problem</i> na vrhu strany.</li></ol>\
      <h2>Kuracija bez koda</h2>\
      <ul>\
        <li><b>Semantične pasti</b> (falšive prijatelje): sųt izračunany avtomatično iz kešov dokazov (kolizija poverhnosti × razhodnost glos); noty sę pokazujųt v <a href='text-check.html'>Prověrkě teksta</a> i v CLI <code>check-text</code>.</li>\
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
pub(super) fn entries_json(metas: &[SiteEntryMeta]) -> String {
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

pub(super) fn categories_json(metas: &[SiteEntryMeta]) -> String {
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

pub(super) fn roots_json(roots: &std::collections::BTreeMap<String, Vec<SiteEntryMeta>>) -> String {
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
