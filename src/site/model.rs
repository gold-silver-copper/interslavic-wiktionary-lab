//! Shared site/build data types and routing primitives.
//!
//! This is the bottom of the site-module dependency graph: rendering and
//! orchestration modules may depend on these types, but this module does not
//! depend on any renderer.

use crate::model::{Confidence, MatchStatus};
use anyhow::{bail, Context, Result};

pub(super) const REPO_URL: &str = "https://github.com/gold-silver-copper/Slovowiki";
pub(super) const SITE_URL: &str = "https://grift.rs/Slovowiki/";

/// The razumlivost row-label / chip tooltip (issue #79): what the number is —
/// and is not — plus the population source.
pub(super) const RAZUM_TITLE: &str = "dolja govoriteljev slovjanskyh językov s poznatym srodnym slovom (po atestaciji) — ne izměrjena razumlivosť; izvor populacij: voting machine (steen)";

/// The official-only variant of [`RAZUM_TITLE`], based on the committee's
/// `sameInLanguages` field rather than corpus membership.
pub(super) const RAZUM_TITLE_OFFICIAL: &str = "dolja govoriteljev slovjanskyh językov, v ktoryh slovo je isto po oficialnom slovniku (sameInLanguages) — ne izměrjena razumlivosť; izvor populacij: voting machine (steen)";

/// The matched-entry variant of [`RAZUM_TITLE`], based on the union of corpus
/// membership and the committee's `sameInLanguages` field.
pub(super) const RAZUM_TITLE_MATCHED: &str = "dolja govoriteljev slovjanskyh językov s poznatym srodnym slovom — po srodnyh slovah v korpusu i po oficialnom sameInLanguages; ne izměrjena razumlivosť; izvor populacij: voting machine (steen)";

/// Identity-safe headword routing. Exact scientific spelling wins; a folded
/// spelling resolves only when one page owns that fold.
#[derive(Default)]
pub(super) struct HeadwordIndex {
    exact: std::collections::HashMap<String, Vec<usize>>,
    folded: std::collections::HashMap<String, Vec<usize>>,
}

impl HeadwordIndex {
    pub(super) fn insert(&mut self, title: &str, id: usize) {
        let exact = title.trim().to_lowercase();
        if exact.is_empty() {
            return;
        }
        let fold = crate::orthography::to_standard(&exact);
        let exact_ids = self.exact.entry(exact).or_default();
        if !exact_ids.contains(&id) {
            exact_ids.push(id);
        }
        let folded_ids = self.folded.entry(fold).or_default();
        if !folded_ids.contains(&id) {
            folded_ids.push(id);
        }
    }

    fn unique(ids: &[usize]) -> Option<usize> {
        (ids.len() == 1).then(|| ids[0])
    }

    pub(super) fn resolve(&self, title: &str) -> Option<usize> {
        let exact = title.trim().to_lowercase();
        match self.exact.get(&exact) {
            Some(ids) => Self::unique(ids),
            None => self.resolve_fold(&crate::orthography::to_standard(&exact)),
        }
    }

    pub(super) fn resolve_fold(&self, fold: &str) -> Option<usize> {
        self.folded.get(fold).and_then(|ids| Self::unique(ids))
    }
}

/// Slice-element view shared by family rendering and graph construction. It
/// keeps `site::Prepared` private to orchestration.
pub(super) trait FamilyEntry {
    fn id(&self) -> usize;
    fn display(&self) -> &str;
    fn set(&self) -> &crate::corpus::CognateSet;
}

/// Committee-authored display fields. This data never feeds generation.
#[derive(Clone, Default)]
pub(super) struct OfficialDisplay {
    pub(super) cells: std::collections::HashMap<String, String>,
    pub(super) de: String,
    pub(super) nl: String,
    pub(super) eo: String,
    pub(super) frequency: Option<f32>,
    pub(super) intelligibility: String,
    pub(super) using_example: String,
}

impl OfficialDisplay {
    pub(super) fn from_entry(e: &crate::official::OfficialEntry) -> Self {
        Self {
            cells: e.cells.clone(),
            de: e.de.clone(),
            nl: e.nl.clone(),
            eo: e.eo.clone(),
            frequency: e.frequency,
            intelligibility: e.intelligibility.clone(),
            using_example: e.using_example.clone(),
        }
    }
}

