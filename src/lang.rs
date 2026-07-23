//! Slavic language metadata: branch membership, script, and human-readable
//! names. Kept deliberately small and dependency-free.

/// The three primary Slavic branches. Balancing evidence across these three is
/// central to how Interslavic vocabulary is selected, so the generator treats
/// the branch as a first-class dimension rather than a per-language count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Branch {
    East,
    West,
    South,
}

impl Branch {
    pub fn code(self) -> &'static str {
        match self {
            Branch::East => "east",
            Branch::West => "west",
            Branch::South => "south",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Branch::East => "vȯzhodnoslovjansky",
            Branch::West => "zapadnoslovjansky",
            Branch::South => "južnoslovjansky",
        }
    }

    pub const ALL: [Branch; 3] = [Branch::East, Branch::West, Branch::South];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Cyrillic,
    Latin,
}

/// Static description of a Slavic language relevant to candidate generation.
#[derive(Debug, Clone, Copy)]
pub struct LangInfo {
    pub code: &'static str,
    pub name: &'static str,
    pub branch: Branch,
    pub script: Script,
    /// Whether this is a modern living standard language. Old Church Slavonic
    /// and the reconstructed/old languages are excluded from the modern
    /// consensus vote but may still be used as etymological hints.
    pub modern: bool,
    /// Column name used in the official dictionary CSV (empty if not present).
    pub csv_col: &'static str,
    /// Speaker population in millions, for the razumlivost display score
    /// (issue #79). Sources: data/VOTING_MACHINE_NOTES.md (van Steenbergen's
    /// voting machine: RU 143.6, PL ~44, UA 37, BY 8.6, CZ 10) plus
    /// conservative estimates for the languages it doesn't list; the small
    /// lects (csb/dsb/hsb/rue/szl) use rounded census figures. `sh` is the
    /// sr+hr+bs macro-code (their sum) and must NEVER enter a population
    /// denominator; non-modern hint languages (cu, orv) carry 0. Distinct
    /// from [`pop_weight`], the hand-tuned consensus tie-break.
    pub speakers_m: f32,
}

/// The modern Slavic languages we treat as first-class consensus voters, plus
/// the archaic/relevant ones used only as etymological hints.
pub const LANGS: &[LangInfo] = &[
    // East
    LangInfo {
        code: "ru",
        name: "rusky",
        branch: Branch::East,
        script: Script::Cyrillic,
        modern: true,
        csv_col: "ru",
        speakers_m: 143.6,
    },
    LangInfo {
        code: "be",
        name: "bělorussky",
        branch: Branch::East,
        script: Script::Cyrillic,
        modern: true,
        csv_col: "be",
        speakers_m: 8.6,
    },
    LangInfo {
        code: "uk",
        name: "ukrajinsky",
        branch: Branch::East,
        script: Script::Cyrillic,
        modern: true,
        csv_col: "uk",
        speakers_m: 37.0,
    },
    // West
    LangInfo {
        code: "pl",
        name: "poljsky",
        branch: Branch::West,
        script: Script::Latin,
        modern: true,
        csv_col: "pl",
        speakers_m: 44.0,
    },
    LangInfo {
        code: "cs",
        name: "čehsky",
        branch: Branch::West,
        script: Script::Latin,
        modern: true,
        csv_col: "cs",
        speakers_m: 10.0,
    },
    LangInfo {
        code: "sk",
        name: "slovacky",
        branch: Branch::West,
        script: Script::Latin,
        modern: true,
        csv_col: "sk",
        speakers_m: 5.2,
    },
    // South
    LangInfo {
        code: "sl",
        name: "slovensky",
        branch: Branch::South,
        script: Script::Latin,
        modern: true,
        csv_col: "sl",
        speakers_m: 2.1,
    },
    LangInfo {
        code: "hr",
        name: "horvatsky",
        branch: Branch::South,
        script: Script::Latin,
        modern: true,
        csv_col: "hr",
        speakers_m: 5.5,
    },
    LangInfo {
        code: "sr",
        name: "srbsky",
        branch: Branch::South,
        script: Script::Cyrillic,
        modern: true,
        csv_col: "sr",
        speakers_m: 8.7,
    },
    LangInfo {
        code: "mk",
        name: "makedonsky",
        branch: Branch::South,
        script: Script::Cyrillic,
        modern: true,
        csv_col: "mk",
        speakers_m: 2.0,
    },
    LangInfo {
        code: "bg",
        name: "bȯlgarsky",
        branch: Branch::South,
        script: Script::Cyrillic,
        modern: true,
        csv_col: "bg",
        speakers_m: 8.0,
    },
    // Archaic South Slavic: only an etymological hint, not a modern voter.
    LangInfo {
        code: "cu",
        name: "starocŕkovnoslovjansky",
        branch: Branch::South,
        script: Script::Cyrillic,
        modern: false,
        csv_col: "cu",
        speakers_m: 0.0,
    },
    // Additional Wiktionary descendant codes (used when reading the dump).
    LangInfo {
        code: "sh",
        name: "srbsko-horvatsky",
        branch: Branch::South,
        script: Script::Latin,
        modern: true,
        csv_col: "",
        speakers_m: 16.7,
    },
    LangInfo {
        code: "bs",
        name: "bosansky",
        branch: Branch::South,
        script: Script::Latin,
        modern: true,
        csv_col: "",
        speakers_m: 2.5,
    },
    LangInfo {
        code: "csb",
        name: "kašubsky",
        branch: Branch::West,
        script: Script::Latin,
        modern: true,
        csv_col: "",
        speakers_m: 0.05,
    },
    LangInfo {
        code: "dsb",
        name: "dolnolužičsky",
        branch: Branch::West,
        script: Script::Latin,
        modern: true,
        csv_col: "",
        speakers_m: 0.007,
    },
    LangInfo {
        code: "hsb",
        name: "gornolužičsky",
        branch: Branch::West,
        script: Script::Latin,
        modern: true,
        csv_col: "",
        speakers_m: 0.013,
    },
    LangInfo {
        code: "rue",
        name: "rusinsky",
        branch: Branch::East,
        script: Script::Cyrillic,
        modern: true,
        csv_col: "",
        speakers_m: 0.6,
    },
    LangInfo {
        code: "szl",
        name: "šlęzsky",
        branch: Branch::West,
        script: Script::Latin,
        modern: true,
        csv_col: "",
        speakers_m: 0.5,
    },
    // Archaic East Slavic (raw-corpus only): an etymological hint, not a modern voter.
    LangInfo {
        code: "orv",
        name: "starovȯstočnoslovjansky",
        branch: Branch::East,
        script: Script::Cyrillic,
        modern: false,
        csv_col: "",
        speakers_m: 0.0,
    },
];

