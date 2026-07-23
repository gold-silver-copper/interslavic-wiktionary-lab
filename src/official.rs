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

#[derive(Debug, Clone)]
pub struct OfficialByform<'a> {
    pub entry: &'a OfficialEntry,
    pub form: String,
}

impl OfficialEntry {
    /// Sanitized citation spellings represented by this official row. The raw
    /// dictionary uses top-level commas for byforms (`iměti, imati`), while
    /// multiword lemmas and parenthetical government hints are part of a single
    /// citation surface.
    pub fn citation_byforms(&self) -> Vec<OfficialByform<'_>> {
        citation_forms(&self.isv)
            .into_iter()
            .map(|form| OfficialByform { entry: self, form })
            .collect()
    }

    pub fn primary_citation_byform(&self) -> Option<String> {
        citation_forms(&self.isv).into_iter().next()
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

pub fn citation_forms(raw: &str) -> Vec<String> {
    let mut forms = Vec::new();
    for part in split_citation_variants(raw) {
        let part = part.trim();
        if part.is_empty() || part.contains('#') || part.contains('!') {
            continue;
        }
        let Some(clean) = crate::forms::citation(part) else {
            continue;
        };
        let clean = clean.trim();
        if clean.is_empty() || clean.contains('#') || clean.contains('!') {
            continue;
        }
        if !forms.iter().any(|seen| seen == clean) {
            forms.push(clean.to_string());
        }
    }
    forms
}

fn split_citation_variants(raw: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    for (i, ch) in raw.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&raw[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&raw[start..]);
    out
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

pub fn load(path: &Path) -> Result<Vec<OfficialEntry>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read official dictionary {}", path.display()))?;

    // The undocumented TSV branch is gone (V15 item 1): it had no in-repo
    // caller and used a quote-blind splitter beside the real RFC-4180 parser.
    let records: Vec<Vec<String>> = read_csv_records(&text);

    let mut it = records.into_iter();
    let header = it.next().context("empty dictionary file")?;
    let col: HashMap<String, usize> = header
        .iter()
        .enumerate()
        .map(|(i, h)| (h.trim().to_lowercase(), i))
        .collect();
    // Loud failure on a wrong-delimiter file (V15.1 item 3): a TSV or
    // semicolon export parses as a one-column CSV whose header has no
    // `isv` key, and every row then silently yielded an empty lemma —
    // check-text classified everything "unknown" with no visible cause.
    anyhow::ensure!(
        col.contains_key("isv"),
        "{}: no `isv` column in the header — the loader reads comma-separated CSV only \
         (got {} header column(s); first cell {:?})",
        path.display(),
        header.len(),
        header.first().map_or("", String::as_str)
    );

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

    fn entry_with_isv(isv: &str) -> OfficialEntry {
        let mut entry = entry_with_same_in("");
        entry.isv = isv.to_string();
        entry
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

    #[test]
    fn citation_byforms_split_top_level_commas_only() {
        let forms = |isv: &str| {
            entry_with_isv(isv)
                .citation_byforms()
                .into_iter()
                .map(|byform| byform.form)
                .collect::<Vec<_>>()
        };
        assert_eq!(forms("iměti, imati"), vec!["iměti", "imati"]);
        assert_eq!(forms("poslědnji, poslědny"), vec!["poslědnji", "poslědny"]);
        assert_eq!(forms("kak, kako"), vec!["kak", "kako"]);
        assert_eq!(forms("pęt na desęte"), vec!["pęt na desęte"]);
        assert_eq!(forms("pozirati (na)"), vec!["pozirati"]);
        assert_eq!(forms("dobry, #dobrějši, !dobrěje, *dobro"), vec!["dobry"]);
    }

    /// V15.1 item 3: a tab-separated export must fail loudly, not parse as
    /// a one-column CSV that silently classifies every token unknown.
    #[test]
    fn tsv_input_is_rejected_with_a_diagnosis() {
        let dir = std::env::temp_dir().join(format!("slovowiki-tsv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("official.tsv");
        std::fs::write(&path, "id\tisv\tpartOfSpeech\ten\n1\tslovo\tn.\tword\n").unwrap();
        let err = load(&path).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("no `isv` column") && msg.contains("comma-separated"),
            "{msg}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
