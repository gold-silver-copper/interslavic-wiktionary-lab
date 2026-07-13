#!/usr/bin/env node
// End-to-end guard for issue #81 browser authoring tools.

import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";

const root = path.resolve(process.argv[2] || "site");
const html = fs.readFileSync(path.join(root, "forms.html"), "utf8");
const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
const client = scripts.find((s) => s.includes("function isvLookupBroad"));
if (!client) throw new Error("forms client script not found");

const fetched = [];
const responseOverrides = new Map();
const localFetch = async (url) => {
  const pathname = new URL(url, "https://slovowiki.invalid/").pathname.replace(/^\//, "");
  fetched.push(pathname);
  if (responseOverrides.has(pathname)) {
    return { ok: true, json: async () => responseOverrides.get(pathname) };
  }
  const file = path.join(root, pathname);
  if (!fs.existsSync(file)) return { ok: false, json: async () => ({}) };
  return { ok: true, json: async () => JSON.parse(fs.readFileSync(file, "utf8")) };
};
const textarea = { value: "", innerHTML: "" };
const context = {
  console,
  fetch: localFetch,
  TextEncoder,
  URLSearchParams,
  location: { search: "" },
  document: { getElementById: (id) => (id === "t" ? textarea : { value: "", innerHTML: "" }) },
};
vm.createContext(context);
vm.runInContext(client, context);
const run = (source) => vm.runInContext(source, context);

const exact = await run("isvLookup('', 'pomočnogo')");
if (!exact.recs.length) throw new Error("exact phonemic lookup regressed");
const broadened = await run("isvLookupBroad('', 'pomocnogo')");
if (!broadened.broadened || !broadened.matchedKeys.includes("pomočnogo")) {
  throw new Error(`ASCII fallback failed: ${JSON.stringify(broadened)}`);
}
const ambiguous = await run("isvLookupBroad('', 'bese drzal')");
for (const key of ["beše drzal", "beše držal"]) {
  if (!ambiguous.matchedKeys.includes(key)) {
    throw new Error(`ASCII ambiguity lost ${key}: ${JSON.stringify(ambiguous.matchedKeys)}`);
  }
}
if (!run("asciiVariants('cszcszc')").tooBroad) {
  throw new Error("ASCII expansion cap did not fail closed");
}
const suggestion = await run("webSuggest('', 'domm')");
if (suggestion.selftestFailed || JSON.stringify(suggestion.values) !== JSON.stringify(["dom", "doma", "Don"])) {
  throw new Error(`browser/CLI suggestion fixture drift: ${JSON.stringify(suggestion)}`);
}
const suggestionShardFetches = fetched.filter((file) => /^api\/suggest\/\d+\.json$/.test(file));
if (suggestionShardFetches.length !== 1) {
  throw new Error(`one unknown token fetched unrelated suggestion shards: ${suggestionShardFetches}`);
}
const malformedShard = await run("fnv1a32('x') % 64");
responseOverrides.set(`api/suggest/${malformedShard}.json`, { rows: { invalid: true } });
const malformedSuggestion = await run("webSuggest('', 'xyzq')");
if (malformedSuggestion.selftestFailed || malformedSuggestion.values.length !== 0) {
  throw new Error(`malformed suggestion data did not fail closed: ${JSON.stringify(malformedSuggestion)}`);
}
const unavailableSuggestion = await run("webSuggest('missing/', 'yyyy')");
if (unavailableSuggestion.selftestFailed || unavailableSuggestion.values.length !== 0) {
  throw new Error(`unavailable suggestion data did not fail closed: ${JSON.stringify(unavailableSuggestion)}`);
}

const checkerHtml = fs.readFileSync(path.join(root, "text-check.html"), "utf8");
if (!checkerHtml.includes("webSuggest('',tok)") || !checkerHtml.includes("applySuggestion(this)")) {
  throw new Error("text checker does not render click-to-replace suggestions");
}
const checkerScripts = [...checkerHtml.matchAll(/<script>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
const checkerClient = checkerScripts.find((script) => script.includes("function applySuggestion"));
if (!checkerClient) throw new Error("text checker client script not found");
const checkerContext = {
  console,
  fetch: localFetch,
  TextEncoder,
  URLSearchParams,
  location: { search: "" },
  document: { getElementById: (id) => (id === "t" ? textarea : { value: "", innerHTML: "" }) },
};
vm.createContext(checkerContext);
vm.runInContext(checkerClient, checkerContext);
vm.runInContext("checkText=()=>{}", checkerContext);
textarea.value = "domm x domm";
vm.runInContext(
  "applySuggestion({dataset:{old:'domm',next:'dom',start:'7',end:'11'}})",
  checkerContext,
);
if (textarea.value !== "domm x dom") {
  throw new Error(`click replaced the wrong duplicate token: ${textarea.value}`);
}
const entries = JSON.parse(fs.readFileSync(path.join(root, "entries.json"), "utf8"));
const bez = entries.find((entry) => entry.title === "bez" && entry.pos === "prep" && entry.official);
if (!bez) throw new Error("official preposition fixture 'bez' missing");
const bezHtml = fs.readFileSync(path.join(root, "entry", `${bez.id}.html`), "utf8");
if (!bezHtml.includes("<th>Upravljanje</th>") || !bezHtml.includes("bez + gen.")) {
  throw new Error("preposition government missing from entry infobox");
}

const suggestFiles = fs.readdirSync(path.join(root, "api", "suggest"));
const suggestBytes = suggestFiles.reduce(
  (total, file) => total + fs.statSync(path.join(root, "api", "suggest", file)).size,
  fs.statSync(path.join(root, "api", "suggest-selftest.json")).size,
);
const largest = Math.max(...suggestFiles.map((file) => fs.statSync(path.join(root, "api", "suggest", file)).size));
console.log(
  `author tools valid: exact + ASCII fallback (${ambiguous.matchedKeys.length} ambiguous keys), ` +
    `CLI-parity suggestions, government display; suggestion payload ${suggestBytes} bytes total, ${largest} max cold shard`,
);
