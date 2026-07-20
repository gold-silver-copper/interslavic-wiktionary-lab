//! Interslavic Wiktionary Lab — evidence-based candidate generation.
//!
//! Main subcommands (see `Command` below for the full list):
//!   * `export` — generate the static site (cognate-set dictionary) from the
//!     committed caches; `make serve` previews it locally.
//!   * `extract-*` — stream the Wiktextract dumps into the committed caches.
//!   * `evaluate`  — reproducible benchmark against the official dictionary.
//!   * `explain`   — print the generator's full reasoning for one word/gloss.
//!   * `check-text` — verify an Interslavic text against the lexicon.

use anyhow::Result;
use clap::{Parser, Subcommand};
use interslavic_wiktionary_lab::{
    check, derive, dump, enrich, eval, forms, inflect_eval, official, site, DEFAULT_DUMP,
    DEFAULT_ENRICH_CACHE, DEFAULT_LEMMA_CACHE, DEFAULT_OFFICIAL, DEFAULT_PROTO_CACHE,
    DEFAULT_RAW_LEMMA_CACHE, DEFAULT_WIKI_DIR,
};
use std::path::PathBuf;

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
    /// English → Interslavic lookup against the exported static API
    /// (`site/api/en`), using the exact normalization, FNV routing, and retry
    /// ladder the API documents — the reference client, so agents need not
    /// reimplement the router. Requires a prior `export`.
    En {
        /// The English word or phrase to look up (omit with --batch).
        query: Option<String>,
        /// Lexicon-building mode: one query per line (blank lines and
        /// #-comments skipped), one selftest pass, shard cache shared,
        /// output in input order (V11 item 7).
        #[arg(long)]
        batch: Option<PathBuf>,
        /// Emit machine-readable JSON instead of the human table.
        #[arg(long)]
        json: bool,
        /// Directory of a previous `export --out` run.
        #[arg(long, default_value = "site")]
        site: PathBuf,
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
        /// Fit and persist the corpus-coverage calibrator
        /// (data/corpus-calibration.json): isotonic fit on the dev split,
        /// holdout-validated (V11 item 5 / issue #90).
        #[arg(long)]
        fit: bool,
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
        /// Emit a per-status summary and exit nonzero when the text fails the
        /// gate (CI mode; see --max-unknown / --max-agreement).
        #[arg(long)]
        summary: bool,
        /// Maximum allowed unknown tokens before --summary fails (default 0).
        #[arg(long, default_value_t = 0)]
        max_unknown: usize,
        /// Maximum allowed agreement warnings before --summary fails
        /// (default 0).
        #[arg(long, default_value_t = 0)]
        max_agreement: usize,
        /// Skip computed false-friend warnings (skips loading the evidence
        /// caches; faster for pure classification/CI gating).
        #[arg(long)]
        no_warnings: bool,
        /// With --summary: also fail when SEVERE false-friend warnings
        /// (severity high/medium) exceed this count.
        #[arg(long)]
        max_severe_warnings: Option<usize>,
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
            forms::install_cli_quiet_inflection_hook();
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
        Command::En {
            query,
            batch,
            json,
            site,
        } => match (query, batch) {
            (None, Some(file)) => site::run_en_batch(&site, &file, json),
            (Some(q), None) => site::run_en_lookup(&site, &q, json),
            _ => anyhow::bail!("pass exactly one of <QUERY> or --batch <file>"),
        },
        Command::Explain { query, official } => eval::explain(&official, &query),
        Command::ProtoEval { official, out } => eval::run_proto_engine(&official, &out),
        Command::CorpusEval { official, fit } => eval::run_corpus_eval(&official, fit),
        Command::DeriveEval { official, out } => derive::run_eval(&official, &out),
        Command::MultiwordEval { official, out } => eval::run_multiword_eval(&official, &out),
        Command::AspectEval { official, out } => eval::run_aspect_eval(&official, &out),
        Command::EvidenceEval { official, out } => eval::run_evidence_eval(&official, &out),
        Command::InflectEval { official, out } => {
            forms::install_cli_quiet_inflection_hook();
            inflect_eval::run_inflect_eval(&official, &out)
        }
        Command::CheckText {
            file,
            json,
            summary,
            max_unknown,
            max_agreement,
            no_warnings,
            max_severe_warnings,
            official,
        } => {
            // A severity gate over warnings that were never computed would
            // pass vacuously — reject the combination instead of letting a
            // CI job believe it is gated.
            anyhow::ensure!(
                !(no_warnings && max_severe_warnings.is_some()),
                "--max-severe-warnings needs the false-friend computation; drop --no-warnings"
            );
            check::run(
                &official,
                &file,
                json,
                summary.then_some(check::SummaryGate {
                    max_unknown,
                    max_agreement,
                    max_severe_warnings,
                }),
                !no_warnings,
            )
        }
        Command::ChecktextEval { official, out } => check::run_eval(&official, &out),
        Command::Audit { official, out } => eval::run_audit(&official, &out),
        Command::Oracle { official, out } => eval::run_oracle(&official, &out),
        Command::SelectEval { official, out } => eval::run_select_eval(&official, &out),
        Command::RepEval { official, out } => eval::run_rep_eval(&official, &out),
        Command::SynonymEval { official, out } => eval::run_synonym_eval(&official, &out),
        Command::Evaluate { official, out } => eval::run(&official, &out),
    }
}
