//! Bundled CSS and static browser JavaScript used by rendered pages.
//!
//! Keeping these payloads here prevents page renderers from accumulating
//! client-side implementation details. Dynamic search behavior stays in the
//! `search` boundary because it shares the search routing and wire schema.

pub(super) fn css() -> String {
    format!("{BASE_CSS}\n{EXTRA_CSS}")
}

pub(super) fn site_base_js(relative_root: &str) -> String {
    format!("var SITE_BASE='{relative_root}';")
}

/// Builds the "Na toj strane" contents tree in the sidebar from the section
/// headings, and hides the box when a page has none (home / search).
pub(super) const TOC_JS: &str = r#"
(function(){ var nav=document.getElementById('toc-nav'); if(!nav)return;
  var hs=document.querySelectorAll('main h2[id], main h3[id]'); var box=nav.closest('.toc-box');
  if(!hs.length){ if(box)box.style.display='none'; return; }
  var html=''; hs.forEach(function(h){ html+="<a class='toc-"+h.tagName.toLowerCase()+"' href='#"+h.id+"'>"+h.textContent+"</a>"; });
  nav.innerHTML=html;
})();
"#;

/// Shared client-side JS for the form index. The injected fold map and shard
/// counts mirror the Rust API router and are covered by browser self-tests.
pub(super) fn forms_js() -> String {
    const JS: &str = r#"
function escHtml(s){return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;').replace(/'/g,'&#39;');}
function isvFold(s){s=s.toLowerCase().trim();const M=__FOLD_MAP__;let out='';for(const c of s){out+=(M[c]!==undefined)?M[c]:c;}return out;}
let routerOk=null;
async function routerSelftest(base){if(routerOk!==null)return routerOk;try{const j=await fetch(base+'api/router-selftest.json').then(r=>r.json());routerOk=j.shards===__SHARDS__&&j.samples.every(([form,key,shard])=>isvFold(form)===key&&fnv1a32(key)%__SHARDS__===shard);}catch(e){routerOk=false;}if(!routerOk){console.error('router selftest FAILED: JS fold/router drifted from the exporter');}return routerOk;}
function fnv1a32(s){const b=new TextEncoder().encode(s);let h=0x811c9dc5>>>0;for(const x of b){h^=x;h=Math.imul(h,16777619)>>>0;}return h>>>0;}
const shardCache={};
async function isvShard(base,n){if(shardCache[n])return shardCache[n];shardCache[n]=fetch(base+'api/forms/'+n+'.json').then(r=>r.ok?r.json():{records:{}}).catch(()=>({records:{}}));return shardCache[n];}
async function isvLookup(base,q){const ok=await routerSelftest(base);const key=isvFold(q);if(!ok){return{key:key,recs:[],selftestFailed:true};}const shard=fnv1a32(key)%__SHARDS__;const j=await isvShard(base,shard);return{key:key,recs:(j.records&&j.records[key])||[]};}
function asciiVariants(key){if(!/^[a-z -]+$/.test(key))return{keys:[],tooBroad:false};let keys=[''];for(const ch of key){const opts=ch==='c'?['c','č']:ch==='s'?['s','š']:ch==='z'?['z','ž']:[ch];if(keys.length*opts.length>64)return{keys:[],tooBroad:true};keys=keys.flatMap(k=>opts.map(o=>k+o));}return{keys:keys.filter(k=>k!==key),tooBroad:false};}
async function isvLookupBroad(base,q){const exact=await isvLookup(base,q);if(exact.selftestFailed||exact.recs.length)return{...exact,matchedKeys:exact.recs.length?[exact.key]:[],broadened:false};const variants=asciiVariants(exact.key);if(variants.tooBroad)return{...exact,matchedKeys:[],broadened:false,tooBroad:true};const hits=await Promise.all(variants.keys.map(k=>isvLookup(base,k)));const seen=new Set(),recs=[],matchedKeys=[];for(const hit of hits){if(hit.selftestFailed)return hit;if(hit.recs.length)matchedKeys.push(hit.key);for(const rec of hit.recs){const id=JSON.stringify(rec);if(!seen.has(id)){seen.add(id);recs.push(rec);}}}return{key:exact.key,recs,matchedKeys,broadened:recs.length>0};}
function lev(a,b){a=Array.from(a);b=Array.from(b);const row=Array.from({length:b.length+1},(_,i)=>i);for(let i=1;i<=a.length;i++){let prev=row[0];row[0]=i;for(let j=1;j<=b.length;j++){const old=row[j];row[j]=Math.min(row[j]+1,row[j-1]+1,prev+(a[i-1]===b[j-1]?0:1));prev=old;}}return row[b.length];}
const suggestCache={};let suggestTest=null;
function rankSuggestions(rows,q){if(!Array.isArray(rows))return[];const key=isvFold(q),first=Array.from(key)[0]||'';return rows.filter(row=>Array.isArray(row)&&typeof row[0]==='string'&&typeof row[1]==='string'&&Array.from(row[0])[0]===first).map(([k,l])=>[lev(k,key),l]).filter(([d])=>d<=2).sort((a,b)=>a[0]-b[0]||(a[1]<b[1]?-1:a[1]>b[1]?1:0)).slice(0,3).map(x=>x[1]);}
async function rawSuggest(base,q){const key=isvFold(q),first=Array.from(key)[0]||'',shard=fnv1a32(first)%__SUGGEST_SHARDS__;if(!suggestCache[shard])suggestCache[shard]=fetch(base+'api/suggest/'+shard+'.json').then(r=>r.ok?r.json():{rows:[]}).catch(()=>({rows:[]}));const j=await suggestCache[shard];return rankSuggestions(j&&j.rows,q);}
async function suggestionSelftest(base){if(suggestTest)return suggestTest;suggestTest=(async()=>{try{const fixture=await fetch(base+'api/suggest-selftest.json').then(r=>r.json());if(fixture.shards!==__SUGGEST_SHARDS__)return false;for(const [q,want] of fixture.samples){const got=rankSuggestions(fixture.rows,q);if(JSON.stringify(got)!==JSON.stringify(want))return false;}return true;}catch(e){return false;}})();return suggestTest;}
async function webSuggest(base,q){return(await suggestionSelftest(base))?{values:await rawSuggest(base,q),selftestFailed:false}:{values:[],selftestFailed:true};}
function recHtml(base,rec){const[form,lemma,id,pos,analyses,,status,prob,gloss]=rec;
 const st=status==='generated'?('<span class="pill">mašinova rekonstrukcija p='+(prob==null?'?':prob.toFixed(2))+'</span>'):('<span class="pill src-official">'+escHtml(status)+'</span>');
 const an=analyses.length?('<span class="muted">'+escHtml(analyses.join(', '))+'</span>'):'<span class="muted">(citatna forma)</span>';
 return '<li><b>'+escHtml(form)+'</b> — <a href="'+base+'entry/'+id+'.html">'+escHtml(lemma)+'</a> <span class="badge pos">'+escHtml(pos)+'</span> '+an+' '+st+' <span class="muted">'+escHtml(gloss)+'</span></li>';}
