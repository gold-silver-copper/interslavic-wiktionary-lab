//! Pure entry-page rendering for generated, official-only, and raw entries.
//!
//! Typed inputs from `model` keep orchestration state out of this boundary;
//! functions here return markup and perform no filesystem writes.

use super::layout::{
    compact, conf_class, esc, pos_code_label, pos_heading, status_pill, truncate, urlencode_q,
};
use super::model::{
    family_key, razum_pct, CorpusEntryInput, FamilyEntry, HeadwordIndex, OfficialDisplay,
    OfficialEntryInput, RawEntryInput, RenderContext, RAZUM_TITLE, RAZUM_TITLE_MATCHED,
    RAZUM_TITLE_OFFICIAL, REPO_URL,
};
use super::navigation::{entry_infobox, razum_row};
use super::search::{search_js, strength_cell, HomeRow};
use super::special::{DerivAgg, DerivRow};
use crate::consensus::MeaningInput;
use crate::generator::Generation;
use crate::lang::Branch;
use crate::model::{Candidate, CandidateSource, Confidence, Evidence, MatchStatus};
use crate::official::OfficialEntry;
use interslavic::{Case as IsvCase, Number as IsvNumber};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

fn page(title: &str, body: &str, depth: usize) -> String {
    super::layout::page(title, body, depth, search_js())
}

