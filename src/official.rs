//! Loader for the official Interslavic dictionary (`official-isv.csv`).
//!
//! This is the complete interslavic-dictionary.com export. Crucially it already
//! contains, for every entry, the modern Slavic cognate in each language plus an
//! English gloss and part of speech. That makes it a *self-contained,
//! leakage-free* benchmark: feed the per-language cognates to the generator and
//! check whether it reproduces the `isv` lemma — without ever showing it the
//! answer.

use crate::lang::Branch;
use crate::model::{parse_noun_traits, NounTraits, Pos};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct OfficialEntry {
    pub id: String,
    /// Official flavored Interslavic lemma (the benchmark target).
    pub isv: String,
    pub addition: String,
    pub pos_raw: String,
    pub pos: Pos,
    pub noun_traits: NounTraits,
    pub english: String,
    /// Which languages/branches the word is natively shared in (`v z j` etc.).
    pub same_in: String,
    pub genesis: String,
    /// Raw per-language cells keyed by language code.
    pub cells: HashMap<String, String>,
    pub frequency: Option<f32>,
    /// German cell (committee reference translation).
    pub de: String,
    /// Dutch cell (committee reference translation).
    pub nl: String,
    /// Esperanto cell (committee reference translation).
    pub eo: String,
    /// Per-language mutual-intelligibility strip, e.g. `be~ bg+ cs~ …`; the bare
    /// `!` placeholder means "no data".
    pub intelligibility: String,
    /// Verbatim committee example sentence (rare; empty when absent).
    pub using_example: String,
}

impl OfficialEntry {
    /// Individual authoritative citation forms from the dictionary's `isv`
    /// cell. About 230 rows encode byforms as comma-separated spellings
    /// (`iměti, imati`); they are lexical aliases, not one lemma containing a
    /// comma. Coinage-flagged rows are excluded as a whole, while exclamatory
    /// notation is excluded per form. Parenthesized government hints are
    /// stripped by the same citation sanitizer used by the forms API.
    pub fn citation_forms(&self) -> Vec<String> {
        if self.isv.contains('#') {
            return Vec::new();
        }
        self.isv
            .split(',')
            .map(str::trim)
            .filter(|form| !form.is_empty() && !form.contains('!'))
            .filter_map(crate::forms::citation)
            .collect()
    }

    /// True when the lemma is a single inflectable word we can benchmark on
    /// (skip multi-word phrases, coinage-flagged forms, bracketed notes).
    pub fn is_benchmarkable(&self) -> bool {
        let w = self.isv.trim();
        !w.is_empty()
            && !w.contains(' ')
            && !w.contains('!')
            && !w.contains('#')
            && !w.contains('"')
            && !w.contains('(')
            && !w.contains('[')
            && matches!(
                self.pos,
                Pos::Noun | Pos::Verb | Pos::Adjective | Pos::Adverb | Pos::Numeral | Pos::Pronoun
            )
    }

