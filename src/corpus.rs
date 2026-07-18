//! Cognate-set dictionary built from the Wiktionary Slavic-lemma corpus.
//!
//! Every inherited Slavic lemma Wiktionary links to a Proto-Slavic ancestor
//! ([`crate::dump::extract_lemmas`]); lemmas sharing an ancestor form a **cognate
//! set**. Each set becomes one Interslavic word: the Proto-Slavic rule engine
//! supplies the form from the *known* reconstruction (no linking needed), the
//! modern reflexes resolve the yers and give the surface consensus, and the
//! **confidence scales with how many languages and branches attest the set** —
//! a word seen in one language is a low-confidence guess, one seen across all
//! three branches is high-confidence.

use crate::consensus::{self, ConsensusConfig, MeaningInput, SourceForm};
use crate::dump::{LemmaCorpus, LemmaEntry};
use crate::lang::Branch;
use crate::model::{Candidate, Confidence, Pos};
use crate::normalize::{self, NormForm};
use crate::orthography as ortho;
use std::collections::BTreeMap;

/// Version of the cognate-coverage ranking formula. Bump and refit the corpus
/// calibrator whenever [`coverage_score`] changes.
pub const COVERAGE_SCORE_MODEL_VERSION: &str = "coverage-languages-branches-v1";

/// Raw corpus coverage ranking feature. This is neither a probability nor the
/// official-row pipeline candidate score; only the dedicated corpus artifact
/// may map it to a probability.
pub fn coverage_score(n_langs: usize, n_branches: usize) -> f32 {
    let lang_term = (n_langs.min(8) as f32) / 8.0;
    let branch_term = (n_branches.min(3) as f32) / 3.0;
    (0.12 + 0.55 * lang_term + 0.33 * branch_term).clamp(0.05, 0.99)
}

/// A group of etymologically-connected modern lemmas — either a shared
/// Proto-Slavic root (inherited) or a shared non-Slavic source (borrowing).
#[derive(Debug, Clone)]
pub struct CognateSet {
    /// Group key: `*orvьnъ` (inherited) or `bor:<skeleton>` (borrowing).
    pub proto: String,
    /// Display ancestor: `*orvьnъ` or `la computare`.
    pub etymon: String,
    pub borrowed: bool,
    pub pos: Pos,
    pub gloss: String,
    pub members: Vec<LemmaEntry>,
}

/// One generated Interslavic word plus its supporting cognate set.
pub struct GeneratedWord {
    pub set: CognateSet,
    pub candidates: Vec<Candidate>,
    pub confidence: Confidence,
    pub score: f32,
    pub n_langs: usize,
    pub n_branches: usize,
    pub reconstruction: Option<crate::model::Reconstruction>,
}

impl GeneratedWord {
    pub fn form(&self) -> &str {
        self.candidates
            .first()
            .map(|c| c.form.as_str())
            .unwrap_or("")
    }
}

/// Branch of a Slavic language, including the smaller lects the corpus carries.
/// Delegates to the single [`crate::lang::LANGS`] registry — this used to be a
/// second hand-coded table that had already drifted from it (it dropped `orv`,
/// which `lang.rs` carries as East/non-modern, the same etymological-hint role
/// as `cu`).
pub fn branch_of(lang: &str) -> Option<Branch> {
    crate::lang::branch_of(lang)
}

fn pos_class(pos: &str) -> &'static str {
    match pos {
        "noun" | "proper_noun" => "n",
        "verb" => "v",
        "adj" => "a",
        "adv" => "adv",
        "pron" => "pron",
        "num" => "num",
        "prep" => "prep",
        "conj" => "conj",
        _ => "x",
    }
}