pub fn lang_info(code: &str) -> Option<&'static LangInfo> {
    LANGS.iter().find(|l| l.code == code)
}

pub fn branch_of(code: &str) -> Option<Branch> {
    lang_info(code).map(|l| l.branch)
}

pub fn lang_name(code: &str) -> &'static str {
    lang_info(code).map_or("slovjansky", |l| l.name)
}

/// The Slavic columns of the official dictionary, in branch order, with the
/// modern-voter flag. Old Church Slavonic (`cu`) is included but flagged
/// non-modern so callers can use it only as an etymological hint.
pub fn official_slavic_cols() -> &'static [LangInfo] {
    // First 12 entries of LANGS are the CSV-backed languages in column order.
    &LANGS[0..12]
}

/// Relative speaker weights, used only as a population tie-break (§4.3).
/// These are hand-tuned tie-break weights, NOT millions of speakers — see
/// [`LangInfo::speakers_m`] for the measured populations. Moved verbatim from
/// consensus.rs (issue #79); the benchmark depends on these exact values.
pub fn pop_weight(code: &str) -> f32 {
    match code {
        "ru" => 1.0,
        "pl" => 0.44,
        "uk" => 0.42,
        "cs" => 0.10,
        "be" => 0.10,
        "sr" => 0.09,
        "bg" => 0.08,
        "sk" => 0.05,
        "hr" => 0.05,
        // Serbo-Croatian macro-code (English Wiktionary's `sh`): the combined
        // sr+hr+bs speaker base.
        "sh" => 0.17,
        "bs" => 0.03,
        "sl" => 0.02,
        "mk" => 0.02,
        _ => 0.0,
    }
}

/// The individual languages a code counts as for population purposes: the
/// `sh` macro-code covers Serbian, Croatian and Bosnian; every other registry
/// code is its own single atom. Codes outside the registry have no atoms
/// (callers skip them).
pub fn population_atoms(code: &str) -> &'static [&'static str] {
    if code == "sh" {
        return &["sr", "hr", "bs"];
    }
    match lang_info(code) {
        Some(l) => std::slice::from_ref(&l.code),
        None => &[],
    }
}

/// Speaker-weighted cognate coverage (issue #79): the share of modern Slavic
/// speakers whose language attests a related word — an attestation-based
/// proxy, NOT measured intelligibility. Percentages in 0-100.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Razumlivost {
    pub overall: f32,
    pub east: f32,
    pub west: f32,
    pub south: f32,
}

/// Population denominator: Σ [`LangInfo::speakers_m`] over the modern registry
/// languages of `branch` (all branches when `None`). The `sh` macro-code is
/// excluded — its speakers are already counted as sr+hr+bs.
fn speakers_denominator(branch: Option<Branch>) -> f32 {
    LANGS
        .iter()
        .filter(|l| l.modern && l.code != "sh" && branch.is_none_or(|b| l.branch == b))
        .map(|l| l.speakers_m)
        .sum()
}