#[derive(Clone)]
pub(super) struct SiteEntryMeta {
    pub(super) id: usize,
    pub(super) title: String,
    pub(super) gloss: String,
    pub(super) pos: String,
    pub(super) conf: Confidence,
    pub(super) score: f32,
    /// Calibrated P(matches an official decision) for generated entries.
    pub(super) prob: Option<f64>,
    /// Pre-match calibrated prior, retained only for matched-entry provenance.
    pub(super) prior: Option<f64>,
    pub(super) n_langs: usize,
    pub(super) n_branches: usize,
    pub(super) borrowed: bool,
    pub(super) official_only: bool,
    pub(super) raw: bool,
    pub(super) official_lemma: Option<String>,
    pub(super) official_sense_id: Option<String>,
    pub(super) aspect: Option<String>,
    pub(super) aspect_partners: Vec<(usize, String)>,
    pub(super) ancestor: String,
    pub(super) languages: Vec<String>,
    pub(super) first: String,
    pub(super) categories: Vec<Vec<String>>,
}

#[derive(Clone)]
pub(super) struct LinkEdge {
    pub(super) source_id: usize,
    pub(super) source_title: String,
    pub(super) target_id: usize,
    pub(super) target_title: String,
    pub(super) kind: String,
}

pub(super) struct BuildMeta {
    pub(super) git: String,
    pub(super) generated: String,
    pub(super) total_entries: usize,
    pub(super) lemma_total: usize,
}

impl BuildMeta {
    pub(super) fn current(total_entries: usize, lemma_total: usize) -> Result<Self> {
        // Cross-revision equivalence checks can pin provenance explicitly. The
        // default remains the checked-out commit, so normal exports keep their
        // truthful revision metadata and per-commit featured-page seed.
        let git = env_override("SLOVOWIKI_BUILD_GIT")?
            .or_else(|| git_output(&["rev-parse", "--short", "HEAD"]))
            .unwrap_or_else(|| "neznany".to_string());
        let generated = match env_override("SOURCE_DATE_EPOCH")? {
            Some(stamp) => format_source_date_epoch(&stamp)?,
            None => git_output(&["show", "-s", "--format=%ct", "HEAD"])
                .map_or_else(|| "0 UNIX".to_string(), |stamp| format!("{stamp} UNIX")),
        };
        Ok(Self {
            git,
            generated,
            total_entries,
            lemma_total,
        })
    }
}

fn env_override(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => {
            let value = value.trim();
            if value.is_empty() {
                bail!("{name} is set but empty");
            }
            Ok(Some(value.to_string()))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => bail!("{name} is not valid Unicode"),
    }
}

pub(super) fn format_source_date_epoch(stamp: &str) -> Result<String> {
    let stamp = stamp.parse::<u64>().with_context(|| {
        format!("SOURCE_DATE_EPOCH must be a non-negative integer, got `{stamp}`")
    })?;
    Ok(format!("{stamp} UNIX"))
}

fn git_output(args: &[&str]) -> Option<String> {
    std::process::Command::new("git")
        .args(args)
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Shared, read-only resources used while rendering entry pages.
#[derive(Clone, Copy)]
pub(super) struct RenderContext<'a> {
    pub(super) enrich: Option<&'a crate::enrich::EnrichIndex>,
    pub(super) xref: Option<&'a crate::enrich::Xref>,
    pub(super) raw_xref: &'a crate::enrich::Xref,
}

#[derive(Clone, Copy)]
pub(super) struct CorpusEntryInput<'a> {
    pub(super) id: usize,
    pub(super) generated: &'a crate::corpus::GeneratedWord,
    pub(super) status: MatchStatus,
    pub(super) official: Option<(usize, &'a str, &'a str)>,
    pub(super) official_grammar: Option<(crate::model::Pos, Option<crate::model::Gender>)>,
    pub(super) official_display: Option<&'a OfficialDisplay>,
    pub(super) family: &'a str,
    pub(super) synonyms: &'a str,
    pub(super) derivation: &'a str,
    pub(super) wiki_top: &'a str,
    pub(super) meta: &'a SiteEntryMeta,
    pub(super) razum_codes: &'a [String],
    pub(super) raw_credit: &'a str,
    pub(super) wiki_bottom: &'a str,
    pub(super) proto_link: &'a str,
    pub(super) context: &'a RenderContext<'a>,
}