/// Group the corpus into cognate sets: inherited lemmas by their Proto-Slavic
/// ancestor; borrowings by **union-find** over two signals — the phonemic
/// skeleton of the Slavic form *and* the skeleton of the (Latin-script) source
/// etymon. Merging on the shared etymon connects variants the Slavic-form
/// skeleton alone splits (avtomobil ≍ automobil via the Latin `automobile`),
/// lifting internationalisms out of the low-confidence singleton tail.
pub fn build_sets(corpus: &LemmaCorpus) -> Vec<CognateSet> {
    let mut inherited: BTreeMap<(String, &'static str), Vec<LemmaEntry>> = BTreeMap::new();
    let mut borrowed: Vec<(&LemmaEntry, String, &'static str)> = Vec::new(); // (lemma, slav_key, pos_class)
    let mut uf = UnionFind::default();
    // Fold-identity bridge edges (issue #86 item 5a), staged and applied
    // AFTER the S:/E: layer so the component delta (= real merges the bridge
    // adds) is measured and reported.
    let mut fold_edges: Vec<(String, String)> = Vec::new();
    // Non-Latin (Greek/Cyrillic) etymons that only key via transliteration
    // (issue #86 item 5b) — counted for the export report.
    let mut translit_keyed = 0usize;

    for e in &corpus.entries {
        if branch_of(&e.lang).is_none() {
            continue;
        }
        // Chain-attested lemmas (issue #86): an old-stage inheritance chain
        // that resolved to neither a proto nor a foreign etymon (schema-1
        // caches only — see dump::LEMMA_CACHE_SCHEMA). They carry a real
        // attestation, so they join the borrowing skeleton layer via their
        // S:/F: nodes (no E: edge is possible) instead of vanishing.
        let chain_attested = e.proto.is_empty() && e.etymon.is_empty();
        if e.is_borrowed() || chain_attested {
            let latin = normalize::to_phonemic_latin(&e.lang, &e.word);
            let sk = intl_key(&latin);
            let pc = pos_class(&e.pos);
            // A 2-consonant skeleton (rn, rm, pp) collides unrelated short words
            // (urna≠arena≠ajran), so cluster short words by their full normalized
            // form instead; only ≥3-consonant skeletons cluster by skeleton.
            let node_key = if sk.chars().count() >= 3 {
                sk
            } else {
                let full: String = latin.chars().filter(|c| c.is_alphanumeric()).collect();
                if full.chars().count() < 2 {
                    continue;
                }
                format!("w:{full}")
            };
            let snode = format!("S:{node_key}/{pc}");
            uf.touch(&snode);
            // Merge on the shared etymon only for a substantial skeleton
            // (≥4), since this edge is transitive and short etymons
            // over-connect. Greek/Cyrillic source words key via
            // transliteration (issue #86: `grc ἀλόη` ≡ `la aloē` → E:aloe).
            if let Some(ek) = etymon_key(&e.etymon) {
                if e.etymon
                    .chars()
                    .any(|c| c.is_alphabetic() && (c as u32) >= 0x370)
                {
                    translit_keyed += 1;
                }
                uf.union(&snode, &format!("E:{ek}/{pc}"));
            }
            // Fold-identity bridge (issue #86 item 5a): identical full
            // phonemic-Latin folds merge across languages regardless of
            // etymon script, gated at ≥4 chars; every node in this layer is
            // borrowing-class by construction. The S: key above is
            // `intl_key(latin)` over the RAW latin — which keeps
            // non-alphanumeric characters (apostrophes, hyphens) that this
            // fold filters out, so the bridge merges exactly the pairs
            // punctuation splits (ru курье́р "kur'jer" ≡ uk кур'єр "kurjer";
            // measured: 6 component merges on the 2026-07 corpus, reported
            // by the stats line below).
            let fold: String = latin.chars().filter(|c| c.is_alphanumeric()).collect();
            if fold.chars().count() >= 4 {
                fold_edges.push((snode.clone(), format!("F:{fold}/{pc}")));
            }
            borrowed.push((e, snode, pc));
        } else if !e.proto.is_empty() {
            // Skip placeholder / bound-morpheme ancestors (B9): they are not roots,
            // so clustering by them fuses unrelated lemmas. (Also filtered at
            // extraction; this guards older caches.)
            let root = e.proto.trim_start_matches('*');
            if root.is_empty()
                || root.starts_with('-')
                || root.ends_with('-')
                || !root.chars().any(|c| c.is_alphabetic())
            {
                continue;
            }
            // Key inherited sets by a *normalized* reconstruction so pure notation
            // variants of the same proto merge into one cognate set: stress-accent
            // variants (*bьràti ≡ *bьrati, *bràtrъ ≡ *bratrъ) and optional-segment
            // notation (*(j)edinъ ≡ *edinъ). POS still gates, so a real homograph
            // (num *edinъ vs adj *edinъ "same") stays split. Merging is safe here —
            // build_sets feeds only the site, never the leakage-free benchmark.
            inherited
                .entry((proto_merge_key(&e.proto), pos_class(&e.pos)))
                .or_default()
                .push(e.clone());
        }
    }

    // Apply the fold-identity bridge and measure what it actually merged:
    // distinct borrowing-layer components before vs after (issue #86).
    let component_count = |uf: &mut UnionFind, nodes: &[(&LemmaEntry, String, &'static str)]| {
        let mut roots = std::collections::BTreeSet::new();
        for (_, snode, _) in nodes {
            roots.insert(uf.find(snode));
        }
        roots.len()
    };
    let comps_before = component_count(&mut uf, &borrowed);
    // A fold's FIRST edge only attaches the fresh F: node; a real
    // cross-component merge is an edge to an already-seen F: node that still
    // returned true — those folds are the bridge's actual work (sampled for
    // the stats line).
    let mut seen_f: std::collections::HashSet<&String> = std::collections::HashSet::new();
    let mut bridge_folds: Vec<&str> = Vec::new();
    for (a, b) in &fold_edges {
        let known_f = !seen_f.insert(b);
        if uf.union(a, b) && known_f && bridge_folds.len() < 10 {
            bridge_folds.push(b.trim_start_matches("F:"));
        }
    }
    let comps_after = component_count(&mut uf, &borrowed);

    let mut sets = Vec::new();
    for ((_key, _), members) in inherited {
        // Display the most common original reconstruction among the merged members.
        let proto = most_common_proto(&members);
        if let Some(set) = finish_set(proto.clone(), proto, false, members) {
            sets.push(set);
        }
    }

    // Assemble borrowed cognate sets from the union-find components.
    let mut comps: BTreeMap<String, Vec<LemmaEntry>> = BTreeMap::new();
    for (e, snode, _) in &borrowed {
        comps.entry(uf.find(snode)).or_default().push((*e).clone());
    }
    let largest = comps.values().map(Vec::len).max().unwrap_or(0);
    println!(
        "borrowing layer: {} lemmas → {} components (fold-identity bridge merged {}: {} → {}{}), {} non-Latin etymons keyed via transliteration, largest component {} members (issue #86)",
        borrowed.len(),
        comps_after,
        comps_before - comps_after,
        comps_before,
        comps_after,
        if bridge_folds.is_empty() {
            String::new()
        } else {
            format!("; folds: {}", bridge_folds.join(", "))
        },
        translit_keyed,
        largest,
    );
    for (root, members) in comps {
        let etymon = most_common_etymon(&members);
        if let Some(set) = finish_set(format!("bor:{root}"), etymon, true, members) {
            sets.push(set);
        }
    }
    sets
}

/// The etymon's skeleton as a merge key. Latin-script source words key
/// directly; Greek and Cyrillic ones transliterate first (issue #86 item 5b:
/// `grc ἀλόη` and `la aloē` must produce the same E: key), so a Greek-script
/// etymon can finally connect the languages that cite it. Other scripts
/// (Arabic, Georgian, …) still return None and those borrowings merge on the
/// Slavic-form skeleton alone. The ≥4-char gate stays — the edge is
/// transitive and short etymons over-connect.
fn etymon_key(etymon: &str) -> Option<String> {
    let (src, word) = match etymon.split_once(' ') {
        Some((src, w)) => (src, w.trim()),
        None => ("", etymon.trim()),
    };
    if word.is_empty() {
        return None;
    }
    let word = if word.chars().any(normalize::is_greek_char) {
        normalize::translit_greek(word)
    } else if word.chars().any(normalize::is_cyrillic_char) {
        normalize::to_phonemic_latin(src, word)
    } else {
        word.to_string()
    };
    if word
        .chars()
        .any(|c| c.is_alphabetic() && (c as u32) >= 0x250)
    {
        return None;
    }
    let key = intl_key(&word);
    if key.chars().count() < 4 {
        None
    } else {
        Some(key)
    }
}

/// Minimal union-find over string node ids (path-halving).
#[derive(Default)]
struct UnionFind {
    parent: std::collections::HashMap<String, String>,
}
impl UnionFind {
    fn touch(&mut self, x: &str) {
        self.parent
            .entry(x.to_string())
            .or_insert_with(|| x.to_string());
    }
    fn find(&mut self, x: &str) -> String {
        let mut cur = x.to_string();
        loop {
            let p = self
                .parent
                .entry(cur.clone())
                .or_insert_with(|| cur.clone())
                .clone();
            if p == cur {
                return cur;
            }
            let gp = self.parent.get(&p).cloned().unwrap_or_else(|| p.clone());
            self.parent.insert(cur.clone(), gp.clone());
            cur = gp;
        }
    }
    /// Union two nodes; true when they were in DIFFERENT components (a real
    /// merge happened, not a no-op).
    fn union(&mut self, a: &str, b: &str) -> bool {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent.insert(ra, rb);
            return true;
        }
        false
    }
}

fn finish_set(
    proto: String,
    etymon: String,
    borrowed: bool,
    mut members: Vec<LemmaEntry>,
) -> Option<CognateSet> {
    members.sort_by(|a, b| (&a.lang, &a.word).cmp(&(&b.lang, &b.word)));
    members.dedup_by(|a, b| a.lang == b.lang && a.word == b.word);
    if members.is_empty() {
        return None;
    }
    let pos = most_common_pos(&members);
    let gloss = representative_gloss(&members);
    Some(CognateSet {
        proto,
        etymon,
        borrowed,
        pos,
        gloss,
        members,
    })
}

/// The skeleton used to cluster internationalisms across languages: the
/// diacritic-folding [`ortho::ascii_skeleton`] (which PRESERVES vowels and
/// does NOT fold c→k) minus the inconsistent glide `j` (kompjuter ≍
/// komputer). Cross-shape adaptations that differ in a vowel or ending (aloe
/// ≠ aloes ≠ aloa) therefore stay separate nodes and only merge through a
/// shared etymon edge. (The comment here used to claim vowel-dropping and
/// c→k folding — that describes `ortho::consonant_key`, not this key;
/// issue #86 item 3.)
fn intl_key(latin: &str) -> String {
    ortho::ascii_skeleton(latin).replace('j', "")
}

/// A normalized reconstruction key that collapses pure notation variants of the
/// same Proto-Slavic form: drops the `*`, any parenthesized *optional* segment
/// (`*(j)edinъ`→`edinъ`), and stress accents (`*bьràti`→`bьrati`), while keeping
/// the etymological letters (ě ę ǫ ъ ь ȯ y) that actually distinguish a
/// reconstruction. Two reconstructions differing only by stress or an optional
/// segment are the same word, so this never fuses distinct roots.
fn proto_merge_key(proto: &str) -> String {
    let s = proto.trim().trim_start_matches('*');
    let mut out = String::with_capacity(s.len());
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth > 0 => {}
            _ => out.push(debase_stress(c)),
        }
    }
    out
}

