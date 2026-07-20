//! Core data model for the candidate-generation pipeline.
//!
//! A `Candidate` is one proposed Interslavic lemma together with a full audit
//! trail: which source strategy produced it, an ordered rule trace, the Slavic
//! evidence it rests on, a numeric score, and a calibrated confidence bucket.
//! The generator emits several candidates per meaning and ranks them.

use crate::lang::Branch;
use serde::{Deserialize, Serialize};

/// Where a candidate came from. Ordered loosely by intrinsic trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateSource {
    /// The official Interslavic dictionary already lists this form (only used
    /// on the production site, never in the leakage-free benchmark path).
    OfficialDictionary,
    /// Derived from a Proto-Slavic reconstruction via the rule engine.
    ProtoSlavicRule,
    /// Modern-Slavic consensus that is explicitly balanced across branches.
    BranchConsensus,
    /// Plain majority of modern Slavic forms (no branch balancing).
    MajorityModernSlavic,
    /// International/borrowed vocabulary adapted to Interslavic spelling.
    BorrowingInternationalism,
}

impl CandidateSource {
    pub fn label(self) -> &'static str {
        match self {
            CandidateSource::OfficialDictionary => "oficialny slovnik",
            CandidateSource::ProtoSlavicRule => "praslovjansko pravilo",
            CandidateSource::BranchConsensus => "medžuvětvovy konsensus",
            CandidateSource::MajorityModernSlavic => "vęčšina sovrěmennyh",
            CandidateSource::BorrowingInternationalism => "internacionalizm",
        }
    }
}

/// Calibrated reliability bucket. The numeric thresholds are tuned against the
/// benchmark so that "High" candidates really do match the official dictionary
/// more often than "Low" ones (confidence calibration).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl Confidence {
    pub fn from_score(score: f32) -> Self {
        if score >= 0.72 {
            Confidence::High
        } else if score >= 0.45 {
            Confidence::Medium
        } else {
            Confidence::Low
        }
    }

    /// Calibrated-probability bucket for DISPLAY surfaces (issue #77); measured
    /// operating points in methodology.md — propose p>=0.6: 71.6%/66.3% P/R,
    /// review p>=0.3: 61.7%/88.9% (holdout). [`Confidence::from_score`] stays
    /// the internal/eval raw-score bucket.
    pub fn from_probability(p: f64) -> Self {
        if p >= crate::calibrate::PROPOSE_T {
            Confidence::High
        } else if p >= crate::calibrate::REVIEW_T {
            Confidence::Medium
        } else {
            Confidence::Low
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Confidence::High => "vysoka",
            Confidence::Medium => "srědnja",
            Confidence::Low => "nizka",
        }
    }
}

/// How a piece of evidence relates to the candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceRelation {
    ProtoSlavicAncestor,
    BaltoSlavicAncestor,
    IndoEuropeanAncestor,
    Cognate,
    Descendant,
    OfficialTranslation,
}

/// One Slavic (or proto) form supporting a candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub lang_code: String,
    pub lang_name: String,
    pub branch: Option<Branch>,
    /// The form exactly as attested (original script).
    pub form: String,
    /// Common-Latin normalization used for alignment.
    pub normalized_form: String,
    pub relation: EvidenceRelation,
    pub source_url: String,
}

/// A single reversible transformation applied while building a candidate. The
/// trace is the scientific audit trail shown on entry pages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleStep {
    pub id: String,
    pub before: String,
    pub after: String,
    pub explanation: String,
    pub reference: Option<String>,
}

impl RuleStep {
    pub fn new(
        id: &str,
        before: impl Into<String>,
        after: impl Into<String>,
        explanation: impl Into<String>,
        reference: Option<&str>,
    ) -> Self {
        RuleStep {
            id: id.to_string(),
            before: before.into(),
            after: after.into(),
            explanation: explanation.into(),
            reference: reference.map(|s| s.to_string()),
        }
    }
}

