#!/usr/bin/env node
// Search-performance harness (issue #71).
//
// Drives the PRODUCTION search client — the exact <script> shipped in the
// exported search.html — inside a Node VM with an instrumented fetch(), and
// measures what a browser would pay for a cold search: files fetched, raw and
// gzipped bytes, JSON-parse time, and total time-to-results per query. Works
// against both index layouts (monolithic search.json and the #71 sharded
// search/ manifest), so before/after runs are directly comparable.
//
// Usage:
//   node tools/search-perf.mjs <site-dir> [--out reports/search-performance.md]
//                              [--label "monolithic (master)"] [--append]
//
// No dependencies beyond Node builtins. Byte counts and hit lists are
// deterministic; wall-clock timings are informational (median of 5 runs).

import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import vm from 'node:vm';
import { performance } from 'node:perf_hooks';

const args = process.argv.slice(2);
const siteDir = args.find(a => !a.startsWith('--'));
if (!siteDir) {
  console.error('usage: node tools/search-perf.mjs <site-dir> [--out FILE] [--label L] [--append]');
  process.exit(2);
}
const outFile = args.includes('--out') ? args[args.indexOf('--out') + 1] : null;
const label = args.includes('--label') ? args[args.indexOf('--label') + 1] : 'unlabeled';
const append = args.includes('--append');

// Representative query mix: exact ISV, flavored/diacritic, prefix, Cyrillic
// alias, Latin source alias, English gloss word, hot single letter, gloss
// segment, raw-only word, and a miss.
const QUERIES = [
  'voda', 'rěka', 'medžu', 'zem', 'пластинка', 'winyl',
  'water', 'baksheesh', 's', 'gramplastinka', 'qqzz',
];

// --- Extract the production search script from the exported page. ---
const searchHtml = fs.readFileSync(path.join(siteDir, 'search.html'), 'utf8');
const scripts = [...searchHtml.matchAll(/<script>([\s\S]*?)<\/script>/g)].map(m => m[1]);
const clientJs = scripts.find(s => s.includes('function scoreAll') || s.includes('function scoreRows'));
if (!clientJs) {
  console.error('could not find the search client <script> in search.html');
  process.exit(1);
}
// SITE_BASE is defined in an earlier inline script on real pages; the VM shim
// resolves fetches against siteDir regardless, so '' is correct here.

