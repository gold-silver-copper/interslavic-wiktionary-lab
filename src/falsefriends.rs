//! Algorithmic false-friend (semantic-trap) detection.
//!
//! Replaces the retired curated `data/semantic-notes.json` with a deterministic
//! computation over the committed evidence caches: an official Interslavic
//! lemma is a *false friend* for language ℓ when ℓ has a word a speaker will
//! read as that lemma (same folded surface after the standard flavorization /
//! citation-ending adaptation) whose English Wiktionary glosses share **no
//! content token** with the official gloss. The old 12 hand-written notes are
//! encoded as test expectations below — never as runtime data.
//!
//! Surfaces are matched at two levels:
//! 1. **exact** — `form_key(flavorize_word(lang, pos, word)) == form_key(isv)`;
//! 2. **loose** — equal ASCII skeletons after additionally folding `y→i`
//!    (ru корыстный → korystny vs koristny), stems ≥ [`LOOSE_MIN_CHARS`] only.
//!
//! Gloss overlap (V11): token comparison with spelling-variant tolerance
//! (autogyro≈autogiro), a mined synonym/hypernym closure ([`SynonymMates`]),
//! junk-sense hygiene, and per-word PRIMARY-sense tracking that grades every
//! note `high`/`medium`/`low` — a slang-only divergence (pl banan) reads as
//! "colloquially also", never as THE meaning. Official homographs pool their
//! glosses first, so a sense the dictionary itself attests (žena 'woman,
//! wife'; vonjati 'smell, stink') is *not* flagged — by design, unlike two of
//! the old curated "ambiguous by attestation" notes.

use crate::dump::{LemmaCorpus, RawSlavicCorpus};
use crate::official::OfficialEntry;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Minimum folded-key length for the loose (y→i-folded skeleton) match level.
const LOOSE_MIN_CHARS: usize = 5;
/// Minimum folded-key length for any collision at all (short function words
/// collide across the whole family for phonology, not semantics).
const MIN_KEY_CHARS: usize = 3;
/// At most this many languages are rendered into the warning sentence (the
/// `collisions` list stays complete).
const MAX_WARNED_LANGS: usize = 4;
/// At most this many divergent senses are quoted per colliding word.
const MAX_QUOTED_SENSES: usize = 2;

/// Sharding of the published notes artifact (V11 item 6): route by
/// `fnv1a32(folded_key) % NOTES_SHARDS`, mirroring the suggest index.
pub const NOTES_SHARDS: u32 = 64;
/// First versioned notes schema — the monolithic unversioned `api/notes.json`
/// is retired in favor of `api/notes/<n>.json` shards.
pub const NOTES_SCHEMA_VERSION: u32 = 1;
/// Frozen router inputs for `api/notes-selftest.json` ([key, shard] pairs).
pub const NOTES_SELFTEST_SAMPLES: &[&str] = &["pytati", "jutro", "čas", "zakuska", "koristny"];

/// Modern languages whose speakers the warnings address, in render order.
/// English Wiktionary's `sh` macro-code covers Serbian/Croatian/Bosnian.
const LANGS: &[(&str, &str)] = &[
    ("ru", "Russian"),
    ("uk", "Ukrainian"),
    ("be", "Belarusian"),
    ("pl", "Polish"),
    ("cs", "Czech"),
    ("sk", "Slovak"),
    ("sl", "Slovene"),
    ("sh", "Serbo-Croatian"),
    ("mk", "Macedonian"),
    ("bg", "Bulgarian"),
];

/// One colliding source word with divergent meaning.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Collision {
    /// Language code (`sh` = Serbo-Croatian macro-code).
    pub lang: String,
    /// The colliding word in its native spelling.
    pub word: String,
    /// The divergent English Wiktionary glosses (deduped, parentheticals kept
    /// for display).
    pub glosses: Vec<String>,
    /// True when the word's PRIMARY sense (first gloss of its first cache
    /// record) agrees with the official gloss — the divergence is a
    /// secondary/colloquial sense (pl banan: 'banana' first, slang later).
    pub primary_agrees: bool,
    /// Surface-match level: `exact` (same folded surface) or `loose`
    /// (y→i-folded skeleton). Exact collisions are the classic traps and
    /// outrank loose ones in the rendered warning (V12 item 2).
    pub level: &'static str,
}

/// A computed semantic-trap note for one folded Interslavic key. The
/// `warning`/`prefer` shape is kept from the curated era so consumers
/// (check-text JSON, `api/notes.json`, English-API candidates) don't break;
/// `collisions` carries the full machine-readable evidence.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Note {
    pub warning: String,
    /// Official lemmas whose gloss best covers the divergent sense (computed,
    /// e.g. urok trap 'lesson' → lekcija), possibly empty.
    pub prefer: Vec<String>,
    /// `high` — the colliding word's primary sense diverges in ≥2 languages;
    /// `medium` — primary-sense divergence in exactly 1 language;
    /// `low` — only secondary/colloquial senses diverge anywhere.
    pub severity: &'static str,
    pub collisions: Vec<Collision>,
}

// The tokenizer cluster (strip_parens, STOPWORDS, gloss_tokens,
// ordered_tokens, light_stem) moved verbatim to crate::gloss (V15 item 4);
// the public path falsefriends::gloss_tokens stays valid.
pub use crate::gloss::stemmed_tokens as gloss_tokens;
use crate::gloss::{ordered_stemmed_tokens as ordered_tokens, strip_parens};

