use super::coverage::{
    inject_generated_derivatives, insert_official_byform_aliases, official_surface_maps,
    plan_raw_pages, raw_lemma_fate, select_official_entry, select_official_surface, RawFate,
};
use super::entries::{cognate_block, entry_page, noun_table, word_chip};
use super::layout::json_str;
use super::model::{
    format_source_date_epoch, proto_stem, razum_pct, BuildMeta, HeadwordIndex, SiteEntryInput,
    SiteEntryMeta,
};
use super::navigation::{
    branch_pattern, entries_json, entry_infobox, entry_meta, needs_review, provenance_block,
    raw_credit_line, union_razum_codes,
};
use super::search::{
    client_fold, collect_source_aliases, conf_letter, keys_json, search_bucket_pair, search_keys,
    search_row_buckets, shard_file_name, source_aliases_json, write_search_index, SearchRow,
    SourceAlias,
};
use super::special::{
    build_proto_reflex_index, fold_proto_accents, forms_page, rule_engine, rule_file_stem,
    rule_key, text_check_page,
};
use super::DeterministicEntryIds;
use crate::consensus::ConsensusConfig;
use crate::model::{Candidate, CandidateSource, Confidence, MatchStatus, Pos};
use interslavic::{
    Animacy as IsvAnimacy, Case as IsvCase, Gender as IsvGender, Number as IsvNumber, ISV,
};
use std::path::Path;

#[test]
fn branch_pattern_renders_the_seven_combinations_canonically() {
    // Issue #73c: the pattern is the exact branch SET (via branch_of), in
    // the fixed V→Z→J order, independent of input order; codes outside
    // the registry drop out; an empty/unresolvable set yields None.
    let l = |codes: &[&str]| -> Vec<String> { codes.iter().map(|s| s.to_string()).collect() };
    assert_eq!(branch_pattern(&l(&["ru"])).as_deref(), Some("V"));
    assert_eq!(branch_pattern(&l(&["pl", "cs"])).as_deref(), Some("Z"));
    assert_eq!(branch_pattern(&l(&["sh"])).as_deref(), Some("J"));
    assert_eq!(branch_pattern(&l(&["ru", "pl"])).as_deref(), Some("V+Z"));
    assert_eq!(branch_pattern(&l(&["uk", "bg"])).as_deref(), Some("V+J"));
    assert_eq!(branch_pattern(&l(&["sk", "mk"])).as_deref(), Some("Z+J"));
    assert_eq!(
        branch_pattern(&l(&["ru", "pl", "sl"])).as_deref(),
        Some("V+Z+J")
    );
    // Canonical order regardless of input order; unknown codes ignored.
    assert_eq!(
        branch_pattern(&l(&["bg", "xx", "ru"])).as_deref(),
        Some("V+J")
    );
    assert_eq!(branch_pattern(&l(&["xx"])), None);
    assert_eq!(branch_pattern(&[]), None);
}

#[test]
fn dictionary_seeded_banner_uses_sanitized_official_byform() {
    let entry = crate::official::OfficialEntry {
        id: "synthetic".to_string(),
        isv: "foo, bar".to_string(),
        addition: String::new(),
        pos_raw: "adj.".to_string(),
        pos: Pos::Adjective,
        noun_traits: crate::model::NounTraits::default(),
        english: "sample gloss".to_string(),
        same_in: String::new(),
        genesis: String::new(),
        cells: std::collections::HashMap::new(),
        frequency: None,
        de: String::new(),
        nl: String::new(),
        eo: String::new(),
        intelligibility: String::new(),
        using_example: String::new(),
    };
    let mut candidate = Candidate::new("bar".to_string(), CandidateSource::BranchConsensus, 0.9);
    candidate.confidence = Confidence::High;
    let generation = crate::generator::Generation {
        candidates: vec![candidate],
        official: Some("bar".to_string()),
        match_status: MatchStatus::OfficialMatch,
        overridden: false,
        reconstruction: None,
    };

    let html = entry_page(1, &entry, &generation, &[], None);
    assert!(html.contains("oficialnomu slovniku: <span class='mention'>bar</span>"));
    assert!(!html.contains("foo, bar"));
}

#[test]
fn rule_keys_disambiguate_engines_and_map_to_safe_files() {
    // Issue #73a: "liquid-metathesis" exists in BOTH engines, so the
    // machine key and the page file must carry the engine tag.
    assert_eq!(
        rule_key("proto", "liquid-metathesis"),
        "proto:liquid-metathesis"
    );
    assert_eq!(
        rule_key("konsensus", "liquid-metathesis"),
        "konsensus:liquid-metathesis"
    );
    assert_ne!(
        rule_file_stem("proto", "liquid-metathesis"),
        rule_file_stem("konsensus", "liquid-metathesis")
    );
    assert_eq!(
        rule_file_stem("proto", "liquid-metathesis"),
        "proto-liquid-metathesis"
    );
    // A hostile id cannot escape the rule/ directory.
    assert_eq!(rule_file_stem("proto", "../x"), "proto-x");
    // Engine derives from the candidate source exactly like the
    // benchmark's is_proto flag (eval::stage_of_step).
    assert_eq!(rule_engine(CandidateSource::ProtoSlavicRule), "proto");
    assert_eq!(rule_engine(CandidateSource::BranchConsensus), "konsensus");
    assert_eq!(
        rule_engine(CandidateSource::BorrowingInternationalism),
        "konsensus"
    );
}