function makeContext(stats) {
  const ctx = {
    SITE_BASE: '',
    console,
    URLSearchParams,
    encodeURIComponent,
    setTimeout: (f) => f && undefined, // debounce is irrelevant headlessly
    clearTimeout: () => {},
    history: { replaceState: () => {} },
    location: { search: '' },
    document: {
      getElementById: () => null,
      addEventListener: () => {},
      querySelectorAll: () => [],
    },
    fetch: async (url) => {
      const rel = url.replace(/^\.\//, '');
      const file = path.join(siteDir, rel);
      const buf = fs.readFileSync(file);
      const gz = zlib.gzipSync(buf, { level: 6 });
      stats.fetches.push({ file: rel, raw: buf.length, gz: gz.length });
      return {
        ok: true,
        json: async () => {
          const t0 = performance.now();
          const v = JSON.parse(buf.toString('utf8'));
          stats.parseMs += performance.now() - t0;
          return v;
        },
      };
    },
  };
  ctx.window = ctx;
  vm.createContext(ctx);
  vm.runInContext(clientJs, ctx);
  return ctx;
}

async function runQuery(ctx, q) {
  // Sharded client exposes searchFor(); the monolithic one has ensure()+scoreAll().
  if (typeof ctx.searchFor === 'function') return await ctx.searchFor(q);
  await ctx.ensure();
  return ctx.scoreAll(q);
}

function fmtBytes(n) {
  if (n >= 1 << 20) return (n / (1 << 20)).toFixed(2) + ' MB';
  if (n >= 1 << 10) return (n / (1 << 10)).toFixed(1) + ' KB';
  return n + ' B';
}
const sum = (a, f) => a.reduce((s, x) => s + f(x), 0);
const median = (a) => a.slice().sort((x, y) => x - y)[Math.floor(a.length / 2)];

// Simulated download seconds on a link, from gzipped bytes.
const linkSecs = (gzBytes, mbps) => (gzBytes * 8) / (mbps * 1e6);

const rows = [];
for (const q of QUERIES) {
  // COLD: fresh context (nothing cached), 5 timed repetitions for the median.
  const times = [];
  let stats, hits;
  for (let rep = 0; rep < 5; rep++) {
    stats = { fetches: [], parseMs: 0 };
    const ctx = makeContext(stats);
    const t0 = performance.now();
    hits = await runQuery(ctx, q);
    times.push(performance.now() - t0);
  }
  const raw = sum(stats.fetches, f => f.raw);
  const gz = sum(stats.fetches, f => f.gz);
  rows.push({
    q,
    fetches: stats.fetches.length,
    files: stats.fetches.map(f => f.file).join(' + '),
    raw, gz,
    coldMs: median(times),
    parseMs: stats.parseMs,
    hits: hits.length,
    top: hits.slice(0, 3).map(h => `${h[1][0]}:${h[1][1]}`).join(', '),
  });
}

// WARM session: one context, all queries in sequence (fetch cache shared).
const warmStats = { fetches: [], parseMs: 0 };
const warmCtx = makeContext(warmStats);
const tWarm0 = performance.now();
for (const q of QUERIES) await runQuery(warmCtx, q);
const warmMs = performance.now() - tWarm0;
const warmRaw = sum(warmStats.fetches, f => f.raw);
const warmGz = sum(warmStats.fetches, f => f.gz);

// --- Report ---
const worst = rows.reduce((a, b) => (b.gz > a.gz ? b : a));
let md = '';
md += `\n## ${label}\n\n`;
md += `Site: \`${siteDir}\` · queries: ${QUERIES.length} · gzip level 6 (proxy for Pages compression). `;
md += `Byte counts, fetch counts and hits are deterministic; timings are Node-measured medians (informational).\n\n`;
md += `**Cold worst-case query** (“${worst.q}”): ${worst.fetches} fetch(es), `;
md += `${fmtBytes(worst.raw)} raw / ${fmtBytes(worst.gz)} gzipped → `;
md += `~${linkSecs(worst.gz, 5).toFixed(1)}s @5 Mbps, ~${linkSecs(worst.gz, 40).toFixed(2)}s @40 Mbps (download alone), `;
md += `+ ${worst.coldMs.toFixed(0)} ms parse+score measured.\n\n`;
md += `| query | fetches | raw | gz | ~s @5 Mbps | cold ms | hits | top hits |\n|---|--:|--:|--:|--:|--:|--:|---|\n`;
for (const r of rows) {
  md += `| \`${r.q}\` | ${r.fetches} | ${fmtBytes(r.raw)} | ${fmtBytes(r.gz)} | ${linkSecs(r.gz, 5).toFixed(2)} | ${r.coldMs.toFixed(0)} | ${r.hits} | ${r.top} |\n`;
}
md += `\nWarm session (all ${QUERIES.length} queries, shared cache): ${warmStats.fetches.length} fetches, `;
md += `${fmtBytes(warmRaw)} raw / ${fmtBytes(warmGz)} gzipped, ${warmMs.toFixed(0)} ms total.\n`;
md += `\nFiles fetched per query:\n`;
for (const r of rows) md += `- \`${r.q}\`: ${r.files || '(none)'}\n`;

console.log(md);
if (outFile) {
  const header = '# Search cold-load performance (issue #71 harness: tools/search-perf.mjs)\n';
  if (append && fs.existsSync(outFile)) {
    fs.appendFileSync(outFile, md);
  } else {
    fs.mkdirSync(path.dirname(outFile), { recursive: true });
    fs.writeFileSync(outFile, header + md);
  }
  console.log(`wrote ${outFile}`);
}