/// Deterministic English synonym-mate pairs, mined from the committed data.
/// Precision matters more than recall here — a wrong mate suppresses a real
/// trap — so only three high-precision constructions feed the closure:
///
/// 1. **Official gloss lists** — comma items inside ONE official entry's
///    gloss name the same meaning by the dictionary's own design
///    ('lecture, lesson, class period').
/// 2. **Definitional parentheticals** in cache glosses — 'kitten (young
///    cat)' pairs the head with each short-parenthetical token.
/// 3. **`X or Y` alternatives** inside one sense segment — 'a snack or
///    appetizer' pairs the tokens adjacent to the `or`.
///
/// Deliberately NOT used: plain token co-occurrence inside a gloss. Slavic
/// čas-family words gloss 'hour' and 'time' together because they ARE the
/// false friend — polysemy co-occurrence is circular evidence, and using it
/// suppressed the curated čas trap.
pub struct SynonymMates {
    /// Unordered synonym pairs (official lists, `X or Y` alternatives).
    symmetric: std::collections::HashSet<(String, String)>,
    /// Directed (head, paren) pairs from definitional parentheticals:
    /// 'kitten (young cat)' ⇒ kitten is-defined-via cat. Direction matters:
    /// a collision token that is the OFFICIAL word's hypernym is harmless
    /// (kote/'cat'), but a collision token that is a hyponym of the official
    /// gloss is exactly the curated čas trap ('hour' vs official 'time' —
    /// 'hour (unit of time)' must NOT suppress it).
    defined_via: std::collections::HashSet<(String, String)>,
}

impl SynonymMates {
    /// Symmetric synonym-mates.
    pub fn are_mates(&self, a: &str, b: &str) -> bool {
        let pair = if a <= b { (a, b) } else { (b, a) };
        self.symmetric
            .contains(&(pair.0.to_string(), pair.1.to_string()))
    }

    /// True when `official_token` is defined via `collision_token` — the
    /// collision names the official meaning's hypernym.
    pub fn official_defined_via(&self, official_token: &str, collision_token: &str) -> bool {
        self.defined_via
            .contains(&(official_token.to_string(), collision_token.to_string()))
    }

    fn insert_pair(set: &mut std::collections::HashSet<(String, String)>, a: &str, b: &str) {
        if a == b || a.chars().count() < 3 || b.chars().count() < 3 {
            return;
        }
        let pair = if a <= b { (a, b) } else { (b, a) };
        set.insert((pair.0.to_string(), pair.1.to_string()));
    }

    /// Top-level `(...)` group contents of a gloss.
    fn paren_groups(gloss: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut depth = 0i32;
        let mut cur = String::new();
        for ch in gloss.chars() {
            match ch {
                '(' => {
                    depth += 1;
                    if depth == 1 {
                        cur.clear();
                        continue;
                    }
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        out.push(std::mem::take(&mut cur));
                        continue;
                    }
                    depth = depth.max(0);
                }
                _ => {}
            }
            if depth >= 1 {
                cur.push(ch);
            }
        }
        out
    }

    fn build<'a>(
        official_glosses: impl Iterator<Item = &'a str>,
        cache_glosses: impl Iterator<Item = &'a str>,
    ) -> Self {
        const MAX_OFFICIAL_TOKENS: usize = 6;
        // A true hypernym label is 1-2 content tokens ('young cat'); longer
        // parentheticals are sentence definitions whose stopword-stripped
        // token count shrinks under a looser cap — pl czas 'time (particular
        // moment or hour…)' must not mint (time→hour)/(time→moment) pairs
        // that suppress the curated čas trap.
        const MAX_PAREN_TOKENS: usize = 2;
        let mut symmetric: std::collections::HashSet<(String, String)> = Default::default();
        let mut defined_via: std::collections::HashSet<(String, String)> = Default::default();
        // 1. Official gloss lists: all pairs, capped length.
        for g in official_glosses {
            let toks: Vec<String> = gloss_tokens(g).into_iter().collect();
            if toks.len() < 2 || toks.len() > MAX_OFFICIAL_TOKENS {
                continue;
            }
            for i in 0..toks.len() {
                for j in (i + 1)..toks.len() {
                    Self::insert_pair(&mut symmetric, &toks[i], &toks[j]);
                }
            }
        }
        for g in cache_glosses {
            // 2. Definitional parentheticals → DIRECTED (head, paren) pairs.
            let head: Vec<String> = gloss_tokens(g).into_iter().collect();
            if !head.is_empty() && head.len() <= MAX_PAREN_TOKENS {
                for paren in Self::paren_groups(g) {
                    let ptoks: Vec<String> = gloss_tokens(&paren).into_iter().collect();
                    if ptoks.is_empty() || ptoks.len() > MAX_PAREN_TOKENS {
                        continue;
                    }
                    for h in &head {
                        for p in &ptoks {
                            if h != p && h.chars().count() >= 3 && p.chars().count() >= 3 {
                                defined_via.insert((h.clone(), p.clone()));
                            }
                        }
                    }
                }
            }
            // 3. `X or Y`: the tokens adjacent to the `or` within one segment.
            for segment in strip_parens(g).split([',', ';']) {
                let lower = segment.to_lowercase();
                let parts: Vec<&str> = lower.split(" or ").collect();
                for pair in parts.windows(2) {
                    let left = ordered_tokens(pair[0]).pop();
                    let right = ordered_tokens(pair[1]).into_iter().next();
                    if let (Some(l), Some(r)) = (left, right) {
                        Self::insert_pair(&mut symmetric, &l, &r);
                    }
                }
            }
        }
        SynonymMates {
            symmetric,
            defined_via,
        }
    }
}

