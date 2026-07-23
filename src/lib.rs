//! Reusable library for Slovowiki's linguistic pipeline and static-site exporter.

// One lint regime for every test module (V15.1 item 8): under cfg(test)
// the unwrap-family denies and the four warn-gated pedantic lints are
// allowed crate-wide — tests legitimately unwrap/panic and are not the
// burn-down's target. The lib target compiles WITHOUT cfg(test), so
// clippy's -D warnings still enforces everything on production code.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::panic,
        clippy::unwrap_in_result,
        clippy::indexing_slicing,
        clippy::map_unwrap_or,
        clippy::redundant_closure_for_method_calls,
        clippy::uninlined_format_args,
        clippy::needless_pass_by_value
    )
)]

pub mod aspect;
pub mod calibrate;
pub mod check;
pub mod coincheck;
pub mod consensus;
pub mod corpus;
pub mod derive;
pub mod derive_eval;
pub mod dump;
pub mod enrich;
pub mod eval;
pub mod falsefriends;
pub mod fingerprint;
pub mod flavorize;
pub mod forms;
pub mod generator;
pub mod gloss;
pub mod glossxref;
pub mod inflect_eval;
pub mod lang;
pub mod model;
pub mod morph;
pub mod normalize;
pub mod novel;
pub mod official;
pub mod orthography;
pub mod pipeline;
pub mod postag;
pub mod proto;
pub mod proto_link;
pub mod release;
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
