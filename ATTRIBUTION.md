# Attribution & licensing

This project mixes **original source code** with **third-party data** and
**machine-generated content derived from that data**. They are licensed
differently. Please read this page before reusing anything.

| Part | What | License |
|---|---|---|
| Source code | Everything under `src/`, `Cargo.toml`, `Makefile`, build/site logic | **MIT** — see [`LICENSE`](LICENSE) |
| Bundled data | `data/official-isv.csv`, `data/RULE_SPEC.md`, `data/VOTING_MACHINE_NOTES.md` | **CC BY-SA 4.0** (derived from ShareAlike sources; credits below) |
| Generated content | The Slavic-lemma corpus (`data/slavic-lemmas.cache.json`), the derived words (`data/novel-words.tsv`), the entry pages the site renders, and the benchmark reports under `reports/` | **CC BY-SA 4.0 + GFDL** (inherited from Wiktionary), and **machine-generated / unverified** |

If you redistribute the data or generated content, you must keep the
attribution below and share adaptations under the same terms (ShareAlike).

---

## Sources and required credits

### Official Interslavic dictionary — `data/official-isv.csv`

The full Interslavic (Medžuslovjansky) dictionary, including per-language Slavic
translations, English glosses, part of speech and frequency. It is a community
project developed by the Interslavic community (notably **Jan van Steenbergen**,
**Vojtěch Merunka**, and contributors).

- Source: <https://interslavic-dictionary.com/>
- Used here as the benchmark gold standard and as Slavic evidence.
- Reused under its Creative Commons license (Attribution-ShareAlike). Please
  consult the source for its exact current terms and attribute the Interslavic
  dictionary project when redistributing.

### Interslavic reference materials — `data/RULE_SPEC.md`

The Proto-Slavic → Interslavic rule specification was synthesized from the
official Interslavic learning materials and grammar:

- **interslavic.fun** — orthography, phonology, grammar, word-formation,
  design-criteria pages. <https://interslavic.fun/>
- **Jan van Steenbergen's Interslavic pages** and the "voting machine"
  reference implementation. <http://steen.free.fr/interslavic/>

These materials are © their authors and reused for research under
Attribution-ShareAlike terms. `data/VOTING_MACHINE_NOTES.md` documents analysis
of van Steenbergen's `voting_machine.html` / `transliteration.js`.

### English Wiktionary via Wiktextract

The Proto-Slavic reconstructions (descendant trees, glosses, Balto-Slavic / PIE
references) used to build `data/proto-slavic.cache.json` and the generated
entries come from **English Wiktionary**, extracted with **Wiktextract**.

- English Wiktionary content is dual-licensed **CC BY-SA (3.0 / 4.0) and GFDL**.
  <https://en.wiktionary.org/wiki/Wiktionary:Copyrights>
- Wiktextract (the extraction tool) by **Tatu Ylonen** is MIT-licensed; the
  extracted *data* keeps Wiktionary's license.
  <https://github.com/tatuylonen/wiktextract>
- The raw dump (`raw-wiktextract-data.jsonl`) is **not** redistributed in this
  repository; it is read locally at build time.

Because generated entries derive from Wiktionary, they carry Wiktionary's
**CC BY-SA + GFDL** obligations and must be attributed to English Wiktionary and
its contributors.

### `interslavic-rs` / `interslavic` crate

Inflection and conjugation tables are produced by the local `interslavic` crate
(dependency at `../interslavic-rs`). Its own license governs that code.

---

## How to attribute when you reuse this

For the **code**: keep the MIT copyright notice.

For the **data or generated content**, credit at minimum:

> Slavic evidence and Interslavic lemmas from the Interslavic dictionary
> (interslavic-dictionary.com) and Interslavic reference materials by Jan van
> Steenbergen (interslavic.fun, steen.free.fr); etymological data from English
> Wiktionary (CC BY-SA / GFDL) via Wiktextract. Interslavic candidate forms are
> machine-generated and not verified standard Interslavic.

## Nature of the generated data

Generated Interslavic candidate forms are **algorithmic reconstructions**, not
authoritative standard Interslavic. Every generated entry is labeled as
machine-generated with a confidence score, and the site marks whether a form
matches the official dictionary. Do not present generated forms as official
Interslavic without review.
