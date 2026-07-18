# Implementation prompt: scientific candidate rules and Wiktionary-grade Interslavic website

Improve `interslavic-wiktionary-lab` from a prototype into a scientifically grounded Interslavic lexicon generator and local Wiktionary-style site.

## Primary goals

1. Replace the current placeholder Proto-Slavic → Interslavic candidate function with a transparent, evidence-weighted rule engine based on comparative Slavic linguistics and Interslavic design principles.
2. Use external Interslavic resources as references, especially:
   - https://interslavic.fun/learn/introduction/
   - https://interslavic.fun/learn/orthography/
   - https://interslavic.fun/learn/phonology/
   - https://interslavic.fun/learn/grammar/nouns/
   - https://interslavic.fun/learn/vocabulary/word-formation/
   - https://interslavic.fun/learn/vocabulary/derivation/
   - https://interslavic.fun/learn/misc/design-criteria/
   - https://interslavic.fun/learn/faq/
   - https://www.interslavic.org/
   - https://steen.free.fr/interslavic/grammar.html
   - https://interslavic-dictionary.com/ and its grammar/dictionary material
3. Use historical linguistic sources from English Wiktionary/Wiktextract:
   - Proto-Slavic (`sla-pro`)
   - Proto-Balto-Slavic (`ine-bsl-pro`)
   - Proto-Indo-European (`ine-pro`)
   - all extracted Slavic descendant languages and forms
4. Use the `interslavic-rs` / `interslavic` crate inside the website to generate full inflection/conjugation tables, similar to Wiktionary declension/conjugation tables for foreign words.

## Candidate generation requirements

Design the generator as an explicit scoring pipeline, not a black box:

```text
Proto-Slavic source form
  + Proto-Balto-Slavic / PIE evidence
  + descendant forms across Slavic branches
  + official Interslavic dictionary matches/overrides
  + Interslavic orthography/phonology/design criteria
  -> candidate forms with scores, explanations, citations
```

The output for every generated candidate should include:

- candidate spelling;
- source Proto-Slavic form;
- normalized descendant forms by language;
- Slavic branch coverage: East, West, South;
- modern core-language coverage;
- Proto-Balto-Slavic and PIE references when available;
- rule trace explaining which transformations were applied;
- confidence score;
- review status;
- citations/links to English Wiktionary source pages.

Use a rule-trace data model such as:

```rust
struct CandidateTrace {
    source: String,
    steps: Vec<RuleStep>,
    descendant_support: Vec<SupportEvidence>,
    score: f32,
    warnings: Vec<String>,
}

struct RuleStep {
    rule_id: String,
    before: String,
    after: String,
    reason: String,
    citation_or_doc: Option<String>,
}
```

## Scientific rule-engine direction

Implement rules in stages:

1. **Normalization**
   - Strip reconstruction marker `*` only for generated display forms.
   - Preserve original Proto-Slavic form as citation data.
   - Normalize stress/diacritics into separate metadata, not by destructive deletion only.
   - Keep yers, nasal vowels, palatalization marks, and accent info available for rule decisions.

2. **Proto-Slavic → Interslavic orthographic mapping**
   - Implement deterministic mappings for yers, nasals, jat, palatal consonants, liquid diphthongs, and common endings.
   - Prefer mappings supported by official Interslavic orthography/phonology docs.
   - Every mapping must have a rule id and explanation.

3. **Descendant consensus scoring**
   - Group descendants by Slavic branch.
   - Reward candidates recognizable across East, West, and South Slavic.
   - Avoid overfitting to one large language.
   - Distinguish inherited cognates from borrowings, reshaped forms, dialectal forms, and descendants marked with raw tags.

4. **Official Interslavic dictionary integration**
   - Load official Interslavic dictionary data when available.
   - If an official entry exists, treat it as authoritative or as a high-priority override.
   - Still show generated evidence and cognate tables.

5. **Manual curation**
   - Add a small review/override file format, e.g. TOML or JSON:

```toml
[[entry]]
proto = "*duša"
official = "duša"
status = "reviewed"
notes = "Matches official Interslavic and broad Slavic consensus."
```

## Website requirements

Keep the current no-SQLite architecture:

- Build phase streams Wiktextract JSONL and writes a compact data artifact.
- Server phase loads that artifact into native Rust structs and in-memory indexes.
- No SQLite or external DB.

Improve page rendering to be closer to Wiktionary:

- Interslavic heading
- Etymology
- Pronunciation
- Part of speech section
- Definitions
- Full inflection/conjugation tables
- Cognates/descendants
- Rule trace
- References
- External Wiktionary source layer

Use local Wiktionary-like CSS only. Do not hotlink Wikimedia CSS/JS.

## Interslavic-rs integration

Add/keep a dependency on the local or published `interslavic` crate.

For generated entries, render forms like Wiktionary does:

### Nouns
Use `ISV::noun` / `ISV::noun_with` to generate:

| Case | Singular | Plural |
|------|----------|--------|
| Nominative | ... | ... |
| Accusative | ... | ... |
| Genitive | ... | ... |
| Locative | ... | ... |
| Dative | ... | ... |
| Instrumental | ... | ... |

When gender/animacy is uncertain, show a warning and either:

- use default inference; or
- show multiple candidate paradigms with labels.

### Adjectives
Use `ISV::adj` to generate case/number/gender forms, including masculine animate/inanimate distinctions.

### Verbs
Use `ISV::verb` to generate present-tense forms first. If the crate later supports more tenses, extend the table.

All generated forms should be marked as machine-generated unless they match official reviewed data.

## External Wiktionary source layer

When a user clicks a Wiktionary source link:

- do not navigate away directly;
- open an internal `/wiktionary?url=...&back=...` wrapper page;
- render an outer Interslavic-site toolbar with:
  - back link to the Interslavic entry;
  - open-on-Wiktionary link;
  - source/license note;
- embed the external source in an iframe if allowed;
- if Wikimedia blocks iframe embedding, show the open link clearly.

## Validation

Run:

```bash
cargo fmt
cargo check
cargo run -- build --max-proto 200 --max-lexemes 10000
cargo run -- serve
```

Verify manually:

- `/`
- `/entry/1`
- `/lexemes`
- `/stats`
- `/api/entries?q=bog`
- `/wiktionary?...`

## Non-goals for this pass

- Do not claim generated forms are final standard Interslavic without review.
- Do not copy or hotlink Wikimedia styles/scripts.
- Do not introduce SQLite or a database server.
- Do not hide uncertainty; show confidence, warnings, and evidence.