#[test]
fn proto_reflex_join_folds_accents_and_attributes_by_membership() {
    // Issue #73b (review-hardened). The fold must strip BOTH combining
    // marks (*pę̑tь) and precomposed accented vowels (*vodà) while
    // keeping etymological letters (ě ≠ e, č ≠ c) apart.
    assert_eq!(
        fold_proto_accents("p\u{119}\u{311}t\u{44c}"),
        "p\u{119}t\u{44c}"
    );
    assert_eq!(fold_proto_accents("vod\u{e0}"), "voda"); // à precomposed
    assert_eq!(fold_proto_accents("\u{f2}ko"), "oko"); // ò precomposed
    assert_ne!(fold_proto_accents("cělo"), fold_proto_accents("čelo"));

    let pe = |word: &str, gloss: &str| crate::dump::ProtoEntry {
        word: word.to_string(),
        pos: "noun".to_string(),
        glosses: vec![gloss.to_string()],
        descendants: vec![("ru".to_string(), "x".to_string())],
        pbs: String::new(),
        pie: String::new(),
        stem_class: None,
    };
    // Cache: homonymous *voda ×2 + the cělo/čelo slug-collision pair.
    let pi = crate::dump::ProtoIndex::build(vec![
        pe("voda", "water"),
        pe("voda", "leash (homonym)"),
        pe("cělo", "whole"),
        pe("čelo", "forehead"),
    ]);
    let set = |proto: &str| crate::corpus::CognateSet {
        proto: proto.to_string(),
        etymon: proto.to_string(),
        borrowed: false,
        pos: crate::model::Pos::Noun,
        gloss: String::new(),
        members: Vec::new(),
    };
    let sets = [
        (1usize, set("*vod\u{e0}")), // precomposed accent joins "voda"
        (2usize, set("*cělo")),
        (3usize, set("*čelo")),
        (4usize, set("*neznajemo")), // honest miss
    ];
    let index = build_proto_reflex_index(Some(&pi), sets.iter().map(|(id, s)| (*id, s)));
    assert_eq!((index.linked, index.misses), (3, 1));
    // Homonyms: the voda page lists BOTH cache entries.
    let voda_slug = index.membership.get(&1).expect("voda membership");
    let voda = index.pages.get(voda_slug).unwrap();
    assert_eq!(voda.recons, vec![0, 1], "homonyms share the page");
    assert_eq!(voda.entry_ids, vec![1]);
    // Slug collision: cělo and čelo both slug to "celo" but are DIFFERENT
    // lexemes → two pages with a deterministic suffix, and membership
    // sends each entry to ITS OWN lexeme's page (entry 679 class bug).
    let celo_slug = index.membership.get(&2).unwrap();
    let chelo_slug = index.membership.get(&3).unwrap();
    assert_ne!(celo_slug, chelo_slug, "cělo vs čelo must not share a page");
    assert_eq!(celo_slug, "celo");
    assert_eq!(chelo_slug, "celo-2");
    assert_eq!(index.pages[celo_slug].recons, vec![2]);
    assert_eq!(index.pages[chelo_slug].recons, vec![3]);
    // The membership invariant the link audit checks: every membership
    // target page lists the member.
    for (id, sl) in &index.membership {
        assert!(
            index.pages[sl].entry_ids.contains(id),
            "membership target must list entry {id}"
        );
    }
    // A borrowed set or a non-'*' ancestor is never looked up.
    let mut borrowed = set("*voda");
    borrowed.borrowed = true;
    let i2 = build_proto_reflex_index(Some(&pi), std::iter::once((8usize, &borrowed)));
    assert!(i2.pages.is_empty() && i2.linked == 0 && i2.misses == 0);
}