"#;
    let fold_map = crate::forms::FOLD_PAIRS
        .iter()
        .map(|(from, to)| format!("'{from}':'{to}'"))
        .collect::<Vec<_>>()
        .join(",");
    JS.replace("__SHARDS__", &crate::forms::SHARDS.to_string())
        .replace(
            "__SUGGEST_SHARDS__",
            &crate::check::SUGGEST_SHARDS.to_string(),
        )
        .replace("__FOLD_MAP__", &format!("{{{fold_map}}}"))
}

pub(super) const RANDOM_PAGE_JS: &str = r#"document.addEventListener('DOMContentLoaded',function(){spotRows().then(function(idx){if(!idx.length)return;var eh=function(s){return String(s==null?'':s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');};var e=idx[Math.floor(Math.random()*idx.length)];var a='entry/'+e[0]+'.html';document.getElementById('random-target').innerHTML='<a href="'+a+'">'+eh(e[1])+'</a> — '+eh(e[2])+'<br><a href="'+a+'">Idi</a>';});});"#;

pub(super) const GRAPH_FILTER_JS: &str = r#"document.querySelectorAll('.graph-filter button').forEach(function(b){b.onclick=function(){var k=b.dataset.kind;document.querySelectorAll('.graph-edge').forEach(function(e){e.style.display=(!k||e.dataset.kind===k)?'':'none';});};});"#;

