//! Candidate-generation orchestrator (production path).
//!
//! Wraps the shared [`crate::pipeline`] (consensus + Proto-Slavic-derived form)
//! and adds the site-only concern: the official-dictionary match status. The
//! **benchmark** never goes through here (it calls the leakage-free pipeline
//! directly) so the official lemma can never leak into a candidate; here, on
//! the site, the official form is allowed as a clearly-labeled extra.

use crate::consensus::{ConsensusConfig, MeaningInput};
use crate::dump::ProtoIndex;
use crate::model::{Candidate, MatchStatus, Reconstruction};
use crate::orthography as ortho;

pub struct Generation {
    /// Ranked generated candidates (algorithmic; excludes the official form).
    pub candidates: Vec<Candidate>,
    /// Optional official form, shown separately as "officially attested".
    pub official: Option<String>,
    pub match_status: MatchStatus,
    /// The linked Proto-Slavic reconstruction, if any.
    pub reconstruction: Option<Reconstruction>,
}

impl Generation {
    pub fn top(&self) -> Option<&Candidate> {
        self.candidates.first()
    }
}

fn official_status_and_display(
    candidates: &[Candidate],
    official_forms: &[&str],
) -> (MatchStatus, Option<String>) {
    let matched_official = candidates.first().and_then(|candidate| {
        official_forms
            .iter()
            .find(|official| ortho::normalized_match(&candidate.form, official))
            .copied()
    });
    let match_status = if official_forms.is_empty() {
        MatchStatus::NoOfficialEntry
    } else if matched_official.is_some() {
        MatchStatus::OfficialMatch
    } else {
        MatchStatus::DiffersFromOfficial
    };
    let display = matched_official
        .or_else(|| official_forms.first().copied())
        .map(str::to_string);
    (match_status, display)
}

/// Generate ranked candidates for one meaning.
///
/// * `official_isv` — the official lemma, used only for status/display.
/// * `proto` — the Proto-Slavic index for reconstruction-derived forms.
pub fn generate(
    input: &MeaningInput,
    official_isv: Option<&str>,
    proto: Option<&ProtoIndex>,
    cfg: &ConsensusConfig,
) -> Generation {
    match official_isv {
        Some(official_isv) => generate_with_official_byforms(input, [official_isv], proto, cfg),
        None => generate_with_official_byforms(input, std::iter::empty::<&str>(), proto, cfg),
    }
}

pub fn generate_with_official_byforms<'a>(
    input: &MeaningInput,
    official_byforms: impl IntoIterator<Item = &'a str>,
    proto: Option<&ProtoIndex>,
    cfg: &ConsensusConfig,
) -> Generation {
    let (candidates, reconstruction) = crate::pipeline::generate(input, proto, cfg);
    let official_forms: Vec<&str> = official_byforms
        .into_iter()
        .map(str::trim)
        .filter(|form| !form.is_empty())
        .collect();

    let (match_status, official_display) =
        official_status_and_display(&candidates, &official_forms);

    Generation {
        candidates,
        official: official_display,
        match_status,
        reconstruction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CandidateSource, Pos};

    #[test]
    fn official_byform_display_prefers_matched_top_candidate() {
        let candidates = vec![Candidate::new(
            "imati".to_string(),
            CandidateSource::BranchConsensus,
            0.9,
        )];
        let (status, display) = official_status_and_display(&candidates, &["iměti", "imati"]);
        assert_eq!(status, MatchStatus::OfficialMatch);
        assert_eq!(display.as_deref(), Some("imati"));
    }

    /// The retired manual-override list (data/overrides.toml, removed) as test
    /// fixtures: the pipeline must derive each adapted internationalism from
    /// the Slavic evidence alone, with no curated lookup. Evidence cells are
    /// copied verbatim from the official CSV rows.
    fn loanword_top(gloss: &str, intl: bool, cells: &[(&str, &str)]) -> String {
        let cells: std::collections::HashMap<String, String> = cells
            .iter()
            .map(|(l, f)| (l.to_string(), f.to_string()))
            .collect();
        let forms = crate::consensus::source_forms_from_cells(&cells, |_, _| String::new());
        let forms = crate::consensus::lemma_forms(forms, Pos::Noun);
        let input = MeaningInput {
            pos: Pos::Noun,
            gender: None,
            gloss: gloss.to_string(),
            forms,
            is_intl_meaning: intl,
            reflexive: false,
        };
        let (candidates, _) =
            crate::pipeline::generate(&input, None, &ConsensusConfig::production());
        candidates
            .first()
            .map(|c| c.form.clone())
            .unwrap_or_default()
    }

    #[test]
    fn derives_kompjuter_from_evidence() {
        let top = loanword_top(
            "computer",
            true,
            &[
                ("ru", "компьютер"),
                ("be", "камп'ютар, кампутар"),
                ("uk", "комп'ютер"),
                ("pl", "komputer"),
                ("cs", "počítač, komputer"),
                ("sk", "počítač, komputer"),
                ("sl", "računalnik"),
                ("hr", "kompjutor, kompjuter, računalo"),
                ("sr", "рачунар, компјутер"),
                ("mk", "компјутер"),
                ("bg", "компютър"),
            ],
        );
        assert_eq!(top, "kompjuter");
    }

    #[test]
    fn derives_futbol_from_evidence() {
        let top = loanword_top(
            "football, soccer",
            false,
            &[
                ("ru", "футбол"),
                ("be", "футбол"),
                ("uk", "футбол"),
                ("pl", "piłka nożna"),
                ("cs", "fotbal"),
                ("sk", "futbal"),
                ("sl", "nogomet"),
                ("hr", "nogomet"),
                ("sr", "фудбал"),
                ("mk", "фудбал"),
                ("bg", "футбол"),
            ],
        );
        assert_eq!(top, "futbol");
    }

    #[test]
    fn derives_dzaz_from_evidence() {
        // Loan [dʒ] must come out as dž (never etymological đ) purely from the
        // cross-branch evidence vote.
        let top = loanword_top(
            "jazz",
            false,
            &[
                ("ru", "джаз"),
                ("be", "джаз"),
                ("uk", "джаз"),
                ("pl", "jazz"),
                ("cs", "jazz"),
                ("sk", "džez"),
                ("sl", "jazz"),
                ("hr", "jazz, džez"),
                ("sr", "џез"),
                ("mk", "џез"),
                ("bg", "джаз"),
            ],
        );
        assert_eq!(top, "džaz");
    }
}