#[test]
fn generated_derivatives_never_collide_and_are_all_generated() {
    // Issue #37 invariants: injected derivatives are pure ADDITIONS that (a)
    // never collide with an official / official-only form_key (incl. INFLECTED
    // forms), (b) are all status="generated", source="lemma", probability set,
    // provenance-tagged, and (c) never re-emit a form the dictionary already has.
    use crate::model::{Gender, Pos};
    let mut form_sink = crate::forms::RecordSink::default();
    let mut lemma_sink = crate::forms::RecordSink::default();
    // An official base + its FULL paradigm — dedup must see inflected forms.
    for s in [&mut form_sink, &mut lemma_sink] {
        s.add(
            "kniga", "", "kniga", 1, "n", "lemma", "official", None, "book",
        );
    }
    crate::forms::paradigm_records(
        &mut form_sink,
        "kniga",
        Pos::Noun,
        Some(Gender::Feminine),
        1,
        "official",
        None,
        "book",
    );
    // Force a would-be derivative to already be official (knižny, denominal
    // adjective): the derivation of it MUST be dropped as a collision.
    for s in [&mut form_sink, &mut lemma_sink] {
        s.add(
            "knižny", "", "knižny", 2, "adj", "lemma", "official", None, "bookish",
        );
    }

    let official_keys = form_sink.form_key_set();
    let mut taken = official_keys.clone();
    let bases = vec![("kniga".to_string(), Pos::Noun, 1usize, "book".to_string())];
    let probs = crate::derive::DerivationProbabilities::flat(0.5);
    let added =
        inject_generated_derivatives(&mut form_sink, &mut lemma_sink, &mut taken, &bases, &probs);
    assert!(
        added > 0,
        "kniga must derive at least one absent family member"
    );

    let records = form_sink.into_records();
    for r in &records {
        if r.status == "generated" {
            assert!(
                !official_keys.contains(&r.key),
                "generated {} collides with an official/official-only key",
                r.key
            );
            assert_eq!(r.source, "lemma", "generated {} must be lemma-only", r.key);
            assert!(
                r.probability.is_some(),
                "generated {} must carry a probability",
                r.key
            );
            assert!(
                r.analyses.iter().any(|a| a.starts_with("deriv:")),
                "generated {} must carry deriv provenance",
                r.key
            );
        }
    }
    // The colliding member survives ONLY as the seeded official record.
    let knizny_key = crate::forms::form_key("knižny");
    assert!(
        records
            .iter()
            .filter(|r| r.key == knizny_key)
            .all(|r| r.status == "official"),
        "knižny must not be re-emitted as a generated derivative"
    );
    // A non-colliding member (the diminutive knižka) ships as generated.
    let knizka_key = crate::forms::form_key("knižka");
    assert!(
        records
            .iter()
            .any(|r| r.key == knizka_key && r.status == "generated"),
        "knižka should ship as a generated derivative"
    );
}

#[test]
fn source_aliases_index_cyrillic_latin_and_folded_forms() {
    // Issue #31: committee/cognate source words become searchable aliases.
    // Each alias carries the attested spelling + its folded search forms so
    // the entry is findable by Cyrillic, transliteration, and diacritic-fold.
    let mut aliases: Vec<SourceAlias> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_source_aliases(
        [
            ("ru", "пластинка"),
            ("pl", "płyta"),
            ("cs", "žena"),
            // A multi-variant cell splits into one alias per listed variant.
            ("uk", "швидкий, скорий"),
        ],
        &mut aliases,
        &mut seen,
    );
    let has = |lang: &str, word: &str| aliases.iter().any(|(l, w, _)| l == lang && w == word);
    // Attested spellings are indexed verbatim (Cyrillic query hits directly).
    assert!(has("ru", "пластинка"), "{aliases:?}");
    assert!(
        has("uk", "швидкий") && has("uk", "скорий"),
        "split: {aliases:?}"
    );
    // Folded search forms make the Latinized / diacritic-folded query hit.
    let forms = |lang: &str, word: &str| {
        aliases
            .iter()
            .find(|(l, w, _)| l == lang && w == word)
            .map(|(_, _, f)| f.clone())
            .unwrap_or_default()
    };
    assert!(
        forms("ru", "пластинка").iter().any(|f| f == "plastinka"),
        "ru transliteration: {aliases:?}"
    );
    // Polish ł does not decompose under the client's NFD fold, so the ASCII
    // skeleton (plyta) must be stored explicitly for the folded query to hit.
    assert!(
        forms("pl", "płyta").iter().any(|f| f == "plyta"),
        "pl skeleton: {aliases:?}"
    );
    assert!(
        forms("cs", "žena").iter().any(|f| f == "zena"),
        "cs skeleton: {aliases:?}"
    );
    // JSON is well-formed and preserves the attested spelling.
    let json = source_aliases_json(&aliases);
    assert!(json.starts_with('[') && json.ends_with(']'), "{json}");
    assert!(json.contains("[\"ru\",\"пластинка\","), "{json}");
}

#[test]
fn search_keys_make_alternatives_and_folds_findable() {
    // The kråtky case (V6 §2): the top candidate is flavored, the official
    // spelling appears as candidate 2 — both the alternative's form and the
    // ASCII folds must be searchable keys.
    let cands = vec![
        Candidate::new(
            "kråtȯky".to_string(),
            CandidateSource::ProtoSlavicRule,
            0.98,
        ),
        Candidate::new(
            "kratky".to_string(),
            CandidateSource::BranchConsensus,
            0.967,
        ),
    ];
    let keys = search_keys(&cands, "kråtȯky");
    let has = |k: &str, r: usize| keys.iter().any(|(kk, rr)| kk == k && *rr == r);
    assert!(has("kratoky", 1), "ASCII fold of the headword: {keys:?}");
    assert!(has("kratky", 2), "the alternative's own form: {keys:?}");
    // The raw headword itself is NOT duplicated (the client matches it).
    assert!(!keys.iter().any(|(k, _)| k == "kråtȯky"));
}