#[derive(Clone, Copy)]
pub(super) struct OfficialEntryInput<'a> {
    pub(super) isv: &'a str,
    pub(super) entry: &'a crate::official::OfficialEntry,
    pub(super) id: usize,
    pub(super) synonyms: &'a str,
    pub(super) derivation: &'a str,
    pub(super) wiki_top: &'a str,
    pub(super) meta: &'a SiteEntryMeta,
    pub(super) raw_credit: &'a str,
    pub(super) wiki_bottom: &'a str,
    pub(super) context: &'a RenderContext<'a>,
}

#[derive(Clone, Copy)]
pub(super) struct RawEntryInput<'a> {
    pub(super) display: &'a str,
    pub(super) lemma: &'a crate::dump::RawSlavicLemma,
    pub(super) id: usize,
    pub(super) meta: &'a SiteEntryMeta,
    pub(super) gloss_xref: &'a crate::glossxref::GlossXref,
    pub(super) context: &'a RenderContext<'a>,
}

pub(super) struct SiteEntryInput<'a> {
    pub(super) id: usize,
    pub(super) title: &'a str,
    pub(super) gloss: &'a str,
    pub(super) pos: &'a str,
    pub(super) confidence: Confidence,
    pub(super) score: f32,
    pub(super) probability: Option<f64>,
    pub(super) n_languages: usize,
    pub(super) n_branches: usize,
    pub(super) borrowed: bool,
    pub(super) official_only: bool,
    pub(super) official_lemma: Option<String>,
    pub(super) ancestor: String,
    pub(super) languages: Vec<String>,
    pub(super) wiki_categories: Vec<Vec<String>>,
}

pub(super) fn slug(value: &str) -> String {
    let folded = crate::orthography::ascii_skeleton(value);
    let mut out = String::new();
    let mut dash = false;
    for ch in folded.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "x".to_string()
    } else {
        out
    }
}

pub(super) fn ancestor_slug(meta: &SiteEntryMeta) -> Option<String> {
    if meta.ancestor.trim().is_empty() || meta.borrowed {
        None
    } else {
        Some(slug(meta.ancestor.trim_start_matches('*')))
    }
}

pub(super) fn razum_pct(langs: &[String]) -> u32 {
    let codes: Vec<&str> = langs.iter().map(String::as_str).collect();
    crate::lang::razumlivost(&codes).overall.round() as u32
}

pub(super) fn quality_label(meta: &SiteEntryMeta) -> &'static str {
    if meta.raw {
        "surova atestacija"
    } else if meta.official_only {
        "samo oficialno"
    } else if meta.official_lemma.is_some() {
        "oficialne sovpadenje"
    } else if matches!(meta.conf, Confidence::High) && meta.n_branches >= 3 {
        "vysoko dokazano"
    } else if matches!(meta.conf, Confidence::Low) || meta.n_branches < 2 {
        "trěbuje prověrky"
    } else {
        "generovano"
    }
}

/// The family key of a cognate set, shared by export planning and rendering.
pub(super) fn family_key(set: &crate::corpus::CognateSet) -> Option<String> {
    if set.borrowed {
        let etymon = set.etymon.trim();
        (!etymon.is_empty()).then(|| format!("et:{etymon}"))
    } else {
        proto_stem(set.proto.trim_start_matches('*')).map(|stem| format!("st:{stem}"))
    }
}

pub(super) fn proto_stem(word: &str) -> Option<String> {
    let word: String = word
        .chars()
        .filter(|ch| !crate::orthography::is_combining_mark(*ch))
        .collect();
    const SUFFIXES: &[&str] = &[
        "ovati", "irati", "nǫti", "ostь", "išče", "ьje", "ica", "ina", "ьcь", "ъka", "ъkъ", "ьnъ",
        "ěti", "ati", "iti", "ti", "y", "a", "o", "ъ", "ь", "ę", "ě", "i",
    ];
    let mut suffixes = SUFFIXES.to_vec();
    suffixes.sort_by_key(|suffix| std::cmp::Reverse(suffix.chars().count()));
    for suffix in suffixes {
        if let Some(stem) = word.strip_suffix(suffix) {
            if stem.chars().count() >= 4 {
                return Some(stem.to_string());
            }
        }
    }
    (word.chars().count() >= 4).then_some(word)
}
