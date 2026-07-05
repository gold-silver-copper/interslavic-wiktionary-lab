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
mod eval;
mod generator;
mod lang;
mod model;
mod morph;
mod normalize;
mod official;
mod orthography;
mod overrides;
mod proto;
mod site;

const DEFAULT_DUMP: &str = "/Users/kisaczka/Desktop/code/english/raw-wiktextract-data.jsonl";
const DEFAULT_DATA: &str = "data/wiktionary-lab.json";
const DEFAULT_OFFICIAL: &str = "data/official-isv.csv";
const DEFAULT_OVERRIDES: &str = "data/overrides.toml";

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
    /// Build the site dataset by running the generator over the official
    /// dictionary's Slavic evidence.
    Build {
        /// Official dictionary (full interslavic-dictionary.com export).
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        /// Output JSON data artifact.
        #[arg(long, default_value = DEFAULT_DATA)]
        output: PathBuf,
        /// Optional Wiktextract dump (reserved for Proto-Slavic augmentation).
        #[arg(long, default_value = DEFAULT_DUMP)]
        dump: Option<PathBuf>,
    },
    /// Launch a local HTTP server over the generated dataset.
    Serve {
        #[arg(long, default_value = DEFAULT_DATA)]
        data: PathBuf,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8765)]
        port: u16,
    },
    /// Explain the generator's output for one word or gloss (manual spot-check).
    Explain {
        /// A gloss (English) or an official Interslavic lemma to look up.
        query: String,
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
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
        Command::Build {
            official,
            output,
            dump: _dump,
        } => site::build(&official, &output),
        Command::Serve { data, host, port } => site::serve(&data, &host, port),
        Command::Explain { query, official } => eval::explain(&official, &query),
        Command::Evaluate {
            official,
            dump,
            out,
        } => eval::run(&official, dump.as_deref(), &out),
    }
}
