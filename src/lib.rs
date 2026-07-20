//! Reusable library for Slovowiki's linguistic pipeline and static-site exporter.

pub mod aspect;
pub mod calibrate;
pub mod check;
pub mod coincheck;
pub mod consensus;
pub mod corpus;
pub mod derive;
pub mod dump;
pub mod enrich;
pub mod eval;
pub mod falsefriends;
pub mod flavorize;
pub mod forms;
pub mod generator;
pub mod glossxref;
pub mod inflect_eval;
pub mod lang;
pub mod model;
pub mod morph;
pub mod normalize;
pub mod official;
pub mod orthography;
pub mod pipeline;
pub mod proto;
pub mod proto_link;
pub mod site;
pub mod thesaurus;

/// Portable default for a Wiktextract JSONL dump. Callers handling a dump in a
/// different location should pass `--dump` explicitly.
pub const DEFAULT_DUMP: &str = "data/raw-wiktextract-data.jsonl";
pub const DEFAULT_OFFICIAL: &str = "data/official-isv.csv";
pub const DEFAULT_PROTO_CACHE: &str = "data/proto-slavic.cache.json";
pub const DEFAULT_LEMMA_CACHE: &str = "data/slavic-lemmas.cache.json";
/// Evidence-free single-token Slavic lemma cache. This is deliberately
/// separate from [`DEFAULT_LEMMA_CACHE`] and is never read by benchmarks.
pub const DEFAULT_RAW_LEMMA_CACHE: &str = "data/raw-slavic-lemmas.cache.json";
pub const DEFAULT_ENRICH_CACHE: &str = "data/wiktionary-enrich.cache.json";
/// Portable directory default for native Wiktionary JSONL extracts.
pub const DEFAULT_WIKI_DIR: &str = "data/wiktionary";