#[test]
fn proto_stems_group_word_families() {
    // One derivational suffix stripped, ≥4-char stem kept.
    assert_eq!(proto_stem("starъ").as_deref(), Some("star"));
    assert_eq!(proto_stem("starostь").as_deref(), Some("star"));
    assert_eq!(proto_stem("starьcь").as_deref(), Some("star"));
    // Combining accent marks (lemma-corpus reconstructions carry them:
    // pę̑tь) are folded first, so accent variants share one key.
    assert_eq!(proto_stem("sta\u{0301}rъ").as_deref(), Some("star"));
    // A root too short after stripping keeps the whole word as its key…
    assert_eq!(proto_stem("pьsъ").as_deref(), Some("pьsъ"));
    // …and a genuinely tiny fragment gets none.
    assert_eq!(proto_stem("kъ"), None);
}

#[test]
fn search_keys_json_is_well_formed() {
    let cands = vec![Candidate::new(
        "běly".to_string(),
        CandidateSource::ProtoSlavicRule,
        0.9,
    )];
    let keys = search_keys(&cands, "běly");
    let json = keys_json(&keys);
    assert!(json.starts_with('[') && json.ends_with(']'));
    assert!(json.contains("[\"bely\",1]"), "{json}");
}

#[test]
fn suppletive_plurals_come_from_the_inflector() {
    // RULE_SPEC §3.1: člověk→ljudi, oko→oči — the pinned inflector rev
    // implements them (with the heteroclite byforms); a rev bump that
    // loses them must fail here, not silently reshape the tables.
    assert!(noun_table("člověk", None).contains("ljudi"));
    assert!(noun_table("oko", None).contains("oči"));
}

#[test]
fn canonical_paradigms_pin_the_inflector_rev() {
    // A crate rev bump that changes these canonical cells (STEEN-G tables)
    // must fail CI, not silently reshape 30k inflection tables.
    let fold = |x: String| crate::orthography::to_standard(&x.to_lowercase());
    assert_eq!(
        fold(ISV::noun("žena", IsvCase::Gen, IsvNumber::Singular)),
        "ženy"
    );
    assert_eq!(
        fold(ISV::noun("grad", IsvCase::Gen, IsvNumber::Singular)),
        "grada"
    );
    assert_eq!(
        fold(ISV::adj(
            "dobry",
            IsvCase::Nom,
            IsvNumber::Singular,
            IsvGender::Feminine,
            IsvAnimacy::Inanimate
        )),
        "dobra"
    );
}

fn raw_lem(lang: &str, word: &str, pos: &str) -> crate::dump::RawSlavicLemma {
    crate::dump::RawSlavicLemma {
        word: word.to_string(),
        lang: lang.to_string(),
        pos: pos.to_string(),
        glosses: vec!["g".to_string()],
        etymology_text: String::new(),
        proto: String::new(),
        etymon: String::new(),
    }
}

#[test]
fn historical_cognates_render_as_labeled_hints_not_branch_evidence() {
    let member = |lang: &str, word: &str| crate::dump::LemmaEntry {
        lang: lang.to_string(),
        word: word.to_string(),
        pos: "noun".to_string(),
        gloss: "water".to_string(),
        proto: "*voda".to_string(),
        etymon: String::new(),
        etymology: Vec::new(),
        categories: Vec::new(),
        topics: Vec::new(),
        tags: Vec::new(),
    };
    let set = crate::corpus::CognateSet {
        proto: "*voda".to_string(),
        etymon: "*voda".to_string(),
        borrowed: false,
        pos: crate::model::Pos::Noun,
        gloss: "water".to_string(),
        members: vec![member("ru", "вода"), member("cu", "вода")],
    };
    let generated = crate::corpus::generate_set(set, &ConsensusConfig::production());
    let html = cognate_block(&generated, None);
    let historical = html.find("historical-hints").unwrap();
    assert!(html[..historical].contains("rusky"), "{html}");
    assert!(!html[..historical].contains("starocŕkov"), "{html}");
    assert!(html[historical..].contains("starocŕkov"), "{html}");
}

#[test]
fn official_matching_requires_pos_and_gloss_evidence() {
    let entries = crate::official::load(Path::new("data/official-isv.csv")).unwrap();
    let rows: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.isv.trim() == "držati")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        select_official_entry(
            &rows,
            &entries,
            crate::model::Pos::Noun,
            "chills, trembling"
        ),
        None
    );
    let matched = select_official_entry(
        &rows,
        &entries,
        crate::model::Pos::Verb,
        "to shiver and tremble",
    )
    .unwrap();
    assert_eq!(entries[matched].english, "shudder, shiver, tremble");

    let bajka_rows: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.isv.trim() == "bajka")
        .map(|(i, _)| i)
        .collect();
    let bajka = select_official_entry(&bajka_rows, &entries, crate::model::Pos::Noun, "fairy tale")
        .unwrap();
    assert_eq!(entries[bajka].english, "fairytale");
}