pub(super) const FORMS_PAGE_JS: &str = r#"async function go(){const q=document.getElementById('formq').value;if(!q)return;const r=await isvLookupBroad('',q);const out=document.getElementById('out');if(r.selftestFailed){out.innerHTML='<p class="notice">Samoprověrka routera ne prošla — klient sę ne shoduje s eksporterom (vidi konzolų). Iskanje je zaprěno da ne davaje krive rezultaty.</p>';return;}if(r.tooBroad){out.innerHTML='<p class="notice">ASCII zapytanje imaje prěmnogo možnyh råzširenij; dodaj fonemične bukvy.</p>';return;}if(!r.recs.length){out.innerHTML='<p>Ničto ne najdeno za ključ <b>'+escHtml(r.key)+'</b>. (Nepoznata forma ili mašinovo prědloženje bez zapisa.)</p>';return;}const broad=r.broadened?(' <span class="muted">ASCII råzširenje: '+r.matchedKeys.map(escHtml).join(', ')+'</span>'):'';out.innerHTML='<p>Ključ: <b>'+escHtml(r.key)+'</b>, '+r.recs.length+' analiz:'+broad+'</p><ul>'+r.recs.map(x=>recHtml('',x)).join('')+'</ul>';}const p=new URLSearchParams(location.search).get('q');if(p){document.getElementById('formq').value=p;go();}"#;

pub(super) const TEXT_CHECK_JS: &str = r#"let noteShards={},notesST=null;async function notesSelftest(){if(notesST!==null)return notesST;try{const j=await fetch('api/notes-selftest.json').then(r=>r.json());notesST=(j.samples||[]).every(([k,s])=>fnv1a32(k)%j.shards===s)?j:false;}catch(e){notesST=false;}if(notesST===false)console.error('notes selftest FAILED: false-friend notes disabled');return notesST;}async function noteFor(key){const st=await notesSelftest();if(!st)return null;const sh=fnv1a32(key)%st.shards;if(!(sh in noteShards))noteShards[sh]=fetch('api/notes/'+sh+'.json').then(r=>r.ok?r.json():{notes:{}}).catch(()=>({notes:{}}));const j=await noteShards[sh];return (j.notes&&j.notes[key])||null;}async function checkText(){const text=document.getElementById('t').value;const toks=[...text.matchAll(/\p{L}+(?:-\p{L}+)*/gu)].map(m=>({text:m[0],start:m.index,end:m.index+m[0].length}));const out=document.getElementById('out');out.innerHTML='<p>Prověrjanje…</p>';const parts=[];let i=0;while(i<toks.length){const item=toks[i],tok=item.text;if(i+1<toks.length){const bi=await isvLookup('',tok+' '+toks[i+1].text);if(bi.selftestFailed){out.innerHTML='<p class="notice">Samoprověrka routera ne prošla — prověrka je zaprěna (vidi konzolų).</p>';return;}if(bi.recs.length){parts.push(render(tok+' '+toks[i+1].text,bi.recs,await noteFor(bi.key),bi.key,[],item.start,toks[i+1].end));i+=2;continue;}}const r=await isvLookup('',tok);if(r.selftestFailed){out.innerHTML='<p class="notice">Samoprověrka routera ne prošla — prověrka je zaprěna (vidi konzolų).</p>';return;}const sug=r.recs.length?{values:[],selftestFailed:false}:await webSuggest('',tok);if(sug.selftestFailed){out.innerHTML='<p class="notice">Samoprověrka predloženij ne prošla — predloženja sųt zaprěna da ne odklanjajųt od CLI.</p>';return;}parts.push(render(tok,r.recs,await noteFor(r.key),r.key,sug.values,item.start,item.end));i+=1;}out.innerHTML='<p>'+parts.join(' ')+'</p><p class='+String.fromCharCode(39)+'muted'+String.fromCharCode(39)+'>Klikni slovo za polnu analizu.</p>';}function applySuggestion(button){const box=document.getElementById('t'),start=Number(button.dataset.start),end=Number(button.dataset.end);if(box.value.slice(start,end)!==button.dataset.old)return;box.value=box.value.slice(0,start)+button.dataset.next+box.value.slice(end);checkText();}function render(tok,recs,note,key,suggestions,start,end){if(!recs.length){const chips=suggestions.map(s=>'<button class="chip" data-start="'+start+'" data-end="'+end+'" data-old="'+escHtml(tok)+'" data-next="'+escHtml(s)+'" onclick="applySuggestion(this)">→ '+escHtml(s)+'</button>').join('');return '<span><a class="chip redlink" href="forms.html?q='+encodeURIComponent(tok)+'" title="nepoznato">'+escHtml(tok)+'</a>'+chips+'</span>';}const gen=recs.every(r=>r[6]==='generated');let ttl=gen?('mašinova rekonstrukcija p='+(recs[0][7]==null?'?':recs[0][7].toFixed(2))):recs.map(r=>r[1]+' ('+(r[4].join(', ')||'lemma')+')').slice(0,4).join('; ');if(note)ttl='⚠ '+note.warning+(note.prefer&&note.prefer.length?' Prefer: '+note.prefer.join(', ')+'.':'')+' — '+ttl;const style=gen?' style="border-color:#c90"':(note?' style="border-color:#c33"':'');return '<a class="chip xref" href="forms.html?q='+encodeURIComponent(tok)+'" title="'+escHtml(ttl)+'"'+style+'>'+(note?'⚠':'')+escHtml(tok)+'</a>';}"#;

