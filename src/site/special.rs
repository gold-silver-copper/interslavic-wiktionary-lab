//! Special-purpose pages and their domain-specific indexes.
//!
//! This boundary owns metrics, datasets, proposals, forms, derivations, and
//! scholarly rule/proto views while reusing the shared layout and model types.

use super::assets::{forms_js, FORMS_PAGE_JS, TEXT_CHECK_JS};
use super::layout::{compact, esc, json_str, pos_code_label, truncate};
use super::model::{
    ancestor_slug, razum_pct, slug, BuildMeta, SiteEntryMeta, RAZUM_TITLE, REPO_URL, SITE_URL,
};
use super::search::search_js;
use crate::lang::Branch;
use crate::model::CandidateSource;
use anyhow::Result;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

fn page(title: &str, body: &str, depth: usize) -> String {
    super::layout::page(title, body, depth, search_js())
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
pub(super) struct RuleRow {
    pub(super) id: usize,
    pub(super) display: String,
    pub(super) before: String,
    pub(super) after: String,
    pub(super) pos: String,
    pub(super) n_langs: usize,
    pub(super) n_branches: usize,
}

/// All firings of one (engine, rule id). The explanation texts are DYNAMIC
/// for several ids (consensus-vote, pick-representative, proto-ancestor, …:
/// they embed the entry's own vote counts / forms), so the first firing's
/// text is kept only as an attributed EXAMPLE — `example_display` names the
/// entry it came from and the pages label it "Priklad (…)", never as the
/// rule's general description. The doc reference IS constant per id.
pub(super) struct RuleAgg {
    pub(super) explanation: String,
    /// Display headword + page id of the entry `explanation` was taken from.
    pub(super) example_display: String,
    pub(super) example_id: usize,
    pub(super) reference: Option<String>,
    pub(super) rows: Vec<RuleRow>,
}

/// (engine, rule id) → aggregated firings. Rule ids are stable but NOT
/// globally unique across engines — "liquid-metathesis" is emitted by both
/// the Proto-Slavic rule engine and the consensus repairs — so the index keys
/// on the pair, the same disambiguation `eval::stage_of_step` uses.
pub(super) type RuleIndex = BTreeMap<(&'static str, String), RuleAgg>;

/// The proto-lemma reflex join (issue #73b). One page per RESOLVED
/// reconstruction word (the accent-folded proto-cache word — canonical, so
/// accented and unaccented ancestor spellings of one lemma share a page),
/// carrying ALL homonymous ProtoEntries under that word. `membership` is the
/// authoritative entry→page map: the infobox "(rekonstrukcija)" link and the
/// root-page links are gated on it, so a link can never point at a proto
/// page that does not list the linking entry.
#[derive(Default)]
pub(super) struct ProtoReflexIndex {
    /// Page slug → group. Slugs derive from the canonical folded word;
    /// collisions between DIFFERENT folded words (cělo vs čelo → "celo") get
    /// deterministic "-2"/"-3"… suffixes in folded-word order.
    pub(super) pages: BTreeMap<String, ProtoReflexPage>,
    /// Entry id → page slug, only for entries whose ancestor resolved.
    pub(super) membership: BTreeMap<usize, String>,
    pub(super) linked: usize,
    pub(super) misses: usize,
}

/// One proto reflex page: a canonical folded reconstruction word, ALL
/// proto-cache entries folding to it (homonyms / accent variants), and the
/// rendered entries whose ancestor resolved to it.
pub(super) struct ProtoReflexPage {
    /// Canonical accent-folded cache word (no '*').
    pub(super) word: String,
    /// ProtoEntry indexes whose folded word equals `word`, in cache order.
    pub(super) recons: Vec<usize>,
    /// Linked entry ids, ascending.
    pub(super) entry_ids: Vec<usize>,
}

/// Engine tag for the rule index, derived from the top candidate's source
/// exactly as the benchmark derives its `is_proto` flag.
pub(super) fn rule_engine(source: CandidateSource) -> &'static str {
    if source == CandidateSource::ProtoSlavicRule {
        "proto"
    } else {
        "konsensus"
    }
}

/// The machine key `rules.json` uses for one rule: `<engine>:<id>`.
pub(super) fn rule_key(engine: &str, id: &str) -> String {
    format!("{engine}:{id}")
}

/// The `rule/` page file stem for one rule: `<engine>-<slug(id)>`. Rule ids
/// are kebab-case ASCII literals today, so `slug` is normally the identity —
/// it only guards a future id against file-name-unsafe characters.
pub(super) fn rule_file_stem(engine: &str, id: &str) -> String {
    format!("{engine}-{}", slug(id))
}

/// Build the rule-fired index from the entries the generated loop renders:
/// only the TOP candidate's trace (pages render `trace_block(top)`, so the
/// index agrees with what pages show), suppressed entries already filtered by
/// the caller. Rows are sorted by display-headword skeleton, deterministic.
pub(super) fn build_rule_index<'a>(
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
pub(super) fn fold_proto_accents(w: &str) -> String {
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
pub(super) fn build_proto_reflex_index<'a>(
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
pub(super) fn rule_page(engine: &str, id: &str, agg: &RuleAgg) -> String {
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
pub(super) fn rules_index_page(rule_index: &RuleIndex) -> String {
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
pub(super) fn rules_json(rule_index: &RuleIndex) -> String {
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
pub(super) fn proto_page(
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
pub(super) fn proto_index_page(
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
pub(super) struct DerivRow {
    pub(super) base_id: usize,
    pub(super) base: String,
    pub(super) form: String,
    pub(super) derived_entry_id: Option<usize>,
    /// Whether the base is an attested official headword (else a machine
    /// reconstruction, marked as such on the pattern page).
    pub(super) official: bool,
}

/// All rendered rows of one derivation pattern plus its human label (the
/// label is constant per pattern; carried here so the page header needs no
/// second derivation pass).
pub(super) struct DerivAgg {
    pub(super) label: &'static str,
    pub(super) rows: Vec<DerivRow>,
}

/// Write the deriv/ pattern pages + the derivations.html overview; returns
/// the total row count for the export's console report.
pub(super) fn write_deriv_pages(
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
pub(super) const DERIV_P_TITLE: &str = "Wilson-95: dolnja granica 95% intervala točnosti togo obrazca na odloženyh oficialnyh parah — ta že věrojętnosť bramkuje mašinove predlogy odvodženj v formovom API (api/forms)";

/// One derivational pattern's page: label + probability header, then every
/// rendered base → derivative row.
pub(super) fn deriv_page(pattern: &str, agg: &DerivAgg, p: f64) -> String {
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
pub(super) fn derivations_index_page(
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
pub(super) struct ProposalRow {
    pub(super) id: usize,
    pub(super) form: String,
    pub(super) pos: String,
    pub(super) prob: f64,
    pub(super) ancestor: String,
    pub(super) n_langs: usize,
    pub(super) n_branches: usize,
    /// Attesting language codes (sorted, deduped) for the razumlivost column
    /// (issue #79). Display-only; NOT written to novel-words.tsv.
    pub(super) langs: Vec<String>,
    pub(super) gloss: String,
    /// `novel` or `near-official` (V12 item 3 reconciliation).
    pub(super) classification: &'static str,
    /// The official byform a near-official proposal reconstructs (empty for
    /// truly novel rows).
    pub(super) official_lemma: String,
}

/// The Predloženja page: ranked novel-word proposals with the calibrated
/// probability, evidence summary and curation notes. The full list is in
/// data/novel-words.tsv; the page shows the propose bucket plus counts.
pub(super) fn proposals_page(
    proposals: &[ProposalRow],
    calibration: Option<&crate::calibrate::CorpusCalibration>,
    curation: &std::collections::HashMap<String, String>,
) -> String {
    let propose_t = crate::calibrate::PROPOSE_T;
    let review_t = crate::calibrate::REVIEW_T;
    // Near-official rows are reconstruction diagnostics (V12 item 3), not
    // proposed words — counted separately, never listed in the propose table.
    let novel: Vec<&ProposalRow> = proposals
        .iter()
        .filter(|r| r.classification == "novel")
        .collect();
    let n_near = proposals.len() - novel.len();
    let n_propose = novel.iter().filter(|r| r.prob >= propose_t).count();
    let n_review = novel.len() - n_propose;
    let mut rows = String::new();
    for r in novel.iter().filter(|r| r.prob >= propose_t).take(600) {
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
            "<b>{n_propose}</b> predloženj (p≥{propose_t:.1}) + <b>{n_review}</b> k pregledu (p≥{review_t:.1}) + <b>{n_near}</b> počti-oficialnyh (rekonstrukcija se razhodi s oficialnoju formoju o 1–2 bukvy — diagnostika zvųkovyh pravil, ne predlog); polny spisok: <a href='novel-words.tsv'>novel-words.tsv</a>."
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

/// The reverse-lookup page for surface forms (issue #11 phase 2): folds the
/// query, routes to the shard client-side, renders every analysis.
pub(super) fn forms_page() -> String {
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>Iskanje form</h1>\
         <p class='lede'>Vpiši kojukoli <b>fleksijnu formu</b> (ne tolika lemmu) — na priklad <span class='mention'>pomoćnogo</span>, <span class='mention'>ljudi</span>, <span class='mention'>piše</span> — i vidiš vse analizy: lemmu, padež/čislo/rod, i stranicu zapisa.</p>\
         <p><input id='formq' type='search' placeholder='forma…' style='min-width:16em' onkeydown='if(event.key===String.fromCharCode(69,110,116,101,114))go()'> <button onclick='go()'>Iskaj</button></p>\
         <div id='out'></div>\
         <p class='muted'>Iste dane služęt strojam: <code>api/forms/&lt;n&gt;.json</code> (indeks razděljeny na {} častij), <code>api/lemmas.json</code>, <code>api/meta.json</code>, <a href='api/agent-guide.md'>api/agent-guide.md</a>.</p>\
         <script>{}{}</script></article>",
        crate::forms::SHARDS,
        forms_js(),
        FORMS_PAGE_JS,
    );
    page("Iskanje form — medžuslovjansky", &body, 0)
}

/// Client-side text verification (issue #11 phase 3): the static twin of the
/// `check-text` CLI. Same tokenizer contract (internal hyphens kept, general
/// two-token lookup so reflexive `sę` verbs and multi-word official lemmas
/// resolve), same semantic-trap notes (fetched from `api/notes.json`), and the
/// CLI's frozen nearest-lemma suggestion contract for unknown tokens.
pub(super) fn text_check_page() -> String {
    let body = format!(
        "<article class='entry'><h1 class='firstHeading'>Prověrka teksta</h1>\
         <p class='lede'>Vstavi medžuslovjansky tekst — vsaky token bųde prověrjeny v slovniku i v polnom indeksu form. Sinje = poznato, žėlta obvodka = mašinova rekonstrukcija, čŕveno = nepoznato, ⚠ = semantična past.</p>\
         <p><textarea id='t' rows='6' style='width:100%'></textarea></p>\
         <p><button onclick='checkText()'>Prověri</button> <span class='muted'>CLI-blizenec: <code>cargo run -- check-text tekst.txt --json</code>.</span></p>\
         <div id='out'></div>\
         <script>{}{}</script></article>",
        forms_js(),
        TEXT_CHECK_JS,
    );
    page("Prověrka teksta — medžuslovjansky", &body, 0)
}

pub(super) fn datasets_page(coverage: &str) -> String {
    let body = format!("<article class='entry'><h1 class='firstHeading'>Fajly za dostavanje</h1><p class='lede'>Statične JSON fajly za raziskovanje i ponovno upotrěbljenje.</p><table class='wikitable'><tr><th>Fajl</th><th>Opis</th></tr><tr><td><a href='entries.json'>entries.json</a></td><td>Metadany zapisa: id, naslov, smysl, čęst rěči, uvěrjenost (kalibrovany kȯšik), <code>prob</code> = modelovo-specifična kalibrovana věrojętnosť (null bez sovmestimoj kalibracije i za oficialne/surove zapisy), <code>official_id</code> = id smysla v izvornom oficialnom slovniku (null za neoficialne), prědȯk, <code>langs_list</code> = sortovany spis kodov atestujučih językov i <code>branch_pattern</code> = vzorec větvi (V/Z/J kombinacija, null bez větvi), <code>aspect</code> i <code>aspect_partners</code> za glagoly — vsako zapytanje po vzorcu atestacije je jedna jq-linija (issues #73, #75).</td></tr><tr><td><a href='edges.json'>edges.json</a></td><td>Vęzi semantičnogo grafa.</td></tr><tr><td><a href='categories.json'>categories.json</a></td><td>Členstvo v kategorijah.</td></tr><tr><td><a href='roots.json'>roots.json</a></td><td>Členstvo v praslovjanskyh korenjah.</td></tr><tr><td><a href='rules.json'>rules.json</a></td><td>Obratny indeks pravil: \u{201e}motor:id-pravila\u{201c} (motor = proto ili konsensus — id pravila ne je unikatny črěz motory) → spis id zapisov, ktoryh pokazany kandidat koristil to pravilo (vidi <a href='rules.html'>indeks pravil</a>; issue #73).</td></tr><tr><td><a href='search/manifest.json'>search/manifest.json</a></td><td>Klientsky indeks iskanja: manifest + razděly po prvoj bukvě (search/*.json; vidi #71).</td></tr><tr><td><a href='novel-words.tsv'>novel-words.tsv</a></td><td>Predloženja novyh slov; samo zaglavje dokolě korpusny model ne imaje vlastnu holdout-validovanu kalibraciju.</td></tr><tr><td><a href='api/meta.json'>api/meta.json</a></td><td>Leksikalny API za stroje: šema, ličby, licencija, routing indeksa.</td></tr><tr><td><a href='api/lemmas.json'>api/lemmas.json</a></td><td>Vse lemmy s statusom, opcionalnoju modelovo-specifičnoju věrojetnostju i vidovymi partnerami glagolov i dokazami rangovanja (frequency, langs, branch_pattern, borrowed; schema 4).</td></tr><tr><td><a href='api/en/meta.json'>api/en/meta.json</a> + api/en/&lt;n&gt;.json</td><td>Anglijsko→medžuslovjansky statičny API za prevodne agenty: normalizovany anglijski ključ → rangovane kandidaty s POS, smyslom, statusom, vidom, semantičnymi notami i povezkoju do <code>api/forms</code>.</td></tr><tr><td><a href='api/aspect-pairs.json'>api/aspect-pairs.json</a></td><td>Produkcijny model glagolskyh par: oficialne i generovane ipf↔pf formy, stranice i pravilo.</td></tr><tr><td>api/forms/&lt;n&gt;.json</td><td>Fleksijny indeks (razděljeny; vidi <a href='api/agent-guide.md'>agent-guide.md</a> i <a href='forms.html'>Iskanje form</a>).</td></tr><tr><td><a href='api/agent-guide.md'>api/agent-guide.md</a></td><td>Vodič za AI agenty i strojne klienty: protokoly iskanja (formy + anglijsky), samoprověrky routerov, pravila dověrjenja, postupy prevoda i prověrjenja teksta.</td></tr><tr><td><a href='build.json'>build.json</a></td><td>Metadany aktualnoj gradby (git, ličby).</td></tr></table>{coverage}</article>");
    page("Fajly za dostavanje", &body, 0)
}

/// The dataset-coverage block on `datasets.html` (issue #35): documents exactly
/// which Slavic-Wiktionary datasets feed the site and the inclusion/exclusion
/// counts. `stats` is the deterministic extraction tally; `rendered`/`deduped` are
/// the site-level split from the raw loop. All numbers regenerate on export.
pub(super) fn datasets_coverage_section(
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

pub(super) fn build_json(build: &BuildMeta) -> String {
    format!(
        "{{\n  \"generated\": {},\n  \"git\": {},\n  \"entries\": {},\n  \"lemmas\": {}\n}}\n",
        json_str(&build.generated),
        json_str(&build.git),
        build.total_entries,
        build.lemma_total
    )
}

pub(super) fn sitemap_xml(metas: &[SiteEntryMeta]) -> String {
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
        let _ = writeln!(s, "  <url><loc>{}{}</loc></url>", SITE_URL, loc);
    }
    for m in metas {
        let _ = writeln!(s, "  <url><loc>{}entry/{}.html</loc></url>", SITE_URL, m.id);
    }
    s.push_str("</urlset>\n");
    s
}

/// A full explainer of every accuracy statistic tracked against the official
/// dictionary. Mostly static content; the confidence-calibration section is
/// rendered live from the committed calibrator (issue #77) so it can never
/// drift from data/score-calibration.json.
pub(super) fn metrics_page(cal: Option<&crate::calibrate::Calibration>) -> String {
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

pub(super) fn about_page(n: usize, norm_rate: f32, exact_rate: f32, top3: f32) -> String {
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