#[test]
fn official_surface_matching_uses_each_byform_without_fake_combined_spellings() {
    let entries = crate::official::load(Path::new("data/official-isv.csv")).unwrap();
    let (exact, folded) = official_surface_maps(&entries);
    assert!(!exact.contains_key("iměti, imati"));
    assert!(!exact.contains_key("poslědnji, poslědny"));

    let imeti = select_official_surface(
        &exact,
        &folded,
        "iměti",
        &entries,
        crate::model::Pos::Verb,
        "have, own, possess",
    )
    .unwrap();
    let imati = select_official_surface(
        &exact,
        &folded,
        "imati",
        &entries,
        crate::model::Pos::Verb,
        "have, own, possess",
    )
    .unwrap();
    assert_eq!(entries[imeti.entry].id, "875");
    assert_eq!(entries[imati.entry].id, "875");
    assert_eq!(imeti.form, "iměti");
    assert_eq!(imati.form, "imati");

    let posledny = select_official_surface(
        &exact,
        &folded,
        "poslědny",
        &entries,
        crate::model::Pos::Adjective,
        "last",
    )
    .unwrap();
    assert_eq!(entries[posledny.entry].id, "2323");
    assert_eq!(posledny.form, "poslědny");

    let kako = select_official_surface(
        &exact,
        &folded,
        "kako",
        &entries,
        crate::model::Pos::Adverb,
        "how",
    )
    .unwrap();
    assert_eq!(entries[kako.entry].id, "1193");
    assert_eq!(kako.form, "kako");

    assert!(select_official_surface(
        &exact,
        &folded,
        "lěgti",
        &entries,
        crate::model::Pos::Verb,
        "lie down",
    )
    .is_none());
}

#[test]
fn matched_official_byform_aliases_route_raw_dedup_to_the_same_page() {
    let entries = crate::official::load(Path::new("data/official-isv.csv")).unwrap();
    let entry = entries
        .iter()
        .position(|entry| entry.id == "875")
        .expect("iměti/imati fixture");
    let mut index = HeadwordIndex::default();
    insert_official_byform_aliases(&mut index, &entries, entry, 42);
    assert_eq!(index.resolve("iměti"), Some(42));
    assert_eq!(index.resolve("imati"), Some(42));
    assert_eq!(index.resolve("iměti, imati"), None);

    let mut raw_covered = std::collections::HashSet::new();
    assert!(matches!(
        raw_lemma_fate(
            &raw_lem("hr", "imati", "verb"),
            &crate::enrich::Xref::new(),
            &index,
            &mut raw_covered,
        ),
        RawFate::DedupedFold { target: 42 }
    ));
}

#[test]
fn headword_routes_exactly_and_abstains_on_ambiguous_folds() {
    let mut index = HeadwordIndex::default();
    index.insert("legti", 1);
    index.insert("lęgti", 2);
    assert_eq!(index.resolve("legti"), Some(1));
    assert_eq!(index.resolve("lęgti"), Some(2));
    assert_eq!(index.resolve("lěgti"), None);
    assert_eq!(index.resolve_fold("legti"), None);
}

/// Issue #64 invariants: the raw pre-pass assigns sequential ids in corpus
/// order, and every raw `(lang, word)` with a Slovowiki home resolves in
/// the plan's cross-reference — its own raw page, the official page its
/// display fold collided with, or the earlier raw twin that claimed the
/// same ě-blind fold. Cognate-xref members stay with the cognate xref;
/// empty words resolve nowhere.
#[test]
fn raw_plan_assigns_ids_and_points_chips_at_internal_pages() {
    let mut xref = crate::enrich::Xref::new();
    xref.insert("pl", "xyz", 7);
    xref.insert("pl", "xyz", 8); // ambiguous, but represented by generated pages
    let mut isv_to_id = HeadwordIndex::default();
    isv_to_id.insert("delo", 42); // an official headword
    let lemmas = vec![
        raw_lem("pl", "winyl", "noun"), // rendered → id 101
        raw_lem("cs", "mouka", "noun"), // rendered (muka) → id 102
        raw_lem("sl", "muka", "noun"),  // raw twin of mouka → points at 102
        raw_lem("sl", "delo", "noun"),  // folds onto official 42
        raw_lem("pl", "xyz", "noun"),   // cognate member → xref resolves
        raw_lem("pl", "", "noun"),      // skipped
    ];
    let plan = plan_raw_pages(&lemmas, &xref, &isv_to_id, 100);
    assert_eq!(plan.pages, vec![(0, 101), (1, 102)]);
    assert_eq!(plan.deduped, 4);
    assert_eq!(plan.xref.get("pl", "winyl"), Some(101));
    assert_eq!(plan.xref.get("cs", "mouka"), Some(102));
    assert_eq!(plan.xref.get("sl", "muka"), Some(102));
    assert_eq!(plan.xref.get("sl", "delo"), Some(42));
    assert_eq!(plan.xref.get("pl", "xyz"), None);
    assert_eq!(plan.xref.get("pl", ""), None);
    // Raw-collision display credit (issue #86 item 6): ONLY the
    // fold-deduped attestation is credited to its target page — cognate
    // members (xref) and raw twins are already visible elsewhere.
    assert_eq!(
        plan.credit.get(&42),
        Some(&vec![("sl".to_string(), "delo".to_string())])
    );
    assert_eq!(plan.credit.len(), 1);
    // The rendered line links to the source-language Wiktionary and
    // carries the display-only disclaimer.
    let line = raw_credit_line(plan.credit.get(&42));
    assert!(line.contains("Takože atestovano"), "{line}");
    assert!(line.contains("sl delo"), "{line}");
    assert!(line.contains("surova atestacija"), "{line}");
    assert_eq!(raw_credit_line(None), "");
    // Cap: 13 credits render 12 + "+1 dalje".
    let many: Vec<(String, String)> = (0..13)
        .map(|i| ("uk".to_string(), format!("слово{i}")))
        .collect();
    let line = raw_credit_line(Some(&many));
    assert!(line.contains("+1 dalje"), "{line}");
}