/// Spelling-variant tolerance: `autogyro`≈`autogiro` (edit distance 1),
/// `chirrup`≈`chirp` (shared 4-char prefix, distance ≤2). Conservative
/// lengths keep `market`≠`marketplace`-class pairs apart (distance 5).
fn near_match(a: &str, b: &str) -> bool {
    let (la, lb) = (a.chars().count(), b.chars().count());
    let min = la.min(lb);
    if min < 5 {
        return false;
    }
    let d = crate::orthography::levenshtein(a, b);
    if d <= 1 {
        return true;
    }
    let prefix = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    min >= 5 && prefix >= 4 && d <= 2
}

/// Token-set overlap for divergence testing (`collision` vs `official`):
/// exact equality, spelling-variant near-match, mined synonym-mates, or the
/// collision naming the official meaning's hypernym (directed).
fn overlaps(
    collision: &BTreeSet<String>,
    official: &BTreeSet<String>,
    mates: &SynonymMates,
) -> bool {
    collision.iter().any(|x| {
        official.iter().any(|y| {
            x == y || near_match(x, y) || mates.are_mates(x, y) || mates.official_defined_via(y, x)
        })
    })
}

/// A gloss fit for quoting in a warning: no proper-noun/pop-culture senses
/// (any uppercase letter — 'YouTube poop', 'Christmas Eve dish'), clipped at
/// the first ';', capped at a word boundary. Junk senses are also excluded
/// from divergence tokens at ingestion.
fn clean_quote(gloss: &str) -> Option<String> {
    let g = gloss.trim();
    if g.is_empty() || g.chars().any(char::is_uppercase) {
        return None;
    }
    let g = g.split(';').next().unwrap_or(g).trim();
    const MAX_QUOTE_CHARS: usize = 90;
    if g.chars().count() <= MAX_QUOTE_CHARS {
        return Some(g.to_string());
    }
    let mut cut = String::new();
    for word in g.split_whitespace() {
        if cut.chars().count() + word.chars().count() + 1 > MAX_QUOTE_CHARS {
            break;
        }
        if !cut.is_empty() {
            cut.push(' ');
        }
        cut.push_str(word);
    }
    Some(format!("{cut}…"))
}

/// The loose comparison key: ASCII skeleton with `y` folded onto `i`.
fn loose_key(folded: &str) -> String {
    crate::orthography::ascii_skeleton(folded).replace('y', "i")
}

/// How language ℓ's dictionary word surfaces to a reader of Interslavic:
/// citation-ending adaptation + per-language flavorization, folded to the
/// standard lookup key (the same transform the raw-attestation pages use).
fn read_as_key(lang: &str, pos: &str, word: &str) -> String {
    crate::forms::form_key(&crate::flavorize::flavorize_word(lang, pos, word))
}

/// Only open-class cache rows participate; closed-class/name rows collide for
/// grammatical, not semantic, reasons.
fn eligible_pos(pos: &str) -> bool {
    matches!(pos, "noun" | "verb" | "adj" | "adv")
}

struct CacheWord {
    lang_idx: usize,
    word: String,
    /// Wiktextract POS of the record ("noun"/"verb"/"adj"/"adv").
    pos: String,
    glosses: Vec<String>,
    tokens: BTreeSet<String>,
}