    /// Registry language codes expanded from the `same_in` column — the
    /// committee's own record of which languages natively share the word
    /// (the honest membership for an official-only razumlivost, issue #79;
    /// the translation cells are filled for every language and say nothing
    /// about relatedness). Token inventory of the committed CSV: branch
    /// markers `v`/`z`/`j` (expanded to the branch's modern CSV languages),
    /// registry codes (ru pl cs bg uk mk sl be sk hr sr cu rue hsb csb dsb;
    /// `sh` expands to sr+hr+bs), and the dictionary's own group codes
    /// `ub`=uk+be, `cz`=cs+sk, `sb`=Sorbian (hsb+dsb), `bm`=bg+mk,
    /// `yu`=Slovene+BCMS. A `~` suffix (partial match) and stray
    /// punctuation (`(sh)`, `#ru`) are stripped and count fully; unknown
    /// leftovers (`ps`, `mg`, `sx` — one or two rows each) are skipped.
    /// Sorted, deduped; empty when the column is empty (~61% of rows).
    pub fn same_in_langs(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        for tok in self.same_in.split_whitespace() {
            let tok = tok.trim_matches(|c: char| !c.is_alphabetic());
            let expanded: Vec<&'static str> = match tok {
                "v" | "z" | "j" => {
                    let branch = match tok {
                        "v" => Branch::East,
                        "z" => Branch::West,
                        _ => Branch::South,
                    };
                    crate::lang::official_slavic_cols()
                        .iter()
                        .filter(|l| l.modern && l.branch == branch)
                        .map(|l| l.code)
                        .collect()
                }
                "ub" => vec!["uk", "be"],
                "cz" => vec!["cs", "sk"],
                "sb" => vec!["hsb", "dsb"],
                "bm" => vec!["bg", "mk"],
                "yu" => vec!["sl", "sr", "hr", "bs"],
                other => crate::lang::population_atoms(other).to_vec(),
            };
            for code in expanded {
                if !out.contains(&code) {
                    out.push(code);
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// Branches represented by the expanded `same_in` language membership.
    /// Deriving this from [`same_in_langs`](Self::same_in_langs) keeps committee
    /// aliases and punctuation normalization identical across both views.
    pub fn native_branches(&self) -> Vec<Branch> {
        let langs = self.same_in_langs();
        [Branch::East, Branch::West, Branch::South]
            .into_iter()
            .filter(|branch| {
                langs
                    .iter()
                    .any(|code| crate::lang::branch_of(code) == Some(*branch))
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
struct IndexedCitation {
    sense_index: usize,
    spelling: String,
}

/// Exact-first index over individual official citation byforms. Standard
/// folding is lookup convenience, never lexical identity: a folded lookup is
/// accepted only when every indexed row under that fold has the same exact
/// official spelling.
pub struct OfficialSpellingIndex {
    exact: HashMap<String, Vec<IndexedCitation>>,
    folded: HashMap<String, Vec<IndexedCitation>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficialCitationMatch {
    pub sense_index: usize,
    pub spelling: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficialSpellingMatch {
    citations: Vec<OfficialCitationMatch>,
}

impl OfficialSpellingMatch {
    pub fn sense_indices(&self) -> Vec<usize> {
        let mut sense_indices: Vec<usize> = self
            .citations
            .iter()
            .map(|citation| citation.sense_index)
            .collect();
        sense_indices.sort_unstable();
        sense_indices.dedup();
        sense_indices
    }

    /// Canonical citation spelling belonging to the selected official sense.
    /// Keeping this association prevents a case-insensitive lookup bucket from
    /// lending another sense's casing (`lev` lion vs `Lev` Leo).
    pub fn spelling_for(&self, sense_index: usize) -> Option<&str> {
        self.citations
            .iter()
            .find(|citation| citation.sense_index == sense_index)
            .map(|citation| citation.spelling.as_str())
    }
}

impl OfficialSpellingIndex {
    pub fn new(entries: &[OfficialEntry]) -> Self {
        let mut exact: HashMap<String, Vec<IndexedCitation>> = HashMap::new();
        let mut folded: HashMap<String, Vec<IndexedCitation>> = HashMap::new();
        for (sense_index, entry) in entries.iter().enumerate() {
            for spelling in entry
                .citation_forms()
                .into_iter()
                .filter(|spelling| !spelling.contains(' '))
            {
                let lower = spelling.to_lowercase();
                let indexed = IndexedCitation {
                    sense_index,
                    spelling,
                };
                exact
                    .entry(lower.clone())
                    .or_default()
                    .push(indexed.clone());
                folded
                    .entry(crate::orthography::to_standard(&lower))
                    .or_default()
                    .push(indexed);
            }
        }
        Self { exact, folded }
    }

    pub fn lookup(&self, form: &str) -> Option<OfficialSpellingMatch> {
        let lower = form.trim().to_lowercase();
        let rows = if let Some(rows) = self.exact.get(&lower) {
            rows.as_slice()
        } else {
            let rows = self
                .folded
                .get(&crate::orthography::to_standard(&lower))?
                .as_slice();
            let mut spellings = rows.iter().map(|row| row.spelling.to_lowercase());
            let first = spellings.next()?;
            if spellings.any(|spelling| spelling != first) {
                return None;
            }
            rows
        };
        let mut citations: Vec<OfficialCitationMatch> = Vec::new();
        for row in rows {
            if !citations.iter().any(|citation| {
                citation.sense_index == row.sense_index && citation.spelling == row.spelling
            }) {
                citations.push(OfficialCitationMatch {
                    sense_index: row.sense_index,
                    spelling: row.spelling.clone(),
                });
            }
        }
        (!citations.is_empty()).then_some(OfficialSpellingMatch { citations })
    }

    /// Whether any official single-word citation occupies this folded spelling.
    /// Used only for exclusion/deduplication, where ambiguity still means the
    /// spelling is already official.
    pub fn contains_fold(&self, form: &str) -> bool {
        self.folded.contains_key(&crate::orthography::to_standard(
            &form.trim().to_lowercase(),
        ))
    }
}

/// Minimal RFC-4180 CSV reader: handles quoted fields with embedded commas,
/// quotes, and newlines. Returns records as vectors of fields.
pub fn read_csv_records(text: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut field = String::new();
    let mut record: Vec<String> = Vec::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(ch);
            }
        } else {
            match ch {
                '"' => in_quotes = true,
                ',' => {
                    record.push(std::mem::take(&mut field));
                }
                '\r' => {}
                '\n' => {
                    record.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut record));
                }
                _ => field.push(ch),
            }
        }
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

/// Detect the delimiter (comma for the full export, tab for the metadata TSV).
fn looks_like_tsv(header: &str) -> bool {
    header.contains('\t') && !header.starts_with("id,")
}

pub fn load(path: &Path) -> Result<Vec<OfficialEntry>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read official dictionary {}", path.display()))?;
    let first_line = text.lines().next().unwrap_or("");

    let records: Vec<Vec<String>> = if looks_like_tsv(first_line) {
        text.lines()
            .map(|l| l.split('\t').map(|s| s.to_string()).collect())
            .collect()
    } else {
        read_csv_records(&text)
    };

    let mut it = records.into_iter();
    let header = it.next().context("empty dictionary file")?;
    let col: HashMap<String, usize> = header
        .iter()
        .enumerate()
        .map(|(i, h)| (h.trim().to_lowercase(), i))
        .collect();

    let get = |rec: &[String], name: &str| -> String {
        col.get(name)
            .and_then(|&i| rec.get(i))
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };

    let lang_codes = [
        "ru", "be", "uk", "pl", "cs", "sk", "sl", "hr", "sr", "mk", "bg", "cu",
    ];
    let mut out = Vec::new();
    for rec in it {
        if rec.iter().all(|f| f.trim().is_empty()) {
            continue;
        }
        let isv = get(&rec, "isv");
        if isv.is_empty() {
            continue;
        }
        let pos_raw = if col.contains_key("partofspeech") {
            get(&rec, "partofspeech")
        } else {
            get(&rec, "pos")
        };
        let mut cells = HashMap::new();
        for code in lang_codes {
            let v = get(&rec, code);
            if !v.is_empty() {
                cells.insert(code.to_string(), v);
            }
        }
        let frequency = get(&rec, "frequency").parse::<f32>().ok();
        out.push(OfficialEntry {
            id: get(&rec, "id"),
            pos: Pos::parse(&pos_raw),
            noun_traits: parse_noun_traits(&pos_raw),
            addition: get(&rec, "addition"),
            english: get(&rec, "en"),
            same_in: get(&rec, "sameinlanguages"),
            genesis: get(&rec, "genesis"),
            de: get(&rec, "de"),
            nl: get(&rec, "nl"),
            eo: get(&rec, "eo"),
            intelligibility: get(&rec, "intelligibility"),
            using_example: get(&rec, "using_example"),
            cells,
            frequency,
            isv,
            pos_raw,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_with_same_in(same_in: &str) -> OfficialEntry {
        OfficialEntry {
            id: String::new(),
            isv: "slovo".to_string(),
            addition: String::new(),
            pos_raw: String::new(),
            pos: Pos::Noun,
            noun_traits: NounTraits::default(),
            english: String::new(),
            same_in: same_in.to_string(),
            genesis: String::new(),
            cells: HashMap::new(),
            frequency: None,
            de: String::new(),
            nl: String::new(),
            eo: String::new(),
            intelligibility: String::new(),
            using_example: String::new(),
        }
    }

    /// The same_in expansion (issue #79 review): branch markers cover the
    /// branch's modern CSV languages, the committee group codes expand to
    /// their members, punctuation/`~` are stripped, `sh` expands to its
    /// atoms, unknown tokens are skipped, and the result dedups.
    #[test]
    fn same_in_langs_expands_committee_tokens() {
        let langs = |s: &str| entry_with_same_in(s).same_in_langs();
        assert_eq!(langs("v"), vec!["be", "ru", "uk"]);
        assert_eq!(langs("cz~"), vec!["cs", "sk"]);
        assert_eq!(langs("bm"), vec!["bg", "mk"]);
        assert_eq!(langs("(sh)"), vec!["bs", "hr", "sr"]);
        assert_eq!(langs("yu"), vec!["bs", "hr", "sl", "sr"]);
        assert_eq!(langs("sb"), vec!["dsb", "hsb"]);
        // Unknown committee typos contribute nothing; dedup across tokens.
        assert_eq!(langs("ps"), Vec::<&str>::new());
        assert_eq!(langs("j yu mk"), vec!["bg", "bs", "hr", "mk", "sl", "sr"]);
        assert_eq!(langs(""), Vec::<&str>::new());
    }

    #[test]
    fn citation_byforms_are_indexed_individually_and_safely() {
        let mut byforms = entry_with_same_in("");
        byforms.isv = "iměti, imati".into();
        assert_eq!(byforms.citation_forms(), ["iměti", "imati"]);
        let mut adjective_byforms = entry_with_same_in("");
        adjective_byforms.isv = "poslědnji, poslědny".into();
        let index = OfficialSpellingIndex::new(&[byforms, adjective_byforms]);
        assert_eq!(
            index.lookup("imati").unwrap().spelling_for(0),
            Some("imati")
        );
        assert_eq!(index.lookup("iměti").unwrap().sense_indices(), [0]);
        assert_eq!(index.lookup("poslědnji").unwrap().sense_indices(), [1]);
        assert_eq!(
            index.lookup("poslědny").unwrap().spelling_for(1),
            Some("poslědny")
        );
        assert!(index.contains_fold("imati"));

        let mut governed = entry_with_same_in("");
        governed.isv = "pozirati (na)".into();
        assert_eq!(governed.citation_forms(), ["pozirati"]);

        let mut flagged = entry_with_same_in("");
        flagged.isv = "#izstava, izstavka".into();
        assert!(flagged.citation_forms().is_empty());
    }

    #[test]
    fn exact_lookup_keeps_canonical_spelling_bound_to_its_sense() {
        let mut common = entry_with_same_in("");
        common.isv = "lev".into();
        let mut proper = entry_with_same_in("");
        proper.isv = "Lev".into();
        let index = OfficialSpellingIndex::new(&[common, proper]);
        let matched = index.lookup("lev").unwrap();
        assert_eq!(matched.sense_indices(), [0, 1]);
        assert_eq!(matched.spelling_for(0), Some("lev"));
        assert_eq!(matched.spelling_for(1), Some("Lev"));
    }

    #[test]
    fn folded_spelling_lookup_abstains_on_distinct_official_spellings() {
        let mut first = entry_with_same_in("");
        first.isv = "dŕžati".into();
        let mut second = entry_with_same_in("");
        second.isv = "držati".into();
        let index = OfficialSpellingIndex::new(&[first, second]);
        assert!(index.lookup("drzati").is_none());
    }

    #[test]
    fn native_branches_share_the_normalized_language_expansion() {
        let branches = |s: &str| entry_with_same_in(s).native_branches();
        assert_eq!(branches("(sh)"), vec![Branch::South]);
        assert_eq!(
            branches("ru (cz sh)"),
            vec![Branch::East, Branch::West, Branch::South]
        );
        assert_eq!(branches("yu"), vec![Branch::South]);
        assert_eq!(branches("#ru~"), vec![Branch::East]);
        assert!(branches("ps").is_empty());
    }
}