/// Razumlivost of a set of attesting language codes: expand each code through
/// [`population_atoms`] (so `sh` and sr/hr/bs never double-count), sum the
/// atoms' speaker populations, and divide by the modern-speaker denominator —
/// overall and per branch. Codes outside the registry are skipped; the
/// non-modern hints (cu, orv) contribute 0 speakers.
pub fn razumlivost(codes: &[&str]) -> Razumlivost {
    let mut atoms: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    for code in codes {
        for a in population_atoms(code) {
            atoms.insert(a);
        }
    }
    let numerator = |branch: Option<Branch>| -> f32 {
        atoms
            .iter()
            .filter_map(|a| lang_info(a))
            .filter(|l| branch.is_none_or(|b| l.branch == b))
            // fold from +0.0: an empty float `sum()` is IEEE -0.0, which a
            // branch with no attestation would render as "-0%".
            .fold(0.0, |acc, l| acc + l.speakers_m)
    };
    let pct = |branch: Option<Branch>| -> f32 {
        let den = speakers_denominator(branch);
        if den > 0.0 {
            100.0 * numerator(branch) / den
        } else {
            0.0
        }
    };
    Razumlivost {
        overall: pct(None),
        east: pct(Some(Branch::East)),
        west: pct(Some(Branch::West)),
        south: pct(Some(Branch::South)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `official_slavic_cols` slices `LANGS[0..12]` by position; pin the layout
    /// that slice assumes — every CSV-backed language is in the first 12 slots
    /// and nothing after them carries a CSV column — so inserting a language in
    /// the wrong place fails here instead of silently dropping a column.
    #[test]
    fn official_slavic_cols_slice_matches_csv_backed_langs() {
        let cols = official_slavic_cols();
        assert_eq!(cols.len(), 12);
        for l in cols {
            assert!(!l.csv_col.is_empty(), "{} has no CSV column", l.code);
        }
        for l in &LANGS[12..] {
            assert!(
                l.csv_col.is_empty(),
                "{} is CSV-backed but outside official_slavic_cols",
                l.code
            );
        }
    }

    /// The raw-extraction language set must stay a superset of the benchmark
    /// corpus set, and every code in either must exist in the LANGS registry
    /// (PR #53 added `sh` everywhere; keep the registries from drifting again).
    #[test]
    fn lang_registries_are_consistent() {
        for code in crate::dump::SLAVIC_LANGS {
            assert!(
                crate::dump::RAW_SLAVIC_LANGS.contains(code),
                "{code} in SLAVIC_LANGS but not RAW_SLAVIC_LANGS"
            );
        }
        for code in crate::dump::RAW_SLAVIC_LANGS {
            assert!(
                lang_info(code).is_some(),
                "{code} in RAW_SLAVIC_LANGS but missing from lang.rs LANGS"
            );
        }
    }

    /// `sh` is the sr+hr+bs macro-code: mixing it with its own atoms must not
    /// double-count the shared speaker base, and it must equal the three atoms.
    #[test]
    fn razumlivost_does_not_double_count_the_sh_macro_code() {
        assert_eq!(razumlivost(&["sh", "sr"]), razumlivost(&["sh"]));
        assert_eq!(razumlivost(&["sh"]), razumlivost(&["sr", "hr", "bs"]));
    }

    /// Full modern coverage is 100% overall and in every branch (the `sh`
    /// expansion and the sh-free denominator agree on the same total).
    #[test]
    fn razumlivost_of_all_modern_codes_is_full_coverage() {
        let codes: Vec<&str> = LANGS.iter().filter(|l| l.modern).map(|l| l.code).collect();
        let r = razumlivost(&codes);
        for v in [r.overall, r.east, r.west, r.south] {
            assert!((v - 100.0).abs() < 0.01, "{r:?}");
        }
    }

    /// A branch with no attestation reports positive zero, never IEEE -0.0
    /// (an empty float `sum()` folds from -0.0 and would render "-0%").
    #[test]
    fn razumlivost_empty_branch_is_positive_zero() {
        let r = razumlivost(&["pl"]);
        assert_eq!(r.east, 0.0);
        assert!(r.east.is_sign_positive(), "east is -0.0");
        assert!(r.south.is_sign_positive(), "south is -0.0");
    }

    /// Every registry code expands to atoms that themselves resolve in the
    /// registry (so razumlivost never silently drops a known language).
    #[test]
    fn population_atoms_resolve_in_the_registry() {
        for l in LANGS {
            let atoms = population_atoms(l.code);
            assert!(!atoms.is_empty(), "{} has no population atoms", l.code);
            for a in atoms {
                assert!(lang_info(a).is_some(), "{a} not in LANGS");
            }
        }
    }
}