/// A ranked Interslavic candidate lemma with its full provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub form: String,
    pub source: CandidateSource,
    pub score: f32,
    pub confidence: Confidence,
    /// How many Slavic branches (0-3) attest the form this candidate rests on.
    pub branch_coverage: u8,
    /// Language codes of the cognate cluster supporting THIS candidate (the
    /// razumlivost display score, issue #79). Empty = membership unknown.
    pub langs: Vec<String>,
    pub trace: Vec<RuleStep>,
    pub evidence: Vec<Evidence>,
    pub warnings: Vec<String>,
}

impl Candidate {
    pub fn new(form: String, source: CandidateSource, score: f32) -> Self {
        Candidate {
            confidence: Confidence::from_score(score),
            form,
            source,
            score,
            branch_coverage: 0,
            langs: Vec::new(),
            trace: Vec::new(),
            evidence: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

/// A Proto-Slavic reconstruction linked to a meaning, shown as the etymological
/// source on entry pages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reconstruction {
    pub word: String,
    pub proto_balto_slavic: String,
    pub proto_indo_european: String,
    /// Link confidence in `[0, 1]` from the leakage-free linker.
    pub confidence: f32,
}

/// Whether a generated candidate agrees with the official Interslavic entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchStatus {
    /// Top candidate equals the official lemma (exact or normalized).
    OfficialMatch,
    /// An official entry exists but the top candidate differs from it.
    DiffersFromOfficial,
    /// No official entry was found for this meaning.
    NoOfficialEntry,
}

impl MatchStatus {
    pub fn label(self) -> &'static str {
        match self {
            MatchStatus::OfficialMatch => "oficialno potvŕđeno",
            MatchStatus::DiffersFromOfficial => "razlikuje se od oficialnogo",
            MatchStatus::NoOfficialEntry => "nema oficialnogo zapisa",
        }
    }
}

/// Normalized part of speech spanning both the official dictionary's tags
/// (`m.`, `v.tr. ipf.`, `adj.`) and Wiktextract's (`noun`, `verb`, `adj`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pos {
    Noun,
    ProperNoun,
    Verb,
    Adjective,
    Adverb,
    Numeral,
    Pronoun,
    Preposition,
    Conjunction,
    Interjection,
    Particle,
    Prefix,
    Suffix,
    Phrase,
    Other,
}

