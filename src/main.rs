//! Interslavic Wiktionary Lab — evidence-based candidate generation.
//!
//! Subcommands:
//!   * `build`    — generate the site dataset from the official dictionary's
//!                  Slavic evidence (fast, self-contained).
//!   * `serve`    — local Wiktionary-style website over the generated dataset.
//!   * `evaluate` — reproducible benchmark against the official dictionary.
//!   * `explain`  — print the generator's full reasoning for one word/gloss.

// The data model and orthography/linguistics helpers intentionally expose a
// broader API surface (evidence relations, alternate configs, helper accessors)
// than any single code path uses; keep them without dead-code noise.
#![allow(dead_code)]

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod consensus;
mod dump;
mod eval;
mod generator;
mod lang;
mod model;
mod morph;
mod normalize;
mod official;
mod orthography;
mod overrides;
mod pipeline;
mod proto;
mod proto_link;
mod site;

const DEFAULT_DUMP: &str = "/Users/kisaczka/Desktop/code/english/raw-wiktextract-data.jsonl";
const DEFAULT_DATA: &str = "data/wiktionary-lab.json";
const DEFAULT_OFFICIAL: &str = "data/official-isv.csv";
const DEFAULT_OVERRIDES: &str = "data/overrides.toml";
const DEFAULT_PROTO_CACHE: &str = "data/proto-slavic.cache.json";

#[derive(Parser)]
#[command(
    author,
    version,
    about = "Naučno obosnovany medžuslovjansky generator kandidatov"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate the static website (one HTML page per meaning + client-side
    /// search) — no server, GitHub Pages hostable.
    Export {
        /// Official dictionary (full interslavic-dictionary.com export).
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        /// Output directory for the static site.
        #[arg(long, default_value = "site")]
        out: PathBuf,
    },
    /// Stream the Wiktextract dump once and cache all Proto-Slavic entries.
    ExtractProto {
        #[arg(long, default_value = DEFAULT_DUMP)]
        dump: PathBuf,
        #[arg(long, default_value = DEFAULT_PROTO_CACHE)]
        out: PathBuf,
    },
    /// Explain the generator's output for one word or gloss (manual spot-check).
    Explain {
        /// A gloss (English) or an official Interslavic lemma to look up.
        query: String,
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
    },
    /// Proto-engine-only benchmark: proto derivation vs official on linked words.
    ProtoEval {
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        #[arg(long, default_value = "target/eval")]
        out: PathBuf,
    },
    /// Data-quality / ceiling audit: classify misses and cognate cohesion.
    Audit {
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        #[arg(long, default_value = "target/eval")]
        out: PathBuf,
    },
    /// Benchmark the candidate generator against the official Interslavic dictionary.
    Evaluate {
        /// Official dictionary: full export with per-language translations.
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        /// Optional Wiktextract dump for the Proto-Slavic benchmark path.
        #[arg(long)]
        dump: Option<PathBuf>,
        /// Output directory for the report artifacts.
        #[arg(long, default_value = "target/eval")]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Export { official, out } => site::export(&official, &out),
        Command::ExtractProto { dump, out } => dump::extract(&dump, &out),
        Command::Explain { query, official } => eval::explain(&official, &query),
        Command::ProtoEval { official, out } => eval::run_proto_engine(&official, &out),
        Command::Audit { official, out } => eval::run_audit(&official, &out),
        Command::Evaluate {
            official,
            dump,
            out,
        } => eval::run(&official, dump.as_deref(), &out),
    }
}