/// Render the "word family" section for entry `i`: links to the siblings that
/// share its ancestor stem/etymon. Empty when the entry has no family.
pub(super) fn family_block<T: FamilyEntry>(
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

/// Strip a single leading `!` committee marker (e.g. `!Baum` → `Baum`).
pub(super) fn strip_official_marker(s: &str) -> &str {
    let s = s.trim();
    s.strip_prefix('!').unwrap_or(s).trim()
}

/// A compact frequency chip for the headword line (verbatim committee value).
/// Empty when the row carries no frequency. Display-only.
pub(super) fn official_frequency_chip(freq: Option<f32>) -> String {
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
pub(super) fn official_translations_block(
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
pub(super) fn official_intelligibility_strip(intel: &str) -> String {
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
pub(super) fn official_example_block(ex: &str) -> String {
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
pub(super) fn official_display_sections(o: &OfficialDisplay) -> String {
    let mut s = official_translations_block(&o.cells, &o.de, &o.nl, &o.eo);
    s.push_str(&official_intelligibility_strip(&o.intelligibility));
    s.push_str(&official_example_block(&o.using_example));
    s
}

pub(super) fn corpus_entry_page(input: CorpusEntryInput<'_>) -> String {
    let CorpusEntryInput {
        id,
        generated: g,
        status,
        official,
        official_grammar,
        official_display,
        family,
        synonyms,
        derivation,
        wiki_top,
        meta,
        razum_codes,
        raw_credit,
        wiki_bottom,
        proto_link,
        context,
    } = input;
    let RenderContext {
        enrich,
        xref,
        raw_xref,
    } = *context;
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
    let (infl_pos, infl_gender) = match official_grammar {
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
    let freq_chip = official_display
        .map(|o| official_frequency_chip(o.frequency))
        .unwrap_or_default();
    let official_sections = official_display
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
pub(super) fn official_only_page(input: OfficialEntryInput<'_>) -> String {
    let OfficialEntryInput {
        isv,
        entry: e,
        id,
        synonyms,
        derivation,
        wiki_top,
        meta,
        raw_credit,
        wiki_bottom,
        context,
    } = input;
    let RenderContext {
        enrich,
        xref,
        raw_xref,
    } = *context;
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
pub(super) fn raw_lemma_page(input: RawEntryInput<'_>) -> String {
    let RawEntryInput {
        display,
        lemma,
        id,
        meta,
        gloss_xref: gx,
        context,
    } = input;
    let RenderContext {
        enrich,
        xref,
        raw_xref,
    } = *context;
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
pub(super) fn word_chip(
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

pub(super) fn cross_lingual_meanings_section(
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
pub(super) fn etym_is_stub(s: &str) -> bool {
    let t = s.trim();
    t.is_empty() || t.contains("??")
}

/// The merged etymology `<section>` for a raw lemma page (issue #33): the native
/// RU/PL/CS etymology (stubs dropped, RU transliterated) and the English-dump
/// `etymology_text` (verbatim), each rendered as a source-labelled card. Returns
/// an empty string when neither source has usable etymology.
pub(super) fn raw_etymology_section(
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
            "<div class='etym-src'><div class='src-head'><span class='lc'>anglijska Wiktionary · {}</span> <a class='ext' href='{}'>{}↗</a></div><p class='etym-raw'>{}</p></div>",
            esc(crate::lang::lang_name(&lemma.lang)),
            esc(&crate::enrich::english_source_url(&lemma.word, Some(&lemma.lang))),
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

/// Display for RUNNING TEXT from a source language (quoted etymology
/// paragraphs, gloss truncations): script-faithful transliteration only —
/// Russian is transliterated, other editions pass through (extending them is
/// issue #38). Words displayed AS WORDS (raw headwords, chips, cognate
/// mentions) use [`crate::flavorize::flavorize_word`] instead (issue #62);
/// flavorizing a quoted sentence would misquote the source.
pub(super) fn source_display(lang: &str, text: &str) -> String {
    crate::flavorize::translit_text(lang, text)
}

/// Human-readable borrowing source: `la computare` → `latinsky computare`.
pub(super) fn etymon_display(etymon: &str) -> String {
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
pub(super) fn synonyms_block(
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
pub(super) fn derivation_block(
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

pub(super) fn word_formation_block(derivation: &str, family: &str) -> String {
    if derivation.trim().is_empty() && family.trim().is_empty() {
        String::new()
    } else {
        format!("<section><h2 id='slovotvorstvo'>Slovotvorstvo</h2>{derivation}{family}</section>")
    }
}

/// The cognate set: every attesting Slavic lemma, grouped by branch.
pub(super) fn cognate_block(
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
                "<tr><td class='lc'>{}</td><td><a href='{}'>{}</a>{}{}</td><td class='muted'>{}</td></tr>",
                esc(crate::lang::lang_name(&m.lang)),
                esc(&crate::enrich::english_source_url(&m.word, Some(&m.lang))),
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
                "<tr><td class='lc'>{}</td><td><a href='{}'>{}</a></td><td class='muted'>{}</td></tr>",
                esc(crate::lang::lang_name(&m.lang)),
                esc(&crate::enrich::english_source_url(&m.word, Some(&m.lang))),
                esc(&visible),
                esc(&truncate(&source_display(&m.lang, &m.gloss), 32)),
            );
        }
        s.push_str("</tbody></table></div>");
    }
    s
}

pub(super) fn unified_etymology_section(
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
            "<p>Iz praslovjanskogo <a class='mention' href='{url}'>*{p}</a>. Niže sųt izvorne etimologije iz anglijskogo i narodnyh Wiktionary.</p>",
            url = esc(&crate::enrich::proto_source_url(&g.set.proto)),
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

pub(super) fn unified_official_etymology_section(
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

pub(super) fn english_etymology_cards(members: &[crate::dump::LemmaEntry]) -> String {
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
            "<div class='etym-src'><div class='src-head'><span class='lc'>anglijska Wiktionary · {}</span> <a class='ext' href='{}'>{}↗</a></div>{}</div>",
            esc(crate::lang::lang_name(&m.lang)),
            esc(&crate::enrich::english_source_url(&m.word, Some(&m.lang))),
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
pub(super) fn native_etymology_cards(
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
            esc(crate::lang::lang_name(lang)),
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
pub(super) fn enrich_connections_section(
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
                esc(crate::lang::lang_name(lang)),
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
pub(super) fn enrich_member_block(
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

pub(super) struct CorpusHomeInput<'a> {
    pub(super) entries: usize,
    pub(super) lemmas: usize,
    pub(super) high: usize,
    pub(super) medium: usize,
    pub(super) low: usize,
    pub(super) official: usize,
    pub(super) official_only: usize,
    pub(super) borrowed: usize,
    pub(super) rows: &'a [HomeRow],
}

pub(super) fn corpus_home(input: CorpusHomeInput<'_>) -> String {
    let CorpusHomeInput {
        entries: n,
        lemmas: lemma_total,
        high,
        medium: med,
        low,
        official,
        official_only,
        borrowed,
        rows,
    } = input;
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

pub(super) fn corpus_about(n: usize, lemma_total: usize, official: usize) -> String {
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

pub(super) fn build_input(entry: &OfficialEntry) -> MeaningInput {
    let forms = crate::consensus::source_forms_from_cells(&entry.cells, |code, form| {
        crate::enrich::english_source_url(form, Some(code))
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

pub(super) fn branch_evidence(input: &MeaningInput) -> Vec<Evidence> {
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
// Entry page
// ---------------------------------------------------------------------------

pub(super) fn entry_page(
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

    let official = g.official.as_deref().unwrap_or(entry.isv.as_str());
    let banner = status_banner(status, top, official);
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

pub(super) fn status_banner(status: MatchStatus, top: &Candidate, official: &str) -> String {
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

pub(super) fn etymology_block(g: &Generation) -> String {
    let Some(r) = &g.reconstruction else {
        return "<p class='muted'>Za sej smysl ne najdena praslovjanska rekonstrukcija; forma je iz medžuvětvovogo konsensusa.</p>".to_string();
    };
    let mut s = format!(
        "<p>Iz praslovjanskogo <a class='mention' href='{}'>*{}</a> <span class='muted'>(uvěrjenost povezanja {:.0}%)</span>.</p>",
        esc(&crate::enrich::proto_source_url(&r.word)),
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

pub(super) fn alternatives_block(
    candidates: &[Candidate],
    top_razum_codes: Option<&[String]>,
) -> String {
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

pub(super) fn trace_block(c: &Candidate) -> String {
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

pub(super) fn evidence_block(evidence: &[Evidence]) -> String {
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

/// Render an inflection table with the dictionary's gender when known — the same
/// gendered declension the API records use (single source), so an
/// out-of-lexicon feminine i-stem (točnosť) is not mis-declined as masculine.
pub(super) fn inflection_table_g(
    word: &str,
    pos_code: &str,
    gender: Option<crate::model::Gender>,
) -> String {
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

pub(super) fn case_rows() -> [(&'static str, IsvCase); 6] {
    [
        ("Nominativ", IsvCase::Nom),
        ("Akuzativ", IsvCase::Acc),
        ("Genitiv", IsvCase::Gen),
        ("Dativ", IsvCase::Dat),
        ("Lokativ", IsvCase::Loc),
        ("Instrumental", IsvCase::Ins),
    ]
}

pub(super) fn noun_table(word: &str, gender: Option<crate::model::Gender>) -> String {
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

pub(super) fn adj_table(word: &str) -> String {
    // Build the whole paradigm once (issue #20) and index it — same AdjParadigm
    // as the API records. The four columns are exactly forms::ADJ_COLS. As in
    // noun_table, a panicking build (none in the official corpus) falls back to
    // the per-cell getters so generated cognate pages degrade to "—", not crash.
    let forms = std::panic::catch_unwind(|| interslavic::adj_forms(word)).ok();
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

pub(super) fn verb_table(word: &str, reflexive: bool) -> String {
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

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

pub(super) fn source_class(s: CandidateSource) -> &'static str {
    match s {
        CandidateSource::ProtoSlavicRule => "src-proto",
        CandidateSource::OfficialDictionary => "src-official",
        _ => "src-consensus",
    }
}

/// The badge explainer under the legacy entry headline: measured operating
/// points read live from the committed calibrator (issue #77), never
/// hand-maintained rates that go stale.
pub(super) fn calibration_note(
    c: Confidence,
    cal: Option<&crate::calibrate::Calibration>,
) -> String {
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