/// Compute the notes map, keyed by the folded Interslavic lookup key
/// (`forms::form_key`). Deterministic: BTree collections throughout, fixed
/// language order, alphabetical tie-breaks.
pub fn compute(
    official: &[OfficialEntry],
    evidence: Option<&LemmaCorpus>,
    raw: Option<&RawSlavicCorpus>,
    enrich: Option<&crate::enrich::EnrichIndex>,
) -> BTreeMap<String, Note> {
    let lang_idx: HashMap<&str, usize> = LANGS.iter().enumerate().map(|(i, l)| (l.0, i)).collect();

    // ---- 0. Synonym-mate closure from the committed data itself. ----
    let mates = SynonymMates::build(
        official.iter().map(|e| e.english.as_str()),
        evidence
            .into_iter()
            .flat_map(|c| c.entries.iter().map(|e| e.gloss.as_str()))
            .chain(raw.into_iter().flat_map(|c| {
                c.lemmas
                    .iter()
                    .flat_map(|e| e.glosses.iter().map(String::as_str))
            })),
    );

    // ---- 1/2. Index cache records by read-as key (exact and loose). ----
    // Divergence is judged per RECORD (one POS/etymology entry of one word),
    // not per pooled word: pl jutro-the-adverb 'tomorrow' must fire even
    // though pl jutro-the-noun also lists an archaic 'morning' sense. Records
    // for the same (lang, word) later merge into one collision for display.
    // Proper-noun/pop-culture senses (uppercase letters — 'YouTube poop')
    // are dropped at ingestion, from both tokens and quotable glosses.
    let mut words: Vec<CacheWord> = Vec::new();
    let mut by_exact: HashMap<String, Vec<usize>> = HashMap::new();
    let mut by_loose: HashMap<String, Vec<usize>> = HashMap::new();
    // (lang, word) → tokens of the PRIMARY sense: first clean gloss of the
    // first cache record, in committed cache order.
    let mut primary_tokens: HashMap<(usize, String), BTreeSet<String>> = HashMap::new();
    {
        let mut add = |lang: &str, word: &str, pos: &str, glosses: Vec<String>| {
            let Some(&li) = lang_idx.get(lang) else {
                return;
            };
            if !eligible_pos(pos) || word.contains(' ') {
                return;
            }
            let glosses: Vec<String> = glosses
                .into_iter()
                .filter(|g| !g.trim().is_empty() && !g.chars().any(char::is_uppercase))
                .collect();
            let tokens: BTreeSet<String> = glosses.iter().flat_map(|g| gloss_tokens(g)).collect();
            if tokens.is_empty() {
                return;
            }
            let key = read_as_key(lang, pos, word);
            if key.chars().count() < MIN_KEY_CHARS {
                return;
            }
            primary_tokens
                .entry((li, word.to_string()))
                .or_insert_with(|| gloss_tokens(&glosses[0]));
            let idx = words.len();
            by_exact.entry(key.clone()).or_default().push(idx);
            if key.chars().count() >= LOOSE_MIN_CHARS {
                by_loose.entry(loose_key(&key)).or_default().push(idx);
            }
            words.push(CacheWord {
                lang_idx: li,
                word: word.to_string(),
                pos: pos.to_string(),
                glosses,
                tokens,
            });
        };
        if let Some(corpus) = evidence {
            for e in &corpus.entries {
                add(&e.lang, &e.word, &e.pos, vec![e.gloss.clone()]);
            }
        }
        if let Some(corpus) = raw {
            for e in &corpus.lemmas {
                add(&e.lang, &e.word, &e.pos, e.glosses.clone());
            }
        }
    }

    // ---- 3. Pool official glosses per folded key (homographs vote jointly),
    // and build the token → official-lemma index that computes `prefer`. ----
    let mut official_tokens: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut official_display: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    let mut by_token: HashMap<String, Vec<usize>> = HashMap::new();
    for (ei, e) in official.iter().enumerate() {
        let isv = e.isv.trim();
        if isv.is_empty() || isv.contains(' ') || isv.contains('#') || e.english.trim().is_empty() {
            continue;
        }
        let key = crate::forms::form_key(isv);
        if key.chars().count() < MIN_KEY_CHARS {
            continue;
        }
        let toks = gloss_tokens(&e.english);
        for t in &toks {
            by_token.entry(t.clone()).or_default().push(ei);
        }
        official_tokens.entry(key.clone()).or_default().extend(toks);
        let display = official_display
            .entry(key)
            .or_insert_with(|| (isv.to_string(), Vec::new()));
        let gloss = e.english.trim().to_string();
        if !display.1.iter().any(|have| have == &gloss) {
            display.1.push(gloss);
        }
    }

    // Bridge index (V12 item 1): (cell language, verbatim cell variant) →
    // official entries listing that word as their translation. The Slavic
    // side is the authority on which official lemma a colliding word
    // actually renders — English tokens only rank within it.
    let mut by_cell_word: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (ei, e) in official.iter().enumerate() {
        for (lang, cell) in &e.cells {
            for (variant, _) in crate::normalize::split_cell(cell) {
                by_cell_word
                    .entry((lang.clone(), variant))
                    .or_default()
                    .push(ei);
            }
        }
    }
    // Collision language → official CSV cell columns (`sh` covers hr+sr).
    let cell_langs = |code: &str| -> Vec<&'static str> {
        match code {
            "sh" => vec!["hr", "sr"],
            "ru" => vec!["ru"],
            "uk" => vec!["uk"],
            "be" => vec!["be"],
            "pl" => vec!["pl"],
            "cs" => vec!["cs"],
            "sk" => vec!["sk"],
            "sl" => vec!["sl"],
            "mk" => vec!["mk"],
            "bg" => vec!["bg"],
            _ => vec![],
        }
    };

    // ---- 4. Detect divergent collisions per official key. ----
    let mut notes: BTreeMap<String, Note> = BTreeMap::new();
    for (key, isv_tokens) in &official_tokens {
        if isv_tokens.is_empty() {
            continue;
        }
        // record idx → matched at exact level (loose-only records are false).
        let mut candidate_idxs: BTreeMap<usize, bool> = BTreeMap::new();
        if let Some(v) = by_exact.get(key) {
            for &i in v {
                candidate_idxs.insert(i, true);
            }
        }
        if key.chars().count() >= LOOSE_MIN_CHARS {
            if let Some(v) = by_loose.get(&loose_key(key)) {
                for &i in v {
                    candidate_idxs.entry(i).or_insert(false);
                }
            }
        }
        // Merge divergent records of the same (lang, word) into one collision.
        struct Merged {
            lang_idx: usize,
            word: String,
            poses: BTreeSet<String>,
            glosses: Vec<String>,
            tokens: BTreeSet<String>,
            primary_agrees: bool,
            exact: bool,
        }
        let mut merged: BTreeMap<(usize, String), Merged> = BTreeMap::new();
        for (w, exact) in candidate_idxs
            .into_iter()
            .map(|(i, exact)| (&words[i], exact))
            .filter(|(w, _)| !overlaps(&w.tokens, isv_tokens, &mates))
        {
            let primary_agrees = primary_tokens
                .get(&(w.lang_idx, w.word.clone()))
                .is_some_and(|p| overlaps(p, isv_tokens, &mates));
            let slot = merged
                .entry((w.lang_idx, w.word.clone()))
                .or_insert_with(|| Merged {
                    lang_idx: w.lang_idx,
                    word: w.word.clone(),
                    poses: BTreeSet::new(),
                    glosses: Vec::new(),
                    tokens: BTreeSet::new(),
                    primary_agrees,
                    exact: false,
                });
            slot.exact |= exact;
            slot.poses.insert(w.pos.clone());
            for g in &w.glosses {
                if !slot.glosses.iter().any(|have| have == g) {
                    slot.glosses.push(g.clone());
                }
            }
            slot.tokens.extend(w.tokens.iter().cloned());
        }
        if merged.is_empty() {
            continue;
        }
        let collisions: Vec<Merged> = merged.into_values().collect();

        // Severity: how many languages misread the word's PRIMARY sense.
        let primary_divergent_langs: BTreeSet<usize> = collisions
            .iter()
            .filter(|c| !c.primary_agrees)
            .map(|c| c.lang_idx)
            .collect();
        let severity = match primary_divergent_langs.len() {
            0 => "low",
            1 => "medium",
            _ => "high",
        };

        let (_, gloss_display) = &official_display[key];
        let mut warning = format!("Official meaning: '{}'.", gloss_display.join("' / '"));
        // Quote primary-sense traps first, colloquial-only divergences after,
        // each with wording that says which kind it is.
        let mut warned_langs: Vec<usize> = Vec::new();
        // Primary-divergent before colloquial; within each, EXACT collisions
        // before loose (V12 item 2: pytati must quote пытать 'torture', the
        // exact classic trap, not only the loose питать 'feed') — and since
        // one language gets one quote, exact-first means the exact sense is
        // the one a reader sees.
        let mut quote_order: Vec<&Merged> = collisions.iter().collect();
        quote_order.sort_by_key(|c| (c.primary_agrees, !c.exact, c.lang_idx, c.word.clone()));
        for w in quote_order {
            if warned_langs.contains(&w.lang_idx) || warned_langs.len() >= MAX_WARNED_LANGS {
                continue;
            }
            let senses: Vec<String> = w
                .glosses
                .iter()
                .filter_map(|g| clean_quote(g))
                .take(MAX_QUOTED_SENSES)
                .collect();
            if senses.is_empty() {
                continue;
            }
            warned_langs.push(w.lang_idx);
            let verb = if w.primary_agrees {
                "speakers may colloquially also read it as"
            } else {
                "speakers may read it as"
            };
            let _ = std::fmt::Write::write_fmt(
                &mut warning,
                format_args!(
                    " {} {} '{}' ({} {}).",
                    LANGS[w.lang_idx].1,
                    verb,
                    senses.join("; "),
                    LANGS[w.lang_idx].0,
                    w.word
                ),
            );
        }
        if warned_langs.is_empty() {
            // Every divergent sense was junk-filtered: nothing quotable means
            // nothing warnable.
            continue;
        }

        // `prefer` (V12 item 1): Slavic evidence first, English tokens second.
        // English polysemy defeats any token threshold (ravnina 'plane
        // (surface)' → aviakarta via the aircraft), so:
        //
        // (1) BRIDGED candidates win: the colliding word itself — or, when
        //     the collision is primary-divergent, one of its native-Wiktionary
        //     synonyms — appears verbatim in the candidate's own cognate cell
        //     for that language (uk урок in lekcija's uk cell; ru питать's
        //     synonym кормить in krmiti's ru cell). Ranked by bridge count,
        //     then English coverage, frequency, lemma. Synonym bridging is
        //     restricted to primary-divergent collisions because enrich
        //     synonyms describe the word's PRIMARY sense — pl barwić's dye
        //     synonyms must not bridge its colloquial 'characterize' sense.
        // (2) Without any bridge, the English fallback keeps V11's coverage
        //     threshold AND additionally requires the candidate's ENTIRE
        //     gloss to sit inside the divergent sense (harakterizovati
        //     {characterize} ⊆ the barwić trap tokens ✓; aviakarta
        //     {plane, ticket, …} ⊄ {plane} ✗).
        // (3) Otherwise: empty. Fail-closed stands.
        const PREFER_MIN_COVERAGE: usize = 600_000;
        let pos_class = |p: crate::model::Pos| match p {
            crate::model::Pos::Noun | crate::model::Pos::ProperNoun => "noun",
            crate::model::Pos::Verb => "verb",
            crate::model::Pos::Adjective => "adj",
            crate::model::Pos::Adverb => "adv",
            _ => "other",
        };
        let token_matches = |t: &String, entry_tokens: &BTreeSet<String>| {
            entry_tokens
                .iter()
                .any(|y| t == y || near_match(t, y) || mates.are_mates(t, y))
        };
        // Candidate discovery: English tokens (fallback + ranking) UNION the
        // Slavic bridge index.
        let mut candidate_entries: BTreeSet<usize> = BTreeSet::new();
        // (entry idx → number of collisions bridging to it)
        let mut bridge_counts: BTreeMap<usize, usize> = BTreeMap::new();
        for c in &collisions {
            for t in &c.tokens {
                if let Some(eis) = by_token.get(t) {
                    candidate_entries.extend(eis.iter().copied());
                }
            }
            // Bridge words: the colliding word, plus its enrich synonyms for
            // primary-divergent collisions.
            let mut bridge_words: Vec<String> = vec![c.word.clone()];
            if !c.primary_agrees {
                if let Some(idx) = enrich {
                    if let Some(entry) = idx.get(LANGS[c.lang_idx].0, &c.word) {
                        bridge_words.extend(entry.synonyms.iter().cloned());
                    }
                }
            }
            let mut bridged_here: BTreeSet<usize> = BTreeSet::new();
            for lang in cell_langs(LANGS[c.lang_idx].0) {
                for w in &bridge_words {
                    if let Some(eis) = by_cell_word.get(&(lang.to_string(), w.trim().to_string())) {
                        bridged_here.extend(eis.iter().copied());
                    }
                }
            }
            for ei in bridged_here {
                // Bridge sanity: the candidate's gloss must share ≥1
                // exact/near token with THIS collision's divergent sense.
                // Enrich synonyms describe some sense of the colliding word,
                // not necessarily the trap one — pl staja's synonym chain
                // reached komnata 'room' for a 'shepherd hut' trap.
                let sane = gloss_tokens(&official[ei].english)
                    .iter()
                    .any(|y| c.tokens.iter().any(|t| t == y || near_match(t, y)));
                if sane {
                    *bridge_counts.entry(ei).or_default() += 1;
                    candidate_entries.insert(ei);
                }
            }
        }
        // English coverage + gloss-subset flag per candidate.
        let mut scores: BTreeMap<usize, (usize, bool)> = BTreeMap::new(); // (coverage, subset)
        for &ei in &candidate_entries {
            let e = &official[ei];
            let entry_pos = pos_class(e.pos);
            let entry_tokens = gloss_tokens(&e.english);
            let mut total = 0usize;
            let mut pos_ok = false;
            let mut subset_of_some = false;
            for c in &collisions {
                if !c.poses.iter().any(|p| p == entry_pos) {
                    continue;
                }
                pos_ok = true;
                let covered = c
                    .tokens
                    .iter()
                    .filter(|t| token_matches(t, &entry_tokens))
                    .count();
                total += covered * 1_000_000 / c.tokens.len();
                // Entire candidate gloss inside this collision's sense?
                // Exact/near matches ONLY — the mates closure is built from
                // official comma-lists, so a candidate's own gloss list
                // ('airplane ticket, plane ticket') would mint ticket≈plane
                // and vacuously subset itself into the trap sense.
                if !entry_tokens.is_empty()
                    && entry_tokens
                        .iter()
                        .all(|y| c.tokens.iter().any(|t| t == y || near_match(t, y)))
                {
                    subset_of_some = true;
                }
            }
            if pos_ok && (total > 0 || bridge_counts.contains_key(&ei)) {
                scores.insert(ei, (total, subset_of_some));
            }
        }
        // (bridged, bridge_count, coverage, freq, lemma-rev) — max wins.
        let mut best: Option<(bool, usize, usize, f32, String)> = None;
        for (ei, (coverage, subset)) in &scores {
            let e = &official[*ei];
            let lemma = e.isv.trim().to_string();
            if &crate::forms::form_key(&lemma) == key {
                continue;
            }
            let bridges = bridge_counts.get(ei).copied().unwrap_or(0);
            // Unbridged candidates must pass the strict English fallback.
            if bridges == 0 && (*coverage < PREFER_MIN_COVERAGE || !subset) {
                continue;
            }
            let freq = e.frequency.unwrap_or(0.0);
            let cand = (bridges > 0, bridges, *coverage, freq, lemma);
            let better = match &best {
                None => true,
                Some(b) => {
                    (
                        cand.0,
                        cand.1,
                        cand.2,
                        cand.3,
                        std::cmp::Reverse(cand.4.clone()),
                    ) > (b.0, b.1, b.2, b.3, std::cmp::Reverse(b.4.clone()))
                }
            };
            if better {
                best = Some(cand);
            }
        }
        let prefer: Vec<String> = best
            .map(|(_, _, _, _, lemma)| vec![lemma])
            .unwrap_or_default();

        notes.insert(
            key.clone(),
            Note {
                warning,
                prefer,
                severity,
                collisions: collisions
                    .into_iter()
                    .map(|w| Collision {
                        lang: LANGS[w.lang_idx].0.to_string(),
                        word: w.word,
                        glosses: w.glosses,
                        primary_agrees: w.primary_agrees,
                        level: if w.exact { "exact" } else { "loose" },
                    })
                    .collect(),
            },
        );
    }
    notes
}