#[test]
fn deterministic_entry_ids_ignore_previous_output() {
    let sequence = || {
        let mut ids = DeterministicEntryIds::default();
        (ids.alloc(), ids.alloc(), ids.max_id())
    };
    assert_eq!(sequence(), (1, 2, 2));
    assert_eq!(sequence(), sequence());
    assert_eq!(
        BuildMeta::current(1, 1).unwrap().generated,
        BuildMeta::current(1, 1).unwrap().generated
    );
    assert_eq!(
        format_source_date_epoch("1784371344").unwrap(),
        "1784371344 UNIX"
    );
    assert!(format_source_date_epoch("not-an-epoch").is_err());
}

/// Test metas for the official-fact-treatment invariants (issue #86).
fn meta_for(
    conf: Confidence,
    prob: Option<f64>,
    prior: Option<f64>,
    official_only: bool,
    official_lemma: Option<&str>,
    langs: &[&str],
) -> SiteEntryMeta {
    let mut m = entry_meta(SiteEntryInput {
        id: 1,
        title: "aloe",
        gloss: "aloe",
        pos: "noun",
        confidence: conf,
        score: 0.30,
        probability: prob,
        n_languages: langs.len(),
        n_branches: 1,
        borrowed: true,
        official_only,
        official_lemma: official_lemma.map(str::to_string),
        ancestor: "grc ἀλόη".to_string(),
        languages: langs.iter().map(|s| s.to_string()).collect(),
        wiki_categories: Vec::new(),
    });
    m.prior = prior;
    m
}

/// Issue #86 defect 1: official dictionary words — matched AND
/// official-only — are facts. They never land on review worklists no
/// matter what the (now-irrelevant) confidence/probability say, while
/// machine-only reconstructions remain curation work.
#[test]
fn official_words_never_need_review() {
    let matched = meta_for(
        Confidence::High,
        None,
        Some(0.14),
        false,
        Some("aloe"),
        &["ru"],
    );
    assert!(!needs_review(&matched));
    let official_only = meta_for(Confidence::High, None, None, true, Some("aloe"), &["ru"]);
    assert!(!needs_review(&official_only));
    // A machine-only reconstruction stays on the worklist even at high
    // confidence (the old first clause `official_lemma.is_none()`).
    let generated = meta_for(
        Confidence::High,
        Some(0.73),
        None,
        false,
        None,
        &["ru", "pl"],
    );
    assert!(needs_review(&generated));
}

/// Issue #86 defect 1: the matched infobox states the fact ("oficialno",
/// no p≈ prior), like official-only pages; the calibrated prior surfaces
/// only as the muted provenance transparency line. Generated entries keep
/// the calibrated badge + p≈ and get no prior line.
#[test]
fn matched_meta_gets_the_official_fact_treatment() {
    let matched = meta_for(
        Confidence::High,
        None,
        Some(0.14),
        false,
        Some("aloe"),
        &["ru"],
    );
    let box_html = entry_infobox(&matched, "", "", "");
    assert!(box_html.contains(">oficialno</span>"), "{box_html}");
    assert!(!box_html.contains("p≈"), "{box_html}");
    let build = BuildMeta {
        git: "test".into(),
        generated: "0 UNIX".into(),
        total_entries: 1,
        lemma_total: 1,
    };
    let prov = provenance_block(&matched, &build);
    assert!(
        prov.contains("Priorna kalibrovana ocěna generatora: p≈0.14"),
        "{prov}"
    );
    // Generated entry: calibrated badge with p≈, no prior line.
    let generated = meta_for(Confidence::Low, Some(0.14), None, false, None, &["ru"]);
    let box_html = entry_infobox(&generated, "", "", "");
    assert!(box_html.contains("nizka"), "{box_html}");
    assert!(box_html.contains("p≈0.14"), "{box_html}");
    assert!(!provenance_block(&generated, &build).contains("Priorna"));
    // Search-row letter: the fact treatment sets g.confidence High for
    // matched entries, so conf_letter must yield "V" for them.
    assert_eq!(conf_letter(Confidence::High), "V");
}

