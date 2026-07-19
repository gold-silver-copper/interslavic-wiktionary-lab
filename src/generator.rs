//! Candidate-generation orchestrator (production path).
//!
//! Wraps the shared [`crate::pipeline`] (consensus + Proto-Slavic-derived form)
//! and adds the site-only concerns: manual overrides and the official-dictionary
//! match status. The **benchmark** never goes through here (it calls the
//! leakage-free pipeline directly) so the official lemma can never leak into a
//! candidate; here, on the site, the official form and overrides are allowed as
//! clearly-labeled extras.

use crate::consensus::{ConsensusConfig, MeaningInput};
use crate::dump::ProtoIndex;
use crate::model::{Candidate, CandidateSource, Confidence, MatchStatus, Reconstruction};
use crate::orthography as ortho;
use crate::overrides::Overrides;

pub struct Generation {
    /// Ranked generated candidates (algorithmic; excludes the official form).
    pub candidates: Vec<Candidate>,
    /// Optional official form, shown separately as "officially attested".
    pub official: Option<String>,
    pub match_status: MatchStatus,
    /// True when a manual override supplied the top form.
    pub overridden: bool,
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
    overrides: &Overrides,
) -> Generation {
    match official_isv {
        Some(official_isv) => {
            generate_with_official_byforms(input, [official_isv], proto, cfg, overrides)
        }
        None => {
            generate_with_official_byforms(input, std::iter::empty::<&str>(), proto, cfg, overrides)
        }
    }
}

pub fn generate_with_official_byforms<'a>(
    input: &MeaningInput,
    official_byforms: impl IntoIterator<Item = &'a str>,
    proto: Option<&ProtoIndex>,
    cfg: &ConsensusConfig,
    overrides: &Overrides,
) -> Generation {
    let (mut candidates, reconstruction) = crate::pipeline::generate(input, proto, cfg);
    let official_forms: Vec<&str> = official_byforms
        .into_iter()
        .map(str::trim)
        .filter(|form| !form.is_empty())
        .collect();

    // Status is computed from the *generated* top candidate vs the official form,
    // before overrides are applied, so overrides never inflate accuracy.
    let (match_status, official_display) =
        official_status_and_display(&candidates, &official_forms);

    // Manual override (site-only; excluded from pure-algorithm accuracy).
    let mut overridden = false;
    if let Some(o) = overrides.lookup(&input.gloss) {
        let mut c = Candidate::new(o.official.clone(), CandidateSource::ManualOverride, 0.99);
        c.confidence = Confidence::High;
        // The override rests on the meaning's whole evidence row (issue #79).
        c.langs = {
            let mut l: Vec<String> = input.forms.iter().map(|f| f.lang_code.clone()).collect();
            l.sort();
            l.dedup();
            l
        };
        c.warnings.push(format!("Ručna korektura: {}", o.reason));
        candidates.insert(0, c);
        overridden = true;
    }

    Generation {
        candidates,
        official: official_display,
        match_status,
        overridden,
        reconstruction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