impl Pos {
    pub fn code(self) -> &'static str {
        match self {
            Pos::Noun => "noun",
            Pos::ProperNoun => "proper_noun",
            Pos::Verb => "verb",
            Pos::Adjective => "adj",
            Pos::Adverb => "adv",
            Pos::Numeral => "num",
            Pos::Pronoun => "pron",
            Pos::Preposition => "prep",
            Pos::Conjunction => "conj",
            Pos::Interjection => "intj",
            Pos::Particle => "particle",
            Pos::Prefix => "prefix",
            Pos::Suffix => "suffix",
            Pos::Phrase => "phrase",
            Pos::Other => "other",
        }
    }

    /// Parse both official ("m.", "v.tr. ipf.", "adj.", "adv.") and Wiktextract
    /// ("noun", "verb", "adj", "proper noun") part-of-speech strings.
    pub fn parse(raw: &str) -> Pos {
        let s = raw.trim().to_lowercase();
        if s.is_empty() {
            return Pos::Other;
        }
        // Wiktextract style first.
        match s.as_str() {
            "noun" => return Pos::Noun,
            "proper noun" | "proper_noun" | "name" => return Pos::ProperNoun,
            "verb" => return Pos::Verb,
            "adj" | "adjective" => return Pos::Adjective,
            "adv" | "adverb" => return Pos::Adverb,
            "num" | "numeral" | "number" => return Pos::Numeral,
            "pron" | "pronoun" => return Pos::Pronoun,
            "prep" | "preposition" | "postp" => return Pos::Preposition,
            "conj" | "conjunction" => return Pos::Conjunction,
            "intj" | "interjection" => return Pos::Interjection,
            "particle" | "prtcl" => return Pos::Particle,
            "prefix" => return Pos::Prefix,
            "suffix" | "affix" => return Pos::Suffix,
            "phrase" | "proverb" | "idiom" => return Pos::Phrase,
            _ => {}
        }
        // Official dictionary style (leading abbreviation).
        if s.starts_with("v.") || s.starts_with("v ") || s == "v" {
            return Pos::Verb;
        }
        if s.starts_with("adj") {
            return Pos::Adjective;
        }
        if s.starts_with("adv") {
            return Pos::Adverb;
        }
        if s.starts_with("num") {
            return Pos::Numeral;
        }
        if s.starts_with("pron") {
            return Pos::Pronoun;
        }
        if s.starts_with("prep") || s.starts_with("postp") {
            return Pos::Preposition;
        }
        if s.starts_with("conj") {
            return Pos::Conjunction;
        }
        if s.starts_with("intj") {
            return Pos::Interjection;
        }
        if s.starts_with("prefix") {
            return Pos::Prefix;
        }
        if s.starts_with("suffix") {
            return Pos::Suffix;
        }
        if s.starts_with("phrase") {
            return Pos::Phrase;
        }
        // Bare gender markers -> noun. `m.`, `f.`, `n.`, `m.anim.`, `f.sg.` ...
        if s.starts_with("m.")
            || s.starts_with("f.")
            || s.starts_with("n.")
            || s == "m"
            || s == "f"
            || s == "n"
            || s.starts_with("m/")
            || s.starts_with("m.")
        {
            return Pos::Noun;
        }
        Pos::Other
    }

    pub fn heading_isv(self) -> &'static str {
        match self {
            Pos::Noun => "Imennik",
            Pos::ProperNoun => "Vlastno imę",
            Pos::Verb => "Glagol",
            Pos::Adjective => "Pridavnik",
            Pos::Adverb => "Prislovnik",
            Pos::Numeral => "Čislovnik",
            Pos::Pronoun => "Městoimę",
            Pos::Preposition => "Predlog",
            Pos::Conjunction => "Sȯjuz",
            Pos::Interjection => "Medžuslovje",
            Pos::Particle => "Čestica",
            Pos::Prefix => "Prefiks",
            Pos::Suffix => "Sufiks",
            Pos::Phrase => "Frazema",
            Pos::Other => "Slovo",
        }
    }
}

/// Grammatical gender parsed from the official dictionary POS tag, used to pick
/// the right noun ending/declension class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Gender {
    Masculine,
    Feminine,
    Neuter,
    Unknown,
}

/// Extra nominal metadata parsed from the official POS tag (`m.anim.`, `f.pl.`).
#[derive(Debug, Clone, Copy, Default)]
pub struct NounTraits {
    pub gender: Option<Gender>,
    pub animate: bool,
    pub plural_only: bool,
    pub singular_only: bool,
    pub indeclinable: bool,
}

pub fn parse_noun_traits(raw: &str) -> NounTraits {
    let s = raw.to_lowercase();
    let mut t = NounTraits::default();
    if s.starts_with("m.") || s == "m" || s.starts_with("m/") || s.starts_with("m ") {
        t.gender = Some(Gender::Masculine);
    } else if s.starts_with("f.") || s == "f" {
        t.gender = Some(Gender::Feminine);
    } else if s.starts_with("n.") || s == "n" {
        t.gender = Some(Gender::Neuter);
    }
    t.animate = s.contains("anim");
    t.plural_only = s.contains(".pl") || s.contains("pl.");
    t.singular_only = s.contains(".sg") || s.contains("sg.");
    t.indeclinable = s.contains("indecl");
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The display bucket cuts exactly at the persisted operating points
    /// (issue #77) — tested against the real consts so a threshold change
    /// re-tests the boundaries.
    #[test]
    fn from_probability_buckets_at_the_operating_points() {
        assert_eq!(
            Confidence::from_probability(crate::calibrate::PROPOSE_T),
            Confidence::High
        );
        assert_eq!(
            Confidence::from_probability(crate::calibrate::PROPOSE_T - 1e-9),
            Confidence::Medium
        );
        assert_eq!(
            Confidence::from_probability(crate::calibrate::REVIEW_T),
            Confidence::Medium
        );
        assert_eq!(
            Confidence::from_probability(crate::calibrate::REVIEW_T - 1e-9),
            Confidence::Low
        );
    }
}