#[test]
fn author_tools_ship_ascii_fallback_and_web_suggestion_parity_checks() {
    let forms = forms_page();
    assert!(forms.contains("isvLookupBroad('',q)"), "{forms}");
    assert!(forms.contains("ASCII råzširenje"), "{forms}");
    let checker = text_check_page();
    assert!(checker.contains("webSuggest('',tok)"), "{checker}");
    assert!(checker.contains("applySuggestion(this)"), "{checker}");
    assert!(checker.contains("suggest-selftest.json"), "{checker}");
}

#[test]
fn official_preposition_infobox_uses_checker_government() {
    let mut m = meta_for(
        Confidence::High,
        None,
        None,
        true,
        Some("bez"),
        &["ru", "pl"],
    );
    m.title = "bez".to_string();
    m.pos = "prep".to_string();
    let html = entry_infobox(&m, "", "", "");
    assert!(html.contains("<th>Upravljanje</th>"), "{html}");
    assert!(html.contains("bez + gen."), "{html}");

    m.title = "ne-prepozicija".to_string();
    m.official_lemma = Some("ne-prepozicija".to_string());
    let html = entry_infobox(&m, "", "", "");
    assert!(!html.contains("Upravljanje"), "{html}");
}

/// Issue #75: aspect metadata is bidirectional machine-readable data and
/// a direct partner link in the entry infobox.
#[test]
fn aspect_partners_are_exported_and_linked() {
    let mut m = meta_for(
        Confidence::High,
        None,
        None,
        true,
        Some("dobaviti"),
        &["ru", "pl"],
    );
    m.aspect = Some("pf".to_string());
    m.aspect_partners = vec![
        (24712, "dobavjati".to_string()),
        (24713, "pridobaviti".to_string()),
    ];
    let json = entries_json(&[m.clone()]);
    assert!(json.contains(r#""aspect":"pf""#), "{json}");
    assert!(
            json.contains(r#""aspect_partners":[{"id":24712,"title":"dobavjati"},{"id":24713,"title":"pridobaviti"}]"#),
            "{json}"
        );
    let html = entry_infobox(&m, "", "", "");
    assert!(html.contains("Glagolsky vid</th><td>pf"), "{html}");
    assert!(html.contains("href='24712.html'>dobavjati</a>"), "{html}");
    assert!(html.contains("href='24713.html'>pridobaviti</a>"), "{html}");
}

/// Issue #86 defect 2: the razumlivost basis for a matched entry is the
/// UNION of corpus members and the official row's sameInLanguages — an
/// aloe-like case (corpus = ru only, committee says "v z j") must read
/// ≈99%, not ru's 52% share. An empty sameInLanguages leaves the corpus
/// basis untouched (and official-only pages keep their same_in-only
/// basis — no union with translation cells, which would re-saturate).
#[test]
fn matched_razum_union_basis() {
    let members = vec!["ru".to_string()];
    // "v z j" expands to every modern CSV language across the branches.
    let same_in: Vec<&'static str> = crate::lang::official_slavic_cols()
        .iter()
        .filter(|l| l.modern)
        .map(|l| l.code)
        .collect();
    let union = union_razum_codes(&members, &same_in);
    assert!(union.contains(&"ru".to_string()));
    assert!(union.contains(&"pl".to_string()));
    assert!(union.contains(&"bg".to_string()));
    let pct = razum_pct(&union);
    assert!(pct >= 95, "union basis should read ≈99%, got {pct}");
    // Corpus basis alone (ru) is far lower — the defect the union fixes.
    assert!(razum_pct(&members) < 60);
    // Empty same_in: the corpus basis is unchanged.
    assert_eq!(union_razum_codes(&members, &[]), members);
}

/// The client fold is generated from CLIENT_FOLD_PAIRS (injected as
/// __SEARCH_FOLD__), so Rust bucketing and JS query folding share one
/// definition (#60/#71). Pin its semantics: Latin diacritics fold to base
/// letters, đ/ł included; Cyrillic passes through for its own shards.
#[test]
fn client_fold_and_buckets_are_stable() {
    assert_eq!(client_fold("Čech"), "cech");
    assert_eq!(client_fold("kråtȯky"), "kratoky");
    assert_eq!(client_fold("vođa"), "voda");
    assert_eq!(client_fold("łapeć"), "lapec");
    assert_eq!(client_fold("пластинка"), "пластинка");
    assert_eq!(client_fold("вода́"), "вода"); // combining mark stripped
    assert_eq!(search_bucket_pair("rěka"), Some(('r', 'e')));
    assert_eq!(search_bucket_pair("s"), Some(('s', '_')));
    assert_eq!(search_bucket_pair("пластинка"), Some(('п', 'л')));
    assert_eq!(search_bucket_pair("…—"), None);
    assert_eq!(shard_file_name("vo"), "vo.json");
    assert_eq!(shard_file_name("п"), "u043f.json");
    assert_eq!(shard_file_name("s_"), "s_.json");
}

/// write_search_index end-to-end on synthetic rows: manifest + shard
/// resolution (two-letter split for a hot bucket, one-letter otherwise),
/// browse/spotlight carry only core rows, the 14-element row shape
/// (aliases last, razumlivost at 12; #79), and the completeness
/// self-check accepts the layout.
#[test]
fn search_index_shards_resolve_completely() {
    fn row(id: usize, display: &str, gloss: &str, core: bool) -> SearchRow {
        let keys: Vec<(String, usize)> = Vec::new();
        let aliases: Vec<SourceAlias> =
            vec![("ru".into(), "пример".into(), vec![format!("primer{id}")])];
        SearchRow {
            id,
            head: format!(
                "[{},{},{},\"noun\",\"N\",\"N\",[],1,1,0,\"\",\"\",0",
                id,
                json_str(display),
                json_str(gloss)
            ),
            aliases: source_aliases_json(&aliases),
            core,
            buckets: search_row_buckets(display, gloss, &keys, &aliases),
        }
    }
    let mut rows: Vec<SearchRow> = vec![
        row(1, "voda", "water", true),
        row(2, "rěka", "river", true),
        row(3, "gramplastinka", "gramophone record", false),
    ];
    // A hot bucket: enough s-rows to exceed the split budget.
    for i in 0..2100 {
        let mut r = row(1000 + i, &format!("slovo{i}"), &"x".repeat(700), false);
        r.aliases = "[]".into();
        r.buckets = search_row_buckets(&format!("slovo{i}"), "", &[], &[]);
        rows.push(r);
    }
    let dir = std::env::temp_dir().join(format!("shard-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (shards, browse) = write_search_index(&dir, &rows).unwrap();
    assert!(shards >= 4, "expected multiple shards, got {shards}");
    assert_eq!(browse, 2); // only the core rows
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("search/manifest.json")).unwrap()).unwrap();
    let sh = manifest["shards"].as_object().unwrap();
    // Hot 's' bucket split by second letter; 'v' stayed single-letter.
    assert!(manifest["splits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "s"));
    assert!(sh.contains_key("sl") && !sh.contains_key("s") && sh.contains_key("v"));
    // A Cyrillic alias gives the row a Cyrillic shard (пример → п…).
    assert!(sh.keys().any(|k| k.starts_with('п')));
    // The raw row is reachable through its own bucket but not in browse.
    let v_file = sh["v"]["f"].as_str().unwrap();
    let v_rows: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("search").join(v_file)).unwrap()).unwrap();
    assert!(v_rows.as_array().unwrap().iter().any(|r| r[0] == 1));
    // Written row shape (schema 2, #79): 14 elements, razumlivost integer
    // at 12, aliases array LAST at 13.
    let voda = v_rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r[0] == 1)
        .unwrap();
    assert_eq!(voda.as_array().unwrap().len(), 14, "{voda}");
    assert!(voda[12].is_u64(), "{voda}");
    assert!(voda[13].is_array(), "{voda}");
    assert_eq!(manifest["schema"], 2);
    let browse_rows: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("search/browse.json")).unwrap()).unwrap();
    assert!(browse_rows.as_array().unwrap().iter().all(|r| r[0] != 3));
    // no_alias rows keep the same shape with an empty aliases tail.
    let b0 = &browse_rows.as_array().unwrap()[0];
    assert_eq!(b0.as_array().unwrap().len(), 14, "{b0}");
    assert!(b0[13].as_array().unwrap().is_empty(), "{b0}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The chip lookup chain: cognate xref beats the raw cross-reference beats
/// the external native-Wiktionary fallback, and a self-link falls through
/// to the external target.
#[test]
fn word_chip_prefers_generated_then_raw_then_external() {
    let mut xref = crate::enrich::Xref::new();
    xref.insert("ru", "дело", 5);
    let mut raw_xref = crate::enrich::Xref::new();
    raw_xref.insert("ru", "грампластинка", 123);
    raw_xref.insert("ru", "дело", 999); // shadowed by the cognate xref
    assert!(word_chip("ru", "дело", "dělo", Some(&xref), &raw_xref, 0).contains("href='5.html'"));
    assert!(word_chip(
        "ru",
        "грампластинка",
        "gramplastinka",
        Some(&xref),
        &raw_xref,
        0
    )
    .contains("href='123.html'"));
    let self_chip = word_chip(
        "ru",
        "грампластинка",
        "gramplastinka",
        Some(&xref),
        &raw_xref,
        123,
    );
    assert!(self_chip.contains("ru.wiktionary.org"), "{self_chip}");
    assert!(
        word_chip("uk", "щось", "ščos", Some(&xref), &raw_xref, 0).contains("uk.wiktionary.org")
    );

    // Once a generated key is ambiguous, do not replace its sense choice
    // with an arbitrary raw-page target; link to the native disambiguation
    // surface instead.
    xref.insert("ru", "дело", 6);
    let ambiguous = word_chip("ru", "дело", "dělo", Some(&xref), &raw_xref, 0);
    assert!(ambiguous.contains("ru.wiktionary.org"), "{ambiguous}");
    assert!(!ambiguous.contains("999.html"), "{ambiguous}");
}
