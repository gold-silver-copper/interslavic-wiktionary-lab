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
    },
    LangInfo {
        code: "be",
        name: "bělorussky",
        branch: Branch::East,
        script: Script::Cyrillic,
        modern: true,
        csv_col: "be",
    },
    LangInfo {
        code: "uk",
        name: "ukrajinsky",
        branch: Branch::East,
        script: Script::Cyrillic,
        modern: true,
        csv_col: "uk",
    },
    // West
    LangInfo {
        code: "pl",
        name: "poljsky",
        branch: Branch::West,
        script: Script::Latin,
        modern: true,
        csv_col: "pl",
    },
    LangInfo {
        code: "cs",
        name: "čehsky",
        branch: Branch::West,
        script: Script::Latin,
        modern: true,
        csv_col: "cs",
    },
    LangInfo {
        code: "sk",
        name: "slovacky",
        branch: Branch::West,
        script: Script::Latin,
        modern: true,
        csv_col: "sk",
    },
    // South
    LangInfo {
        code: "sl",
        name: "slovensky",
        branch: Branch::South,
        script: Script::Latin,
        modern: true,
        csv_col: "sl",
    },
    LangInfo {
        code: "hr",
        name: "horvatsky",
        branch: Branch::South,
        script: Script::Latin,
        modern: true,
        csv_col: "hr",
    },
    LangInfo {
        code: "sr",
        name: "srbsky",
        branch: Branch::South,
        script: Script::Cyrillic,
        modern: true,
        csv_col: "sr",
    },
    LangInfo {
        code: "mk",
        name: "makedonsky",
        branch: Branch::South,
        script: Script::Cyrillic,
        modern: true,
        csv_col: "mk",
    },
    LangInfo {
        code: "bg",
        name: "bȯlgarsky",
        branch: Branch::South,
        script: Script::Cyrillic,
        modern: true,
        csv_col: "bg",
    },
    // Archaic South Slavic: only an etymological hint, not a modern voter.
    LangInfo {
        code: "cu",
        name: "starocŕkovnoslovjansky",
        branch: Branch::South,
        script: Script::Cyrillic,
        modern: false,
        csv_col: "cu",
    },
    // Additional Wiktionary descendant codes (used when reading the dump).
    LangInfo {
        code: "sh",
        name: "srbsko-horvatsky",
        branch: Branch::South,
        script: Script::Latin,
        modern: true,
        csv_col: "",
    },
    LangInfo {
        code: "bs",
        name: "bosansky",
        branch: Branch::South,
        script: Script::Latin,
        modern: true,
        csv_col: "",
    },
    LangInfo {
        code: "csb",
        name: "kašubsky",
        branch: Branch::West,
        script: Script::Latin,
        modern: true,
        csv_col: "",
    },
    LangInfo {
        code: "dsb",
        name: "dolnolužičsky",
        branch: Branch::West,
        script: Script::Latin,
        modern: true,
        csv_col: "",
    },
    LangInfo {
        code: "hsb",
        name: "gornolužičsky",
        branch: Branch::West,
        script: Script::Latin,
        modern: true,
        csv_col: "",
    },
    LangInfo {
        code: "rue",
        name: "rusinsky",
        branch: Branch::East,
        script: Script::Cyrillic,
        modern: true,
        csv_col: "",
    },
    LangInfo {
        code: "szl",
        name: "šlęzsky",
        branch: Branch::West,
        script: Script::Latin,
        modern: true,
        csv_col: "",
    },
    // Archaic East Slavic (raw-corpus only): an etymological hint, not a modern voter.
    LangInfo {
        code: "orv",
        name: "starovȯstočnoslovjansky",
        branch: Branch::East,
        script: Script::Cyrillic,
        modern: false,
        csv_col: "",
    },
];

pub fn lang_info(code: &str) -> Option<&'static LangInfo> {
    LANGS.iter().find(|l| l.code == code)
}

pub fn branch_of(code: &str) -> Option<Branch> {
    lang_info(code).map(|l| l.branch)
}

pub fn lang_name(code: &str) -> &'static str {
    lang_info(code).map(|l| l.name).unwrap_or("slovjansky")
}

/// The Slavic columns of the official dictionary, in branch order, with the
/// modern-voter flag. Old Church Slavonic (`cu`) is included but flagged
/// non-modern so callers can use it only as an etymological hint.
pub fn official_slavic_cols() -> &'static [LangInfo] {
    // First 12 entries of LANGS are the CSV-backed languages in column order.
    &LANGS[0..12]
}