/// Strip a stress-accented base vowel to its plain base; leave etymological
/// letters untouched (mirrors the reconstruction-cleaning in the proto engine).
fn debase_stress(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ā' | 'ǎ' | 'ȁ' | 'ȃ' => 'a',
        'è' | 'é' | 'ê' | 'ẽ' | 'ē' | 'ȅ' | 'ȇ' => 'e',
        'ì' | 'í' | 'î' | 'ĩ' | 'ī' | 'ȉ' | 'ȋ' => 'i',
        'ò' | 'ó' | 'ô' | 'õ' | 'ō' | 'ȍ' | 'ȏ' => 'o',
        'ù' | 'ú' | 'û' | 'ũ' | 'ū' | 'ȕ' | 'ȗ' => 'u',
        'ý' | 'ỳ' | 'ŷ' | 'ȳ' => 'y',
        other => other,
    }
}

/// The most common original reconstruction among merged members (for display).
fn most_common_proto(members: &[LemmaEntry]) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for m in members {
        if !m.proto.is_empty() {
            *counts.entry(m.proto.as_str()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(p, _)| p.to_string())
        .unwrap_or_default()
}

fn most_common_etymon(members: &[LemmaEntry]) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for m in members {
        if !m.etymon.is_empty() {
            *counts.entry(m.etymon.as_str()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(e, _)| e.to_string())
        .unwrap_or_default()
}

fn most_common_pos(members: &[LemmaEntry]) -> Pos {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for m in members {
        *counts.entry(m.pos.as_str()).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(p, _)| Pos::parse(p))
        .unwrap_or(Pos::Other)
}

/// The gloss shared by the most members (the cognate-set's meaning), preferring a
/// major reference language on ties.
fn representative_gloss(members: &[LemmaEntry]) -> String {
    const PREF: &[&str] = &["ru", "pl", "cs", "uk", "sl", "bg"];
    let mut counts: BTreeMap<&str, (usize, i32)> = BTreeMap::new();
    for m in members {
        let g = m.gloss.trim();
        if g.is_empty() {
            continue;
        }
        let pref = PREF.iter().position(|l| *l == m.lang).map(|p| -(p as i32));
        let e = counts.entry(g).or_insert((0, pref.unwrap_or(-99)));
        e.0 += 1;
        if let Some(p) = pref {
            e.1 = e.1.max(p);
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, (n, pref))| (*n, *pref))
        .map(|(g, _)| g.to_string())
        .unwrap_or_default()
}

/// Generate the Interslavic word for a cognate set.
pub fn generate_set(set: CognateSet, cfg: &ConsensusConfig) -> GeneratedWord {
    // One primary source form per language (extra senses become secondary).
    let mut forms: Vec<SourceForm> = Vec::new();
    let mut seen_lang: BTreeMap<&str, bool> = BTreeMap::new();
    for m in &set.members {
        // Branch AND the modern-voter flag both come from the single lang.rs
        // registry (a local `!= "cu"` here used to mislabel other non-modern
        // hint languages, e.g. `orv`, as voters).
        let Some(info) = crate::lang::lang_info(&m.lang) else {
            continue;
        };
        let latin = normalize::to_phonemic_latin(&m.lang, &m.word);
        let skeleton = ortho::ascii_skeleton(&latin);
        if skeleton.is_empty() {
            continue;
        }
        let first = !seen_lang.contains_key(m.lang.as_str());
        seen_lang.insert(m.lang.as_str(), true);
        forms.push(SourceForm {
            lang_code: m.lang.clone(),
            branch: info.branch,
            modern: info.modern,
            norm: NormForm {
                original: m.word.clone(),
                latin,
                skeleton,
                flagged: false,
            },
            source_url: crate::enrich::english_source_url(&m.word, Some(&m.lang)),
            primary: first,
        });
    }

    let (forms, reflexive) = consensus::strip_reflexive(forms, set.pos);
    let input = MeaningInput {
        pos: set.pos,
        gender: None,
        gloss: set.gloss.clone(),
        forms,
        // Borrowings are internationalisms: trigger the -cija/-izm/-ist ending
        // normalization and the international-cluster preference.
        is_intl_meaning: set.borrowed,
        reflexive,
    };
    // Historical lects such as OCS are etymological hints, not modern
    // attestations. Consensus already excludes them from voting; keep the same
    // boundary for candidate support, displayed evidence counts and confidence.
    let mut modern_langs: Vec<String> = input
        .forms
        .iter()
        .filter(|f| f.modern)
        .map(|f| f.lang_code.clone())
        .collect();
    modern_langs.sort();
    modern_langs.dedup();
    let mut modern_branches = Vec::new();
    for f in input.forms.iter().filter(|f| f.modern) {
        if !modern_branches.contains(&f.branch) {
            modern_branches.push(f.branch);
        }
    }

    // Cross-branch consensus surface + alternatives.
    let mut candidates = consensus::generate(&input, cfg);

    // Inherited words get their authoritative form from the *known* Proto-Slavic
    // ancestor; borrowings have no reconstruction and rely on the consensus.
    let mut reconstruction = None;
    if !set.borrowed {
        let reflexes: Vec<String> = input
            .forms
            .iter()
            .filter(|f| f.modern && f.primary)
            .map(|f| f.norm.latin.clone())
            .collect();
        let proto_word = set.proto.trim_start_matches('*');
        // stem_class stays None on the site path: site output must not change
        // until the display side ships its own readers (issue #76).
        let mut pc =
            crate::proto::generate_with_reflexes(proto_word, set.pos, None, &reflexes, None);
        if reflexive && !pc.form.is_empty() && !pc.form.ends_with(" sę") {
            pc.form.push_str(" sę");
        }
        if !pc.form.is_empty() {
            pc.trace.insert(
                0,
                crate::model::RuleStep::new(
                    "proto-ancestor",
                    set.proto.clone(),
                    pc.form.clone(),
                    format!(
                        "Praslovjanska rekonstrukcija {} (dana etimologijeju Wiktionary).",
                        set.proto
                    ),
                    Some("https://interslavic.fun/learn/orthography/"),
                ),
            );
            reconstruction = Some(crate::model::Reconstruction {
                word: proto_word.to_string(),
                proto_balto_slavic: String::new(),
                proto_indo_european: String::new(),
                confidence: 1.0,
            });
            // The reconstruction is authoritative for the form; place it first.
            pc.score = 0.99;
            // Supported by the modern members of the cognate set (issue #79
            // razumlivost). Historical hints can shape etymology, but do not
            // claim present-day speaker coverage.
            pc.langs = modern_langs.clone();
            candidates.insert(0, pc);
        }
    }

    // Dedupe by standard spelling, keeping the proto-derived (flavored) form.
    dedupe(&mut candidates);

    // Confidence scales with cognate coverage (the core of the design).
    let n_langs = modern_langs.len();
    let n_branches = modern_branches.len();
    let (confidence, score) = coverage_confidence(n_langs, n_branches);
    if let Some(top) = candidates.first_mut() {
        top.confidence = confidence;
        top.score = score;
        top.branch_coverage = n_branches as u8;
    }
    // The headword's coverage score must dominate its alternatives, otherwise the
    // displayed ranking is non-monotone (an alternative outscoring the headword).
    for c in candidates.iter_mut().skip(1) {
        if c.score >= score {
            c.score = (score - 0.01).max(0.01);
            c.confidence = Confidence::from_score(c.score);
        }
    }

    GeneratedWord {
        set,
        candidates,
        confidence,
        score,
        n_langs,
        n_branches,
        reconstruction,
    }
}

/// The confidence model the user asked for: more attesting languages / branches →
/// higher confidence. A single-language guess is Low; a word spread across the
/// branches is High.
fn coverage_confidence(n_langs: usize, n_branches: usize) -> (Confidence, f32) {
    let score = coverage_score(n_langs, n_branches);
    let confidence = if n_langs >= 5 && n_branches >= 2 {
        Confidence::High
    } else if n_langs >= 3 && n_branches >= 2 {
        Confidence::Medium
    } else {
        Confidence::Low
    };
    (confidence, score)
}

fn dedupe(candidates: &mut Vec<Candidate>) {
    use crate::model::CandidateSource;
    candidates.sort_by(|a, b| {
        b.score.total_cmp(&a.score).then(
            ((b.source == CandidateSource::ProtoSlavicRule) as u8)
                .cmp(&((a.source == CandidateSource::ProtoSlavicRule) as u8)),
        )
    });
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<Candidate> = Vec::new();
    for c in candidates.drain(..) {
        let key = ortho::to_standard(&c.form.to_lowercase());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(c);
    }
    *candidates = out;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dump::{LemmaCorpus, LemmaEntry};

    fn le(lang: &str, word: &str, pos: &str, proto: &str, etymon: &str) -> LemmaEntry {
        LemmaEntry {
            lang: lang.into(),
            word: word.into(),
            pos: pos.into(),
            gloss: "x".into(),
            proto: proto.into(),
            etymon: etymon.into(),
            etymology: Vec::new(),
            categories: Vec::new(),
            topics: Vec::new(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn coverage_confidence_is_monotone_and_formula_is_pinned() {
        let mut prev = -1.0;
        for branches in 1..=3 {
            for nl in 1..=8 {
                let (_, s) = coverage_confidence(nl, branches);
                assert!(
                    s >= prev || branches > 1,
                    "score not monotone at {nl} langs"
                );
                assert!(coverage_score(nl + 1, branches) >= s);
                assert!(coverage_score(nl, branches + 1) >= s);
                prev = s;
            }
        }
        assert!((coverage_score(1, 1) - 0.29875).abs() < 1e-6);
        assert!((coverage_score(3, 2) - 0.546_25).abs() < 1e-6);
        assert!((coverage_score(8, 3) - 0.99).abs() < 1e-6);
        assert!(matches!(coverage_confidence(6, 3).0, Confidence::High));
        assert!(matches!(coverage_confidence(3, 2).0, Confidence::Medium));
        assert!(matches!(coverage_confidence(1, 1).0, Confidence::Low));
    }

    #[test]
    fn historical_hints_do_not_inflate_modern_coverage() {
        let modern_only = CognateSet {
            proto: "*voda".into(),
            etymon: "*voda".into(),
            borrowed: false,
            pos: Pos::Noun,
            gloss: "water".into(),
            members: vec![le("ru", "вода", "noun", "*voda", "")],
        };
        let base = generate_set(modern_only.clone(), &ConsensusConfig::production());
        assert_eq!((base.n_langs, base.n_branches), (1, 1));
        for hint in ["cu", "orv"] {
            let mut with_hint = modern_only.clone();
            with_hint
                .members
                .push(le(hint, "вода", "noun", "*voda", ""));
            let hinted = generate_set(with_hint, &ConsensusConfig::production());
            assert_eq!(
                (hinted.n_langs, hinted.n_branches, hinted.score),
                (base.n_langs, base.n_branches, base.score),
                "historical hint {hint} inflated coverage"
            );
            assert_eq!(hinted.candidates[0].langs, ["ru"]);
        }
    }

    #[test]
    fn headword_outscores_its_alternatives() {
        let members = vec![
            le("ru", "вода", "noun", "*voda", ""),
            le("pl", "woda", "noun", "*voda", ""),
            le("cs", "voda", "noun", "*voda", ""),
        ];
        let set = CognateSet {
            proto: "*voda".into(),
            etymon: "*voda".into(),
            borrowed: false,
            pos: Pos::Noun,
            gloss: "water".into(),
            members,
        };
        let g = generate_set(set, &ConsensusConfig::production());
        let top = g.candidates[0].score;
        assert!(
            g.candidates.iter().skip(1).all(|c| c.score <= top),
            "an alternative outscores the headword: {:?}",
            g.candidates.iter().map(|c| c.score).collect::<Vec<_>>()
        );
    }

    #[test]
    fn build_sets_skips_placeholder_and_bound_proto() {
        let corpus = LemmaCorpus {
            schema: crate::dump::LEMMA_CACHE_SCHEMA,
            source: String::new(),
            entry_count: 3,
            entries: vec![
                le("ru", "x", "noun", "*-", ""),
                le("pl", "y", "noun", "*per-", ""),
                le("cs", "voda", "noun", "*voda", ""),
            ],
        };
        let sets = build_sets(&corpus);
        assert!(
            sets.iter()
                .all(|s| !s.proto.starts_with("*-") && !s.proto.ends_with('-')),
            "a placeholder/bound-morpheme ancestor formed a cognate set"
        );
        assert_eq!(sets.iter().filter(|s| !s.borrowed).count(), 1); // only *voda
    }

    /// Issue #66: en.wiktionary files sh lemmas in EITHER script; a Cyrillic
    /// sh member must normalize to Latin, and no candidate surface may carry
    /// Cyrillic letters.
    #[test]
    fn cyrillic_sh_members_normalize_to_latin() {
        let set = CognateSet {
            proto: String::new(),
            etymon: "ota بخشش".into(),
            borrowed: true,
            pos: Pos::Noun,
            gloss: "baksheesh".into(),
            members: vec![
                le("bg", "бакшиш", "noun", "", "ota بخشش"),
                le("mk", "бакшиш", "noun", "", "ota بخشش"),
                le("ru", "бакшиш", "noun", "", "tr bahşiş"),
                le("sh", "бакшиш", "noun", "", "ota بخشش"),
            ],
        };
        let g = generate_set(set, &ConsensusConfig::production());
        assert_eq!(g.form(), "bakšiš");
        for c in &g.candidates {
            assert!(
                !c.form.chars().any(crate::normalize::is_cyrillic_char),
                "cyrillic leaked into candidate: {}",
                c.form
            );
        }
    }

    #[test]
    fn intl_key_ignores_the_j_glide() {
        // kompjuter and komputer must share an internationalism key.
        assert_eq!(intl_key("kompjuter"), intl_key("komputer"));
    }

    /// Issue #86 item 5b: Greek and Cyrillic source words transliterate
    /// before the Latin-script check, so `grc ἀλόη` and `la aloē` produce
    /// the SAME E: key; scripts with no Latin alignment still return None,
    /// and the ≥4-char gate holds after transliteration.
    #[test]
    fn greek_and_cyrillic_etymons_key_like_latin() {
        assert_eq!(etymon_key("grc ἀλόη"), Some("aloe".to_string()));
        assert_eq!(etymon_key("la aloē"), Some("aloe".to_string()));
        assert_eq!(etymon_key("grc ᾰ̓λόη"), etymon_key("la aloē")); // polytonic
        assert_eq!(etymon_key("ru спутник"), Some("sputnik".to_string()));
        assert_eq!(etymon_key("ota بخشش"), None); // Arabic script: no key
        assert_eq!(etymon_key("grc ὥρα"), None); // 3 letters after translit: gated
        assert_eq!(etymon_key("la aloe"), Some("aloe".to_string())); // unchanged
    }

    /// Issue #86: proto-less etymon-less CHAIN lemmas (old-stage inheritance
    /// that resolved to nothing; schema-1 caches only) join the borrowing
    /// skeleton layer and group with fold-identical borrowings across
    /// languages — instead of silently vanishing from the site.
    #[test]
    fn chain_attested_lemmas_group_in_the_borrowing_layer() {
        let corpus = LemmaCorpus {
            schema: crate::dump::LEMMA_CACHE_SCHEMA,
            source: String::new(),
            entry_count: 3,
            entries: vec![
                le("cs", "aloe", "noun", "", ""),           // unresolved chain lemma
                le("ru", "алоэ", "noun", "", "grc ἀλόη"),   // borrowing, Greek etymon
                le("pl", "aloes", "noun", "", "frm aloès"), // different fold → own set
            ],
        };
        let sets = build_sets(&corpus);
        let borrowed: Vec<_> = sets.iter().filter(|s| s.borrowed).collect();
        assert_eq!(borrowed.len(), 2, "{borrowed:?}");
        let merged = borrowed
            .iter()
            .find(|s| s.members.len() == 2)
            .expect("cs aloe ≡ ru алоэ should share one set");
        let langs: Vec<_> = merged.members.iter().map(|m| m.lang.as_str()).collect();
        assert_eq!(langs, ["cs", "ru"]);
        // The merged set displays the etymon its borrowing member carries.
        assert_eq!(merged.etymon, "grc ἀλόη");
    }
}