/// How every cached Slavic word reads against ONE coined/queried surface
/// (V12 item 6, `coin-check`): scan both caches for records whose read-as
/// key equals the surface's folded key (or loose y→i skeleton), returning
/// per-language readings with clean glosses. A linear scan — fine for a
/// single-word CLI query.
pub fn surface_readings(
    surface: &str,
    evidence: Option<&LemmaCorpus>,
    raw: Option<&RawSlavicCorpus>,
) -> Vec<Collision> {
    let lang_idx: HashMap<&str, usize> = LANGS.iter().enumerate().map(|(i, l)| (l.0, i)).collect();
    let key = crate::forms::form_key(surface);
    if key.chars().count() < MIN_KEY_CHARS {
        return Vec::new();
    }
    let loose = if key.chars().count() >= LOOSE_MIN_CHARS {
        Some(loose_key(&key))
    } else {
        None
    };
    let mut merged: BTreeMap<(usize, String), (Vec<String>, bool)> = BTreeMap::new();
    let mut scan = |lang: &str, word: &str, pos: &str, glosses: &[String]| {
        let Some(&li) = lang_idx.get(lang) else {
            return;
        };
        if !eligible_pos(pos) || word.contains(' ') {
            return;
        }
        let k = read_as_key(lang, pos, word);
        let exact = k == key;
        let loose_hit = !exact
            && loose
                .as_ref()
                .is_some_and(|l| k.chars().count() >= LOOSE_MIN_CHARS && &loose_key(&k) == l);
        if !exact && !loose_hit {
            return;
        }
        let entry = merged
            .entry((li, word.to_string()))
            .or_insert_with(|| (Vec::new(), false));
        entry.1 |= exact;
        for g in glosses {
            let g = g.trim();
            if !g.is_empty()
                && !g.chars().any(char::is_uppercase)
                && !entry.0.iter().any(|have| have == g)
            {
                entry.0.push(g.to_string());
            }
        }
    };
    if let Some(corpus) = evidence {
        for e in &corpus.entries {
            scan(&e.lang, &e.word, &e.pos, std::slice::from_ref(&e.gloss));
        }
    }
    if let Some(corpus) = raw {
        for e in &corpus.lemmas {
            scan(&e.lang, &e.word, &e.pos, &e.glosses);
        }
    }
    merged
        .into_iter()
        .filter(|(_, (glosses, _))| !glosses.is_empty())
        .map(|((li, word), (glosses, exact))| Collision {
            lang: LANGS[li].0.to_string(),
            word,
            glosses,
            primary_agrees: false,
            level: if exact { "exact" } else { "loose" },
        })
        .collect()
}

