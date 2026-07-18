//! Shared semantic matching between generated corpus candidates and official senses.
//!
//! Spelling only identifies candidate rows. A match additionally requires the
//! same part of speech and positive gloss evidence, preventing homographs from
//! becoming labels or published official matches by spelling alone.

use crate::model::{Candidate, Pos};
use crate::official::OfficialEntry;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficialMatch {
    /// One-based rank in the generated publication candidate list.
    pub candidate_rank: usize,
    /// Index in the loaded official-entry vector.
    pub sense_index: usize,
    /// Stable official dictionary sense identifier.
    pub sense_id: String,
    /// The individual official citation spelling that matched. Dictionary rows
    /// may list comma-separated byforms (for example `iměti, imati`).
    pub spelling: String,
}

#[derive(Debug, Clone)]
struct IndexedSpelling {
    sense_index: usize,
    spelling: String,
}

pub struct OfficialIndex {
    exact: HashMap<String, Vec<IndexedSpelling>>,
    folded: HashMap<String, Vec<IndexedSpelling>>,
}

impl OfficialIndex {
    pub fn new(entries: &[OfficialEntry]) -> Self {
        let mut exact: HashMap<String, Vec<IndexedSpelling>> = HashMap::new();
        let mut folded: HashMap<String, Vec<IndexedSpelling>> = HashMap::new();
        for (sense_index, entry) in entries.iter().enumerate() {
            // About 230 dictionary rows list byform variants in one CSV cell.
            // They are separate citation spellings everywhere else in the API,
            // so index each one independently rather than mistaking the space
            // after the comma for a multi-word lemma.
            for spelling in entry.isv.split(',').map(str::trim) {
                if spelling.is_empty() || spelling.contains(' ') || spelling.contains('#') {
                    continue;
                }
                let lower = spelling.to_lowercase();
                let indexed = IndexedSpelling {
                    sense_index,
                    spelling: spelling.to_string(),
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

    /// Resolve the first publication candidate with lexical and semantic
    /// evidence. Exact scientific spelling is preferred. A folded lookup is
    /// rejected when it aliases distinct official spellings.
    pub fn match_candidates(
        &self,
        candidates: &[Candidate],
        entries: &[OfficialEntry],
        pos: Pos,
        gloss: &str,
    ) -> Option<OfficialMatch> {
        candidates.iter().take(5).enumerate().find_map(|(rank, c)| {
            self.match_form_with_spelling(&c.form, entries, pos, gloss)
                .map(|matched| OfficialMatch {
                    candidate_rank: rank + 1,
                    sense_id: entries[matched.sense_index].id.clone(),
                    sense_index: matched.sense_index,
                    spelling: matched.spelling,
                })
        })
    }

    pub fn match_form(
        &self,
        form: &str,
        entries: &[OfficialEntry],
        pos: Pos,
        gloss: &str,
    ) -> Option<usize> {
        self.match_form_with_spelling(form, entries, pos, gloss)
            .map(|matched| matched.sense_index)
    }

    fn match_form_with_spelling(
        &self,
        form: &str,
        entries: &[OfficialEntry],
        pos: Pos,
        gloss: &str,
    ) -> Option<IndexedSpelling> {
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
        let mut senses: Vec<usize> = rows.iter().map(|row| row.sense_index).collect();
        senses.sort_unstable();
        senses.dedup();
        let sense_index = select_official_entry(&senses, entries, pos, gloss)?;
        rows.iter()
            .find(|row| row.sense_index == sense_index)
            .cloned()
    }

    pub fn contains_fold(&self, form: &str) -> bool {
        self.folded.contains_key(&crate::orthography::to_standard(
            &form.trim().to_lowercase(),
        ))
    }
}

/// Normalized comma/semicolon/slash-delimited gloss alternatives. A leading
/// English infinitive marker is removed (`to have` → `have`), while an internal
/// `to` is retained (`have to` stays distinct), so modal and lexical senses do
/// not collapse merely because the content-token matcher drops stopwords.
fn gloss_alternatives(gloss: &str) -> Vec<String> {
    let mut alternatives: Vec<String> = gloss
        .split([',', ';', '/'])
        .filter_map(|part| {
            let mut words: Vec<String> = part
                .to_lowercase()
                .split(|c: char| !c.is_alphabetic())
                .filter(|word| !word.is_empty())
                .map(str::to_string)
                .collect();
            if words.first().is_some_and(|word| word == "to") {
                words.remove(0);
            }
            (!words.is_empty()).then(|| words.join(" "))
        })
        .collect();
    alternatives.sort();
    alternatives.dedup();
    alternatives
}

/// Choose the uniquely strongest compatible sense from an already
/// spelling-filtered row set. Exact/folded spelling without semantic evidence
/// is never positive, and an unresolved semantic tie abstains instead of making
/// dictionary row order part of lexical identity.
pub fn select_official_entry(
    rows: &[usize],
    entries: &[OfficialEntry],
    pos: Pos,
    gloss: &str,
) -> Option<usize> {
    let wanted = crate::dump::gloss_tokens(gloss);
    let compact = wanted.join("");
    let wanted_alternatives = gloss_alternatives(gloss);
    let scored: Vec<(usize, (bool, usize, bool))> = rows
        .iter()
        .copied()
        .filter(|&i| entries[i].pos == pos)
        .map(|i| {
            let actual = crate::dump::gloss_tokens(&entries[i].english);
            let overlap = wanted.iter().filter(|token| actual.contains(token)).count();
            let compound = !compact.is_empty() && compact == actual.join("");
            let actual_alternatives = gloss_alternatives(&entries[i].english);
            let exact_alternative = wanted_alternatives
                .iter()
                .any(|alternative| actual_alternatives.contains(alternative));
            (i, (exact_alternative, overlap, compound))
        })
        .filter(|(_, (exact_alternative, overlap, compound))| {
            *exact_alternative || *overlap > 0 || *compound
        })
        .collect();
    let best_score = scored.iter().map(|(_, score)| *score).max()?;
    let mut best = scored
        .iter()
        .filter(|(_, score)| *score == best_score)
        .map(|(i, _)| *i);
    let winner = best.next()?;
    best.next().is_none().then_some(winner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NounTraits, Pos};
    use std::collections::HashMap;

    fn entry(id: &str, isv: &str, pos: Pos, english: &str) -> OfficialEntry {
        OfficialEntry {
            id: id.into(),
            isv: isv.into(),
            addition: String::new(),
            pos_raw: pos.code().into(),
            pos,
            noun_traits: NounTraits::default(),
            english: english.into(),
            same_in: String::new(),
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

    #[test]
    fn spelling_requires_pos_and_semantics() {
        let entries = vec![
            entry("1", "bajka", Pos::Noun, "fairytale"),
            entry("2", "bajka", Pos::Verb, "speak"),
        ];
        let index = OfficialIndex::new(&entries);
        assert_eq!(
            index.match_form("bajka", &entries, Pos::Noun, "fairy tale"),
            Some(0)
        );
        assert_eq!(
            index.match_form("bajka", &entries, Pos::Adjective, "fairy tale"),
            None
        );
        assert_eq!(
            index.match_form("bajka", &entries, Pos::Noun, "machine"),
            None
        );
    }

    #[test]
    fn comma_separated_byforms_are_individual_official_spellings() {
        let entries = vec![entry("1", "iměti, imati", Pos::Verb, "have, possess, own")];
        let index = OfficialIndex::new(&entries);
        assert_eq!(
            index.match_form("imati", &entries, Pos::Verb, "to have"),
            Some(0)
        );
        assert_eq!(
            index.match_form("iměti", &entries, Pos::Verb, "to have"),
            Some(0)
        );
        assert!(index.contains_fold("imati"));
        let candidate = Candidate::new(
            "imati".into(),
            crate::model::CandidateSource::BranchConsensus,
            0.9,
        );
        let matched = index
            .match_candidates(&[candidate], &entries, Pos::Verb, "to have")
            .unwrap();
        assert_eq!(matched.spelling, "imati");
    }

    #[test]
    fn semantic_ties_abstain_and_infinitive_direction_disambiguates() {
        let entries = vec![
            entry("417", "iměti, imati", Pos::Verb, "must, have to"),
            entry("875", "iměti, imati", Pos::Verb, "have, possess, own"),
        ];
        let index = OfficialIndex::new(&entries);
        assert_eq!(
            index.match_form("imati", &entries, Pos::Verb, "to have"),
            Some(1),
            "lexical `to have` must not resolve to modal `have to`"
        );

        let tied = vec![
            entry("1", "banka", Pos::Noun, "bank"),
            entry("2", "banka", Pos::Noun, "bank"),
        ];
        assert_eq!(
            OfficialIndex::new(&tied).match_form("banka", &tied, Pos::Noun, "bank"),
            None,
            "equal semantic evidence must not be broken by CSV order"
        );
    }

    #[test]
    fn ambiguous_fold_is_rejected() {
        let entries = vec![
            entry("1", "dŕžati", Pos::Verb, "hold"),
            entry("2", "držati", Pos::Verb, "hold"),
        ];
        let index = OfficialIndex::new(&entries);
        assert_eq!(
            index.match_form("drzati", &entries, Pos::Verb, "hold"),
            None
        );
        assert_eq!(
            index.match_form("dŕžati", &entries, Pos::Verb, "unrelated"),
            None
        );
    }
}
