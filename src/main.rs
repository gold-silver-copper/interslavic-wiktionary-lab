//! Interslavic Wiktionary Lab — evidence-based candidate generation.
//!
//! Main subcommands (see `Command` below for the full list):
//!   * `export`    — generate the static site (cognate-set dictionary) from the
//!                   committed caches; `make serve` previews it locally.
//!   * `extract-*` — stream the Wiktextract dumps into the committed caches.
//!   * `evaluate`  — reproducible benchmark against the official dictionary.
//!   * `explain`   — print the generator's full reasoning for one word/gloss.
//!   * `check-text` — verify an Interslavic text against the lexicon.

// The data model and orthography/linguistics helpers intentionally expose a
// broader API surface (evidence relations, alternate configs, helper accessors)
// than any single code path uses; keep them without dead-code noise.
#![allow(dead_code)]

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod aspect;
mod calibrate;
mod check;
mod consensus;
mod corpus;
mod corpus_calibrate;
mod corpus_reference;
mod derive;
mod dump;
mod enrich;
mod eval;
mod flavorize;
mod forms;
mod generator;
mod glossxref;
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
mod thesaurus;

pub const DEFAULT_DUMP: &str = "/Users/kisaczka/Desktop/code/wikidata/raw-wiktextract-data.jsonl";
const DEFAULT_OFFICIAL: &str = "data/official-isv.csv";
const DEFAULT_OVERRIDES: &str = "data/overrides.toml";
const DEFAULT_PROTO_CACHE: &str = "data/proto-slavic.cache.json";
pub const DEFAULT_LEMMA_CACHE: &str = "data/slavic-lemmas.cache.json";
/// RAW single-token Slavic lemma cache (issue #33). Distinct from
/// `DEFAULT_LEMMA_CACHE`: this is the evidence-free extraction and must never be
/// read by the benchmark path.
pub const DEFAULT_RAW_LEMMA_CACHE: &str = "data/raw-slavic-lemmas.cache.json";
const DEFAULT_ENRICH_CACHE: &str = "data/wiktionary-enrich.cache.json";
const DEFAULT_WIKI_DIR: &str = "/Users/kisaczka/Desktop/code/wikidata";

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
    /// Stream the dump once and cache every inherited Slavic lemma with its
    /// Proto-Slavic ancestor (the corpus the cognate-set site is built from).
    ExtractLemmas {
        #[arg(long, default_value = DEFAULT_DUMP)]
        dump: PathBuf,
        #[arg(long, default_value = DEFAULT_LEMMA_CACHE)]
        out: PathBuf,
    },
    /// Stream the dump once and cache every single-token Slavic lemma WITHOUT
    /// the etymological-evidence filter (issue #33, PR-1). A SEPARATE path from
    /// `extract-lemmas`: it keeps low-evidence dictionary words and writes the
    /// distinct raw cache, which no benchmark path reads.
    ExtractRawSlavic {
        #[arg(long, default_value = DEFAULT_DUMP)]
        dump: PathBuf,
        #[arg(long, default_value = DEFAULT_RAW_LEMMA_CACHE)]
        out: PathBuf,
    },
    /// Stream the native RU/PL/CS Wiktionary dumps once and cache per-cognate
    /// enrichment (native etymology, extra senses, related/synonym/antonym links)
    /// for every word that appears in the corpus — shown on the site.
    ExtractEnrich {
        /// Directory holding `ru-extract.jsonl` / `pl-extract.jsonl` / `cs-extract.jsonl`.
        #[arg(long, default_value = DEFAULT_WIKI_DIR)]
        dir: PathBuf,
        #[arg(long, default_value = DEFAULT_LEMMA_CACHE)]
        lemmas: PathBuf,
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        #[arg(long, default_value = DEFAULT_ENRICH_CACHE)]
        out: PathBuf,
    },
    /// Auditable raw-lemma coverage report (issue #35): what datasets fed the raw
    /// Slavic path, how many words were included, and how many excluded and why.
    /// Reads the raw cache + its extraction tally, replicates the export dedup to
    /// split kept lemmas into rendered-raw vs deduped, and measures the native
    /// ru/pl/cs enrichment join. Writes target/eval/raw-coverage.{md,json}.
    Coverage {
        #[arg(long, default_value = "target/eval")]
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
    /// Benchmark the SITE's generation path (corpus::generate_set) against the
    /// official dictionary — the cognate-set path's own leakage-free accuracy.
    /// Prints to stdout only (no report file).
    CorpusEval {
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
    },
    /// Fit the dedicated corpus coverage probability map on the fixed train
    /// split and report metrics on the untouched holdout split.
    CorpusCalibrate {
        #[arg(long, default_value = DEFAULT_LEMMA_CACHE)]
        lemmas: PathBuf,
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        #[arg(long, default_value = "target/eval")]
        out: PathBuf,
        #[arg(long, default_value = crate::calibrate::CORPUS_PATH)]
        artifact: PathBuf,
    },
    /// Benchmark the derivation layer: mined official base→derivative pairs,
    /// seam-aware layer vs naive concatenation (Track A / issue #1).
    DeriveEval {
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        #[arg(long, default_value = "target/eval")]
        out: PathBuf,
    },
    /// Multi-word & aspect-pair benchmark: reflexive `X sę`, two-token
    /// collocations (per-position reconstruction), ipf/pf pair accuracy
    /// (Track B / issue #2).
    MultiwordEval {
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        #[arg(long, default_value = "target/eval")]
        out: PathBuf,
    },
    /// Dedicated perfective↔imperfective pair benchmark (issue #75):
    /// both/either/pairing correctness, dev/holdout, and paired significance.
    AspectEval {
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        #[arg(long, default_value = "target/eval")]
        out: PathBuf,
    },
    /// Evidence-growth audit + augmentation A/B vs the root-absent ceiling
    /// (Track E / issue #4).
    EvidenceEval {
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        #[arg(long, default_value = "target/eval")]
        out: PathBuf,
    },
    /// Inflection validation: blank-cell census + RULE_SPEC §3 grammar
    /// invariants over every official lemma (Track F / issue #5).
    InflectEval {
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        #[arg(long, default_value = "target/eval")]
        out: PathBuf,
    },
    /// Verify an Interslavic text against the lexicon: classify every token
    /// (known-lemma / known-form / generated / unknown), suggest nearest
    /// lemmas, apply curated semantic-trap warnings (issue #11).
    CheckText {
        /// Text file to verify.
        file: PathBuf,
        /// Emit machine-readable JSON instead of the human summary.
        #[arg(long)]
        json: bool,
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
    },
    /// check-text benchmark: fixture classification + agreement gold/error
    /// sets (issue #13).
    ChecktextEval {
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
    /// DIAGNOSTIC-ONLY oracle ladder (V7 §2.4): per-stage headroom upper bounds.
    /// Reads the official answer to make one stage perfect at a time — this can
    /// never feed production; it only ranks where the recoverable error lives.
    Oracle {
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        #[arg(long, default_value = "target/eval")]
        out: PathBuf,
    },
    /// Cluster-selection headroom: measure how much of the editorial wrong-cluster
    /// error a *leakage-free* recognizability rule (max languages/branches,
    /// internationalism-first) recovers vs the answer-reading oracle-cluster.
    SelectEval {
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        #[arg(long, default_value = "target/eval")]
        out: PathBuf,
    },
    /// Representative-selection headroom: measure how much of the +3.7pp
    /// oracle-representative ceiling a *leakage-free* rule (medoid / modal-skeleton
    /// / shortest) recovers vs the fixed REP_PRIORITY surface choice.
    RepEval {
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        #[arg(long, default_value = "target/eval")]
        out: PathBuf,
    },
    /// Synonym-aware accuracy: credit a prediction that reproduces ANY official
    /// Interslavic lemma whose gloss matches the concept (a valid synonym the
    /// committee didn't pick as the headword), and break misses into synonym /
    /// other-sense / not-official. Diagnostic — an honest picture, never a gate.
    SynonymEval {
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        #[arg(long, default_value = "target/eval")]
        out: PathBuf,
    },
    /// Benchmark the candidate generator against the official Interslavic dictionary.
    /// The Proto-Slavic rung reads the committed proto cache (`make extract-proto`).
    Evaluate {
        /// Official dictionary: full export with per-language translations.
        #[arg(long, default_value = DEFAULT_OFFICIAL)]
        official: PathBuf,
        /// Output directory for the report artifacts.
        #[arg(long, default_value = "target/eval")]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Export { official, out } => {
            // The site is the cognate-set dictionary built from the Wiktionary
            // Slavic-lemma corpus when it's available; otherwise fall back to the
            // official-dictionary-seeded site.
            let lemmas = std::path::Path::new(DEFAULT_LEMMA_CACHE);
            if lemmas.exists() {
                site::export_corpus(lemmas, &official, &out)
            } else {
                site::export(&official, &out)
            }
        }
        Command::ExtractProto { dump, out } => dump::extract(&dump, &out),
        Command::ExtractLemmas { dump, out } => dump::extract_lemmas(&dump, &out),
        Command::ExtractRawSlavic { dump, out } => dump::extract_raw_slavic(&dump, &out),
        Command::ExtractEnrich {
            dir,
            lemmas,
            official,
            out,
        } => {
            let corpus = dump::LemmaCorpus::load(&lemmas)?;
            let official = official::load(&official)?;
            // Union the RAW low-evidence Slavic lemmas (issue #33) into the wanted
            // set so raw ru/pl/cs words gain native enrichment too. Loaded from the
            // committed cache; absent → empty, extract-enrich still runs.
            let raw = dump::RawSlavicCorpus::load(std::path::Path::new(DEFAULT_RAW_LEMMA_CACHE))
                .map(|c| c.lemmas)
                .unwrap_or_default();
            let wanted = enrich::build_wanted(&corpus, &official, &raw);
            let total: usize = wanted.values().map(|s| s.len()).sum();
            println!(
                "Enriching {} wanted cognate words across {:?} from {}",
                total,
                enrich::ENRICH_LANGS,
                dir.display()
            );
            enrich::extract(&dir, &wanted, &out)
        }
        Command::Coverage { out } => site::run_coverage(&out),
        Command::Explain { query, official } => eval::explain(&official, &query),
        Command::ProtoEval { official, out } => eval::run_proto_engine(&official, &out),
        Command::CorpusEval { official } => eval::run_corpus_eval(&official),
        Command::CorpusCalibrate {
            lemmas,
            official,
            out,
            artifact,
        } => corpus_calibrate::run(&lemmas, &official, &out, &artifact),
        Command::DeriveEval { official, out } => derive::run_eval(&official, &out),
        Command::MultiwordEval { official, out } => eval::run_multiword_eval(&official, &out),
        Command::AspectEval { official, out } => eval::run_aspect_eval(&official, &out),
        Command::EvidenceEval { official, out } => eval::run_evidence_eval(&official, &out),
        Command::InflectEval { official, out } => site::run_inflect_eval(&official, &out),
        Command::CheckText {
            file,
            json,
            official,
        } => check::run(&official, &file, json),
        Command::ChecktextEval { official, out } => check::run_eval(&official, &out),
        Command::Audit { official, out } => eval::run_audit(&official, &out),
        Command::Oracle { official, out } => eval::run_oracle(&official, &out),
        Command::SelectEval { official, out } => eval::run_select_eval(&official, &out),
        Command::RepEval { official, out } => eval::run_rep_eval(&official, &out),
        Command::SynonymEval { official, out } => eval::run_synonym_eval(&official, &out),
        Command::Evaluate { official, out } => eval::run(&official, &out),
    }
}