/// Load the caches and compute notes. The ACTUAL contract (V15 item 2):
/// an ABSENT cache degrades silently to fewer/no notes so `check-text`
/// stays usable without them; a cache that EXISTS but fails to load
/// (corrupt/stale schema) is a hard error naming the cache — these feed
/// the shipped notes shards, and silent degradation is precisely what the
/// schema stamps exist to prevent. (The V15-era warn-and-continue helper
/// was unreachable under this contract and is gone — V15.1 item 6.)
pub fn compute_from_default_caches(
    official: &[OfficialEntry],
) -> anyhow::Result<BTreeMap<String, Note>> {
    let evidence = crate::dump::load_optional(
        std::path::Path::new(crate::DEFAULT_LEMMA_CACHE),
        LemmaCorpus::load,
    )?;
    let raw = crate::dump::load_optional(
        std::path::Path::new(crate::DEFAULT_RAW_LEMMA_CACHE),
        RawSlavicCorpus::load,
    )?;
    let enrich = crate::dump::load_optional(
        std::path::Path::new(crate::DEFAULT_ENRICH_CACHE),
        crate::enrich::EnrichIndex::load,
    )?;
    Ok(compute(
        official,
        evidence.as_ref(),
        raw.as_ref(),
        enrich.as_ref(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes() -> BTreeMap<String, Note> {
        let official =
            crate::official::load(std::path::Path::new(crate::DEFAULT_OFFICIAL)).unwrap();
        compute_from_default_caches(&official).expect("caches load")
    }

    /// The retired curated notes (data/semantic-notes.json, removed) as a
    /// held-out sanity set: the detector must independently rediscover the
    /// genuine surface-collision traps among them.
    #[test]
    fn rediscovers_curated_false_friends() {
        let notes = notes();
        let hit = |key: &str, lang: &str| {
            notes
                .get(key)
                .map(|n| n.collisions.iter().any(|c| c.lang == lang))
                .unwrap_or(false)
        };
        assert!(hit("pytati", "ru"), "pytati / ru пытать 'torture'");
        assert!(hit("jutro", "pl"), "jutro / pl jutro 'tomorrow'");
        assert!(hit("čas", "sh"), "čas / sh час 'hour, moment'");
        assert!(hit("urok", "ru"), "urok / ru урок 'lesson'");
        assert!(hit("rok", "sh"), "rok / sh rok 'deadline'");
        assert!(hit("slovo", "sh"), "slovo / sh slovo 'letter'");
        assert!(hit("koristny", "ru"), "koristny / ru корыстный (loose y/i)");
        assert!(hit("trg", "sh"), "trg / sh trg 'square'");
    }

    /// The two curated "ambiguous by attestation" notes must NOT fire: the
    /// dictionary itself attests both senses, so the colliding word's gloss
    /// overlaps the pooled official gloss. Documented behavior change.
    #[test]
    fn attested_ambiguity_is_not_a_false_friend() {
        let notes = notes();
        assert!(!notes.contains_key("žena"), "žena 'woman/wife' is attested");
        for key in ["vonjati", "vonjeti"] {
            if let Some(n) = notes.get(key) {
                assert!(
                    !n.collisions
                        .iter()
                        .any(|c| c.lang == "cs" || c.lang == "ru"),
                    "vonjati polarity is attested ambiguity, not a false friend"
                );
            }
        }
    }

    /// urok's divergent sense is 'lesson'; the computed prefer must point at
    /// an official word for it. Under V12's Slavic bridge the winner is
    /// pouka — its OWN ru/uk/bg cells list урок, three bridging languages to
    /// lekcija's one (uk only) — but either is a correct lesson-word.
    #[test]
    fn prefer_recovers_a_lesson_word_for_urok() {
        let notes = notes();
        let urok = notes.get("urok").expect("urok note");
        assert_eq!(urok.prefer.len(), 1, "{:?}", urok.prefer);
        assert!(
            matches!(urok.prefer[0].as_str(), "pouka" | "lekcija"),
            "{:?}",
            urok.prefer
        );
    }

    /// Volume + determinism guard: the detector's yield should stay in a sane
    /// band (a threshold regression that floods or empties the notes artifact
    /// must fail loudly, not ship silently).
    #[test]
    fn note_volume_stays_sane() {
        let notes = notes();
        let collisions: usize = notes.values().map(|n| n.collisions.len()).sum();
        let sev = |lvl: &str| notes.values().filter(|n| n.severity == lvl).count();
        eprintln!(
            "false-friend notes: {} keys, {collisions} collisions ({} high / {} medium / {} low)",
            notes.len(),
            sev("high"),
            sev("medium"),
            sev("low"),
        );
        assert!(
            notes.len() >= 100,
            "suspiciously few notes: {}",
            notes.len()
        );
        assert!(
            notes.len() <= 20_000,
            "suspiciously many notes: {}",
            notes.len()
        );
    }

    /// V11 item 1: the observed false-positive classes must not fire (or
    /// must be downgraded to colloquial/low severity).
    /// V12 item 2: exact-level collisions outrank loose in the rendered
    /// warning — pytati must quote пытать 'torture' (exact), not only the
    /// loose питать 'feed'; the machine-readable list stays complete and
    /// level-annotated.
    #[test]
    fn exact_collision_is_the_quoted_one() {
        let notes = notes();
        let pytati = notes.get("pytati").expect("pytati note");
        assert!(
            pytati.warning.contains("torture"),
            "warning must quote the exact пытать sense: {}",
            pytati.warning
        );
        let levels: Vec<(&str, &str)> = pytati
            .collisions
            .iter()
            .map(|c| (c.word.as_str(), c.level))
            .collect();
        assert!(
            levels
                .iter()
                .any(|(w, l)| w.contains("пытать") && *l == "exact"),
            "{levels:?}"
        );
        assert!(
            levels
                .iter()
                .any(|(w, l)| w.contains("питать") && *l == "loose"),
            "{levels:?}"
        );
    }

    /// V12 item 1: English polysemy must not mint prefers — the four
    /// observed bad prefers go empty or sensible, the good ones survive.
    #[test]
    fn slavic_bridge_fixes_polysemy_prefers() {
        let notes = notes();
        let prefer = |k: &str| notes.get(k).map(|n| n.prefer.clone()).unwrap_or_default();
        // Bad: must no longer emit the observed polysemy answers.
        for (key, bad) in [
            ("ravnina", "aviakarta"),
            ("skloniti", "opadati"),
            ("gojiti", "dobyti"),
        ] {
            assert!(
                !prefer(key).iter().any(|p| p == bad),
                "{key} still prefers {bad}: {:?}",
                prefer(key)
            );
        }
        // staja: the V12 brief listed komnata as a graze, but the data says
        // otherwise — bg стая genuinely means 'room' and komnata's own bg
        // cell is exactly 'стая': a DIRECT dictionary bridge, the mechanism
        // this item asked for. Accept empty or a dictionary-bridged
        // room/barn word; never an English-polysemy answer.
        let staja = prefer("staja");
        assert!(
            staja.is_empty() || matches!(staja[0].as_str(), "komnata" | "ambar" | "hlěv" | "soba"),
            "staja prefer must be bridged or empty: {staja:?}"
        );
        // Good: must still emit a sensible suggestion.
        assert!(!prefer("urok").is_empty(), "urok lost its prefer");
        assert!(!prefer("čas").is_empty(), "čas lost its prefer");
        eprintln!(
            "V12 fixtures: ravnina={:?} skloniti={:?} gojiti={:?} staja={:?} pytati={:?} čas={:?} barviti={:?}",
            prefer("ravnina"), prefer("skloniti"), prefer("gojiti"),
            prefer("staja"), prefer("pytati"), prefer("čas"), prefer("barviti"),
        );
    }

    /// V11 item 2: prefer must be POS-compatible and coverage-thresholded —
    /// the observed misleading suggestions must be gone (empty is fine;
    /// wrong is not).
    #[test]
    fn bad_prefers_are_fixed_or_empty() {
        let notes = notes();
        let not_prefers = |key: &str, bad: &str| {
            if let Some(n) = notes.get(key) {
                assert!(
                    !n.prefer.iter().any(|p| p == bad),
                    "{key} still prefers {bad}: {:?}",
                    n.prefer
                );
            }
        };
        not_prefers("staja", "stabiľny"); // adjective for a noun sense
        not_prefers("banan", "dětę"); // one grazed token of a slang sense
        not_prefers("cvrkot", "mrdati"); // verb for a noun sense
        not_prefers("kazniti", "smŕť"); // noun for a verb sense
        not_prefers("gojiti", "vaga"); // noun for a verb sense
    }

    #[test]
    fn observed_false_positives_are_fixed() {
        let notes = notes();
        // Spelling variants: 'autogyro' vs ru 'autogiro' (near-match).
        assert!(
            !notes.contains_key("avtožir"),
            "avtožir spelling-variant FP"
        );
        // Synonyms via or-lists: 'snack' vs 'appetizer' — the uk/pl appetizer
        // collisions must be gone; bg закуска 'breakfast' is a REAL trap and
        // may stay.
        if let Some(n) = notes.get("zakuska") {
            assert!(
                !n.collisions
                    .iter()
                    .any(|c| c.glosses.iter().any(|g| g.contains("appetizer"))),
                "zakuska synonym FP: {:?}",
                n.warning
            );
        }
        // Hypernym via definitional parenthetical: 'kitten (young cat)' —
        // ru котэ 'cat' is not a trap for official 'kitten'.
        assert!(!notes.contains_key("kote"), "kote hypernym FP");
        // Primary sense agrees → severity low, colloquial wording: pl banan
        // primarily means banana; the slang senses must not read as THE
        // meaning.
        if let Some(n) = notes.get("banan") {
            assert_eq!(n.severity, "low", "banan: {}", n.warning);
            assert!(
                n.warning.contains("colloquially"),
                "banan wording: {}",
                n.warning
            );
        }
        // 'chirrup'≈'chirp' near-match: the chirp record agrees, so cvrkot is
        // at most a colloquial note for the 'bustle, stir' sense.
        if let Some(n) = notes.get("cvrkot") {
            assert_eq!(n.severity, "low", "cvrkot: {}", n.warning);
        }
        // Junk glosses never appear in warnings ('YouTube poop').
        if let Some(n) = notes.get("pup") {
            assert!(
                !n.warning.contains("YouTube"),
                "pup junk gloss quoted: {}",
                n.warning
            );
        }
        // Severity sanity on the curated traps: primary-sense divergence in
        // several languages.
        assert_eq!(notes["urok"].severity, "high");
        assert_eq!(notes["čas"].severity, "high");
    }

    #[test]
    fn gloss_tokens_strip_parentheticals_and_stopwords() {
        let t = gloss_tokens("hour (unit of time: one twenty-fourth of a day)");
        assert!(t.contains("hour"));
        assert!(!t.contains("time"));
        let t = gloss_tokens("to ask");
        assert_eq!(t.into_iter().collect::<Vec<_>>(), vec!["ask"]);
    }
}