pub(super) const BASE_CSS: &str = include_str!("../../static/wiktionary.css");
// Wiktionary/MediaWiki look for the generated pages (light theme, one column).
pub(super) const EXTRA_CSS: &str = r#"
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
.translations-table td.lc{color:var(--muted);white-space:nowrap;width:9em}
.example-official{border-left:3px solid var(--border);background:var(--page);padding:.45rem .75rem;margin:.5rem 0;font-style:italic}
.attr-official{font-size:.82em;margin:.35rem 0 0}
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
.hit .hsrc{font-size:.8em;color:var(--muted);background:var(--th);border:1px solid var(--line);border-radius:2px;padding:.02rem .35rem;margin-left:.35em;white-space:nowrap}

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
.pill.bad{background:#f6dada;border-color:#e0a0a0}
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
.hit .hs,.hit .ha,.hit .hl,.hit .hrz{font-size:.85em;white-space:nowrap}.hit .ha,.hit .hl,.hit .hrz{color:var(--muted)}

/* Razumlivost (issue #79): compact per-branch coverage bars + search chip. */
.razb{display:inline-flex;align-items:center;gap:.2em;margin-left:.45em;font-size:.8em;color:var(--muted);white-space:nowrap}
.razt{display:inline-block;width:2.4em;height:.55em;background:var(--th);border:1px solid var(--line);border-radius:2px;overflow:hidden}
.razf{display:block;height:100%;background:#a7c4ee}
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
.conn ul.quotes{list-style:none;margin:.2em 0 .4em 0;padding:0}
.conn li.quote{font-size:.92em;color:var(--muted);font-style:italic;margin:.15em 0;border-left:2px solid var(--line);padding-left:.5em}
.conn li.quote .cite{font-style:normal;font-size:.9em}
.chips{display:flex;flex-wrap:wrap;gap:.3rem}
a.chip{display:inline-block;background:var(--th);border:1px solid var(--line);border-radius:10px;padding:.05em .55em;font-size:.9em;color:var(--text)}
a.chip:hover{background:#eaf3ff;border-color:var(--link);text-decoration:none}
a.chip.xref{border-color:var(--link);color:var(--link);background:#eaf3ff}
a.chip.xref::before{content:'→\00a0';opacity:.65}
a.chip.xref:hover{background:var(--link);color:#fff}
a.redlink{color:#ba0000!important;border-color:#d33!important;background:#fff5f5!important}
a.redlink::after{content:' ?';font-size:.8em}
.entry-tabs{display:flex;gap:.2rem;border-bottom:1px solid var(--border);margin:.1rem 0 .75rem;flex-wrap:wrap}
.entry-tabs a{display:inline-block;padding:.25rem .65rem;border:1px solid var(--border);border-bottom:none;background:var(--th);color:var(--link);border-radius:2px 2px 0 0}
.entry-tabs a.active{background:#fff;color:var(--text);font-weight:bold;position:relative;top:1px;text-decoration:none}
.catlinks{border:1px solid var(--line);background:var(--page);padding:.35rem .55rem;margin:1.2rem 0 .7rem;font-size:.92em}.catlinks a{color:var(--link);background:none;border:0;padding:0}.catlinks a:visited{color:var(--visited)}.word-index td:first-child{white-space:nowrap}.filter-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:.6rem;border:1px solid var(--line);background:var(--page);padding:.7rem;margin:.8rem 0}.filter-grid label{font-size:.9em;color:var(--muted)}.filter-grid select,.filter-grid input{width:100%;box-sizing:border-box;margin-top:.15rem;padding:.3rem;border:1px solid var(--border);background:#fff}.hq{color:var(--muted);font-size:.82em;margin-left:.4em}.graph-list .badge{min-width:4.5em;text-align:center}.dab{border-left:6px solid #36c}.reference-list li{margin:.25rem 0}.alphabet-index a{display:inline-block;margin:.05rem .35rem .05rem 0}.stat-box h3{font-size:1.05rem;margin:.2rem 0;border-bottom:1px solid var(--line)}.index-summary th{width:24%}.category-list{columns:2;column-gap:2rem}.entry-infobox{float:right;width:260px;margin:.2rem 0 .9rem 1rem;font-size:.9em}.entry-infobox caption{font-family:Georgia,serif;font-weight:bold;padding:.25rem}.entry-grid{display:grid;grid-template-columns:minmax(0,1fr) 320px;gap:1.15rem;align-items:start}.entry-main{min-width:0}.entry-rail{position:sticky;top:.75rem;max-height:calc(100vh - 1.5rem);overflow:auto;align-self:start}.entry-rail .entry-infobox{float:none;width:auto;margin:0 0 .8rem;font-size:.9em}.rail-box{border:1px solid var(--line);background:var(--page);padding:.55rem .65rem;margin:0 0 .8rem;overflow-x:auto}.rail-box h2{font-size:1.18rem;margin:.05rem 0 .45rem}.rail-box .wikitable{font-size:.86em;margin:.2rem 0}.rail-box .wikitable th,.rail-box .wikitable td{padding:.22rem .32rem}.pipeline-diagram{border:1px solid var(--line);background:var(--page);padding:.55rem;white-space:pre-wrap}.graph-filter button{margin:.15rem .25rem .15rem 0;border:1px solid var(--line);background:var(--page);color:var(--link);padding:.2rem .45rem}.source-table th{width:10rem}@media(max-width:1150px){.entry-grid{display:block}.entry-rail{position:static;max-height:none;overflow:visible}.entry-rail .entry-infobox{margin:.6rem 0}.rail-box{margin:.8rem 0}}@media(max-width:900px){.entry-infobox{float:none;width:auto;margin:.6rem 0}}

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

/* Strict wiki link styling: links are plain blue text, never button/chip pills. */
*{border-radius:0!important}
a.ext,a.chip,a.chip.xref,a.redlink,.entry-tabs a,.hit,.dropdown .hit,.dropdown .hit.more,.search-page #page-results .hit,.stat-card{display:inline!important;background:none!important;border:0!important;box-shadow:none!important;padding:0!important;color:var(--link)!important;text-decoration:none!important}
a.ext:hover,a.chip:hover,a.chip.xref:hover,a.redlink:hover,.entry-tabs a:hover,.hit:hover,.dropdown .hit:hover,.search-page #page-results .hit:hover,.stat-card:hover{background:none!important;color:var(--link)!important;text-decoration:underline!important}
a.chip.xref::before,a.redlink::after{content:''!important}.chips{display:block}.chips a{margin-right:.7em}.entry-tabs{display:block;border-bottom:1px solid var(--border);padding-bottom:.2rem}.entry-tabs a{margin-right:1em}.entry-tabs a.active{font-weight:bold;position:static;color:var(--text)!important}.results .hit,.dropdown .hit,.search-page #page-results .hit{display:block!important;padding:.18rem 0!important;border-bottom:1px solid var(--line)!important;color:var(--text)!important}.results .hit b,.dropdown .hit b,.search-page #page-results .hit b{color:var(--link)}button,.portal-box button,.graph-filter button,.hsearch-go,.side-link{background:none!important;border:0!important;box-shadow:none!important;color:var(--link)!important;padding:0!important;font:inherit!important;cursor:pointer!important}.hsearch-go{padding:0 .35rem!important;border:1px solid var(--border)!important;border-left:0!important}.hsearch-go:hover,button:hover,.portal-box button:hover,.graph-filter button:hover,.side-link:hover{text-decoration:underline!important;background:none!important;color:var(--link)!important}.badge,.pill,.reliability{border-radius:0!important}.cat-more summary{color:var(--link);cursor:pointer}.cat-more summary:hover{text-decoration:underline}

/* Wider readable canvas and sticky rails that stay below the fixed header. */
.layout{max-width:1680px}
main{max-width:none;width:100%;box-sizing:border-box;padding-left:2rem;padding-right:2rem}
.site-footer{max-width:1680px;box-sizing:border-box}
.sidebar{top:56px;max-height:calc(100vh - 56px)}
.bottom-meta{border-top:1px solid var(--line);border-bottom:1px solid var(--line);margin:1.2rem 0 .8rem;padding:.35rem 0}.bottom-meta>summary{color:var(--link);cursor:pointer}.bottom-meta>summary:hover{text-decoration:underline}.bottom-meta section{margin:.75rem 0}.bottom-meta h2{font-size:1.15rem}
@media (min-width:1151px){.entry-grid{grid-template-columns:minmax(0,1fr) 340px;gap:1.4rem}.entry-rail{position:sticky;top:64px;max-height:calc(100vh - 76px);overflow-y:auto;overflow-x:hidden}}
@media (max-width:900px){main{width:auto;padding-left:1rem;padding-right:1rem}}

"#;
