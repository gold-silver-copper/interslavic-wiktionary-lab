//! Search routing, folding, sharding, rendering, and browser behavior.
//!
//! Shard writing remains here because file names, manifest metadata, row
//! encoding, and client lookup logic form one compatibility-sensitive schema.

use super::layout::{conf_class, json_str};
use crate::model::{Candidate, Confidence, MatchStatus};
use crate::official::OfficialEntry;
use anyhow::Result;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

fn page(title: &str, body: &str, depth: usize) -> String {
    super::layout::page(title, body, depth, search_js())
}

/// The full search-results page (search.html). Reads `?q=` and lists every match;
/// the header search box (present on every page) submits here on Enter.
pub(super) fn search_page() -> String {
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

// ---------------------------------------------------------------------------
// Home page
// ---------------------------------------------------------------------------

/// One row of the home word list.
pub(super) struct HomeRow {
    pub(super) freq: f32,
    pub(super) id: usize,
    pub(super) form: String,
    pub(super) gloss: String,
    pub(super) pos: String,
    pub(super) status: MatchStatus,
    pub(super) conf: Confidence,
    pub(super) score: f32,
    /// Calibrated probability shown in the strength cell (issue #77); `None`
    /// for official-only rows and calibrator-less exports (raw score shown).
    pub(super) prob: Option<f64>,
}

/// Compact strength letter for the search index (V/S/N = high/medium/low).
pub(super) fn conf_letter(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "V",
        Confidence::Medium => "S",
        Confidence::Low => "N",
    }
}

/// The "guess strength" cell: the confidence badge plus the calibrated
/// probability when a calibrator is fitted (issue #77), else the raw ranking
/// score (which is NOT a probability — ECE 0.185).
pub(super) fn strength_cell(conf: Confidence, prob: Option<f64>, raw_score: f32) -> String {
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
pub(super) struct SearchRow {
    pub(super) id: usize,
    pub(super) head: String,
    pub(super) aliases: String,
    /// O/N rows feed browse.json (filter-browse, substring fallback) and the
    /// spotlight sample; R rows are reachable through queries only.
    pub(super) core: bool,
    /// (first, second-or-'_') folded letters of every searchable string.
    pub(super) buckets: std::collections::BTreeSet<(char, char)>,
}

impl SearchRow {
    pub(super) fn full(&self) -> String {
        format!("{},{}]", self.head, self.aliases)
    }
    pub(super) fn no_alias(&self) -> String {
        format!("{},[]]", self.head)
    }
}

/// The client search fold, defined ONCE here and injected into SEARCH_JS as
/// `__SEARCH_FOLD__` — the exporter's bucketing and the browser's query fold
/// can never drift (closes #60 for the search page; the JS additionally
/// NFD-strips combining marks, which agrees with these pairs on every
/// precomposed letter). Latin diacritics fold to base letters; Cyrillic passes
/// through, giving Cyrillic queries their own shard alphabet.
pub(super) const CLIENT_FOLD_PAIRS: &[(char, &str)] = &[
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
pub(super) fn client_fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.to_lowercase().chars() {
        if crate::orthography::is_combining_mark(c) {
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
pub(super) fn search_bucket_pair(s: &str) -> Option<(char, char)> {
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
pub(super) fn search_row_buckets(
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
pub(super) fn shard_file_name(key: &str) -> String {
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
pub(super) const SHARD_SPLIT_BUDGET: usize = 1_500_000;

/// Write the sharded search index under `out_dir/search/`: per-letter shard
/// files, `manifest.json` (key → file, loaded eagerly by the client),
/// `browse.json` (core O/N rows without aliases — empty-query filter browse
/// and the substring fallback), and `spotlight.json` (a deterministic sample
/// of high-confidence rows for the random-word widgets). Ends with a
/// completeness self-check: every bucket pair of every row must RESOLVE (the
/// client's key2→key1 rule) to a shard that contains the row — a violation is
/// a hard error, never a silently unfindable page.
pub(super) fn write_search_index(out_dir: &Path, rows: &[SearchRow]) -> Result<(usize, usize)> {
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
        "splits": splits.iter().map(std::string::ToString::to_string).collect::<Vec<_>>(),
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
pub(super) fn search_js() -> &'static str {
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
pub(super) fn search_keys(candidates: &[Candidate], display: &str) -> Vec<(String, usize)> {
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
pub(super) fn keys_json(keys: &[(String, usize)]) -> String {
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
pub(super) type SourceAlias = (String, String, Vec<String>);

/// The committee's source cells for one official entry, in a deterministic order
/// (the 12 Slavic CSV columns, then `de`/`nl`/`eo`). Kept stable so `search.json`
/// is byte-reproducible despite `cells` being a `HashMap`.
pub(super) fn official_cell_pairs(e: &OfficialEntry) -> Vec<(&str, &str)> {
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
pub(super) fn collect_source_aliases<'a>(
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
pub(super) fn source_aliases_json(aliases: &[SourceAlias]) -> String {
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
pub(super) const SEARCH_JS: &str = r#"
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
